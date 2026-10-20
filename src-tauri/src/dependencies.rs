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
use sha1::Sha1;
use sha2::Digest;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
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
    /// `mod` for Fabric, `plugin` for Paper/Bukkit metadata.
    #[serde(default)]
    pub kind: String,
    #[serde(skip)]
    pub depends: BTreeMap<String, Value>,
    #[serde(skip)]
    pub optional: BTreeMap<String, Value>,
    #[serde(skip)]
    pub breaks: BTreeMap<String, Value>,
    #[serde(skip)]
    pub provides: Vec<String>,
    #[serde(skip)]
    pub client_only: bool,
    #[serde(skip)]
    pub sha1: Option<String>,
    #[serde(skip)]
    pub api_version: Option<String>,
}
fn issue(report: &mut Report, code: &str, source: &str, target: &str, detail: impl ToString) {
    let detail = detail.to_string();
    if report.issues.iter().any(|item| {
        item.code == code && item.source == source && item.target == target && item.detail == detail
    }) {
        return;
    }
    report.issues.push(Issue {
        code: code.into(),
        source: source.into(),
        target: target.into(),
        detail,
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
fn member_if_present<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    max: u64,
) -> Option<Vec<u8>> {
    let mut file = zip.by_name(name).ok()?;
    if file.size() > max {
        return None;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > max {
        return None;
    }
    Some(bytes)
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
    if let Some(bytes) = member_if_present(&mut zip, "fabric.mod.json", 1024 * 1024) {
        return fabric_module(bytes, name, depth, budget, report, &mut zip);
    }
    // Paper reads paper-plugin.yml first; plugin.yml covers Bukkit, Spigot and
    // older Paper plugins. Both are read in memory and never extracted.
    for (member, kind) in [("paper-plugin.yml", "plugin"), ("plugin.yml", "plugin")] {
        if let Some(bytes) = member_if_present(&mut zip, member, 256 * 1024) {
            return plugin_module(bytes, name, kind, report);
        }
    }
    Err(AppError::user(
        "No fabric.mod.json, paper-plugin.yml or plugin.yml was found.",
    ))
}
fn fabric_module<R: Read + Seek>(
    bytes: Vec<u8>,
    name: &str,
    depth: u8,
    budget: &mut u64,
    report: &mut Report,
    zip: &mut zip::ZipArchive<R>,
) -> Result<()> {
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
        kind: "mod".into(),
        depends: map("depends"),
        optional: map("recommends"),
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
        sha1: None,
        api_version: None,
    });
    if let Some(nested) = v["jars"].as_array() {
        for item in nested {
            let Some(path) = item["file"].as_str() else {
                continue;
            };
            // Archive member names are only looked up in memory, never used as filesystem paths.
            let data = read_member(zip, path, (*budget).min(32 * 1024 * 1024))?;
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
fn plugin_module(bytes: Vec<u8>, name: &str, kind: &str, report: &mut Report) -> Result<()> {
    let text = String::from_utf8_lossy(&bytes);
    let lines = yaml_lines(&text);
    let id = top_scalar(&lines, "name")
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AppError::user("Missing plugin name."))?;
    let version = top_scalar(&lines, "version").unwrap_or_else(|| "?".into());
    let api_version = top_scalar(&lines, "api-version");
    let mut depends = BTreeMap::new();
    for dependency in yaml_list(&lines, "depend") {
        depends.insert(dependency, Value::String("*".into()));
    }
    let mut optional = BTreeMap::new();
    for dependency in yaml_list(&lines, "softdepend") {
        optional.insert(dependency, Value::String("*".into()));
    }
    // paper-plugin.yml declares dependencies as nested server/bootstrap maps.
    for (dependency, required) in paper_dependencies(&lines) {
        if required {
            depends.insert(dependency, Value::String("*".into()));
        } else {
            optional.insert(dependency, Value::String("*".into()));
        }
    }
    report.modules.push(Module {
        id,
        version,
        file: name.into(),
        kind: kind.into(),
        depends,
        optional,
        breaks: BTreeMap::new(),
        provides: Vec::new(),
        client_only: false,
        sha1: None,
        api_version,
    });
    Ok(())
}

// A deliberately small YAML subset: plugin metadata uses flat mappings, block
// or inline lists, and one nested `dependencies` map. Unknown structures are
// ignored rather than failing the whole file.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (Some('"'), '\\') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (None, c @ ('"' | '\'')) => quote = Some(c),
            (None, '#') => {
                let before = line[..index].chars().next_back();
                if before.map(|c| c.is_whitespace()).unwrap_or(true) {
                    return &line[..index];
                }
            }
            _ => {}
        }
    }
    line
}
fn yaml_lines(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .filter_map(|raw| {
            let line = strip_comment(raw);
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed == "---" || trimmed == "..." {
                return None;
            }
            let leading = line.len() - line.trim_start_matches([' ', '\t']).len();
            let indent = line[..leading]
                .chars()
                .fold(0, |n, c| n + if c == '\t' { 2 } else { 1 });
            Some((indent, trimmed.to_string()))
        })
        .collect()
}
fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let first = value.chars().next().unwrap();
        let last = value.chars().next_back().unwrap();
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            let inner = &value[1..value.len() - 1];
            return if first == '"' {
                inner
                    .replace("\\\"", "\"")
                    .replace("\\n", "\n")
                    .replace("\\\\", "\\")
            } else {
                inner.replace("''", "'")
            };
        }
    }
    value.to_string()
}
fn split_key(line: &str) -> Option<(String, String)> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (Some('"'), '\\') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (None, c @ ('"' | '\'')) => quote = Some(c),
            (None, ':') => {
                let key = unquote(&line[..index]);
                if key.is_empty() {
                    return None;
                }
                return Some((key, line[index + 1..].trim().to_string()));
            }
            _ => {}
        }
    }
    None
}
fn top_line(lines: &[(usize, String)], key: &str) -> Option<(usize, String)> {
    lines
        .iter()
        .enumerate()
        .find_map(|(index, (indent, line))| {
            if *indent != 0 {
                return None;
            }
            let (found, value) = split_key(line)?;
            (found == key).then_some((index, value))
        })
}
fn top_scalar(lines: &[(usize, String)], key: &str) -> Option<String> {
    let (index, value) = top_line(lines, key)?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.starts_with('|') || value.starts_with('>') {
        let folded = value.starts_with('>');
        let mut parts = Vec::new();
        for (indent, line) in &lines[index + 1..] {
            if *indent == 0 {
                break;
            }
            parts.push(line.as_str());
        }
        return Some(if folded {
            parts.join(" ")
        } else {
            parts.join("\n")
        });
    }
    Some(unquote(value))
}
fn inline_list(value: &str) -> Vec<String> {
    let inner = value
        .trim()
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .unwrap_or(value);
    let mut items = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in inner.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (None, c @ ('"' | '\'')) => quote = Some(c),
            (None, ',') => {
                items.push(unquote(current.trim()));
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    items.push(unquote(current.trim()));
    items.retain(|item| !item.is_empty());
    items
}
fn yaml_list(lines: &[(usize, String)], key: &str) -> Vec<String> {
    let Some((index, value)) = top_line(lines, key) else {
        return Vec::new();
    };
    if value.trim().starts_with('[') {
        return inline_list(&value);
    }
    if !value.trim().is_empty() {
        return vec![unquote(&value)];
    }
    let mut items = Vec::new();
    for (indent, line) in &lines[index + 1..] {
        if *indent == 0 {
            break;
        }
        let item = line.trim();
        if let Some(item) = item.strip_prefix("- ") {
            items.push(unquote(item));
        } else if item == "-" {
            continue;
        } else if !items.is_empty() {
            break;
        }
    }
    items.retain(|item| !item.is_empty());
    items
}
/// Required and optional plugin IDs from a Paper `dependencies` mapping.
fn paper_dependencies(lines: &[(usize, String)]) -> Vec<(String, bool)> {
    let Some((start, _)) = top_line(lines, "dependencies") else {
        return Vec::new();
    };
    let mut entries: Vec<(String, bool)> = Vec::new();
    let mut section_indent: Option<usize> = None;
    let mut plugin_indent: Option<usize> = None;
    for (indent, line) in &lines[start + 1..] {
        if *indent == 0 {
            break;
        }
        let Some((key, value)) = split_key(line) else {
            continue;
        };
        match section_indent {
            None => {
                section_indent = Some(*indent);
                continue;
            }
            Some(section) if *indent <= section => {
                if *indent < section {
                    break;
                }
                plugin_indent = None;
                continue;
            }
            Some(_) => match plugin_indent {
                None => {
                    plugin_indent = Some(*indent);
                    entries.push((key, true));
                }
                Some(plugin) if *indent == plugin => entries.push((key, true)),
                Some(plugin) if *indent > plugin => {
                    if key == "required" {
                        if let Some(last) = entries.last_mut() {
                            last.1 = !matches!(value.trim(), "false" | "no" | "off");
                        }
                    }
                }
                _ => {}
            },
        }
    }
    let mut merged: BTreeMap<String, bool> = BTreeMap::new();
    for (id, required) in entries {
        merged
            .entry(id)
            .and_modify(|current| *current |= required)
            .or_insert(required);
    }
    merged.into_iter().collect()
}

// Fabric predicates follow Semantic Versioning precedence, including
// pre-release ordering, build metadata, partial versions and wildcards.
// Syntax we cannot parse stays explicitly unknown instead of guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PreId {
    Num(u64),
    Text(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct SemVer {
    nums: Vec<u64>,
    pre: Vec<PreId>,
}
impl Ord for PreId {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (PreId::Num(a), PreId::Num(b)) => a.cmp(b),
            (PreId::Text(a), PreId::Text(b)) => a.cmp(b),
            (PreId::Num(_), PreId::Text(_)) => Ordering::Less,
            (PreId::Text(_), PreId::Num(_)) => Ordering::Greater,
        }
    }
}
impl PartialOrd for PreId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        let len = self.nums.len().max(other.nums.len());
        for index in 0..len {
            let a = self.nums.get(index).copied().unwrap_or(0);
            let b = other.nums.get(index).copied().unwrap_or(0);
            match a.cmp(&b) {
                Ordering::Equal => {}
                order => return order,
            }
        }
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    match a.cmp(b) {
                        Ordering::Equal => {}
                        order => return order,
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        }
    }
}
impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
fn semver(raw: &str) -> Option<SemVer> {
    let raw = raw.trim().trim_start_matches(['v', 'V']);
    let raw = raw.split('+').next()?;
    let (core, pre) = match raw.split_once('-') {
        Some((core, pre)) => (core, pre),
        None => (raw, ""),
    };
    let mut nums = Vec::new();
    for part in core.split('.') {
        if part.is_empty() {
            return None;
        }
        nums.push(part.parse().ok()?);
    }
    if nums.is_empty() {
        return None;
    }
    let pre = if pre.is_empty() {
        Vec::new()
    } else {
        pre.split('.')
            .map(|id| {
                id.parse()
                    .map(PreId::Num)
                    .unwrap_or_else(|_| PreId::Text(id.into()))
            })
            .collect()
    };
    Some(SemVer { nums, pre })
}
/// A partial or wildcard version matches a prefix of the numeric components.
fn prefix_match(version: &SemVer, definite: &[u64]) -> bool {
    let mut actual = version.nums.clone();
    if actual.len() < definite.len() {
        actual.resize(definite.len(), 0);
    }
    actual[..definite.len()] == definite[..]
}
fn comparison(version: &SemVer, op: &str, value: &str, prerelease: bool) -> Option<bool> {
    let wildcard = value.split('.').any(|p| matches!(p, "*" | "x" | "X"));
    if op == "=" && !prerelease {
        if !wildcard && value.split('.').count() >= 3 {
            return Some(version == &semver(value)?);
        }
        let mut definite = Vec::new();
        let mut saw_wildcard = false;
        for part in value.split('.') {
            if matches!(part, "*" | "x" | "X") {
                saw_wildcard = true;
            } else if saw_wildcard {
                return None;
            } else {
                definite.push(part.parse::<u64>().ok()?);
            }
        }
        return Some(prefix_match(version, &definite));
    }
    let mut base = semver(value)?;
    // A trailing `-` explicitly includes the stated version's pre-releases.
    if prerelease && base.pre.is_empty() && matches!(op, ">=" | "=" | "~" | "^") {
        base.pre.push(PreId::Num(0));
    }
    let in_range = match op {
        ">=" => version >= &base,
        ">" => version > &base,
        "<=" => version <= &base,
        "<" => version < &base,
        "=" => version == &base,
        "~" | "^" => {
            let count = base.nums.len();
            let index = if op == "~" {
                usize::from(count > 1)
            } else {
                base.nums
                    .iter()
                    .position(|n| *n > 0)
                    .unwrap_or(count.saturating_sub(1))
            };
            let mut upper = base.nums.clone();
            upper[index] = upper[index].checked_add(1)?;
            for n in upper.iter_mut().skip(index + 1) {
                *n = 0;
            }
            let upper = SemVer {
                nums: upper,
                pre: Vec::new(),
            };
            version >= &base && version < &upper
        }
        _ => return None,
    };
    Some(in_range)
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
    let req = req.trim();
    if req == "*" || req.is_empty() {
        return Some(true);
    }
    let Some(version) = semver(version) else {
        return None;
    };
    // `||` separates alternatives; whitespace separates constraints that must
    // all hold. A trailing `-` accepts pre-releases of the stated version.
    let mut unknown = false;
    for alternative in req.split("||") {
        let mut all = true;
        let mut part_unknown = false;
        let parts: Vec<_> = alternative.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }
        for part in parts {
            let (op, value) = [">=", "<=", ">", "<", "=", "~", "^"]
                .iter()
                .find_map(|op| part.strip_prefix(op).map(|v| (*op, v)))
                .unwrap_or(("=", part));
            let (value, prerelease) = match value.strip_suffix('-') {
                Some(value) => (value, true),
                None => (value, false),
            };
            match comparison(&version, op, value, prerelease) {
                Some(true) => {}
                Some(false) => {
                    all = false;
                    break;
                }
                None => {
                    part_unknown = true;
                    all = false;
                    break;
                }
            }
        }
        if all {
            return Some(true);
        }
        unknown |= part_unknown;
    }
    if unknown {
        None
    } else {
        Some(false)
    }
}
fn push_edge(report: &mut Report, source: &str, target: &str, relation: &str, requirement: String) {
    if report.edges.iter().any(|edge| {
        edge.source == source
            && edge.target == target
            && edge.relation == relation
            && edge.requirement == requirement
    }) {
        return;
    }
    report.edges.push(Edge {
        source: source.into(),
        target: target.into(),
        relation: relation.into(),
        requirement,
    });
}
/// Dependencies provided by the platform itself. Their exact version is not
/// part of the installed content, so a declared range stays "unknown" rather
/// than becoming a false missing-module issue.
fn platform_target(target: &str) -> bool {
    matches!(
        target,
        "fabricloader" | "minecraft" | "java" | "paper" | "bukkit" | "spigot" | "server"
    )
}
fn api_compatible(api: &str, minecraft: &str) -> Option<bool> {
    let api = semver(api)?;
    let server = semver(minecraft)?;
    Some(server >= api)
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
        // Bukkit resolves plugin names case-insensitively; Fabric IDs are
        // already lowercase. Keep the original key for display and duplicate
        // detection.
        installed
            .entry(module.id.to_lowercase())
            .or_insert_with(|| module.version.clone());
        if module.kind == "plugin" {
            if let Some(api) = &module.api_version {
                match api_compatible(api, minecraft) {
                    Some(true) => {}
                    Some(false) => issue(
                        report,
                        "apiVersion",
                        &module.file,
                        &module.id,
                        format!("{api} / {minecraft}"),
                    ),
                    None => issue(report, "unknown", &module.file, "api-version", api),
                }
            }
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
            ("optional", &module.optional),
            ("incompatible", &module.breaks),
        ] {
            for (target, req) in requirements {
                push_edge(report, &module.id, target, relation, req.to_string());
                let found = installed
                    .get(target)
                    .or_else(|| installed.get(&target.to_lowercase()));
                let matched = found.and_then(|version| predicate(version, req));
                let code = match (relation, found, matched) {
                    ("required", None, _) if platform_target(target) => Some("unknown"),
                    ("required", None, _) => Some("missing"),
                    ("required", Some(_), Some(false)) => Some("version"),
                    ("required", Some(_), None) => Some("unknown"),
                    ("incompatible", Some(_), Some(true)) => Some("conflict"),
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
fn sha1_file(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}
fn local(dir: &Path, software: ServerType, minecraft: &str, java: Option<u32>) -> Result<Report> {
    let Some(folder) = software.content_folder() else {
        return Ok(Report::default());
    };
    let mut report = Report::default();
    let path = storage::child(dir, folder)?;
    if !path.exists() {
        return Ok(report);
    }
    let mut budget = 64 * 1024 * 1024;
    let mut files: Vec<_> = fs::read_dir(path)?.collect::<std::io::Result<_>>()?;
    files.sort_by_key(|e| e.file_name());
    for (n, file) in files
        .iter()
        .filter(|e| {
            e.path().is_file()
                && e.path()
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("jar"))
        })
        .enumerate()
    {
        if n >= 256 {
            issue(&mut report, "unreadable", folder, "", "256 JAR limit");
            break;
        }
        storage::no_links(&file.path())?;
        let name = file.file_name().to_string_lossy().into_owned();
        let first = report.modules.len();
        match jar(
            fs::File::open(file.path())?,
            &name,
            0,
            &mut budget,
            &mut report,
        ) {
            Ok(()) => {
                // Only the top-level JAR can be mapped back to Modrinth; nested
                // entries are archived inside it and have no file of their own.
                let hash = sha1_file(&file.path());
                for module in &mut report.modules[first..] {
                    if module.file == name {
                        module.sha1 = hash.clone();
                    }
                }
            }
            Err(e) => issue(&mut report, "unreadable", &name, "", e.message),
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
        // Managed files are known by filename. Everything else gets one SHA-1
        // lookup, which is the only stable way to identify a manually installed
        // JAR: its name and Fabric ID need not match a Modrinth project.
        let mut hashes: Vec<(String, String)> = Vec::new();
        let mut seen = BTreeSet::new();
        for module in &report.modules {
            if module.file.contains('!') {
                continue;
            }
            if listing
                .entries
                .iter()
                .any(|entry| entry.file == module.file)
            {
                continue;
            }
            if let Some(hash) = &module.sha1 {
                if seen.insert(hash.clone()) {
                    hashes.push((module.file.clone(), hash.clone()));
                }
            }
        }
        let mut complete = true;
        if listing.entries.len() + hashes.len() > 128 {
            hashes.truncate(128usize.saturating_sub(listing.entries.len()));
            complete = false;
            issue(
                &mut report,
                "remoteError",
                "Modrinth",
                "",
                "Only the first 128 JARs are mapped per check.",
            );
        }
        enum Job {
            Managed(crate::modrinth::ContentEntry),
            Local(String, String),
        }
        enum Outcome {
            Managed(String, Result<Vec<crate::modrinth::ProjectDependency>>),
            Local(String, Result<Option<crate::modrinth::ProjectVersion>>),
        }
        let jobs = listing.entries.iter().cloned().map(Job::Managed).chain(
            hashes
                .into_iter()
                .map(|(file, hash)| Job::Local(file, hash)),
        );
        let mut checks = stream::iter(jobs)
            .map(|job| {
                let client = client.clone();
                async move {
                    match job {
                        Job::Managed(entry) => {
                            let result =
                                crate::modrinth::version_dependencies(&client, &entry.version_id)
                                    .await;
                            Outcome::Managed(entry.title.clone(), result)
                        }
                        Job::Local(file, hash) => {
                            let result = crate::modrinth::version_by_hash_opt(&client, &hash).await;
                            Outcome::Local(file, result)
                        }
                    }
                }
            })
            .buffer_unordered(4);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut local_versions = Vec::new();
        let mut managed_deps = Vec::new();
        loop {
            let next = match tokio::time::timeout_at(deadline, checks.next()).await {
                Ok(value) => value,
                Err(_) => {
                    issue(&mut report,"remoteError","Modrinth","","Inspection timed out after 30 seconds; retry to complete the remote graph.");
                    complete = false;
                    break;
                }
            };
            let Some(outcome) = next else {
                break;
            };
            match outcome {
                Outcome::Managed(title, Ok(deps)) => managed_deps.push((title, deps)),
                Outcome::Managed(title, Err(e)) => {
                    complete = false;
                    issue(&mut report, "remoteError", &title, "", e.message);
                }
                Outcome::Local(_, Ok(Some(version))) => local_versions.push(version),
                Outcome::Local(_, Ok(None)) => {}
                Outcome::Local(file, Err(e)) => {
                    complete = false;
                    issue(&mut report, "remoteError", &file, "", e.message);
                }
            }
        }
        // Evaluate only after every lookup finished, so a dependency whose
        // target was resolved late is not reported as missing.
        let mut known: Vec<(String, String, String)> = listing
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.project_id.clone(),
                    entry.version_id.clone(),
                    entry.title.clone(),
                )
            })
            .collect();
        for version in &local_versions {
            known.push((
                version.project_id.clone(),
                version.id.clone(),
                version.version_number.clone(),
            ));
        }
        let remote_issue = |report: &mut Report,
                            source: &str,
                            dep: &crate::modrinth::ProjectDependency| {
            let target = dep
                .project_id
                .as_deref()
                .or(dep.version_id.as_deref())
                .unwrap_or("?");
            push_edge(
                report,
                source,
                target,
                &dep.dependency_type,
                dep.version_id.clone().unwrap_or_else(|| "*".into()),
            );
            let present = known.iter().find(|(project, version, _)| {
                dep.project_id.as_ref() == Some(project) || dep.version_id.as_ref() == Some(version)
            });
            let code = match (dep.dependency_type.as_str(), present) {
                ("required", None) => Some("remoteMissing"),
                ("required", Some((_, version, _)))
                    if dep.version_id.as_ref().is_some_and(|v| v != version) =>
                {
                    Some("version")
                }
                ("incompatible", Some((_, version, _)))
                    if dep.version_id.as_ref().is_none_or(|v| v == version) =>
                {
                    Some("conflict")
                }
                _ => None,
            };
            if let Some(code) = code {
                issue(
                    report,
                    code,
                    source,
                    target,
                    dep.version_id.as_deref().unwrap_or(""),
                );
            }
        };
        for (source, deps) in &managed_deps {
            for dep in deps {
                remote_issue(&mut report, source, dep);
            }
        }
        for version in &local_versions {
            for dep in &version.dependencies {
                remote_issue(&mut report, &version.project_id, dep);
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
            ("1.2.3-beta", ">=1.2", Some(true)),
            ("1.2.4", ">=1.2-", Some(true)),
            ("1.2.0-rc.1", ">=1.2.0", Some(false)),
            ("1.2.3", "~1.2.3", Some(true)),
            ("1.3.0", "~1.2.3", Some(false)),
            ("1.2.9", "~1.2", Some(true)),
            ("1.3.0", "~1.2", Some(false)),
            ("1.20.4", "1.20", Some(true)),
            ("1.20-rc1", ">=1.20- <1.21", Some(true)),
            ("bad", ">=1", None),
        ] {
            assert_eq!(predicate(v, &json!(req)), expected, "{v} {req}");
        }
        assert_eq!(predicate("2.0", &json!(["1.*", ">=2.0"])), Some(true));
        assert_eq!(predicate("1.5", &json!("<=1.0 || >=1.4")), Some(true));
        assert_eq!(predicate("1.2.3+build.5", &json!("=1.2.3")), Some(true));
    }
    #[test]
    fn plugin_yaml_reports_paper_dependencies_and_api_version() {
        let yaml = r#"
name: ExamplePlugin
version: '1.4.0'
main: com.example.Plugin
api-version: '1.20'
depend: [Vault, "ProtocolLib"]
softdepend:
  - PlaceholderAPI
  - LuckPerms
dependencies:
  server:
    Vault:
      load: BEFORE
      required: true
      join-classpath: true
    OptionalLib:
      load: AFTER
      required: false
"#;
        let (depends, optional, api) = {
            let lines = yaml_lines(yaml);
            let mut depends = BTreeMap::new();
            for dependency in yaml_list(&lines, "depend") {
                depends.insert(dependency, Value::String("*".into()));
            }
            let mut optional = BTreeMap::new();
            for dependency in yaml_list(&lines, "softdepend") {
                optional.insert(dependency, Value::String("*".into()));
            }
            for (dependency, required) in paper_dependencies(&lines) {
                if required {
                    depends.insert(dependency, Value::String("*".into()));
                } else {
                    optional.insert(dependency, Value::String("*".into()));
                }
            }
            (depends, optional, top_scalar(&lines, "api-version"))
        };
        assert_eq!(api.as_deref(), Some("1.20"));
        assert!(depends.contains_key("Vault"));
        assert!(depends.contains_key("ProtocolLib"));
        assert!(!depends.contains_key("OptionalLib"));
        assert!(optional.contains_key("PlaceholderAPI"));
        assert!(optional.contains_key("LuckPerms"));
        assert!(optional.contains_key("OptionalLib"));

        let mut report = Report::default();
        report.modules.push(Module {
            id: "ExamplePlugin".into(),
            version: "1.4.0".into(),
            file: "ExamplePlugin.jar".into(),
            kind: "plugin".into(),
            depends,
            optional,
            breaks: BTreeMap::new(),
            provides: vec![],
            client_only: false,
            sha1: None,
            api_version: api,
        });
        report.modules.push(Module {
            id: "Vault".into(),
            version: "1.7".into(),
            file: "Vault.jar".into(),
            kind: "plugin".into(),
            depends: BTreeMap::new(),
            optional: BTreeMap::new(),
            breaks: BTreeMap::new(),
            provides: vec![],
            client_only: false,
            sha1: None,
            api_version: None,
        });
        inspect(&mut report, "1.19.4", None);
        assert!(report.issues.iter().any(|i| i.code == "apiVersion"));
        assert!(report
            .issues
            .iter()
            .any(|i| i.code == "missing" && i.target == "ProtocolLib"));
        assert!(!report.issues.iter().any(|i| i.target == "PlaceholderAPI"));
        assert!(report
            .edges
            .iter()
            .any(|e| e.relation == "optional" && e.target == "LuckPerms"));
    }
    #[test]
    fn plugin_jar_metadata_is_read_from_the_archive() {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("plugin.yml", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"name: Sample\nversion: 2.1\nmain: a.B\nload: POSTWORLD\n")
            .unwrap();
        let mut report = Report::default();
        jar(
            Cursor::new(zip.finish().unwrap().into_inner()),
            "Sample.jar",
            0,
            &mut (64 * 1024 * 1024),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.modules.len(), 1);
        assert_eq!(report.modules[0].id, "Sample");
        assert_eq!(report.modules[0].kind, "plugin");
        assert_eq!(report.modules[0].version, "2.1");
    }
    #[test]
    fn duplicate_missing_and_breaks() {
        let module = |id: &str, depends, breaks| Module {
            id: id.into(),
            version: "1.0".into(),
            file: format!("{id}.jar"),
            kind: "mod".into(),
            depends,
            optional: BTreeMap::new(),
            breaks,
            provides: vec![],
            client_only: false,
            sha1: None,
            api_version: None,
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
