//! Unified public access for Minecraft servers.
//!
//! Every provider is reached through the same small `TunnelProvider` contract:
//! it can be activated once, follows the server lifecycle on its own, reports a
//! status/address, and can be deactivated. Adding a provider only requires a new
//! `TunnelProvider` implementation and an id in [`Provider`].
//!
//! LAN-only is the zero-configuration baseline. SakuraFrp and playit reuse the
//! existing managed integrations; their credentials stay in the DPAPI vault.
//! Custom FRP wraps the user's own `frpc` configuration and is supervised here
//! so it starts and stops with the server like the managed providers.
use crate::{
    error::{AppError, Result},
    models::Status,
    network, playit, sakura,
    server::manager::Manager,
    storage, tunnel,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::net::TcpStream;

const FILE: &str = "public-access.json";
type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Lan,
    Sakura,
    Playit,
    Custom,
}
impl Provider {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "lan" => Some(Self::Lan),
            "sakura" => Some(Self::Sakura),
            "playit" => Some(Self::Playit),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lan => "lan",
            Self::Sakura => "sakura",
            Self::Playit => "playit",
            Self::Custom => "custom",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Tcp,
    Udp,
}
impl Protocol {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "tcp" => Some(Self::Tcp),
            "udp" => Some(Self::Udp),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub protocol: Protocol,
    pub port: u16,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    servers: BTreeMap<String, Entry>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Entry {
    provider: Provider,
    protocol: Protocol,
    /// Empty means "follow the server's current port".
    local_port: Option<u16>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub provider: String,
    pub protocol: String,
    pub local_port: u16,
    pub phase: String,
    pub address: Option<String>,
    pub message: Option<String>,
    pub authorized: bool,
    pub configured: bool,
    pub claim_url: Option<String>,
    pub tunnel_id: Option<u32>,
    pub verified: bool,
}
#[derive(Clone, Default)]
pub struct State {
    phase: String,
    address: Option<String>,
    message: Option<String>,
    authorized: bool,
    configured: bool,
    claim_url: Option<String>,
    tunnel_id: Option<u32>,
    verified: bool,
}

#[derive(Default)]
pub struct Service {
    gate: tokio::sync::Mutex<()>,
    verify: Mutex<BTreeMap<String, Verification>>,
    errors: Mutex<BTreeMap<String, String>>,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}
#[derive(Clone)]
struct Verification {
    address: String,
    verified: bool,
    next: Instant,
}

/// The contract every tunnel provider implements. Methods return boxed futures
/// so the registry below can select a provider at runtime without adding an
/// async-trait dependency.
pub trait TunnelProvider: Send + Sync {
    fn kind(&self) -> Provider;
    fn status<'a>(&'a self, m: &'a Manager, id: &'a str, target: Target) -> Fut<'a, Result<State>>;
    fn activate<'a>(
        &'a self,
        m: &'a Manager,
        id: &'a str,
        target: Target,
        node: Option<u32>,
        custom: Option<tunnel::Config>,
    ) -> Fut<'a, Result<()>>;
    fn deactivate<'a>(&'a self, m: &'a Manager, id: &'a str) -> Fut<'a, Result<()>>;
}

struct LanProvider;
struct SakuraProvider;
struct PlayitProvider;
struct CustomProvider;
static LAN: LanProvider = LanProvider;
static SAKURA: SakuraProvider = SakuraProvider;
static PLAYIT: PlayitProvider = PlayitProvider;
static CUSTOM: CustomProvider = CustomProvider;
static PROVIDERS: [&'static dyn TunnelProvider; 4] = [&LAN, &SAKURA, &PLAYIT, &CUSTOM];

fn provider(kind: Provider) -> &'static dyn TunnelProvider {
    PROVIDERS
        .iter()
        .copied()
        .find(|p| p.kind() == kind)
        .expect("every provider is registered")
}

