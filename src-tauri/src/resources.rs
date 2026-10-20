//! Workspace-wide memory planning, port checks and the queued starter.
//!
//! The queue starts only servers that are already installed and stopped, one
//! after another with a configurable stagger. It never stops anything and
//! cancelling only prevents servers that have not started yet. Every start
//! goes through the same `Manager::start` path as a manual start, so EULA,
//! port, Java and file checks still apply. Memory is assessed against the
//! configured maxima and the system's total RAM; per-step free memory is
//! checked again immediately before each launch.

use crate::{
    error::{AppError, Result},
    models::{Server, Status},
    network,
    server::manager::{memory_info, Manager},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub const BUDGET_PERCENT: u64 = 90;
pub const MEMORY_RESERVE_MB: u64 = 512;
pub const MAX_STAGGER_SECS: u64 = 120;
pub const MAX_WAIT_READY_SECS: u64 = 180;
const DEFAULT_STAGGER_SECS: u64 = 10;
const DEFAULT_WAIT_READY_SECS: u64 = 60;
const MIN_PORT: u16 = 1024;
const POLL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartOptions {
    #[serde(default = "default_stagger")]
    pub stagger_secs: u64,
    #[serde(default = "default_wait")]
    pub wait_ready_secs: u64,
    /// Start even when the summed maxima exceed the workspace budget.
    #[serde(default)]
    pub force: bool,
}

fn default_stagger() -> u64 {
    DEFAULT_STAGGER_SECS
}

fn default_wait() -> u64 {
    DEFAULT_WAIT_READY_SECS
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            stagger_secs: DEFAULT_STAGGER_SECS,
            wait_ready_secs: DEFAULT_WAIT_READY_SECS,
            force: false,
        }
    }
}

