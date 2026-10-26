//! Managed playit integration. Agent protocol/API: playit-cloud/playit-agent v0.17.1.
//! Secrets stay in DPAPI; only a child environment receives the decrypted key.
use crate::{
    error::{AppError, Result},
    models::Status,
    secrets,
    server::manager::Manager,
    storage,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    task::{JoinHandle, JoinSet},
};

const API: &str = "https://api.playit.gg";
const SECRET: &str = "playit.agent";
const DOWNLOAD: &str = "https://github.com/playit-cloud/playit-agent/releases/download/v0.17.1/playit-windows-x86_64-signed.exe";
const SHA256: &str = "9b00d6ff7d37d1052e5ae097e1348e11deae8617cd7a8ba39d1777f2006316a3";
#[derive(Default)]
pub struct Service {
    pub gate: tokio::sync::Mutex<()>,
    infos: Mutex<BTreeMap<String, Info>>,
    claim: Mutex<Option<String>>,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    flags: Mutex<BTreeMap<String, Arc<AtomicBool>>>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Config {
    servers: BTreeMap<String, Entry>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Entry {
    enabled: bool,
    tunnel_id: Option<String>,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub enabled: bool,
    pub authorized: bool,
    pub phase: String,
    pub address: Option<String>,
    pub claim_url: Option<String>,
    pub message: Option<String>,
}
fn config_path(m: &Manager) -> Result<PathBuf> {
    storage::child(&m.root, "playit.json")
}
fn load(m: &Manager) -> Result<Config> {
    let path = config_path(m)?;
    if !path.exists() {
        return Ok(Config::default());
    }
    if std::fs::metadata(&path)?.len() > 1024 * 1024 {
        return Err(AppError::user("Playit configuration is too large."));
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
pub fn info(m: &Manager, id: &str) -> Result<Info> {
    m.metadata(id)?;
    let enabled = load(m)?.servers.get(id).is_some_and(|e| e.enabled);
    let mut info = m
        .playit
        .infos
        .lock()
        .unwrap()
        .get(id)
        .cloned()
        .unwrap_or_default();
    info.enabled = enabled;
    info.authorized = secrets::load(m, SECRET)?.is_some();
    if !enabled {
        info.phase = "disabled".into();
        info.address = None;
    } else if info.phase.is_empty() {
        info.phase = "preparing".into();
    }
    if enabled && info.authorized && !is_live(m, id) && info.phase != "error" {
        info.phase = "ready".into();
        info.address = None;
    }
    info.claim_url = m
        .playit
        .claim
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| format!("https://playit.gg/claim/{c}"));
    Ok(info)
}
pub async fn enable(m: &Manager, id: &str, enabled: bool) -> Result<Info> {
    m.ready()?;
    m.metadata(id)?;
    if enabled && !(cfg!(target_os = "windows") && cfg!(target_arch = "x86_64")) {
        return Err(AppError::user(
            "Automatic playit currently supports Windows x64.",
        ));
    }
    let _gate = m.playit.gate.lock().await;
    let mut cfg = load(m)?;
    cfg.servers.entry(id.into()).or_default().enabled = enabled;
    storage::write_json(&config_path(m)?, &cfg)?;
    flag(m, id, enabled).store(enabled, Ordering::SeqCst);
    set(
        m,
        id,
        if enabled { "preparing" } else { "disabled" },
        None,
        None,
    );
    if !cfg.servers.values().any(|e| e.enabled) {
        *m.playit.claim.lock().unwrap() = None;
    }
    info(m, id)
}
fn flag(m: &Manager, id: &str, enabled: bool) -> Arc<AtomicBool> {
    m.playit
        .flags
        .lock()
        .unwrap()
        .entry(id.into())
        .or_insert_with(|| Arc::new(AtomicBool::new(enabled)))
        .clone()
}
fn is_live(m: &Manager, id: &str) -> bool {
    !m.shutting_down.load(Ordering::SeqCst)
        && m.runtime(id)
            .is_ok_and(|r| matches!(r.snapshot().status, Status::Starting | Status::Running))
}
pub fn open_claim(m: &Manager) -> Result<()> {
    let code = m
        .playit
        .claim
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| AppError::user("The authorization link is not ready. Please wait."))?;
    open::that(format!("https://playit.gg/claim/{code}"))
        .map_err(|_| AppError::user("Could not open the authorization page."))?;
    Ok(())
}
pub async fn reset_authorization(m: &Manager) -> Result<()> {
    m.ensure_writable()?;
    let _gate = m.playit.gate.lock().await;
    if m.runtimes
        .lock()
        .unwrap()
        .values()
        .any(|r| r.snapshot().status.active())
    {
        return Err(AppError::user("请先停止所有服务器，再重新授权 playit。"));
    }
    secrets::delete(m, SECRET)?;
    *m.playit.claim.lock().unwrap() = None;
    m.playit.infos.lock().unwrap().clear();
    Ok(())
}
fn set(m: &Manager, id: &str, phase: &str, address: Option<String>, message: Option<String>) {
    m.playit.infos.lock().unwrap().insert(
        id.into(),
        Info {
            phase: phase.into(),
            address,
            message,
            ..Info::default()
        },
    );
}
async fn api(
    client: &reqwest::Client,
    path: &str,
    key: Option<&str>,
    body: Value,
) -> Result<Value> {
    let mut request = client
        .post(format!("{API}{path}"))
        .json(&body)
        .timeout(Duration::from_secs(15));
    if let Some(key) = key {
        request = request.header("Authorization", format!("Agent-Key {key}"));
    }
    let mut response = request.send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err(AppError::user("Playit response exceeded the size limit."));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    decode(value)
}
fn decode(value: Value) -> Result<Value> {
    if value["status"] == "success" {
        return Ok(value["data"].clone());
    }
    // Never echo arbitrary provider responses: they may contain credentials.
    let reason = value["data"].as_str().unwrap_or("RequestRejected");
    let message = match reason {
        "RequiresVerifiedAccount" => "请先在 playit 官网完成邮箱验证，然后重试。",
        "RequiresPlayitPremium" | "RequiresPremium" => {
            "playit 账户当前不支持此线路，请检查账户套餐或额度。"
        }
        "CodeExpired" | "CodeNotFound" => "授权链接已过期，请关闭再开启自动联机以重新授权。",
        "UserRejected" => "你已拒绝授权，请关闭再开启自动联机以重试。",
        _ => "playit 暂时未接受请求，请检查账户状态、免费端口额度或稍后重试。",
    };
    Err(AppError::user(message))
}
async fn authorize(m: &Manager, client: &reqwest::Client) -> Result<Option<String>> {
    if let Some(key) = secrets::load(m, SECRET)? {
        return Ok(Some(key));
    }
    let code = {
        let mut claim = m.playit.claim.lock().unwrap();
        claim
            .get_or_insert_with(|| uuid::Uuid::new_v4().simple().to_string()[..10].to_string())
            .clone()
    };
    let status = api(
        client,
        "/claim/setup",
        None,
        json!({"code":code,"agent_type":"assignable","version":"EasyLauncher playit 0.17.1"}),
    )
    .await?;
    if status != "UserAccepted" {
        if status == "UserRejected" {
            return Err(AppError::user("授权已被拒绝，请关闭再开启自动联机以重试。"));
        }
        return Ok(None);
    }
    let result = api(client, "/claim/exchange", None, json!({"code":code})).await?;
    let key = result["secret_key"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::user("Playit did not return an agent key."))?;
    secrets::save(m, SECRET, key)?;
    *m.playit.claim.lock().unwrap() = None;
    Ok(Some(key.into()))
}
async fn executable(m: &Manager, client: &reqwest::Client) -> Result<PathBuf> {
    let dir = storage::child(&m.root, "playit-agent")?;
    std::fs::create_dir_all(&dir)?;
    let path = storage::child(&dir, "playit-0.17.1.exe")?;
    if path.exists() && std::fs::metadata(&path)?.len() <= 32 * 1024 * 1024 {
        if hex::encode(Sha256::digest(std::fs::read(&path)?)) == SHA256 {
            return Ok(path);
        }
    }
    let mut response = client
        .get(DOWNLOAD)
        .timeout(Duration::from_secs(180))
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 32 * 1024 * 1024 {
            return Err(AppError::user("Playit download exceeded the size limit."));
        }
        bytes.extend_from_slice(&chunk);
    }
    if hex::encode(Sha256::digest(&bytes)) != SHA256 {
        return Err(AppError::user(
            "Playit download checksum mismatch. Please retry.",
        ));
    }
    storage::atomic_write(&path, &bytes)?;
    Ok(path)
}
fn launch(m: &Manager, path: PathBuf, key: &str) -> Result<Child> {
    let mut command = Command::new(path);
    command
        .args(["--stdout", "start"])
        .env("PLAYIT_SECRET", key)
        .env_remove("PLAYIT_SECRET_PATH")
        .env_remove("API_BASE")
        .current_dir(storage::child(&m.root, "playit-agent")?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let child = command.spawn()?;
    m.process_group.attach(&child)?;
    Ok(child)
}

// Each active server gets a private loopback bridge. Dropping it closes all
// connections, even when a cloud API is unavailable or the agent serves other servers.
struct Bridge {
    port: u16,
    target: u16,
    task: JoinHandle<()>,
    next_check: Instant,
    endpoint: Option<(String, String, u16)>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}
/// UDP tunnels do not need the loopback TCP bridge: the agent can target the
/// server port directly. This only throttles the allocation API calls.
struct UdpCheck {
    next: Instant,
    endpoint: Option<(String, String, u16)>,
}
impl UdpCheck {
    fn new(now: Instant) -> Self {
        Self {
            next: now,
            endpoint: None,
        }
    }
}
async fn bridge(
    target: u16,
    guard: Option<(Arc<crate::server::manager::Runtime>, Arc<AtomicBool>)>,
) -> Result<Bridge> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        let mut heartbeat = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if guard.as_ref().is_some_and(|(runtime, enabled)| !enabled.load(Ordering::SeqCst) || !matches!(runtime.snapshot().status, Status::Starting | Status::Running)) { break; }
                }
                accepted = listener.accept() => {
                    let Ok((mut incoming, _)) = accepted else { break; };
                    if connections.len() >= 256 { continue; }
                    connections.spawn(async move {
                        if let Ok(Ok(mut game)) = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(("127.0.0.1", target))).await {
                            let _ = tokio::io::copy_bidirectional(&mut incoming, &mut game).await;
                        }
                    });
                }
                _ = connections.join_next(), if !connections.is_empty() => {}
            }
        }
    });
    Ok(Bridge {
        port,
        target,
        task,
        next_check: Instant::now(),
        endpoint: None,
    })
}
fn tunnel_name(id: &str, protocol: crate::public_access::Protocol) -> String {
    if protocol == crate::public_access::Protocol::Udp {
        format!("EasyLauncher-{id}-udp")
    } else {
        format!("EasyLauncher-{id}")
    }
}
fn create_body(
    id: &str,
    agent: &str,
    port: u16,
    protocol: crate::public_access::Protocol,
) -> Value {
    let udp = protocol == crate::public_access::Protocol::Udp;
    json!({
        "name": tunnel_name(id, protocol),
        "tunnel_type": if udp { "minecraft-bedrock" } else { "minecraft-java" },
        "port_type": protocol.as_str(),
        "port_count": 1,
        "origin": {"type":"agent","data":{"agent_id":agent,"local_ip":"127.0.0.1","local_port":port}},
        "enabled": true,
        "alloc": null,
        "firewall_id": null,
        "proxy_protocol": null
    })
}
fn allocated(tunnel: &Value) -> Option<(String, String, u16)> {
    if tunnel["alloc"]["status"] != "allocated" || tunnel["active"] != true {
        return None;
    }
    let alloc = &tunnel["alloc"]["data"];
    let host = alloc["ip_hostname"].as_str()?;
    if host.is_empty()
        || host.len() > 253
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
    {
        return None;
    }
    let port = u16::try_from(alloc["port_start"].as_u64()?)
        .ok()
        .filter(|p| *p > 0)?;
    // Explicit host:port works without depending on Minecraft SRV lookup.
    Some((format!("{host}:{port}"), host.into(), port))
}
async fn ensure_tunnel(
    client: &reqwest::Client,
    key: &str,
    id: &str,
    port: u16,
    protocol: crate::public_access::Protocol,
    entry: &mut Entry,
) -> Result<Option<(String, String, u16)>> {
    let agent = api(client, "/agents/rundata", Some(key), json!({})).await?;
    let agent_id = agent["agent_id"]
        .as_str()
        .ok_or_else(|| AppError::user("Playit agent identity missing."))?;
    let list = api(
        client,
        "/tunnels/list",
        Some(key),
        json!({"agent_id":agent_id,"tunnel_id":null}),
    )
    .await?;
    let tunnels = list["tunnels"]
        .as_array()
        .ok_or_else(|| AppError::user("Playit tunnel list missing."))?;
    let name = tunnel_name(id, protocol);
    let existing = tunnels
        .iter()
        .find(|t| {
            t["id"].as_str() == entry.tunnel_id.as_deref()
                && entry.tunnel_id.is_some()
                && t["port_type"].as_str() == Some(protocol.as_str())
        })
        .or_else(|| tunnels.iter().find(|t| t["name"].as_str() == Some(&name)));
    if let Some(t) = existing {
        let tunnel_id = t["id"]
            .as_str()
            .ok_or_else(|| AppError::user("Playit tunnel identity missing."))?;
        entry.tunnel_id = Some(tunnel_id.into());
        if t["origin"]["data"]["local_port"] != port || t["active"] != true {
            api(client, "/tunnels/update", Some(key), json!({"tunnel_id":tunnel_id,"local_ip":"127.0.0.1","local_port":port,"agent_id":agent_id,"enabled":true})).await?;
        }
        return Ok(allocated(t));
    }
    let created = api(
        client,
        "/tunnels/create",
        Some(key),
        create_body(id, agent_id, port, protocol),
    )
    .await?;
    entry.tunnel_id = Some(
        created["id"]
            .as_str()
            .ok_or_else(|| AppError::user("Playit did not return a tunnel ID."))?
            .into(),
    );
    Ok(None)
}
fn varint(mut n: u32, out: &mut Vec<u8>) {
    loop {
        let b = (n & 127) as u8;
        n >>= 7;
        out.push(if n == 0 { b } else { b | 128 });
        if n == 0 {
            break;
        }
    }
}
async fn read_varint(stream: &mut TcpStream) -> std::io::Result<usize> {
    let mut n = 0usize;
    for shift in (0..35).step_by(7) {
        let b = stream.read_u8().await?;
        n |= ((b & 127) as usize) << shift;
        if b & 128 == 0 {
            return Ok(n);
        }
    }
    Err(std::io::Error::other("Invalid Minecraft VarInt"))
}
pub(crate) async fn probe(host: &str, port: u16) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut stream = TcpStream::connect((host, port)).await.ok()?;
        let mut handshake = vec![0];
        varint(47, &mut handshake);
        varint(host.len() as u32, &mut handshake);
        handshake.extend_from_slice(host.as_bytes());
        handshake.extend_from_slice(&port.to_be_bytes());
        handshake.push(1);
        let mut packet = Vec::new();
        varint(handshake.len() as u32, &mut packet);
        packet.extend(handshake);
        packet.extend([1, 0]);
        stream.write_all(&packet).await.ok()?;
        let size = read_varint(&mut stream).await.ok()?;
        if !(3..=1024 * 1024).contains(&size) {
            return None;
        }
        let mut body = vec![0; size];
        stream.read_exact(&mut body).await.ok()?;
        valid_status(&body).then_some(())
    })
    .await
    .is_ok_and(|r| r.is_some())
}
fn valid_status(body: &[u8]) -> bool {
    if body.first() != Some(&0) {
        return false;
    }
    let mut len = 0usize;
    for (i, b) in body.iter().skip(1).take(5).enumerate() {
        len |= ((b & 127) as usize) << (i * 7);
        if b & 128 == 0 {
            let start = i + 2;
            return body
                .get(start..)
                .filter(|s| s.len() == len)
                .and_then(|s| serde_json::from_slice::<Value>(s).ok())
                .is_some_and(|v| v["version"].is_object() && v["players"].is_object());
        }
    }
    false
}

