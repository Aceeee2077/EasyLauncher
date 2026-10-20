use crate::{
    error::{AppError, Result},
    models::{Backup, BackupReason, RestoreOutcome},
    server::manager::Manager,
    storage,
};
use std::{fs, path::Path, sync::Arc};

fn world_names(m: &Manager, id: &str) -> Result<Vec<String>> {
    let p = m.properties(id)?;
    let base = p
        .values
        .get("level-name")
        .cloned()
        .unwrap_or("world".into());
    let dir = m.dir(id)?;
    crate::minecraft::properties::validate_world(&base)?;
    let names = vec![
        base.clone(),
        format!("{base}_nether"),
        format!("{base}_the_end"),
    ];
    for name in &names {
        storage::child(&dir, name)?;
    }
    Ok(names)
}
fn backup_root(m: &Manager, id: &str) -> Result<std::path::PathBuf> {
    m.metadata(id)?;
    let p = storage::child(&m.dir(id)?, "backups")?;
    fs::create_dir_all(&p)?;
    Ok(p)
}
fn create(m: &Manager, id: &str, reason: BackupReason) -> Result<Backup> {
    let dir = m.dir(id)?;
    let meta = m.metadata(id)?;
    let names = world_names(m, id)?;
    let root = backup_root(m, id)?;
    let bid = uuid::Uuid::new_v4().to_string();
    let temp = storage::child(&root, &format!("{bid}.part"))?;
    fs::create_dir(&temp)?;
    let result = (|| {
        let mut worlds = Vec::new();
        let mut size = 0;
        for name in names {
            let p = storage::child(&dir, &name)?;
            if p.is_dir() {
                size += storage::copy_tree(&p, &storage::child(&temp, &name)?)?;
                worlds.push(name);
            }
        }
        if worlds.is_empty() {
            return Err(AppError::user("No world data exists yet. Start the server once, then stop it before creating a backup."));
        }
        let backup = Backup {
            id: bid.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            size_bytes: size,
            worlds,
            safety: reason == BackupReason::Safety,
            reason,
            label: None,
            minecraft_version: Some(meta.minecraft_version),
            server_type: Some(meta.server_type),
        };
        crate::backup_diff::create_index(&temp, &backup.worlds)?;
        storage::write_json(&storage::child(&temp, "backup.json")?, &backup)?;
        fs::rename(&temp, storage::managed(&root, &bid)?)?;
        Ok(backup)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(temp);
    }
    result
}
pub fn list(m: &Manager, id: &str) -> Result<Vec<Backup>> {
    let root = backup_root(m, id)?;
    let mut list = Vec::new();
    for e in fs::read_dir(root)?.flatten() {
        if let Some(id) = e.file_name().to_str() {
            if storage::validate_id(id).is_ok() {
                if let Ok(backup) = read(&e.path()) {
                    list.push(backup);
                }
            }
        }
    }
    list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(list)
}
pub async fn compare(
    m: Arc<Manager>,
    id: String,
    before_id: String,
    after_id: String,
) -> Result<crate::backup_diff::Diff> {
    let r = m.runtime(&id)?;
    let _gate = r.gate.lock().await;
    tokio::task::spawn_blocking(move || {
        let root = backup_root(&m, &id)?;
        let before = storage::managed(&root, &before_id)?;
        let after = storage::managed(&root, &after_id)?;
        let a = read(&before)?;
        let b = read(&after)?;
        crate::backup_diff::diff(&before, &a.worlds, &after, &b.worlds)
    })
    .await
    .map_err(|e| AppError::new("Could not compare backups.", e))?
}
fn read(dir: &Path) -> Result<Backup> {
    storage::no_links(dir)?;
    let mut b: Backup = serde_json::from_slice(&fs::read(storage::child(dir, "backup.json")?)?)?;
    // Older safety snapshots predate the reason field.
    if b.reason == BackupReason::Manual && b.safety {
        b.reason = BackupReason::Safety;
    }
    storage::validate_id(&b.id)?;
    if dir.file_name().and_then(|s| s.to_str()) != Some(&b.id) {
        return Err(AppError::user("Backup metadata does not match its folder."));
    }
    for name in &b.worlds {
        storage::child(dir, name)?;
    }
    Ok(b)
}
pub async fn create_backup(m: Arc<Manager>, id: String, reason: BackupReason) -> Result<Backup> {
    m.ready()?;
    let r = m.runtime(&id)?;
    let _g = r.gate.lock().await;
    r.idle()?;
    let result = tokio::task::spawn_blocking({
        let m = m.clone();
        let id = id.clone();
        move || create(&m, &id, reason)
    })
    .await
    .map_err(|e| AppError::new("Backup task failed.", e))?;
    m.emit(
        "backup-progress",
        serde_json::json!({"id":id,"complete":result.is_ok()}),
    );
    result
}
pub async fn restore_backup(
    m: Arc<Manager>,
    id: String,
    backup_id: String,
    confirmed: bool,
) -> Result<RestoreOutcome> {
    if !confirmed {
        return Err(AppError::user(
            "Confirm restoration before replacing world data.",
        ));
    }
    m.ready()?;
    let r = m.runtime(&id)?;
    let _g = r.gate.lock().await;
    r.idle()?;
    tokio::task::spawn_blocking(move || restore(&m, &id, &backup_id))
        .await
        .map_err(|e| AppError::new("Restore task failed.", e))?
}
fn restore(m: &Manager, id: &str, backup_id: &str) -> Result<RestoreOutcome> {
    let dir = m.dir(id)?;
    let source = storage::managed(&backup_root(m, id)?, backup_id)?;
    let backup = read(&source)?;
    storage::validate_tree(&source)?;
    let current = world_names(m, id)?;
    // Refuse renamed worlds instead of restoring into a folder the server will never load.
    if backup.worlds.is_empty() || backup.worlds.iter().any(|w| !current.contains(w)) {
        return Err(AppError::user("This backup uses a different level-name. Restore the original level-name in server settings first."));
    }
    let safety = if current.iter().any(|w| dir.join(w).is_dir()) {
        Some(create(m, id, BackupReason::Safety)?)
    } else {
        None
    };
    let stage = storage::child(&dir, &format!("restore-{}", uuid::Uuid::new_v4()))?;
    fs::create_dir(&stage)?;
    let result = (|| {
        let new = storage::child(&stage, "new")?;
        let old = storage::child(&stage, "old")?;
        fs::create_dir(&new)?;
        fs::create_dir(&old)?;
        for name in &backup.worlds {
            let from = storage::child(&source, name)?;
            if !from.is_dir() {
                return Err(AppError::user(
                    "The backup is incomplete. No world was replaced.",
                ));
            }
            storage::copy_tree(&from, &storage::child(&new, name)?)?;
        }
        let mut moved = Vec::new();
        let mut installed = Vec::new();
        let replace = (|| {
            for name in &current {
                let original = storage::child(&dir, name)?;
                if original.exists() {
                    storage::validate_tree(&original)?;
                    fs::rename(&original, storage::child(&old, name)?)?;
                    moved.push(name.clone());
                }
            }
            for name in &backup.worlds {
                fs::rename(storage::child(&new, name)?, storage::child(&dir, name)?)?;
                installed.push(name.clone());
            }
            Ok(())
        })();
        if let Err(error) = replace {
            let rollback: Result<()> = (|| {
                for name in installed {
                    let p = storage::child(&dir, &name)?;
                    storage::validate_tree(&p)?;
                    fs::remove_dir_all(p)?;
                }
                for name in moved {
                    fs::rename(storage::child(&old, &name)?, storage::child(&dir, &name)?)?;
                }
                Ok(())
            })();
            if let Err(rollback) = rollback {
                return Err(AppError::new("Restore failed. Recovery files and a safety backup have been preserved. Do not start this server until recovery is complete.",format!("{error}; rollback: {rollback}; recovery: {}",stage.display())));
            }
            return Err(error);
        }
        Ok(())
    })();
    if result.is_ok() {
        fs::remove_dir_all(stage)?;
    } else if !stage.join("old").exists()
        || fs::read_dir(stage.join("old"))
            .map(|mut i| i.next().is_none())
            .unwrap_or(false)
    {
        let _ = fs::remove_dir_all(stage);
    }
    result.map(|_| RestoreOutcome {
        backup_id: backup.id,
        safety_backup_id: safety.map(|b| b.id),
    })
}
pub fn set_label(m: &Manager, id: &str, backup_id: &str, label: Option<String>) -> Result<Backup> {
    let dir = storage::managed(&backup_root(m, id)?, backup_id)?;
    let mut backup = read(&dir)?;
    let label = label
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());
    if let Some(l) = &label {
        if l.chars().count() > 60 || l.contains(['\n', '\r', '\0']) {
            return Err(AppError::user(
                "Keep backup labels under 60 characters on a single line.",
            ));
        }
    }
    backup.label = label;
    storage::write_json(&storage::child(&dir, "backup.json")?, &backup)?;
    Ok(backup)
}
pub async fn delete_backup(
    m: Arc<Manager>,
    id: String,
    backup_id: String,
    confirmed: bool,
) -> Result<()> {
    m.ready()?;
    if !confirmed {
        return Err(AppError::user("Confirm backup deletion first."));
    }
    let r = m.runtime(&id)?;
    let _g = r.gate.lock().await;
    r.idle()?;
    let p = storage::managed(&backup_root(&m, &id)?, &backup_id)?;
    read(&p)?;
    storage::validate_tree(&p)?;
    fs::remove_dir_all(p)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copies_nested_world_and_rejects_wrong_manifest() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("world");
        fs::create_dir_all(source.join("region")).unwrap();
        fs::write(source.join("region/r.0.0.mca"), b"world-data").unwrap();
        let target = d.path().join("copy");
        assert_eq!(storage::copy_tree(&source, &target).unwrap(), 10);
        assert_eq!(
            fs::read(target.join("region/r.0.0.mca")).unwrap(),
            b"world-data"
        );
        let b = Backup {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: "now".into(),
            size_bytes: 10,
            worlds: vec!["../escape".into()],
            safety: false,
            reason: BackupReason::Manual,
            label: None,
            minecraft_version: None,
            server_type: None,
        };
        storage::write_json(&d.path().join("backup.json"), &b).unwrap();
        assert!(read(d.path()).is_err());
    }
    #[test]
    fn legacy_metadata_defaults_without_new_fields() {
        let d = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let dir = d.path().join(&id);
        fs::create_dir_all(dir.join("world")).unwrap();
        fs::write(
            dir.join("backup.json"),
            format!(
                r#"{{"id":"{id}","createdAt":"now","sizeBytes":1,"worlds":["world"],"safety":true}}"#
            ),
        )
        .unwrap();
        let b = read(&dir).unwrap();
        assert_eq!(b.reason, BackupReason::Safety);
        assert!(b.label.is_none());
        assert!(b.minecraft_version.is_none());
        assert!(b.server_type.is_none());
        let manual: Backup =
            serde_json::from_str(r#"{"id":"x","createdAt":"now","sizeBytes":1,"worlds":[]}"#)
                .unwrap();
        assert_eq!(manual.reason, BackupReason::Manual);
    }
}
