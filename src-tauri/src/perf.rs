//! Local performance history and profiling.
//!
//! TPS and MSPT are read from the server's own RCON responses (`tick query`,
//! Paper's `tps` or Spark's `spark tps`). Sampling is opt-in and stored in the
//! server folder, so nothing leaves the machine. JFR recordings are started
//! only on explicit request through the JDK `jcmd` shipped beside the configured
//! Java executable; recordings stay in `profiles/` inside the server folder.
//! Runtime downloads are JREs, which have no `jcmd`, so the tool is probed
//! first and the UI reports that limitation instead of offering a dead button.

use crate::{
    error::{AppError, Result},
    models::{Server, ServerType, Status},
    rcon,
    server::manager::Manager,
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::process::Command;

pub const MIN_INTERVAL_SECS: u64 = 5;
pub const MAX_INTERVAL_SECS: u64 = 60;
const DEFAULT_INTERVAL_SECS: u64 = 10;
const MAX_SAMPLES: usize = 720;
const FLUSH_EVERY: usize = 6;
const LOOP_SECS: u64 = 5;
const TOOL_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TOOL_OUTPUT: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
}

fn default_interval() -> u64 {
    DEFAULT_INTERVAL_SECS
}

impl Default for PerfConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: DEFAULT_INTERVAL_SECS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfSample {
    pub id: String,
    pub at: String,
    #[serde(default)]
    pub tps1m: Option<f32>,
    #[serde(default)]
    pub tps5m: Option<f32>,
    #[serde(default)]
    pub tps15m: Option<f32>,
    #[serde(default)]
    pub mspt: Option<f32>,
    #[serde(default)]
    pub players: u32,
    #[serde(default)]
    pub memory_mb: Option<u64>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfIssue {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub name: String,
    pub file: String,
    pub started_at: String,
    pub duration_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileFile {
    pub name: String,
    pub size_bytes: u64,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfReport {
    pub id: String,
    pub enabled: bool,
    pub interval_secs: u64,
    pub samples: Vec<PerfSample>,
    pub latest: Option<PerfSample>,
    pub last_error: Option<PerfIssue>,
    /// RCON is enabled and the process is ready, so sampling can succeed.
    pub rcon_ready: bool,
    pub spark: bool,
    pub recordings: Vec<Recording>,
    pub profiles: Vec<ProfileFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JfrTools {
    pub java_path: String,
    pub jcmd: Option<String>,
    pub available: bool,
    /// Stable code (`noJava` or `jreOnly`) the UI localizes.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapInfo {
    pub at: String,
    pub used_mb: Option<f64>,
    pub committed_mb: Option<f64>,
    pub max_mb: Option<f64>,
    pub metaspace_used_mb: Option<f64>,
    pub collector: Option<String>,
    /// True when `GC.heap_info` ran but its text was not a known collector layout.
    pub unparsed: bool,
}

pub struct PerfState {
    config: PerfConfig,
    samples: VecDeque<PerfSample>,
    last_error: Option<PerfIssue>,
    last_sample: Option<Instant>,
    writes: usize,
    recordings: Vec<Recording>,
}

impl PerfState {
    fn new(config: PerfConfig, samples: VecDeque<PerfSample>) -> Self {
        Self {
            config,
            samples,
            last_error: None,
            last_sample: None,
            writes: 0,
            recordings: Vec::new(),
        }
    }
}

fn history_path(m: &Manager, id: &str) -> Result<PathBuf> {
    storage::child(&m.dir(id)?, "perf-history.json")
}

fn config_path(m: &Manager, id: &str) -> Result<PathBuf> {
    storage::child(&m.dir(id)?, "perf.json")
}

fn profiles_dir(m: &Manager, id: &str) -> Result<PathBuf> {
    storage::child(&m.dir(id)?, "profiles")
}

fn valid_config(config: &PerfConfig) -> bool {
    (MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&config.interval_secs)
}

fn read_config(m: &Manager, id: &str) -> PerfConfig {
    let path = match config_path(m, id) {
        Ok(path) => path,
        Err(_) => return PerfConfig::default(),
    };
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PerfConfig>(&bytes).ok())
        .filter(valid_config)
        .unwrap_or_default()
}

fn read_history(m: &Manager, id: &str) -> VecDeque<PerfSample> {
    let path = match history_path(m, id) {
        Ok(path) => path,
        Err(_) => return VecDeque::new(),
    };
    let mut samples = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<PerfSample>>(&bytes).ok())
        .unwrap_or_default();
    if samples.len() > MAX_SAMPLES {
        samples = samples.split_off(samples.len() - MAX_SAMPLES);
    }
    samples.into()
}

pub fn state(m: &Manager, id: &str) -> Result<Arc<Mutex<PerfState>>> {
    m.runtime(id)?;
    if let Some(found) = m
        .perf
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
    {
        return Ok(found);
    }
    let config = read_config(m, id);
    let samples = read_history(m, id);
    let mut perf = m.perf.lock().unwrap_or_else(|e| e.into_inner());
    Ok(perf
        .entry(id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(PerfState::new(config, samples))))
        .clone())
}

fn write_history(m: &Manager, id: &str, samples: &VecDeque<PerfSample>) -> Result<()> {
    let list: Vec<&PerfSample> = samples.iter().collect();
    storage::write_json(&history_path(m, id)?, &list)
}

pub fn save_config(m: &Manager, id: &str, config: PerfConfig) -> Result<PerfConfig> {
    if !valid_config(&config) {
        return Err(AppError::user(format!(
            "Choose a sampling interval between {MIN_INTERVAL_SECS} and {MAX_INTERVAL_SECS} seconds."
        ))
        .with_code("perf.invalidInterval"));
    }
    storage::write_json(&config_path(m, id)?, &config)?;
    let state = state(m, id)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    state.config = config.clone();
    // Re-enabling records a fresh sample on the next loop tick instead of
    // waiting for the interval that was configured when it was disabled.
    state.last_sample = None;
    if !config.enabled {
        state.last_error = None;
    }
    Ok(config)
}

pub fn profiles(m: &Manager, id: &str) -> Result<Vec<ProfileFile>> {
    let dir = profiles_dir(m, id)?;
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().ends_with(".jfr") {
            continue;
        }
        match entry.file_type() {
            Ok(kind) if kind.is_file() => {}
            _ => continue,
        }
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        out.push(ProfileFile {
            name,
            size_bytes: meta.len(),
            modified_at: meta
                .modified()
                .ok()
                .map(|at| chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339()),
        });
    }
    out.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    Ok(out)
}

