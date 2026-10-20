//! Deterministic crash signatures. The first critical cause found in the log is
//! treated as the root cause; later errors are kept as contributing evidence so
//! the last stack frame does not hide the original failure.

use crate::extensions::CrashAnalyzer;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct CrashFinding {
    pub id: &'static str,
    pub severity: &'static str,
    pub symptom: bool,
    pub evidence: Vec<String>,
    pub first_line: Option<usize>,
    pub occurrences: usize,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct CrashAnalysis {
    pub findings: Vec<CrashFinding>,
    pub root_cause: Option<&'static str>,
    pub scanned_lines: usize,
}

struct Signature {
    id: &'static str,
    severity: &'static str,
    /// Symptoms such as a watchdog dump explain the shutdown but not why it began.
    symptom: bool,
    needles: &'static [&'static str],
}

const SIGNATURES: &[Signature] = &[
    Signature {
        id: "javaVersion",
        severity: "critical",
        symptom: false,
        needles: &[
            "unsupportedclassversionerror",
            "class file version",
            "compiled by a more recent version",
            "requires java ",
            "java version mismatch",
        ],
    },
    Signature {
        id: "jvmOptions",
        severity: "critical",
        symptom: false,
        needles: &[
            "unrecognized vm option",
            "improperly specified vm option",
            "could not create the java virtual machine",
            "multiple garbage collectors selected",
        ],
    },
    Signature {
        id: "outOfMemory",
        severity: "critical",
        symptom: false,
        needles: &[
            "outofmemoryerror",
            "java heap space",
            "gc overhead limit exceeded",
            "insufficient memory for the java runtime",
            "unable to allocate memory",
        ],
    },
    Signature {
        id: "portInUse",
        severity: "critical",
        symptom: false,
        needles: &[
            "address already in use",
            "failed to bind to port",
            "bindexception",
            "perhaps a server is already running on that port",
        ],
    },
    Signature {
        id: "diskSpace",
        severity: "critical",
        symptom: false,
        needles: &["no space left on device", "not enough space on the disk"],
    },
    Signature {
        id: "fileAccess",
        severity: "critical",
        symptom: false,
        needles: &[
            "accessdeniedexception",
            "permission denied",
            "the process cannot access the file",
        ],
    },
    Signature {
        id: "missingJar",
        severity: "critical",
        symptom: false,
        needles: &["unable to access jarfile"],
    },
    Signature {
        id: "eula",
        severity: "critical",
        symptom: false,
        needles: &["you need to agree to the eula", "eula=false"],
    },
    Signature {
        id: "worldCorruption",
        severity: "critical",
        symptom: false,
        needles: &[
            "region file format",
            "failed to load chunk",
            "chunk file at",
            "world is corrupt",
            "corrupted chunk",
            "invalid chunk",
            "exception generating new chunk",
        ],
    },
    Signature {
        id: "modConflict",
        severity: "warning",
        symptom: false,
        needles: &[
            "duplicatemodsfoundexception",
            "duplicate mod",
            "modresolutionexception",
            "incompatible mod set",
            "incompatible modset",
            "mixin apply failed",
        ],
    },
    Signature {
        id: "missingDependency",
        severity: "warning",
        symptom: false,
        needles: &[
            "missing or unsupported mandatory dependencies",
            "which is missing",
            "could not find required mod",
        ],
    },
    Signature {
        id: "pluginError",
        severity: "warning",
        symptom: false,
        needles: &[
            "could not load 'plugins/",
            "error occurred while enabling",
            "failed to enable plugin",
        ],
    },
    Signature {
        id: "watchdog",
        severity: "warning",
        symptom: true,
        needles: &["a single server tick took", "server thread dump"],
    },
];

const GENERIC: &[&str] = &["encountered an unexpected exception", "exception in thread"];

fn priority(id: &str) -> usize {
    SIGNATURES
        .iter()
        .position(|signature| signature.id == id)
        .unwrap_or(usize::MAX)
}

