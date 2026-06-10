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
    menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State,
};
use tokio::sync::Mutex;

const TRAY_ID: &str = "droplocal-tray";

struct ManagedState {
    settings_path: PathBuf,
    settings: Mutex<DesktopSettings>,
    runtime: Mutex<Option<ServerRuntime>>,
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
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn open_share_url(app: AppHandle) -> Result<(), String> {
    let url = current_primary_url(&app).await?;
    open::that_detached(url).map_err(|error| error.to_string())
}

#[tauri::command]
async fn copy_share_url(app: AppHandle) -> Result<(), String> {
    let url = current_primary_url(&app).await?;
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    clipboard.set_text(url).map_err(|error| error.to_string())
}

#[tauri::command]
fn build_qr_svg(payload: String) -> Result<String, String> {
    let qr = QrCode::new(payload.as_bytes()).map_err(|error| error.to_string())?;
    Ok(qr
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .dark_color(svg::Color("#1f2a1d"))
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

    {
        let existing = state.runtime.lock().await;
        if let Some(runtime) = existing.as_ref() {
            let snapshot = runtime.snapshot();
            emit_runtime_update(app, &snapshot);
            return Ok(snapshot);
        }
    }

    let settings = { state.settings.lock().await.clone() };

    let config = ServerConfig {
        requested_port: settings.port,
        storage_dir: settings.resolved_storage_dir(),
        auto_clean_on_quit: settings.auto_clean_on_quit,
    };

    let runtime = server::start(config)
        .await
        .map_err(|error| format!("failed to start server: {error}"))?;
    let snapshot = runtime.snapshot();

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
}

fn reveal_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let status_item = MenuItemBuilder::with_id("status", "DropLocal status is in dashboard")
        .enabled(false)
        .build(app)?;
    let show_item = MenuItemBuilder::with_id("show_window", "Open Dashboard").build(app)?;
    let open_item = MenuItemBuilder::with_id("open_browser", "Open in Browser").build(app)?;
    let copy_item = MenuItemBuilder::with_id("copy_url", "Copy URL").build(app)?;
    let toggle_item =
        MenuItemBuilder::with_id("toggle_server", "Start / Stop Server").build(app)?;
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
            &separator_bottom,
            &toggle_item,
            &quit_item,
        ])
        .build()?;

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
                "quit" => {
                    tauri::async_runtime::spawn(async move {
                        let _ = stop_server_inner(&app_handle).await;
                        app_handle.exit(0);
                    });
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
                settings: Mutex::new(settings),
                runtime: Mutex::new(None),
            });

            create_tray(app.handle())?;

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = start_server_inner(&app_handle).await {
                    eprintln!("droplocal desktop startup failed: {error}");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            build_qr_svg,
            copy_share_url,
            get_runtime_status,
            get_settings,
            open_share_url,
            restart_server_with_settings,
            save_settings,
            start_server,
            stop_server
        ])
        .run(tauri::generate_context!())
        .expect("error while running DropLocal desktop");
}
