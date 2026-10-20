use crate::{
    error::{AppError, Result},
    models::ConsoleLine,
    server::manager::Manager,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{IpAddr, UdpSocket},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 64 * 1024;
const CONSOLE_LINES: usize = 160;
const PAGE: &str = include_str!("mobile.html");
const MANIFEST: &str = include_str!("mobile-manifest.json");
const WORKER: &str = include_str!("mobile-sw.js");
const ICON_192: &[u8] = include_bytes!("mobile-icon-192.png");
const ICON_512: &[u8] = include_bytes!("mobile-icon-512.png");
const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const SESSION_LIMIT: usize = 16;
const LOGIN_WINDOW: Duration = Duration::from_secs(60);
const LOGIN_FAILURES: u32 = 8;
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; manifest-src 'self'; worker-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebConsoleInfo {
    pub running: bool,
    pub url: String,
    pub token: String,
    pub port: u16,
}

struct Session {
    info: WebConsoleInfo,
    stop: watch::Sender<bool>,
}

struct SessionRecord {
    csrf: String,
    created: Instant,
}
struct LoginFailures {
    count: u32,
    window: Instant,
}
struct Auth {
    token: String,
    sessions: Mutex<HashMap<String, SessionRecord>>,
    failures: Mutex<LoginFailures>,
}
#[derive(Debug)]
enum LoginError {
    Bad,
    Rate,
}
impl Auth {
    fn new(token: String) -> Self {
        Self {
            token,
            sessions: Mutex::new(HashMap::new()),
            failures: Mutex::new(LoginFailures {
                count: 0,
                window: Instant::now(),
            }),
        }
    }
    fn login(&self, attempt: &str) -> std::result::Result<(String, String), LoginError> {
        {
            let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
            if failures.window.elapsed() >= LOGIN_WINDOW {
                failures.count = 0;
                failures.window = Instant::now();
            }
            if failures.count >= LOGIN_FAILURES {
                return Err(LoginError::Rate);
            }
        }
        if !same_secret(attempt, &self.token) {
            let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
            failures.count = failures.count.saturating_add(1);
            return Err(LoginError::Bad);
        }
        {
            let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
            failures.count = 0;
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let csrf = uuid::Uuid::new_v4().simple().to_string();
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|_, session| session.created.elapsed() < SESSION_TTL);
        if sessions.len() >= SESSION_LIMIT {
            let oldest = sessions
                .iter()
                .min_by_key(|(_, session)| session.created)
                .map(|(id, _)| id.clone());
            if let Some(oldest) = oldest {
                sessions.remove(&oldest);
            }
        }
        sessions.insert(
            id.clone(),
            SessionRecord {
                csrf: csrf.clone(),
                created: Instant::now(),
            },
        );
        Ok((id, csrf))
    }
    fn csrf(&self, id: &str) -> Option<String> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|_, session| session.created.elapsed() < SESSION_TTL);
        sessions.get(id).map(|session| session.csrf.clone())
    }
    fn validate(&self, id: &str, csrf: &str) -> bool {
        self.csrf(id)
            .map(|expected| same_secret(&expected, csrf))
            .unwrap_or(false)
    }
}

pub struct WebConsole {
    manager: Arc<Manager>,
    gate: tokio::sync::Mutex<()>,
    session: Mutex<Option<Session>>,
}
impl WebConsole {
    pub fn new(manager: Arc<Manager>) -> Self {
        Self {
            manager,
            gate: tokio::sync::Mutex::new(()),
            session: Mutex::new(None),
        }
    }
    pub fn info(&self) -> Option<WebConsoleInfo> {
        self.session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|s| s.info.clone())
    }
    pub async fn start(&self) -> Result<WebConsoleInfo> {
        let _g = self.gate.lock().await;
        if let Some(info) = self.info() {
            return Ok(info);
        }
        let listener = TcpListener::bind(("0.0.0.0", 0))
            .await
            .map_err(|e| AppError::new("Could not open the phone console port.", e))?;
        let port = listener
            .local_addr()
            .map_err(|e| AppError::new("Could not read the phone console address.", e))?
            .port();
        let token = uuid::Uuid::new_v4().simple().to_string();
        let host = local_ip().unwrap_or_else(|| "127.0.0.1".into());
        let url = format!("http://{host}:{port}/#token={token}");
        let auth = Arc::new(Auth::new(token.clone()));
        let (stop, mut stopped) = watch::channel(false);
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(Session {
            info: WebConsoleInfo {
                running: true,
                url: url.clone(),
                token: token.clone(),
                port,
            },
            stop,
        });
        let manager = self.manager.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    _ = stopped.changed() => break,
                    accepted = listener.accept() => match accepted {
                        Ok((mut stream, _)) => {
                            let manager = manager.clone();
                            let auth = auth.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = serve(&mut stream, &manager, &auth).await;
                            });
                        }
                        Err(_) => break,
                    },
                }
            }
        });
        self.manager.logger.log(
            "INFO",
            &format!("Phone console listening on http://{host}:{port}/"),
        );
        Ok(self.info().expect("session was just created"))
    }
    pub fn stop(&self) {
        if let Some(session) = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = session.stop.send(true);
            self.manager.logger.log("INFO", "Phone console stopped");
        }
    }
}

