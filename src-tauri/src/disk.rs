use crate::{
    error::{AppError, Result},
    server::manager::Manager,
    storage,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Bytes and file counts for one top-level part of the server folder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskCategory {
    pub id: String,
    pub bytes: u64,
    pub files: u64,
}

/// A regenerable group the user can remove, with a short preview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskCleanup {
    pub id: String,
    pub bytes: u64,
    pub files: u64,
    pub sample: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskReport {
    pub server_id: String,
    pub total_bytes: u64,
    pub free_bytes: Option<u64>,
    pub locked: bool,
    pub categories: Vec<DiskCategory>,
    pub cleanups: Vec<DiskCleanup>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskCleanupResult {
    pub freed_bytes: u64,
    pub removed_files: u64,
    pub failures: Vec<String>,
}

const CATEGORY_ORDER: [&str; 10] = [
    "worlds",
    "backups",
    "logs",
    "crashReports",
    "mods",
    "plugins",
    "libraries",
    "versions",
    "cache",
    "other",
];

const CLEANUP_ORDER: [&str; 3] = ["logs", "crashReports", "residue"];

#[cfg(windows)]
fn linked(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0 || meta.file_type().is_symlink()
}

#[cfg(not(windows))]
fn linked(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

fn measure(path: &Path) -> (u64, u64) {
    let mut bytes = 0;
    let mut files = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let meta = match fs::symlink_metadata(entry.path()) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if linked(&meta) {
                continue;
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                bytes += meta.len();
                files += 1;
            }
        }
    }
    (bytes, files)
}

fn free_bytes(path: &Path) -> Option<u64> {
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

fn bucket_for(dir: &Path, name: &str, meta: &fs::Metadata, worlds: &[String]) -> &'static str {
    if !meta.is_dir() {
        return "other";
    }
    match name {
        "backups" => "backups",
        "logs" => "logs",
        "crash-reports" => "crashReports",
        "mods" => "mods",
        "plugins" => "plugins",
        "libraries" => "libraries",
        "versions" => "versions",
        "cache" => "cache",
        _ if worlds.iter().any(|w| w == name) || dir.join(name).join("level.dat").is_file() => {
            "worlds"
        }
        _ => "other",
    }
}

fn world_names(m: &Manager, id: &str) -> Vec<String> {
    let base = m
        .properties(id)
        .ok()
        .and_then(|p| p.values.get("level-name").cloned())
        .unwrap_or_else(|| "world".into());
    vec![
        base.clone(),
        format!("{base}_nether"),
        format!("{base}_the_end"),
    ]
}

fn logs(dir: &Path) -> Vec<PathBuf> {
    let root = match storage::child(dir, "logs") {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "latest.log" {
            continue;
        }
        let meta = match fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_file() && !linked(&meta) {
            out.push(entry.path());
        }
    }
    out
}

fn crash_reports(dir: &Path) -> Vec<PathBuf> {
    let root = match storage::child(dir, "crash-reports") {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let meta = match fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_file() && !linked(&meta) {
            out.push(entry.path());
        }
    }
    out
}

/// Leftovers from interrupted restores or downloads, never live game data.
fn residue(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for folder in [dir.to_path_buf(), dir.join("backups")] {
        let entries = match fs::read_dir(&folder) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let matches = name.starts_with("restore-") || name.ends_with(".part");
            if !matches {
                continue;
            }
            if let Ok(meta) = fs::symlink_metadata(entry.path()) {
                if !linked(&meta) {
                    out.push(entry.path());
                }
            }
        }
    }
    out
}

fn collect(dir: &Path, target: &str) -> Result<Vec<PathBuf>> {
    match target {
        "logs" => Ok(logs(dir)),
        "crashReports" => Ok(crash_reports(dir)),
        "residue" => Ok(residue(dir)),
        _ => Err(AppError::user("Unknown cleanup target.")),
    }
}

