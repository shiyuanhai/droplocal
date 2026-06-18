use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, DefaultBodyLimit, Multipart, Path as AxumPath, Query, State,
    },
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    fs,
    io::AsyncWriteExt,
    net::TcpListener,
    sync::{broadcast, oneshot, RwLock},
    task::JoinHandle,
};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

const MAX_PORT_RETRIES: u16 = 20;
const AUTO_PORT_PRIMARY: u16 = 80;
const AUTO_PORT_FALLBACK: u16 = 3000;
const MDNS_HOSTNAME: &str = "drop.local";
const AUTH_COOKIE: &str = "droplocal_auth";
const INDEX_FILE_NAME: &str = ".droplocal.json";
const EXPIRY_SWEEP_INTERVAL_SECS: u64 = 30;
const INVITE_TTL_SECS: u64 = 10 * 60;
const AUTH_FAILURE_LOCK_THRESHOLD: u32 = 3;
const AUTH_BACKOFF_BASE_SECS: u64 = 1;
const AUTH_BACKOFF_MAX_SECS: u64 = 30;
const EMBEDDED_UI: &str = include_str!("../../../../ui.html");
const FAVICON_SVG: &str = include_str!("../../../../assets/brand/logo.svg");
const TOUCH_ICON_PNG: &[u8] = include_bytes!("../../../../assets/brand/apple-touch-icon.png");
const QRCODE_VENDOR_JS: &str = include_str!("../../../../assets/vendor/qrcode.js");
const ICON_192_PNG: &[u8] = include_bytes!("../../../../assets/brand/icon-192.png");
const ICON_512_PNG: &[u8] = include_bytes!("../../../../assets/brand/icon-512.png");
const WEB_MANIFEST: &str = r##"{"id":"/","name":"DropLocal","short_name":"DropLocal","description":"Drop it local. Pick it up anywhere.","start_url":"/","scope":"/","display":"standalone","background_color":"#F5F7FB","theme_color":"#4F6BF5","categories":["productivity","utilities"],"icons":[{"src":"/icons/icon-192.png","sizes":"192x192","type":"image/png"},{"src":"/icons/icon-512.png","sizes":"512x512","type":"image/png"}]}"##;
const SERVICE_WORKER_TEMPLATE: &str = r#"const CACHE_NAME = "droplocal-shell-__DROPLOCAL_VERSION__";
const SHELL_ASSETS = [
  "/",
  "/manifest.webmanifest",
  "/favicon.svg",
  "/apple-touch-icon.png",
  "/icons/icon-192.png",
  "/icons/icon-512.png",
  "/vendor/qrcode.js"
];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) => cache.addAll(SHELL_ASSETS))
      .then(() => self.skipWaiting())
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.map((key) => (key.startsWith("droplocal-shell-") && key !== CACHE_NAME ? caches.delete(key) : undefined)))
      )
      .then(() => self.clients.claim())
  );
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") {
    return;
  }

  const url = new URL(request.url);
  if (url.origin !== self.location.origin || url.pathname.startsWith("/api/") || url.pathname === "/ws") {
    return;
  }

  if (request.mode === "navigate") {
    event.respondWith(
      fetch(request)
        .then((response) => {
          const copy = response.clone();
          caches.open(CACHE_NAME).then((cache) => cache.put("/", copy));
          return response;
        })
        .catch(() => caches.match("/"))
    );
    return;
  }

  if (SHELL_ASSETS.includes(url.pathname)) {
    event.respondWith(
      caches.match(request).then((cached) => {
        const network = fetch(request)
          .then((response) => {
            if (response.ok) {
              caches.open(CACHE_NAME).then((cache) => cache.put(request, response.clone()));
            }
            return response;
          })
          .catch(() => cached);
        return cached || network;
      })
    );
  }
});"#;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub requested_port: u16,
    pub storage_dir: PathBuf,
    pub auto_clean_on_quit: bool,
    pub pin: String,
    pub expire_minutes: u32,
    pub enable_mdns: bool,
    pub network_interface: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterface {
    pub interface: String,
    pub address: String,
    pub url: String,
    pub private: bool,
    #[serde(rename = "virtual")]
    pub is_virtual: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReachabilityStatus {
    pub ok: bool,
    pub checked_at: String,
    pub status_code: u16,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub running: bool,
    pub port: u16,
    pub requested_port: u16,
    pub fallback_count: u16,
    pub primary_url: String,
    pub friendly_url: String,
    pub all_urls: Vec<String>,
    pub connected_devices: usize,
    pub snippet_count: usize,
    pub file_count: usize,
    pub uptime_seconds: u64,
    pub upload_dir: String,
    pub selected_interface: String,
    pub preferred_interface: String,
    pub preferred_found: bool,
    pub network_interfaces: Vec<NetworkInterface>,
    pub reachability: ReachabilityStatus,
    pub pin_enabled: bool,
}

pub struct ServerRuntime {
    state: Arc<ServerState>,
    port: u16,
    requested_port: u16,
    fallback_count: u16,
    primary_url: String,
    friendly_url: Option<String>,
    all_urls: Vec<String>,
    selected_interface: String,
    preferred_interface: String,
    preferred_found: bool,
    network_interfaces: Vec<NetworkInterface>,
    reachability: ReachabilityStatus,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
    auto_clean_on_quit: bool,
    mdns: Option<mdns_sd::ServiceDaemon>,
    sweeper: Option<JoinHandle<()>>,
}

impl ServerRuntime {
    /// Live feed of the same events the WebSocket clients receive
    /// (file:new, snippet:new, device:count, …) for desktop notifications.
    pub fn subscribe_events(&self) -> broadcast::Receiver<SocketEnvelope> {
        self.state.events_tx.subscribe()
    }

    pub fn snapshot(&self) -> RuntimeStatus {
        RuntimeStatus {
            running: true,
            port: self.port,
            requested_port: self.requested_port,
            fallback_count: self.fallback_count,
            primary_url: self.primary_url.clone(),
            friendly_url: self.friendly_url.clone().unwrap_or_default(),
            all_urls: self.all_urls.clone(),
            connected_devices: self.state.connected_devices.load(Ordering::SeqCst),
            snippet_count: self.state.snippet_len(),
            file_count: self.state.file_len(),
            uptime_seconds: self.state.uptime_seconds(),
            upload_dir: self.state.upload_dir.to_string_lossy().to_string(),
            selected_interface: self.selected_interface.clone(),
            preferred_interface: self.preferred_interface.clone(),
            preferred_found: self.preferred_found,
            network_interfaces: self.network_interfaces.clone(),
            reachability: self.reachability.clone(),
            pin_enabled: !self.state.pin.is_empty(),
        }
    }

    pub async fn create_invite_url(&self) -> String {
        let base_url = self
            .friendly_url
            .as_deref()
            .filter(|url| !url.is_empty())
            .unwrap_or(&self.primary_url);
        self.state
            .create_invite(base_url, &self.primary_url)
            .await
            .url
    }

    pub async fn debug_info(&self) -> String {
        let snapshot = self.snapshot();
        let mut lines = vec![
            format!("DropLocal {}", env!("CARGO_PKG_VERSION")),
            format!("Primary: {}", snapshot.primary_url),
            format!("Friendly: {}", snapshot.friendly_url),
            format!("Selected interface: {}", snapshot.selected_interface),
            format!("Preferred interface: {}", snapshot.preferred_interface),
            format!(
                "PIN enabled: {}",
                if self.state.pin.is_empty() {
                    "no"
                } else {
                    "yes"
                }
            ),
            format!(
                "Local check: {}",
                if snapshot.reachability.ok {
                    "ok"
                } else {
                    "failed"
                }
            ),
            format!("Upload dir: {}", snapshot.upload_dir),
        ];
        for entry in snapshot.network_interfaces {
            lines.push(format!(
                "Interface {}: {} {}",
                entry.interface,
                entry.address,
                if entry.selected { "[selected]" } else { "" }
            ));
        }
        lines.join("\n")
    }

    pub async fn drop_text(&self, raw_text: &str) -> anyhow::Result<()> {
        let text = raw_text.trim();
        if text.is_empty() {
            return Err(anyhow::anyhow!("Clipboard has no text to drop"));
        }
        let snippet = Snippet {
            id: Uuid::new_v4().to_string(),
            text: text.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            sender: None,
        };
        self.state.snippets.write().await.insert(0, snippet.clone());
        self.state.emit("snippet:new", json!(snippet));
        save_index_spawn(&self.state);
        Ok(())
    }

    pub async fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(daemon) = self.mdns.take() {
            let _ = daemon.shutdown();
        }

        if let Some(sweeper) = self.sweeper.take() {
            sweeper.abort();
        }

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }

        if self.auto_clean_on_quit {
            // Opt-in cleanup: wipe shared files, the index, and the folder.
            let files = self.state.take_files().await;
            for file in files {
                if let Err(error) = fs::remove_file(&file.path).await {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(error.into());
                    }
                }
            }
            let _ = fs::remove_file(self.state.upload_dir.join(INDEX_FILE_NAME)).await;
            fs::remove_dir_all(&self.state.upload_dir).await.ok();
        } else {
            // Persistent by default: flush the index so a restart restores
            // the stream.
            save_index(&self.state).await;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct ServerState {
    snippets: RwLock<Vec<Snippet>>,
    files: RwLock<Vec<StoredFile>>,
    events_tx: broadcast::Sender<SocketEnvelope>,
    connected_devices: AtomicUsize,
    started_at: Instant,
    upload_dir: PathBuf,
    primary_url: String,
    friendly_url: String,
    share_urls: Vec<String>,
    selected_interface: String,
    preferred_interface: String,
    preferred_found: bool,
    network_interfaces: Vec<NetworkInterface>,
    reachability: RwLock<ReachabilityStatus>,
    pin: String,
    session_token: String,
    invites: RwLock<HashMap<String, Instant>>,
    auth_failures: RwLock<HashMap<String, AuthFailure>>,
    expire_minutes: u32,
    devices: RwLock<std::collections::HashMap<String, DeviceInfo>>,
}

#[derive(Debug, Clone)]
struct AuthFailure {
    failures: u32,
    locked_until: Option<Instant>,
}

#[derive(Debug, Clone, Serialize)]
struct DeviceInfo {
    id: String,
    #[serde(rename = "clientId")]
    client_id: String,
    name: String,
}

fn sanitize_device_name(raw: &str) -> String {
    raw.chars()
        .filter(|ch| !ch.is_control())
        .take(32)
        .collect::<String>()
        .trim()
        .to_string()
}

impl ServerState {
    fn new(
        upload_dir: PathBuf,
        primary_url: String,
        friendly_url: String,
        share_urls: Vec<String>,
        selected_interface: String,
        preferred_interface: String,
        preferred_found: bool,
        network_interfaces: Vec<NetworkInterface>,
        reachability: ReachabilityStatus,
        pin: String,
        expire_minutes: u32,
    ) -> Self {
        let (events_tx, _events_rx) = broadcast::channel(120);

        Self {
            snippets: RwLock::new(Vec::new()),
            files: RwLock::new(Vec::new()),
            events_tx,
            connected_devices: AtomicUsize::new(0),
            started_at: Instant::now(),
            upload_dir,
            primary_url,
            friendly_url,
            share_urls,
            selected_interface,
            preferred_interface,
            preferred_found,
            network_interfaces,
            reachability: RwLock::new(reachability),
            pin,
            session_token: Uuid::new_v4().to_string(),
            invites: RwLock::new(HashMap::new()),
            auth_failures: RwLock::new(HashMap::new()),
            expire_minutes,
            devices: RwLock::new(std::collections::HashMap::new()),
        }
    }

    async fn emit_device_list(&self) {
        let devices: Vec<DeviceInfo> = self.devices.read().await.values().cloned().collect();
        self.emit(
            "device:list",
            json!({ "devices": serde_json::to_value(devices).unwrap_or(json!([])) }),
        );
    }

    fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    fn snippet_len(&self) -> usize {
        self.snippets
            .try_read()
            .map(|items| items.len())
            .unwrap_or(0)
    }

    fn file_len(&self) -> usize {
        self.files.try_read().map(|items| items.len()).unwrap_or(0)
    }

    async fn take_files(&self) -> Vec<StoredFile> {
        let mut files = self.files.write().await;
        std::mem::take(&mut *files)
    }

    fn emit(&self, event: &str, data: Value) {
        let _ = self.events_tx.send(SocketEnvelope {
            event: event.to_string(),
            data,
        });
    }

    fn device_count(&self) -> usize {
        self.connected_devices.load(Ordering::SeqCst)
    }

    fn auth_cookie(&self) -> String {
        format!(
            "{AUTH_COOKIE}={}; Path=/; HttpOnly; SameSite=Lax",
            self.session_token
        )
    }

    async fn auth_retry_after(&self, key: &str) -> Option<u64> {
        let failures = self.auth_failures.read().await;
        let record = failures.get(key)?;
        let locked_until = record.locked_until?;
        let now = Instant::now();
        if locked_until <= now {
            return None;
        }
        Some(locked_until.duration_since(now).as_secs().max(1))
    }

    async fn clear_auth_failures(&self, key: &str) {
        self.auth_failures.write().await.remove(key);
    }

    async fn record_auth_failure(&self, key: &str) -> Option<u64> {
        let mut failures = self.auth_failures.write().await;
        let current = failures.get(key).cloned().unwrap_or(AuthFailure {
            failures: 0,
            locked_until: None,
        });
        let count = current.failures.saturating_add(1);
        let delay = if count >= AUTH_FAILURE_LOCK_THRESHOLD {
            let exponent = count.saturating_sub(AUTH_FAILURE_LOCK_THRESHOLD);
            let factor = 2u64.saturating_pow(exponent);
            Some((AUTH_BACKOFF_BASE_SECS.saturating_mul(factor)).min(AUTH_BACKOFF_MAX_SECS))
        } else {
            None
        };
        failures.insert(
            key.to_string(),
            AuthFailure {
                failures: count,
                locked_until: delay.map(|seconds| Instant::now() + Duration::from_secs(seconds)),
            },
        );
        delay
    }

    async fn validate_invite(&self, token: &str) -> bool {
        let token = token.trim();
        if token.is_empty() {
            return false;
        }
        let now = Instant::now();
        let mut invites = self.invites.write().await;
        invites.retain(|_, expires_at| *expires_at > now);
        invites
            .get(token)
            .is_some_and(|expires_at| *expires_at > now)
    }

    async fn create_invite(&self, base_url: &str, fallback_base_url: &str) -> InvitePayload {
        let token = Uuid::new_v4().simple().to_string();
        let expires_at = Instant::now() + Duration::from_secs(INVITE_TTL_SECS);
        self.invites.write().await.insert(token.clone(), expires_at);
        InvitePayload {
            url: with_invite_token(base_url, &token),
            fallback_url: if fallback_base_url.is_empty() {
                String::new()
            } else {
                with_invite_token(fallback_base_url, &token)
            },
            token,
            expires_at: (Utc::now() + chrono::Duration::seconds(INVITE_TTL_SECS as i64))
                .to_rfc3339(),
            ttl_seconds: INVITE_TTL_SECS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Snippet {
    id: String,
    text: String,
    timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sender: Option<DropSender>,
}

#[derive(Debug, Deserialize)]
struct NewSnippet {
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DropSender {
    name: String,
    #[serde(rename = "clientId")]
    client_id: String,
}

#[derive(Debug, Deserialize)]
struct AuthPayload {
    #[serde(default)]
    pin: String,
    #[serde(default)]
    invite: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InvitePayload {
    url: String,
    fallback_url: String,
    token: String,
    expires_at: String,
    ttl_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct ZipQuery {
    #[serde(default)]
    ids: String,
}

#[derive(Debug, Deserialize)]
struct DownloadQuery {
    #[serde(default)]
    inline: String,
}

#[derive(Debug, Deserialize)]
struct CleanupQuery {
    #[serde(default = "default_cleanup_type")]
    r#type: String,
    #[serde(rename = "olderThanMinutes", default)]
    older_than_minutes: u64,
}

fn default_cleanup_type() -> String {
    "all".to_string()
}

struct ZipEntry {
    name: String,
    crc: u32,
    size: u32,
    dos_time: u16,
    dos_date: u16,
    offset: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileMeta {
    id: String,
    name: String,
    size: u64,
    timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sender: Option<DropSender>,
}

#[derive(Debug, Clone)]
struct StoredFile {
    meta: FileMeta,
    mime_type: String,
    path: PathBuf,
}

/// On-disk index entry, flat and camelCase so the Node CLI and the desktop
/// app can read each other's index when pointed at the same folder.
#[derive(Debug, Serialize, Deserialize)]
struct IndexFile {
    id: String,
    name: String,
    size: u64,
    timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sender: Option<DropSender>,
    #[serde(rename = "mimeType", default)]
    mime_type: String,
    path: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct PersistedIndex {
    #[serde(default)]
    snippets: Vec<Snippet>,
    #[serde(default)]
    files: Vec<IndexFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SocketEnvelope {
    pub event: String,
    pub data: Value,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": self.message
            })),
        )
            .into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

pub fn stopped_status() -> RuntimeStatus {
    RuntimeStatus {
        running: false,
        port: 0,
        requested_port: 0,
        fallback_count: 0,
        primary_url: String::new(),
        friendly_url: String::new(),
        all_urls: Vec::new(),
        connected_devices: 0,
        snippet_count: 0,
        file_count: 0,
        uptime_seconds: 0,
        upload_dir: String::new(),
        selected_interface: String::new(),
        preferred_interface: String::new(),
        preferred_found: true,
        network_interfaces: Vec::new(),
        reachability: ReachabilityStatus {
            ok: false,
            checked_at: String::new(),
            status_code: 0,
            error: String::new(),
        },
        pin_enabled: false,
    }
}

pub async fn start(config: ServerConfig) -> anyhow::Result<ServerRuntime> {
    fs::create_dir_all(&config.storage_dir).await?;

    let listener = bind_listener(config.requested_port).await?;
    let bound_port = listener.local_addr()?.port();
    let fallback_count = bound_port.saturating_sub(config.requested_port);

    let network_interfaces = build_network_interfaces(bound_port, &config.network_interface);
    let urls = build_share_urls(&network_interfaces, bound_port);
    let primary_url = urls
        .first()
        .cloned()
        .unwrap_or_else(|| format!("http://127.0.0.1:{bound_port}"));
    let selected_interface = network_interfaces
        .iter()
        .find(|entry| entry.selected)
        .map(|entry| entry.interface.clone())
        .unwrap_or_default();
    let preferred_interface = config.network_interface.trim().to_string();
    let preferred_found = preferred_interface.is_empty()
        || network_interfaces.iter().any(|entry| {
            entry.interface.eq_ignore_ascii_case(&preferred_interface)
                || entry.address.eq_ignore_ascii_case(&preferred_interface)
        });

    let mdns = if config.enable_mdns {
        register_mdns(bound_port)
    } else {
        None
    };
    let friendly_url = mdns.as_ref().map(|_| {
        if bound_port == 80 {
            format!("http://{MDNS_HOSTNAME}")
        } else {
            format!("http://{MDNS_HOSTNAME}:{bound_port}")
        }
    });

    let mut reachability = ReachabilityStatus {
        ok: false,
        checked_at: String::new(),
        status_code: 0,
        error: "not checked".to_string(),
    };

    let state = Arc::new(ServerState::new(
        config.storage_dir.clone(),
        primary_url.clone(),
        friendly_url.clone().unwrap_or_default(),
        urls.clone(),
        selected_interface.clone(),
        preferred_interface.clone(),
        preferred_found,
        network_interfaces.clone(),
        reachability.clone(),
        config.pin.trim().to_string(),
        config.expire_minutes,
    ));

    restore_index(&state).await;

    let router = Router::new()
        .route("/", get(index_html))
        .route("/api/auth", axum::routing::post(auth))
        .route("/favicon.svg", get(favicon_svg))
        .route("/favicon.ico", get(favicon_svg))
        .route("/apple-touch-icon.png", get(touch_icon_png))
        .route("/vendor/qrcode.js", get(qrcode_vendor_js))
        .route("/sw.js", get(service_worker_js))
        .route("/manifest.webmanifest", get(web_manifest))
        .route("/icons/icon-192.png", get(icon_192))
        .route("/icons/icon-512.png", get(icon_512))
        .route("/api/info", get(info))
        .route("/api/diagnostics", get(diagnostics))
        .route("/api/invites", axum::routing::post(create_invite))
        .route("/api/snippets", get(list_snippets).post(create_snippet))
        .route("/api/snippets/{id}", delete(delete_snippet))
        .route("/api/files", get(list_files).post(upload_files))
        .route("/api/files.zip", get(download_zip))
        .route("/api/files/{id}", get(download_file).delete(delete_file))
        .route("/api/drops", delete(clear_drops))
        .route("/api/status", get(status))
        .route("/ws", get(ws_upgrade))
        // Files stream to disk chunk-by-chunk; axum's 2 MB default body cap
        // would otherwise reject any real upload.
        .layer(DefaultBodyLimit::disable())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_auth,
        ))
        .with_state(state.clone());

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let server = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });

        if let Err(error) = server.await {
            eprintln!("droplocal desktop server error: {error}");
        }
    });

    let sweeper = if config.expire_minutes > 0 {
        let sweep_state = state.clone();
        Some(tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(EXPIRY_SWEEP_INTERVAL_SECS));
            loop {
                ticker.tick().await;
                sweep_expired(&sweep_state).await;
            }
        }))
    } else {
        None
    };

    tokio::time::sleep(Duration::from_millis(40)).await;
    reachability = check_local_reachability(bound_port).await;
    *state.reachability.write().await = reachability.clone();

    Ok(ServerRuntime {
        state,
        port: bound_port,
        requested_port: config.requested_port,
        fallback_count,
        primary_url,
        friendly_url,
        all_urls: urls,
        selected_interface,
        preferred_interface,
        preferred_found,
        network_interfaces,
        reachability,
        shutdown_tx: Some(shutdown_tx),
        task: Arc::new(tokio::sync::Mutex::new(Some(task))),
        auto_clean_on_quit: config.auto_clean_on_quit,
        mdns,
        sweeper,
    })
}

