//! Managed SakuraFrp (樱花 FRP) tunnels for Minecraft Java servers.
//!
//! Tunnel metadata is managed through the official v4 API. The data plane is
//! the user's installed `frpc.exe`; the access key is stored with DPAPI and is
//! only passed to that child through its environment, never on the command
//! line, in logs, or to the frontend.
use crate::{
    error::{AppError, Result},
    models::Status,
    secrets,
    server::manager::Manager,
    storage,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::process::{Child, Command};

const API: &str = "https://api.natfrp.com/v4";
const SECRET: &str = "sakura.token";
const PANEL_URL: &str = "https://www.natfrp.com/tunnel/";
const TOKEN_URL: &str = "https://www.natfrp.com/user/profile";
const USER_AGENT: &str = "EasyLauncher/0.2.2";
/// `1 << 2`: the node currently accepts new tunnels.
const ACCEPT_NEW: u32 = 1 << 2;
/// `1 << 9`: the node is offline.
const OFFLINE: u32 = 1 << 9;
/// `1 << 11`: the node allows publicly advertised game servers.
const PUBLIC_GAMES: u32 = 1 << 11;

#[derive(Default)]
pub struct Service {
    pub gate: tokio::sync::Mutex<()>,
    /// Bumped when the access key changes so live connections restart.
    generation: AtomicU64,
    infos: Mutex<BTreeMap<String, Info>>,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Config {
    servers: BTreeMap<String, Entry>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Entry {
    enabled: bool,
    tunnel_id: Option<u32>,
    node: Option<u32>,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub enabled: bool,
    pub authorized: bool,
    pub phase: String,
    pub address: Option<String>,
    pub tunnel_id: Option<u32>,
    pub message: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: u32,
    pub name: String,
    pub host: String,
    pub vip: i64,
    pub flag: u32,
    pub available: bool,
    pub offline: bool,
    pub public_games: bool,
}
impl Node {
    fn usable(&self) -> bool {
        self.available && !self.offline
    }
}
#[derive(Clone, Debug)]
struct Bound {
    id: u32,
    address: String,
    host: String,
    port: u16,
    online: bool,
}
struct Run {
    child: Option<Child>,
    bound: Option<Bound>,
    next_api: Instant,
    next_probe: Instant,
    next_spawn: Instant,
    verified: bool,
}
impl Run {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            child: None,
            bound: None,
            next_api: now,
            next_probe: now,
            next_spawn: now,
            verified: false,
        }
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
        self.bound = None;
        self.verified = false;
    }
}
#[derive(Default)]
struct NodeCache {
    nodes: BTreeMap<u32, Node>,
    at: Option<Instant>,
}
impl NodeCache {
    fn fresh(&self) -> bool {
        self.at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(30 * 60))
    }
}

