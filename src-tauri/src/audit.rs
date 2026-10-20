//! Local audit trail for destructive and state-changing operations. Entries
//! never contain secrets; only the action, server identity and a short detail.
use crate::{error::Result, server::manager::Manager, storage};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    sync::Mutex,
};

const FILE: &str = "audit.log";
const MAX_BYTES: u64 = 1024 * 1024;
const MAX_DETAIL: usize = 512;
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub at: String,
    pub action: String,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub detail: String,
}
fn clean(value: &str, limit: usize) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(limit)
        .collect::<String>()
        .trim()
        .to_string()
}
fn rotate(root: &std::path::Path) {
    let current = root.join(FILE);
    let older = fs::metadata(&current)
        .map(|m| m.len() > MAX_BYTES)
        .unwrap_or(false);
    if !older {
        return;
    }
    let _ = fs::remove_file(root.join("audit.2.log"));
    let _ = fs::rename(root.join("audit.1.log"), root.join("audit.2.log"));
    let _ = fs::rename(&current, root.join("audit.1.log"));
}
/// Best effort: an audit failure must never fail the operation the user asked
/// for. The application log records the problem instead.
pub fn record(
    m: &Manager,
    action: &str,
    server_id: Option<&str>,
    server_name: Option<&str>,
    detail: impl ToString,
) {
    let action = clean(action, 64);
    if action.is_empty() {
        return;
    }
    let entry = Entry {
        at: chrono::Utc::now().to_rfc3339(),
        action,
        server_id: server_id.map(String::from),
        server_name: server_name.map(String::from),
        detail: clean(&detail.to_string(), MAX_DETAIL),
    };
    let Ok(json) = serde_json::to_string(&entry) else {
        return;
    };
    let Ok(_guard) = LOCK.lock() else {
        return;
    };
    let current = m.root.join(FILE);
    if storage::no_links(&current).is_err() {
        m.logger
            .log("WARN", "Audit log skipped because the file is a link.");
        return;
    }
    rotate(&m.root);
    match OpenOptions::new().create(true).append(true).open(&current) {
        Ok(mut file) => {
            if let Err(e) = writeln!(file, "{json}") {
                m.logger.log("WARN", &format!("Audit write failed: {e}"));
            }
        }
        Err(e) => m
            .logger
            .log("WARN", &format!("Audit log could not be opened: {e}")),
    }
}
pub fn list(m: &Manager, limit: usize) -> Result<Vec<Entry>> {
    let limit = limit.clamp(1, 500);
    let current = m.root.join(FILE);
    if !current.exists() {
        return Ok(Vec::new());
    }
    storage::no_links(&current)?;
    let meta = fs::metadata(&current)?;
    if meta.len() > 8 * 1024 * 1024 {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&current)?;
    Ok(text
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
        .take(limit)
        .collect())
}
pub fn clear(m: &Manager) -> Result<()> {
    let Ok(_guard) = LOCK.lock() else {
        return Ok(());
    };
    for name in [FILE, "audit.1.log", "audit.2.log"] {
        let path = m.root.join(name);
        storage::no_links(&path)?;
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entries_parse_and_details_are_scrubbed() {
        let entry = Entry {
            at: "2026-01-01T00:00:00Z".into(),
            action: "server.delete".into(),
            server_id: Some("id".into()),
            server_name: Some("Survival".into()),
            detail: "removed 3 backups".into(),
        };
        let line = serde_json::to_string(&entry).unwrap();
        let parsed: Entry = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed.action, "server.delete");
        assert_eq!(parsed.server_name.as_deref(), Some("Survival"));
        assert!(!clean("line\nbreak\u{0}", 40).contains('\n'));
        assert_eq!(clean(&"a".repeat(100), 10).len(), 10);
    }
}