async fn require_auth(
    State(state): State<Arc<ServerState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if state.pin.is_empty() {
        return next.run(request).await;
    }

    let public = matches!(
        request.uri().path(),
        "/" | "/favicon.svg"
            | "/favicon.ico"
            | "/apple-touch-icon.png"
            | "/vendor/qrcode.js"
            | "/sw.js"
            | "/manifest.webmanifest"
            | "/icons/icon-192.png"
            | "/icons/icon-512.png"
            | "/api/auth"
    );
    if public {
        return next.run(request).await;
    }

    let expected = format!("{AUTH_COOKIE}={}", state.session_token);
    let invite = query_value(request.uri().query(), "invite");
    let authorized_by_invite = state.validate_invite(&invite).await;
    let authorized = authorized_by_invite
        || request
            .headers()
            .get("cookie")
            .and_then(|value| value.to_str().ok())
            .map(|cookies| cookies.split(';').any(|part| part.trim() == expected))
            .unwrap_or(false);

    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "PIN required" })),
        )
            .into_response()
    }
}

async fn auth(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<AuthPayload>,
) -> Response {
    if state.pin.is_empty() {
        return (StatusCode::OK, Json(json!({ "ok": true }))).into_response();
    }

    let key = addr.ip().to_string();
    let invite_valid = state.validate_invite(&payload.invite).await;
    if !invite_valid {
        if let Some(retry_after) = state.auth_retry_after(&key).await {
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "error": "Too many PIN attempts" })),
            )
                .into_response();
            if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
                response
                    .headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, value);
            }
            return response;
        }
    }

    if invite_valid
        || (!payload.pin.trim().is_empty() && constant_time_eq(payload.pin.trim(), &state.pin))
    {
        state.clear_auth_failures(&key).await;
        let cookie = state.auth_cookie();
        let mut response = (StatusCode::OK, Json(json!({ "ok": true }))).into_response();
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response
                .headers_mut()
                .insert(axum::http::header::SET_COOKIE, value);
        }
        response
    } else {
        if let Some(retry_after) = state.record_auth_failure(&key).await {
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "error": "Too many PIN attempts" })),
            )
                .into_response();
            if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
                response
                    .headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, value);
            }
            response
        } else {
            (StatusCode::FORBIDDEN, Json(json!({ "error": "Wrong PIN" }))).into_response()
        }
    }
}