fn recording_expired(record: &Recording, now: chrono::DateTime<chrono::Utc>) -> bool {
    chrono::DateTime::parse_from_rfc3339(&record.started_at)
        .map(|start| {
            let elapsed = (now - start.with_timezone(&chrono::Utc)).num_seconds();
            elapsed >= record.duration_secs as i64 + 5
        })
        .unwrap_or(true)
}

fn prune_recordings(state: &mut PerfState, now: chrono::DateTime<chrono::Utc>) -> Vec<Recording> {
    state
        .recordings
        .retain(|record| !recording_expired(record, now));
    state.recordings.clone()
}

pub fn report(m: &Manager, id: &str) -> Result<PerfReport> {
    let server = m.metadata(id)?;
    let state = state(m, id)?;
    let (enabled, interval_secs, samples, last_error, recordings) = {
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        let recordings = prune_recordings(&mut state, chrono::Utc::now());
        (
            state.config.enabled,
            state.config.interval_secs,
            state.samples.iter().cloned().collect::<Vec<_>>(),
            state.last_error.clone(),
            recordings,
        )
    };
    let rcon_ready = rcon::info(m, id)
        .map(|info| info.enabled && info.reachable)
        .unwrap_or(false);
    Ok(PerfReport {
        id: id.into(),
        enabled,
        interval_secs,
        latest: samples.last().cloned(),
        samples,
        last_error,
        rcon_ready,
        spark: spark_installed(m, &server),
        recordings,
        profiles: profiles(m, id)?,
    })
}

