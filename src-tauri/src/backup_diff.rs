use crate::{
    error::{AppError, Result},
    storage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct Index {
    version: u32,
    files: BTreeMap<String, Fingerprint>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    pub kind: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub added: u32,
    pub removed: u32,
    pub modified: u32,
    pub unchanged: u32,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub changes: Vec<Change>,
    pub truncated: bool,
}
fn walk(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, Fingerprint>,
    depth: u16,
) -> Result<()> {
    if depth > 64 || files.len() >= 250_000 {
        return Err(AppError::user(
            "Backup file index exceeds the inspection limit.",
        ));
    }
    storage::no_links(path)?;
    for e in fs::read_dir(path)? {
        let e = e?;
        let path = e.path();
        storage::no_links(&path)?;
        if e.file_type()?.is_dir() {
            walk(root, &path, files, depth + 1)?;
        } else if e.file_type()?.is_file() {
            if files.len() >= 250_000 {
                return Err(AppError::user(
                    "Backup file index exceeds the inspection limit.",
                ));
            }
            let mut f = fs::File::open(&path)?;
            let mut hash = Sha256::new();
            let mut buf = [0u8; 65536];
            let mut bytes = 0;
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hash.update(&buf[..n]);
                bytes += n as u64;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|e| AppError::new("Invalid backup path.", e))?
                .to_str()
                .ok_or_else(|| AppError::user("A backup path is not valid Unicode."))?
                .replace('\\', "/");
            files.insert(
                relative,
                Fingerprint {
                    bytes,
                    sha256: format!("{:x}", hash.finalize()),
                },
            );
        } else {
            return Err(AppError::user(
                "A backup contains an unsupported special file.",
            ));
        }
    }
    Ok(())
}
fn build(dir: &Path, worlds: &[String]) -> Result<Index> {
    let mut files = BTreeMap::new();
    for world in worlds {
        walk(dir, &storage::child(dir, world)?, &mut files, 0)?;
    }
    Ok(Index { version: 1, files })
}
pub fn create_index(dir: &Path, worlds: &[String]) -> Result<()> {
    storage::write_json(&storage::child(dir, "files.json")?, &build(dir, worlds)?)
}
fn load(dir: &Path, worlds: &[String]) -> Result<Index> {
    let path = storage::child(dir, "files.json")?;
    if !path.exists() {
        create_index(dir, worlds)?;
    }
    if fs::metadata(&path)?.len() > 64 * 1024 * 1024 {
        return Err(AppError::user("Backup index is too large."));
    }
    let index: Index = serde_json::from_slice(&fs::read(path)?)?;
    if index.version != 1 || index.files.len() > 250_000 {
        return Err(AppError::user("Unsupported backup file index."));
    }
    Ok(index)
}
fn compare(before: Index, after: Index) -> Diff {
    let mut result = Diff {
        added: 0,
        removed: 0,
        modified: 0,
        unchanged: 0,
        before_bytes: before
            .files
            .values()
            .fold(0u64, |n, f| n.saturating_add(f.bytes)),
        after_bytes: after
            .files
            .values()
            .fold(0u64, |n, f| n.saturating_add(f.bytes)),
        changes: Vec::new(),
        truncated: false,
    };
    let paths: BTreeSet<_> = before.files.keys().chain(after.files.keys()).collect();
    for path in paths {
        let a = before.files.get(path);
        let b = after.files.get(path);
        let kind = match (a, b) {
            (None, Some(_)) => {
                result.added += 1;
                "added"
            }
            (Some(_), None) => {
                result.removed += 1;
                "removed"
            }
            (Some(a), Some(b)) if a != b => {
                result.modified += 1;
                "modified"
            }
            _ => {
                result.unchanged += 1;
                continue;
            }
        };
        if result.changes.len() < 500 {
            result.changes.push(Change {
                path: path.clone(),
                kind: kind.into(),
                before_bytes: a.map(|f| f.bytes).unwrap_or(0),
                after_bytes: b.map(|f| f.bytes).unwrap_or(0),
            });
        } else {
            result.truncated = true;
        }
    }
    result
}
pub fn diff(
    before: &Path,
    before_worlds: &[String],
    after: &Path,
    after_worlds: &[String],
) -> Result<Diff> {
    Ok(compare(
        load(before, before_worlds)?,
        load(after, after_worlds)?,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hashes_find_same_size_changes_and_legacy_indexes() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        for dir in [&a, &b] {
            fs::create_dir_all(dir.join("world/region")).unwrap();
            fs::write(dir.join("world/same"), b"same").unwrap();
        }
        fs::write(a.join("world/region/chunk"), b"abcd").unwrap();
        fs::write(b.join("world/region/chunk"), b"efgh").unwrap();
        fs::write(a.join("world/deleted"), b"gone").unwrap();
        fs::write(b.join("world/new"), b"new").unwrap();
        let worlds = vec!["world".into()];
        let result = diff(&a, &worlds, &b, &worlds).unwrap();
        assert_eq!(
            (
                result.added,
                result.removed,
                result.modified,
                result.unchanged
            ),
            (1, 1, 1, 1)
        );
        assert!(a.join("files.json").is_file());
        assert_eq!(result.before_bytes, 12);
        assert_eq!(result.after_bytes, 11);
        assert!(result
            .changes
            .iter()
            .any(|c| c.path == "world/region/chunk" && c.kind == "modified"));
    }
}