async fn restore_index(state: &Arc<ServerState>) {
    let raw = match fs::read_to_string(state.upload_dir.join(INDEX_FILE_NAME)).await {
        Ok(raw) => raw,
        Err(_) => return,
    };
    let parsed: PersistedIndex = match serde_json::from_str(&raw) {
        Ok(parsed) => parsed,
        Err(_) => return,
    };

    *state.snippets.write().await = parsed.snippets;

    let mut restored = Vec::new();
    for entry in parsed.files {
        let path = PathBuf::from(&entry.path);
        if fs::metadata(&path).await.is_ok() {
            restored.push(StoredFile {
                meta: FileMeta {
                    id: entry.id,
                    name: entry.name,
                    size: entry.size,
                    timestamp: entry.timestamp,
                    sender: entry.sender,
                },
                mime_type: if entry.mime_type.is_empty() {
                    "application/octet-stream".to_string()
                } else {
                    entry.mime_type
                },
                path,
            });
        }
    }
    *state.files.write().await = restored;
}

async fn save_index(state: &Arc<ServerState>) {
    let snippets = state.snippets.read().await.clone();
    let files: Vec<IndexFile> = state
        .files
        .read()
        .await
        .iter()
        .map(|stored| IndexFile {
            id: stored.meta.id.clone(),
            name: stored.meta.name.clone(),
            size: stored.meta.size,
            timestamp: stored.meta.timestamp.clone(),
            sender: stored.meta.sender.clone(),
            mime_type: stored.mime_type.clone(),
            path: stored.path.to_string_lossy().to_string(),
        })
        .collect();

    if let Ok(payload) = serde_json::to_string(&PersistedIndex { snippets, files }) {
        let _ = fs::write(state.upload_dir.join(INDEX_FILE_NAME), payload).await;
    }
}

