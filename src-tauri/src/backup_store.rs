//! Chunked, content-addressed backup storage.
//!
//! Worlds are split into fixed-size pieces, each piece is hashed with SHA-256
//! and compressed with Zstandard. Chunks with the same hash are stored once and
//! shared by every backup of the same server, so an unchanged file or a
//! re-visited region costs no additional disk space. Each backup keeps a
//! manifest of file paths, file hashes and chunk references; `files.json`
//! remains a plain file index so backup comparison needs no rewrite.

use crate::{
    backup_diff::Fingerprint,
    error::{AppError, Result},
    models::BackupFormat,
    storage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};

pub const MANIFEST: &str = "manifest.json";
pub const CHUNK_DIR: &str = "chunks";
pub const CHUNK_SIZE: usize = 1024 * 1024;
const MANIFEST_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 250_000;
const MAX_CHUNK_BYTES: usize = 16 * 1024 * 1024;
const MAX_READ_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DEPTH: u16 = 64;
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub version: u32,
    pub chunk_size: u32,
    pub worlds: Vec<String>,
    pub files: BTreeMap<String, ManifestFile>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFile {
    pub bytes: u64,
    pub sha256: String,
    pub chunks: Vec<ChunkRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkRef {
    pub sha256: String,
    pub bytes: u32,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateStats {
    pub logical_bytes: u64,
    pub stored_bytes: u64,
    pub chunk_count: u32,
    pub file_count: u32,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreInfo {
    pub backups: u32,
    pub chunked_backups: u32,
    pub legacy_backups: u32,
    pub logical_bytes: u64,
    /// Estimated disk use: the shared chunk store plus legacy directory copies.
    pub stored_bytes: u64,
    pub chunk_count: u32,
    pub chunk_bytes: u64,
    pub dedupe_saved_bytes: u64,
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}
fn corrupt(details: impl ToString) -> AppError {
    AppError::new(
        "This backup is damaged or incomplete. Restore another backup.",
        details,
    )
    .with_code("backup.corrupt")
}
fn chunks_root(root: &Path) -> Result<PathBuf> {
    storage::child(root, CHUNK_DIR)
}
fn chunk_path(root: &Path, hash: &str) -> Result<PathBuf> {
    if !valid_hash(hash) {
        return Err(corrupt("Invalid chunk hash."));
    }
    let hash = hash.to_ascii_lowercase();
    Ok(chunks_root(root)?
        .join(&hash[..2])
        .join(format!("{hash}.zst")))
}
fn write_chunk(root: &Path, hash: &str, bytes: &[u8]) -> Result<u64> {
    let path = chunk_path(root, hash)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        storage::no_links(parent)?;
    }
    storage::no_links(&path)?;
    if let Ok(meta) = fs::metadata(&path) {
        if meta.is_file() && meta.len() > 0 {
            return Ok(meta.len());
        }
    }
    let compressed = zstd::stream::encode_all(bytes, 3)?;
    storage::atomic_write(&path, &compressed)?;
    Ok(compressed.len() as u64)
}
fn read_chunk(root: &Path, chunk: &ChunkRef) -> Result<Vec<u8>> {
    let path = chunk_path(root, &chunk.sha256)?;
    storage::no_links(&path)?;
    let compressed = fs::read(&path).map_err(|e| corrupt(format!("{}: {e}", path.display())))?;
    let mut decoder = zstd::stream::Decoder::new(compressed.as_slice())
        .map_err(|e| corrupt(format!("{}: {e}", path.display())))?;
    let mut bytes = Vec::with_capacity(chunk.bytes as usize);
    decoder
        .by_ref()
        .take(chunk.bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| corrupt(format!("{}: {e}", path.display())))?;
    if bytes.len() != chunk.bytes as usize || hex(&Sha256::digest(&bytes)) != chunk.sha256 {
        return Err(corrupt(format!(
            "Chunk {} failed its integrity check.",
            chunk.sha256
        )));
    }
    Ok(bytes)
}
/// Rebuild one file from a chunked backup after verifying every chunk and the
/// final file hash. Used by read-only world inspection.
pub fn read_file(root: &Path, backup_dir: &Path, relative: &str) -> Result<Vec<u8>> {
    let manifest = load_manifest(backup_dir)?;
    let file = manifest
        .files
        .get(relative)
        .ok_or_else(|| corrupt("The backup does not contain the requested file."))?;
    if file.bytes > MAX_READ_FILE_BYTES {
        return Err(AppError::user("That backup file is too large to inspect."));
    }
    let mut bytes = Vec::with_capacity(file.bytes as usize);
    for chunk in &file.chunks {
        bytes.extend_from_slice(&read_chunk(root, chunk)?);
    }
    if bytes.len() as u64 != file.bytes || hex(&Sha256::digest(&bytes)) != file.sha256 {
        return Err(corrupt(
            "A reconstructed backup file failed its integrity check.",
        ));
    }
    Ok(bytes)
}

fn collect(
    world_root: &Path,
    path: &Path,
    relative: &str,
    depth: u16,
    files: &mut BTreeMap<String, PathBuf>,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(AppError::user(
            "A world folder is nested too deeply to back up.",
        ));
    }
    storage::no_links(path)?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        storage::no_links(&path)?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| AppError::user("A world file name is not valid Unicode."))?
            .to_string();
        let child = format!("{relative}/{name}");
        if entry.file_type()?.is_dir() {
            collect(world_root, &path, &child, depth + 1, files)?;
        } else if entry.file_type()?.is_file() {
            if files.len() >= MAX_FILES {
                return Err(AppError::user(
                    "The world contains too many files for one backup.",
                ));
            }
            if !path.starts_with(world_root) {
                return Err(AppError::user("A world path left its managed folder."));
            }
            files.insert(child, path);
        } else {
            return Err(AppError::user(
                "A world contains an unsupported special file.",
            ));
        }
    }
    Ok(())
}
fn store_file(
    root: &Path,
    path: &Path,
    relative: &str,
    manifest: &mut Manifest,
    unique: &mut BTreeMap<String, u64>,
) -> Result<()> {
    let mut file = fs::File::open(path)?;
    let mut buffer = vec![0u8; CHUNK_SIZE];
    let mut file_hash = Sha256::new();
    let mut file_bytes = 0u64;
    let mut chunks = Vec::new();
    loop {
        let mut read = 0;
        while read < buffer.len() {
            let n = file.read(&mut buffer[read..])?;
            if n == 0 {
                break;
            }
            read += n;
        }
        if read == 0 {
            break;
        }
        let piece = &buffer[..read];
        let hash = hex(&Sha256::digest(piece));
        let stored = write_chunk(root, &hash, piece)?;
        unique.entry(hash.clone()).or_insert(stored);
        file_hash.update(piece);
        file_bytes += read as u64;
        chunks.push(ChunkRef {
            sha256: hash,
            bytes: read as u32,
        });
        if read < buffer.len() {
            break;
        }
    }
    manifest.files.insert(
        relative.to_string(),
        ManifestFile {
            bytes: file_bytes,
            sha256: hex(&file_hash.finalize()),
            chunks,
        },
    );
    Ok(())
}
/// Store every file of the given world folders as compressed chunks.
pub fn create(
    root: &Path,
    backup_dir: &Path,
    world_root: &Path,
    worlds: &[String],
) -> Result<CreateStats> {
    let mut manifest = Manifest {
        version: MANIFEST_VERSION,
        chunk_size: CHUNK_SIZE as u32,
        worlds: worlds.to_vec(),
        files: BTreeMap::new(),
    };
    let mut unique = BTreeMap::new();
    for world in worlds {
        let dir = storage::child(world_root, world)?;
        if !dir.is_dir() {
            return Err(AppError::user(
                "A world folder disappeared during the backup.",
            ));
        }
        let mut paths = BTreeMap::new();
        collect(&dir, &dir, world, 0, &mut paths)?;
        if manifest.files.len().saturating_add(paths.len()) > MAX_FILES {
            return Err(AppError::user(
                "The worlds contain too many files for one backup.",
            ));
        }
        for (relative, path) in paths {
            store_file(root, &path, &relative, &mut manifest, &mut unique)?;
        }
    }
    let index = manifest
        .files
        .iter()
        .map(|(path, file)| {
            (
                path.clone(),
                Fingerprint {
                    bytes: file.bytes,
                    sha256: file.sha256.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    storage::write_json(&storage::child(backup_dir, MANIFEST)?, &manifest)?;
    crate::backup_diff::write_index(backup_dir, index)?;
    let logical_bytes = manifest.files.values().map(|f| f.bytes).sum();
    Ok(CreateStats {
        logical_bytes,
        stored_bytes: unique.values().sum(),
        chunk_count: unique.len() as u32,
        file_count: manifest.files.len() as u32,
    })
}
fn safe_component(name: &str) -> bool {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 200
        || name.ends_with(['.', ' '])
        || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|', '\0'])
        || name.chars().any(char::is_control)
    {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !RESERVED.contains(&stem.as_str())
}
fn validate_relative(path: &str, worlds: &[String]) -> Result<()> {
    if path.is_empty() || path.len() > 1024 || path.contains('\\') {
        return Err(corrupt("A backup contains an unsafe file path."));
    }
    let mut parts = path.split('/');
    let first = parts.next().unwrap_or("");
    if !worlds.iter().any(|world| world == first) {
        return Err(corrupt("A backup file points outside the recorded worlds."));
    }
    let mut count = 1;
    for part in parts {
        count += 1;
        if count > MAX_DEPTH as usize + 1 || !safe_component(part) {
            return Err(corrupt("A backup contains an unsafe file path."));
        }
    }
    Ok(())
}
fn join_relative(root: &Path, path: &str) -> PathBuf {
    let mut result = root.to_path_buf();
    for part in path.split('/') {
        result.push(part);
    }
    result
}
/// Read and validate the manifest of a chunked backup.
pub fn load_manifest(backup_dir: &Path) -> Result<Manifest> {
    storage::no_links(backup_dir)?;
    let file = storage::child(backup_dir, MANIFEST)?;
    if fs::metadata(&file)?.len() > MAX_MANIFEST_BYTES {
        return Err(corrupt("The backup manifest is too large."));
    }
    let manifest: Manifest = serde_json::from_slice(&fs::read(&file)?)
        .map_err(|e| corrupt(format!("Invalid backup manifest: {e}")))?;
    if manifest.version != MANIFEST_VERSION
        || manifest.chunk_size as usize > MAX_CHUNK_BYTES
        || manifest.chunk_size == 0
        || manifest.worlds.is_empty()
        || manifest.worlds.len() > 8
        || manifest.files.len() > MAX_FILES
    {
        return Err(corrupt("The backup manifest uses an unsupported layout."));
    }
    for world in &manifest.worlds {
        if !safe_component(world) || world.contains('/') {
            return Err(corrupt(
                "The backup manifest contains an unsafe world name.",
            ));
        }
    }
    for (path, file) in &manifest.files {
        validate_relative(path, &manifest.worlds)?;
        if !valid_hash(&file.sha256) || file.chunks.len() > MAX_FILES {
            return Err(corrupt("The backup manifest contains invalid file data."));
        }
        let mut bytes = 0u64;
        for chunk in &file.chunks {
            if !valid_hash(&chunk.sha256)
                || chunk.bytes == 0
                || chunk.bytes as usize > MAX_CHUNK_BYTES
            {
                return Err(corrupt("The backup manifest contains invalid chunk data."));
            }
            bytes = bytes.saturating_add(chunk.bytes as u64);
        }
        if bytes != file.bytes {
            return Err(corrupt(
                "The backup manifest has an inconsistent file size.",
            ));
        }
    }
    Ok(manifest)
}
/// Rebuild the recorded worlds below `destination`. The destination must be an
/// empty staging folder; existing world folders are swapped in only after this
/// function has verified every chunk.
pub fn restore(root: &Path, backup_dir: &Path, destination: &Path) -> Result<u64> {
    let manifest = load_manifest(backup_dir)?;
    fs::create_dir_all(destination)?;
    for world in &manifest.worlds {
        fs::create_dir_all(join_relative(destination, world))?;
    }
    let mut total = 0u64;
    for (path, file) in &manifest.files {
        validate_relative(path, &manifest.worlds)?;
        let target = join_relative(destination, path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
            storage::no_links(parent)?;
        }
        storage::no_links(&target)?;
        let mut out = BufWriter::new(fs::File::create(&target)?);
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        for chunk in &file.chunks {
            let data = read_chunk(root, chunk)?;
            hash.update(&data);
            bytes += data.len() as u64;
            out.write_all(&data)?;
        }
        out.flush()?;
        if bytes != file.bytes || hex(&hash.finalize()) != file.sha256 {
            return Err(corrupt(format!("File {path} failed its integrity check.")));
        }
        total += bytes;
    }
    Ok(total)
}
/// Every chunk hash referenced by a chunked backup.
pub fn referenced_chunks(backup_dir: &Path) -> Result<BTreeSet<String>> {
    let manifest = load_manifest(backup_dir)?;
    Ok(manifest
        .files
        .values()
        .flat_map(|file| file.chunks.iter())
        .map(|chunk| chunk.sha256.to_ascii_lowercase())
        .collect())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRecord {
    #[serde(default)]
    format: BackupFormat,
}
fn referenced_for_root(root: &Path) -> Result<BTreeSet<String>> {
    let mut referenced = BTreeSet::new();
    for entry in fs::read_dir(root)?.flatten() {
        let name = match entry.file_name().to_str() {
            Some(name) => name.to_string(),
            None => continue,
        };
        if storage::validate_id(&name).is_err()
            || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
        {
            continue;
        }
        let dir = entry.path();
        let record: StoredRecord =
            serde_json::from_slice(&fs::read(storage::child(&dir, "backup.json")?)?)
                .map_err(|e| corrupt(format!("Invalid backup metadata in {name}: {e}")))?;
        if record.format == BackupFormat::Chunked {
            referenced.extend(referenced_chunks(&dir)?);
        }
    }
    Ok(referenced)
}
fn walk_chunks(root: &Path, mut visit: impl FnMut(&Path, &str, u64) -> Result<()>) -> Result<()> {
    let chunks = chunks_root(root)?;
    if !chunks.exists() {
        return Ok(());
    }
    storage::no_links(&chunks)?;
    for prefix in fs::read_dir(&chunks)? {
        let prefix = prefix?;
        if !prefix.file_type()?.is_dir() {
            continue;
        }
        storage::no_links(&prefix.path())?;
        for entry in fs::read_dir(prefix.path())? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(hash) = name.strip_suffix(".zst") {
                let bytes = entry.metadata()?.len();
                visit(&path, hash, bytes)?;
            }
        }
    }
    Ok(())
}
pub fn chunk_count(root: &Path) -> Result<(u32, u64)> {
    let mut count = 0u32;
    let mut bytes = 0u64;
    walk_chunks(root, |_, _, size| {
        count = count.saturating_add(1);
        bytes = bytes.saturating_add(size);
        Ok(())
    })?;
    Ok((count, bytes))
}
/// Remove chunks no remaining backup refers to.
pub fn garbage_collect(root: &Path) -> Result<(u32, u64)> {
    let mut referenced = referenced_for_root(root)?;
    let chunks = chunks_root(root)?;
    if !chunks.exists() {
        return Ok((0, 0));
    }
    let mut removed = 0u32;
    let mut bytes = 0u64;
    let mut prefixes = Vec::new();
    for prefix in fs::read_dir(&chunks)? {
        let prefix = prefix?;
        if !prefix.file_type()?.is_dir() {
            continue;
        }
        let prefix_path = prefix.path();
        storage::no_links(&prefix_path)?;
        prefixes.push(prefix_path.clone());
        for entry in fs::read_dir(&prefix_path)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(hash) = name.strip_suffix(".zst") {
                if valid_hash(hash) && !referenced.remove(&hash.to_ascii_lowercase()) {
                    let size = entry.metadata()?.len();
                    fs::remove_file(&path)?;
                    removed = removed.saturating_add(1);
                    bytes = bytes.saturating_add(size);
                }
            }
        }
    }
    for prefix in prefixes {
        if fs::read_dir(&prefix)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false)
        {
            fs::remove_dir(prefix)?;
        }
    }
    Ok((removed, bytes))
}
pub fn store_info(root: &Path) -> Result<StoreInfo> {
    #[derive(Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
    struct Record {
        #[serde(default)]
        size_bytes: u64,
        #[serde(default)]
        format: BackupFormat,
    }
    let mut info = StoreInfo {
        backups: 0,
        chunked_backups: 0,
        legacy_backups: 0,
        logical_bytes: 0,
        stored_bytes: 0,
        chunk_count: 0,
        chunk_bytes: 0,
        dedupe_saved_bytes: 0,
    };
    let (chunks, chunk_bytes) = chunk_count(root)?;
    info.chunk_count = chunks;
    info.chunk_bytes = chunk_bytes;
    info.stored_bytes = chunk_bytes;
    for entry in fs::read_dir(root)?.flatten() {
        let name = match entry.file_name().to_str() {
            Some(name) => name.to_string(),
            None => continue,
        };
        if storage::validate_id(&name).is_err()
            || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
        {
            continue;
        }
        let record: Record = match fs::read(storage::child(&entry.path(), "backup.json")?) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(record) => record,
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        info.backups = info.backups.saturating_add(1);
        info.logical_bytes = info.logical_bytes.saturating_add(record.size_bytes);
        match record.format {
            BackupFormat::Chunked => {
                info.chunked_backups = info.chunked_backups.saturating_add(1);
            }
            BackupFormat::Directory => {
                info.legacy_backups = info.legacy_backups.saturating_add(1);
                info.stored_bytes = info.stored_bytes.saturating_add(record.size_bytes);
            }
        }
    }
    info.dedupe_saved_bytes = info.logical_bytes.saturating_sub(info.stored_bytes);
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("backups");
        let server = temp.path().join("server");
        fs::create_dir_all(root.join(CHUNK_DIR)).unwrap();
        fs::create_dir_all(server.join("world/region")).unwrap();
        (temp, root, server)
    }
    fn backup(root: &Path, server: &Path) -> (PathBuf, CreateStats) {
        let id = uuid::Uuid::new_v4().to_string();
        let dir = root.join(&id);
        fs::create_dir(&dir).unwrap();
        let stats = create(root, &dir, server, &["world".into()]).unwrap();
        storage::write_json(
            &dir.join("backup.json"),
            &crate::models::Backup {
                id,
                created_at: chrono::Utc::now().to_rfc3339(),
                size_bytes: stats.logical_bytes,
                worlds: vec!["world".into()],
                safety: false,
                reason: crate::models::BackupReason::Manual,
                label: None,
                minecraft_version: None,
                server_type: None,
                format: BackupFormat::Chunked,
                stored_bytes: stats.stored_bytes,
                chunk_count: stats.chunk_count,
                file_count: stats.file_count,
            },
        )
        .unwrap();
        (dir, stats)
    }
    fn chunk_files(root: &Path) -> u64 {
        let mut count = 0;
        walk_chunks(root, |_, _, _| {
            count += 1;
            Ok(())
        })
        .unwrap();
        count
    }
    #[test]
    fn chunked_backup_roundtrips_and_shares_identical_chunks() {
        let (temp, root, server) = setup();
        let large = vec![7u8; CHUNK_SIZE + 123];
        fs::write(server.join("world/region/a.mca"), &large).unwrap();
        fs::write(server.join("world/region/b.mca"), &large).unwrap();
        fs::write(server.join("world/level.dat"), b"level").unwrap();
        let (first, stats) = backup(&root, &server);
        assert_eq!(stats.file_count, 3);
        assert_eq!(stats.chunk_count, 3);
        assert_eq!(chunk_files(&root), 3);
        assert!(first.join("manifest.json").is_file());
        assert!(first.join("files.json").is_file());
        let (second, second_stats) = backup(&root, &server);
        assert_eq!(chunk_files(&root), 3);
        assert_eq!(second_stats.chunk_count, 3);
        let restored = temp.path().join("restored");
        restore(&root, &first, &restored).unwrap();
        assert_eq!(
            fs::read(restored.join("world/region/a.mca")).unwrap(),
            large
        );
        assert_eq!(
            fs::read(restored.join("world/level.dat")).unwrap(),
            b"level"
        );
        let (_, removed) = garbage_collect(&root).unwrap();
        assert_eq!(removed, 0);
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(&second).unwrap();
        let (count, _) = chunk_count(&root).unwrap();
        assert_eq!(count, 3);
        garbage_collect(&root).unwrap();
        assert_eq!(chunk_count(&root).unwrap().0, 0);
    }
    #[test]
    fn changed_files_only_add_new_chunks() {
        let (_temp, root, server) = setup();
        fs::write(server.join("world/region/a.mca"), vec![1u8; CHUNK_SIZE]).unwrap();
        backup(&root, &server);
        let before = chunk_files(&root);
        fs::write(server.join("world/region/a.mca"), vec![2u8; CHUNK_SIZE]).unwrap();
        backup(&root, &server);
        assert_eq!(chunk_files(&root), before + 1);
    }
    #[test]
    fn tampered_chunks_are_rejected() {
        let (_temp, root, server) = setup();
        fs::write(server.join("world/level.dat"), b"world").unwrap();
        let (dir, _) = backup(&root, &server);
        let chunk = chunk_path(
            &root,
            &load_manifest(&dir).unwrap().files["world/level.dat"].chunks[0].sha256,
        )
        .unwrap();
        fs::write(&chunk, b"not zstd").unwrap();
        let error = restore(&root, &dir, &root.join("restore")).unwrap_err();
        assert_eq!(error.code, "backup.corrupt");
    }
    #[test]
    fn manifest_paths_cannot_escape_the_stage() {
        let worlds = vec!["world".to_string()];
        assert!(validate_relative("world/region/r.0.0.mca", &worlds).is_ok());
        for path in [
            "../escape",
            "world/../escape",
            "world\\escape",
            "other/file",
            "world/C:/file",
            "world/con",
        ] {
            assert!(validate_relative(path, &worlds).is_err(), "{path}");
        }
    }
}
