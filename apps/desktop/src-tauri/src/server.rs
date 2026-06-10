use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, Multipart, Path as AxumPath, State,
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
const MDNS_HOSTNAME: &str = "droplocal.local";
const AUTH_COOKIE: &str = "droplocal_auth";
const INDEX_FILE_NAME: &str = ".droplocal.json";
const EXPIRY_SWEEP_INTERVAL_SECS: u64 = 30;
const EMBEDDED_UI: &str = include_str!("../../../../ui.html");
const FAVICON_SVG: &str = include_str!("../../../../assets/brand/logo.svg");
const TOUCH_ICON_PNG: &[u8] = include_bytes!("../../../../assets/brand/apple-touch-icon.png");
const QRCODE_VENDOR_JS: &str = include_str!("../../../../assets/vendor/qrcode.js");
const ICON_192_PNG: &[u8] = include_bytes!("../../../../assets/brand/icon-192.png");
const ICON_512_PNG: &[u8] = include_bytes!("../../../../assets/brand/icon-512.png");
const WEB_MANIFEST: &str = r##"{"name":"DropLocal","short_name":"DropLocal","description":"Drop it local. Pick it up anywhere.","start_url":"/","display":"standalone","background_color":"#F5F7FB","theme_color":"#4F6BF5","icons":[{"src":"/icons/icon-192.png","sizes":"192x192","type":"image/png"},{"src":"/icons/icon-512.png","sizes":"512x512","type":"image/png"}]}"##;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub requested_port: u16,
    pub storage_dir: PathBuf,
    pub auto_clean_on_quit: bool,
    pub pin: String,
    pub expire_minutes: u32,
    pub enable_mdns: bool,
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
}

pub struct ServerRuntime {
    state: Arc<ServerState>,
    port: u16,
    requested_port: u16,
    fallback_count: u16,
    primary_url: String,
    friendly_url: Option<String>,
    all_urls: Vec<String>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
    auto_clean_on_quit: bool,
    mdns: Option<mdns_sd::ServiceDaemon>,
    sweeper: Option<JoinHandle<()>>,
}

impl ServerRuntime {
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
        }
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
    pin: String,
    session_token: String,
    expire_minutes: u32,
}

impl ServerState {
    fn new(
        upload_dir: PathBuf,
        primary_url: String,
        friendly_url: String,
        share_urls: Vec<String>,
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
            pin,
            session_token: Uuid::new_v4().to_string(),
            expire_minutes,
        }
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Snippet {
    id: String,
    text: String,
    timestamp: String,
}

#[derive(Debug, Deserialize)]
struct NewSnippet {
    text: String,
}

#[derive(Debug, Deserialize)]
struct AuthPayload {
    pin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileMeta {
    id: String,
    name: String,
    size: u64,
    timestamp: String,
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
struct SocketEnvelope {
    event: String,
    data: Value,
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
    }
}

pub async fn start(config: ServerConfig) -> anyhow::Result<ServerRuntime> {
    fs::create_dir_all(&config.storage_dir).await?;

    let listener = bind_listener(config.requested_port).await?;
    let bound_port = listener.local_addr()?.port();
    let fallback_count = bound_port.saturating_sub(config.requested_port);

    let urls = build_share_urls(bound_port);
    let primary_url = urls
        .first()
        .cloned()
        .unwrap_or_else(|| format!("http://127.0.0.1:{bound_port}"));

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

    let state = Arc::new(ServerState::new(
        config.storage_dir.clone(),
        primary_url.clone(),
        friendly_url.clone().unwrap_or_default(),
        urls.clone(),
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
        .route("/manifest.webmanifest", get(web_manifest))
        .route("/icons/icon-192.png", get(icon_192))
        .route("/icons/icon-512.png", get(icon_512))
        .route("/api/info", get(info))
        .route("/api/snippets", get(list_snippets).post(create_snippet))
        .route("/api/snippets/{id}", delete(delete_snippet))
        .route("/api/files", get(list_files).post(upload_files))
        .route("/api/files/{id}", get(download_file).delete(delete_file))
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
        let server =
            axum::serve(listener, router.into_make_service()).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });

        if let Err(error) = server.await {
            eprintln!("droplocal desktop server error: {error}");
        }
    });