fn config_path(m: &Manager) -> Result<std::path::PathBuf> {
    storage::child(&m.root, FILE)
}
fn load(m: &Manager) -> Result<Config> {
    let path = config_path(m)?;
    if !path.exists() {
        return Ok(Config::default());
    }
    if std::fs::metadata(&path)?.len() > 1024 * 1024 {
        return Err(AppError::user("Public access configuration is too large."));
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

/// The server is considered reachable by a tunnel once Java is starting or
/// running. Stopping is excluded so a tunnel never targets a dying process.
pub fn is_live(m: &Manager, id: &str) -> bool {
    !m.shutting_down.load(std::sync::atomic::Ordering::SeqCst)
        && m.runtime(id)
            .is_ok_and(|r| matches!(r.snapshot().status, Status::Starting | Status::Running))
}

/// Resolve the configured provider. Servers configured before this module
/// existed are inferred from the playit/Sakura settings they already had.
fn selection(m: &Manager, id: &str) -> Result<(Provider, Protocol, Option<u16>)> {
    if let Some(entry) = load(m)?.servers.get(id) {
        return Ok((entry.provider, entry.protocol, entry.local_port));
    }
    if sakura::info(m, id).is_ok_and(|info| info.enabled) {
        return Ok((Provider::Sakura, Protocol::Tcp, None));
    }
    if playit::info(m, id).is_ok_and(|info| info.enabled) {
        return Ok((Provider::Playit, Protocol::Tcp, None));
    }
    Ok((Provider::Lan, Protocol::Tcp, None))
}

/// The local endpoint a provider should expose. `local_port` is optional so
/// Java servers keep following their `server.properties` port automatically,
/// while a Bedrock/UDP server can point at its own port.
pub fn target(m: &Manager, id: &str) -> Result<Target> {
    let (_, protocol, local_port) = selection(m, id)?;
    let port = local_port
        .filter(|port| *port > 0)
        .unwrap_or(m.metadata(id)?.port);
    Ok(Target { protocol, port })
}

/// Whether the server is already listening locally. TCP is checked with a real
/// connection; UDP has no handshake, so the server status is the signal.
pub async fn local_ready(m: &Manager, id: &str) -> bool {
    let Ok(target) = target(m, id) else {
        return false;
    };
    match target.protocol {
        Protocol::Tcp => tokio::time::timeout(
            Duration::from_millis(800),
            TcpStream::connect(("127.0.0.1", target.port)),
        )
        .await
        .is_ok_and(|result| result.is_ok()),
        Protocol::Udp => is_live(m, id),
    }
}

pub async fn info(m: &Manager, id: &str) -> Result<Info> {
    m.metadata(id)?;
    let (kind, protocol, _) = selection(m, id)?;
    let target = target(m, id)?;
    let state = provider(kind).status(m, id, target).await?;
    Ok(Info {
        provider: kind.as_str().into(),
        protocol: protocol.as_str().into(),
        local_port: target.port,
        phase: state.phase,
        address: state.address,
        message: state.message,
        authorized: state.authorized,
        configured: state.configured,
        claim_url: state.claim_url,
        tunnel_id: state.tunnel_id,
        verified: state.verified,
    })
}

/// Save the one-time choice and hand the server over to that provider.
///
/// Switching providers deactivates the previous one; its saved credentials and
/// tunnel ids stay in place so switching back does not require another setup.
pub async fn configure(
    m: &Manager,
    id: &str,
    provider_name: &str,
    protocol_name: &str,
    local_port: Option<u16>,
    node: Option<u32>,
    custom: Option<tunnel::Config>,
) -> Result<Info> {
    m.ready()?;
    m.ensure_writable()?;
    m.metadata(id)?;
    let kind = Provider::parse(provider_name)
        .ok_or_else(|| AppError::user("Choose a supported public access provider."))?;
    let protocol = Protocol::parse(protocol_name)
        .ok_or_else(|| AppError::user("Choose TCP for Java or UDP for Bedrock."))?;
    if local_port.is_some_and(|port| port == 0) {
        return Err(AppError::user("Enter a port between 1 and 65535."));
    }
    if kind == Provider::Custom {
        if let Some(config) = &custom {
            tunnel::save(m, id, config)?;
        }
    }
    {
        let _gate = m.public_access.gate.lock().await;
        let mut config = load(m)?;
        let entry = config.servers.entry(id.into()).or_default();
        entry.provider = kind;
        entry.protocol = protocol;
        entry.local_port = local_port.filter(|port| *port > 0);
        storage::write_json(&config_path(m)?, &config)?;
    }
    // Stop whatever the server used before this change. A provider that fails
    // to stop must not block the newly selected one; its worker is no longer
    // selected anyway, and the failure is recorded for the next status read.
    for other in PROVIDERS.iter().copied().filter(|p| p.kind() != kind) {
        if let Err(error) = other.deactivate(m, id).await {
            m.logger.log(
                "WARN",
                &format!(
                    "{id}: failed to stop {} ({})",
                    other.kind().as_str(),
                    error.message
                ),
            );
        }
    }
    let target = Target {
        protocol,
        port: local_port.unwrap_or(m.metadata(id)?.port),
    };
    provider(kind).activate(m, id, target, node, custom).await?;
    m.public_access.verify.lock().unwrap().remove(id);
    m.public_access.errors.lock().unwrap().remove(id);
    info(m, id).await
}

/// Open the provider's setup/management page, when it has one.
pub fn open_setup(provider_name: &str) -> Result<()> {
    match Provider::parse(provider_name) {
        Some(Provider::Sakura) => sakura::open_token_page(),
        Some(Provider::Playit) => Ok(()),
        Some(Provider::Custom) => Ok(()),
        _ => Err(AppError::user("This provider has no setup page.")),
    }
}

fn configured(config: &tunnel::Config) -> bool {
    !config.executable.trim().is_empty()
        && !config.config_path.trim().is_empty()
        && !config.address.trim().is_empty()
}
fn host_port(address: &str) -> Option<(String, u16)> {
    let (host, port) = address.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty()
        || host.len() > 253
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:_".contains(&b))
    {
        return None;
    }
    let port = port.parse::<u16>().ok().filter(|port| *port > 0)?;
    Some((host.into(), port))
}
fn live_phase(m: &Manager, id: &str, ready: bool) -> &'static str {
    if !is_live(m, id) {
        "ready"
    } else if ready {
        "online"
    } else {
        "connecting"
    }
}
fn waiting(state: &str) -> Option<String> {
    (state == "connecting").then(|| "正在等待 Minecraft 监听本地端口…".into())
}

impl TunnelProvider for LanProvider {
    fn kind(&self) -> Provider {
        Provider::Lan
    }
    fn status<'a>(&'a self, m: &'a Manager, id: &'a str, target: Target) -> Fut<'a, Result<State>> {
        Box::pin(async move {
            m.metadata(id)?;
            let ready = is_live(m, id) && local_ready(m, id).await;
            let phase = live_phase(m, id, ready);
            Ok(State {
                phase: phase.into(),
                address: network::lan_address().map(|ip| format!("{ip}:{}", target.port)),
                message: waiting(phase),
                authorized: true,
                configured: true,
                verified: phase == "online",
                ..State::default()
            })
        })
    }
    fn activate<'a>(
        &'a self,
        _m: &'a Manager,
        _id: &'a str,
        _target: Target,
        _node: Option<u32>,
        _custom: Option<tunnel::Config>,
    ) -> Fut<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn deactivate<'a>(&'a self, _m: &'a Manager, _id: &'a str) -> Fut<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