fn save_index_spawn(state: &Arc<ServerState>) {
    let state = state.clone();
    tokio::spawn(async move {
        save_index(&state).await;
    });
}

async fn sweep_expired(state: &Arc<ServerState>) {
    if state.expire_minutes == 0 {
        return;
    }
    let cutoff = Utc::now() - chrono::Duration::minutes(i64::from(state.expire_minutes));
    let is_expired = |timestamp: &str| {
        chrono::DateTime::parse_from_rfc3339(timestamp)
            .map(|parsed| parsed.with_timezone(&Utc) <= cutoff)
            .unwrap_or(false)
    };

    let removed_snippets: Vec<String> = {
        let mut snippets = state.snippets.write().await;
        let (expired, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut *snippets)
            .into_iter()
            .partition(|snippet| is_expired(&snippet.timestamp));
        *snippets = kept;
        expired.into_iter().map(|snippet| snippet.id).collect()
    };

    let removed_files: Vec<StoredFile> = {
        let mut files = state.files.write().await;
        let (expired, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut *files)
            .into_iter()
            .partition(|file| is_expired(&file.meta.timestamp));
        *files = kept;
        expired
    };

    for id in &removed_snippets {
        state.emit("snippet:delete", json!({ "id": id }));
    }
    for file in &removed_files {
        let _ = fs::remove_file(&file.path).await;
        state.emit("file:delete", json!({ "id": file.meta.id }));
    }

    if !removed_snippets.is_empty() || !removed_files.is_empty() {
        save_index(state).await;
    }
}

/// Best-effort mDNS registration so the server is reachable as
/// `http://drop.local`. Returns `None` when registration fails —
/// the IP URL keeps working either way.
fn register_mdns(port: u16) -> Option<mdns_sd::ServiceDaemon> {
    let daemon = mdns_sd::ServiceDaemon::new().ok()?;
    let properties: &[(&str, &str)] = &[];
    let info = mdns_sd::ServiceInfo::new(
        "_http._tcp.local.",
        "DropLocal",
        &format!("{MDNS_HOSTNAME}."),
        "",
        port,
        properties,
    )
    .ok()?
    .enable_addr_auto();

    daemon.register(info).ok()?;
    Some(daemon)
}

fn query_value(query: Option<&str>, key: &str) -> String {
    let Some(query) = query else {
        return String::new();
    };
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let Some(name) = parts.next() else {
            continue;
        };
        if name == key {
            return parts.next().unwrap_or("").to_string();
        }
    }
    String::new()
}

fn header_value(headers: &HeaderMap, name: &str, max_len: usize) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .chars()
        .filter(|ch| !ch.is_control())
        .take(max_len)
        .collect::<String>()
        .trim()
        .to_string()
}

fn sender_from_headers(headers: &HeaderMap) -> Option<DropSender> {
    let name = header_value(headers, "x-droplocal-device-name", 32);
    let client_id = header_value(headers, "x-droplocal-client-id", 32);
    if name.is_empty() && client_id.is_empty() {
        return None;
    }
    Some(DropSender {
        name: if name.is_empty() {
            "Device".to_string()
        } else {
            name
        },
        client_id,
    })
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let len = left.len().max(right.len()).max(1);
    let mut diff = left.len() ^ right.len();
    for index in 0..len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}

fn with_invite_token(base_url: &str, token: &str) -> String {
    let separator = if base_url.contains('?') { "&" } else { "?" };
    format!("{base_url}{separator}invite={token}")
}

async fn bind_listener(requested_port: u16) -> anyhow::Result<TcpListener> {
    if requested_port == 0 {
        // Auto mode: port 80 gives a portless URL (http://drop.local);
        // fall back to the classic 3000+ scan, then an ephemeral port.
        if let Ok(listener) =
            TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], AUTO_PORT_PRIMARY))).await
        {
            return Ok(listener);
        }

        for offset in 0..=MAX_PORT_RETRIES {
            let port = AUTO_PORT_FALLBACK + offset;
            if let Ok(listener) = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await {
                return Ok(listener);
            }
        }

        return Ok(TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await?);
    }

    for offset in 0..=MAX_PORT_RETRIES {
        let Some(port) = requested_port.checked_add(offset) else {
            break;
        };

        match TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await {
            Ok(listener) => return Ok(listener),
            Err(error)
                if error.kind() == std::io::ErrorKind::AddrInUse && offset < MAX_PORT_RETRIES =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        }
    }

    Err(anyhow::anyhow!(
        "unable to bind any port from {} to {}",
        requested_port,
        requested_port.saturating_add(MAX_PORT_RETRIES)
    ))
}

fn build_network_interfaces(port: u16, preferred_interface: &str) -> Vec<NetworkInterface> {
    let preferred = preferred_interface.trim().to_lowercase();
    let mut found = Vec::new();

    if let Ok(netifs) = local_ip_address::list_afinet_netifas() {
        // Score: real private LAN < virtual private (VPN/container) < public.
        let mut rows: Vec<(i16, String, NetworkInterface)> = netifs
            .into_iter()
            .filter_map(|(name, ip)| match ip {
                IpAddr::V4(v4) if !v4.is_loopback() => {
                    let address = v4.to_string();
                    let virtual_interface = is_virtual_interface(&name);
                    let matched = !preferred.is_empty()
                        && (name.to_lowercase() == preferred
                            || address.to_lowercase() == preferred);
                    let base_score = match (is_private_ipv4(v4), virtual_interface) {
                        (true, false) => 0,
                        (true, true) => 1,
                        (false, _) => 2,
                    };
                    Some((
                        base_score - if matched { 10 } else { 0 },
                        name.clone(),
                        NetworkInterface {
                            interface: name,
                            address: address.clone(),
                            url: format!("http://{address}:{port}"),
                            private: is_private_ipv4(v4),
                            is_virtual: virtual_interface,
                            selected: false,
                        },
                    ))
                }
                _ => None,
            })
            .collect();

        rows.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
        found.extend(rows.into_iter().map(|(_, _, row)| row));
    }

    if let Some(first) = found.first_mut() {
        first.selected = true;
    }

    found
}

fn build_share_urls(interfaces: &[NetworkInterface], port: u16) -> Vec<String> {
    if interfaces.is_empty() {
        return vec![format!("http://127.0.0.1:{port}")];
    }
    interfaces.iter().map(|entry| entry.url.clone()).collect()
}

