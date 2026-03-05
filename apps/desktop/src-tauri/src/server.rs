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
        Multipart, Path as AxumPath, State,
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
const EMBEDDED_UI: &str = include_str!("../../../../ui.html");

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub requested_port: u16,
    pub storage_dir: PathBuf,
    pub auto_clean_on_quit: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub running: bool,
    pub port: u16,
    pub requested_port: u16,
    pub fallback_count: u16,
    pub primary_url: String,
    pub all_urls: Vec<String>,
    pub connected_devices: usize,
    pub snippet_count: usize,
    pub file_count: usize,
    pub uptime_seconds: u64,
    pub upload_dir: String,
}

#[derive(Debug)]
pub struct ServerRuntime {
    state: Arc<ServerState>,
    port: u16,
    requested_port: u16,
    fallback_count: u16,
    primary_url: String,
    all_urls: Vec<String>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Arc<tokio::sync::Mutex<Option<JoinHandle<()>>>>,
    auto_clean_on_quit: bool,
}

impl ServerRuntime {
    pub fn snapshot(&self) -> RuntimeStatus {
        RuntimeStatus {
            running: true,
            port: self.port,
            requested_port: self.requested_port,
            fallback_count: self.fallback_count,
            primary_url: self.primary_url.clone(),
            all_urls: self.all_urls.clone(),
            connected_devices: self.state.connected_devices.load(Ordering::SeqCst),
            snippet_count: self.state.snippet_len(),
            file_count: self.state.file_len(),
            uptime_seconds: self.state.uptime_seconds(),
            upload_dir: self.state.upload_dir.to_string_lossy().to_string(),
        }
    }

    pub async fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }

        let files = self.state.take_files().await;
        for file in files {
            if let Err(error) = fs::remove_file(&file.path).await {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error.into());
                }
            }
        }

        if self.auto_clean_on_quit {
            fs::remove_dir_all(&self.state.upload_dir).await.ok();
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
}

impl ServerState {
    fn new(upload_dir: PathBuf) -> Self {
        let (events_tx, _events_rx) = broadcast::channel(120);

        Self {
            snippets: RwLock::new(Vec::new()),
            files: RwLock::new(Vec::new()),
            events_tx,
            connected_devices: AtomicUsize::new(0),
            started_at: Instant::now(),
            upload_dir,
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

#[derive(Debug, Clone, Serialize)]
struct Snippet {
    id: String,
    text: String,
    timestamp: String,
}

#[derive(Debug, Deserialize)]
struct NewSnippet {
    text: String,
}

#[derive(Debug, Clone, Serialize)]
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

    let state = Arc::new(ServerState::new(config.storage_dir.clone()));
    let router = Router::new()
        .route("/", get(index_html))
        .route("/api/snippets", get(list_snippets).post(create_snippet))
        .route("/api/snippets/{id}", delete(delete_snippet))
        .route("/api/files", get(list_files).post(upload_files))
        .route("/api/files/{id}", get(download_file).delete(delete_file))
        .route("/api/status", get(status))
        .route("/ws", get(ws_upgrade))
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

    Ok(ServerRuntime {
        state,
        port: bound_port,
        requested_port: config.requested_port,
        fallback_count,
        primary_url,
        all_urls: urls,
        shutdown_tx: Some(shutdown_tx),
        task: Arc::new(tokio::sync::Mutex::new(Some(task))),
        auto_clean_on_quit: config.auto_clean_on_quit,
    })
}

async fn bind_listener(requested_port: u16) -> anyhow::Result<TcpListener> {
    if requested_port == 0 {
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
        let mut urls: Vec<(bool, String)> = netifs
            .into_iter()
            .filter_map(|(_name, ip)| match ip {
                IpAddr::V4(v4) if !v4.is_loopback() => {
                    Some((is_private_ipv4(v4), format!("http://{v4}:{port}")))
                }
                _ => None,
            })
            .collect();

        urls.sort_by(|left, right| right.0.cmp(&left.0));
        found.extend(urls.into_iter().map(|(_, url)| url));
    }

    if found.is_empty() {
        found.push(format!("http://127.0.0.1:{port}"));
    }

    found
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

    Ok((StatusCode::CREATED, Json(snippet)))
}

async fn delete_snippet(
    State(state): State<Arc<ServerState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<impl IntoResponse> {
    let mut snippets = state.snippets.write().await;
    if let Some(index) = snippets.iter().position(|entry| entry.id == id) {
        snippets.remove(index);
        state.emit("snippet:delete", json!({ "id": id }));
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
    use super::{is_private_ipv4, sanitize_file_name};
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
}