fn phase_from(value: &str) -> String {
    match value {
        "disabled" | "needs_token" => "needs_setup",
        "preparing" | "authorizing" | "downloading" | "ready" | "waiting" | "connecting"
        | "online" | "error" => value,
        _ => "preparing",
    }
    .into()
}

impl TunnelProvider for SakuraProvider {
    fn kind(&self) -> Provider {
        Provider::Sakura
    }
    fn status<'a>(
        &'a self,
        m: &'a Manager,
        id: &'a str,
        _target: Target,
    ) -> Fut<'a, Result<State>> {
        Box::pin(async move {
            let info = sakura::info(m, id)?;
            let mut phase = phase_from(&info.phase);
            if !info.authorized {
                phase = "needs_setup".into();
            }
            let verified = phase == "online";
            Ok(State {
                phase,
                address: info.address,
                message: info.message,
                authorized: info.authorized,
                configured: info.authorized,
                tunnel_id: info.tunnel_id,
                verified,
                ..State::default()
            })
        })
    }
    fn activate<'a>(
        &'a self,
        m: &'a Manager,
        id: &'a str,
        _target: Target,
        node: Option<u32>,
        _custom: Option<tunnel::Config>,
    ) -> Fut<'a, Result<()>> {
        Box::pin(async move {
            sakura::enable(m, id, true, node).await?;
            Ok(())
        })
    }
    fn deactivate<'a>(&'a self, m: &'a Manager, id: &'a str) -> Fut<'a, Result<()>> {
        Box::pin(async move {
            sakura::enable(m, id, false, None).await?;
            Ok(())
        })
    }
}