    let sweeper = if config.expire_minutes > 0 {
        let sweep_state = state.clone();
        Some(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(
                EXPIRY_SWEEP_INTERVAL_SECS,
            ));
            loop {
                ticker.tick().await;
                sweep_expired(&sweep_state).await;
            }
        }))
    } else {
        None
    };

    Ok(ServerRuntime {
        state,
        port: bound_port,
        requested_port: config.requested_port,
        fallback_count,
        primary_url,
        friendly_url,
        all_urls: urls,
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
            | "/manifest.webmanifest"
            | "/icons/icon-192.png"
            | "/icons/icon-512.png"
            | "/api/auth"
    );
    if public {
        return next.run(request).await;
    }

    let expected = format!("{AUTH_COOKIE}={}", state.session_token);
    let authorized = request
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

async fn auth(State(state): State<Arc<ServerState>>, Json(payload): Json<AuthPayload>) -> Response {
    if state.pin.is_empty() {
        return (StatusCode::OK, Json(json!({ "ok": true }))).into_response();
    }

    if payload.pin.trim() == state.pin {
        let cookie = format!(
            "{AUTH_COOKIE}={}; Path=/; HttpOnly; SameSite=Lax",
            state.session_token
        );
        let mut response = (StatusCode::OK, Json(json!({ "ok": true }))).into_response();
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response
                .headers_mut()
                .insert(axum::http::header::SET_COOKIE, value);
        }
        response
    } else {
        (StatusCode::FORBIDDEN, Json(json!({ "error": "Wrong PIN" }))).into_response()
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
/// `http://droplocal.local`. Returns `None` when registration fails —
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

async fn bind_listener(requested_port: u16) -> anyhow::Result<TcpListener> {
    if requested_port == 0 {
        // Auto mode: port 80 gives a portless URL (http://droplocal.local);
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

fn build_share_urls(port: u16) -> Vec<String> {
    let mut found = Vec::new();

    if let Ok(netifs) = local_ip_address::list_afinet_netifas() {
        // Score: real private LAN < virtual private (VPN/container) < public.
        let mut urls: Vec<(u8, String, String)> = netifs
            .into_iter()
            .filter_map(|(name, ip)| match ip {
                IpAddr::V4(v4) if !v4.is_loopback() => {
                    let score = match (is_private_ipv4(v4), is_virtual_interface(&name)) {
                        (true, false) => 0,
                        (true, true) => 1,
                        (false, _) => 2,
                    };
                    Some((score, name, format!("http://{v4}:{port}")))
                }
                _ => None,
            })
            .collect();

        urls.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
        found.extend(urls.into_iter().map(|(_, _, url)| url));
    }

    if found.is_empty() {
        found.push(format!("http://127.0.0.1:{port}"));
    }

    found
}

/// VPN tunnels, container bridges and link-local helpers advertise private
/// IPv4 addresses that peers on the real LAN cannot reach — keep them out of
/// the primary share URL.
fn is_virtual_interface(name: &str) -> bool {
    let lowered = name.to_lowercase();
    ["utun", "tun", "tap", "docker", "vmnet", "bridge", "br-", "zt", "awdl", "llw", "veth"]
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

async fn index_html() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        EMBEDDED_UI,
    )
}

async fn favicon_svg() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "image/svg+xml")],
        FAVICON_SVG,
    )
}

async fn touch_icon_png() -> impl IntoResponse {
    (StatusCode::OK, [("content-type", "image/png")], TOUCH_ICON_PNG)
}

async fn qrcode_vendor_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/javascript; charset=utf-8")],
        QRCODE_VENDOR_JS,
    )
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
            "interfaces": []
        }
    }))
}

async fn web_manifest() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/manifest+json; charset=utf-8")],
        WEB_MANIFEST,
    )
}

async fn icon_192() -> impl IntoResponse {
    (StatusCode::OK, [("content-type", "image/png")], ICON_192_PNG)
}

async fn icon_512() -> impl IntoResponse {
    (StatusCode::OK, [("content-type", "image/png")], ICON_512_PNG)
}

async fn list_snippets(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let snippets = state.snippets.read().await.clone();
    Json(snippets)
}

async fn create_snippet(
    State(state): State<Arc<ServerState>>,
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
    mut multipart: Multipart,
) -> ApiResult<Response> {
    fs::create_dir_all(&state.upload_dir)
        .await
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

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
    headers.insert(
        "content-disposition",
        HeaderValue::from_str(&content_disposition(&stored.meta.name)).unwrap_or_else(|_| {
            HeaderValue::from_str("attachment").expect("static attachment header")
        }),
    );

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

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
    let new_count = state.connected_devices.fetch_add(1, Ordering::SeqCst) + 1;
    state.emit("device:count", json!({ "count": new_count }));

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

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(message)) = receiver.next().await {
            if matches!(message, Message::Close(_)) {
                break;
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
    state.emit("device:count", json!({ "count": after_disconnect }));
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
        assert!(info["urls"]["primary"].as_str().unwrap().starts_with("http"));

        let created: Value = client
            .post(format!("{base}/api/snippets"))
            .json(&json!({ "text": "hello from rust" }))
            .send()
            .await
            .expect("create snippet")
            .json()
            .await
            .expect("snippet json");
        assert_eq!(created["text"], "hello from rust");

        let part = reqwest::multipart::Part::bytes(b"rust upload body".to_vec())
            .file_name("note.txt")
            .mime_str("text/plain")
            .expect("part");
        let form = reqwest::multipart::Form::new().part("file", part);
        let uploaded: Value = client
            .post(format!("{base}/api/files"))
            .multipart(form)
            .send()
            .await
            .expect("upload")
            .json()
            .await
            .expect("upload json");
        assert_eq!(uploaded["name"], "note.txt");

        let downloaded = client
            .get(format!("{base}/api/files/{}", uploaded["id"].as_str().unwrap()))
            .send()
            .await
            .expect("download")
            .text()
            .await
            .expect("download body");
        assert_eq!(downloaded, "rust upload body");

        let deleted = client
            .delete(format!("{base}/api/files/{}", uploaded["id"].as_str().unwrap()))
            .send()
            .await
            .expect("delete file");
        assert_eq!(deleted.status(), reqwest::StatusCode::OK);

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
        assert_eq!(ui.status(), reqwest::StatusCode::OK, "UI shell stays public");

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