impl StartOptions {
    pub fn normalize(&self) -> Result<Self> {
        if self.stagger_secs > MAX_STAGGER_SECS || self.wait_ready_secs > MAX_WAIT_READY_SECS {
            return Err(AppError::user(format!(
                "Choose a stagger of 0–{MAX_STAGGER_SECS} seconds and a readiness wait of 0–{MAX_WAIT_READY_SECS} seconds."
            ))
            .with_code("resources.invalidOptions"));
        }
        Ok(self.clone())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceServer {
    pub id: String,
    pub name: String,
    pub status: Status,
    pub installed: bool,
    pub active: bool,
    pub memory_mb: u32,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PortIssueKind {
    Duplicate,
    Occupied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortIssue {
    pub id: String,
    pub name: String,
    pub port: u16,
    pub kind: PortIssueKind,
    pub suggestion: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceReport {
    pub total_mb: u64,
    pub available_mb: u64,
    pub recommended_mb: u32,
    /// Sum of maximum heaps that may run before the 90% system budget is hit.
    pub budget_mb: u64,
    pub running_mb: u64,
    pub idle_mb: u64,
    pub projected_mb: u64,
    pub headroom_mb: u64,
    pub budget_ok: bool,
    pub tight: bool,
    pub servers: Vec<ResourceServer>,
    pub ports: Vec<PortIssue>,
    pub queue: Option<QueueState>,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub memory_mb: u32,
    pub port: u16,
    pub installed: bool,
    pub ready: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub id: String,
    pub name: String,
    pub memory_mb: u32,
    pub port: u16,
    pub ready: bool,
    pub reason: Option<String>,
    /// Nominal delay from queue start, based on the stagger and ready entries.
    pub start_after_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartPlan {
    pub entries: Vec<PlanEntry>,
    pub startable: usize,
    pub requested_mb: u64,
    pub running_mb: u64,
    pub projected_mb: u64,
    pub budget_mb: u64,
    pub budget_ok: bool,
    pub stagger_secs: u64,
    pub wait_ready_secs: u64,
    pub force: bool,
}

/// Determines the servers whose ports collide with another queued server or
/// with something already listening. Returned indices refer to `candidates`.
pub fn port_conflicts(candidates: &[Candidate], running_ports: &HashSet<u16>) -> HashSet<usize> {
    let mut counts: HashMap<u16, usize> = HashMap::new();
    for candidate in candidates.iter().filter(|candidate| candidate.installed) {
        *counts.entry(candidate.port).or_default() += 1;
    }
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            running_ports.contains(&candidate.port)
                || counts.get(&candidate.port).copied().unwrap_or(0) > 1
        })
        .map(|(index, _)| index)
        .collect()
}

/// Finds a nearby free port, preferring `preferred` and then wrapping back
/// into the conventional Minecraft range.
pub fn suggest_port(
    used: &BTreeSet<u16>,
    preferred: u16,
    is_free: impl Fn(u16) -> bool,
) -> Option<u16> {
    let start = preferred.max(MIN_PORT);
    (start..u16::MAX)
        .chain(25565..start)
        .take(4096)
        .find(|port| *port >= MIN_PORT && !used.contains(port) && is_free(*port))
}

pub fn assess(
    candidates: &[Candidate],
    running_mb: u64,
    budget_mb: u64,
    options: &StartOptions,
) -> StartPlan {
    let mut entries = Vec::with_capacity(candidates.len());
    let mut startable = 0usize;
    let mut requested_mb = 0u64;
    for candidate in candidates {
        let start_after_secs = if candidate.ready {
            let offset = startable as u64 * options.stagger_secs;
            startable += 1;
            requested_mb += candidate.memory_mb as u64;
            offset
        } else {
            0
        };
        entries.push(PlanEntry {
            id: candidate.id.clone(),
            name: candidate.name.clone(),
            memory_mb: candidate.memory_mb,
            port: candidate.port,
            ready: candidate.ready,
            reason: candidate.reason.clone(),
            start_after_secs,
        });
    }
    let projected_mb = running_mb + requested_mb;
    StartPlan {
        entries,
        startable,
        requested_mb,
        running_mb,
        projected_mb,
        budget_mb,
        budget_ok: projected_mb <= budget_mb,
        stagger_secs: options.stagger_secs,
        wait_ready_secs: options.wait_ready_secs,
        force: options.force,
    }
}

pub fn build_report(
    servers: Vec<ResourceServer>,
    unavailable: &HashSet<u16>,
    total_mb: u64,
    available_mb: u64,
    recommended_mb: u32,
    queue: Option<QueueState>,
) -> ResourceReport {
    let budget_mb = total_mb * BUDGET_PERCENT / 100;
    let running_mb: u64 = servers
        .iter()
        .filter(|server| server.active)
        .map(|server| server.memory_mb as u64)
        .sum();
    let idle_mb: u64 = servers
        .iter()
        .filter(|server| !server.active && server.installed)
        .map(|server| server.memory_mb as u64)
        .sum();
    let projected_mb = running_mb + idle_mb;
    ResourceReport {
        total_mb,
        available_mb,
        recommended_mb,
        budget_mb,
        running_mb,
        idle_mb,
        projected_mb,
        headroom_mb: budget_mb.saturating_sub(running_mb),
        budget_ok: projected_mb <= budget_mb,
        tight: projected_mb + MEMORY_RESERVE_MB > budget_mb || available_mb < 1024,
        ports: port_issues(&servers, unavailable),
        servers,
        queue,
    }
}

fn port_issues(servers: &[ResourceServer], unavailable: &HashSet<u16>) -> Vec<PortIssue> {
    let idle: Vec<&ResourceServer> = servers
        .iter()
        .filter(|server| server.installed && !server.active)
        .collect();
    let mut counts: HashMap<u16, usize> = HashMap::new();
    for server in servers.iter().filter(|server| server.installed) {
        *counts.entry(server.port).or_default() += 1;
    }
    let mut used: BTreeSet<u16> = servers
        .iter()
        .filter(|server| server.installed)
        .map(|server| server.port)
        .collect();
    let mut issues = Vec::new();
    for server in idle {
        let duplicate = counts.get(&server.port).copied().unwrap_or(0) > 1;
        let occupied = unavailable.contains(&server.port);
        if !duplicate && !occupied {
            continue;
        }
        let suggestion = suggest_port(&used, server.port.saturating_add(1).max(MIN_PORT), |port| {
            !unavailable.contains(&port)
        });
        if let Some(port) = suggestion {
            used.insert(port);
        }
        issues.push(PortIssue {
            id: server.id.clone(),
            name: server.name.clone(),
            port: server.port,
            kind: if duplicate {
                PortIssueKind::Duplicate
            } else {
                PortIssueKind::Occupied
            },
            suggestion,
        });
    }
    issues
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    Queued,
    Waiting,
    Starting,
    Started,
    Failed,
    Skipped,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    pub id: String,
    pub name: String,
    pub memory_mb: u32,
    pub port: u16,
    pub step: Step,
    pub detail: Option<String>,
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueState {
    pub running: bool,
    pub cancelled: bool,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub stagger_secs: u64,
    pub wait_ready_secs: u64,
    pub memory_mb: u64,
    pub budget_mb: u64,
    pub entries: Vec<QueueEntry>,
}

pub struct QueueHandle {
    state: Mutex<QueueState>,
    cancel: AtomicBool,
}

impl QueueHandle {
    fn state(&self) -> QueueState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub fn state(m: &Manager) -> Option<QueueState> {
    m.queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|handle| handle.state())
}

fn failure(message: impl Into<String>, code: &str) -> AppError {
    AppError::user(message).with_code(code)
}

fn load(m: &Manager, ids: &[String]) -> Result<(Vec<Candidate>, u64, u64)> {
    if ids.is_empty() {
        return Err(failure(
            "Select at least one server to include in the queue.",
            "resources.nothingToStart",
        ));
    }
    let (total_mb, _, _) = memory_info();
    let budget_mb = total_mb * BUDGET_PERCENT / 100;
    let views = m.list()?;
    let running_mb: u64 = views
        .iter()
        .filter(|view| view.runtime.status.active())
        .map(|view| view.server.memory_max_mb as u64)
        .sum();
    let running_ports: HashSet<u16> = views
        .iter()
        .filter(|view| view.runtime.status.active())
        .map(|view| view.server.port)
        .collect();
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let view = views
            .iter()
            .find(|view| &view.server.id == id)
            .ok_or_else(|| failure("This server could not be found.", "server.notFound"))?;
        let server: &Server = &view.server;
        let status = view.runtime.status;
        let active = status.active() || status == Status::Installing;
        let ready = server.installed && !active;
        let reason = if !server.installed {
            Some("server.notInstalled".to_string())
        } else if active {
            Some("resources.busy".to_string())
        } else {
            None
        };
        candidates.push(Candidate {
            id: server.id.clone(),
            name: server.name.clone(),
            memory_mb: server.memory_max_mb,
            port: server.port,
            installed: server.installed,
            ready,
            reason,
        });
    }
    let conflicts = port_conflicts(&candidates, &running_ports);
    for (index, candidate) in candidates.iter_mut().enumerate() {
        if !candidate.ready {
            continue;
        }
        if conflicts.contains(&index) {
            candidate.ready = false;
            candidate.reason = Some("resources.portConflict".into());
        } else if !network::port_free(candidate.port) {
            candidate.ready = false;
            candidate.reason = Some("resources.portOccupied".into());
        }
    }
    Ok((candidates, running_mb, budget_mb))
}

pub fn plan(m: &Manager, ids: &[String], options: StartOptions) -> Result<StartPlan> {
    let options = options.normalize()?;
    let (candidates, running_mb, budget_mb) = load(m, ids)?;
    Ok(assess(&candidates, running_mb, budget_mb, &options))
}

pub fn start(m: &Arc<Manager>, ids: Vec<String>, options: StartOptions) -> Result<QueueState> {
    m.ready()?;
    m.ensure_writable()?;
    let options = options.normalize()?;
    let mut slot = m.queue.lock().unwrap_or_else(|e| e.into_inner());
    if slot
        .as_ref()
        .map(|handle| handle.state().running)
        .unwrap_or(false)
    {
        return Err(failure(
            "A start queue is already running. Wait for it to finish or cancel it.",
            "resources.queueRunning",
        ));
    }
    let (candidates, running_mb, budget_mb) = load(m, &ids)?;
    let plan = assess(&candidates, running_mb, budget_mb, &options);
    if plan.startable == 0 {
        return Err(failure(
            "None of the selected servers can start right now. Check installation, state and ports.",
            "resources.nothingToStart",
        ));
    }
    if !plan.budget_ok && !options.force {
        return Err(failure(
            "The selected servers exceed the memory budget. Lower the RAM limits, choose fewer servers, or allow exceeding the budget.",
            "resources.memoryBudget",
        ));
    }
    let entries = candidates
        .iter()
        .zip(plan.entries.iter())
        .map(|(candidate, entry)| QueueEntry {
            id: candidate.id.clone(),
            name: candidate.name.clone(),
            memory_mb: candidate.memory_mb,
            port: candidate.port,
            step: if entry.ready {
                Step::Queued
            } else {
                Step::Skipped
            },
            detail: entry.reason.clone(),
            started_at: None,
        })
        .collect();
    let queue = QueueState {
        running: true,
        cancelled: false,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        stagger_secs: options.stagger_secs,
        wait_ready_secs: options.wait_ready_secs,
        memory_mb: plan.requested_mb,
        budget_mb,
        entries,
    };
    let handle = Arc::new(QueueHandle {
        state: Mutex::new(queue.clone()),
        cancel: AtomicBool::new(false),
    });
    *slot = Some(handle.clone());
    drop(slot);
    let manager = m.clone();
    tauri::async_runtime::spawn(async move {
        run(manager, handle, options).await;
    });
    Ok(queue)
}

pub fn cancel(m: &Manager) -> Result<QueueState> {
    let handle = m
        .queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| failure("No start queue has run yet.", "resources.queueIdle"))?;
    if !handle.state().running {
        return Err(failure(
            "The start queue has already finished.",
            "resources.queueIdle",
        ));
    }
    handle.cancel.store(true, Ordering::SeqCst);
    let mut state = handle.state.lock().unwrap_or_else(|e| e.into_inner());
    state.cancelled = true;
    Ok(state.clone())
}

fn publish(m: &Manager, handle: &QueueHandle) {
    m.emit("start-queue", handle.state());
}

fn update(handle: &QueueHandle, id: &str, step: Step, detail: Option<String>) {
    let mut state = handle.state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = state.entries.iter_mut().find(|entry| entry.id == id) {
        entry.step = step;
        if detail.is_some() {
            entry.detail = detail;
        }
        if step == Step::Starting {
            entry.started_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }
}

fn sweep(handle: &QueueHandle, step: Step, detail: &str) {
    let mut state = handle.state.lock().unwrap_or_else(|e| e.into_inner());
    for entry in state.entries.iter_mut() {
        if matches!(entry.step, Step::Queued | Step::Waiting) {
            entry.step = step;
            entry.detail = Some(detail.to_string());
        }
    }
}

/// Sleeps in short slices so a cancel request is noticed quickly. Returns
/// false when the queue was cancelled.
async fn pause(handle: &QueueHandle, total: Duration) -> bool {
    let deadline = Instant::now() + total;
    loop {
        if handle.cancel.load(Ordering::SeqCst) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        tokio::time::sleep((deadline - now).min(Duration::from_millis(250))).await;
    }
}

enum Wait {
    Running,
    Exited(String),
    Timeout,
    Cancelled,
}

async fn wait_ready(m: &Arc<Manager>, id: &str, secs: u64, handle: &QueueHandle) -> Wait {
    if secs == 0 {
        return Wait::Timeout;
    }
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if handle.cancel.load(Ordering::SeqCst) {
            return Wait::Cancelled;
        }
        match m.runtime(id) {
            Ok(runtime) => {
                let snapshot = runtime.snapshot();
                if snapshot.status == Status::Running {
                    return Wait::Running;
                }
                if matches!(
                    snapshot.status,
                    Status::Crashed | Status::Stopped | Status::InstallationFailed
                ) {
                    return Wait::Exited(
                        snapshot
                            .exit_reason
                            .or(snapshot.message)
                            .unwrap_or_else(|| "The server exited before it became ready.".into()),
                    );
                }
            }
            Err(e) => return Wait::Exited(e.message),
        }
        if Instant::now() >= deadline {
            return Wait::Timeout;
        }
        tokio::time::sleep(POLL).await;
    }
}

async fn run(m: Arc<Manager>, handle: Arc<QueueHandle>, options: StartOptions) {
    let entries: Vec<(String, u32)> = {
        let state = handle.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .entries
            .iter()
            .filter(|entry| entry.step == Step::Queued)
            .map(|entry| (entry.id.clone(), entry.memory_mb))
            .collect()
    };
    let mut launched = 0usize;
    for (index, (id, memory_mb)) in entries.iter().enumerate() {
        if handle.cancel.load(Ordering::SeqCst) {
            break;
        }
        if index > 0 {
            update(&handle, id, Step::Waiting, None);
            publish(&m, &handle);
            if !pause(&handle, Duration::from_secs(options.stagger_secs)).await {
                break;
            }
        }
        if handle.cancel.load(Ordering::SeqCst) {
            break;
        }
        let (_, available_mb, _) = memory_info();
        if !options.force && *memory_mb as u64 + MEMORY_RESERVE_MB > available_mb {
            update(
                &handle,
                id,
                Step::Skipped,
                Some(format!(
                    "Only {available_mb} MB is available; {memory_mb} MB plus a {MEMORY_RESERVE_MB} MB reserve was requested."
                )),
            );
            publish(&m, &handle);
            continue;
        }
        update(&handle, id, Step::Starting, None);
        publish(&m, &handle);
        if let Err(e) = m.start(id).await {
            update(&handle, id, Step::Failed, Some(e.message));
            publish(&m, &handle);
            continue;
        }
        launched += 1;
        match wait_ready(&m, id, options.wait_ready_secs, &handle).await {
            Wait::Running => {
                update(&handle, id, Step::Started, None);
            }
            Wait::Exited(reason) => {
                update(&handle, id, Step::Failed, Some(reason));
            }
            Wait::Timeout => {
                update(
                    &handle,
                    id,
                    Step::Started,
                    Some(format!(
                        "Java is still starting; readiness was not confirmed within {} seconds.",
                        options.wait_ready_secs
                    )),
                );
            }
            Wait::Cancelled => {
                update(
                    &handle,
                    id,
                    Step::Started,
                    Some(
                        "The queue was cancelled after this server started; it keeps running."
                            .into(),
                    ),
                );
                break;
            }
        }
        publish(&m, &handle);
    }
    let cancelled = handle.cancel.load(Ordering::SeqCst);
    if cancelled {
        sweep(
            &handle,
            Step::Cancelled,
            "The queue was cancelled before this server started.",
        );
    }
    {
        let mut state = handle.state.lock().unwrap_or_else(|e| e.into_inner());
        state.cancelled = cancelled;
        state.running = false;
        state.finished_at = Some(chrono::Utc::now().to_rfc3339());
    }
    publish(&m, &handle);
    m.logger.log(
        "INFO",
        &format!(
            "Start queue finished after launching {launched} server(s){}",
            if cancelled { " (cancelled)" } else { "" }
        ),
    );
}

pub fn report(m: &Manager) -> Result<ResourceReport> {
    let (total_mb, available_mb, recommended_mb) = memory_info();
    let views = m.list()?;
    let servers: Vec<ResourceServer> = views
        .iter()
        .map(|view| ResourceServer {
            id: view.server.id.clone(),
            name: view.server.name.clone(),
            status: view.runtime.status,
            installed: view.server.installed,
            active: view.runtime.status.active(),
            memory_mb: view.server.memory_max_mb,
            port: view.server.port,
        })
        .collect();
    let mut unavailable = HashSet::new();
    for server in &servers {
        if server.installed && !server.active && !network::port_free(server.port) {
            unavailable.insert(server.port);
        }
    }
    Ok(build_report(
        servers,
        &unavailable,
        total_mb,
        available_mb,
        recommended_mb,
        state(m),
    ))
}

pub async fn apply_port(m: &Arc<Manager>, id: &str, port: u16) -> Result<()> {
    m.ready()?;
    m.ensure_writable()?;
    if port < MIN_PORT {
        return Err(failure(
            "Choose a port between 1024 and 65535.",
            "resources.invalidPort",
        ));
    }
    let target = m.metadata(id)?;
    if target.port != port {
        let taken = m
            .list()?
            .into_iter()
            .any(|view| view.server.id != id && view.server.port == port);
        if taken {
            return Err(failure(
                "Another managed server already uses this port.",
                "resources.portTaken",
            ));
        }
        if !network::port_free(port) {
            return Err(AppError::user(
                "This port is already in use or unavailable. Pick another one.",
            )
            .with_code("network.portInUse"));
        }
    }
    let mut values = BTreeMap::new();
    values.insert("server-port".to_string(), port.to_string());
    m.save_properties(id, Some(values), None).await?;
    m.logger
        .log("INFO", &format!("Changed the port of {id} to {port}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, memory_mb: u32, port: u16, ready: bool) -> Candidate {
        Candidate {
            id: id.into(),
            name: id.to_uppercase(),
            memory_mb,
            port,
            installed: true,
            ready,
            reason: if ready {
                None
            } else {
                Some("server.notInstalled".into())
            },
        }
    }

    #[test]
    fn budget_uses_maxima_and_reports_overflow() {
        let candidates = vec![
            candidate("a", 2048, 25565, true),
            candidate("b", 4096, 25566, true),
        ];
        let options = StartOptions::default();
        let plan = assess(&candidates, 2048, 8192, &options);
        assert_eq!((plan.startable, plan.requested_mb), (2, 6144));
        assert_eq!(plan.projected_mb, 8192);
        assert!(plan.budget_ok);
        let tight = assess(&candidates, 4096, 8192, &options);
        assert!(!tight.budget_ok);
        assert_eq!(tight.projected_mb, 10240);
        let forced = assess(
            &candidates,
            4096,
            8192,
            &StartOptions {
                force: true,
                ..StartOptions::default()
            },
        );
        assert!(!forced.budget_ok);
    }

    #[test]
    fn stagger_counts_only_ready_servers() {
        let candidates = vec![
            candidate("a", 1024, 25565, true),
            candidate("b", 1024, 25566, false),
            candidate("c", 1024, 25567, true),
            candidate("d", 1024, 25568, true),
        ];
        let plan = assess(
            &candidates,
            0,
            16384,
            &StartOptions {
                stagger_secs: 15,
                wait_ready_secs: 0,
                force: false,
            },
        );
        let offsets: Vec<u64> = plan.entries.iter().map(|e| e.start_after_secs).collect();
        assert_eq!(offsets, vec![0, 0, 15, 30]);
        assert_eq!(plan.startable, 3);
        assert!(plan.entries[0].reason.is_none());
        assert_eq!(
            plan.entries[1].reason.as_deref(),
            Some("server.notInstalled")
        );
    }

    #[test]
    fn duplicates_and_running_ports_are_flagged() {
        let candidates = vec![
            candidate("a", 1024, 25565, true),
            candidate("b", 1024, 25565, true),
            candidate("c", 1024, 25567, true),
        ];
        let running: HashSet<u16> = [25567u16].into_iter().collect();
        let conflicts = port_conflicts(&candidates, &running);
        assert_eq!(conflicts.len(), 3);
        assert!(conflicts.contains(&0) && conflicts.contains(&1) && conflicts.contains(&2));
        assert!(port_conflicts(&candidates[..1], &HashSet::new()).is_empty());
        let mut missing = candidate("d", 1024, 25566, false);
        missing.installed = false;
        assert!(port_conflicts(
            &[candidate("e", 1024, 25566, true), missing],
            &HashSet::new()
        )
        .is_empty());
    }

    #[test]
    fn port_suggestions_skip_used_and_busy_ports() {
        let used: BTreeSet<u16> = [25566, 25567].into_iter().collect();
        let busy: HashSet<u16> = [25568].into_iter().collect();
        let suggestion = suggest_port(&used, 25565, |port| !busy.contains(&port));
        assert_eq!(suggestion, Some(25565));
        let used: BTreeSet<u16> = [25566, 25567, 25568].into_iter().collect();
        let suggestion = suggest_port(&used, 25566, |port| port != 25569);
        assert_eq!(suggestion, Some(25570));
        assert_eq!(suggest_port(&BTreeSet::new(), 25565, |_| false), None);
    }

    #[test]
    fn report_separates_running_and_idle_and_finds_issues() {
        let server = |id: &str, port: u16, active: bool| ResourceServer {
            id: id.into(),
            name: id.into(),
            status: if active {
                Status::Running
            } else {
                Status::Stopped
            },
            installed: true,
            active,
            memory_mb: 2048,
            port,
        };
        let servers = vec![
            server("a", 25565, true),
            server("b", 25565, false),
            server("c", 25570, false),
        ];
        let unavailable: HashSet<u16> = [25570].into_iter().collect();
        let report = build_report(servers, &unavailable, 16_384, 4096, 4096, None);
        assert_eq!((report.running_mb, report.idle_mb), (2048, 4096));
        assert_eq!(report.projected_mb, 6144);
        assert_eq!(report.budget_mb, 14_745);
        assert_eq!(report.headroom_mb, 12_697);
        assert!(report.budget_ok);
        assert!(!report.tight);
        assert_eq!(report.ports.len(), 2);
        assert_eq!(report.ports[0].kind, PortIssueKind::Duplicate);
        assert_eq!(report.ports[0].suggestion, Some(25566));
        assert_eq!(report.ports[1].kind, PortIssueKind::Occupied);
        assert_eq!(report.ports[1].suggestion, Some(25571));
    }

    #[test]
    fn report_marks_a_tight_workspace() {
        let servers = vec![ResourceServer {
            id: "a".into(),
            name: "A".into(),
            status: Status::Stopped,
            installed: true,
            active: false,
            memory_mb: 8192,
            port: 25565,
        }];
        let report = build_report(servers, &HashSet::new(), 8192, 1400, 2048, None);
        assert_eq!(report.budget_mb, 7372);
        assert!(!report.budget_ok);
        assert!(report.tight);
    }

    #[test]
    fn options_are_bounded() {
        assert!(StartOptions::default().normalize().is_ok());
        assert_eq!(
            StartOptions {
                stagger_secs: MAX_STAGGER_SECS + 1,
                ..StartOptions::default()
            }
            .normalize()
            .unwrap_err()
            .code,
            "resources.invalidOptions"
        );
        assert_eq!(
            StartOptions {
                wait_ready_secs: MAX_WAIT_READY_SECS + 1,
                ..StartOptions::default()
            }
            .normalize()
            .unwrap_err()
            .code,
            "resources.invalidOptions"
        );
    }

    #[tokio::test]
    async fn cancellation_stops_the_pause_and_marks_pending_entries() {
        let queue = QueueState {
            running: true,
            cancelled: false,
            started_at: "2026-10-20T00:00:00Z".into(),
            finished_at: None,
            stagger_secs: 10,
            wait_ready_secs: 30,
            memory_mb: 2048,
            budget_mb: 8192,
            entries: vec![
                QueueEntry {
                    id: "a".into(),
                    name: "A".into(),
                    memory_mb: 1024,
                    port: 25565,
                    step: Step::Waiting,
                    detail: None,
                    started_at: None,
                },
                QueueEntry {
                    id: "b".into(),
                    name: "B".into(),
                    memory_mb: 1024,
                    port: 25566,
                    step: Step::Queued,
                    detail: None,
                    started_at: None,
                },
            ],
        };
        let handle = QueueHandle {
            state: Mutex::new(queue),
            cancel: AtomicBool::new(false),
        };
        handle.cancel.store(true, Ordering::SeqCst);
        assert!(!pause(&handle, Duration::from_secs(30)).await);
        sweep(&handle, Step::Cancelled, "The queue was cancelled.");
        let state = handle.state();
        assert!(state.entries.iter().all(|e| e.step == Step::Cancelled));
    }

    #[test]
    fn empty_selection_produces_an_empty_plan() {
        let plan = assess(&[], 0, 1024, &StartOptions::default());
        assert_eq!(plan.startable, 0);
        assert_eq!(plan.requested_mb, 0);
        assert!(plan.budget_ok);
    }
}
