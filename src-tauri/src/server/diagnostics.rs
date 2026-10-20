use crate::{
    error::Result,
    extensions::CrashAnalyzer,
    log_assistant::sanitize,
    minecraft::properties::Properties,
    models::{JvmPreset, Status},
    server::{
        crash,
        manager::{read_limited, Manager},
    },
    storage,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// A single health rule result. `values` feeds localized templates in the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheck {
    pub id: String,
    /// "ok" | "warning" | "critical" | "info"
    pub level: String,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub server_id: String,
    pub score: u32,
    pub checks: Vec<HealthCheck>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosisFinding {
    pub id: String,
    /// "critical" | "warning"
    pub severity: String,
    pub evidence: Vec<String>,
    /// Exactly one finding is marked as the first actionable root cause.
    pub root_cause: bool,
    /// A symptom explains the shutdown but not why it started.
    pub symptom: bool,
    pub line: Option<usize>,
    pub occurrences: usize,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosisReport {
    pub server_id: String,
    pub available: bool,
    pub source: Option<String>,
    pub generated_at: String,
    pub exit_code: Option<i32>,
    pub root_cause: Option<String>,
    pub scanned_lines: usize,
    pub findings: Vec<DiagnosisFinding>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    pub server_id: String,
    pub source: Option<String>,
    pub generated_at: String,
    pub text: String,
}

fn item(id: &str, level: &str, values: Vec<(&str, String)>) -> HealthCheck {
    HealthCheck {
        id: id.into(),
        level: level.into(),
        values: values
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

fn preset_name(preset: JvmPreset) -> &'static str {
    match preset {
        JvmPreset::Balanced => "Balanced",
        JvmPreset::Aikar => "Aikar",
        JvmPreset::Paper => "Paper",
        JvmPreset::Custom => "Custom",
    }
}

/// 0–100 readiness score; critical findings weigh more than warnings.
fn readiness(checks: &[HealthCheck]) -> u32 {
    let mut score: i32 = 100;
    for check in checks {
        match check.level.as_str() {
            "critical" => score -= 25,
            "warning" => score -= 8,
            _ => {}
        }
    }
    score.max(0) as u32
}

fn eula_accepted(dir: &Path) -> bool {
    let file = match storage::child(dir, "eula.txt") {
        Ok(p) => p,
        Err(_) => return false,
    };
    fs::read_to_string(file)
        .map(|text| {
            Properties::parse(&text)
                .values
                .get("eula")
                .map(|v| v == "true")
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn disk_available(path: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut best: Option<(usize, u64)> = None;
    for disk in disks.iter() {
        let mount = disk.mount_point();
        if path.starts_with(mount) {
            let len = mount.as_os_str().len();
            if best.map(|(l, _)| len >= l).unwrap_or(true) {
                best = Some((len, disk.available_space()));
            }
        }
    }
    best.map(|(_, bytes)| bytes)
        .or_else(|| disks.iter().next().map(|d| d.available_space()))
}

fn residue(dir: &Path) -> (usize, usize) {
    let mut restore = 0;
    let mut partial = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("restore-") {
                restore += 1;
            }
            if name.ends_with(".part") {
                partial += 1;
            }
        }
    }
    (restore, partial)
}

fn log_summary(dir: &Path) -> Option<(u64, u64, String)> {
    let logs = storage::child(dir, "logs").ok()?;
    let latest = storage::child(&logs, "latest.log").ok()?;
    if !latest.is_file() {
        return None;
    }
    let text = read_limited(&latest).ok()?;
    let mut errors = 0;
    let mut warnings = 0;
    for line in text.lines() {
        if line.contains("/ERROR") || line.contains("ERROR]") {
            errors += 1;
        } else if line.contains("/WARN") || line.contains("WARN]") {
            warnings += 1;
        }
    }
    Some((errors, warnings, "logs/latest.log".into()))
}

/// Score every pre-flight rule and return a 0–100 readiness score.
pub async fn health(m: &Manager, id: &str) -> Result<HealthReport> {
    let s = m.metadata(id)?;
    let dir = m.dir(id)?;
    let status = m
        .runtime(id)
        .map(|r| r.snapshot().status)
        .unwrap_or(Status::Stopped);
    let mut checks = Vec::new();

    // Installed artifact.
    let jar = storage::child(&dir, "server.jar")?;
    checks.push(if s.installed && jar.is_file() {
        item(
            "install",
            "ok",
            vec![
                (
                    "artifact",
                    s.artifact
                        .clone()
                        .unwrap_or_else(|| format!("{:?}", s.server_type)),
                ),
                ("version", s.minecraft_version.clone()),
            ],
        )
    } else {
        item(
            "install",
            "critical",
            vec![("version", s.minecraft_version.clone())],
        )
    });

    // EULA.
    checks.push(if eula_accepted(&dir) {
        item(
            "eula",
            "ok",
            vec![("acceptedAt", s.eula_accepted_at.clone().unwrap_or_default())],
        )
    } else {
        item("eula", "critical", vec![])
    });

    // Java runtime.
    let required = s.required_java.unwrap_or_else(|| {
        crate::java::version::required_java(&s.minecraft_version, s.server_type, None)
    });
    let java_path = s.java_path.clone().unwrap_or_default();
    checks.push(
        match crate::java::detector::validate(&java_path, required).await {
            Ok(runtime) => item(
                "java",
                "ok",
                vec![
                    ("path", runtime.path),
                    ("version", runtime.major.to_string()),
                    ("required", required.to_string()),
                ],
            ),
            Err(e) => item(
                "java",
                "critical",
                vec![("required", required.to_string()), ("message", e.message)],
            ),
        },
    );

    // Memory headroom.
    let (total, available, _) = super::manager::memory_info();
    let max = s.memory_max_mb as u64;
    let percent = if total > 0 {
        (max.saturating_mul(100) / total) as u32
    } else {
        0
    };
    let memory_level = if max < 1024 || max > available {
        "warning"
    } else if percent >= 70 {
        "warning"
    } else {
        "ok"
    };
    checks.push(item(
        "memory",
        memory_level,
        vec![
            ("max", max.to_string()),
            ("total", total.to_string()),
            ("available", available.to_string()),
            ("percent", percent.to_string()),
        ],
    ));

    // JVM flags: a broken custom preset prevents startup, ignored flags are only a hint.
    let jvm = vec![
        ("preset", preset_name(s.jvm_preset).into()),
        ("flags", s.jvm_flags.len().to_string()),
    ];
    checks.push(match s.jvm_preset {
        JvmPreset::Custom if crate::server::process::validate_jvm_flags(&s.jvm_flags).is_ok() => {
            item("jvm", "ok", jvm)
        }
        JvmPreset::Custom => item("jvm", "critical", jvm),
        _ if !s.jvm_flags.is_empty() => item("jvm", "info", jvm),
        _ => item("jvm", "ok", jvm),
    });

    // Port availability is only meaningful while nothing is listening.
    let port = m
        .properties(id)
        .ok()
        .and_then(|p| p.values.get("server-port").and_then(|v| v.parse().ok()))
        .unwrap_or(s.port);
    checks.push(if status.active() {
        item("port", "info", vec![("port", port.to_string())])
    } else {
        match crate::network::check_port(port) {
            Ok(()) => item("port", "ok", vec![("port", port.to_string())]),
            Err(_) => item("port", "critical", vec![("port", port.to_string())]),
        }
    });

    // Windows firewall rule used by LAN players. A missing rule is only advisory.
    #[cfg(windows)]
    {
        let rule = crate::network::rule_name(port);
        let values = vec![("port", port.to_string()), ("rule", rule)];
        let allowed =
            tokio::task::spawn_blocking(move || crate::network::firewall_rule(port)).await;
        checks.push(match allowed {
            Ok(true) => item("firewall", "ok", values),
            Ok(false) => item("firewall", "warning", values),
            Err(_) => item("firewall", "info", values),
        });
    }
    #[cfg(not(windows))]
    checks.push(item("firewall", "info", vec![("port", port.to_string())]));

    // Internet reachability is optional; an absent UPnP mapping never lowers the score.
    checks.push(match crate::upnp::mapping(m, id) {
        Some(mapping) if mapping.enabled => item(
            "upnp",
            "ok",
            vec![
                ("external", mapping.external_port.to_string()),
                ("internal", mapping.internal_port.to_string()),
            ],
        ),
        _ => item("upnp", "info", vec![("port", port.to_string())]),
    });

    // Properties schema plus risky-but-legal values.
    match m.properties(id) {
        Ok(p) => {
            let mut risk: Vec<&str> = Vec::new();
            if p.validate().is_err() {
                checks.push(item("properties", "critical", vec![]));
            } else {
                if p.values
                    .get("online-mode")
                    .map(|v| v == "false")
                    .unwrap_or(false)
                {
                    risk.push("online-mode");
                }
                if p.values
                    .get("allow-flight")
                    .map(|v| v == "true")
                    .unwrap_or(false)
                {
                    risk.push("allow-flight");
                }
                for key in ["view-distance", "simulation-distance"] {
                    if p.values
                        .get(key)
                        .and_then(|v| v.parse::<u32>().ok())
                        .map(|v| v > 16)
                        .unwrap_or(false)
                    {
                        risk.push(key);
                    }
                }
                if p.values
                    .get("max-players")
                    .and_then(|v| v.parse::<u32>().ok())
                    .map(|v| v > 100)
                    .unwrap_or(false)
                {
                    risk.push("max-players");
                }
                checks.push(item(
                    "properties",
                    if risk.is_empty() { "ok" } else { "warning" },
                    vec![("keys", risk.join(", "))],
                ));
            }
        }
        Err(_) => checks.push(item("properties", "critical", vec![])),
    }

    let (restore, partial) = residue(&dir);
    checks.push(if restore == 0 {
        item("restore", "ok", vec![])
    } else {
        item("restore", "critical", vec![("count", restore.to_string())])
    });
    checks.push(if partial == 0 {
        item("partial", "ok", vec![])
    } else {
        item("partial", "warning", vec![("count", partial.to_string())])
    });

    // Free disk space on the volume that holds this server.
    let free = disk_available(&dir);
    checks.push(match free {
        Some(bytes) if bytes < 512 * 1024 * 1024 => item(
            "disk",
            "critical",
            vec![("availableMb", (bytes / 1024 / 1024).to_string())],
        ),
        Some(bytes) if bytes < 2 * 1024 * 1024 * 1024 => item(
            "disk",
            "warning",
            vec![("availableMb", (bytes / 1024 / 1024).to_string())],
        ),
        Some(bytes) => item(
            "disk",
            "ok",
            vec![("availableMb", (bytes / 1024 / 1024).to_string())],
        ),
        None => item("disk", "info", vec![]),
    });

    // Backup freshness, plus whether an automatic backup task is enabled.
    let backups = crate::backup::list(m, id).unwrap_or_default();
    let scheduled = crate::schedule::list(m, id)
        .map(|tasks| {
            tasks
                .iter()
                .filter(|task| task.enabled && task.action == crate::schedule::TaskAction::Backup)
                .count()
        })
        .unwrap_or(0);
    let age = backups
        .first()
        .and_then(|b| chrono::DateTime::parse_from_rfc3339(&b.created_at).ok())
        .map(|t| {
            chrono::Utc::now()
                .signed_duration_since(t.with_timezone(&chrono::Utc))
                .num_days()
        });
    checks.push(match age {
        None => item(
            "backup",
            "warning",
            vec![("count", "0".into()), ("scheduled", scheduled.to_string())],
        ),
        Some(days) if days > 7 => item(
            "backup",
            "warning",
            vec![
                ("count", backups.len().to_string()),
                ("ageDays", days.to_string()),
                ("scheduled", scheduled.to_string()),
            ],
        ),
        Some(days) => item(
            "backup",
            "ok",
            vec![
                ("count", backups.len().to_string()),
                ("ageDays", days.to_string()),
                ("scheduled", scheduled.to_string()),
            ],
        ),
    });

    // Recent log noise.
    checks.push(match log_summary(&dir) {
        Some((0, warnings, file)) if warnings == 0 => item("logs", "ok", vec![("file", file)]),
        Some((errors, warnings, file)) => item(
            "logs",
            if errors > 0 { "warning" } else { "info" },
            vec![
                ("file", file),
                ("errors", errors.to_string()),
                ("warnings", warnings.to_string()),
            ],
        ),
        None => item("logs", "info", vec![]),
    });

    Ok(HealthReport {
        server_id: s.id,
        score: readiness(&checks),
        checks,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn exit_code(m: &Manager, id: &str) -> Option<i32> {
    m.runtime(id)
        .ok()
        .and_then(|runtime| runtime.snapshot().exit_code)
}

/// Read the available log tail once, turn signatures into API findings and add
/// server-specific context such as the configured heap and port.
fn inspect(m: &Manager, id: &str) -> Result<DiagnosisReport> {
    let (source, text) = log_text(m, id)?;
    let exit = exit_code(m, id);
    let recent: Vec<String> = text.lines().map(str::to_string).collect();
    let analysis = crash::SignatureAnalyzer.analyze(exit, &recent);
    let root_id = analysis.root_cause.map(str::to_string);
    let scanned_lines = analysis.scanned_lines;
    let mut findings: Vec<DiagnosisFinding> = analysis
        .findings
        .into_iter()
        .map(|finding| DiagnosisFinding {
            root_cause: root_id.as_deref() == Some(finding.id),
            id: finding.id.into(),
            severity: finding.severity.into(),
            evidence: finding.evidence,
            symptom: finding.symptom,
            line: finding.first_line,
            occurrences: finding.occurrences,
            values: finding.values,
        })
        .collect();

    if let Ok(server) = m.metadata(id) {
        let configured_port = m
            .properties(id)
            .ok()
            .and_then(|properties| {
                properties
                    .values
                    .get("server-port")
                    .and_then(|value| value.parse::<u16>().ok())
            })
            .unwrap_or(server.port);
        for finding in &mut findings {
            match finding.id.as_str() {
                "outOfMemory" => {
                    finding
                        .values
                        .entry("maxMemoryMb".into())
                        .or_insert_with(|| server.memory_max_mb.to_string());
                }
                "portInUse" => {
                    finding
                        .values
                        .entry("port".into())
                        .or_insert_with(|| configured_port.to_string());
                }
                _ => {}
            }
        }
    }

    Ok(DiagnosisReport {
        server_id: id.to_string(),
        available: source.is_some() || !findings.is_empty(),
        source,
        generated_at: chrono::Utc::now().to_rfc3339(),
        exit_code: exit,
        root_cause: root_id,
        scanned_lines,
        findings,
    })
}

fn newest_crash_report(dir: &Path) -> Option<(PathBuf, String)> {
    let reports = storage::child(dir, "crash-reports").ok()?;
    if !reports.is_dir() {
        return None;
    }
    let mut newest: Option<(std::time::SystemTime, PathBuf, String)> = None;
    for entry in fs::read_dir(&reports).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("txt") {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) if m.is_file() => m,
            _ => continue,
        };
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        let name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        if storage::child(&reports, &name).is_err() {
            continue;
        }
        if newest
            .as_ref()
            .map(|(t, _, _)| modified > *t)
            .unwrap_or(true)
        {
            newest = Some((modified, path, name));
        }
    }
    newest.map(|(_, path, name)| (path, format!("crash-reports/{name}")))
}

/// Read the newest available file; current process output also catches JVM failures
/// that occur before Minecraft creates a log file.
pub fn log_text(m: &Manager, id: &str) -> Result<(Option<String>, String)> {
    m.metadata(id)?;
    let dir = m.dir(id)?;
    let mut candidates = Vec::new();
    if let Some((path, name)) = newest_crash_report(&dir) {
        if let Ok(time) = fs::metadata(&path).and_then(|v| v.modified()) {
            candidates.push((time, path, name));
        }
    }
    let logs = storage::child(&dir, "logs")?;
    let latest = storage::child(&logs, "latest.log")?;
    if let Ok(time) = fs::metadata(&latest).and_then(|v| v.modified()) {
        candidates.push((time, latest, "logs/latest.log".into()));
    }
    if let Some(started) = m
        .runtime(id)
        .ok()
        .and_then(|r| r.snapshot().started_at)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
    {
        candidates.retain(|(time, _, _)| chrono::DateTime::<chrono::Utc>::from(*time) >= started);
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let mut source = None;
    let mut text = String::new();
    if let Some((_, path, name)) = candidates.first() {
        text = read_limited(path)?;
        source = Some(name.clone());
    }
    if let Ok(runtime) = m.runtime(id) {
        let snap = runtime.snapshot();
        if let Some(started) = &snap.started_at {
            let lines = runtime.console.lock().unwrap();
            let current: Vec<_> = lines
                .iter()
                .filter(|l| &l.timestamp >= started && l.stream != "system")
                .collect();
            if !current.is_empty() {
                let output = current
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                text.push_str("\n");
                text.push_str(&output);
                source = Some(
                    source
                        .map(|s| format!("{s} + console"))
                        .unwrap_or_else(|| "console".into()),
                );
            }
        }
    }
    Ok((source, text))
}
fn advice(language: &str, id: &str) -> (&'static str, &'static str) {
    let en = language == "en";
    match id {
        "javaVersion" => {
            if en {
                (
                    "Java version mismatch",
                    "Install the Java major the server requires and select it under Runtime settings; do not point the server at an older system Java.",
                )
            } else {
                (
                    "Java 版本不匹配",
                    "安装该服务器要求的 Java 主版本，并在运行时设置中选择它；不要让服务器继续使用较旧的系统 Java。",
                )
            }
        }
        "jvmOptions" => {
            if en {
                (
                    "Invalid or conflicting JVM options",
                    "Switch to the Balanced preset, confirm the Java version, then restore custom tuning flags one at a time.",
                )
            } else {
                (
                    "JVM 参数无效或互相冲突",
                    "先切换到均衡 JVM 预设，确认 Java 版本兼容，再逐项恢复自定义调优参数。",
                )
            }
        }
        "outOfMemory" => {
            if en {
                (
                    "Out of memory",
                    "Raise the maximum heap if the machine has free RAM, or lower view/simulation distance and remove memory-heavy mods or datapacks.",
                )
            } else {
                (
                    "内存不足",
                    "在物理内存允许时提高最大堆内存；否则降低视距与模拟距离，并移除占用内存过高的模组或数据包。",
                )
            }
        }
        "portInUse" => {
            if en {
                (
                    "Port already in use",
                    "Stop the other process using this port, or change server-port in server settings and restart.",
                )
            } else {
                (
                    "端口被占用",
                    "关闭占用该端口的其他程序，或在服务器设置中更换 server-port 后重新启动。",
                )
            }
        }
        "diskSpace" => {
            if en {
                (
                    "Not enough disk space",
                    "Free at least several hundred MB on the server volume, then restart; backups and logs are usually the safest items to clean.",
                )
            } else {
                (
                    "磁盘空间不足",
                    "在服务器所在磁盘释放至少数百 MB 后重试；通常可以优先清理旧备份与历史日志。",
                )
            }
        }
        "fileAccess" => {
            if en {
                (
                    "File access denied",
                    "Check antivirus or folder permissions so EasyLauncher can read and write the server directory; close programs that lock world files.",
                )
            } else {
                (
                    "文件访问被拒绝",
                    "检查杀毒软件或文件夹权限，确保 EasyLauncher 可以读写服务器目录；关闭占用世界文件的程序。",
                )
            }
        }
        "missingJar" => {
            if en {
                (
                    "Missing server.jar",
                    "Reinstall the server core before starting again.",
                )
            } else {
                ("缺少 server.jar", "重新安装服务器核心后再启动。")
            }
        }
        "eula" => {
            if en {
                (
                    "EULA not accepted",
                    "Accept the Minecraft EULA in server settings or the creation wizard.",
                )
            } else {
                (
                    "EULA 未接受",
                    "在服务器设置或创建向导中接受 Minecraft EULA。",
                )
            }
        }
        "worldCorruption" => {
            if en {
                (
                    "World data may be corrupted",
                    "Restore the affected world from a backup. If no backup exists, move the damaged region file aside and let the server regenerate that region.",
                )
            } else {
                (
                    "世界数据可能损坏",
                    "优先从备份恢复受影响的世界；如无备份，先移走损坏的区域文件，让服务器重建该区域。",
                )
            }
        }
        "modConflict" => {
            if en {
                (
                    "Mod conflict",
                    "Remove duplicate or incompatible mods, then check the named mods for a shared Minecraft and loader version.",
                )
            } else {
                (
                    "模组冲突",
                    "移除重复或不兼容的模组，并检查提示中的模组是否使用相同的 Minecraft 与加载器版本。",
                )
            }
        }
        "missingDependency" => {
            if en {
                (
                    "Missing mod dependency",
                    "Install the dependency named in the log, or use the complete modpack instead of individual files.",
                )
            } else {
                (
                    "缺少模组依赖",
                    "安装日志中提示的前置模组，或改用包含完整依赖的整合包。",
                )
            }
        }
        "pluginError" => {
            if en {
                (
                    "Plugin failed to load",
                    "Confirm the plugin matches the server version, then read its full stack trace or temporarily remove it to isolate the failure.",
                )
            } else {
                (
                    "插件加载失败",
                    "确认插件与服务器版本匹配，查看其完整堆栈；必要时先移出该插件以确认问题来源。",
                )
            }
        }
        "watchdog" => {
            if en {
                (
                    "Server stall or watchdog timeout",
                    "Look for expensive mods or chunk loaders and lower view and simulation distance; inspect the thread dump for the blocking thread.",
                )
            } else {
                (
                    "服务器卡顿或看门狗超时",
                    "排查性能压力过大的模组或区块加载器，适当降低视距与模拟距离，并根据线程转储定位阻塞线程。",
                )
            }
        }
        _ => {
            if en {
                (
                    "Unclassified startup error",
                    "Expand the evidence or review the console around startup; the first exception usually contains the original cause.",
                )
            } else {
                (
                    "未分类的启动异常",
                    "展开证据或查看启动前后的控制台输出；通常第一个异常才包含原始原因。",
                )
            }
        }
    }
}

fn values_text(values: &BTreeMap<String, String>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn write_finding(out: &mut String, language: &str, finding: &DiagnosisFinding, heading: bool) {
    let en = language == "en";
    let (title, fix) = advice(language, &finding.id);
    if heading {
        out.push_str(&format!(
            "### {} (`{}`, {})\n\n",
            title, finding.id, finding.severity
        ));
    } else {
        out.push_str(&format!(
            "**{}** (`{}`, {})\n\n",
            title, finding.id, finding.severity
        ));
    }
    if let Some(line) = finding.line {
        out.push_str(&format!(
            "- {}: {}\n",
            if en {
                "First line"
            } else {
                "首次出现行号"
            },
            line
        ));
    }
    if finding.occurrences > 1 {
        out.push_str(&format!(
            "- {}: {}\n",
            if en { "Matches" } else { "命中次数" },
            finding.occurrences
        ));
    }
    if !finding.values.is_empty() {
        out.push_str(&format!(
            "- {}: {}\n",
            if en { "Details" } else { "相关细节" },
            sanitize(&values_text(&finding.values))
        ));
    }
    if !finding.evidence.is_empty() {
        out.push_str(&format!(
            "\n{}:\n",
            if en {
                "Evidence (sanitized)"
            } else {
                "证据（已脱敏）"
            }
        ));
        for line in &finding.evidence {
            out.push_str(&format!("- {}\n", sanitize(line)));
        }
    }
    out.push_str(&format!(
        "\n{}: {}\n\n",
        if en {
            "Recommended fix"
        } else {
            "修复建议"
        },
        fix
    ));
}

fn report_text(language: &str, report: &DiagnosisReport) -> String {
    let en = language == "en";
    let mut out = String::new();
    out.push_str(if en {
        "# EasyLauncher crash analysis\n\n"
    } else {
        "# EasyLauncher 崩溃诊断\n\n"
    });
    out.push_str(&format!(
        "- {}: {}\n",
        if en { "Generated" } else { "生成时间" },
        report.generated_at
    ));
    out.push_str(&format!(
        "- {}: {}\n",
        if en { "Log source" } else { "日志来源" },
        report
            .source
            .as_deref()
            .unwrap_or(if en { "none" } else { "无" })
    ));
    out.push_str(&format!(
        "- {}: {}\n",
        if en { "Lines scanned" } else { "扫描行数" },
        report.scanned_lines
    ));
    if let Some(code) = report.exit_code {
        out.push_str(&format!(
            "- {}: {code}\n",
            if en { "Exit code" } else { "退出码" }
        ));
    }
    out.push('\n');

    if let Some(root) = report.findings.iter().find(|finding| finding.root_cause) {
        out.push_str(if en {
            "## Root cause\n\n"
        } else {
            "## 根因\n\n"
        });
        write_finding(&mut out, language, root, false);
    } else {
        out.push_str(if en {
            "## Root cause\n\nNo known crash signature matched the available log tail. Open the console and inspect the first exception.\n\n"
        } else {
            "## 根因\n\n可用日志末尾没有匹配到已知崩溃特征。请在控制台查看第一个异常。\n\n"
        });
    }

    let others: Vec<&DiagnosisFinding> = report
        .findings
        .iter()
        .filter(|finding| !finding.root_cause)
        .collect();
    if !others.is_empty() {
        out.push_str(if en {
            "## Other findings\n\n"
        } else {
            "## 其他发现\n\n"
        });
        for finding in others {
            write_finding(&mut out, language, finding, true);
        }
    }

    out.push_str(if en {
        "## Privacy\n\nPaths, URLs, IP addresses and credential-bearing lines were redacted from the evidence above. This report is generated locally only when requested; review it before sharing.\n"
    } else {
        "## 隐私\n\n上方证据中的路径、URL、IP 地址和含凭据的行已遮挡。报告仅在你主动请求时本地生成，分享前仍请人工检查。\n"
    });
    out.chars().take(32_000).collect()
}

pub fn diagnose(m: &Manager, id: &str) -> Result<DiagnosisReport> {
    inspect(m, id)
}

/// Build a shareable Markdown report without writing it to disk. Paths,
/// addresses, URLs and credential lines are sanitized from the evidence.
pub fn sanitized_report(m: &Manager, id: &str, language: &str) -> Result<CrashReport> {
    let report = inspect(m, id)?;
    let language = if language == "en" { "en" } else { "zh" };
    let text = report_text(language, &report);
    Ok(CrashReport {
        server_id: report.server_id,
        source: report.source,
        generated_at: report.generated_at,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnosis_finding(
        id: &str,
        severity: &str,
        root_cause: bool,
        evidence: Vec<&str>,
    ) -> DiagnosisFinding {
        DiagnosisFinding {
            id: id.into(),
            severity: severity.into(),
            evidence: evidence.into_iter().map(str::to_string).collect(),
            root_cause,
            symptom: id == "generic",
            line: Some(12),
            occurrences: 2,
            values: BTreeMap::new(),
        }
    }

    #[test]
    fn sanitized_report_keeps_actions_and_hides_private_data() {
        let report = DiagnosisReport {
            server_id: "fixture".into(),
            available: true,
            source: Some("logs/latest.log".into()),
            generated_at: "2026-10-20T00:00:00Z".into(),
            exit_code: Some(1),
            root_cause: Some("fileAccess".into()),
            scanned_lines: 42,
            findings: vec![
                diagnosis_finding(
                    "fileAccess",
                    "critical",
                    true,
                    vec![
                        "java.io.IOException: permission denied for C:\\Users\\Alice\\server",
                        "password=private",
                        "cannot open D:\\GameServers\\Private\\world",
                        "connection from 192.168.1.2 rejected",
                    ],
                ),
                diagnosis_finding(
                    "outOfMemory",
                    "critical",
                    false,
                    vec!["java.lang.OutOfMemoryError: Java heap space"],
                ),
            ],
        };
        let text = report_text("en", &report);
        assert!(text.contains("# EasyLauncher crash analysis"));
        assert!(text.contains("## Root cause"));
        assert!(text.contains("Recommended fix"));
        assert!(text.contains("permission denied"));
        assert!(text.contains("Exit code: 1"));
        assert!(text.contains("Other findings"));
        for private in ["Alice", "192.168.1.2", "GameServers", "private"] {
            assert!(!text.contains(private), "report leaked {private}");
        }
        assert!(text.contains("[redacted"));
    }

    #[test]
    fn chinese_report_uses_localized_headings() {
        let report = DiagnosisReport {
            server_id: "fixture".into(),
            available: true,
            source: None,
            generated_at: "2026-10-20T00:00:00Z".into(),
            exit_code: None,
            root_cause: Some("portInUse".into()),
            scanned_lines: 3,
            findings: vec![diagnosis_finding(
                "portInUse",
                "critical",
                true,
                vec!["Failed to bind to port 25565"],
            )],
        };
        let text = report_text("zh", &report);
        assert!(text.contains("## 根因"));
        assert!(text.contains("修复建议"));
        assert!(text.contains("端口被占用"));
        assert!(!text.contains("## Root cause"));
    }

    #[test]
    fn readiness_penalizes_problems_only() {
        let passing = vec![item("install", "ok", vec![]), item("port", "info", vec![])];
        assert_eq!(readiness(&passing), 100);
        let problems = vec![
            item("install", "critical", vec![]),
            item("memory", "warning", vec![]),
        ];
        assert_eq!(readiness(&problems), 67);
        assert_eq!(readiness(&vec![item("eula", "critical", vec![]); 8]), 0);
    }
}