fn config_path(m: &Manager) -> Result<PathBuf> {
    storage::child(&m.root, "sakura.json")
}
fn load(m: &Manager) -> Result<Config> {
    let path = config_path(m)?;
    if !path.exists() {
        return Ok(Config::default());
    }
    if std::fs::metadata(&path)?.len() > 1024 * 1024 {
        return Err(AppError::user("SakuraFrp configuration is too large."));
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
fn tunnel_name(id: &str) -> String {
    // Sakura's panel limits tunnel names; IDs are UUIDs, so the first 12
    // characters are enough to keep servers apart.
    format!("EasyLauncher-{}", id.chars().take(12).collect::<String>())
}
fn token(m: &Manager) -> Result<String> {
    secrets::load(m, SECRET)?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::user("请先填写樱花 FRP 访问密钥。"))
}
fn is_live(m: &Manager, id: &str) -> bool {
    !m.shutting_down.load(std::sync::atomic::Ordering::SeqCst)
        && m.runtime(id)
            .is_ok_and(|r| matches!(r.snapshot().status, Status::Starting | Status::Running))
}
fn set(m: &Manager, id: &str, phase: &str, address: Option<String>, message: Option<String>) {
    m.sakura.infos.lock().unwrap().insert(
        id.into(),
        Info {
            phase: phase.into(),
            address,
            message,
            ..Info::default()
        },
    );
}

pub fn info(m: &Manager, id: &str) -> Result<Info> {
    m.metadata(id)?;
    let entry = load(m)?.servers.get(id).cloned().unwrap_or_default();
    let mut info = m
        .sakura
        .infos
        .lock()
        .unwrap()
        .get(id)
        .cloned()
        .unwrap_or_default();
    info.enabled = entry.enabled;
    info.authorized = secrets::load(m, SECRET)?.is_some();
    info.tunnel_id = entry.tunnel_id.or(info.tunnel_id);
    if !info.enabled {
        info.phase = "disabled".into();
        info.address = None;
        info.message = None;
    } else if !info.authorized {
        info.phase = "needs_token".into();
        info.address = None;
    } else if info.phase.is_empty() {
        info.phase = "preparing".into();
    }
    // A saved preference only starts a connection while the server is live.
    if info.enabled && info.authorized && !is_live(m, id) && info.phase != "error" {
        info.phase = "ready".into();
        info.address = None;
        info.message = None;
    }
    if info.enabled && !is_live(m, id) && info.phase != "error" {
        info.address = None;
    }
    Ok(info)
}

pub async fn enable(m: &Manager, id: &str, enabled: bool, node: Option<u32>) -> Result<Info> {
    m.ready()?;
    m.metadata(id)?;
    if enabled {
        m.ensure_writable()?;
        // The key can be entered after selecting the provider; the worker
        // reports `needs_token` until it arrives.
    }
    let _gate = m.sakura.gate.lock().await;
    let mut cfg = load(m)?;
    let entry = cfg.servers.entry(id.into()).or_default();
    entry.enabled = enabled;
    if let Some(node) = node {
        entry.node = Some(node);
    }
    storage::write_json(&config_path(m)?, &cfg)?;
    set(
        m,
        id,
        if enabled { "preparing" } else { "disabled" },
        None,
        None,
    );
    info(m, id)
}

pub async fn save_token(m: &Manager, value: &str) -> Result<()> {
    m.ensure_writable()?;
    let value = value.trim();
    if !(16..=128).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(AppError::user(
            "访问密钥格式不正确，请重新从樱花 FRP 管理面板复制。",
        ));
    }
    request(&m.client(), reqwest::Method::GET, "/user/info", value, None).await?;
    secrets::save(m, SECRET, value)?;
    m.sakura.generation.fetch_add(1, Ordering::SeqCst);
    m.sakura.infos.lock().unwrap().clear();
    Ok(())
}
pub async fn reset_token(m: &Manager) -> Result<()> {
    m.ensure_writable()?;
    let _gate = m.sakura.gate.lock().await;
    secrets::delete(m, SECRET)?;
    m.sakura.generation.fetch_add(1, Ordering::SeqCst);
    m.sakura.infos.lock().unwrap().clear();
    Ok(())
}
pub async fn nodes(m: &Manager) -> Result<Vec<Node>> {
    let token = token(m)?;
    let value = request(&m.client(), reqwest::Method::GET, "/nodes", &token, None).await?;
    let mut nodes: Vec<Node> = parse_nodes(&value)?.into_values().collect();
    sort_nodes(&mut nodes);
    Ok(nodes)
}
pub fn open_panel() -> Result<()> {
    open::that(PANEL_URL).map_err(|_| AppError::user("无法打开樱花 FRP 管理面板。"))?;
    Ok(())
}
pub fn open_token_page() -> Result<()> {
    open::that(TOKEN_URL).map_err(|_| AppError::user("无法打开樱花 FRP 用户信息页面。"))?;
    Ok(())
}