fn spark_installed(m: &Manager, server: &Server) -> bool {
    let folder = match server.server_type.content_folder() {
        Some(folder) => folder,
        None => return false,
    };
    let dir = match m.dir(&server.id) {
        Ok(dir) => dir,
        Err(_) => return false,
    };
    std::fs::read_dir(dir.join(folder))
        .map(|entries| {
            entries.flatten().any(|entry| {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                name.starts_with("spark") && name.ends_with(".jar")
            })
        })
        .unwrap_or(false)
}

fn tick_commands_for(kind: ServerType, spark: bool) -> Vec<&'static str> {
    if kind.plugins() {
        vec!["tps", "tick query"]
    } else if spark {
        vec!["tick query", "spark tps"]
    } else {
        vec!["tick query"]
    }
}

pub fn jfr_tools(m: &Manager, id: &str) -> Result<JfrTools> {
    let server = m.metadata(id)?;
    let java = server
        .java_path
        .as_deref()
        .filter(|path| Path::new(path).is_file())
        .map(str::to_string);
    let jcmd = java.as_deref().and_then(|java| jdk_tool(java, "jcmd"));
    let reason = if java.is_none() {
        Some("noJava")
    } else if jcmd.is_none() {
        Some("jreOnly")
    } else {
        None
    };
    Ok(JfrTools {
        java_path: java.unwrap_or_default(),
        available: reason.is_none(),
        jcmd: jcmd.map(|path| path.to_string_lossy().into_owned()),
        reason: reason.map(str::to_string),
    })
}

fn jdk_tool(java: &str, name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let configured = Path::new(java).parent().map(|parent| parent.join(&file));
    // Package-manager installs often expose `/usr/bin/java` as a symlink; the
    // JDK tools live beside the resolved executable, not the symlink.
    let resolved = std::fs::canonicalize(java)
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(&file)));
    [configured, resolved]
        .into_iter()
        .flatten()
        .find(|candidate| candidate.is_file())
}

fn normalize_settings(settings: Option<String>) -> Result<&'static str> {
    match settings.as_deref().unwrap_or("profile") {
        "profile" => Ok("profile"),
        "default" => Ok("default"),
        _ => Err(
            AppError::user("Choose the default or profile JFR settings.")
                .with_code("perf.invalidSettings"),
        ),
    }
}

fn validate_duration(duration_secs: u64) -> Result<()> {
    if (15..=600).contains(&duration_secs) {
        Ok(())
    } else {
        Err(
            AppError::user("Choose a JFR recording length between 15 seconds and 10 minutes.")
                .with_code("perf.invalidDuration"),
        )
    }
}

fn valid_recording_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn running_pid(m: &Manager, id: &str) -> Result<u32> {
    let snapshot = m.runtime(id)?.snapshot();
    if snapshot.status != Status::Running {
        return Err(AppError::user(
            "Start the server and wait until it is ready before using JDK tools.",
        )
        .with_code("perf.notRunning"));
    }
    snapshot.pid.ok_or_else(|| {
        AppError::user("The server process identifier is not available yet.")
            .with_code("perf.notRunning")
    })
}

async fn run_tool(jcmd: &Path, pid: u32, args: &[String]) -> Result<String> {
    let mut command = Command::new(jcmd);
    command
        .arg(pid.to_string())
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(TOOL_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            AppError::user("The JDK tool did not respond within 20 seconds.")
                .with_code("perf.toolTimeout")
        })?
        .map_err(|e| {
            AppError::new(
                "Could not run the JDK tool beside the configured Java executable.",
                e,
            )
            .with_code("perf.toolFailed")
        })?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text: String = text.chars().take(MAX_TOOL_OUTPUT).collect();
    if !output.status.success() {
        return Err(AppError::new(
            "The JDK tool rejected the request. Check that the server is still running.",
            text,
        )
        .with_code("perf.toolFailed"));
    }
    Ok(text)
}