impl TunnelProvider for PlayitProvider {
    fn kind(&self) -> Provider {
        Provider::Playit
    }
    fn status<'a>(
        &'a self,
        m: &'a Manager,
        id: &'a str,
        _target: Target,
    ) -> Fut<'a, Result<State>> {
        Box::pin(async move {
            let info = playit::info(m, id)?;
            let mut phase = phase_from(&info.phase);
            if !info.authorized && phase != "authorizing" {
                phase = "needs_setup".into();
            }
            let verified = phase == "online";
            Ok(State {
                phase,
                address: info.address,
                message: info.message,
                authorized: info.authorized,
                configured: info.authorized,
                claim_url: info.claim_url,
                verified,
                ..State::default()
            })
        })
    }
    fn activate<'a>(
        &'a self,
        m: &'a Manager,
        id: &'a str,
        _target: Target,
        _node: Option<u32>,
        _custom: Option<tunnel::Config>,
    ) -> Fut<'a, Result<()>> {
        Box::pin(async move {
            playit::enable(m, id, true).await?;
            Ok(())
        })
    }
    fn deactivate<'a>(&'a self, m: &'a Manager, id: &'a str) -> Fut<'a, Result<()>> {
        Box::pin(async move {
            playit::enable(m, id, false).await?;
            Ok(())
        })
    }
}

impl TunnelProvider for CustomProvider {
    fn kind(&self) -> Provider {
        Provider::Custom
    }
    fn status<'a>(&'a self, m: &'a Manager, id: &'a str, target: Target) -> Fut<'a, Result<State>> {
        Box::pin(async move {
            let info = tunnel::info(m, id).await?;
            let configured = configured(&info.config);
            let error = m.public_access.errors.lock().unwrap().get(id).cloned();
            let live = is_live(m, id);
            let mut verified = false;
            if configured && info.running {
                if target.protocol == Protocol::Udp {
                    verified = true;
                } else {
                    verified = m
                        .public_access
                        .verify
                        .lock()
                        .unwrap()
                        .get(id)
                        .is_some_and(|v| v.address == info.config.address && v.verified);
                }
            }
            let phase = if !configured {
                "needs_setup"
            } else if !live {
                "ready"
            } else if !info.running {
                "connecting"
            } else if verified {
                "online"
            } else {
                "connecting"
            };
            let message = error.or_else(|| {
                if !configured {
                    Some("填写 frpc 可执行文件、配置文件和公网地址后即可启用。".into())
                } else if live && !info.running {
                    Some(
                        info.exit_code
                            .map(|code| format!("frpc 已退出（代码 {code}），正在重试。"))
                            .unwrap_or_else(|| "正在启动自定义 frpc…".into()),
                    )
                } else if info.running && !verified {
                    Some("frpc 已启动，正在验证公网地址…".into())
                } else {
                    None
                }
            });
            Ok(State {
                phase: phase.into(),
                address: (!info.config.address.is_empty()).then_some(info.config.address),
                message,
                authorized: configured,
                configured,
                verified,
                ..State::default()
            })
        })
    }
    fn activate<'a>(
        &'a self,
        m: &'a Manager,
        id: &'a str,
        _target: Target,
        _node: Option<u32>,
        custom: Option<tunnel::Config>,
    ) -> Fut<'a, Result<()>> {
        Box::pin(async move {
            if let Some(config) = custom {
                tunnel::save(m, id, &config)?;
            }
            Ok(())
        })
    }
    fn deactivate<'a>(&'a self, m: &'a Manager, id: &'a str) -> Fut<'a, Result<()>> {
        Box::pin(async move {
            if tunnel::info(m, id).await.is_ok_and(|info| info.running) {
                tunnel::stop(m, id).await?;
            }
            Ok(())
        })
    }
}