#[derive(Deserialize)]
struct ActionBody {
    id: String,
    action: String,
}
#[derive(Deserialize)]
struct CommandBody {
    id: String,
    command: String,
}
#[derive(Deserialize)]
struct SessionBody {
    token: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MobileServer {
    id: String,
    name: String,
    status: crate::models::Status,
    players: usize,
    port: u16,
    message: Option<String>,
}
#[derive(Serialize)]
struct MobileState {
    servers: Vec<MobileServer>,
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn serve(
    stream: &mut TcpStream,
    manager: &Arc<Manager>,
    auth: &Arc<Auth>,
) -> std::io::Result<()> {
    let request = match read_request(stream).await {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(()),
        Err(_) => {
            return respond(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                b"Bad request",
            )
            .await
        }
    };
    let asset = match request.path.as_str() {
        "/" | "/index.html" => Some((
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
            format!("Content-Security-Policy: {CSP}\r\n"),
        )),
        "/manifest.webmanifest" => Some((
            "application/manifest+json; charset=utf-8",
            MANIFEST.as_bytes(),
            String::new(),
        )),
        "/sw.js" => Some((
            "text/javascript; charset=utf-8",
            WORKER.as_bytes(),
            "Service-Worker-Allowed: /\r\n".to_string(),
        )),
        "/icon-192.png" | "/favicon.ico" => Some(("image/png", ICON_192, String::new())),
        "/icon-512.png" => Some(("image/png", ICON_512, String::new())),
        _ => None,
    };
    if let Some((content_type, body, extra)) = asset {
        if request.method != "GET" {
            return respond(
                stream,
                "405 Method Not Allowed",
                "text/plain; charset=utf-8",
                b"Method not allowed",
            )
            .await;
        }
        return respond_with(stream, "200 OK", content_type, body, &extra).await;
    }
    if !request.path.starts_with("/api/") {
        return respond(
            stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"Not found",
        )
        .await;
    }
    if request.path == "/api/session" {
        return match request.method.as_str() {
            "POST" => {
                let body = match body_json::<SessionBody>(&request.body) {
                    Ok(body) => body,
                    Err(e) => return json_error(stream, "400 Bad Request", &e).await,
                };
                match auth.login(&body.token) {
                    Ok((id, csrf)) => {
                        let cookie = format!(
                            "Set-Cookie: el_session={id}; Path=/; HttpOnly; SameSite=Strict; Max-Age=43200\r\n"
                        );
                        let body = serde_json::to_vec(&serde_json::json!({ "csrf": csrf }))
                            .unwrap_or_else(|_| b"{}".to_vec());
                        respond_with(
                            stream,
                            "200 OK",
                            "application/json; charset=utf-8",
                            &body,
                            &cookie,
                        )
                        .await
                    }
                    Err(LoginError::Rate) => respond_with(
                        stream,
                        "429 Too Many Requests",
                        "application/json; charset=utf-8",
                        br#"{"error":"Too many failed attempts. Wait a minute and try again."}"#,
                        "Retry-After: 60\r\n",
                    )
                    .await,
                    Err(LoginError::Bad) => {
                        json_error(
                            stream,
                            "401 Unauthorized",
                            "Invalid or expired access token.",
                        )
                        .await
                    }
                }
            }
            "GET" => match session_csrf(&request, auth) {
                Some(csrf) => json_ok(stream, &serde_json::json!({ "csrf": csrf })).await,
                None => {
                    json_error(
                        stream,
                        "401 Unauthorized",
                        "Sign in again by scanning the QR code in EasyLauncher.",
                    )
                    .await
                }
            },
            _ => {
                respond(
                    stream,
                    "405 Method Not Allowed",
                    "text/plain; charset=utf-8",
                    b"Method not allowed",
                )
                .await
            }
        };
    }
    let Some(csrf) = session_csrf(&request, auth) else {
        return json_error(
            stream,
            "401 Unauthorized",
            "Sign in again by scanning the QR code in EasyLauncher.",
        )
        .await;
    };
    if request.method != "GET" {
        let offered = request
            .headers
            .get("x-csrf-token")
            .map(String::as_str)
            .unwrap_or("");
        if !same_secret(offered, &csrf) {
            return json_error(stream, "403 Forbidden", "Missing or invalid CSRF token.").await;
        }
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/state") => {
            let servers = manager.list().unwrap_or_default();
            let payload = MobileState {
                servers: servers
                    .into_iter()
                    .map(|view| MobileServer {
                        id: view.server.id,
                        name: view.server.name,
                        status: view.runtime.status,
                        players: view.runtime.players.len(),
                        port: view.server.port,
                        message: view.runtime.message,
                    })
                    .collect(),
            };
            json_ok(stream, &payload).await
        }
        ("GET", "/api/console") => {
            let id = request.query.get("id").cloned().unwrap_or_default();
            let runtime = match manager.runtime(&id) {
                Ok(runtime) => runtime,
                Err(e) => return json_error(stream, "404 Not Found", &e.message).await,
            };
            let lines: Vec<ConsoleLine> = {
                let console = runtime.console.lock().unwrap_or_else(|e| e.into_inner());
                console
                    .iter()
                    .skip(console.len().saturating_sub(CONSOLE_LINES))
                    .cloned()
                    .collect()
            };
            json_ok(stream, &serde_json::json!({ "lines": lines })).await
        }
        ("POST", "/api/action") => {
            let body = match body_json::<ActionBody>(&request.body) {
                Ok(body) => body,
                Err(e) => return json_error(stream, "400 Bad Request", &e).await,
            };
            let result = match body.action.as_str() {
                "start" => manager.start(&body.id).await,
                "stop" => manager.stop(&body.id).await,
                "restart" => manager.restart(&body.id).await,
                _ => Err(AppError::user("Unknown action.")),
            };
            match result {
                Ok(()) => json_ok(stream, &serde_json::json!({ "ok": true })).await,
                Err(e) => json_error(stream, "409 Conflict", &e.message).await,
            }
        }
        ("POST", "/api/command") => {
            let body = match body_json::<CommandBody>(&request.body) {
                Ok(body) => body,
                Err(e) => return json_error(stream, "400 Bad Request", &e).await,
            };
            match manager.send(&body.id, &body.command).await {
                Ok(()) => json_ok(stream, &serde_json::json!({ "ok": true })).await,
                Err(e) => json_error(stream, "409 Conflict", &e.message).await,
            }
        }
        _ => {
            respond(
                stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"Not found",
            )
            .await
        }
    }
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let mut buffer = Vec::new();
    let head_end = loop {
        if buffer.len() > MAX_HEAD {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "headers too large",
            ));
        }
        let mut chunk = [0u8; 4096];
        let read = tokio::time::timeout(Duration::from_secs(15), stream.read(&mut chunk))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "read timeout"))??;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find(&buffer, b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let method = first.next().unwrap_or_default().to_uppercase();
    let target = first.next().unwrap_or("/");
    let (path, query) = split_target(target);
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "body too large",
        ));
    }
    let mut body = buffer[head_end..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0u8; length - body.len()];
        let read = tokio::time::timeout(Duration::from_secs(15), stream.read(&mut chunk))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "read timeout"))??;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Ok(Some(Request {
        method,
        path,
        query,
        headers,
        body,
    }))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn split_target(target: &str) -> (String, HashMap<String, String>) {
    let (path, raw) = target.split_once('?').unwrap_or((target, ""));
    let mut query = HashMap::new();
    for pair in raw.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(decode(key), decode(value));
    }
    (decode(path), query)
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn body_json<T: for<'de> Deserialize<'de>>(body: &[u8]) -> std::result::Result<T, String> {
    serde_json::from_slice(body).map_err(|_| "The request body is not valid JSON.".into())
}