fn sort_nodes(nodes: &mut [Node]) {
    nodes.sort_by(|a, b| {
        b.usable()
            .cmp(&a.usable())
            .then(a.vip.cmp(&b.vip))
            .then_with(|| b.public_games.cmp(&a.public_games))
            .then_with(|| a.name.cmp(&b.name))
    });
}
fn best_node(nodes: &BTreeMap<u32, Node>) -> Option<u32> {
    nodes
        .values()
        .filter(|node| node.usable())
        .min_by(|a, b| {
            a.vip
                .cmp(&b.vip)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        })
        .map(|node| node.id)
}
fn parse_nodes(value: &Value) -> Result<BTreeMap<u32, Node>> {
    let object = value
        .as_object()
        .ok_or_else(|| AppError::user("樱花 FRP 返回的节点列表格式不正确。"))?;
    let mut nodes = BTreeMap::new();
    for (key, node) in object {
        let Ok(id) = key.parse::<u32>() else {
            continue;
        };
        let flag = node["flag"].as_u64().unwrap_or(0) as u32;
        let name = node["name"].as_str().unwrap_or_default().to_string();
        let host = node["host"].as_str().unwrap_or_default().to_string();
        if host.is_empty() {
            continue;
        }
        nodes.insert(
            id,
            Node {
                id,
                name,
                host,
                vip: node["vip"].as_i64().unwrap_or(0),
                flag,
                available: flag & ACCEPT_NEW != 0,
                offline: flag & OFFLINE != 0,
                public_games: flag & PUBLIC_GAMES != 0,
            },
        );
    }
    if nodes.is_empty() {
        return Err(AppError::user("樱花 FRP 没有返回可用节点。"));
    }
    Ok(nodes)
}
fn bound_from(
    nodes: &BTreeMap<u32, Node>,
    tunnel: &Value,
    protocol: &str,
    edited: bool,
) -> Result<(Bound, bool)> {
    let id = tunnel["id"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| AppError::user("樱花 FRP 隧道缺少 ID。"))?;
    if tunnel["type"].as_str() != Some(protocol) {
        return Err(AppError::user(format!(
            "这条樱花 FRP 隧道不是 {protocol} 类型。请在管理面板删除它，或重新选择协议后再开启穿透。"
        )));
    }
    let node_id = tunnel["node"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| AppError::user("樱花 FRP 隧道缺少节点信息。"))?;
    let node = nodes
        .get(&node_id)
        .ok_or_else(|| AppError::user("找不到这条隧道使用的樱花 FRP 节点。"))?;
    if node.host.is_empty()
        || node.host.len() > 253
        || !node
            .host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
    {
        return Err(AppError::user("樱花 FRP 节点的公网地址格式不正确。"));
    }
    let port = tunnel["remote"]
        .as_str()
        .and_then(|value| value.parse::<u16>().ok())
        .or_else(|| {
            tunnel["remote"]
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
        })
        .filter(|port| *port > 0)
        .ok_or_else(|| {
            AppError::user("这条樱花 FRP 隧道还没有分配公网端口，请在管理面板检查隧道状态。")
        })?;
    Ok((
        Bound {
            id,
            address: format!("{}:{port}", node.host),
            host: node.host.clone(),
            port,
            online: tunnel["online"].as_bool().unwrap_or(false),
        },
        edited,
    ))
}