/// Supervises custom frpc processes and verifies their public TCP endpoint.
/// playit and Sakura already supervise themselves from their own workers.
pub fn spawn(m: Arc<Manager>) {
    let owner = m.clone();
    let task = tauri::async_runtime::spawn(async move {
        loop {
            if let Err(error) = tick(&m).await {
                m.logger.log(
                    "WARN",
                    &format!("Public access supervisor: {}", error.message),
                );
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    *owner.public_access.task.lock().unwrap() = Some(task);
}

async fn tick(m: &Manager) -> Result<()> {
    let config = {
        let _gate = m.public_access.gate.lock().await;
        load(m)?
    };
    if m.settings()?.read_only {
        return Ok(());
    }
    for (id, entry) in config.servers {
        if entry.provider != Provider::Custom || m.metadata(&id).is_err() {
            continue;
        }
        let target = target(m, &id)?;
        let info = tunnel::info(m, &id).await?;
        if !is_live(m, &id) {
            if info.running {
                if let Err(error) = tunnel::stop(m, &id).await {
                    m.public_access
                        .errors
                        .lock()
                        .unwrap()
                        .insert(id.clone(), error.message.clone());
                }
            }
            m.public_access.verify.lock().unwrap().remove(&id);
            m.public_access.errors.lock().unwrap().remove(&id);
            continue;
        }
        if !local_ready(m, &id).await {
            continue;
        }
        if !info.running {
            if !configured(&info.config) {
                continue;
            }
            match tunnel::start_saved(m, &id).await {
                Ok(_) => {
                    // A restarted frpc must be probed again before its address
                    // is reported as verified.
                    m.public_access.verify.lock().unwrap().remove(&id);
                    m.public_access.errors.lock().unwrap().remove(&id);
                }
                Err(error) => {
                    m.logger
                        .log("WARN", &format!("{id}: custom frpc failed: {error}"));
                    m.public_access
                        .errors
                        .lock()
                        .unwrap()
                        .insert(id.clone(), error.message.clone());
                }
            }
            continue;
        }
        m.public_access.errors.lock().unwrap().remove(&id);
        if target.protocol == Protocol::Udp {
            continue;
        }
        let now = Instant::now();
        let due = {
            let mut verify = m.public_access.verify.lock().unwrap();
            match verify.get(&id) {
                Some(entry) if entry.address == info.config.address => entry.next <= now,
                _ => {
                    verify.insert(
                        id.clone(),
                        Verification {
                            address: info.config.address.clone(),
                            verified: false,
                            next: now,
                        },
                    );
                    true
                }
            }
        };
        if !due {
            continue;
        }
        let verified = match host_port(&info.config.address) {
            Some((host, port)) => playit::probe(&host, port).await,
            None => false,
        };
        m.public_access.verify.lock().unwrap().insert(
            id,
            Verification {
                address: info.config.address,
                verified,
                next: now + Duration::from_secs(if verified { 60 } else { 8 }),
            },
        );
    }
    Ok(())
}

pub async fn shutdown(m: &Manager) {
    let task = m.public_access.task.lock().unwrap().take();
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_and_protocols_round_trip() {
        for value in ["lan", "sakura", "playit", "custom"] {
            assert_eq!(Provider::parse(value).unwrap().as_str(), value);
        }
        for value in ["tcp", "udp"] {
            assert_eq!(Protocol::parse(value).unwrap().as_str(), value);
        }
        assert!(Provider::parse("unknown").is_none());
        assert!(Protocol::parse("icmp").is_none());
        assert_eq!(Provider::default(), Provider::Lan);
        assert_eq!(Protocol::default(), Protocol::Tcp);
    }

    #[test]
    fn unsafe_or_incomplete_custom_addresses_are_rejected() {
        assert_eq!(
            host_port("play.example.com:25565"),
            Some(("play.example.com".into(), 25565))
        );
        assert_eq!(
            host_port("[2001:db8::1]:25565"),
            Some(("2001:db8::1".into(), 25565))
        );
        for address in ["", "example.com", "evil/path:25565", "host:0", "host:99999"] {
            assert!(host_port(address).is_none(), "{address}");
        }
    }

    #[test]
    fn every_provider_is_registered_once() {
        for kind in [
            Provider::Lan,
            Provider::Sakura,
            Provider::Playit,
            Provider::Custom,
        ] {
            assert_eq!(provider(kind).kind(), kind);
        }
        let mut kinds: Vec<_> = PROVIDERS.iter().map(|p| p.kind()).collect();
        kinds.sort_by_key(|kind| kind.as_str());
        kinds.dedup();
        assert_eq!(kinds.len(), 4);
    }

    #[test]
    fn config_defaults_to_lan_over_tcp() {
        let config: Config = serde_json::from_str(r#"{"servers":{}}"#).unwrap();
        assert!(config.servers.is_empty());
        let entry: Entry = serde_json::from_str("{}").unwrap();
        assert_eq!(entry.provider, Provider::Lan);
        assert_eq!(entry.protocol, Protocol::Tcp);
        assert!(entry.local_port.is_none());
    }
}