fn number_after(text: &str, marker: &str) -> Option<u64> {
    let lower = text.to_ascii_lowercase();
    let start = lower.find(marker)? + marker.len();
    let tail = &text[start..];
    let digit = tail.find(|c: char| c.is_ascii_digit())?;
    let number: String = tail[digit..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    number.parse().ok()
}

fn port(text: &str) -> Option<u16> {
    number_after(text, "port")
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn java_for_class_file(version: u64) -> String {
    if (45..=48).contains(&version) {
        format!("1.{}", version - 44)
    } else if version >= 49 {
        (version - 44).to_string()
    } else {
        version.to_string()
    }
}

fn quoted_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(opening) if c == opening => {
                if !current.trim().is_empty() {
                    words.push(current.trim().to_string());
                }
                current.clear();
                quote = None;
            }
            Some(_) => current.push(c),
            None if c == '\'' || c == '"' || c == '`' => {
                current.clear();
                quote = Some(c);
            }
            None => {}
        }
    }
    words
}

fn mod_names(lines: &[&str]) -> Vec<String> {
    let mut names = Vec::new();
    for line in lines.iter().take(10) {
        for word in quoted_words(line) {
            let valid = word.len() >= 2
                && word.len() <= 80
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-.:/".contains(c));
            if valid && !names.iter().any(|name| name == &word) {
                names.push(word);
            }
            if names.len() >= 8 {
                return names;
            }
        }
    }
    names
}

fn plugin_name(lines: &[&str]) -> Option<String> {
    for line in lines.iter().take(6) {
        for word in quoted_words(line) {
            if !word.to_ascii_lowercase().ends_with(".jar") {
                continue;
            }
            let name = word
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&word)
                .trim_end_matches(".jar")
                .trim_end_matches(".JAR");
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

fn jvm_option(lines: &[&str]) -> Option<String> {
    lines
        .iter()
        .take(6)
        .flat_map(|line| line.split_whitespace())
        .find(|word| word.starts_with("-X") || word.starts_with("-XX"))
        .map(|word| {
            word.trim_matches(|c: char| c == '\'' || c == '"')
                .to_string()
        })
        .filter(|word| word.len() <= 120)
}

fn memory_kind(line: &str) -> &'static str {
    let lower = line.to_ascii_lowercase();
    if lower.contains("java heap space") {
        "heap"
    } else if lower.contains("metaspace") {
        "metaspace"
    } else if lower.contains("gc overhead") {
        "gc"
    } else if lower.contains("native") || lower.contains("unable to allocate") {
        "native"
    } else {
        "unknown"
    }
}

fn chunk_position(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let tail = &line[lower.find("chunk")? + "chunk".len()..];
    let mut numbers = Vec::new();
    let chars: Vec<char> = tail.chars().collect();
    let mut index = 0;
    while index < chars.len() && numbers.len() < 3 {
        let start = index;
        if chars[index] == '-' {
            index += 1;
        }
        let digits = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
        if index > digits && numbers.len() < 3 {
            let value: String = chars[start..index].iter().collect();
            numbers.push(value);
        } else {
            index += 1;
        }
    }
    if numbers.is_empty() {
        None
    } else {
        Some(numbers.join(", "))
    }
}

fn extract_values(id: &str, lines: &[&str]) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    match id {
        "portInUse" => {
            if let Some(value) = lines.iter().find_map(|line| port(line)) {
                values.insert("port".into(), value.to_string());
            }
        }
        "javaVersion" => {
            for line in lines.iter().take(10) {
                if let Some(version) = number_after(line, "class file version") {
                    values.insert("classFile".into(), version.to_string());
                    values.insert("requiredJava".into(), java_for_class_file(version));
                }
                if let Some(version) = number_after(line, "requires java ") {
                    if version >= 8 && version <= 99 {
                        values.insert("requiredJava".into(), version.to_string());
                    }
                }
            }
        }
        "outOfMemory" => {
            if let Some(line) = lines.first() {
                values.insert("memory".into(), memory_kind(line).into());
            }
        }
        "modConflict" | "missingDependency" => {
            let names = mod_names(lines);
            if !names.is_empty() {
                values.insert("mods".into(), names.join(", "));
            }
        }
        "pluginError" => {
            if let Some(name) = plugin_name(lines) {
                values.insert("plugin".into(), name);
            }
        }
        "jvmOptions" => {
            if let Some(option) = jvm_option(lines) {
                values.insert("option".into(), option);
            }
        }
        "worldCorruption" => {
            if let Some(position) = lines.iter().find_map(|line| chunk_position(line)) {
                values.insert("chunk".into(), position);
            }
        }
        _ => {}
    }
    values
}