pub async fn start_jfr(
    m: &Arc<Manager>,
    id: &str,
    duration_secs: u64,
    settings: Option<String>,
) -> Result<Recording> {
    let pid = running_pid(m, id)?;
    validate_duration(duration_secs)?;
    let settings = normalize_settings(settings)?;
    let tools = jfr_tools(m, id)?;
    let jcmd = tools.jcmd.ok_or_else(|| {
        AppError::user(
            "This Java runtime is a JRE without jcmd. Select a full JDK in server settings to record JFR profiles.",
        )
        .with_code("perf.jreOnly")
    })?;
    let dir = profiles_dir(m, id)?;
    std::fs::create_dir_all(&dir)?;
    let name = format!("easy-{}", chrono::Local::now().format("%Y%m%d-%H%M%S%3f"));
    let file = storage::child(&dir, &format!("{name}.jfr"))?;
    run_tool(
        Path::new(&jcmd),
        pid,
        &[
            "JFR.start".into(),
            format!("name={name}"),
            format!("settings={settings}"),
            format!("duration={duration_secs}s"),
            format!("filename={}", file.display()),
        ],
    )
    .await?;
    let record = Recording {
        name,
        file: file
            .file_name()
            .map(|file| file.to_string_lossy().into_owned())
            .unwrap_or_default(),
        started_at: chrono::Utc::now().to_rfc3339(),
        duration_secs,
    };
    let state = state(m, id)?;
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .recordings
        .push(record.clone());
    Ok(record)
}

pub async fn stop_jfr(m: &Arc<Manager>, id: &str, name: &str) -> Result<()> {
    if !valid_recording_name(name) {
        return Err(
            AppError::user("The recording name is not valid.").with_code("perf.invalidRecording")
        );
    }
    let pid = running_pid(m, id)?;
    let tools = jfr_tools(m, id)?;
    let jcmd = tools.jcmd.ok_or_else(|| {
        AppError::user("This Java runtime does not include jcmd.").with_code("perf.jreOnly")
    })?;
    run_tool(
        Path::new(&jcmd),
        pid,
        &["JFR.stop".into(), format!("name={name}")],
    )
    .await?;
    let state = state(m, id)?;
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .recordings
        .retain(|record| record.name != name);
    Ok(())
}

pub fn delete_profile(m: &Manager, id: &str, name: &str) -> Result<()> {
    if !name.to_ascii_lowercase().ends_with(".jfr") {
        return Err(AppError::user("Only JFR profile files can be deleted.")
            .with_code("perf.invalidProfile"));
    }
    let active = state(m, id)?
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .recordings
        .iter()
        .any(|record| record.file == name);
    if active {
        return Err(
            AppError::user("Stop the recording before deleting its file.")
                .with_code("perf.recordingActive"),
        );
    }
    let dir = profiles_dir(m, id)?;
    let file = storage::child(&dir, name)?;
    if !file.is_file() {
        return Err(
            AppError::user("This profile file no longer exists.").with_code("perf.profileMissing")
        );
    }
    std::fs::remove_file(file)?;
    Ok(())
}

pub async fn heap_info(m: &Arc<Manager>, id: &str) -> Result<HeapInfo> {
    let server = m.metadata(id)?;
    let pid = running_pid(m, id)?;
    let tools = jfr_tools(m, id)?;
    let jcmd = tools.jcmd.ok_or_else(|| {
        AppError::user(
            "This Java runtime is a JRE without jcmd. Select a full JDK to read heap details.",
        )
        .with_code("perf.jreOnly")
    })?;
    let text = run_tool(Path::new(&jcmd), pid, &["GC.heap_info".into()]).await?;
    let mut info = parse_heap_info(&text, Some(server.memory_max_mb as f64));
    info.at = chrono::Utc::now().to_rfc3339();
    Ok(info)
}