fn cleanup(dir: &Path, target: &str) -> DiskCleanup {
    let paths = collect(dir, target).unwrap_or_default();
    let mut bytes = 0;
    let mut files = 0;
    let mut sample = Vec::new();
    for path in &paths {
        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let (b, f) = if meta.is_dir() {
            measure(path)
        } else {
            (meta.len(), 1)
        };
        bytes += b;
        files += f;
        if sample.len() < 10 {
            sample.push(
                path.strip_prefix(dir)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    DiskCleanup {
        id: target.into(),
        bytes,
        files,
        sample,
    }
}

pub fn report(m: &Manager, id: &str) -> Result<DiskReport> {
    let dir = m.dir(id)?;
    let worlds = world_names(m, id);
    let mut buckets: BTreeMap<&'static str, (u64, u64)> = BTreeMap::new();
    let mut total = 0u64;
    for entry in fs::read_dir(&dir)?.flatten() {
        let meta = match fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if linked(&meta) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let (bytes, files) = if meta.is_dir() {
            measure(&entry.path())
        } else if meta.is_file() {
            (meta.len(), 1)
        } else {
            continue;
        };
        total += bytes;
        let bucket = buckets
            .entry(bucket_for(&dir, &name, &meta, &worlds))
            .or_insert((0, 0));
        bucket.0 += bytes;
        bucket.1 += files;
    }
    let categories = CATEGORY_ORDER
        .iter()
        .filter_map(|id| {
            buckets.get(id).map(|(bytes, files)| DiskCategory {
                id: (*id).into(),
                bytes: *bytes,
                files: *files,
            })
        })
        .collect();
    let cleanups = CLEANUP_ORDER
        .iter()
        .map(|target| cleanup(&dir, target))
        .filter(|item| item.bytes > 0)
        .collect();
    let locked = m.runtime(id).map(|r| r.idle().is_err()).unwrap_or(false);
    Ok(DiskReport {
        server_id: id.into(),
        total_bytes: total,
        free_bytes: free_bytes(&dir),
        locked,
        categories,
        cleanups,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub fn clean(
    m: &Manager,
    id: &str,
    targets: Vec<String>,
    confirmed: bool,
) -> Result<DiskCleanupResult> {
    if !confirmed {
        return Err(AppError::user(
            "Confirm the cleanup before files are removed.",
        ));
    }
    let mut selected: Vec<String> = Vec::new();
    for target in targets {
        if !CLEANUP_ORDER.contains(&target.as_str()) {
            return Err(AppError::user("Unknown cleanup target."));
        }
        if !selected.contains(&target) {
            selected.push(target);
        }
    }
    if selected.is_empty() {
        return Err(AppError::user("Select at least one item to clean."));
    }
    m.runtime(id)?.idle()?;
    let dir = m.dir(id)?;
    let mut result = DiskCleanupResult {
        freed_bytes: 0,
        removed_files: 0,
        failures: Vec::new(),
    };
    for target in selected {
        for path in collect(&dir, &target)? {
            let meta = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if linked(&meta) {
                continue;
            }
            let (bytes, files) = if meta.is_dir() {
                measure(&path)
            } else {
                (meta.len(), 1)
            };
            let removed = if meta.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            match removed {
                Ok(()) => {
                    result.freed_bytes += bytes;
                    result.removed_files += files;
                }
                Err(e) => {
                    let name = path
                        .strip_prefix(&dir)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    result.failures.push(format!("{name}: {e}"));
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_regenerable_groups() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("world")).unwrap();
        fs::write(root.join("world").join("level.dat"), b"world").unwrap();
        fs::create_dir_all(root.join("logs")).unwrap();
        fs::write(root.join("logs").join("latest.log"), b"keep").unwrap();
        fs::write(root.join("logs").join("2025-01-01-1.log.gz"), b"old").unwrap();
        fs::create_dir_all(root.join("crash-reports")).unwrap();
        fs::write(root.join("crash-reports").join("crash.txt"), b"boom").unwrap();
        fs::write(root.join("server.jar"), b"jar").unwrap();

        let archives = logs(root);
        assert_eq!(archives.len(), 1);
        assert!(archives[0].ends_with("2025-01-01-1.log.gz"));
        assert_eq!(crash_reports(root).len(), 1);

        let (bytes, files) = measure(&root.join("world"));
        assert_eq!((bytes, files), (5, 1));
        assert_eq!(
            bucket_for(
                root,
                "logs",
                &fs::symlink_metadata(root.join("logs")).unwrap(),
                &[]
            ),
            "logs"
        );
        assert_eq!(
            bucket_for(
                root,
                "server.jar",
                &fs::symlink_metadata(root.join("server.jar")).unwrap(),
                &[]
            ),
            "other"
        );
    }

    #[test]
    fn residue_only_targets_temporary_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("restore-abc"), b"x").unwrap();
        fs::write(dir.path().join("world.part"), b"x").unwrap();
        fs::write(dir.path().join("world"), b"x").unwrap();
        let found = residue(dir.path());
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n.starts_with("restore-") || n.ends_with(".part")
        }));
    }
}
