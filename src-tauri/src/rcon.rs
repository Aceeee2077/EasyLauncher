//! Local RCON: a narrow, opt-in control channel to the managed Java process.
//!
//! Every connection is made to `127.0.0.1`; the password stays in the server's
//! own `server.properties`, exactly where Minecraft expects it. Enabling RCON
//! necessarily makes Minecraft listen on all interfaces, so the UI warns about
//! that and EasyLauncher never enables it without an explicit user action.
use crate::{
    error::{AppError, Result},
    models::Status,
    server::manager::Manager,
    storage,
};
use std::{net::Ipv4Addr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

const HOST: Ipv4Addr = Ipv4Addr::LOCALHOST;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PACKET: usize = 8 * 1024 * 1024;
const MAX_COMMAND: usize = 4096;
const DEFAULT_PORT: u16 = 25575;
const MIN_PORT: u16 = 1024;
const MIN_PASSWORD: usize = 12;
const PACKET_AUTH: i32 = 3;
const PACKET_AUTH_RESPONSE: i32 = 2;
const PACKET_EXECUTE: i32 = 2;
const PACKET_RESPONSE: i32 = 0;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RconInfo {
    pub enabled: bool,
    pub port: u16,
    pub password: Option<String>,
    /// True when the current Java process is running, so RCON can be queried now.
    pub reachable: bool,
}

/// The on-disk configuration, independent of whether Java is running.
fn config(m: &Manager, id: &str) -> Result<RconInfo> {
    let p = m.properties(id)?;
    let enabled = p
        .values
        .get("enable-rcon")
        .map(|v| v == "true")
        .unwrap_or(false);
    let port = p
        .values
        .get("rcon.port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let password = p
        .values
        .get("rcon.password")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let reachable = m
        .runtime(id)
        .map(|r| r.snapshot().status == Status::Running)
        .unwrap_or(false);
    Ok(RconInfo {
        enabled,
        port,
        password,
        reachable,
    })
}

pub fn info(m: &Manager, id: &str) -> Result<RconInfo> {
    config(m, id)
}

/// Enable RCON and generate a strong password when none is stored yet.
pub async fn enable(m: &Arc<Manager>, id: &str, port: Option<u16>) -> Result<RconInfo> {
    m.ready()?;
    let r = m.runtime(id)?;
    let _gate = r.gate.lock().await;
    r.idle()?;
    let mut p = m.properties(id)?;
    let server_port = p
        .values
        .get("server-port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(m.metadata(id)?.port);
    let port = port.unwrap_or_else(|| suggest_port(server_port));
    validate_port(port, server_port)?;
    if !crate::network::port_free(port) {
        return Err(AppError::user(format!(
            "RCON port {port} is already in use. Choose another port."
        ))
        .with_code("rcon.portInUse"));
    }
    let password = p
        .values
        .get("rcon.password")
        .map(|v| v.trim().to_string())
        .filter(|v| v.chars().count() >= MIN_PASSWORD)
        .unwrap_or_else(generate_password);
    p.set("enable-rcon", "true");
    p.set("rcon.port", &port.to_string());
    p.set("rcon.password", &password);
    p.set("broadcast-rcon-to-ops", "false");
    p.validate()?;
    storage::atomic_write(
        &storage::child(&m.dir(id)?, "server.properties")?,
        p.serialize().as_bytes(),
    )?;
    m.logger.log(
        "INFO",
        &format!("Enabled local RCON for {id} on port {port}"),
    );
    config(m, id)
}

/// Disable RCON without deleting the stored port and password.
pub async fn disable(m: &Arc<Manager>, id: &str) -> Result<RconInfo> {
    m.ready()?;
    let r = m.runtime(id)?;
    let _gate = r.gate.lock().await;
    r.idle()?;
    let mut p = m.properties(id)?;
    p.set("enable-rcon", "false");
    p.validate()?;
    storage::atomic_write(
        &storage::child(&m.dir(id)?, "server.properties")?,
        p.serialize().as_bytes(),
    )?;
    m.logger
        .log("INFO", &format!("Disabled local RCON for {id}"));
    config(m, id)
}

/// Execute one command through RCON. The frontend already has console access,
/// so this deliberately allows the same command surface.
pub async fn execute(m: &Arc<Manager>, id: &str, command: &str) -> Result<String> {
    let command = validate_command(command)?;
    let output = run(m, id, command).await?;
    if let Ok(r) = m.runtime(id) {
        m.line(&r, "rcon", &format!("> {command}"));
        for line in output.lines().filter(|line| !line.trim().is_empty()) {
            m.line(&r, "rcon", line);
        }
    }
    Ok(output)
}

/// Authoritative online player list, replacing log-observed state on success.
pub async fn players(m: &Arc<Manager>, id: &str) -> Result<Vec<String>> {
    let output = run(m, id, "list").await?;
    let names = parse_player_list(&output).ok_or_else(|| {
        AppError::user(
            "The server returned an unrecognized player list. Check that the server is ready and try again.",
        )
        .with_code("rcon.playerList")
    })?;
    if let Ok(r) = m.runtime(id) {
        r.snapshot.lock().unwrap_or_else(|e| e.into_inner()).players = names.clone();
        m.emit_status(&r);
    }
    Ok(names)
}

async fn run(m: &Manager, id: &str, command: &str) -> Result<String> {
    let cfg = config(m, id)?;
    if !cfg.enabled {
        return Err(AppError::user(
            "RCON is disabled for this server. Enable it in server settings, restart the server, then try again.",
        )
        .with_code("rcon.disabled"));
    }
    if !cfg.reachable {
        return Err(AppError::user(
            "Start the server and wait until it is ready before using RCON.",
        )
        .with_code("rcon.notRunning"));
    }
    let password = cfg.password.ok_or_else(|| {
        AppError::user("No RCON password is stored. Disable and re-enable RCON to generate one.")
            .with_code("rcon.noPassword")
    })?;
    raw(cfg.port, &password, command).await
}

fn suggest_port(server_port: u16) -> u16 {
    let next = server_port.saturating_add(1);
    if (MIN_PORT..=u16::MAX).contains(&next) && next != server_port {
        next
    } else {
        DEFAULT_PORT
    }
}

fn validate_port(port: u16, server_port: u16) -> Result<()> {
    if port < MIN_PORT {
        return Err(
            AppError::user("Choose an RCON port between 1024 and 65535.")
                .with_code("rcon.invalidPort"),
        );
    }
    if port == server_port {
        return Err(
            AppError::user("The RCON port must differ from the Minecraft server port.")
                .with_code("rcon.invalidPort"),
        );
    }
    Ok(())
}

fn generate_password() -> String {
    // 128 bits of hex; safe inside Java properties without escaping.
    uuid::Uuid::new_v4().simple().to_string()
}

fn validate_command(command: &str) -> Result<&str> {
    let command = command.trim();
    if command.is_empty() || command.len() > MAX_COMMAND || command.contains(['\n', '\r', '\0']) {
        return Err(
            AppError::user("Enter one RCON command, up to 4096 characters.")
                .with_code("rcon.invalidCommand"),
        );
    }
    Ok(command)
}

/// Parse the vanilla/Paper `list` response. Returns `None` for unknown formats
/// so callers never present a guessed player list as authoritative.
pub fn parse_player_list(output: &str) -> Option<Vec<String>> {
    let body = output.split_once("players online:")?.1;
    let names: Vec<String> = body
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| name.trim_matches('"').to_string())
        .filter(|name| crate::server::process::valid_player(name))
        .collect();
    Some(names)
}

/// One request/response exchange against a local RCON endpoint.
async fn raw(port: u16, password: &str, command: &str) -> Result<String> {
    let mut client = timeout(IO_TIMEOUT, RconClient::connect(port, password))
        .await
        .map_err(|_| {
            AppError::user("RCON did not respond in time. Check that the server is ready.")
                .with_code("rcon.timeout")
        })??;
    timeout(IO_TIMEOUT, client.command(command))
        .await
        .map_err(|_| {
            AppError::user("RCON did not respond in time. Try again once the server is ready.")
                .with_code("rcon.timeout")
        })?
}

struct RconClient {
    stream: TcpStream,
    next_id: i32,
}

impl RconClient {
    async fn connect(port: u16, password: &str) -> Result<Self> {
        let stream = TcpStream::connect((HOST, port)).await.map_err(|e| {
            AppError::new(
                "Could not connect to the local RCON port. Check that enable-rcon is true and the server has restarted.",
                e,
            )
            .with_code("rcon.unreachable")
        })?;
        let _ = stream.set_nodelay(true);
        let mut client = Self { stream, next_id: 1 };
        let id = client.take_id();
        client.write_packet(id, PACKET_AUTH, password).await?;
        // Some servers send an empty RESPONSE_VALUE before the auth response.
        for _ in 0..2 {
            let (reply_id, kind, _) = client.read_packet().await?;
            if kind == PACKET_AUTH_RESPONSE || reply_id == -1 {
                if reply_id == -1 {
                    return Err(AppError::user(
                        "RCON authentication failed. Disable and re-enable RCON to rotate the password.",
                    )
                    .with_code("rcon.authFailed"));
                }
                return Ok(client);
            }
        }
        Err(
            AppError::user("The RCON endpoint did not complete authentication.")
                .with_code("rcon.authFailed"),
        )
    }

    async fn command(&mut self, command: &str) -> Result<String> {
        let id = self.take_id();
        self.write_packet(id, PACKET_EXECUTE, command).await?;
        let (reply_id, kind, body) = self.read_packet().await?;
        if reply_id == -1 {
            return Err(AppError::user(
                "RCON rejected the command because the session is not authenticated.",
            )
            .with_code("rcon.authFailed"));
        }
        if kind != PACKET_RESPONSE {
            return Err(
                AppError::user("The RCON endpoint returned an unexpected packet.")
                    .with_code("rcon.protocol"),
            );
        }
        Ok(body)
    }

    fn take_id(&mut self) -> i32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    async fn write_packet(&mut self, id: i32, kind: i32, body: &str) -> Result<()> {
        let size = 4 + 4 + body.len() + 2;
        if size > MAX_PACKET {
            return Err(AppError::user("The RCON request is too large."));
        }
        let mut packet = Vec::with_capacity(size + 4);
        packet.extend_from_slice(&(size as i32).to_le_bytes());
        packet.extend_from_slice(&id.to_le_bytes());
        packet.extend_from_slice(&kind.to_le_bytes());
        packet.extend_from_slice(body.as_bytes());
        packet.push(0);
        packet.push(0);
        self.stream.write_all(&packet).await.map_err(|e| {
            AppError::new("Could not send the RCON request.", e).with_code("rcon.io")
        })?;
        self.stream
            .flush()
            .await
            .map_err(|e| AppError::new("Could not flush the RCON request.", e).with_code("rcon.io"))
    }

    async fn read_packet(&mut self) -> Result<(i32, i32, String)> {
        let mut size = [0_u8; 4];
        self.stream.read_exact(&mut size).await.map_err(|e| {
            AppError::new("Could not read the RCON response.", e).with_code("rcon.io")
        })?;
        let size = i32::from_le_bytes(size);
        if !(10..=MAX_PACKET as i32).contains(&size) {
            return Err(
                AppError::user("The RCON endpoint returned an invalid packet size.")
                    .with_code("rcon.protocol"),
            );
        }
        let mut payload = vec![0_u8; size as usize];
        self.stream.read_exact(&mut payload).await.map_err(|e| {
            AppError::new("The RCON response ended unexpectedly.", e).with_code("rcon.io")
        })?;
        let id = i32::from_le_bytes(payload[0..4].try_into().unwrap());
        let kind = i32::from_le_bytes(payload[4..8].try_into().unwrap());
        let end = payload.len().saturating_sub(2);
        let body = String::from_utf8_lossy(&payload[8..end]).into_owned();
        Ok((id, kind, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn player_list_formats() {
        assert_eq!(
            parse_player_list("There are 0 of a max of 20 players online: "),
            Some(vec![])
        );
        assert_eq!(
            parse_player_list("There are 2 of a max of 20 players online: Alex, Steve"),
            Some(vec!["Alex".into(), "Steve".into()])
        );
        assert_eq!(
            parse_player_list("There are 1 of a max of 20 players online: player_1"),
            Some(vec!["player_1".into()])
        );
        assert_eq!(parse_player_list("unknown response"), None);
    }
    #[test]
    fn commands_and_ports_are_validated() {
        assert!(validate_command("list").is_ok());
        assert!(validate_command("say hello").is_ok());
        assert!(validate_command("stop\nop Steve").is_err());
        assert!(validate_command("  ").is_err());
        assert!(validate_port(25575, 25565).is_ok());
        assert!(validate_port(25565, 25565).is_err());
        assert!(validate_port(80, 25565).is_err());
        assert_ne!(generate_password().len(), 0);
        assert_ne!(generate_password(), generate_password());
    }
    #[test]
    fn suggested_port_avoids_the_server_port() {
        assert_eq!(suggest_port(25565), 25566);
        assert_eq!(suggest_port(65535), DEFAULT_PORT);
        assert_eq!(suggest_port(25574), 25575);
    }

    /// Minimal Source RCON server used only to verify framing and auth handling.
    async fn fixture(password: &'static str) -> (u16, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind((HOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let mut commands = Vec::new();
            let (mut stream, _) = listener.accept().await.unwrap();
            for _ in 0..2 {
                let Some(size) = read_size(&mut stream).await else {
                    break;
                };
                let mut payload = vec![0_u8; size];
                stream.read_exact(&mut payload).await.unwrap();
                let id = i32::from_le_bytes(payload[0..4].try_into().unwrap());
                let kind = i32::from_le_bytes(payload[4..8].try_into().unwrap());
                let end = payload.len() - 2;
                let body = String::from_utf8_lossy(&payload[8..end]).into_owned();
                if kind == PACKET_AUTH {
                    let reply = if body == password { id } else { -1 };
                    write_fixture(&mut stream, reply, PACKET_AUTH_RESPONSE, "").await;
                } else {
                    commands.push(body.clone());
                    let text = if body == "list" {
                        "There are 1 of a max of 20 players online: Alex"
                    } else {
                        "ok"
                    };
                    write_fixture(&mut stream, id, PACKET_RESPONSE, text).await;
                }
            }
            commands
        });
        (port, handle)
    }
    async fn read_size(stream: &mut TcpStream) -> Option<usize> {
        let mut size = [0_u8; 4];
        stream.read_exact(&mut size).await.ok()?;
        Some(i32::from_le_bytes(size) as usize)
    }
    async fn write_fixture(stream: &mut TcpStream, id: i32, kind: i32, body: &str) {
        let size = 4 + 4 + body.len() + 2;
        let mut packet = Vec::new();
        packet.extend_from_slice(&(size as i32).to_le_bytes());
        packet.extend_from_slice(&id.to_le_bytes());
        packet.extend_from_slice(&kind.to_le_bytes());
        packet.extend_from_slice(body.as_bytes());
        packet.extend_from_slice(&[0, 0]);
        stream.write_all(&packet).await.unwrap();
    }
    #[tokio::test]
    async fn rcon_roundtrip_and_auth_failure() {
        let (port, server) = fixture("correct-horse-battery").await;
        let out = raw(port, "correct-horse-battery", "list").await.unwrap();
        assert_eq!(parse_player_list(&out), Some(vec!["Alex".to_string()]));
        let commands = server.await.unwrap();
        assert_eq!(commands, vec!["list"]);

        let (port, server) = fixture("correct-horse-battery").await;
        let err = raw(port, "wrong", "list").await.unwrap_err();
        assert_eq!(err.code, "rcon.authFailed");
        let _ = server.await;
    }
}