fn strip_colors(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

fn strip_parentheses(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut depth = 0usize;
    for c in line.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn floats(text: &str) -> Vec<f32> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
        if index < chars.len() && chars[index] == '.' {
            index += 1;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
        }
        let token: String = chars[start..index].iter().collect();
        if let Ok(value) = token.parse::<f32>() {
            if value.is_finite() {
                out.push(value);
            }
        }
    }
    out
}

fn valid_tps(value: f32) -> Option<f32> {
    (value.is_finite() && (0.0..=200.0).contains(&value)).then_some(value)
}

fn valid_mspt(value: f32) -> Option<f32> {
    (value.is_finite() && (0.0..=60_000.0).contains(&value)).then_some(value)
}

fn tps_values(text: &str) -> Option<[f32; 3]> {
    for line in text.lines() {
        let line = strip_colors(line);
        if !line.to_ascii_lowercase().contains("tps") {
            continue;
        }
        let line = strip_parentheses(&line);
        let body = line.rsplit(':').next().unwrap_or(&line);
        let values: Vec<f32> = floats(body).into_iter().filter_map(valid_tps).collect();
        if values.len() >= 3 {
            return Some([values[0], values[1], values[2]]);
        }
        if let Some(value) = values.first() {
            return Some([*value; 3]);
        }
    }
    None
}

fn mspt_value(text: &str) -> Option<f32> {
    for line in text.lines() {
        let line = strip_colors(line);
        let lower = line.to_ascii_lowercase();
        if lower.contains("average time per tick") {
            if let Some(value) = floats(&line).into_iter().find_map(valid_mspt) {
                return Some(value);
            }
        }
        if lower.contains("tick times") || lower.contains("mspt") {
            let body = line.rsplit(':').next().unwrap_or(&line);
            if let Some(value) = floats(body).into_iter().find_map(valid_mspt) {
                return Some(value);
            }
        }
    }
    None
}

fn source_for(command: &str) -> &'static str {
    if command.starts_with("spark") {
        "spark"
    } else if command == "tps" {
        "paper"
    } else {
        "tick"
    }
}

#[derive(Debug, Clone, Default)]
pub struct TickReading {
    pub tps1m: Option<f32>,
    pub tps5m: Option<f32>,
    pub tps15m: Option<f32>,
    pub mspt: Option<f32>,
    pub source: String,
}

/// Parse known tick-timing responses. Returns `None` for unknown command
/// output so callers can try the next command instead of recording zeros.
pub fn parse_tick_output(command: &str, output: &str) -> Option<TickReading> {
    let tps = tps_values(output);
    let mspt = mspt_value(output);
    let mut reading = TickReading {
        tps1m: tps.map(|values| values[0]),
        tps5m: tps.map(|values| values[1]),
        tps15m: tps.map(|values| values[2]),
        mspt,
        source: source_for(command).into(),
    };
    if reading.tps1m.is_none() {
        if let Some(mspt) = mspt.filter(|value| *value > 0.0) {
            reading.tps1m = Some((1000.0 / mspt).min(20.0));
            reading.tps5m = reading.tps1m;
            reading.tps15m = reading.tps1m;
        }
    }
    (reading.tps1m.is_some() || reading.mspt.is_some()).then_some(reading)
}

async fn collect_sample(m: &Arc<Manager>, id: &str) -> Result<PerfSample> {
    let server = m.metadata(id)?;
    let snapshot = m.runtime(id)?.snapshot();
    if snapshot.status != Status::Running {
        return Err(AppError::user(
            "Start the server and wait until it is ready before collecting tick timings.",
        )
        .with_code("perf.notRunning"));
    }
    let spark = spark_installed(m, &server);
    let mut last_output = String::new();
    for command in tick_commands_for(server.server_type, spark) {
        let output = rcon::query(m, id, command).await?;
        if let Some(reading) = parse_tick_output(command, &output) {
            let memory_mb = crate::monitor::latest(m, id)
                .ok()
                .flatten()
                .map(|metrics| metrics.memory_mb);
            return Ok(PerfSample {
                id: id.into(),
                at: chrono::Utc::now().to_rfc3339(),
                tps1m: reading.tps1m,
                tps5m: reading.tps5m,
                tps15m: reading.tps15m,
                mspt: reading.mspt,
                players: snapshot.players.len() as u32,
                memory_mb,
                source: reading.source,
            });
        }
        last_output = output.chars().take(400).collect();
    }
    Err(AppError::new(
        "The server did not return recognizable tick timings. Use a version with `tick query`, or install Spark for Fabric/Vanilla servers.",
        last_output,
    )
    .with_code("perf.noReading"))
}