async fn check_local_reachability(port: u16) -> ReachabilityStatus {
    let checked_at = Utc::now().to_rfc3339();
    match tokio::time::timeout(
        Duration::from_millis(800),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    {
        Ok(Ok(_stream)) => ReachabilityStatus {
            ok: true,
            checked_at,
            status_code: 0,
            error: String::new(),
        },
        Ok(Err(error)) => ReachabilityStatus {
            ok: false,
            checked_at,
            status_code: 0,
            error: error.to_string(),
        },
        Err(error) => ReachabilityStatus {
            ok: false,
            checked_at,
            status_code: 0,
            error: error.to_string(),
        },
    }
}

/// VPN tunnels, container bridges and link-local helpers advertise private
/// IPv4 addresses that peers on the real LAN cannot reach — keep them out of
/// the primary share URL.
fn is_virtual_interface(name: &str) -> bool {
    let lowered = name.to_lowercase();
    [
        "utun", "tun", "tap", "docker", "vmnet", "bridge", "br-", "zt", "awdl", "llw", "veth",
    ]
    .iter()
    .any(|prefix| lowered.starts_with(prefix))
}

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    match octets {
        [10, _, _, _] => true,
        [172, second, _, _] if (16..=31).contains(&second) => true,
        [192, 168, _, _] => true,
        _ => false,
    }
}

async fn index_html(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut response = (
        StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        EMBEDDED_UI,
    )
        .into_response();

    if !state.pin.is_empty() {
        if let Some(invite) = query.get("invite") {
            if state.validate_invite(invite).await {
                if let Ok(value) = HeaderValue::from_str(&state.auth_cookie()) {
                    response
                        .headers_mut()
                        .insert(axum::http::header::SET_COOKIE, value);
                }
            }
        }
    }

    response
}

async fn favicon_svg() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "image/svg+xml")],
        FAVICON_SVG,
    )
}

async fn touch_icon_png() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "image/png")],
        TOUCH_ICON_PNG,
    )
}

async fn qrcode_vendor_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/javascript; charset=utf-8")],
        QRCODE_VENDOR_JS,
    )
}

async fn service_worker_js() -> Response {
    let body = SERVICE_WORKER_TEMPLATE.replace("__DROPLOCAL_VERSION__", env!("CARGO_PKG_VERSION"));
    let mut response = (
        StatusCode::OK,
        [("content-type", "application/javascript; charset=utf-8")],
        body,
    )
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    response
        .headers_mut()
        .insert("service-worker-allowed", HeaderValue::from_static("/"));
    response
}

async fn info(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let friendly = if state.friendly_url.is_empty() {
        Value::Null
    } else {
        Value::String(state.friendly_url.clone())
    };

    Json(json!({
        "name": "DropLocal",
        "version": env!("CARGO_PKG_VERSION"),
        "urls": {
            "primary": state.primary_url,
            "friendly": friendly,
            "all": state.share_urls,
            "interfaces": state.network_interfaces
        }
    }))
}

async fn diagnostics(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let mut warnings = Vec::new();
    if !state.preferred_interface.is_empty() && !state.preferred_found {
        warnings.push(format!(
            "Preferred interface \"{}\" was not found.",
            state.preferred_interface
        ));
    }
    if let Some(selected) = state.network_interfaces.iter().find(|entry| entry.selected) {
        if selected.is_virtual {
            warnings.push(
                "The selected interface looks virtual/VPN-backed; phones may not reach it."
                    .to_string(),
            );
        }
    }
    if state.friendly_url.is_empty() {
        warnings
            .push("mDNS friendly address is unavailable; use the IP URL or QR code.".to_string());
    }
    let reachability = state.reachability.read().await.clone();

    Json(json!({
        "name": "DropLocal",
        "version": env!("CARGO_PKG_VERSION"),
        "running": true,
        "primaryUrl": state.primary_url,
        "friendlyUrl": state.friendly_url,
        "selectedInterface": state.selected_interface,
        "preferredInterface": state.preferred_interface,
        "preferredFound": state.preferred_found,
        "interfaces": state.network_interfaces,
        "mdns": {
            "enabled": true,
            "available": !state.friendly_url.is_empty(),
            "url": state.friendly_url,
        },
        "reachability": reachability,
        "pinEnabled": !state.pin.is_empty(),
        "inviteTtlSeconds": INVITE_TTL_SECS,
        "uploadDir": state.upload_dir.to_string_lossy(),
        "warnings": warnings
    }))
}

async fn create_invite(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let base_url = if state.friendly_url.is_empty() {
        state.primary_url.as_str()
    } else {
        state.friendly_url.as_str()
    };
    let invite = state.create_invite(base_url, &state.primary_url).await;
    (StatusCode::CREATED, Json(invite))
}

async fn web_manifest() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/manifest+json; charset=utf-8")],
        WEB_MANIFEST,
    )
}

async fn icon_192() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "image/png")],
        ICON_192_PNG,
    )
}

async fn icon_512() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "image/png")],
        ICON_512_PNG,
    )
}

async fn list_snippets(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let snippets = state.snippets.read().await.clone();
    Json(snippets)
}

async fn create_snippet(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(payload): Json<NewSnippet>,
) -> ApiResult<impl IntoResponse> {
    let text = payload.text.trim();
    if text.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Field \"text\" is required",
        ));
    }

    let snippet = Snippet {
        id: Uuid::new_v4().to_string(),
        text: text.to_string(),
        timestamp: Utc::now().to_rfc3339(),
        sender: sender_from_headers(&headers),
    };

    state.snippets.write().await.insert(0, snippet.clone());
    state.emit(
        "snippet:new",
        serde_json::to_value(&snippet).unwrap_or(json!({})),
    );
    save_index_spawn(&state);

    Ok((StatusCode::CREATED, Json(snippet)))
}

async fn delete_snippet(
    State(state): State<Arc<ServerState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<impl IntoResponse> {
    let mut snippets = state.snippets.write().await;
    if let Some(index) = snippets.iter().position(|entry| entry.id == id) {
        snippets.remove(index);
        drop(snippets);
        state.emit("snippet:delete", json!({ "id": id }));
        save_index_spawn(&state);
        Ok((StatusCode::OK, Json(json!({ "ok": true }))))
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "Snippet not found"))
    }
}

async fn list_files(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let files: Vec<FileMeta> = state
        .files
        .read()
        .await
        .iter()
        .map(|entry| entry.meta.clone())
        .collect();

    Json(files)
}

async fn upload_files(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<Response> {
    fs::create_dir_all(&state.upload_dir)
        .await
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let sender = sender_from_headers(&headers);
    let mut uploaded = Vec::new();

    loop {
        let next = multipart
            .next_field()
            .await
            .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, error.to_string()))?;

        let Some(mut field) = next else {
            break;
        };

        let raw_name = field.file_name().unwrap_or("file");
        let safe_name = sanitize_file_name(raw_name);
        let id = Uuid::new_v4().to_string();
        let target_path = state.upload_dir.join(format!("{id}-{safe_name}"));

        let mut output = fs::File::create(&target_path)
            .await
            .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

        let mut size: u64 = 0;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, error.to_string()))?
        {
            size = size.saturating_add(chunk.len() as u64);
            output.write_all(&chunk).await.map_err(|error| {
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            })?;
        }

        let file_meta = FileMeta {
            id: id.clone(),
            name: safe_name,
            size,
            timestamp: Utc::now().to_rfc3339(),
            sender: sender.clone(),
        };

        let mime_type = field
            .content_type()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        state.files.write().await.insert(
            0,
            StoredFile {
                meta: file_meta.clone(),
                mime_type,
                path: target_path,
            },
        );

        state.emit(
            "file:new",
            serde_json::to_value(&file_meta).unwrap_or(json!({})),
        );
        uploaded.push(file_meta);
    }

    if uploaded.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "No file found in upload payload",
        ));
    }

    save_index_spawn(&state);

    let body = if uploaded.len() == 1 {
        serde_json::to_value(&uploaded[0]).unwrap_or(json!({}))
    } else {
        serde_json::to_value(uploaded).unwrap_or(json!([]))
    };

    Ok((StatusCode::CREATED, Json(body)).into_response())
}

async fn download_file(
    State(state): State<Arc<ServerState>>,
    AxumPath(id): AxumPath<String>,
    axum::extract::Query(query): axum::extract::Query<DownloadQuery>,
) -> ApiResult<Response> {
    let stored = state
        .files
        .read()
        .await
        .iter()
        .find(|entry| entry.meta.id == id)
        .cloned()
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "File not found"))?;

    let file = fs::File::open(&stored.path)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => {
                ApiError::new(StatusCode::NOT_FOUND, "File missing on disk")
            }
            _ => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_str(&stored.mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    let disposition = if query.inline == "1" {
        HeaderValue::from_static("inline")
    } else {
        HeaderValue::from_str(&content_disposition(&stored.meta.name))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
    };
    headers.insert("content-disposition", disposition);

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok((StatusCode::OK, headers, body).into_response())
}

