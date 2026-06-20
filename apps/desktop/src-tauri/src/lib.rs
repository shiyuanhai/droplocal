mod server;
mod settings;

use std::path::PathBuf;

use anyhow::Context;
use arboard::Clipboard;
use qrcode::render::svg;
use qrcode::QrCode;
use server::{RuntimeStatus, ServerConfig, ServerRuntime};
use settings::{load_or_default, save as save_settings_file, DesktopSettings};
use tauri::{
    menu::{MenuBuilder, MenuItem, MenuItemBuilder, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent, Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as AutostartManagerExt};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;
use tokio::sync::Mutex;

const TRAY_ID: &str = "droplocal-tray";
/// CLI marker added to the login-item command so a launch-at-login start can
/// be told apart from a manual one (manual launches always show the window).
const AUTOSTART_FLAG: &str = "--from-autostart";

struct ManagedState {
    settings_path: PathBuf,
    settings: Mutex<DesktopSettings>,
    runtime: Mutex<Option<ServerRuntime>>,
}

/// Tray menu items whose labels track the server state.
struct TrayHandles {
    status: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
}

#[tauri::command]
async fn get_runtime_status(state: State<'_, ManagedState>) -> Result<RuntimeStatus, String> {
    let runtime = state.runtime.lock().await;
    if let Some(runtime) = runtime.as_ref() {
        Ok(runtime.snapshot())
    } else {
        Ok(server::stopped_status())
    }
}

#[tauri::command]
async fn start_server(app: AppHandle) -> Result<RuntimeStatus, String> {
    start_server_inner(&app).await
}

#[tauri::command]
async fn stop_server(app: AppHandle) -> Result<RuntimeStatus, String> {
    stop_server_inner(&app).await
}

#[tauri::command]
async fn restart_server_with_settings(app: AppHandle) -> Result<RuntimeStatus, String> {
    stop_server_inner(&app).await?;
    start_server_inner(&app).await
}

#[tauri::command]
async fn get_settings(state: State<'_, ManagedState>) -> Result<DesktopSettings, String> {
    let settings = state.settings.lock().await;
    Ok(settings.clone())
}

#[tauri::command]
async fn save_settings(
    app: AppHandle,
    state: State<'_, ManagedState>,
    settings: DesktopSettings,
) -> Result<(), String> {
    let validated = settings.validated();

    {
        let mut current = state.settings.lock().await;
        *current = validated.clone();
    }

    save_settings_file(&state.settings_path, &validated)
        .await
        .map_err(|error| error.to_string())?;

    apply_system_integration(&app, &validated);
    Ok(())
}

#[tauri::command]
async fn open_share_url(app: AppHandle) -> Result<(), String> {
    let url = current_primary_url(&app).await?;
    open::that_detached(url).map_err(|error| error.to_string())
}

#[tauri::command]
async fn copy_share_url(app: AppHandle) -> Result<(), String> {
    // Prefer the friendly mDNS address when available — it's what a human
    // wants to pass along; the IP URL stays visible in the dashboard.
    let state = app.state::<ManagedState>();
    let url = {
        let runtime = state.runtime.lock().await;
        runtime
            .as_ref()
            .map(|entry| {
                let snapshot = entry.snapshot();
                if snapshot.friendly_url.is_empty() {
                    snapshot.primary_url
                } else {
                    snapshot.friendly_url
                }
            })
            .filter(|url| !url.is_empty())
            .ok_or_else(|| "DropLocal server is not running".to_string())?
    };
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    clipboard.set_text(url).map_err(|error| error.to_string())
}

#[tauri::command]
async fn copy_invite_link(app: AppHandle) -> Result<(), String> {
    let state = app.state::<ManagedState>();
    let url = {
        let runtime = state.runtime.lock().await;
        let Some(runtime) = runtime.as_ref() else {
            return Err("DropLocal server is not running".to_string());
        };
        runtime.create_invite_url().await
    };
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    clipboard.set_text(url).map_err(|error| error.to_string())
}

#[tauri::command]
async fn copy_debug_info(app: AppHandle) -> Result<(), String> {
    let state = app.state::<ManagedState>();
    let debug = {
        let runtime = state.runtime.lock().await;
        let Some(runtime) = runtime.as_ref() else {
            return Err("DropLocal server is not running".to_string());
        };
        runtime.debug_info().await
    };
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    clipboard.set_text(debug).map_err(|error| error.to_string())
}

#[tauri::command]
async fn drop_clipboard(app: AppHandle) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    let text = clipboard
        .get_text()
        .map_err(|_| "Clipboard has no text to drop".to_string())?;

    let state = app.state::<ManagedState>();
    let runtime = state.runtime.lock().await;
    let Some(runtime) = runtime.as_ref() else {
        return Err("DropLocal server is not running".to_string());
    };
    runtime
        .drop_text(&text)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn build_qr_svg(payload: String) -> Result<String, String> {
    let qr = QrCode::new(payload.as_bytes()).map_err(|error| error.to_string())?;
    Ok(qr
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .dark_color(svg::Color("#0f172a"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

async fn current_primary_url(app: &AppHandle) -> Result<String, String> {
    let state = app.state::<ManagedState>();
    let runtime = state.runtime.lock().await;
    runtime
        .as_ref()
        .map(|entry| entry.snapshot().primary_url)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "DropLocal server is not running".to_string())
}

async fn start_server_inner(app: &AppHandle) -> Result<RuntimeStatus, String> {
    let state = app.state::<ManagedState>();

    // Snapshot under the lock, emit after releasing it — emit_runtime_update
    // blocks on a main-thread dispatch (tray text), and the Exit handler
    // locks this mutex from the main thread.
    let existing_snapshot = {
        let existing = state.runtime.lock().await;
        existing.as_ref().map(|runtime| runtime.snapshot())
    };
    if let Some(snapshot) = existing_snapshot {
        emit_runtime_update(app, &snapshot);
        return Ok(snapshot);
    }

    let settings = { state.settings.lock().await.clone() };

    let config = ServerConfig {
        requested_port: settings.port,
        storage_dir: settings.resolved_storage_dir(),
        auto_clean_on_quit: settings.auto_clean_on_quit,
        pin: settings.pin.clone(),
        expire_minutes: settings.expire_minutes,
        enable_mdns: true,
        network_interface: settings.network_interface.clone(),
    };

    let runtime = server::start(config)
        .await
        .map_err(|error| format!("failed to start server: {error}"))?;
    let snapshot = runtime.snapshot();

    spawn_notification_listener(app.clone(), &runtime);

    {
        let mut slot = state.runtime.lock().await;
        *slot = Some(runtime);
    }

    emit_runtime_update(app, &snapshot);
    Ok(snapshot)
}

async fn stop_server_inner(app: &AppHandle) -> Result<RuntimeStatus, String> {
    let state = app.state::<ManagedState>();

    let runtime = {
        let mut slot = state.runtime.lock().await;
        slot.take()
    };

    if let Some(mut runtime) = runtime {
        runtime
            .stop()
            .await
            .map_err(|error| format!("failed to stop server: {error}"))?;
    }

    let stopped = server::stopped_status();
    emit_runtime_update(app, &stopped);
    Ok(stopped)
}

fn emit_runtime_update(app: &AppHandle, status: &RuntimeStatus) {
    let _ = app.emit("droplocal://runtime-updated", status.clone());

    if let Some(handles) = app.try_state::<TrayHandles>() {
        let _ = handles.status.set_text(tray_status_text(status));
        let _ = handles.toggle.set_text(if status.running {
            "Stop Server"
        } else {
            "Start Server"
        });
    }
}

fn tray_status_text(status: &RuntimeStatus) -> String {
    if !status.running {
        return "Stopped".to_string();
    }

    let url = if status.friendly_url.is_empty() {
        &status.primary_url
    } else {
        &status.friendly_url
    };
    let host = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');

    if host.is_empty() {
        "Running".to_string()
    } else {
        format!("Running — {host}")
    }
}

/// Sync OS-level presentation with the saved settings: Dock icon visibility
/// (macOS activation policy) and the login item. Both are best-effort —
/// failures are logged, never surfaced as save errors.
fn apply_system_integration(app: &AppHandle, settings: &DesktopSettings) {
    #[cfg(target_os = "macos")]
    {
        let policy = if settings.show_dock_icon {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        };
        if let Err(error) = app.set_activation_policy(policy) {
            eprintln!("droplocal: failed to set activation policy: {error}");
        }
    }

    let autolaunch = app.autolaunch();
    let result = if settings.launch_at_login {
        // enable() idempotently rewrites the login item with the current
        // executable path, healing stale entries (dev builds, app moves) —
        // so it runs on every launch, not only on state mismatch. Never
        // record a Gatekeeper-translocated or DMG path: it dies on reboot.
        let translocated = std::env::current_exe().is_ok_and(|exe| {
            let exe = exe.to_string_lossy().to_string();
            exe.contains("/AppTranslocation/") || exe.starts_with("/Volumes/")
        });
        if translocated {
            eprintln!(
                "droplocal: skipping launch-at-login registration from a temporary path; move DropLocal to /Applications first"
            );
            Ok(())
        } else {
            autolaunch.enable()
        }
    } else {
        match autolaunch.is_enabled() {
            Ok(true) => autolaunch.disable(),
            Ok(false) => Ok(()),
            Err(error) => {
                eprintln!("droplocal: failed to query launch-at-login: {error}");
                Ok(())
            }
        }
    };
    if let Err(error) = result {
        eprintln!("droplocal: failed to update launch-at-login: {error}");
    }
}

/// Update check with a consent dialog. Runs on launch (quiet when already
/// current) and from the tray. Failures are logged, never fatal — the
/// updater only works on releases signed with the updater key.
async fn check_for_updates(app: AppHandle, manual: bool) {
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            eprintln!("droplocal updater unavailable: {error}");
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            let version = update.version.clone();
            let dialog_app = app.clone();
            let confirmed = tauri::async_runtime::spawn_blocking(move || {
                dialog_app
                    .dialog()
                    .message(format!(
                        "DropLocal {version} is available.\nInstall now and relaunch?"
                    ))
                    .title("Update available")
                    .kind(MessageDialogKind::Info)
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "Install & Relaunch".to_string(),
                        "Later".to_string(),
                    ))
                    .blocking_show()
            })
            .await
            .unwrap_or(false);

            if !confirmed {
                return;
            }

            match update.download_and_install(|_, _| {}, || {}).await {
                Ok(()) => {
                    app.restart();
                }
                Err(error) => eprintln!("droplocal update failed: {error}"),
            }
        }
        Ok(None) => {
            if manual {
                let dialog_app = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    dialog_app
                        .dialog()
                        .message("DropLocal is up to date.")
                        .title("No updates")
                        .kind(MessageDialogKind::Info)
                        .blocking_show()
                })
                .await;
            } else {
                eprintln!("droplocal is up to date");
            }
        }
        Err(error) => eprintln!("droplocal update check failed: {error}"),
    }
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Forward server events to desktop notifications, honoring the settings.
fn spawn_notification_listener(app: AppHandle, runtime: &ServerRuntime) {
    let mut events = runtime.subscribe_events();
    tauri::async_runtime::spawn(async move {
        let mut last_count: u64 = 0;
        while let Ok(envelope) = events.recv().await {
            let settings = {
                let state = app.state::<ManagedState>();
                let guard = state.settings.lock().await;
                guard.clone()
            };

            match envelope.event.as_str() {
                "device:count" => {
                    let count = envelope
                        .data
                        .get("count")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    if settings.notify_on_device_connect && count > last_count {
                        notify(
                            &app,
                            "DropLocal",
                            &format!("A device connected ({count} online)"),
                        );
                    }
                    last_count = count;
                }
                "file:new" if settings.notify_on_new_drop => {
                    let name = envelope
                        .data
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("file");
                    notify(&app, "New drop", name);
                }
                "snippet:new" if settings.notify_on_new_drop => {
                    notify(&app, "New drop", "A note was shared");
                }
                _ => {}
            }
        }
    });
}

