use crate::{
    error::Result,
    minecraft::properties::Properties,
    models::{JvmPreset, Status},
    server::manager::{read_limited, Manager},
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosisReport {
    pub server_id: String,
    pub available: bool,
    pub source: Option<String>,
    pub generated_at: String,
    pub findings: Vec<DiagnosisFinding>,
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

struct Pattern {
    id: &'static str,
    severity: &'static str,
    needles: &'static [&'static str],
}

const PATTERNS: &[Pattern] = &[
    Pattern {
        id: "jvmOptions",
        severity: "critical",
        needles: &[
            "unrecognized vm option",
            "improperly specified vm option",
            "could not create the java virtual machine",
            "multiple garbage collectors selected",
        ],
    },
    Pattern {
        id: "outOfMemory",
        severity: "critical",
        needles: &[
            "outofmemoryerror",
            "java heap space",
            "gc overhead limit exceeded",
        ],
    },
    Pattern {
        id: "portInUse",
        severity: "critical",
        needles: &[
            "address already in use",
            "failed to bind to port",
            "bindexception",
        ],
    },
    Pattern {
        id: "javaVersion",
        severity: "critical",
        needles: &[
            "unsupportedclassversionerror",
            "class file version",
            "compiled by a more recent version",
        ],
    },
    Pattern {
        id: "missingJar",
        severity: "critical",
        needles: &["unable to access jarfile"],
    },
    Pattern {
        id: "eula",
        severity: "critical",
        needles: &["you need to agree to the eula", "eula=false"],
    },
    Pattern {
        id: "diskSpace",
        severity: "critical",
        needles: &["no space left on device", "not enough space on the disk"],
    },
    Pattern {
        id: "worldCorruption",
        severity: "critical",
        needles: &[
            "region file",
            "failed to load chunk",
            "chunk file at",
            "world is corrupt",
        ],
    },
    Pattern {
        id: "fileAccess",
        severity: "critical",
        needles: &[
            "accessdeniedexception",
            "permission denied",
            "the process cannot access the file",
        ],
    },
    Pattern {
        id: "modConflict",
        severity: "warning",
        needles: &[
            "duplicatemodsfoundexception",
            "duplicate mod",
            "modresolutionexception",
            "incompatible mod set",
        ],
    },
    Pattern {
        id: "missingDependency",
        severity: "warning",
        needles: &[
            "missing or unsupported mandatory dependencies",
            "which is missing",
        ],
    },
    Pattern {
        id: "watchdog",
        severity: "warning",
        needles: &["a single server tick took", "server thread dump"],
    },
    Pattern {
        id: "pluginError",
        severity: "warning",
        needles: &["could not load 'plugins/", "error occurred while enabling"],
    },
];

const GENERIC: &[&str] = &["encountered an unexpected exception", "exception in thread"];

fn matched_lines(lines: &[&str], lower: &[String], needles: &[&str]) -> Vec<String> {
    let mut evidence = Vec::new();
    for (line, low) in lines.iter().zip(lower.iter()) {
        if needles.iter().any(|n| low.contains(n)) {
            let text: String = line.trim().chars().take(400).collect();
            if !evidence.contains(&text) {
                evidence.push(text);
            }
            if evidence.len() >= 3 {
                break;
            }
        }
    }
    evidence
}

fn analyze(server_id: &str, source: Option<String>, text: &str) -> DiagnosisReport {
    let lines: Vec<&str> = text.lines().collect();
    let lower: Vec<String> = lines.iter().map(|l| l.to_ascii_lowercase()).collect();
    let mut findings = Vec::new();
    for pattern in PATTERNS {
        let evidence = matched_lines(&lines, &lower, pattern.needles);
        if !evidence.is_empty() {
            findings.push(DiagnosisFinding {
                id: pattern.id.into(),
                severity: pattern.severity.into(),
                evidence,
            });
        }
    }
    // A generic error is only interesting when no specific signature matched.
    if findings.is_empty() {
        let evidence = matched_lines(&lines, &lower, GENERIC);
        if !evidence.is_empty() {
            findings.push(DiagnosisFinding {
                id: "generic".into(),
                severity: "warning".into(),
                evidence,
            });
        }
    }
    findings.sort_by_key(|f| if f.severity == "critical" { 0 } else { 1 });
    DiagnosisReport {
        server_id: server_id.into(),
        available: source.is_some(),
        source,
        generated_at: chrono::Utc::now().to_rfc3339(),
        findings,
    }
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
pub fn diagnose(m: &Manager, id: &str) -> Result<DiagnosisReport> {
    let (source, text) = log_text(m, id)?;
    Ok(analyze(id, source, &text))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_common_failures() {
        let text = "[12:00:00] [Server thread/ERROR]: java.lang.OutOfMemoryError: Java heap space\n\
[12:00:00] [Server thread/ERROR]: java.lang.UnsupportedClassVersionError: class file version 65.0\n";
        let report = analyze("fixture", Some("logs/latest.log".into()), text);
        assert!(report.available);
        assert_eq!(report.findings[0].id, "outOfMemory");
        assert!(report.findings.iter().any(|f| f.id == "javaVersion"));
        assert_eq!(report.findings[0].evidence.len(), 1);
    }
    #[test]
    fn generic_only_without_specific_signal() {
        let generic = analyze(
            "fixture",
            Some("logs/latest.log".into()),
            "x Exception in thread \"main\"",
        );
        assert_eq!(generic.findings[0].id, "generic");
        let specific = analyze(
            "fixture",
            Some("logs/latest.log".into()),
            "Exception in thread \"main\"\nAddress already in use",
        );
        assert_eq!(specific.findings.len(), 1);
        assert_eq!(specific.findings[0].id, "portInUse");
        assert_eq!(specific.findings[0].severity, "critical");
    }
    #[test]
    fn missing_source_is_reported() {
        let report = analyze("fixture", None, "");
        assert!(!report.available);
        assert!(report.findings.is_empty());
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