/* ---------- streaming zip (store method, data descriptors) ---------- */

fn crc32_update(crc: u32, chunk: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (n, slot) in table.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        table
    });

    let mut value = crc;
    for &byte in chunk {
        value = table[((value ^ u32::from(byte)) & 0xFF) as usize] ^ (value >> 8);
    }
    value
}

fn dos_date_time(timestamp: &str) -> (u16, u16) {
    use chrono::{Datelike, Timelike};
    let parsed = chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let dos_time = ((parsed.hour() as u16) << 11)
        | ((parsed.minute() as u16) << 5)
        | (parsed.second() as u16 / 2);
    let dos_date = (((parsed.year().max(1980) - 1980) as u16) << 9)
        | ((parsed.month() as u16) << 5)
        | parsed.day() as u16;
    (dos_time, dos_date)
}

fn zip_local_header(name: &str, dos_time: u16, dos_date: u16) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut header = Vec::with_capacity(30 + name_bytes.len());
    header.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    header.extend_from_slice(&20u16.to_le_bytes()); // version needed
    header.extend_from_slice(&0x0808u16.to_le_bytes()); // data descriptor + UTF-8
    header.extend_from_slice(&0u16.to_le_bytes()); // store
    header.extend_from_slice(&dos_time.to_le_bytes());
    header.extend_from_slice(&dos_date.to_le_bytes());
    header.extend_from_slice(&[0u8; 12]); // crc + sizes live in the descriptor
    header.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes()); // extra len
    header.extend_from_slice(name_bytes);
    header
}

fn zip_data_descriptor(crc: u32, size: u32) -> Vec<u8> {
    let mut descriptor = Vec::with_capacity(16);
    descriptor.extend_from_slice(&0x0807_4b50u32.to_le_bytes());
    descriptor.extend_from_slice(&crc.to_le_bytes());
    descriptor.extend_from_slice(&size.to_le_bytes());
    descriptor.extend_from_slice(&size.to_le_bytes());
    descriptor
}

fn zip_central_directory(entries: &[ZipEntry], offset: u32) -> Vec<u8> {
    let mut directory = Vec::new();
    for entry in entries {
        let name_bytes = entry.name.as_bytes();
        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes()); // version made by
        directory.extend_from_slice(&20u16.to_le_bytes()); // version needed
        directory.extend_from_slice(&0x0808u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes()); // store
        directory.extend_from_slice(&entry.dos_time.to_le_bytes());
        directory.extend_from_slice(&entry.dos_date.to_le_bytes());
        directory.extend_from_slice(&entry.crc.to_le_bytes());
        directory.extend_from_slice(&entry.size.to_le_bytes());
        directory.extend_from_slice(&entry.size.to_le_bytes());
        directory.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        directory.extend_from_slice(&[0u8; 12]); // extra/comment/disk/attrs
        directory.extend_from_slice(&entry.offset.to_le_bytes());
        directory.extend_from_slice(name_bytes);
    }

    let mut end = Vec::with_capacity(22);
    end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    end.extend_from_slice(&[0u8; 4]); // disk numbers
    end.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    end.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    end.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    end.extend_from_slice(&offset.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes()); // comment len

    directory.extend_from_slice(&end);
    directory
}

fn unique_zip_name(name: &str, used: &mut std::collections::HashSet<String>) -> String {
    let mut candidate = name.to_string();
    let mut counter = 1;
    while used.contains(&candidate) {
        candidate = match name.rfind('.') {
            Some(dot) if dot > 0 => format!("{} ({}){}", &name[..dot], counter, &name[dot..]),
            _ => format!("{name} ({counter})"),
        };
        counter += 1;
    }
    used.insert(candidate.clone());
    candidate
}

async fn write_zip(
    mut writer: tokio::io::DuplexStream,
    files: Vec<StoredFile>,
) -> std::io::Result<()> {
    use tokio::io::AsyncReadExt;

    let mut entries: Vec<ZipEntry> = Vec::new();
    let mut used_names = std::collections::HashSet::new();
    let mut offset: u32 = 0;

    for stored in files {
        let name = unique_zip_name(&stored.meta.name, &mut used_names);
        let (dos_time, dos_date) = dos_date_time(&stored.meta.timestamp);
        let header = zip_local_header(&name, dos_time, dos_date);
        writer.write_all(&header).await?;

        let mut file = fs::File::open(&stored.path).await?;
        let mut crc: u32 = 0xFFFF_FFFF;
        let mut size: u32 = 0;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            crc = crc32_update(crc, &buffer[..read]);
            size = size.saturating_add(read as u32);
            writer.write_all(&buffer[..read]).await?;
        }
        let crc = crc ^ 0xFFFF_FFFF;

        writer.write_all(&zip_data_descriptor(crc, size)).await?;
        entries.push(ZipEntry {
            name,
            crc,
            size,
            dos_time,
            dos_date,
            offset,
        });
        offset = offset
            .wrapping_add(header.len() as u32)
            .wrapping_add(size)
            .wrapping_add(16);
    }

    writer
        .write_all(&zip_central_directory(&entries, offset))
        .await?;
    writer.shutdown().await
}

async fn download_zip(
    State(state): State<Arc<ServerState>>,
    axum::extract::Query(query): axum::extract::Query<ZipQuery>,
) -> ApiResult<Response> {
    let requested: Vec<String> = query
        .ids
        .split(',')
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();

    let mut selected = Vec::new();
    {
        let files = state.files.read().await;
        for id in &requested {
            if let Some(stored) = files.iter().find(|entry| entry.meta.id == *id) {
                selected.push(stored.clone());
            }
        }
    }

    let mut checked = Vec::new();
    for stored in selected {
        match fs::metadata(&stored.path).await {
            Ok(meta) if meta.len() >= u64::from(u32::MAX) => {
                return Err(ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "File too large for zip (4 GB limit)",
                ));
            }
            Ok(_) => checked.push(stored),
            Err(_) => continue,
        }
    }

    if checked.is_empty() {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "No matching files"));
    }

    let file_count = checked.len();
    let (writer, reader) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let _ = write_zip(writer, checked).await;
    });

    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/zip"));
    headers.insert(
        "content-disposition",
        HeaderValue::from_str(&content_disposition(&format!(
            "droplocal-{file_count}-files.zip"
        )))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );

    let body = Body::from_stream(ReaderStream::new(reader));
    Ok((StatusCode::OK, headers, body).into_response())
}

async fn delete_file(
    State(state): State<Arc<ServerState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<impl IntoResponse> {
    let mut files = state.files.write().await;
    let Some(index) = files.iter().position(|entry| entry.meta.id == id) else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "File not found"));
    };

    let removed = files.remove(index);
    drop(files);

    if let Err(error) = fs::remove_file(&removed.path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                error.to_string(),
            ));
        }
    }

    state.emit("file:delete", json!({ "id": removed.meta.id }));
    save_index_spawn(&state);
    Ok((StatusCode::OK, Json(json!({ "ok": true }))))
}