async fn sample_once(m: &Arc<Manager>, id: &str) -> Result<PerfSample> {
    let outcome = collect_sample(m, id).await;
    let state = state(m, id)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    state.last_sample = Some(Instant::now());
    match outcome {
        Ok(sample) => {
            state.last_error = None;
            state.samples.push_back(sample.clone());
            while state.samples.len() > MAX_SAMPLES {
                state.samples.pop_front();
            }
            state.writes += 1;
            if state.writes % FLUSH_EVERY == 0 {
                let _ = write_history(m, id, &state.samples);
            }
            drop(state);
            m.emit("server-perf", sample.clone());
            Ok(sample)
        }
        Err(error) => {
            state.last_error = Some(PerfIssue {
                code: error.code.clone(),
                message: error.message.clone(),
            });
            Err(error)
        }
    }
}

pub async fn sample_now(m: &Arc<Manager>, id: &str) -> Result<PerfSample> {
    sample_once(m, id).await
}

fn due(m: &Manager, id: &str) -> bool {
    let state = match state(m, id) {
        Ok(state) => state,
        Err(_) => return false,
    };
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    state.config.enabled
        && state
            .last_sample
            .map(|at| at.elapsed() >= Duration::from_secs(state.config.interval_secs))
            .unwrap_or(true)
}

/// Opt-in background sampler. It does nothing unless recording was enabled for
/// a running server, and it never starts the game or any JDK tool.
pub fn spawn(m: Arc<Manager>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(LOOP_SECS)).await;
            if m.shutting_down.load(Ordering::SeqCst) {
                continue;
            }
            let ids: Vec<String> = m
                .runtimes
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, runtime)| runtime.snapshot().status == Status::Running)
                .map(|(id, _)| id.clone())
                .collect();
            for id in ids {
                if !due(&m, &id) {
                    continue;
                }
                // Errors are persisted in the report for the UI; repeated log
                // lines every interval would only add noise.
                let _ = sample_once(&m, &id).await;
            }
        }
    });
}

fn size_mb_after(line: &str, keyword: &str) -> Option<f64> {
    let lower = line.to_ascii_lowercase();
    let start = lower.find(keyword)? + keyword.len();
    let tail = &lower[start..];
    let chars: Vec<char> = tail.chars().collect();
    let mut index = 0;
    while index < chars.len() && !chars[index].is_ascii_digit() {
        index += 1;
    }
    let number_start = index;
    while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.') {
        index += 1;
    }
    if index == number_start {
        return None;
    }
    let number: f64 = chars[number_start..index]
        .iter()
        .collect::<String>()
        .parse()
        .ok()?;
    let unit = chars.get(index).copied().unwrap_or('b');
    let mb = match unit {
        'k' => number / 1024.0,
        'm' => number,
        'g' => number * 1024.0,
        _ => number / (1024.0 * 1024.0),
    };
    mb.is_finite().then_some(mb)
}

fn collector_name(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let name = if lower.contains("garbage-first") || lower.contains("g1") {
        "G1"
    } else if lower.contains("zheap") || lower.contains("zgc") {
        "ZGC"
    } else if lower.contains("shenandoah") {
        "Shenandoah"
    } else if lower.contains("ps young") || lower.contains("ps old") {
        "Parallel"
    } else if lower.contains("def new generation") {
        "Serial"
    } else {
        return None;
    };
    Some(name.into())
}