pub fn spawn(m: Arc<Manager>) {
    let owner = m.clone();
    let task = tauri::async_runtime::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent("EasyLauncher/0.2.2")
            .connect_timeout(Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut child: Option<Child> = None;
        let mut bridges: BTreeMap<String, Bridge> = BTreeMap::new();
        let mut udp: BTreeMap<String, UdpCheck> = BTreeMap::new();
        loop {
            let result = tick(&m, &client, &mut child, &mut bridges, &mut udp).await;
            if let Err(e) = &result {
                if let Ok(cfg) = load(&m) {
                    for (id, entry) in cfg.servers {
                        if entry.enabled {
                            set(&m, &id, "error", None, Some(e.message.clone()));
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(if result.is_err() { 15 } else { 3 })).await;
        }
    });
    *owner.playit.task.lock().unwrap() = Some(task);
}
async fn tick(
    m: &Manager,
    client: &reqwest::Client,
    child: &mut Option<Child>,
    bridges: &mut BTreeMap<String, Bridge>,
    udp: &mut BTreeMap<String, UdpCheck>,
) -> Result<()> {
    let mut cfg = {
        let _gate = m.playit.gate.lock().await;
        load(m)?
    };
    // Prune deleted servers without taking any runtime lock across an await.
    cfg.servers.retain(|id, _| m.metadata(id).is_ok());
    if m.settings()?.read_only {
        bridges.clear();
        if let Some(mut old) = child.take() {
            let _ = old.kill().await;
        }
        for (id, e) in &cfg.servers {
            if e.enabled {
                set(
                    m,
                    id,
                    "error",
                    None,
                    Some("只读模式下暂停自动联机。".into()),
                );
            }
        }
        return Ok(());
    }
    let live = |id: &str| {
        is_live(m, id)
            && flag(m, id, cfg.servers.get(id).is_some_and(|e| e.enabled)).load(Ordering::SeqCst)
    };
    bridges.retain(|id, _| cfg.servers.get(id).is_some_and(|e| e.enabled) && live(id));
    udp.retain(|id, _| cfg.servers.get(id).is_some_and(|e| e.enabled) && live(id));
    let any_live = cfg.servers.iter().any(|(id, e)| e.enabled && live(id));
    if !any_live {
        if let Some(mut old) = child.take() {
            let _ = old.kill().await;
        }
    }
    if !cfg.servers.values().any(|e| e.enabled) {
        *m.playit.claim.lock().unwrap() = None;
        return Ok(());
    }
    let Some(key) = authorize(m, client).await? else {
        for (id, e) in &cfg.servers {
            if e.enabled {
                set(m, id, "authorizing", None, None);
            }
        }
        return Ok(());
    };
    if let Some(c) = child {
        if c.try_wait()?.is_some() {
            *child = None;
        }
    }
    if any_live && child.is_none() {
        for (id, e) in &cfg.servers {
            if e.enabled && live(id) {
                set(m, id, "downloading", None, None);
            }
        }
        let path = executable(m, client).await?;
        if cfg.servers.iter().any(|(id, e)| e.enabled && live(id)) {
            *child = Some(launch(m, path, &key)?);
        }
    }
    for id in cfg.servers.keys() {
        let _ = flag(m, id, cfg.servers[id].enabled);
    }
    let live = |id: &str| is_live(m, id) && flag(m, id, false).load(Ordering::SeqCst);
    for (id, entry) in &mut cfg.servers {
        if !entry.enabled {
            continue;
        }
        if !live(id) {
            set(m, id, "ready", None, None);
            continue;
        }
        let target = crate::public_access::target(m, id)?;
        if !crate::public_access::local_ready(m, id).await {
            set(
                m,
                id,
                "waiting",
                None,
                Some("正在等待 Minecraft 监听本地端口…".into()),
            );
            continue;
        }
        let udp_tunnel = target.protocol == crate::public_access::Protocol::Udp;
        let now = Instant::now();
        let (local_port, endpoint) = if udp_tunnel {
            bridges.remove(id);
            let state = udp.entry(id.clone()).or_insert_with(|| UdpCheck::new(now));
            if state.next > now {
                continue;
            }
            (target.port, state.endpoint.clone())
        } else {
            udp.remove(id);
            if bridges
                .get(id)
                .is_some_and(|b| b.target != target.port || b.task.is_finished())
            {
                bridges.remove(id);
            }
            if !bridges.contains_key(id) {
                bridges.insert(
                    id.clone(),
                    bridge(target.port, Some((m.runtime(id)?, flag(m, id, true)))).await?,
                );
                set(m, id, "connecting", None, None);
            }
            let bridge = bridges.get_mut(id).expect("bridge inserted above");
            if bridge.next_check > now {
                continue;
            }
            bridge.next_check = now + Duration::from_secs(20);
            (bridge.port, bridge.endpoint.clone())
        };
        let endpoint = match endpoint {
            Some(endpoint) => Ok(Some(endpoint)),
            None => ensure_tunnel(client, &key, id, local_port, target.protocol, entry).await,
        };
        match endpoint {
            Ok(Some((address, host, port))) => {
                let verified = if udp_tunnel {
                    true
                } else {
                    probe(&host, port).await
                };
                let fresh = if verified {
                    Some((address.clone(), host, port))
                } else {
                    None
                };
                if udp_tunnel {
                    if let Some(state) = udp.get_mut(id) {
                        state.endpoint = fresh;
                        state.next = Instant::now() + Duration::from_secs(20);
                    }
                } else if let Some(bridge) = bridges.get_mut(id) {
                    bridge.endpoint = fresh;
                }
                if live(id) {
                    set(
                        m,
                        id,
                        if verified { "online" } else { "connecting" },
                        Some(address),
                        None,
                    );
                } else {
                    bridges.remove(id);
                    udp.remove(id);
                    set(m, id, "ready", None, None);
                }
            }
            Ok(None) => {
                if let Some(state) = udp.get_mut(id) {
                    state.next = Instant::now() + Duration::from_secs(20);
                }
            }
            Err(e) => {
                if let Some(state) = udp.get_mut(id) {
                    state.next = Instant::now() + Duration::from_secs(15);
                }
                set(m, id, "error", None, Some(e.message));
            }
        }
    }
    // Merge only IDs; a disable made while a network call was pending must win.
    let _gate = m.playit.gate.lock().await;
    let mut latest = load(m)?;
    let mut changed = false;
    for (id, entry) in cfg.servers {
        if let Some(current) = latest.servers.get_mut(&id) {
            if current.tunnel_id != entry.tunnel_id {
                current.tunnel_id = entry.tunnel_id;
                changed = true;
            }
        }
    }
    if changed {
        storage::write_json(&config_path(m)?, &latest)?;
    }
    Ok(())
}
pub async fn shutdown(m: &Manager) {
    let task = m.playit.task.lock().unwrap().take();
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_requires_active_assigned_endpoint() {
        let mut t = json!({"active":true,"alloc":{"status":"allocated","data":{"ip_hostname":"game.ply.gg","port_start":25566}}});
        assert_eq!(allocated(&t).unwrap().0, "game.ply.gg:25566");
        t["active"] = json!(false);
        assert!(allocated(&t).is_none());
        t["active"] = json!(true);
        t["alloc"]["data"]["ip_hostname"] = json!("evil/path");
        assert!(allocated(&t).is_none());
    }
    #[test]
    fn only_minecraft_status_counts_as_reachable() {
        assert!(!valid_status(b"HTTP/1.1 200 OK"));
        assert!(!valid_status(&[0, 2, b'{', b'}']));
        let json = br#"{"version":{"name":"1.21"},"players":{"online":0}}"#;
        let mut body = vec![0];
        varint(json.len() as u32, &mut body);
        body.extend(json);
        assert!(valid_status(&body));
        body.pop();
        assert!(!valid_status(&body));
    }
    #[test]
    fn creation_only_targets_loopback_and_free_java_allocation() {
        let body = create_body(
            "server",
            "agent",
            45678,
            crate::public_access::Protocol::Tcp,
        );
        assert_eq!(body["origin"]["data"]["local_ip"], "127.0.0.1");
        assert_eq!(body["port_type"], "tcp");
        assert_eq!(body["tunnel_type"], "minecraft-java");
        assert!(body["alloc"].is_null());
        let udp = create_body(
            "server",
            "agent",
            19132,
            crate::public_access::Protocol::Udp,
        );
        assert_eq!(udp["port_type"], "udp");
        assert_eq!(udp["tunnel_type"], "minecraft-bedrock");
        assert_eq!(udp["origin"]["data"]["local_port"], 19132);
        assert!(
            decode(json!({"status":"error","data":{"secret_key":"private"}}))
                .unwrap_err()
                .to_string()
                .find("private")
                .is_none()
        );
    }
    #[tokio::test]
    async fn bridge_forwards_and_closes_existing_connections_on_drop() {
        let game = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let b = bridge(game.local_addr().unwrap().port(), None)
            .await
            .unwrap();
        let mut player = TcpStream::connect(("127.0.0.1", b.port)).await.unwrap();
        let (mut server, _) = game.accept().await.unwrap();
        player.write_all(b"hello").await.unwrap();
        let mut bytes = [0; 5];
        server.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"hello");
        drop(b);
        let result = tokio::time::timeout(Duration::from_secs(2), player.read_u8())
            .await
            .unwrap();
        assert!(result.is_err());
    }
}