async fn clear_drops(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<CleanupQuery>,
) -> ApiResult<impl IntoResponse> {
    let cleanup_type = query.r#type.trim();
    if !matches!(cleanup_type, "all" | "notes" | "snippets" | "files") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Invalid cleanup type",
        ));
    }

    let cutoff = if query.older_than_minutes > 0 {
        Some(Utc::now() - chrono::Duration::minutes(query.older_than_minutes as i64))
    } else {
        None
    };
    let should_remove = |timestamp: &str| {
        let Some(cutoff) = cutoff else {
            return true;
        };
        chrono::DateTime::parse_from_rfc3339(timestamp)
            .map(|parsed| parsed.with_timezone(&Utc) <= cutoff)
            .unwrap_or(false)
    };

    let removed_snippets: Vec<String> = if matches!(cleanup_type, "all" | "notes" | "snippets") {
        let mut snippets = state.snippets.write().await;
        let (removed, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut *snippets)
            .into_iter()
            .partition(|snippet| should_remove(&snippet.timestamp));
        *snippets = kept;
        removed.into_iter().map(|snippet| snippet.id).collect()
    } else {
        Vec::new()
    };

    let removed_files: Vec<StoredFile> = if matches!(cleanup_type, "all" | "files") {
        let mut files = state.files.write().await;
        let (removed, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut *files)
            .into_iter()
            .partition(|file| should_remove(&file.meta.timestamp));
        *files = kept;
        removed
    } else {
        Vec::new()
    };

    for id in &removed_snippets {
        state.emit("snippet:delete", json!({ "id": id }));
    }
    for file in &removed_files {
        let _ = fs::remove_file(&file.path).await;
        state.emit("file:delete", json!({ "id": file.meta.id }));
    }

    if !removed_snippets.is_empty() || !removed_files.is_empty() {
        save_index_spawn(&state);
    }

    Ok((
        StatusCode::OK,
        Json(json!({
            "deletedSnippets": removed_snippets.len(),
            "deletedFiles": removed_files.len()
        })),
    ))
}

async fn status(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let snippet_count = state.snippets.read().await.len();
    let file_count = state.files.read().await.len();

    Json(json!({
        "connectedDevices": state.device_count(),
        "uptimeSeconds": state.uptime_seconds(),
        "snippetCount": snippet_count,
        "fileCount": file_count
    }))
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<ServerState>) {
    let mut events_rx = state.events_tx.subscribe();
    let conn_id = Uuid::new_v4().to_string();
    let new_count = state.connected_devices.fetch_add(1, Ordering::SeqCst) + 1;
    state.devices.write().await.insert(
        conn_id.clone(),
        DeviceInfo {
            id: conn_id.clone(),
            client_id: String::new(),
            name: String::new(),
        },
    );
    state.emit("device:count", json!({ "count": new_count }));
    state.emit_device_list().await;

    let (mut sender, mut receiver) = socket.split();
    let initial = SocketEnvelope {
        event: "device:count".to_string(),
        data: json!({ "count": new_count }),
    };

    if let Ok(initial_payload) = serde_json::to_string(&initial) {
        if sender
            .send(Message::Text(initial_payload.into()))
            .await
            .is_err()
        {
            let after_disconnect = state.connected_devices.fetch_sub(1, Ordering::SeqCst) - 1;
            state.emit("device:count", json!({ "count": after_disconnect }));
            return;
        }
    }

    let mut send_task = tokio::spawn(async move {
        while let Ok(message) = events_rx.recv().await {
            let Ok(serialized) = serde_json::to_string(&message) else {
                continue;
            };

            if sender.send(Message::Text(serialized.into())).await.is_err() {
                break;
            }
        }
    });

    let recv_state = state.clone();
    let recv_conn_id = conn_id.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(message)) = receiver.next().await {
            match message {
                Message::Close(_) => break,
                Message::Text(text) => {
                    let Ok(parsed) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    if parsed.get("type").and_then(|v| v.as_str()) == Some("hello") {
                        {
                            let mut devices = recv_state.devices.write().await;
                            if let Some(info) = devices.get_mut(&recv_conn_id) {
                                info.name = sanitize_device_name(
                                    parsed.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                );
                                info.client_id = sanitize_device_name(
                                    parsed
                                        .get("clientId")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(""),
                                );
                            }
                        }
                        recv_state.emit_device_list().await;
                    }
                }
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => {
            recv_task.abort();
        }
        _ = &mut recv_task => {
            send_task.abort();
        }
    };

    let after_disconnect = state.connected_devices.fetch_sub(1, Ordering::SeqCst) - 1;
    state.devices.write().await.remove(&conn_id);
    state.emit("device:count", json!({ "count": after_disconnect }));
    state.emit_device_list().await;
}

