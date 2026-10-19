use crate::{
    error::{AppError, Result},
    models::Status,
    server::manager::Manager,
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::Ordering,
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use sysinfo::{Pid, ProcessesToUpdate, System};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(3);
const MAX_SAMPLES: usize = 60;
const MAX_ALERTS: usize = 50;
const STREAK_REQUIRED: u8 = 3;
const ALERT_COOLDOWN: Duration = Duration::from_secs(300);
const LAG_WINDOW_SECS: i64 = 300;
const LAG_MARKER: &str = "Can't keep up";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum AlertKind {
    Cpu,
    Memory,
    Lag,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertRules {
    pub enabled: bool,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub lag_events: u32,
}
impl Default for AlertRules {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_percent: 90.0,
            memory_percent: 90.0,
            lag_events: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerMetrics {
    pub id: String,
    pub sampled_at: String,
    pub status: Status,
    pub cpu_percent: f32,
    pub memory_mb: u64,
    pub memory_limit_mb: u32,
    pub memory_percent: f32,
    pub players: u32,
    pub max_players: u32,
    pub uptime_secs: u64,
    pub lag_events: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsSample {
    pub at: String,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub players: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerAlert {
    pub id: String,
    pub server_id: String,
    pub at: String,
    pub kind: AlertKind,
    pub value: f32,
    pub threshold: f32,
}

pub struct MonitorState {
    system: System,
    latest: Option<ServerMetrics>,
    history: VecDeque<MetricsSample>,
    alerts: VecDeque<ServerAlert>,
    rules: AlertRules,
    streak: HashMap<AlertKind, u8>,
    last_alert: HashMap<AlertKind, Instant>,
    run_started: Option<String>,
}
impl MonitorState {
    fn new(rules: AlertRules) -> Self {
        Self {
            system: System::new(),
            latest: None,
            history: VecDeque::new(),
            alerts: VecDeque::new(),
            rules,
            streak: HashMap::new(),
            last_alert: HashMap::new(),
            run_started: None,
        }
    }
    fn reset_run(&mut self) {
        self.latest = None;
        self.history.clear();
        self.alerts.clear();
        self.streak.clear();
        self.last_alert.clear();
    }
}

fn rules_path(m: &Manager, id: &str) -> Result<std::path::PathBuf> {
    storage::child(&m.dir(id)?, "monitor.json")
}

fn read_rules(m: &Manager, id: &str) -> Result<AlertRules> {
    let path = rules_path(m, id)?;
    if !path.exists() {
        return Ok(AlertRules::default());
    }
    let saved: AlertRules = serde_json::from_slice(&std::fs::read(path)?)?;
    validate_rules(&saved)?;
    Ok(saved)
}

fn validate_rules(rules: &AlertRules) -> Result<()> {
    if !(10.0..=100.0).contains(&rules.cpu_percent) {
        return Err(AppError::user(
            "Choose a CPU alert threshold between 10% and 100%.",
        ));
    }
    if !(10.0..=100.0).contains(&rules.memory_percent) {
        return Err(AppError::user(
            "Choose a memory alert threshold between 10% and 100%.",
        ));
    }
    if !(1..=100).contains(&rules.lag_events) {
        return Err(AppError::user(
            "Choose between 1 and 100 tick-lag warnings per five minutes.",
        ));
    }
    Ok(())
}

pub fn state(m: &Manager, id: &str) -> Result<Arc<Mutex<MonitorState>>> {
    m.runtime(id)?;
    if let Some(found) = m
        .monitors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
    {
        return Ok(found);
    }
    let rules = read_rules(m, id)?;
    let mut monitors = m.monitors.lock().unwrap_or_else(|e| e.into_inner());
    Ok(monitors
        .entry(id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(MonitorState::new(rules))))
        .clone())
}

pub fn latest(m: &Manager, id: &str) -> Result<Option<ServerMetrics>> {
    if !matches!(
        m.runtime(id)?.snapshot().status,
        Status::Starting | Status::Running | Status::Stopping
    ) {
        return Ok(None);
    }
    Ok(state(m, id)?
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .latest
        .clone())
}

pub fn history(m: &Manager, id: &str) -> Result<Vec<MetricsSample>> {
    Ok(state(m, id)?
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .history
        .iter()
        .cloned()
        .collect())
}

pub fn alerts(m: &Manager, id: &str) -> Result<Vec<ServerAlert>> {
    Ok(state(m, id)?
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .alerts
        .iter()
        .cloned()
        .collect())
}

pub fn clear_alerts(m: &Manager, id: &str) -> Result<()> {
    state(m, id)?
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .alerts
        .clear();
    Ok(())
}

pub fn rules(m: &Manager, id: &str) -> Result<AlertRules> {
    Ok(state(m, id)?
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .rules
        .clone())
}

pub fn save_rules(m: &Manager, id: &str, rules: AlertRules) -> Result<AlertRules> {
    validate_rules(&rules)?;
    let state = state(m, id)?;
    storage::write_json(&rules_path(m, id)?, &rules)?;
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .rules
        .clone_from(&rules);
    Ok(rules)
}

/// Samples the managed Java process and evaluates alert thresholds.
pub fn sample(m: &Manager, id: &str) -> Result<Option<ServerMetrics>> {
    let runtime = m.runtime(id)?;
    let snapshot = runtime.snapshot();
    if !matches!(
        snapshot.status,
        Status::Starting | Status::Running | Status::Stopping
    ) {
        let state = state(m, id)?;
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        state.latest = None;
        state.streak.clear();
        state.run_started = None;
        return Ok(None);
    }
    let max_players = m
        .properties(id)
        .ok()
        .and_then(|p| p.values.get("max-players").and_then(|v| v.parse().ok()))
        .unwrap_or(20);
    let limit = m.metadata(id)?.memory_max_mb.max(1);
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1) as f32;
    let lag_events = lag_events(&runtime);
    let mem_limit_mb = limit as f32;
    let state = state(m, id)?;
    let alerts = {
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        if state.run_started != snapshot.started_at {
            state.reset_run();
            state.run_started = snapshot.started_at.clone();
        }
        let (mut cpu, mut memory) = (0.0f32, 0u64);
        if let Some(pid) = snapshot.pid {
            let pid = Pid::from_u32(pid);
            state
                .system
                .refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
            if let Some(process) = state.system.process(pid) {
                cpu = (process.cpu_usage() / cores).clamp(0.0, 100.0);
                memory = process.memory() / 1024 / 1024;
            }
        }
        let memory_percent = (memory as f32 / mem_limit_mb * 100.0).clamp(0.0, 100.0);
        let now = chrono::Utc::now();
        let metrics = ServerMetrics {
            id: id.into(),
            sampled_at: now.to_rfc3339(),
            status: snapshot.status,
            cpu_percent: cpu,
            memory_mb: memory,
            memory_limit_mb: limit,
            memory_percent,
            players: snapshot.players.len() as u32,
            max_players,
            uptime_secs: uptime(&snapshot.started_at, now),
            lag_events,
        };
        state.history.push_back(MetricsSample {
            at: metrics.sampled_at.clone(),
            cpu_percent: metrics.cpu_percent,
            memory_percent: metrics.memory_percent,
            players: metrics.players,
        });
        while state.history.len() > MAX_SAMPLES {
            state.history.pop_front();
        }
        state.latest = Some(metrics.clone());
        evaluate(&mut state, &metrics)
    };
    m.emit("server-metrics", metrics_payload(id, &state));
    for alert in alerts {
        m.emit("server-alert", alert);
    }
    Ok(latest(m, id)?)
}

/// Re-emits the newest sample; avoids serialising the state guard.
fn metrics_payload(id: &str, state: &Arc<Mutex<MonitorState>>) -> ServerMetrics {
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .latest
        .clone()
        .unwrap_or(ServerMetrics {
            id: id.into(),
            sampled_at: chrono::Utc::now().to_rfc3339(),
            status: Status::Running,
            cpu_percent: 0.0,
            memory_mb: 0,
            memory_limit_mb: 0,
            memory_percent: 0.0,
            players: 0,
            max_players: 0,
            uptime_secs: 0,
            lag_events: 0,
        })
}

fn lag_events(runtime: &crate::server::manager::Runtime) -> u32 {
    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(LAG_WINDOW_SECS);
    runtime
        .console
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|line| {
            line.text.contains(LAG_MARKER)
                && chrono::DateTime::parse_from_rfc3339(&line.timestamp)
                    .map(|at| at.with_timezone(&chrono::Utc) >= cutoff)
                    .unwrap_or(false)
        })
        .count() as u32
}

fn uptime(started_at: &Option<String>, now: chrono::DateTime<chrono::Utc>) -> u64 {
    started_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|start| (now - start.with_timezone(&chrono::Utc)).num_seconds().max(0) as u64)
        .unwrap_or(0)
}

fn evaluate(state: &mut MonitorState, metrics: &ServerMetrics) -> Vec<ServerAlert> {
    let mut out = Vec::new();
    if !state.rules.enabled || metrics.status != Status::Running {
        state.streak.clear();
        return out;
    }
    let checks = [
        (
            AlertKind::Cpu,
            metrics.cpu_percent,
            state.rules.cpu_percent,
        ),
        (
            AlertKind::Memory,
            metrics.memory_percent,
            state.rules.memory_percent,
        ),
        (
            AlertKind::Lag,
            metrics.lag_events as f32,
            state.rules.lag_events as f32,
        ),
    ];
    for (kind, value, threshold) in checks {
        let streak = state.streak.entry(kind).or_insert(0);
        if value >= threshold {
            *streak = streak.saturating_add(1);
        } else {
            *streak = 0;
        }
        if *streak < STREAK_REQUIRED {
            continue;
        }
        *streak = 0;
        let cooled = state
            .last_alert
            .get(&kind)
            .map(|at| at.elapsed() >= ALERT_COOLDOWN)
            .unwrap_or(true);
        if !cooled {
            continue;
        }
        state.last_alert.insert(kind, Instant::now());
        let alert = ServerAlert {
            id: uuid::Uuid::new_v4().to_string(),
            server_id: metrics.id.clone(),
            at: chrono::Utc::now().to_rfc3339(),
            kind,
            value,
            threshold,
        };
        state.alerts.push_front(alert.clone());
        while state.alerts.len() > MAX_ALERTS {
            state.alerts.pop_back();
        }
        out.push(alert);
    }
    out
}

pub fn spawn(m: Arc<Manager>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(SAMPLE_INTERVAL).await;
            if m.shutting_down.load(Ordering::SeqCst) {
                continue;
            }
            let ids: Vec<String> = m
                .runtimes
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, r)| {
                    matches!(
                        r.snapshot().status,
                        Status::Starting | Status::Running | Status::Stopping
                    )
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in ids {
                if let Err(e) = sample(&m, &id) {
                    m.logger
                        .log("WARN", &format!("Monitoring failed for {id}: {e}"));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(cpu: f32, memory: f32, lag: u32) -> ServerMetrics {
        ServerMetrics {
            id: "fixture".into(),
            sampled_at: chrono::Utc::now().to_rfc3339(),
            status: Status::Running,
            cpu_percent: cpu,
            memory_mb: 512,
            memory_limit_mb: 1024,
            memory_percent: memory,
            players: 1,
            max_players: 20,
            uptime_secs: 10,
            lag_events: lag,
        }
    }

    #[test]
    fn rule_bounds() {
        assert!(validate_rules(&AlertRules::default()).is_ok());
        for bad in [
            AlertRules {
                cpu_percent: 5.0,
                ..AlertRules::default()
            },
            AlertRules {
                memory_percent: 101.0,
                ..AlertRules::default()
            },
            AlertRules {
                lag_events: 0,
                ..AlertRules::default()
            },
        ] {
            assert!(validate_rules(&bad).is_err());
        }
    }

    #[test]
    fn alerts_need_consecutive_breaches() {
        let mut state = MonitorState::new(AlertRules::default());
        assert!(evaluate(&mut state, &metrics(95.0, 20.0, 0)).is_empty());
        assert!(evaluate(&mut state, &metrics(95.0, 20.0, 0)).is_empty());
        let fired = evaluate(&mut state, &metrics(95.0, 20.0, 0));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].kind, AlertKind::Cpu);
        // Cooldown suppresses an immediate repeat.
        for _ in 0..STREAK_REQUIRED {
            assert!(evaluate(&mut state, &metrics(99.0, 20.0, 0)).is_empty());
        }
    }

    #[test]
    fn disabled_rules_and_streak_reset() {
        let mut state = MonitorState::new(AlertRules {
            enabled: false,
            ..AlertRules::default()
        });
        for _ in 0..10 {
            assert!(evaluate(&mut state, &metrics(100.0, 100.0, 99)).is_empty());
        }
        state.rules.enabled = true;
        assert!(evaluate(&mut state, &metrics(95.0, 20.0, 0)).is_empty());
        assert!(evaluate(&mut state, &metrics(10.0, 20.0, 0)).is_empty());
        assert!(evaluate(&mut state, &metrics(95.0, 20.0, 0)).is_empty());
        assert!(evaluate(&mut state, &metrics(95.0, 20.0, 0)).is_empty());
    }
}