fn earliest<'a>(findings: impl Iterator<Item = &'a CrashFinding>) -> Option<&'a CrashFinding> {
    findings.min_by_key(|finding| {
        (
            finding.first_line.unwrap_or(usize::MAX),
            priority(finding.id),
        )
    })
}

/// Scan the available log tail. Findings are ordered with the root cause first,
/// then remaining critical findings, warnings and symptoms by first appearance.
pub fn analyze_text(exit_code: Option<i32>, text: &str) -> CrashAnalysis {
    let lines: Vec<&str> = text.lines().collect();
    let lower: Vec<String> = lines.iter().map(|line| line.to_ascii_lowercase()).collect();
    let mut findings = Vec::new();

    for signature in SIGNATURES {
        let mut matches = Vec::new();
        for (index, line) in lower.iter().enumerate() {
            if signature.needles.iter().any(|needle| line.contains(needle)) {
                matches.push(index);
            }
        }
        if matches.is_empty() {
            continue;
        }
        let mut evidence = Vec::new();
        for &index in matches.iter().take(3) {
            let line: String = lines[index].trim().chars().take(400).collect();
            if !evidence.contains(&line) {
                evidence.push(line);
            }
        }
        let matched_text: Vec<&str> = matches
            .iter()
            .take(10)
            .filter_map(|index| lines.get(*index).copied())
            .collect();
        findings.push(CrashFinding {
            id: signature.id,
            severity: signature.severity,
            symptom: signature.symptom,
            evidence,
            first_line: Some(matches[0] + 1),
            occurrences: matches.len(),
            values: extract_values(signature.id, &matched_text),
        });
    }

    if findings.is_empty() {
        let mut evidence = Vec::new();
        let mut first_line = None;
        for (index, line) in lower.iter().enumerate() {
            if GENERIC.iter().any(|needle| line.contains(needle)) {
                let text: String = lines[index].trim().chars().take(400).collect();
                if !evidence.contains(&text) {
                    evidence.push(text);
                }
                if first_line.is_none() {
                    first_line = Some(index + 1);
                }
                if evidence.len() >= 3 {
                    break;
                }
            }
        }
        if let Some(code) = exit_code.filter(|code| *code != 0) {
            evidence.push(format!("Process exited with code {code}."));
        }
        if !evidence.is_empty() {
            findings.push(CrashFinding {
                id: "generic",
                severity: "warning",
                symptom: true,
                evidence,
                first_line,
                occurrences: 1,
                values: BTreeMap::new(),
            });
        }
    }

    let causes: Vec<&CrashFinding> = findings.iter().filter(|finding| !finding.symptom).collect();
    let root = earliest(
        causes
            .iter()
            .copied()
            .filter(|finding| finding.severity == "critical"),
    )
    .or_else(|| earliest(causes.iter().copied()))
    .or_else(|| earliest(findings.iter()));
    let root_id = root.map(|finding| finding.id);

    findings.sort_by_key(|finding| {
        (
            if Some(finding.id) == root_id { 0 } else { 1 },
            if finding.severity == "critical" { 0 } else { 1 },
            finding.first_line.unwrap_or(usize::MAX),
            priority(finding.id),
        )
    });

    CrashAnalysis {
        findings,
        root_cause: root_id,
        scanned_lines: lines.len(),
    }
}

/// Contract implementation used by diagnostics. The full analysis is returned
/// so the caller can map it to its serialized report and add server context.
pub struct SignatureAnalyzer;

impl CrashAnalyzer for SignatureAnalyzer {
    type Output = CrashAnalysis;

