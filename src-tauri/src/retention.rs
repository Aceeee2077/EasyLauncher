//! Grandfather-father-son retention for local world backups.
//!
//! A policy keeps the newest backup of each of the most recent N days, weeks
//! and months. Safety snapshots, labelled backups and unreadable dates are
//! never deleted automatically, and the newest backup is always protected.

use crate::{
    backup::{self, backup_root},
    backup_store,
    error::{AppError, Result},
    models::Backup,
    server::manager::Manager,
    storage,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const POLICY: &str = "retention.json";

fn default_daily() -> u32 {
    7
}
fn default_weekly() -> u32 {
    4
}
fn default_monthly() -> u32 {
    3
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_daily")]
    pub daily: u32,
    #[serde(default = "default_weekly")]
    pub weekly: u32,
    #[serde(default = "default_monthly")]
    pub monthly: u32,
    #[serde(default = "default_true")]
    pub keep_labeled: bool,
}
impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            daily: default_daily(),
            weekly: default_weekly(),
            monthly: default_monthly(),
            keep_labeled: true,
        }
    }
}
impl RetentionPolicy {
    fn validate(&self) -> Result<()> {
        if self.daily > 365 || self.weekly > 365 || self.monthly > 365 {
            return Err(AppError::user(
                "Retention windows must be 365 or fewer backups.",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPreview {
    pub total: u32,
    pub keep: u32,
    pub candidates: Vec<Backup>,
    pub reclaimable_bytes: u64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionOutcome {
    pub deleted: u32,
    pub reclaimed_bytes: u64,
}

pub fn load(root: &std::path::Path) -> Result<RetentionPolicy> {
    let path = storage::child(root, POLICY)?;
    if !path.exists() {
        return Ok(RetentionPolicy::default());
    }
    let policy: RetentionPolicy = serde_json::from_slice(&std::fs::read(path)?)?;
    policy.validate()?;
    Ok(policy)
}
pub fn save(root: &std::path::Path, policy: &RetentionPolicy) -> Result<()> {
    policy.validate()?;
    storage::write_json(&storage::child(root, POLICY)?, policy)
}
fn when(backup: &Backup) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&backup.created_at)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}
fn size(backup: &Backup) -> u64 {
    if backup.stored_bytes > 0 {
        backup.stored_bytes
    } else {
        backup.size_bytes
    }
}
fn bucket(
    eligible: &[(&DateTime<Utc>, &Backup)],
    keep: &mut BTreeSet<String>,
    take: u32,
    key: impl Fn(&DateTime<Utc>) -> String,
) {
    if take == 0 {
        return;
    }
    let mut newest: BTreeMap<String, &Backup> = BTreeMap::new();
    for (date, backup) in eligible {
        newest.entry(key(date)).or_insert(backup);
    }
    for (_, backup) in newest.iter().rev().take(take as usize) {
        keep.insert(backup.id.clone());
    }
}
/// Returns the ids to keep and the backups a cleanup would remove. Backups
/// with an unreadable timestamp are conservatively kept.
pub fn select(backups: Vec<Backup>, policy: &RetentionPolicy) -> (BTreeSet<String>, Vec<Backup>) {
    if !policy.enabled {
        return (backups.into_iter().map(|b| b.id).collect(), Vec::new());
    }
    let mut keep = BTreeSet::new();
    let mut dated = Vec::new();
    for backup in &backups {
        match when(backup) {
            Some(date) => dated.push((date, backup)),
            None => {
                keep.insert(backup.id.clone());
            }
        }
    }
    dated.sort_by(|a, b| b.0.cmp(&a.0));
    if let Some((_, latest)) = dated.first() {
        keep.insert(latest.id.clone());
    }
    let mut eligible = Vec::new();
    for (date, backup) in &dated {
        let protected = backup.safety
            || (policy.keep_labeled && backup.label.as_deref().is_some_and(|l| !l.is_empty()));
        if protected {
            keep.insert(backup.id.clone());
        } else {
            eligible.push((date, *backup));
        }
    }
    bucket(&eligible, &mut keep, policy.daily, |d| {
        d.format("%Y-%m-%d").to_string()
    });
    bucket(&eligible, &mut keep, policy.weekly, |d| {
        d.format("%G-W%V").to_string()
    });
    bucket(&eligible, &mut keep, policy.monthly, |d| {
        d.format("%Y-%m").to_string()
    });
    let candidates = backups
        .into_iter()
        .filter(|backup| !keep.contains(&backup.id))
        .collect();
    (keep, candidates)
}
pub fn preview_sync(m: &Manager, id: &str) -> Result<RetentionPreview> {
    let root = backup_root(m, id)?;
    let policy = load(&root)?;
    let backups = backup::list(m, id)?;
    let total = backups.len() as u32;
    let (keep, candidates) = select(backups, &policy);
    Ok(RetentionPreview {
        total,
        keep: keep.len() as u32,
        reclaimable_bytes: candidates.iter().map(size).sum(),
        candidates,
    })
}
fn apply_inner(m: &Manager, id: &str) -> Result<RetentionOutcome> {
    let root = backup_root(m, id)?;
    let policy = load(&root)?;
    let (_, candidates) = select(backup::list(m, id)?, &policy);
    let mut deleted = 0u32;
    let mut reclaimed = 0u64;
    for backup in candidates {
        let legacy = backup.format == crate::models::BackupFormat::Directory;
        let before = if legacy { backup.size_bytes } else { 0 };
        backup::delete_dir(&root, &backup.id)?;
        deleted = deleted.saturating_add(1);
        reclaimed = reclaimed.saturating_add(before);
    }
    let (_, freed) = backup_store::garbage_collect(&root)?;
    reclaimed = reclaimed.saturating_add(freed);
    Ok(RetentionOutcome {
        deleted,
        reclaimed_bytes: reclaimed,
    })
}
/// Called after an automatic backup. A disabled policy does nothing.
pub(crate) fn apply_if_enabled(m: &Manager, id: &str) -> Result<Option<RetentionOutcome>> {
    let root = backup_root(m, id)?;
    if !load(&root)?.enabled {
        return Ok(None);
    }
    apply_inner(m, id).map(Some)
}
pub async fn preview(m: Arc<Manager>, id: String) -> Result<RetentionPreview> {
    let r = m.runtime(&id)?;
    let _gate = r.gate.lock().await;
    tokio::task::spawn_blocking(move || preview_sync(&m, &id))
        .await
        .map_err(|e| AppError::new("Could not inspect backup retention.", e))?
}
pub async fn apply(m: Arc<Manager>, id: String, confirmed: bool) -> Result<RetentionOutcome> {
    if !confirmed {
        return Err(AppError::user(
            "Confirm backup cleanup before deleting old backups.",
        ));
    }
    m.ready()?;
    let r = m.runtime(&id)?;
    let _gate = r.gate.lock().await;
    r.idle()?;
    tokio::task::spawn_blocking({
        let m = m.clone();
        move || {
            let root = backup_root(&m, &id)?;
            if !load(&root)?.enabled {
                return Err(AppError::user(
                    "Enable the retention policy before applying a cleanup.",
                ));
            }
            apply_inner(&m, &id)
        }
    })
    .await
    .map_err(|e| AppError::new("Backup retention task failed.", e))?
}
pub fn policy(m: &Manager, id: &str) -> Result<RetentionPolicy> {
    load(&backup_root(m, id)?)
}
pub fn set_policy(m: &Manager, id: &str, policy: &RetentionPolicy) -> Result<RetentionPolicy> {
    let root = backup_root(m, id)?;
    save(&root, policy)?;
    Ok(policy.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BackupFormat, BackupReason};

    fn backup(id: &str, at: &str, reason: BackupReason, label: Option<&str>) -> Backup {
        Backup {
            id: id.into(),
            created_at: at.into(),
            size_bytes: 10,
            worlds: vec!["world".into()],
            safety: reason == BackupReason::Safety,
            reason,
            label: label.map(str::to_string),
            minecraft_version: None,
            server_type: None,
            format: BackupFormat::Chunked,
            stored_bytes: 4,
            chunk_count: 1,
            file_count: 1,
        }
    }
    fn ids(candidates: &[Backup]) -> Vec<&str> {
        candidates.iter().map(|b| b.id.as_str()).collect()
    }
    #[test]
    fn daily_weekly_and_monthly_buckets_keep_the_newest() {
        let backups = vec![
            backup(
                "today",
                "2026-10-20T12:00:00Z",
                BackupReason::Scheduled,
                None,
            ),
            backup(
                "yesterday",
                "2026-10-19T12:00:00Z",
                BackupReason::Scheduled,
                None,
            ),
            backup(
                "day-before",
                "2026-10-18T12:00:00Z",
                BackupReason::Scheduled,
                None,
            ),
            backup(
                "last-week",
                "2026-10-12T12:00:00Z",
                BackupReason::Scheduled,
                None,
            ),
            backup(
                "august",
                "2026-08-01T12:00:00Z",
                BackupReason::Scheduled,
                None,
            ),
            backup("safety", "2025-01-01T12:00:00Z", BackupReason::Safety, None),
            backup(
                "labelled",
                "2025-01-02T12:00:00Z",
                BackupReason::Manual,
                Some("keep me"),
            ),
            backup("undated", "not a date", BackupReason::Manual, None),
        ];
        let policy = RetentionPolicy {
            enabled: true,
            daily: 2,
            weekly: 2,
            monthly: 2,
            keep_labeled: true,
        };
        let (keep, candidates) = select(backups, &policy);
        assert_eq!(ids(&candidates), vec!["last-week"]);
        for id in [
            "today",
            "safety",
            "labelled",
            "undated",
            "day-before",
            "august",
        ] {
            assert!(keep.contains(id), "{id}");
        }
    }
    #[test]
    fn disabled_or_empty_policies_stay_conservative() {
        let backups = vec![
            backup("new", "2026-10-20T12:00:00Z", BackupReason::Manual, None),
            backup("old", "2026-10-01T12:00:00Z", BackupReason::Manual, None),
        ];
        let disabled = RetentionPolicy::default();
        let (_, candidates) = select(backups.clone(), &disabled);
        assert!(candidates.is_empty());
        let empty = RetentionPolicy {
            enabled: true,
            daily: 0,
            weekly: 0,
            monthly: 0,
            keep_labeled: true,
        };
        let (_, candidates) = select(backups, &empty);
        assert_eq!(ids(&candidates), vec!["old"]);
    }
}