async fn request(
    client: &reqwest::Client,
    method: reqwest::Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Result<Value> {
    let mut request = client
        .request(method, format!("{API}{path}"))
        .bearer_auth(token)
        .timeout(Duration::from_secs(20));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.map_err(|e| {
        AppError::user(if e.is_timeout() {
            "连接樱花 FRP 超时，请检查网络后重试。"
        } else {
            "无法连接樱花 FRP，请检查网络后重试。"
        })
    })?;
    let status = response.status();
    let mut bytes = Vec::new();
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| AppError::new("无法读取樱花 FRP 的响应，请检查网络后重试。", e))?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(AppError::user("樱花 FRP 返回的数据超过大小限制。"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let message = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| value["msg"].as_str().map(|msg| clean_message(msg, token)));
    if !status.is_success() {
        return Err(AppError::user(match status.as_u16() {
            401 => "樱花 FRP 访问密钥无效或权限不足，请重新填写访问密钥。".into(),
            403 => message
                .unwrap_or_else(|| "樱花 FRP 访问密钥无效或权限不足，请重新填写访问密钥。".into()),
            429 => "樱花 FRP 请求过于频繁，请稍后重试。".into(),
            _ => message.unwrap_or_else(|| "樱花 FRP 拒绝了请求，请稍后重试。".into()),
        }));
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::new("樱花 FRP 返回了无法解析的数据。", e))?;
    // Some endpoints can report business errors with HTTP 200.
    if value["code"].as_u64().is_some_and(|code| code >= 400) {
        return Err(AppError::user(
            value["msg"]
                .as_str()
                .map(|msg| clean_message(msg, token))
                .unwrap_or_else(|| "樱花 FRP 拒绝了请求，请稍后重试。".into()),
        ));
    }
    Ok(value)
}
fn clean_message(message: &str, secret: &str) -> String {
    let message = message.replace(secret, "[redacted]");
    let message: String = message
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    if message.trim().is_empty() {
        "樱花 FRP 拒绝了请求，请稍后重试。".into()
    } else {
        format!("樱花 FRP：{}", message.trim())
    }
}

async fn load_nodes(client: &reqwest::Client, token: &str) -> Result<BTreeMap<u32, Node>> {
    let value = request(client, reqwest::Method::GET, "/nodes", token, None).await?;
    parse_nodes(&value)
}
async fn ensure_tunnel(
    client: &reqwest::Client,
    token: &str,
    id: &str,
    port: u16,
    protocol: &str,
    entry: &mut Entry,
    cache: &mut NodeCache,
) -> Result<(Bound, bool)> {
    if !cache.fresh() {
        cache.nodes = load_nodes(client, token).await?;
        cache.at = Some(Instant::now());
    }
    let list = request(client, reqwest::Method::GET, "/tunnels", token, None).await?;
    let tunnels = list
        .as_array()
        .ok_or_else(|| AppError::user("樱花 FRP 返回的隧道列表格式不正确。"))?;
    let name = tunnel_name(id);
    let mut tunnel = tunnels
        .iter()
        .find(|tunnel| {
            entry.tunnel_id.is_some() && tunnel["id"].as_u64() == entry.tunnel_id.map(u64::from)
        })
        .or_else(|| {
            tunnels
                .iter()
                .find(|tunnel| tunnel["name"].as_str() == Some(&name))
        })
        .cloned();
    let mut restart = false;
    if tunnel.is_none() {
        let node = match entry.node {
            Some(node) => node,
            None => best_node(&cache.nodes)
                .ok_or_else(|| AppError::user("没有可创建隧道的樱花 FRP 节点，请稍后重试。"))?,
        };
        let created = request(
            client,
            reqwest::Method::POST,
            "/tunnels",
            token,
            Some(json!({
                "name": name,
                "type": protocol,
                "node": node,
                "local_ip": "127.0.0.1",
                "local_port": port,
            })),
        )
        .await?;
        entry.tunnel_id = created["id"]
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .or(entry.tunnel_id);
        entry.node = Some(node);
        let list = request(client, reqwest::Method::GET, "/tunnels", token, None).await?;
        tunnel = list
            .as_array()
            .and_then(|tunnels| {
                tunnels.iter().find(|tunnel| {
                    tunnel["id"].as_u64() == entry.tunnel_id.map(u64::from)
                        || tunnel["name"].as_str() == Some(&name)
                })
            })
            .cloned();
        if tunnel.is_none() {
            return Err(AppError::user(
                "樱花 FRP 已创建隧道，但暂时无法读取它的状态，正在重试。",
            ));
        }
        restart = true;
    }
    let tunnel = tunnel.expect("checked above");
    let id = tunnel["id"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| AppError::user("樱花 FRP 隧道缺少 ID。"))?;
    if entry.tunnel_id != Some(id) {
        entry.tunnel_id = Some(id);
        restart = true;
    }
    if tunnel["type"].as_str() != Some(protocol) {
        return Err(AppError::user(format!(
            "这条樱花 FRP 隧道不是 {protocol} 类型。请在管理面板删除它，或重新选择协议后再开启穿透。"
        )));
    }
    if let Some(node) = tunnel["node"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
    {
        entry.node = Some(node);
    }
    if tunnel["local_ip"].as_str() != Some("127.0.0.1")
        || tunnel["local_port"].as_u64() != Some(u64::from(port))
    {
        request(
            client,
            reqwest::Method::POST,
            "/tunnel/edit",
            token,
            Some(json!({"id": id, "local_ip": "127.0.0.1", "local_port": port})),
        )
        .await?;
        restart = true;
        let mut tunnel = tunnel;
        tunnel["local_ip"] = json!("127.0.0.1");
        tunnel["local_port"] = json!(port);
        return bound_from(&cache.nodes, &tunnel, protocol, restart);
    }
    bound_from(&cache.nodes, &tunnel, protocol, restart)
}

fn executable() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for variable in [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "LOCALAPPDATA",
        "ProgramData",
    ] {
        if let Some(root) = std::env::var_os(variable) {
            candidates.push(
                PathBuf::from(root)
                    .join("SakuraFrpLauncher")
                    .join("frpc.exe"),
            );
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("Programs")
                .join("SakuraFrpLauncher")
                .join("frpc.exe"),
        );
    }
    candidates.extend(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|directory| directory.join("frpc.exe")),
    );
    candidates
        .into_iter()
        .find(|path| path.is_file() && path.file_name().is_some_and(|name| name == "frpc.exe"))
}
fn launch(m: &Manager, path: &Path, token: &str, tunnel: u32) -> Result<Child> {
    let directory = storage::child(&m.root, "sakura")?;
    std::fs::create_dir_all(&directory)?;
    let mut command = Command::new(path);
    command
        .arg("-n")
        .env("NATFRP_TOKEN", token)
        .env("NATFRP_TARGET", tunnel.to_string())
        .current_dir(&directory)
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

pub fn spawn(m: Arc<Manager>) {
    let owner = m.clone();
    let task = tauri::async_runtime::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .build()
        {
            Ok(client) => client,
            Err(_) => return,
        };
        let mut runs: BTreeMap<String, Run> = BTreeMap::new();
        let mut cache = NodeCache::default();
        let mut generation = m.sakura.generation.load(Ordering::SeqCst);
        loop {
            let current = m.sakura.generation.load(Ordering::SeqCst);
            if current != generation {
                for run in runs.values_mut() {
                    run.stop();
                }
                runs.clear();
                generation = current;
            }
            let result = tick(&m, &client, &mut runs, &mut cache).await;
            if let Err(error) = &result {
                if let Ok(cfg) = load(&m) {
                    for (id, entry) in cfg.servers {
                        if entry.enabled {
                            set(&m, &id, "error", None, Some(error.message.clone()));
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(if result.is_err() { 15 } else { 3 })).await;
        }
    });
    *owner.sakura.task.lock().unwrap() = Some(task);
}

async fn tick(
    m: &Manager,
    client: &reqwest::Client,
    runs: &mut BTreeMap<String, Run>,
    cache: &mut NodeCache,
) -> Result<()> {
    let mut cfg = {
        let _gate = m.sakura.gate.lock().await;
        load(m)?
    };
    cfg.servers.retain(|id, _| m.metadata(id).is_ok());
    if m.settings()?.read_only {
        for run in runs.values_mut() {
            run.stop();
        }
        runs.clear();
        for (id, entry) in &cfg.servers {
            if entry.enabled {
                set(
                    m,
                    id,
                    "error",
                    None,
                    Some("只读模式下暂停樱花穿透。".into()),
                );
            }
        }
        return Ok(());
    }
    for (id, run) in runs.iter_mut() {
        if !cfg.servers.get(id).is_some_and(|entry| entry.enabled) || !is_live(m, id) {
            run.stop();
        }
    }
    runs.retain(|id, _| cfg.servers.get(id).is_some_and(|entry| entry.enabled) && is_live(m, id));
    let any_live = cfg
        .servers
        .iter()
        .any(|(id, entry)| entry.enabled && is_live(m, id));
    if !any_live {
        return Ok(());
    }
    let Some(token) = secrets::load(m, SECRET)? else {
        for run in runs.values_mut() {
            run.stop();
        }
        for (id, entry) in &cfg.servers {
            if entry.enabled {
                set(m, id, "needs_token", None, None);
            }
        }
        return Ok(());
    };
    let exe = executable();
    let ids: Vec<String> = cfg.servers.keys().cloned().collect();
    for id in ids {
        if !cfg.servers.get(&id).is_some_and(|entry| entry.enabled) {
            continue;
        }
        if !is_live(m, &id) {
            if let Some(run) = runs.get_mut(&id) {
                run.stop();
            }
            runs.remove(&id);
            set(m, &id, "ready", None, None);
            continue;
        }
        let run = runs.entry(id.clone()).or_insert_with(Run::new);
        let now = Instant::now();
        let target = crate::public_access::target(m, &id)?;
        let protocol = target.protocol;
        if run
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_some())
        {
            run.child = None;
            run.bound = None;
            run.verified = false;
            run.next_spawn = now + Duration::from_secs(10);
            set(
                m,
                &id,
                "error",
                None,
                Some("樱花 FRP 连接进程意外退出，正在重试。".into()),
            );
        }
        if run.bound.is_none() || now >= run.next_api {
            run.next_api = now + Duration::from_secs(60);
            let entry = cfg.servers.get_mut(&id).expect("enabled entry");
            match ensure_tunnel(
                client,
                &token,
                &id,
                target.port,
                protocol.as_str(),
                entry,
                cache,
            )
            .await
            {
                Ok((bound, restart)) => {
                    let endpoint_changed = run
                        .bound
                        .as_ref()
                        .is_some_and(|old| old.id != bound.id || old.address != bound.address);
                    let went_offline =
                        run.bound.as_ref().is_some_and(|old| old.online) && !bound.online;
                    if restart {
                        run.stop();
                        run.next_spawn = now;
                    } else if endpoint_changed || went_offline {
                        // The old verification no longer describes this
                        // endpoint; probe again before sharing it.
                        run.verified = false;
                        run.next_probe = now;
                    }
                    run.bound = Some(bound);
                }
                Err(error) => {
                    run.next_api = now + Duration::from_secs(15);
                    set(m, &id, "error", None, Some(error.message));
                    continue;
                }
            }
        }
        let Some(bound) = run.bound.as_ref() else {
            continue;
        };
        // Wait until the Minecraft process is actually listening before
        // connecting the tunnel, otherwise the agent starts against a dead port.
        if run.child.is_none()
            && now >= run.next_spawn
            && !crate::public_access::local_ready(m, &id).await
        {
            set(
                m,
                &id,
                "waiting",
                Some(bound.address.clone()),
                Some("正在等待 Minecraft 监听本地端口…".into()),
            );
            continue;
        }
        if run.child.is_none() && now >= run.next_spawn {
            let Some(path) = &exe else {
                set(
                    m,
                    &id,
                    "error",
                    None,
                    Some(
                        "未找到樱花 FRP 的 frpc.exe。请先安装樱花启动器，或确认 frpc.exe 位于 Program Files\\SakuraFrpLauncher。"
                            .into(),
                    ),
                );
                continue;
            };
            match launch(m, path, &token, bound.id) {
                Ok(child) => {
                    run.child = Some(child);
                    run.next_probe = now + Duration::from_secs(3);
                    run.verified = false;
                }
                Err(error) => {
                    run.next_spawn = now + Duration::from_secs(10);
                    set(m, &id, "error", None, Some(error.message));
                    continue;
                }
            }
        }
        let bound = run.bound.as_ref().expect("bound set above");
        if now >= run.next_probe {
            // The agent may report the tunnel online before the local server
            // is ready. The Minecraft status handshake is the ground truth for
            // TCP. UDP has no handshake, so the tunnel's own state is used.
            let verified = if protocol == crate::public_access::Protocol::Udp {
                bound.online
            } else {
                crate::playit::probe(&bound.host, bound.port).await
            };
            run.verified = verified;
            run.next_probe = now + Duration::from_secs(if verified { 60 } else { 8 });
        }
        if run.verified {
            set(m, &id, "online", Some(bound.address.clone()), None);
        } else {
            set(
                m,
                &id,
                "connecting",
                Some(bound.address.clone()),
                Some(if bound.online {
                    "隧道已连接，正在通过公网验证 Minecraft 响应…".into()
                } else {
                    "正在连接樱花 FRP 节点…".into()
                }),
            );
        }
    }
    // Only merge tunnel IDs and nodes; a disable made while a request was in
    // flight must win over this loop's snapshot.
    let _gate = m.sakura.gate.lock().await;
    let mut latest = load(m)?;
    let mut changed = false;
    for (id, entry) in cfg.servers {
        if let Some(current) = latest.servers.get_mut(&id) {
            if current.tunnel_id != entry.tunnel_id {
                current.tunnel_id = entry.tunnel_id;
                changed = true;
            }
            if current.node != entry.node && entry.node.is_some() {
                current.node = entry.node;
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
    let task = m.sakura.task.lock().unwrap().take();
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, name: &str, vip: i64, flag: u32) -> Node {
        Node {
            id,
            name: name.into(),
            host: format!("node-{id}.natfrp.io"),
            vip,
            flag,
            available: flag & ACCEPT_NEW != 0,
            offline: flag & OFFLINE != 0,
            public_games: flag & PUBLIC_GAMES != 0,
        }
    }
    #[test]
    fn node_flags_describe_availability_and_public_games() {
        let parsed = parse_nodes(&json!({
            "5": {"name": "多线一区", "host": "a.natfrp.io", "vip": 1, "flag": (ACCEPT_NEW | PUBLIC_GAMES)},
            "6": {"name": "普通节点", "host": "b.natfrp.io", "vip": 0, "flag": ACCEPT_NEW},
            "7": {"name": "满载", "host": "c.natfrp.io", "vip": 0, "flag": 0},
        }))
        .unwrap();
        assert!(parsed[&5].public_games);
        assert!(parsed[&5].usable());
        assert!(!parsed[&7].available);
        let mut nodes: Vec<Node> = parsed.into_values().collect();
        sort_nodes(&mut nodes);
        assert_eq!(nodes[0].id, 6, "the free, usable node is listed first");
    }
    #[test]
    fn best_node_prefers_an_available_free_node() {
        let mut nodes = BTreeMap::new();
        nodes.insert(1, node(1, "vip", 2, ACCEPT_NEW));
        nodes.insert(2, node(2, "full", 0, 0));
        nodes.insert(3, node(3, "offline", 0, ACCEPT_NEW | OFFLINE));
        nodes.insert(4, node(4, "free", 0, ACCEPT_NEW));
        assert_eq!(best_node(&nodes), Some(4));
    }
    #[test]
    fn endpoint_requires_a_tcp_port_and_a_safe_host() {
        let nodes = parse_nodes(&json!({
            "5": {"name": "node", "host": "idea-leaper-1.natfrp.io", "vip": 0, "flag": ACCEPT_NEW}
        }))
        .unwrap();
        let (bound, _) = bound_from(
            &nodes,
            &json!({"id": 9, "type": "tcp", "node": 5, "remote": "25565", "online": true, "local_port": 25565}),
            "tcp",
            false,
        )
        .unwrap();
        assert_eq!(bound.address, "idea-leaper-1.natfrp.io:25565");
        assert!(bound.online);
        assert!(bound_from(
            &nodes,
            &json!({"id": 9, "type": "udp", "node": 5, "remote": "19132", "local_port": 19132}),
            "udp",
            false,
        )
        .is_ok());
        for tunnel in [
            json!({"id": 9, "type": "udp", "node": 5, "remote": "25565"}),
            json!({"id": 9, "type": "tcp", "node": 5, "remote": "example.tld"}),
            json!({"id": 9, "type": "tcp", "node": 5, "remote": "0"}),
        ] {
            assert!(bound_from(&nodes, &tunnel, "tcp", false).is_err());
        }
        let unsafe_nodes = parse_nodes(&json!({
            "5": {"name": "node", "host": "evil/path", "vip": 0, "flag": ACCEPT_NEW}
        }))
        .unwrap();
        assert!(bound_from(
            &unsafe_nodes,
            &json!({"id": 9, "type": "tcp", "node": 5, "remote": "25565"}),
            "tcp",
            false
        )
        .is_err());
    }
    #[test]
    fn provider_errors_are_cleaned_and_never_echo_secrets() {
        assert_eq!(
            clean_message("  quota\n exhausted  ", "secret"),
            "樱花 FRP：quota exhausted"
        );
        assert_eq!(
            clean_message(" \n ", "secret"),
            "樱花 FRP 拒绝了请求，请稍后重试。"
        );
        assert_eq!(
            clean_message("bad secret", "secret"),
            "樱花 FRP：bad [redacted]"
        );
    }
    #[test]
    fn tunnel_names_are_short_and_unique_per_server() {
        assert_eq!(tunnel_name("abc-123"), "EasyLauncher-abc-123");
        assert_eq!(
            tunnel_name("12345678-1234-1234-1234-123456789abc"),
            "EasyLauncher-12345678-123"
        );
    }
}