fn same_secret(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn cookie(headers: &HashMap<String, String>, name: &str) -> Option<String> {
    headers
        .get("cookie")?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().to_string())
}

fn session_csrf(request: &Request, auth: &Auth) -> Option<String> {
    let id = cookie(&request.headers, "el_session")?;
    auth.csrf(&id)
}

fn local_ip() -> Option<String> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip.to_string()),
        _ => None,
    }
}

async fn json_ok<T: Serialize>(stream: &mut TcpStream, value: &T) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    respond(stream, "200 OK", "application/json; charset=utf-8", &body).await
}

async fn json_error(stream: &mut TcpStream, status: &str, message: &str) -> std::io::Result<()> {
    let body = serde_json::to_vec(&serde_json::json!({ "error": message }))
        .unwrap_or_else(|_| b"{}".to_vec());
    respond(stream, status, "application/json; charset=utf-8", &body).await
}

async fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    respond_with(stream, status, content_type, body, "").await
}

async fn respond_with(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    extra: &str,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nX-Frame-Options: DENY\r\nCross-Origin-Resource-Policy: same-origin\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_and_secret_parsing() {
        let (path, query) = split_target("/api/console?id=abc&token=a%2Bb");
        assert_eq!(path, "/api/console");
        assert_eq!(query.get("id").map(String::as_str), Some("abc"));
        assert_eq!(query.get("token").map(String::as_str), Some("a+b"));
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "ab"));
    }

    #[test]
    fn cookie_header_is_parsed_by_name() {
        let mut headers = HashMap::new();
        headers.insert(
            "cookie".to_string(),
            "theme=dark; el_session=abc123; other=1".to_string(),
        );
        assert_eq!(cookie(&headers, "el_session").as_deref(), Some("abc123"));
        assert_eq!(cookie(&headers, "EL_SESSION").as_deref(), Some("abc123"));
        assert_eq!(cookie(&headers, "missing"), None);
        assert_eq!(cookie(&HashMap::new(), "el_session"), None);
    }

    #[test]
    fn login_issues_matching_session_and_csrf() {
        let auth = Auth::new("secret-token".into());
        let (id, csrf) = auth.login("secret-token").expect("valid token logs in");
        assert!(!id.is_empty());
        assert!(!csrf.is_empty());
        assert_eq!(auth.csrf(&id).as_deref(), Some(csrf.as_str()));
        assert!(auth.validate(&id, &csrf));
        assert!(!auth.validate(&id, "wrong"));
        assert!(!auth.validate("unknown", &csrf));
        assert!(matches!(auth.login("wrong-token"), Err(LoginError::Bad)));
    }

    #[test]
    fn login_is_rate_limited_after_repeated_failures() {
        let auth = Auth::new("secret-token".into());
        for _ in 0..LOGIN_FAILURES {
            assert!(matches!(auth.login("nope"), Err(LoginError::Bad)));
        }
        assert!(matches!(auth.login("secret-token"), Err(LoginError::Rate)));
    }

    #[test]
    fn embedded_page_is_mobile_friendly() {
        assert!(PAGE.contains("viewport"));
        assert!(PAGE.contains("/api/state"));
        assert!(PAGE.contains("/api/action"));
        assert!(PAGE.contains("/api/session"));
        assert!(PAGE.contains("credentials"));
        assert!(PAGE.contains("X-CSRF-Token"));
        assert!(PAGE.contains("location.hash"));
        assert!(!PAGE.contains("token=\" + token"));
    }

    #[test]
    fn embedded_page_supports_install_and_notifications() {
        assert!(PAGE.contains("rel=\"manifest\""));
        assert!(PAGE.contains("manifest.webmanifest"));
        assert!(PAGE.contains("apple-touch-icon"));
        assert!(PAGE.contains("serviceWorker.register"));
        assert!(PAGE.contains("isSecureContext"));
        assert!(PAGE.contains("showNotification"));
        assert!(PAGE.contains("beforeinstallprompt"));
        assert!(PAGE.contains("粘贴从电脑复制的访问令牌"));
    }

    #[test]
    fn manifest_references_installable_icons() {
        let manifest: serde_json::Value =
            serde_json::from_str(MANIFEST).expect("manifest is valid JSON");
        assert_eq!(manifest["start_url"], "/");
        assert_eq!(manifest["scope"], "/");
        assert_eq!(manifest["display"], "standalone");
        assert_eq!(manifest["theme_color"], "#08120d");
        let icons = manifest["icons"].as_array().expect("icons array");
        let sizes: Vec<&str> = icons
            .iter()
            .filter_map(|icon| icon["sizes"].as_str())
            .collect();
        assert!(sizes.contains(&"192x192"));
        assert!(sizes.contains(&"512x512"));
        assert!(icons.iter().all(|icon| icon["src"]
            .as_str()
            .is_some_and(|src| src.starts_with('/') && src.ends_with(".png"))));
    }

    #[test]
    fn service_worker_keeps_live_data_out_of_the_cache() {
        assert!(WORKER.contains("\"/api/\""));
        assert!(WORKER.contains("caches.match"));
        assert!(WORKER.contains("navigate"));
        assert!(WORKER.contains("notificationclick"));
        assert!(WORKER.contains("skipWaiting"));
        assert!(!WORKER.contains("cache.put(event.request"));
    }

    #[test]
    fn embedded_icons_are_valid_png_files() {
        for (bytes, expected) in [(ICON_192, 192u32), (ICON_512, 512u32)] {
            assert!(bytes.len() > 1024);
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
            let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            assert_eq!(width, expected);
            assert_eq!(height, expected);
        }
    }

    #[test]
    fn csp_allows_manifest_and_worker_assets() {
        assert!(CSP.contains("manifest-src 'self'"));
        assert!(CSP.contains("worker-src 'self'"));
        assert!(CSP.contains("default-src 'none'"));
    }
}
