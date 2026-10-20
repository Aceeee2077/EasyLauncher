//! Read-only dependency inspection. JAR metadata is never executed or extracted.
use crate::{
    error::{AppError, Result},
    models::ServerType,
    server::manager::Manager,
    storage,
};
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Seek},
    path::Path,
    sync::Arc,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub requirement: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub code: String,
    pub source: String,
    pub target: String,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub modules: Vec<Module>,
    pub edges: Vec<Edge>,
    pub issues: Vec<Issue>,
    pub remote_checked: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Module {
    pub id: String,
    pub version: String,
    pub file: String,
    #[serde(skip)]
    pub depends: BTreeMap<String, Value>,
    #[serde(skip)]
    pub breaks: BTreeMap<String, Value>,
    #[serde(skip)]
    pub provides: Vec<String>,
    #[serde(skip)]
    pub client_only: bool,
}
fn issue(report: &mut Report, code: &str, source: &str, target: &str, detail: impl ToString) {
    report.issues.push(Issue {
        code: code.into(),
        source: source.into(),
        target: target.into(),
        detail: detail.to_string(),
    });
}
fn read_member<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    max: u64,
) -> Result<Vec<u8>> {
    let f = zip
        .by_name(name)
        .map_err(|e| AppError::new("Missing JAR metadata.", e))?;
    if f.size() > max {
        return Err(AppError::user("JAR metadata exceeds the inspection limit."));
    }
    let mut bytes = Vec::new();
    f.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(AppError::user("JAR metadata exceeds the inspection limit."));
    }
    Ok(bytes)
}
fn jar<R: Read + Seek>(
    reader: R,
    name: &str,
    depth: u8,
    budget: &mut u64,
    report: &mut Report,
) -> Result<()> {
    if report.modules.len() >= 512 || depth > 4 {
        return Err(AppError::user("Nested JAR inspection limit reached."));
    }
    let mut zip =
        zip::ZipArchive::new(reader).map_err(|e| AppError::new("Cannot read JAR archive.", e))?;
    let bytes = read_member(&mut zip, "fabric.mod.json", 1024 * 1024)?;
    let v: Value = serde_json::from_slice(&bytes)?;
    let id = v["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::user("Missing Fabric mod ID."))?;
    let map = |key: &str| {
        v[key]
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    };
    report.modules.push(Module {
        id: id.into(),
        version: v["version"].as_str().unwrap_or("?").into(),
        file: name.into(),
        depends: map("depends"),
        breaks: map("breaks"),
        provides: v["provides"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        client_only: v["environment"].as_str() == Some("client"),
    });
    if let Some(nested) = v["jars"].as_array() {
        for item in nested {
            let Some(path) = item["file"].as_str() else {
                continue;
            };
            // Archive member names are only looked up in memory, never used as filesystem paths.
            let data = read_member(&mut zip, path, (*budget).min(32 * 1024 * 1024))?;
            *budget = budget
                .checked_sub(data.len() as u64)
                .ok_or_else(|| AppError::user("Nested JAR byte limit reached."))?;
            if let Err(e) = jar(
                Cursor::new(data),
                &format!("{name}!{path}"),
                depth + 1,
                budget,
                report,
            ) {
                issue(report, "unreadable", name, path, e.message);
            }
        }
    }
    Ok(())
}

// Common numeric Fabric predicates. Unsupported syntax remains explicitly unknown.
fn numeric(v: &str) -> Option<Vec<u64>> {
    if v.contains(['-', '+']) {
        return None;
    }
    v.split('.').map(|n| n.parse().ok()).collect()
}
fn predicate(version: &str, requirement: &Value) -> Option<bool> {
    if let Some(array) = requirement.as_array() {
        let matches: Vec<_> = array.iter().map(|v| predicate(version, v)).collect();
        return if matches.contains(&Some(true)) {
            Some(true)
        } else if matches.contains(&None) {
            None
        } else {
            Some(false)
        };
    }
    let req = requirement.as_str()?;
    if req == "*" {
        return Some(true);
    }
    let parts: Vec<_> = req.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let mut unknown = false;
    for part in parts {
        let (op, value) = [">=", "<=", ">", "<", "=", "~", "^"]
            .iter()
            .find_map(|op| part.strip_prefix(op).map(|v| (*op, v)))
            .unwrap_or(("=", part));
        let outcome = if value.contains(['*', 'x', 'X']) && op == "=" {
            let prefix: Vec<_> = value
                .split('.')
                .take_while(|p| !matches!(*p, "*" | "x" | "X"))
                .collect();
            let actual: Vec<_> = version.split('.').collect();
            Some(actual.starts_with(&prefix))
        } else if op == "=" && version == value {
            Some(true)
        } else if let (Some(mut a), Some(mut b)) = (numeric(version), numeric(value)) {
            let original = b.clone();
            let len = a.len().max(b.len());
            a.resize(len, 0);
            b.resize(len, 0);
            match op {
                "=" => Some(a == b),
                ">=" => Some(a >= b),
                "<=" => Some(a <= b),
                ">" => Some(a > b),
                "<" => Some(a < b),
                "~" | "^" => {
                    let mut upper = b.clone();
                    let idx = if op == "~" {
                        if original.len() > 1 {
                            1
                        } else {
                            0
                        }
                    } else {
                        original
                            .iter()
                            .position(|n| *n > 0)
                            .unwrap_or(original.len() - 1)
                    };
                    upper[idx] = upper[idx].checked_add(1)?;
                    for n in upper.iter_mut().skip(idx + 1) {
                        *n = 0;
                    }
                    Some(a >= b && a < upper)
                }
                _ => None,
            }
        } else {
            None
        };
        if outcome == Some(false) {
            return Some(false);
        }
        unknown |= outcome.is_none();
    }
    if unknown {
        None
    } else {
        Some(true)
    }
}
fn inspect(report: &mut Report, minecraft: &str, java: Option<u32>) {
    let modules = report.modules.clone();
    let mut installed: BTreeMap<String, String> = BTreeMap::new();
    installed.insert("minecraft".into(), minecraft.into());
    if let Some(java) = java {
        installed.insert("java".into(), java.to_string());
    }
    for module in &modules {
        if installed
            .insert(module.id.clone(), module.version.clone())
            .is_some()
        {
            issue(
                report,
                "duplicate",
                &module.file,
                &module.id,
                &module.version,
            );
        }
        for alias in &module.provides {
            installed
                .entry(alias.clone())
                .or_insert_with(|| module.version.clone());
        }
        if module.client_only {
            issue(report, "clientOnly", &module.file, &module.id, "");
        }
    }
    for module in &modules {
        if module.client_only {
            continue;
        }
        for (relation, requirements) in [
            ("required", &module.depends),
            ("incompatible", &module.breaks),
        ] {
            for (target, req) in requirements {
                report.edges.push(Edge {
                    source: module.id.clone(),
                    target: target.clone(),
                    relation: relation.into(),
                    requirement: req.to_string(),
                });
                let found = installed.get(target);
                let matched = found.and_then(|version| predicate(version, req));
                let code = match (relation, found, matched) {
                    ("required", None, _) if matches!(target.as_str(), "fabricloader" | "java") => {
                        Some("unknown")
                    }
                    ("required", None, _) => Some("missing"),
                    ("required", Some(_), Some(false)) => Some("version"),
                    ("incompatible", Some(_), Some(true)) => Some("conflict"),
                    (_, Some(_), None) => Some("unknown"),
                    _ => None,
                };
                if let Some(code) = code {
                    issue(
                        report,
                        code,
                        &module.id,
                        target,
                        format!("{} / {}", found.map(String::as_str).unwrap_or("?"), req),
                    );
                }
            }
        }
    }
}
fn local(dir: &Path, software: ServerType, minecraft: &str, java: Option<u32>) -> Result<Report> {
    let mut report = Report::default();
    if software != ServerType::Fabric {
        return Ok(report);
    }
    let path = storage::child(dir, "mods")?;
    if !path.exists() {
        return Ok(report);
    }
    let mut budget = 64 * 1024 * 1024;
    let mut files: Vec<_> = fs::read_dir(path)?.collect::<std::io::Result<_>>()?;
    files.sort_by_key(|e| e.file_name());
    for (n, file) in files
        .iter()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("jar"))
        })
        .enumerate()
    {
        if n >= 256 {
            issue(&mut report, "unreadable", "mods", "", "256 JAR limit");
            break;
        }
        storage::no_links(&file.path())?;
        let name = file.file_name().to_string_lossy().into_owned();
        if let Err(e) = jar(
            fs::File::open(file.path())?,
            &name,
            0,
            &mut budget,
            &mut report,
        ) {
            issue(&mut report, "unreadable", &name, "", e.message);
        }
    }
    inspect(&mut report, minecraft, java);
    Ok(report)
}
pub async fn check(m: Arc<Manager>, id: String, remote: bool) -> Result<Report> {
    let runtime = m.runtime(&id)?;
    let _gate = runtime.gate.lock().await;
    let server = m.metadata(&id)?;
    let dir = m.dir(&id)?;
    let java = if let Some(path) = &server.java_path {
        crate::java::detector::inspect(path)
            .await
            .ok()
            .map(|j| j.major)
    } else {
        None
    };
    let mut report = tokio::task::spawn_blocking(move || {
        local(&dir, server.server_type, &server.minecraft_version, java)
    })
    .await
    .map_err(|e| AppError::new("Dependency inspection failed.", e))??;
    if remote {
        let listing = crate::modrinth::listing(&m, &id)?;
        if listing.entries.len() > 128 {
            return Err(AppError::user(
                "Check at most 128 managed projects at a time.",
            ));
        }
        let client = m.client();
        let mut checks = stream::iter(listing.entries.iter().cloned())
            .map(|entry| {
                let client = client.clone();
                async move {
                    let result =
                        crate::modrinth::version_dependencies(&client, &entry.version_id).await;
                    (entry, result)
                }
            })
            .buffer_unordered(4);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut complete = true;
        loop {
            let next = match tokio::time::timeout_at(deadline, checks.next()).await {
                Ok(value) => value,
                Err(_) => {
                    issue(&mut report,"remoteError","Modrinth","","Inspection timed out after 30 seconds; retry to complete the remote graph.");
                    complete = false;
                    break;
                }
            };
            let Some((entry, result)) = next else {
                break;
            };
            match result {
                Ok(deps) => {
                    for dep in deps {
                        let target = dep
                            .project_id
                            .as_deref()
                            .or(dep.version_id.as_deref())
                            .unwrap_or("?");
                        report.edges.push(Edge {
                            source: entry.project_id.clone(),
                            target: target.into(),
                            relation: dep.dependency_type.clone(),
                            requirement: dep.version_id.clone().unwrap_or_else(|| "*".into()),
                        });
                        let present = listing.entries.iter().find(|e| {
                            dep.project_id.as_ref() == Some(&e.project_id)
                                || dep.version_id.as_ref() == Some(&e.version_id)
                        });
                        let code = match (dep.dependency_type.as_str(), present) {
                            ("required", None) => Some("remoteMissing"),
                            ("required", Some(e))
                                if dep.version_id.as_ref().is_some_and(|v| v != &e.version_id) =>
                            {
                                Some("version")
                            }
                            ("incompatible", Some(e))
                                if dep.version_id.as_ref().is_none_or(|v| v == &e.version_id) =>
                            {
                                Some("conflict")
                            }
                            _ => None,
                        };
                        if let Some(code) = code {
                            issue(
                                &mut report,
                                code,
                                &entry.title,
                                target,
                                dep.version_id.as_deref().unwrap_or(""),
                            );
                        }
                    }
                }
                Err(e) => {
                    complete = false;
                    issue(&mut report, "remoteError", &entry.title, "", e.message);
                }
            }
        }
        report.remote_checked = complete;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn predicates_preserve_uncertainty() {
        for (v, req, expected) in [
            ("1.21.4", ">=1.20 <1.22", Some(true)),
            ("1.21.4", "1.20.x", Some(false)),
            ("0.4.2", "^0.4.0", Some(true)),
            ("0.5.0", "^0.4.0", Some(false)),
            ("1.2.3-beta", ">=1.2", None),
        ] {
            assert_eq!(predicate(v, &json!(req)), expected, "{v} {req}");
        }
        assert_eq!(predicate("2.0", &json!(["1.*", ">=2.0"])), Some(true));
    }
    #[test]
    fn duplicate_missing_and_breaks() {
        let module = |id: &str, depends, breaks| Module {
            id: id.into(),
            version: "1.0".into(),
            file: format!("{id}.jar"),
            depends,
            breaks,
            provides: vec![],
            client_only: false,
        };
        let mut report = Report::default();
        report.modules = vec![
            module(
                "a",
                BTreeMap::from([("missing".into(), json!("*"))]),
                BTreeMap::from([("b".into(), json!(">=1"))]),
            ),
            module("b", BTreeMap::new(), BTreeMap::new()),
            module("b", BTreeMap::new(), BTreeMap::new()),
        ];
        inspect(&mut report, "1.21", Some(21));
        for code in ["duplicate", "missing", "conflict"] {
            assert!(report.issues.iter().any(|i| i.code == code));
        }
    }
    #[test]
    fn nested_fabric_metadata_is_read_without_extraction() {
        fn zip_bytes(meta: &str, child: Option<&[u8]>) -> Vec<u8> {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
            z.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(meta.as_bytes()).unwrap();
            if let Some(child) = child {
                z.start_file("nested.jar", zip::write::SimpleFileOptions::default())
                    .unwrap();
                z.write_all(child).unwrap();
            }
            z.finish().unwrap().into_inner()
        }
        let nested = zip_bytes(r#"{"id":"child","version":"1"}"#, None);
        let parent = zip_bytes(
            r#"{"id":"parent","version":"1","jars":[{"file":"nested.jar"}],"depends":{"child":"*"}}"#,
            Some(&nested),
        );
        let mut report = Report::default();
        jar(
            Cursor::new(parent),
            "parent.jar",
            0,
            &mut (64 * 1024 * 1024),
            &mut report,
        )
        .unwrap();
        inspect(&mut report, "1.21", Some(21));
        assert_eq!(report.modules.len(), 2);
        assert!(report.issues.is_empty());
    }
}