/// Parse `jcmd GC.heap_info` output from the common HotSpot collectors.
pub fn parse_heap_info(text: &str, max_mb: Option<f64>) -> HeapInfo {
    let mut used = None;
    let mut committed = None;
    let mut metaspace = None;
    let mut summed_used = 0.0f64;
    let mut summed_total = 0.0f64;
    let mut saw_generation = false;
    let mut saw_summary = false;

    for raw in text.lines() {
        let line = raw.trim();
        let lower = line.to_ascii_lowercase();
        if lower.contains("metaspace") {
            metaspace = size_mb_after(line, "used");
            continue;
        }
        if lower.contains("heap") && lower.contains("used") {
            used = size_mb_after(line, "used");
            committed = size_mb_after(line, "committed")
                .or_else(|| size_mb_after(line, "total"))
                .or_else(|| size_mb_after(line, "capacity"));
            saw_summary = used.is_some();
            if saw_summary {
                continue;
            }
        }
        // Serial and Parallel print separate generations rather than a heap
        // summary. "the space" duplicates the survivor space already counted in
        // the young generation total, so it is excluded.
        if lower.contains("generation")
            && !lower.contains("metaspace")
            && !lower.contains("the space")
        {
            if let Some(value) = size_mb_after(line, "used") {
                summed_used += value;
                if let Some(total) = size_mb_after(line, "total") {
                    summed_total += total;
                } else if let Some(capacity) = size_mb_after(line, "capacity") {
                    summed_total += capacity;
                }
                saw_generation = true;
            }
        }
    }

    if !saw_summary && saw_generation {
        used = Some(summed_used);
        committed = (summed_total > 0.0).then_some(summed_total);
    }

    HeapInfo {
        at: String::new(),
        used_mb: used,
        committed_mb: committed,
        max_mb,
        metaspace_used_mb: metaspace,
        collector: collector_name(text),
        unparsed: used.is_none() && metaspace.is_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vanilla_tick_query() {
        let output = "Target tick rate: 20.0 per second\n\
Average time per tick: 0.085 ms (min 0.021 ms, max 0.490 ms)";
        let reading = parse_tick_output("tick query", output).unwrap();
        assert_eq!(reading.mspt, Some(0.085));
        assert_eq!(reading.tps1m, Some(20.0));
        assert_eq!(reading.source, "tick");
    }

    #[test]
    fn parses_paper_tps_and_tick_times() {
        let output = "§aTPS from last 1m, 5m, 15m: §a19.8, §a20.0, §a20.0\n\
Tick times from last 1m, 5m, 15m: 1.2, 0.9, 0.8";
        let reading = parse_tick_output("tps", output).unwrap();
        assert_eq!(reading.tps1m, Some(19.8));
        assert_eq!(reading.tps5m, Some(20.0));
        assert_eq!(reading.tps15m, Some(20.0));
        assert_eq!(reading.mspt, Some(1.2));
        assert_eq!(reading.source, "paper");
    }

    #[test]
    fn parses_spark_tps_without_period_numbers() {
        let output = "TPS: 18.5 (1m), 19.6 (5m), 19.9 (15m)";
        let reading = parse_tick_output("spark tps", output).unwrap();
        assert_eq!(reading.tps1m, Some(18.5));
        assert_eq!(reading.tps5m, Some(19.6));
        assert_eq!(reading.tps15m, Some(19.9));
        assert_eq!(reading.source, "spark");
    }

    #[test]
    fn derives_tps_from_mspt_when_tps_is_absent() {
        let reading = parse_tick_output("tick query", "Average time per tick: 100.0 ms").unwrap();
        assert_eq!(reading.mspt, Some(100.0));
        assert_eq!(reading.tps1m, Some(10.0));
    }

    #[test]
    fn unknown_output_is_not_recorded_as_zero() {
        assert!(parse_tick_output("tick query", "Unknown or incomplete command").is_none());
        assert!(parse_tick_output("tick query", "").is_none());
    }

    #[test]
    fn command_priority_matches_server_family() {
        assert_eq!(
            tick_commands_for(ServerType::Paper, true),
            vec!["tps", "tick query"]
        );
        assert_eq!(
            tick_commands_for(ServerType::Fabric, true),
            vec!["tick query", "spark tps"]
        );
        assert_eq!(
            tick_commands_for(ServerType::Vanilla, false),
            vec!["tick query"]
        );
    }

    #[test]
    fn config_interval_is_bounded() {
        assert!(!valid_config(&PerfConfig {
            enabled: true,
            interval_secs: MIN_INTERVAL_SECS - 1
        }));
        assert!(!valid_config(&PerfConfig {
            enabled: true,
            interval_secs: MAX_INTERVAL_SECS + 1
        }));
        assert!(valid_config(&PerfConfig {
            enabled: true,
            interval_secs: DEFAULT_INTERVAL_SECS
        }));
    }

    #[test]
    fn parses_g1_heap_summary() {
        let text = "garbage-first heap   total 1048576K, used 524288K [0x00000000, 0x00000000]\n\
Metaspace       used 12345K, committed 12800K, reserved 1056768K";
        let info = parse_heap_info(text, Some(2048.0));
        assert_eq!(info.collector.as_deref(), Some("G1"));
        assert_eq!(info.used_mb, Some(512.0));
        assert_eq!(info.committed_mb, Some(1024.0));
        assert_eq!(info.max_mb, Some(2048.0));
        assert!(!info.unparsed);
        assert!((info.metaspace_used_mb.unwrap() - 12.0557).abs() < 0.01);
    }

    #[test]
    fn parses_serial_generations_without_double_counting() {
        let text = "def new generation   total 30720K, used 3072K [0x0, 0x1, 0x2)\n\
  eden space 27328K,  11% used [0x0, 0x1, 0x2)\n\
  from space 3392K,   0% used [0x0, 0x1, 0x2)\n\
  to   space 3392K,   0% used [0x0, 0x1, 0x2)\n\
tenured generation   total 68352K, used 1024K [0x3, 0x4, 0x5)\n\
   the space 68352K,   1% used [0x3, 0x4, 0x5)\n\
Metaspace       used 10000K, committed 10240K, reserved 1056768K";
        let info = parse_heap_info(text, None);
        assert_eq!(info.collector.as_deref(), Some("Serial"));
        assert_eq!(info.used_mb, Some(4.0));
        assert_eq!(info.committed_mb, Some(96.75));
        assert_eq!(info.metaspace_used_mb, Some(10000.0 / 1024.0));
    }

    #[test]
    fn unrecognized_heap_output_is_marked() {
        let info = parse_heap_info("Heap inspection is not available", None);
        assert!(info.unparsed);
        assert!(info.used_mb.is_none());
        assert_eq!(info.collector, None);
    }

    #[test]
    fn expired_recordings_are_pruned_after_their_duration() {
        let record = Recording {
            name: "easy-test".into(),
            file: "easy-test.jfr".into(),
            started_at: "2026-10-20T10:00:00Z".into(),
            duration_secs: 60,
        };
        let during = chrono::DateTime::parse_from_rfc3339("2026-10-20T10:00:30Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let after = chrono::DateTime::parse_from_rfc3339("2026-10-20T10:01:10Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(!recording_expired(&record, during));
        assert!(recording_expired(&record, after));
    }

    #[test]
    fn recording_names_and_settings_are_restricted() {
        assert!(valid_recording_name("easy-20261020-100000"));
        assert!(!valid_recording_name("easy start"));
        assert!(!valid_recording_name(""));
        assert_eq!(normalize_settings(None).unwrap(), "profile");
        assert_eq!(
            normalize_settings(Some("default".into())).unwrap(),
            "default"
        );
        assert!(normalize_settings(Some("continuous".into())).is_err());
        assert!(validate_duration(10).is_err());
        assert!(validate_duration(60).is_ok());
    }
}