    fn analyze(&self, exit_code: Option<i32>, recent_lines: &[String]) -> CrashAnalysis {
        analyze_text(exit_code, &recent_lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_root_cause_beats_later_errors() {
        let text = "[00:00:01] [Server thread/ERROR]: Failed to bind to port 25565\n\
[00:00:02] [Server thread/ERROR]: Address already in use: bind\n\
[00:00:03] [Server thread/ERROR]: Encountered an unexpected exception\n\
[00:00:04] [Server thread/ERROR]: java.lang.RuntimeException: plugin failed";
        let report = analyze_text(Some(1), text);
        assert_eq!(report.root_cause, Some("portInUse"));
        assert_eq!(report.findings[0].id, "portInUse");
        assert_eq!(report.findings[0].first_line, Some(1));
        assert_eq!(report.findings[0].occurrences, 2);
        assert_eq!(report.findings[0].values["port"], "25565");
        assert!(!report
            .findings
            .iter()
            .any(|finding| finding.id == "generic"));
    }

    #[test]
    fn symptoms_are_not_root_causes() {
        let text = "A single server tick took 120.00 seconds\n\
Duplicate mods found for modid 'example'\n";
        let report = analyze_text(Some(1), text);
        assert_eq!(report.root_cause, Some("modConflict"));
        assert_eq!(report.findings[0].id, "modConflict");
        let watchdog = report
            .findings
            .iter()
            .find(|finding| finding.id == "watchdog")
            .unwrap();
        assert!(watchdog.symptom);
        assert_ne!(report.root_cause, Some("watchdog"));
    }

    #[test]
    fn root_cause_uses_earliest_critical_finding() {
        let text = "java.lang.UnsupportedClassVersionError: class file version 65.0\n\
java.lang.OutOfMemoryError: Java heap space\n";
        let report = analyze_text(None, text);
        assert_eq!(report.root_cause, Some("javaVersion"));
        assert_eq!(report.findings[0].values["requiredJava"], "21");
        assert_eq!(report.findings[0].values["classFile"], "65");
        assert_eq!(report.findings[1].id, "outOfMemory");
    }

    #[test]
    fn extracts_mods_plugins_and_chunk_context() {
        let text = "Duplicate mods found for modid 'create'\n\
Mod 'create' requires 'flywheel', which is missing!\n\
Failed to load chunk (12, -4) in world\n\
Could not load 'plugins/EssentialsX.jar'\n";
        let report = analyze_text(Some(1), text);
        let ids: Vec<&str> = report.findings.iter().map(|finding| finding.id).collect();
        for expected in [
            "modConflict",
            "missingDependency",
            "worldCorruption",
            "pluginError",
        ] {
            assert!(ids.contains(&expected), "missing {expected}");
        }
        let conflict = report
            .findings
            .iter()
            .find(|finding| finding.id == "modConflict")
            .unwrap();
        assert!(conflict.values["mods"].contains("create"));
        let dependency = report
            .findings
            .iter()
            .find(|finding| finding.id == "missingDependency")
            .unwrap();
        assert!(dependency.values["mods"].contains("flywheel"));
        let corruption = report
            .findings
            .iter()
            .find(|finding| finding.id == "worldCorruption")
            .unwrap();
        assert_eq!(corruption.values["chunk"], "12, -4");
        let plugin = report
            .findings
            .iter()
            .find(|finding| finding.id == "pluginError")
            .unwrap();
        assert_eq!(plugin.values["plugin"], "EssentialsX");
    }

    #[test]
    fn exit_code_alone_is_generic() {
        let report = analyze_text(Some(3), "");
        assert_eq!(report.root_cause, Some("generic"));
        assert_eq!(report.findings.len(), 1);
        assert!(report.findings[0].symptom);
        assert!(report.findings[0].evidence[0].contains("code 3"));
    }

    #[test]
    fn contract_returns_the_root_cause() {
        let lines = vec![
            "java.lang.OutOfMemoryError: Java heap space".to_string(),
            "Encountered an unexpected exception".to_string(),
        ];
        let analysis = SignatureAnalyzer.analyze(Some(1), &lines);
        assert_eq!(analysis.root_cause, Some("outOfMemory"));
        assert_eq!(analysis.findings[0].id, "outOfMemory");
    }
}
