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
    time::Duration,
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
        let url = format!("http://{host}:{port}/?token={token}");
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
                            let token = token.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = serve(&mut stream, &manager, &token).await;
                            });
                        }
                        Err(_) => break,
                    },
                }
            }
        });
        self.manager
            .logger
            .log("INFO", &format!("Phone console listening on {url}"));
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
    body: Vec<u8>,
}

async fn serve(stream: &mut TcpStream, manager: &Arc<Manager>, token: &str) -> std::io::Result<()> {
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
    let authorized = request
        .query
        .get("token")
        .map(|value| same_secret(value, token))
        .unwrap_or(false);
    if request.path == "/" || request.path == "/index.html" {
        return if authorized {
            respond(
                stream,
                "200 OK",
                "text/html; charset=utf-8",
                PAGE.as_bytes(),
            )
            .await
        } else {
            respond(
                stream,
                "401 Unauthorized",
                "text/plain; charset=utf-8",
                "This link is not valid. Scan the QR code in EasyLauncher again.".as_bytes(),
            )
            .await
        };
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
    if !authorized {
        return json_error(
            stream,
            "401 Unauthorized",
            "Invalid or expired access token.",
        )
        .await;
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
    let length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
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
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
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
    fn embedded_page_is_mobile_friendly() {
        assert!(PAGE.contains("viewport"));
        assert!(PAGE.contains("/api/state"));
        assert!(PAGE.contains("/api/action"));
        assert!(PAGE.contains("token"));
    }
}