fn reveal_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// True when this process was started by the login item (autostart), so the
/// dashboard should stay hidden in the menu bar instead of popping up.
fn launched_at_login() -> bool {
    std::env::args().any(|arg| arg == AUTOSTART_FLAG)
}

/// First time the user closes the dashboard, tell them it's still running in
/// the menu bar / system tray rather than quitting — otherwise it looks like
/// the app vanished. Shown once, then remembered in settings.
fn show_menu_bar_hint_once(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<ManagedState>();

        let snapshot = {
            let mut settings = state.settings.lock().await;
            if settings.menu_bar_hint_seen {
                return;
            }
            settings.menu_bar_hint_seen = true;
            settings.clone()
        };

        let _ = save_settings_file(&state.settings_path, &snapshot).await;

        #[cfg(target_os = "macos")]
        let body = "DropLocal is still running in the menu bar — click its icon up top to reopen, or use Quit to stop sharing.";
        #[cfg(target_os = "windows")]
        let body = "DropLocal is still running in the system tray — click its icon to reopen, or use Quit to stop sharing.";
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let body = "DropLocal is still running in the background.";

        notify(&app, "DropLocal is still running", body);
    });
}

fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let status_item = MenuItemBuilder::with_id("status", "Starting…")
        .enabled(false)
        .build(app)?;
    let show_item = MenuItemBuilder::with_id("show_window", "Open Dashboard").build(app)?;
    let open_item = MenuItemBuilder::with_id("open_browser", "Open in Browser").build(app)?;
    let copy_item = MenuItemBuilder::with_id("copy_url", "Copy URL").build(app)?;
    let drop_clipboard_item =
        MenuItemBuilder::with_id("drop_clipboard", "Drop Clipboard").build(app)?;
    let toggle_item = MenuItemBuilder::with_id("toggle_server", "Start Server").build(app)?;
    let update_item = MenuItemBuilder::with_id("check_updates", "Check for Updates").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit DropLocal").build(app)?;
    let separator_top = PredefinedMenuItem::separator(app)?;
    let separator_bottom = PredefinedMenuItem::separator(app)?;

    let menu = MenuBuilder::new(app)
        .items(&[
            &status_item,
            &separator_top,
            &show_item,
            &open_item,
            &copy_item,
            &drop_clipboard_item,
            &separator_bottom,
            &toggle_item,
            &update_item,
            &quit_item,
        ])
        .build()?;

    app.manage(TrayHandles {
        status: status_item.clone(),
        toggle: toggle_item.clone(),
    });

    let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../../icons/tray-icon.png"))?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_icon)
        .icon_as_template(true)
        .menu(&menu)
        .tooltip("DropLocal")
        .on_menu_event(|app, event| {
            let app_handle = app.clone();
            match event.id().as_ref() {
                "show_window" => {
                    reveal_main_window(&app_handle);
                }
                "open_browser" => {
                    tauri::async_runtime::spawn(async move {
                        let _ = open_share_url(app_handle).await;
                    });
                }
                "copy_url" => {
                    tauri::async_runtime::spawn(async move {
                        let _ = copy_share_url(app_handle).await;
                    });
                }
                "drop_clipboard" => {
                    tauri::async_runtime::spawn(async move {
                        match drop_clipboard(app_handle.clone()).await {
                            Ok(()) => notify(&app_handle, "DropLocal", "Clipboard dropped"),
                            Err(error) => notify(&app_handle, "DropLocal", &error),
                        }
                    });
                }
                "toggle_server" => {
                    tauri::async_runtime::spawn(async move {
                        let status = {
                            let state = app_handle.state::<ManagedState>();
                            let runtime = state.runtime.lock().await;
                            runtime
                                .as_ref()
                                .map(|entry| entry.snapshot().running)
                                .unwrap_or(false)
                        };

                        if status {
                            let _ = stop_server_inner(&app_handle).await;
                        } else {
                            let _ = start_server_inner(&app_handle).await;
                        }
                    });
                }
                "check_updates" => {
                    tauri::async_runtime::spawn(async move {
                        check_for_updates(app_handle, true).await;
                    });
                }
                "quit" => {
                    tauri::async_runtime::spawn(async move {
                        let _ = stop_server_inner(&app_handle).await;
                        app_handle.exit(0);
                    });
                }
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut app = tauri::Builder::default()
        // Must be the first plugin so a second launch exits before doing
        // any work; it reveals the running instance's dashboard instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            reveal_main_window(app);
        }))
        // The login-item launch carries this marker so setup() can start
        // silently in the menu bar, while a manual launch always shows the
        // window.
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_FLAG]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            // Menu-bar app: closing the dashboard hides it, the server keeps
            // running. Quitting lives in the tray menu. Linux is exempt —
            // stock GNOME has no tray host, so the tray icon may not exist
            // and closing the window must remain a real quit.
            if let WindowEvent::CloseRequested { api, .. } = event {
                #[cfg(not(target_os = "linux"))]
                {
                    api.prevent_close();
                    let _ = window.hide();
                    show_menu_bar_hint_once(window.app_handle());
                }
                #[cfg(target_os = "linux")]
                let _ = (window, api);
            }
        })
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .context("unable to locate app config directory")?;
            let settings_path = config_dir.join("settings.json");
            let settings = tauri::async_runtime::block_on(load_or_default(&settings_path))
                .unwrap_or_else(|_| DesktopSettings::default());

            app.manage(ManagedState {
                settings_path,
                settings: Mutex::new(settings.clone()),
                runtime: Mutex::new(None),
            });

            create_tray(app.handle())?;
            apply_system_integration(app.handle(), &settings);

            // Show the dashboard (QR + share link) on every manual launch so
            // the window is easy to find — only a launch-at-login start stays
            // silent in the menu bar. On Linux the window always shows: the
            // tray is the only other affordance and not every desktop has one.
            if cfg!(target_os = "linux") || !launched_at_login() {
                reveal_main_window(app.handle());
            }

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = start_server_inner(&app_handle).await {
                    eprintln!("droplocal desktop startup failed: {error}");
                }
            });

            // Quiet update check shortly after launch; the dialog only
            // appears when an update actually exists.
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                check_for_updates(update_handle, false).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            build_qr_svg,
            copy_debug_info,
            drop_clipboard,
            copy_invite_link,
            copy_share_url,
            get_runtime_status,
            get_settings,
            open_share_url,
            restart_server_with_settings,
            save_settings,
            start_server,
            stop_server
        ])
        .build(tauri::generate_context!())
        .expect("error while building DropLocal desktop");

    // Seed the activation policy before the event loop starts: the setup
    // hook runs after AppKit applies the default Regular policy, so setting
    // Accessory only there makes the Dock icon flash on every launch.
    #[cfg(target_os = "macos")]
    {
        let show_dock = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| dir.join("settings.json"))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|raw| serde_json::from_str::<DesktopSettings>(&raw).ok())
            .map(|settings| settings.show_dock_icon)
            .unwrap_or(false);
        if !show_dock {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
        }
    }

    app.run(|app, event| match event {
        // Tray app: keep the event loop alive when the last window closes
        // (code is None). Programmatic exits — tray Quit, updater restart —
        // carry Some(code) and pass through. On Linux the window is the
        // primary surface, so closing it quits as before.
        #[cfg(not(target_os = "linux"))]
        tauri::RunEvent::ExitRequested {
            code: None, api, ..
        } => {
            api.prevent_exit();
        }
        // macOS: relaunching the app (Finder/Spotlight/Launchpad/Dock) while
        // the window is hidden delivers a reopen event — show the dashboard.
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => {
            reveal_main_window(app);
        }
        // Cmd+Q / NSApp `terminate:` skips ExitRequested entirely on macOS;
        // Exit is the only hook that still fires there, so stop the server
        // (mDNS unregister, auto-clean) here. Idempotent with the tray Quit
        // path, which already stopped it.
        tauri::RunEvent::Exit => {
            let state = app.state::<ManagedState>();
            let runtime =
                tauri::async_runtime::block_on(async { state.runtime.lock().await.take() });
            if let Some(mut runtime) = runtime {
                let _ = tauri::async_runtime::block_on(runtime.stop());
            }
        }
        _ => {}
    });
}