fn sanitize_file_name(file_name: &str) -> String {
    let base = Path::new(file_name)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let cleaned = base
        .chars()
        .filter(|ch| !ch.is_control() && *ch != '/' && *ch != '\\')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

fn content_disposition(file_name: &str) -> String {
    let safe_ascii = file_name.replace('"', "");
    format!("attachment; filename=\"{safe_ascii}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn private_ip_ranges() {
        assert!(is_private_ipv4(Ipv4Addr::new(192, 168, 1, 22)));
        assert!(is_private_ipv4(Ipv4Addr::new(10, 0, 0, 6)));
        assert!(is_private_ipv4(Ipv4Addr::new(172, 18, 1, 10)));
        assert!(!is_private_ipv4(Ipv4Addr::new(8, 8, 8, 8)));
    }

    #[test]
    fn filename_sanitization() {
        assert_eq!(sanitize_file_name("../hello.txt"), "hello.txt");
        assert_eq!(sanitize_file_name(""), "file");
    }

    #[test]
    fn virtual_interfaces_are_detected() {
        assert!(is_virtual_interface("utun4"));
        assert!(is_virtual_interface("docker0"));
        assert!(!is_virtual_interface("en0"));
        assert!(!is_virtual_interface("wlan0"));
    }

    fn test_config(dir: &Path, pin: &str, auto_clean: bool) -> ServerConfig {
        ServerConfig {
            requested_port: 0,
            storage_dir: dir.to_path_buf(),
            auto_clean_on_quit: auto_clean,
            pin: pin.to_string(),
            expire_minutes: 0,
            enable_mdns: false,
            network_interface: String::new(),
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("droplocal-rs-{label}-{}", Uuid::new_v4()))
    }

    #[tokio::test]
    async fn http_api_lifecycle() {
        let dir = temp_dir("api");
        let mut runtime = start(test_config(&dir, "", true)).await.expect("start");
        let base = format!("http://127.0.0.1:{}", runtime.snapshot().port);
        let client = reqwest::Client::new();

        let info: Value = client
            .get(format!("{base}/api/info"))
            .send()
            .await
            .expect("info request")
            .json()
            .await
            .expect("info json");
        assert_eq!(info["name"], "DropLocal");
        assert!(info["urls"]["primary"]
            .as_str()
            .unwrap()
            .starts_with("http"));
        assert!(info["urls"]["interfaces"].as_array().is_some());

        let manifest: Value = client
            .get(format!("{base}/manifest.webmanifest"))
            .send()
            .await
            .expect("manifest request")
            .json()
            .await
            .expect("manifest json");
        assert_eq!(manifest["id"], "/");
        assert_eq!(manifest["scope"], "/");
        assert_eq!(manifest["display"], "standalone");

        let service_worker = client
            .get(format!("{base}/sw.js"))
            .send()
            .await
            .expect("service worker request");
        assert_eq!(service_worker.status(), reqwest::StatusCode::OK);
        assert_eq!(
            service_worker
                .headers()
                .get("service-worker-allowed")
                .unwrap(),
            "/"
        );
        let service_worker = service_worker.text().await.expect("service worker body");
        assert!(service_worker.contains("droplocal-shell-"));
        assert!(service_worker.contains("url.pathname.startsWith(\"/api/\")"));

        let diagnostics: Value = client
            .get(format!("{base}/api/diagnostics"))
            .send()
            .await
            .expect("diagnostics request")
            .json()
            .await
            .expect("diagnostics json");
        assert_eq!(diagnostics["name"], "DropLocal");
        assert_eq!(diagnostics["running"], true);

        let created: Value = client
            .post(format!("{base}/api/snippets"))
            .header("x-droplocal-device-name", "MacBook Pro")
            .header("x-droplocal-client-id", "client-test")
            .json(&json!({ "text": "hello from rust" }))
            .send()
            .await
            .expect("create snippet")
            .json()
            .await
            .expect("snippet json");
        assert_eq!(created["text"], "hello from rust");
        assert_eq!(created["sender"]["name"], "MacBook Pro");
        assert_eq!(created["sender"]["clientId"], "client-test");

        let part = reqwest::multipart::Part::bytes(b"rust upload body".to_vec())
            .file_name("note.txt")
            .mime_str("text/plain")
            .expect("part");
        let form = reqwest::multipart::Form::new().part("file", part);
        let uploaded: Value = client
            .post(format!("{base}/api/files"))
            .header("x-droplocal-device-name", "Pixel 7")
            .header("x-droplocal-client-id", "phone-client")
            .multipart(form)
            .send()
            .await
            .expect("upload")
            .json()
            .await
            .expect("upload json");
        assert_eq!(uploaded["name"], "note.txt");
        assert_eq!(uploaded["sender"]["name"], "Pixel 7");
        assert_eq!(uploaded["sender"]["clientId"], "phone-client");

        let downloaded = client
            .get(format!(
                "{base}/api/files/{}",
                uploaded["id"].as_str().unwrap()
            ))
            .send()
            .await
            .expect("download")
            .text()
            .await
            .expect("download body");
        assert_eq!(downloaded, "rust upload body");

        let deleted = client
            .delete(format!(
                "{base}/api/files/{}",
                uploaded["id"].as_str().unwrap()
            ))
            .send()
            .await
            .expect("delete file");
        assert_eq!(deleted.status(), reqwest::StatusCode::OK);

        runtime.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn bulk_cleanup_clears_selected_drop_types() {
        let dir = temp_dir("cleanup");
        let mut runtime = start(test_config(&dir, "", true)).await.expect("start");
        let base = format!("http://127.0.0.1:{}", runtime.snapshot().port);
        let client = reqwest::Client::new();

        let created = client
            .post(format!("{base}/api/snippets"))
            .json(&json!({ "text": "cleanup note" }))
            .send()
            .await
            .expect("create snippet");
        assert_eq!(created.status(), reqwest::StatusCode::CREATED);

        let part = reqwest::multipart::Part::bytes(b"cleanup body".to_vec())
            .file_name("cleanup.txt")
            .mime_str("text/plain")
            .expect("part");
        let uploaded: Value = client
            .post(format!("{base}/api/files"))
            .multipart(reqwest::multipart::Form::new().part("file", part))
            .send()
            .await
            .expect("upload")
            .json()
            .await
            .expect("upload json");

        let notes_only: Value = client
            .delete(format!("{base}/api/drops?type=notes"))
            .send()
            .await
            .expect("notes cleanup")
            .json()
            .await
            .expect("cleanup json");
        assert_eq!(notes_only["deletedSnippets"], 1);
        assert_eq!(notes_only["deletedFiles"], 0);

        let snippets: Value = client
            .get(format!("{base}/api/snippets"))
            .send()
            .await
            .expect("list snippets")
            .json()
            .await
            .expect("snippets json");
        assert_eq!(snippets.as_array().unwrap().len(), 0);

        let files: Value = client
            .get(format!("{base}/api/files"))
            .send()
            .await
            .expect("list files")
            .json()
            .await
            .expect("files json");
        assert_eq!(files.as_array().unwrap().len(), 1);
        assert_eq!(files[0]["id"], uploaded["id"]);

        let invalid = client
            .delete(format!("{base}/api/drops?type=unknown"))
            .send()
            .await
            .expect("invalid cleanup");
        assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);

        let all: Value = client
            .delete(format!("{base}/api/drops"))
            .send()
            .await
            .expect("all cleanup")
            .json()
            .await
            .expect("cleanup json");
        assert_eq!(all["deletedSnippets"], 0);
        assert_eq!(all["deletedFiles"], 1);

        let files: Value = client
            .get(format!("{base}/api/files"))
            .send()
            .await
            .expect("list files")
            .json()
            .await
            .expect("files json");
        assert_eq!(files.as_array().unwrap().len(), 0);

        runtime.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn zip_download_bundles_files() {
        let dir = temp_dir("zip");
        let mut runtime = start(test_config(&dir, "", true)).await.expect("start");
        let base = format!("http://127.0.0.1:{}", runtime.snapshot().port);
        let client = reqwest::Client::new();

        let mut ids = Vec::new();
        for (name, body) in [
            ("first.txt", "zip me first"),
            ("second.txt", "zip me second"),
        ] {
            let part = reqwest::multipart::Part::bytes(body.as_bytes().to_vec())
                .file_name(name)
                .mime_str("text/plain")
                .expect("part");
            let uploaded: Value = client
                .post(format!("{base}/api/files"))
                .multipart(reqwest::multipart::Form::new().part("file", part))
                .send()
                .await
                .expect("upload")
                .json()
                .await
                .expect("json");
            ids.push(uploaded["id"].as_str().unwrap().to_string());
        }

        let response = client
            .get(format!("{base}/api/files.zip?ids={}", ids.join(",")))
            .send()
            .await
            .expect("zip request");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/zip"
        );

        let bytes = response.bytes().await.expect("zip body");
        assert_eq!(&bytes[0..4], b"PK\x03\x04", "local header magic");
        assert!(
            bytes.windows(4).any(|window| window == b"PK\x05\x06"),
            "end of central directory present"
        );
        assert!(
            bytes.windows(9).any(|window| window == b"first.txt"),
            "contains first name"
        );

        let missing = client
            .get(format!("{base}/api/files.zip?ids=nope"))
            .send()
            .await
            .expect("missing request");
        assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);

        runtime.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn pin_protects_the_api() {
        let dir = temp_dir("pin");
        let mut runtime = start(test_config(&dir, "4471", true)).await.expect("start");
        let base = format!("http://127.0.0.1:{}", runtime.snapshot().port);
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .expect("client");

        let unauthorized = client
            .get(format!("{base}/api/snippets"))
            .send()
            .await
            .expect("unauthorized request");
        assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

        let ui = client.get(&base).send().await.expect("ui request");
        assert_eq!(
            ui.status(),
            reqwest::StatusCode::OK,
            "UI shell stays public"
        );

        let wrong = client
            .post(format!("{base}/api/auth"))
            .json(&json!({ "pin": "0000" }))
            .send()
            .await
            .expect("wrong pin");
        assert_eq!(wrong.status(), reqwest::StatusCode::FORBIDDEN);

        let right = client
            .post(format!("{base}/api/auth"))
            .json(&json!({ "pin": "4471" }))
            .send()
            .await
            .expect("right pin");
        assert_eq!(right.status(), reqwest::StatusCode::OK);

        let authorized = client
            .get(format!("{base}/api/snippets"))
            .send()
            .await
            .expect("authorized request");
        assert_eq!(authorized.status(), reqwest::StatusCode::OK);

        let invite = client
            .post(format!("{base}/api/invites"))
            .send()
            .await
            .expect("invite request");
        assert_eq!(invite.status(), reqwest::StatusCode::CREATED);
        let invite: Value = invite.json().await.expect("invite json");
        assert!(invite["url"].as_str().unwrap().contains("invite="));

        let fresh_client = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .expect("fresh client");
        let invite_auth = fresh_client
            .post(format!("{base}/api/auth"))
            .json(&json!({ "invite": invite["token"].as_str().unwrap() }))
            .send()
            .await
            .expect("invite auth");
        assert_eq!(invite_auth.status(), reqwest::StatusCode::OK);

        let invite_authorized = fresh_client
            .get(format!("{base}/api/snippets"))
            .send()
            .await
            .expect("invite authorized request");
        assert_eq!(invite_authorized.status(), reqwest::StatusCode::OK);

        for _ in 0..2 {
            let wrong = client
                .post(format!("{base}/api/auth"))
                .json(&json!({ "pin": "0000" }))
                .send()
                .await
                .expect("wrong pin");
            assert_eq!(wrong.status(), reqwest::StatusCode::FORBIDDEN);
        }

        let locked = client
            .post(format!("{base}/api/auth"))
            .json(&json!({ "pin": "0000" }))
            .send()
            .await
            .expect("locked pin");
        assert_eq!(locked.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
        assert!(locked.headers().get("retry-after").is_some());

        runtime.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn index_persists_across_restarts() {
        let dir = temp_dir("persist");
        let client = reqwest::Client::new();

        let mut first = start(test_config(&dir, "", false)).await.expect("start 1");
        let base = format!("http://127.0.0.1:{}", first.snapshot().port);
        client
            .post(format!("{base}/api/snippets"))
            .json(&json!({ "text": "survives restarts" }))
            .send()
            .await
            .expect("create");
        first.stop().await.expect("stop 1");

        let mut second = start(test_config(&dir, "", false)).await.expect("start 2");
        let base = format!("http://127.0.0.1:{}", second.snapshot().port);
        let listed: Value = client
            .get(format!("{base}/api/snippets"))
            .send()
            .await
            .expect("list")
            .json()
            .await
            .expect("list json");
        assert_eq!(listed[0]["text"], "survives restarts");
        second.stop().await.expect("stop 2");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
