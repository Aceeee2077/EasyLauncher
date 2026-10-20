//! Bounded player history, derived only from observed server logger events.
use crate::{error::Result, server::manager::Manager, storage};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, sync::Arc, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    pub name: String,
    pub visits: u64,
    pub seconds: u64,
    pub first_seen: String,
    pub last_seen: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub name: String,
    pub joined_at: String,
    pub left_at: String,
    pub seconds: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Day {
    pub date: String,
    pub joins: u64,
    pub seconds: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub players: BTreeMap<String, Player>,
    pub sessions: Vec<Session>,
    pub days: BTreeMap<String, Day>,
    #[serde(default)]
    active: BTreeMap<String, String>,
    #[serde(default)]
    checkpoint_at: Option<String>,
    #[serde(default)]
    pub tracking_since: String,
}
fn elapsed(from: &str, to: DateTime<Utc>) -> u64 {
    DateTime::parse_from_rfc3339(from)
        .map(|start| (to - start.with_timezone(&Utc)).num_seconds().max(0) as u64)
        .unwrap_or(0)
}
impl Store {
    fn tick(&mut self, now: DateTime<Utc>) {
        if self.active.is_empty() {
            self.checkpoint_at = Some(now.to_rfc3339());
            return;
        }
        let from = self
            .checkpoint_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or(now);
        // Attribute observed play time across UTC date boundaries.
        let mut cursor = from.min(now).max(now - chrono::Duration::days(31));
        while cursor < now {
            let midnight = cursor
                .date_naive()
                .succ_opt()
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc();
            let end = midnight.min(now);
            let seconds = (end - cursor).num_seconds().max(0) as u64;
            let date = cursor.format("%Y-%m-%d").to_string();
            let day = self.days.entry(date.clone()).or_insert(Day {
                date,
                joins: 0,
                seconds: 0,
            });
            day.seconds = day
                .seconds
                .saturating_add(seconds.saturating_mul(self.active.len() as u64));
            for name in self.active.keys() {
                if let Some(p) = self.players.get_mut(name) {
                    p.seconds = p.seconds.saturating_add(seconds);
                    p.last_seen = end.to_rfc3339();
                }
            }
            cursor = end;
        }
        self.checkpoint_at = Some(now.to_rfc3339());
        while self.days.len() > 30 {
            self.days.pop_first();
        }
    }
    fn event(&mut self, name: &str, joined: bool, now: DateTime<Utc>) {
        self.tick(now);
        if joined && !self.active.contains_key(name) {
            if self.players.len() >= 10000 && !self.players.contains_key(name) {
                return;
            }
            let stamp = now.to_rfc3339();
            let p = self.players.entry(name.into()).or_insert_with(|| Player {
                name: name.into(),
                visits: 0,
                seconds: 0,
                first_seen: stamp.clone(),
                last_seen: stamp.clone(),
            });
            p.visits = p.visits.saturating_add(1);
            p.last_seen = stamp.clone();
            self.active.insert(name.into(), stamp);
            let date = now.format("%Y-%m-%d").to_string();
            let day = self.days.entry(date.clone()).or_insert(Day {
                date,
                joins: 0,
                seconds: 0,
            });
            day.joins = day.joins.saturating_add(1);
        } else if !joined {
            if let Some(start) = self.active.remove(name) {
                self.sessions.push(Session {
                    name: name.into(),
                    seconds: elapsed(&start, now),
                    joined_at: start,
                    left_at: now.to_rfc3339(),
                });
                if self.sessions.len() > 200 {
                    self.sessions.remove(0);
                }
            }
        }
    }
}
pub fn load(dir: &Path) -> Result<Store> {
    let path = storage::child(dir, "player-stats.json")?;
    let mut store: Store = if path.exists() {
        if fs::metadata(&path)?.len() > 8 * 1024 * 1024 {
            return Err(crate::error::AppError::user(
                "Player history exceeds the storage limit.",
            ));
        }
        serde_json::from_slice(&fs::read(path)?)?
    } else {
        Store {
            tracking_since: Utc::now().to_rfc3339(),
            ..Store::default()
        }
    };
    if store.players.len() > 10000
        || store.active.len() > 10000
        || store.sessions.len() > 200
        || store.days.len() > 31
    {
        return Err(crate::error::AppError::user(
            "Invalid player history limits.",
        ));
    }
    // An interrupted app cannot account for time after its last observation.
    let end = store
        .checkpoint_at
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    if let Some(end) = end {
        let names: Vec<_> = store.active.keys().cloned().collect();
        for name in names {
            store.event(&name, false, end);
        }
    }
    store.active.clear();
    store.checkpoint_at = Some(Utc::now().to_rfc3339());
    Ok(store)
}
fn with_store<T>(m: &Manager, id: &str, f: impl FnOnce(&mut Store) -> T) -> Result<T> {
    let dir = m.dir(id)?;
    m.metadata(id)?;
    let mut stores = m.player_stats.lock().unwrap_or_else(|e| e.into_inner());
    if !stores.contains_key(id) {
        stores.insert(id.into(), load(&dir)?);
    }
    let store = stores.get_mut(id).unwrap();
    let result = f(store);
    storage::write_json(&storage::child(&dir, "player-stats.json")?, store)?;
    Ok(result)
}
pub fn observe(m: &Manager, id: &str, name: &str, joined: bool) -> Result<()> {
    with_store(m, id, |s| s.event(name, joined, Utc::now()))
}
pub fn finish(m: &Manager, id: &str) -> Result<()> {
    with_store(m, id, |s| {
        let now = Utc::now();
        let names: Vec<_> = s.active.keys().cloned().collect();
        for name in names {
            s.event(&name, false, now);
        }
    })
}
pub fn report(m: &Manager, id: &str) -> Result<Store> {
    with_store(m, id, |s| {
        s.tick(Utc::now());
        s.clone()
    })
}
pub fn spawn(m: Arc<Manager>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            if m.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            let manager = m.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let ids: Vec<_> = manager
                    .player_stats
                    .lock()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect();
                for id in ids {
                    if let Err(e) = report(&manager, &id) {
                        manager.logger.log("WARN", &format!("Player history: {e}"));
                    }
                }
            })
            .await;
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
    #[test]
    fn duplicate_events_midnight_and_interrupted_app() {
        let mut store = Store::default();
        store.event("Alex", true, at("2026-10-18T23:59:50Z"));
        store.event("Alex", true, at("2026-10-18T23:59:55Z"));
        store.event("Alex", false, at("2026-10-19T00:00:10Z"));
        assert_eq!(store.players["Alex"].seconds, 20);
        assert_eq!(store.players["Alex"].visits, 1);
        assert_eq!(store.days["2026-10-18"].seconds, 10);
        assert_eq!(store.days["2026-10-19"].seconds, 10);
        store.event("Alex", true, at("2026-10-19T00:01:00Z"));
        store.tick(at("2026-10-19T00:01:30Z"));
        let d = tempfile::tempdir().unwrap();
        storage::write_json(&d.path().join("player-stats.json"), &store).unwrap();
        let reopened = load(d.path()).unwrap();
        assert!(reopened.active.is_empty());
        assert_eq!(reopened.players["Alex"].seconds, 50);
        assert_eq!(reopened.sessions.last().unwrap().seconds, 30);
    }
}
