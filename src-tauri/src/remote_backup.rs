//! Remote backup targets.
//!
//! A backup is packaged into a self-contained ZIP archive: either the chunked
//! manifest, file index and every referenced zstd chunk, or a legacy directory
//! backup. The archive can optionally be encrypted with an age passphrase
//! before it leaves this machine. WebDAV and S3-compatible targets upload the
//! archive directly; rclone is invoked as a local process instead. Passwords,
//! secret keys and passphrases are stored in the DPAPI vault and are never
//! written to the remote target configuration.
//!
//! S3 multipart uploads keep their completed parts on disk so a failed upload
//! resumes instead of starting over. WebDAV has no standard resume mechanism,
//! so an interrupted WebDAV upload restarts from the beginning. rclone uses the
//! resumability of its own backend.

use crate::{
    backup, backup_store,
    error::{AppError, Result},
    models::{Backup, BackupFormat},
    secrets,
    server::manager::Manager,
    storage,
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use hmac::{Hmac, Mac};
use reqwest::{
    header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE},
    Method, RequestBuilder, StatusCode, Url,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufReader, Read, Seek, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt},
    sync::Mutex as AsyncMutex,
};
use tokio_util::io::ReaderStream;

const DIR: &str = "remote-backups";
const CONFIG: &str = "config.json";
const HISTORY: &str = "history.json";
const PENDING: &str = "pending.json";
const STAGING: &str = "staging";
const HISTORY_VERSION: u32 = 1;
const PENDING_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_HISTORY: usize = 100;
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_FILES: usize = 250_000;
const MULTIPART_THRESHOLD: u64 = 64 * 1024 * 1024;
const PART_SIZE: u64 = 64 * 1024 * 1024;
const MAX_PARTS: u32 = 10_000;
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
static UPLOAD_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum RemoteKind {
    #[default]
    Webdav,
    S3,
    Rclone,
}
impl RemoteKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Webdav => "webdav",
            Self::S3 => "s3",
            Self::Rclone => "rclone",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteConfig {
    pub kind: RemoteKind,
    /// WebDAV collection URL, or a custom S3 endpoint.
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    /// Folder below the target. Segments use letters, digits, dot, dash or underscore.
    pub prefix: String,
    /// rclone remote and optional base path, for example `myremote:mc-backups`.
    pub remote: String,
    pub username: String,
    /// S3 access key id. This is an identifier, not a secret; the secret key lives in the vault.
    pub access_key: String,
    pub rclone_path: String,
    pub encrypt: bool,
    /// Download the uploaded object again and compare its SHA-256 digest.
    pub verify_content: bool,
    pub retries: u32,
    /// Path-style S3 requests, required by MinIO and most self-hosted gateways.
    pub path_style: bool,
}
impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            kind: RemoteKind::Webdav,
            endpoint: String::new(),
            bucket: String::new(),
            region: "us-east-1".into(),
            prefix: String::new(),
            remote: String::new(),
            username: String::new(),
            access_key: String::new(),
            rclone_path: String::new(),
            encrypt: true,
            verify_content: true,
            retries: 3,
            path_style: true,
        }
    }
}
impl RemoteConfig {
    fn validate(&self) -> Result<()> {
        if self.retries > 10 {
            return Err(AppError::user("Remote retries must be between 0 and 10."));
        }
        for (value, max, label) in [
            (&self.endpoint, 1000usize, "endpoint"),
            (&self.bucket, 255, "bucket"),
            (&self.region, 64, "region"),
            (&self.username, 256, "username"),
            (&self.access_key, 256, "access key"),
            (&self.rclone_path, 1000, "rclone path"),
            (&self.remote, 500, "rclone remote"),
        ] {
            if value.chars().count() > max || value.chars().any(char::is_control) {
                return Err(AppError::user(format!(
                    "The remote {label} is too long or contains unsupported characters."
                )));
            }
        }
        if !self.prefix.is_empty() {
            for segment in self.prefix.split('/') {
                if segment.is_empty()
                    || segment.len() > 100
                    || segment == "."
                    || segment == ".."
                    || !segment
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                {
                    return Err(AppError::user(
                        "Remote folders use letters, digits, dots, dashes or underscores.",
                    )
                    .with_code("remote.prefix"));
                }
            }
        }
        match self.kind {
            RemoteKind::Webdav => {
                if self.endpoint.trim().is_empty() {
                    return Err(AppError::user("Enter the WebDAV collection URL."));
                }
                parse_base_url(&self.endpoint, "WebDAV URL")?;
            }
            RemoteKind::S3 => {
                if self.bucket.trim().is_empty() {
                    return Err(AppError::user("Enter the S3 bucket name."));
                }
                validate_bucket(&self.bucket)?;
                if self.region.trim().is_empty()
                    || !self
                        .region
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                {
                    return Err(AppError::user(
                        "Enter a lowercase S3 region such as us-east-1.",
                    ));
                }
                if self.access_key.trim().is_empty() {
                    return Err(AppError::user("Enter the S3 access key id."));
                }
                if !self.endpoint.trim().is_empty() {
                    parse_base_url(&self.endpoint, "S3 endpoint")?;
                }
            }
            RemoteKind::Rclone => {
                let remote = self.remote.trim();
                if remote.is_empty() || !remote.contains(':') {
                    return Err(AppError::user(
                        "Enter an rclone destination such as myremote:mc-backups.",
                    )
                    .with_code("remote.rclone"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteSecretsInput {
    pub password: Option<String>,
    pub secret_key: Option<String>,
    pub passphrase: Option<String>,
    /// Delete the saved password, secret key and passphrase before saving new values.
    pub clear: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteArchive {
    pub backup_id: String,
    pub object: String,
    pub bytes: u64,
    pub sha256: String,
    pub encrypted: bool,
    pub uploaded_at: String,
    pub backup_created_at: String,
    pub label: Option<String>,
    pub kind: RemoteKind,
    /// `sha256`, `size`, or `backend` for rclone's own checksum mode.
    pub verification: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePending {
    pub backup_id: String,
    pub bytes: u64,
    pub parts: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    pub configured: bool,
    pub config: RemoteConfig,
    pub secret_set: bool,
    pub passphrase_set: bool,
    pub secrets_available: bool,
    pub rclone_available: bool,
    pub archives: Vec<RemoteArchive>,
    pub pending: Option<RemotePending>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteUploadOutcome {
    pub archive: RemoteArchive,
    pub verification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompletedPart {
    number: u32,
    etag: String,
    bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct S3State {
    upload_id: Option<String>,
    parts: Vec<CompletedPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pending {
    version: u32,
    backup_id: String,
    target: String,
    object: String,
    staging: String,
    bytes: u64,
    sha256: String,
    encrypted: bool,
    #[serde(default)]
    s3: Option<S3State>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryFile {
    version: u32,
    archives: Vec<RemoteArchive>,
}
impl Default for HistoryFile {
    fn default() -> Self {
        Self {
            version: HISTORY_VERSION,
            archives: Vec::new(),
        }
    }
}

struct Prepared {
    pending: Pending,
    path: PathBuf,
    backup: Backup,
}

#[derive(Debug, Clone, Copy)]
struct S3Creds<'a> {
    access: &'a str,
    secret: &'a str,
    region: &'a str,
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}
fn sanitize(message: impl ToString, max: usize) -> String {
    let text = message
        .to_string()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>();
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        trimmed.to_string()
    } else {
        let mut out = trimmed.chars().take(max).collect::<String>();
        out.push('…');
        out
    }
}

fn remote_root(m: &Manager, id: &str) -> Result<PathBuf> {
    m.metadata(id)?;
    let path = storage::child(&m.dir(id)?, DIR)?;
    fs::create_dir_all(&path)?;
    storage::no_links(&path)?;
    Ok(path)
}
fn staging_root(root: &Path) -> Result<PathBuf> {
    let path = storage::child(root, STAGING)?;
    fs::create_dir_all(&path)?;
    storage::no_links(&path)?;
    Ok(path)
}
fn config_path(root: &Path) -> Result<PathBuf> {
    storage::child(root, CONFIG)
}
fn pending_path(root: &Path) -> Result<PathBuf> {
    storage::child(root, PENDING)
}
fn history_path(root: &Path) -> Result<PathBuf> {
    storage::child(root, HISTORY)
}
fn secret_name(id: &str, role: &str) -> String {
    format!("remote.{id}.{role}")
}
fn has_secret(m: &Manager, id: &str, role: &str) -> Result<bool> {
    if !secrets::available() {
        return Ok(false);
    }
    Ok(secrets::load(m, &secret_name(id, role))?.is_some())
}

fn load_config(root: &Path) -> Result<Option<RemoteConfig>> {
    let path = config_path(root)?;
    if !path.exists() {
        return Ok(None);
    }
    storage::no_links(&path)?;
    if fs::metadata(&path)?.len() > MAX_CONFIG_BYTES {
        return Err(
            AppError::user("The remote backup configuration is too large.")
                .with_code("remote.config"),
        );
    }
    let mut config: RemoteConfig = serde_json::from_slice(&fs::read(&path)?)?;
    normalize_config(&mut config)?;
    Ok(Some(config))
}
fn normalize_config(config: &mut RemoteConfig) -> Result<()> {
    config.endpoint = config.endpoint.trim().trim_end_matches('/').to_string();
    config.bucket = config.bucket.trim().to_string();
    config.region = config.region.trim().to_ascii_lowercase();
    if config.region.is_empty() && config.kind == RemoteKind::S3 {
        config.region = "us-east-1".into();
    }
    config.remote = config.remote.trim().trim_end_matches('/').to_string();
    config.username = config.username.trim().to_string();
    config.access_key = config.access_key.trim().to_string();
    config.rclone_path = config.rclone_path.trim().to_string();
    config.prefix = config.prefix.trim().trim_matches('/').to_string();
    config.retries = config.retries.min(10);
    config.validate()
}
fn save_config(root: &Path, config: &RemoteConfig) -> Result<()> {
    config.validate()?;
    storage::write_json(&config_path(root)?, config)
}

fn load_history(root: &Path) -> Result<HistoryFile> {
    let path = history_path(root)?;
    if !path.exists() {
        return Ok(HistoryFile::default());
    }
    storage::no_links(&path)?;
    if fs::metadata(&path)?.len() > 16 * 1024 * 1024 {
        return Err(AppError::user("The remote upload history is too large."));
    }
    let history: HistoryFile = serde_json::from_slice(&fs::read(&path)?)?;
    if history.version != HISTORY_VERSION {
        return Err(AppError::user(
            "The remote upload history was written by a newer version.",
        ));
    }
    Ok(history)
}
fn save_history(root: &Path, history: &HistoryFile) -> Result<()> {
    storage::write_json(&history_path(root)?, history)
}
fn append_history(root: &Path, archive: &RemoteArchive) -> Result<()> {
    let mut history = load_history(root).unwrap_or_default();
    history.archives.retain(|a| a.object != archive.object);
    history.archives.insert(0, archive.clone());
    history.archives.truncate(MAX_HISTORY);
    save_history(root, &history)
}

fn read_pending(root: &Path) -> Result<Option<Pending>> {
    let path = pending_path(root)?;
    if !path.exists() {
        return Ok(None);
    }
    storage::no_links(&path)?;
    if fs::metadata(&path)?.len() > 1024 * 1024 {
        return Ok(None);
    }
    let pending: Pending = match serde_json::from_slice(&fs::read(&path)?) {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };
    if pending.version != PENDING_VERSION
        || pending.backup_id.is_empty()
        || !safe_file_name(&pending.staging)
        || pending.object.is_empty()
        || !pending.object.split('/').all(safe_segment)
    {
        return Ok(None);
    }
    Ok(Some(pending))
}
fn save_pending(root: &Path, pending: &Pending) -> Result<()> {
    storage::write_json(&pending_path(root)?, pending)
}
fn clear_pending(root: &Path) -> Result<()> {
    let path = pending_path(root)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
fn remove_staging_file(root: &Path, name: &str) -> Result<()> {
    if !safe_file_name(name) {
        return Err(AppError::user("The remote staging file name is invalid."));
    }
    let path = storage::child(&staging_root(root)?, name)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 100
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}
fn validate_bucket(bucket: &str) -> Result<()> {
    if bucket.len() < 3
        || bucket.len() > 63
        || !bucket
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        || !bucket
            .bytes()
            .next()
            .map(|b| b.is_ascii_alphanumeric())
            .unwrap_or(false)
        || !bucket
            .bytes()
            .last()
            .map(|b| b.is_ascii_alphanumeric())
            .unwrap_or(false)
        || bucket.contains("..")
    {
        return Err(AppError::user(
            "S3 bucket names use 3-63 lowercase letters, digits, dots or dashes.",
        )
        .with_code("remote.bucket"));
    }
    Ok(())
}
fn parse_base_url(raw: &str, field: &str) -> Result<Url> {
    let url = Url::parse(raw.trim()).map_err(|_| {
        AppError::user(format!("Enter a complete {field}, including https://."))
            .with_code("remote.url")
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::user(format!(
            "The {field} must be an http(s) address without embedded credentials, a query or a fragment."
        ))
        .with_code("remote.url"));
    }
    Ok(url)
}
fn object_key(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}
fn object_name(backup: &Backup, encrypted: bool) -> String {
    let stamp = DateTime::parse_from_rfc3339(&backup.created_at)
        .map(|date| date.format("%Y%m%d-%H%M%S").to_string())
        .unwrap_or_else(|_| "undated".into());
    let short = backup.id.chars().take(8).collect::<String>();
    format!(
        "easylauncher-{stamp}-{short}.zip{}",
        if encrypted { ".age" } else { "" }
    )
}
fn target_signature(config: &RemoteConfig) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        config.kind.label(),
        config.endpoint,
        config.bucket,
        config.prefix,
        config.remote,
        config.encrypt
    )
}

fn hash_file_sync(path: &Path) -> Result<(u64, String)> {
    let mut reader = BufReader::new(fs::File::open(path)?);
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((total, hex(&hash.finalize())))
}

fn zip_error(error: zip::result::ZipError) -> AppError {
    AppError::new("Could not build the backup archive.", error).with_code("remote.archive")
}
fn add_zip_file<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    name: &str,
    path: &Path,
    stored: bool,
    seen: &mut BTreeSet<String>,
    count: &mut usize,
) -> Result<()> {
    if !seen.insert(name.to_string()) {
        return Ok(());
    }
    if *count >= MAX_ARCHIVE_FILES {
        return Err(AppError::user(
            "This backup has too many files to upload as one archive.",
        ));
    }
    *count += 1;
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(if stored {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        })
        .unix_permissions(0o644);
    zip.start_file(name, options).map_err(zip_error)?;
    let mut file = fs::File::open(path)?;
    std::io::copy(&mut file, zip)?;
    Ok(())
}
fn add_zip_tree<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    current: &Path,
    prefix: &str,
    seen: &mut BTreeSet<String>,
    count: &mut usize,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        storage::no_links(&entry.path())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains(['/', '\\', '\0']) {
            return Err(AppError::user(
                "A legacy backup contains a file name that cannot be archived.",
            ));
        }
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if entry.file_type()?.is_dir() {
            add_zip_tree(zip, &entry.path(), &relative, seen, count)?;
        } else if entry.file_type()?.is_file() {
            add_zip_file(
                zip,
                &relative,
                &entry.path(),
                !relative.ends_with(".json"),
                seen,
                count,
            )?;
        } else {
            return Err(AppError::user(
                "A legacy backup contains an unsupported special file.",
            ));
        }
    }
    Ok(())
}
fn build_archive(root: &Path, source: &Path, backup: &Backup, dest: &Path) -> Result<()> {
    storage::no_links(source)?;
    storage::validate_tree(source)?;
    let file = fs::File::create(dest)?;
    let mut zip = zip::ZipWriter::new(file);
    let mut seen = BTreeSet::new();
    let mut count = 0usize;
    add_zip_file(
        &mut zip,
        "backup.json",
        &storage::child(source, "backup.json")?,
        false,
        &mut seen,
        &mut count,
    )?;
    match backup.format {
        BackupFormat::Chunked => {
            add_zip_file(
                &mut zip,
                backup_store::MANIFEST,
                &storage::child(source, backup_store::MANIFEST)?,
                false,
                &mut seen,
                &mut count,
            )?;
            let index = storage::child(source, "files.json")?;
            if index.is_file() {
                add_zip_file(&mut zip, "files.json", &index, false, &mut seen, &mut count)?;
            }
            let chunks = backup_store::referenced_chunks(source)?;
            for hash in chunks {
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(AppError::user("The backup chunk index is damaged."));
                }
                let hash = hash.to_ascii_lowercase();
                let path = root
                    .join(backup_store::CHUNK_DIR)
                    .join(&hash[..2])
                    .join(format!("{hash}.zst"));
                if !path.is_file() {
                    return Err(AppError::user(
                        "A compressed backup chunk is missing. Restore another backup or create a new one.",
                    )
                    .with_code("backup.corrupt"));
                }
                add_zip_file(
                    &mut zip,
                    &format!("chunks/{}/{hash}.zst", &hash[..2]),
                    &path,
                    true,
                    &mut seen,
                    &mut count,
                )?;
            }
        }
        BackupFormat::Directory => {
            add_zip_tree(&mut zip, source, "", &mut seen, &mut count)?;
        }
    }
    let file = zip.finish().map_err(zip_error)?;
    file.sync_all()?;
    Ok(())
}
fn encrypt_archive(input: &Path, output: &Path, passphrase: &str) -> Result<()> {
    let encryptor = age::Encryptor::with_user_passphrase(age::secrecy::SecretString::from(
        passphrase.to_owned(),
    ));
    let out = fs::File::create(output)?;
    let mut writer = encryptor
        .wrap_output(std::io::BufWriter::new(out))
        .map_err(|e| AppError::new("Could not start archive encryption.", e))?;
    let mut source = BufReader::new(fs::File::open(input)?);
    std::io::copy(&mut source, &mut writer)?;
    let mut out = writer
        .finish()
        .map_err(|e| AppError::new("Could not finish archive encryption.", e))?;
    out.flush()?;
    out.get_ref().sync_all()?;
    Ok(())
}

fn prepare(m: &Manager, id: &str, config: &RemoteConfig, backup_id: &str) -> Result<Prepared> {
    let root = backup::backup_root(m, id)?;
    let source = storage::managed(&root, backup_id)?;
    let backup = backup::read(&source)?;
    if backup.size_bytes > MAX_ARCHIVE_BYTES {
        return Err(AppError::user(
            "This backup is too large for one remote archive.",
        ));
    }
    let dir = remote_root(m, id)?;
    let staging = staging_root(&dir)?;
    let name = object_name(&backup, config.encrypt);
    let object = object_key(&config.prefix, &name);
    let target = target_signature(config);
    let staging_name = format!(
        "upload-{}.zip{}",
        backup.id,
        if config.encrypt { ".age" } else { "" }
    );
    let staging_path = storage::child(&staging, &staging_name)?;
    let passphrase = if config.encrypt {
        let value = secrets::load(m, &secret_name(id, "passphrase"))?.ok_or_else(|| {
            AppError::user("Set an archive encryption passphrase before uploading this backup.")
                .with_code("remote.passphrase")
        })?;
        if !(8..=1024).contains(&value.chars().count()) || value.contains('\0') {
            return Err(AppError::user(
                "The archive passphrase must be between 8 and 1024 characters.",
            )
            .with_code("remote.passphrase"));
        }
        Some(value)
    } else {
        None
    };

    if let Some(pending) = read_pending(&dir)? {
        if pending.backup_id == backup.id
            && pending.target == target
            && pending.object == object
            && pending.staging == staging_name
            && pending.encrypted == config.encrypt
            && staging_path.is_file()
        {
            let (bytes, sha256) = hash_file_sync(&staging_path)?;
            if bytes == pending.bytes && sha256 == pending.sha256 {
                return Ok(Prepared {
                    pending,
                    path: staging_path,
                    backup,
                });
            }
        }
    }

    let zip_temp = storage::child(&staging, &format!("{staging_name}.part"))?;
    if zip_temp.exists() {
        fs::remove_file(&zip_temp)?;
    }
    let build = (|| {
        build_archive(&root, &source, &backup, &zip_temp)?;
        let (zip_bytes, zip_sha) = hash_file_sync(&zip_temp)?;
        if zip_bytes > MAX_ARCHIVE_BYTES {
            return Err(AppError::user(
                "The packaged backup archive is too large for a remote upload.",
            ));
        }
        let (bytes, sha256) = if let Some(passphrase) = passphrase.as_deref() {
            let encrypted_temp = storage::child(&staging, &format!("{staging_name}.enc.part"))?;
            if encrypted_temp.exists() {
                fs::remove_file(&encrypted_temp)?;
            }
            encrypt_archive(&zip_temp, &encrypted_temp, passphrase)?;
            fs::remove_file(&zip_temp)?;
            if staging_path.exists() {
                fs::remove_file(&staging_path)?;
            }
            fs::rename(&encrypted_temp, &staging_path)?;
            hash_file_sync(&staging_path)?
        } else {
            if staging_path.exists() {
                fs::remove_file(&staging_path)?;
            }
            fs::rename(&zip_temp, &staging_path)?;
            (zip_bytes, zip_sha)
        };
        let pending = Pending {
            version: PENDING_VERSION,
            backup_id: backup.id.clone(),
            target,
            object,
            staging: staging_name.clone(),
            bytes,
            sha256,
            encrypted: config.encrypt,
            s3: None,
        };
        save_pending(&dir, &pending)?;
        Ok(Prepared {
            pending,
            path: staging_path,
            backup,
        })
    })();
    if build.is_err() {
        let _ = fs::remove_file(&zip_temp);
        let _ = fs::remove_file(storage::child(
            &staging,
            &format!("{staging_name}.enc.part"),
        )?);
    }
    let prepared = build?;
    // Remove archives from abandoned uploads. The current staging file is kept.
    for entry in fs::read_dir(&staging)?.flatten() {
        if entry.file_name().to_string_lossy() != prepared.pending.staging {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(prepared)
}

pub fn info(m: &Manager, id: &str) -> Result<RemoteInfo> {
    let root = remote_root(m, id)?;
    let loaded = load_config(&root)?;
    let configured = loaded.is_some();
    let config = loaded.unwrap_or_default();
    let secret_set = match config.kind {
        RemoteKind::Webdav => has_secret(m, id, "password")?,
        RemoteKind::S3 => has_secret(m, id, "secret")?,
        RemoteKind::Rclone => false,
    };
    let passphrase_set = has_secret(m, id, "passphrase")?;
    let pending = read_pending(&root)?.and_then(|pending| {
        let path = storage::child(&staging_root(&root).ok()?, &pending.staging).ok()?;
        path.is_file().then(|| RemotePending {
            backup_id: pending.backup_id.clone(),
            bytes: pending.bytes,
            parts: pending
                .s3
                .as_ref()
                .map(|s| s.parts.len() as u32)
                .unwrap_or(0),
        })
    });
    Ok(RemoteInfo {
        configured,
        secret_set,
        passphrase_set,
        secrets_available: secrets::available(),
        rclone_available: find_rclone(&config).is_some(),
        config,
        archives: load_history(&root).unwrap_or_default().archives,
        pending,
    })
}

pub fn save(
    m: &Manager,
    id: &str,
    mut config: RemoteConfig,
    credentials: RemoteSecretsInput,
) -> Result<RemoteInfo> {
    normalize_config(&mut config)?;
    let root = remote_root(m, id)?;
    for value in [
        credentials.password.as_deref(),
        credentials.secret_key.as_deref(),
        credentials.passphrase.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if value.chars().count() > 4096 || value.contains('\0') {
            return Err(AppError::user("A saved remote credential is too long."));
        }
    }
    if let Some(passphrase) = credentials.passphrase.as_deref() {
        if !passphrase.is_empty() && !(8..=1024).contains(&passphrase.chars().count()) {
            return Err(AppError::user(
                "The archive passphrase must be between 8 and 1024 characters.",
            )
            .with_code("remote.passphrase"));
        }
    }
    save_config(&root, &config)?;
    if credentials.clear {
        for role in ["password", "secret", "passphrase"] {
            secrets::delete(m, &secret_name(id, role))?;
        }
    }
    for (role, value) in [
        ("password", credentials.password.as_deref()),
        ("secret", credentials.secret_key.as_deref()),
        ("passphrase", credentials.passphrase.as_deref()),
    ] {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            secrets::save(m, &secret_name(id, role), value)?;
        }
    }
    info(m, id)
}

fn upload_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("EasyLauncher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(TRANSFER_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::new("Could not prepare the remote upload.", e))
}
fn network_error(error: reqwest::Error) -> AppError {
    AppError::new(
        "Could not reach the remote backup target. Check the address, network and credentials.",
        error,
    )
    .with_code("remote.network")
}
async fn response_error(context: &str, response: reqwest::Response) -> AppError {
    let status = response.status();
    let body = response.bytes().await.unwrap_or_default();
    let text = sanitize(String::from_utf8_lossy(&body[..body.len().min(4096)]), 600);
    let mut error = AppError::new(
        format!("{context} failed with HTTP {}.", status.as_u16()),
        text,
    );
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        error = error.with_code("remote.auth");
    } else if status == StatusCode::NOT_FOUND {
        error = error.with_code("remote.notFound");
    } else {
        error = error.with_code("remote.http");
    }
    error
}
fn retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(500 * (1u64 << attempt.min(5)))
}
fn method(name: &str) -> Result<Method> {
    Method::from_bytes(name.as_bytes())
        .map_err(|e| AppError::new("The remote HTTP method is invalid.", e))
}

fn join_key(base: &Url, key: &str) -> Result<Url> {
    let mut url = base.clone();
    let trim_trailing = url.path().len() > 1;
    {
        let mut segments = url.path_segments_mut().map_err(|_| {
            AppError::user("The remote URL cannot contain a folder path.").with_code("remote.url")
        })?;
        if trim_trailing {
            segments.pop_if_empty();
        }
        for segment in key.split('/') {
            if segment.is_empty() {
                return Err(AppError::user("The remote object path is invalid."));
            }
            segments.push(segment);
        }
    }
    Ok(url)
}
fn dir_url(base: &Url, key: &str) -> Result<Url> {
    let mut url = if key.is_empty() {
        base.clone()
    } else {
        join_key(base, key)?
    };
    {
        let mut segments = url.path_segments_mut().map_err(|_| {
            AppError::user("The remote URL cannot contain a folder path.").with_code("remote.url")
        })?;
        segments.push("");
    }
    Ok(url)
}
fn dav_request(
    client: &reqwest::Client,
    method: Method,
    url: Url,
    config: &RemoteConfig,
    password: Option<&str>,
) -> RequestBuilder {
    let request = client.request(method, url);
    if config.username.is_empty() && password.is_none() {
        request
    } else {
        request.basic_auth(&config.username, password)
    }
}
async fn dav_ensure_dirs(
    client: &reqwest::Client,
    config: &RemoteConfig,
    password: Option<&str>,
) -> Result<()> {
    let base = parse_base_url(&config.endpoint, "WebDAV URL")?;
    let mut partial = String::new();
    for segment in config.prefix.split('/').filter(|s| !s.is_empty()) {
        if !partial.is_empty() {
            partial.push('/');
        }
        partial.push_str(segment);
        let url = dir_url(&base, &partial)?;
        let response = dav_request(client, method("MKCOL")?, url, config, password)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if !status.is_success() && status != StatusCode::METHOD_NOT_ALLOWED {
            return Err(response_error("Creating the WebDAV folder", response).await);
        }
    }
    Ok(())
}
fn xml_content_length(xml: &str) -> Option<u64> {
    let lower = xml.to_ascii_lowercase();
    let position = lower.find("getcontentlength")?;
    let open = lower[position..].find('>')? + position + 1;
    let rest = &xml[open..];
    let end = rest.find('<')?;
    rest[..end].trim().parse().ok()
}
async fn dav_size(
    client: &reqwest::Client,
    config: &RemoteConfig,
    password: Option<&str>,
    object: &str,
) -> Result<u64> {
    let base = parse_base_url(&config.endpoint, "WebDAV URL")?;
    let url = join_key(&base, object)?;
    let response = dav_request(client, Method::HEAD, url.clone(), config, password)
        .send()
        .await
        .map_err(network_error)?;
    let status = response.status();
    if status.is_success() {
        return response.content_length().ok_or_else(|| {
            AppError::user("The WebDAV server did not report the uploaded file size.")
                .with_code("remote.verify")
        });
    }
    if matches!(
        status,
        StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED
    ) {
        let response = dav_request(client, method("PROPFIND")?, url, config, password)
            .header("Depth", "0")
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(
                r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:getcontentlength/></d:prop></d:propfind>"#,
            )
            .send()
            .await
            .map_err(network_error)?;
        if !response.status().is_success() {
            return Err(response_error("Checking the WebDAV file", response).await);
        }
        let body = response.text().await.map_err(network_error)?;
        return xml_content_length(&body).ok_or_else(|| {
            AppError::user("The WebDAV server did not report the uploaded file size.")
                .with_code("remote.verify")
        });
    }
    Err(response_error("Checking the WebDAV file", response).await)
}
async fn hash_response(response: reqwest::Response) -> Result<(u64, String)> {
    let mut stream = response.bytes_stream();
    let mut hash = Sha256::new();
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(network_error)?;
        hash.update(&chunk);
        total += chunk.len() as u64;
    }
    Ok((total, hex(&hash.finalize())))
}

async fn webdav_upload(
    client: &reqwest::Client,
    config: &RemoteConfig,
    pending: &Pending,
    path: &Path,
    password: Option<&str>,
) -> Result<&'static str> {
    let base = parse_base_url(&config.endpoint, "WebDAV URL")?;
    let final_url = join_key(&base, &pending.object)?;
    let part_url = join_key(&base, &format!("{}.part", pending.object))?;
    dav_ensure_dirs(client, config, password).await?;
    let attempts = config.retries + 1;
    let mut last = None;
    for attempt in 0..attempts {
        let result = async {
            let file = tokio::fs::File::open(path)
                .await
                .map_err(|e| AppError::new("Could not read the upload archive.", e))?;
            let length = file
                .metadata()
                .await
                .map_err(|e| AppError::new("Could not read the upload archive.", e))?
                .len();
            if length != pending.bytes {
                return Err(AppError::user(
                    "The upload archive changed while it was being prepared.",
                )
                .with_code("remote.staging"));
            }
            let body = reqwest::Body::wrap_stream(ReaderStream::new(file));
            let response = dav_request(client, Method::PUT, part_url.clone(), config, password)
                .header(CONTENT_LENGTH, length)
                .body(body)
                .send()
                .await
                .map_err(network_error)?;
            if !response.status().is_success() {
                return Err(response_error("Uploading to WebDAV", response).await);
            }
            let response = dav_request(client, method("MOVE")?, part_url.clone(), config, password)
                .header("Destination", final_url.as_str())
                .header("Overwrite", "T")
                .send()
                .await
                .map_err(network_error)?;
            if !response.status().is_success() {
                return Err(response_error("Finalizing the WebDAV upload", response).await);
            }
            Ok::<(), AppError>(())
        }
        .await;
        match result {
            Ok(()) => {
                last = None;
                break;
            }
            Err(error) => {
                last = Some(error);
                if attempt + 1 < attempts {
                    tokio::time::sleep(retry_delay(attempt)).await;
                }
            }
        }
    }
    if let Some(error) = last {
        return Err(error);
    }
    let size = dav_size(client, config, password, &pending.object).await?;
    if size != pending.bytes {
        return Err(AppError::new(
            "The uploaded WebDAV file has the wrong size.",
            format!("expected {} bytes, found {size}", pending.bytes),
        )
        .with_code("remote.verify"));
    }
    if config.verify_content {
        let url = join_key(&base, &pending.object)?;
        let response = dav_request(client, Method::GET, url, config, password)
            .send()
            .await
            .map_err(network_error)?;
        if !response.status().is_success() {
            return Err(
                response_error("Downloading the WebDAV file for verification", response).await,
            );
        }
        let (bytes, sha256) = hash_response(response).await?;
        if bytes != pending.bytes || sha256 != pending.sha256 {
            return Err(AppError::user(
                "The uploaded WebDAV file did not match its local checksum. The remote copy may be corrupt.",
            )
            .with_code("remote.verify"));
        }
        return Ok("sha256");
    }
    Ok("size")
}

const AWS_SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
fn aws_encode(value: &str) -> String {
    percent_encoding::percent_encode(value.as_bytes(), AWS_SET).to_string()
}
fn with_query(mut url: Url, query: &[(&str, &str)]) -> Url {
    if query.is_empty() {
        url.set_query(None);
    } else {
        let encoded = query
            .iter()
            .map(|(key, value)| format!("{}={}", aws_encode(key), aws_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        url.set_query(Some(&encoded));
    }
    url
}
fn canonical_query(url: &Url) -> String {
    let mut pairs = url
        .query_pairs()
        .map(|(key, value)| (aws_encode(&key), aws_encode(&value)))
        .collect::<Vec<_>>();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}
fn host_header(url: &Url) -> String {
    match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_string(),
    }
}
fn amz_date(time: DateTime<Utc>) -> String {
    time.format("%Y%m%dT%H%M%SZ").to_string()
}
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}
fn canonical_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn sigv4_authorization(
    http_method: &str,
    url: &Url,
    payload_hash: &str,
    extra_headers: &[(&str, &str)],
    time: DateTime<Utc>,
    access_key: &str,
    secret_key: &str,
    region: &str,
) -> Result<String> {
    let mut headers = BTreeMap::new();
    headers.insert("host".to_string(), host_header(url));
    headers.insert("x-amz-content-sha256".to_string(), payload_hash.to_string());
    headers.insert("x-amz-date".to_string(), amz_date(time));
    for (name, value) in extra_headers {
        headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
    }
    let canonical_headers = headers
        .iter()
        .map(|(name, value)| format!("{name}:{}\n", canonical_value(value)))
        .collect::<String>();
    let signed_headers = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let canonical_request = format!(
        "{http_method}\n{path}\n{}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        canonical_query(url)
    );
    let scope = format!("{}/{region}/s3/aws4_request", time.format("%Y%m%d"));
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        amz_date(time),
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let date_key = hmac_sha256(
        format!("AWS4{secret_key}").as_bytes(),
        time.format("%Y%m%d").to_string().as_bytes(),
    );
    let region_key = hmac_sha256(&date_key, region.as_bytes());
    let service_key = hmac_sha256(&region_key, b"s3");
    let signing_key = hmac_sha256(&service_key, b"aws4_request");
    let signature = hex(&hmac_sha256(&signing_key, string_to_sign.as_bytes()));
    Ok(format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    ))
}
fn s3_request(
    client: &reqwest::Client,
    http_method: Method,
    url: Url,
    payload_hash: &str,
    extra_headers: &[(&str, &str)],
    credentials: S3Creds<'_>,
) -> Result<RequestBuilder> {
    let time = Utc::now();
    let authorization = sigv4_authorization(
        http_method.as_str(),
        &url,
        payload_hash,
        extra_headers,
        time,
        credentials.access,
        credentials.secret,
        credentials.region,
    )?;
    let mut request = client
        .request(http_method, url)
        .header("x-amz-date", amz_date(time))
        .header("x-amz-content-sha256", payload_hash)
        .header(AUTHORIZATION, authorization);
    for (name, value) in extra_headers {
        request = request.header(*name, *value);
    }
    Ok(request)
}

fn s3_url(config: &RemoteConfig, key: &str, query: &[(&str, &str)]) -> Result<Url> {
    let mut url = if !config.endpoint.is_empty() {
        let endpoint = parse_base_url(&config.endpoint, "S3 endpoint")?;
        if config.path_style {
            join_key(&endpoint, &format!("{}/{key}", config.bucket))?
        } else {
            let host = endpoint.host_str().ok_or_else(|| {
                AppError::user("The S3 endpoint is missing a host name.").with_code("remote.url")
            })?;
            let host = format!("{}.{host}", config.bucket);
            let origin = match endpoint.port() {
                Some(port) => format!("{}://{host}:{port}/", endpoint.scheme()),
                None => format!("{}://{host}/", endpoint.scheme()),
            };
            let url =
                Url::parse(&origin).map_err(|e| AppError::new("The S3 endpoint is invalid.", e))?;
            let base_path = endpoint.path().trim_matches('/');
            let key = if base_path.is_empty() {
                key.to_string()
            } else {
                format!("{base_path}/{key}")
            };
            join_key(&url, &key)?
        }
    } else {
        let url = Url::parse(&format!(
            "https://{}.s3.{}.amazonaws.com/",
            config.bucket, config.region
        ))
        .map_err(|e| AppError::new("The S3 bucket URL is invalid.", e))?;
        join_key(&url, key)?
    };
    url = with_query(url, query);
    Ok(url)
}

fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let lower = xml.to_ascii_lowercase();
    let open = format!("<{}", tag.to_ascii_lowercase());
    let start = lower.find(&open)?;
    let after = lower[start..].find('>')? + start + 1;
    let rest = &xml[after..];
    let end = rest.find('<')?;
    Some(xml_unescape(rest[..end].trim()))
}
fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn s3_error_message(body: &str) -> String {
    let code = xml_tag(body, "Code").unwrap_or_default();
    let message = xml_tag(body, "Message").unwrap_or_default();
    sanitize(
        if message.is_empty() {
            body.to_string()
        } else if code.is_empty() {
            message
        } else {
            format!("{code}: {message}")
        },
        600,
    )
}
async fn s3_response_error(context: &str, response: reqwest::Response) -> AppError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let mut error = AppError::new(
        format!("{context} failed with HTTP {}.", status.as_u16()),
        s3_error_message(&body),
    );
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        error = error.with_code("remote.auth");
    } else if status == StatusCode::NOT_FOUND {
        error = error.with_code("remote.notFound");
    } else {
        error = error.with_code("remote.http");
    }
    error
}
fn parse_parts(xml: &str) -> Vec<CompletedPart> {
    let lower = xml.to_ascii_lowercase();
    let mut result = Vec::new();
    let mut offset = 0usize;
    while let Some(found) = lower[offset..].find("<part>") {
        let start = offset + found;
        let end = lower[start..]
            .find("</part>")
            .map(|value| start + value)
            .unwrap_or(xml.len());
        let block = &xml[start..end];
        if let (Some(number), Some(etag)) = (xml_tag(block, "PartNumber"), xml_tag(block, "ETag")) {
            if let Ok(number) = number.parse::<u32>() {
                if number > 0 && !etag.is_empty() {
                    result.push(CompletedPart {
                        number,
                        etag,
                        bytes: 0,
                    });
                }
            }
        }
        offset = end.saturating_add(7);
        if offset >= xml.len() {
            break;
        }
    }
    result
}

async fn s3_verify(
    client: &reqwest::Client,
    config: &RemoteConfig,
    pending: &Pending,
    credentials: S3Creds<'_>,
) -> Result<&'static str> {
    let url = s3_url(config, &pending.object, &[])?;
    let mut size = None;
    let mut last = None;
    for attempt in 0..config.retries.max(2) + 1 {
        let response = s3_request(
            client,
            Method::HEAD,
            url.clone(),
            EMPTY_SHA256,
            &[],
            credentials,
        )?
        .send()
        .await;
        match response {
            Ok(response) if response.status().is_success() => {
                if let Some(length) = response.content_length() {
                    size = Some(length);
                    last = None;
                    break;
                }
                last = Some(
                    AppError::user("The S3 target did not report the uploaded file size.")
                        .with_code("remote.verify"),
                );
            }
            Ok(response) => {
                last = Some(s3_response_error("Checking the S3 object", response).await);
            }
            Err(error) => last = Some(network_error(error)),
        }
        if attempt < config.retries.max(2) {
            tokio::time::sleep(retry_delay(attempt)).await;
        }
    }
    let size = match (size, last) {
        (Some(size), _) => size,
        (None, Some(error)) => return Err(error),
        (None, None) => {
            return Err(
                AppError::user("The S3 object could not be verified.").with_code("remote.verify")
            )
        }
    };
    if size != pending.bytes {
        return Err(AppError::new(
            "The uploaded S3 object has the wrong size.",
            format!("expected {} bytes, found {size}", pending.bytes),
        )
        .with_code("remote.verify"));
    }
    if config.verify_content {
        let response = s3_request(client, Method::GET, url, EMPTY_SHA256, &[], credentials)?
            .send()
            .await
            .map_err(network_error)?;
        if !response.status().is_success() {
            return Err(
                s3_response_error("Downloading the S3 object for verification", response).await,
            );
        }
        let (bytes, sha256) = hash_response(response).await?;
        if bytes != pending.bytes || sha256 != pending.sha256 {
            return Err(AppError::user(
                "The uploaded S3 object did not match its local checksum. The remote copy may be corrupt.",
            )
            .with_code("remote.verify"));
        }
        return Ok("sha256");
    }
    Ok("size")
}

async fn s3_upload_part(
    client: &reqwest::Client,
    config: &RemoteConfig,
    pending: &Pending,
    path: &Path,
    number: u32,
    offset: u64,
    length: u64,
    upload_id: &str,
    credentials: S3Creds<'_>,
) -> Result<String> {
    let number_text = number.to_string();
    let url = s3_url(
        config,
        &pending.object,
        &[("partNumber", &number_text), ("uploadId", upload_id)],
    )?;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| AppError::new("Could not read the upload archive.", e))?;
    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|e| AppError::new("Could not seek in the upload archive.", e))?;
    let body = reqwest::Body::wrap_stream(ReaderStream::new(file.take(length)));
    let response = s3_request(client, Method::PUT, url, UNSIGNED_PAYLOAD, &[], credentials)?
        .header(CONTENT_LENGTH, length)
        .body(body)
        .send()
        .await
        .map_err(network_error)?;
    if !response.status().is_success() {
        return Err(s3_response_error("Uploading an S3 part", response).await);
    }
    let etag = response
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::new(
                "The S3 target did not return a part checksum.",
                "An ETag header is required.",
            )
            .with_code("remote.verify")
        })?;
    Ok(etag)
}

async fn s3_list_parts(
    client: &reqwest::Client,
    config: &RemoteConfig,
    pending: &Pending,
    upload_id: &str,
    credentials: S3Creds<'_>,
) -> Result<Option<Vec<CompletedPart>>> {
    let mut parts = Vec::new();
    let mut marker: Option<String> = None;
    for _ in 0..100 {
        let marker_text = marker.clone();
        let mut query: Vec<(&str, &str)> = vec![("uploadId", upload_id), ("max-parts", "1000")];
        if let Some(ref marker) = marker_text {
            query.push(("part-number-marker", marker));
        }
        let url = s3_url(config, &pending.object, &query)?;
        let response = s3_request(client, Method::GET, url, EMPTY_SHA256, &[], credentials)?
            .send()
            .await
            .map_err(network_error)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(s3_response_error("Listing S3 upload parts", response).await);
        }
        let xml = response.text().await.map_err(network_error)?;
        parts.extend(parse_parts(&xml));
        let truncated = xml_tag(&xml, "IsTruncated")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !truncated {
            return Ok(Some(parts));
        }
        marker = xml_tag(&xml, "NextPartNumberMarker").filter(|value| !value.is_empty());
        if marker.is_none() {
            return Ok(Some(parts));
        }
    }
    Err(AppError::user("The S3 upload has too many parts to list.").with_code("remote.s3"))
}

async fn s3_initiate(
    client: &reqwest::Client,
    config: &RemoteConfig,
    pending: &Pending,
    credentials: S3Creds<'_>,
) -> Result<String> {
    let url = s3_url(config, &pending.object, &[("uploads", "")])?;
    let response = s3_request(client, Method::POST, url, EMPTY_SHA256, &[], credentials)?
        .header(CONTENT_LENGTH, 0)
        .send()
        .await
        .map_err(network_error)?;
    if !response.status().is_success() {
        return Err(s3_response_error("Starting the S3 multipart upload", response).await);
    }
    let xml = response.text().await.map_err(network_error)?;
    xml_tag(&xml, "UploadId").ok_or_else(|| {
        AppError::new(
            "The S3 target did not return an upload identifier.",
            sanitize(xml, 400),
        )
        .with_code("remote.s3")
    })
}

async fn s3_complete(
    client: &reqwest::Client,
    config: &RemoteConfig,
    pending: &Pending,
    upload_id: &str,
    parts: &[CompletedPart],
    credentials: S3Creds<'_>,
) -> Result<()> {
    let mut xml =
        String::from("<CompleteMultipartUpload xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">");
    for part in parts {
        xml.push_str(&format!(
            "<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>",
            part.number,
            xml_escape(&part.etag)
        ));
    }
    xml.push_str("</CompleteMultipartUpload>");
    let payload_hash = hex(&Sha256::digest(xml.as_bytes()));
    let url = s3_url(config, &pending.object, &[("uploadId", upload_id)])?;
    let mut last = None;
    for attempt in 0..config.retries + 1 {
        let response = s3_request(
            client,
            Method::POST,
            url.clone(),
            &payload_hash,
            &[],
            credentials,
        )?
        .header(CONTENT_TYPE, "application/xml")
        .body(xml.clone())
        .send()
        .await;
        match response {
            Ok(response) if response.status().is_success() => {
                let body = response.text().await.unwrap_or_default();
                if xml_tag(&body, "Code").is_some() {
                    last = Some(
                        AppError::new(
                            "The S3 target rejected the completed upload.",
                            s3_error_message(&body),
                        )
                        .with_code("remote.s3"),
                    );
                } else {
                    return Ok(());
                }
            }
            Ok(response) => {
                let status = response.status();
                if status == StatusCode::NOT_FOUND {
                    // A retry after a lost response sees no upload id. If the
                    // object already exists with the right size, complete is done.
                    let head = s3_request(
                        client,
                        Method::HEAD,
                        s3_url(config, &pending.object, &[])?,
                        EMPTY_SHA256,
                        &[],
                        credentials,
                    )?
                    .send()
                    .await;
                    if let Ok(head) = head {
                        if head.status().is_success()
                            && head.content_length() == Some(pending.bytes)
                        {
                            return Ok(());
                        }
                    }
                }
                last =
                    Some(s3_response_error("Completing the S3 multipart upload", response).await);
            }
            Err(error) => last = Some(network_error(error)),
        }
        if attempt < config.retries {
            tokio::time::sleep(retry_delay(attempt)).await;
        }
    }
    Err(last.unwrap_or_else(|| {
        AppError::user("The S3 multipart upload could not be completed.").with_code("remote.s3")
    }))
}

async fn s3_put_single(
    client: &reqwest::Client,
    config: &RemoteConfig,
    pending: &Pending,
    path: &Path,
    credentials: S3Creds<'_>,
) -> Result<()> {
    let url = s3_url(config, &pending.object, &[])?;
    let mut last = None;
    for attempt in 0..config.retries + 1 {
        let result = async {
            let file = tokio::fs::File::open(path)
                .await
                .map_err(|e| AppError::new("Could not read the upload archive.", e))?;
            let length = file
                .metadata()
                .await
                .map_err(|e| AppError::new("Could not read the upload archive.", e))?
                .len();
            if length != pending.bytes {
                return Err(AppError::user(
                    "The upload archive changed while it was being prepared.",
                )
                .with_code("remote.staging"));
            }
            let body = reqwest::Body::wrap_stream(ReaderStream::new(file));
            let response = s3_request(
                client,
                Method::PUT,
                url.clone(),
                &pending.sha256,
                &[],
                credentials,
            )?
            .header(CONTENT_LENGTH, length)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .send()
            .await
            .map_err(network_error)?;
            if !response.status().is_success() {
                return Err(s3_response_error("Uploading the S3 object", response).await);
            }
            Ok::<(), AppError>(())
        }
        .await;
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = Some(error);
                if attempt < config.retries {
                    tokio::time::sleep(retry_delay(attempt)).await;
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| AppError::user("The S3 upload failed.").with_code("remote.s3")))
}

async fn s3_multipart(
    client: &reqwest::Client,
    config: &RemoteConfig,
    root: &Path,
    pending: &mut Pending,
    path: &Path,
    credentials: S3Creds<'_>,
) -> Result<()> {
    let total_parts = ((pending.bytes + PART_SIZE - 1) / PART_SIZE).max(1) as u32;
    if total_parts > MAX_PARTS {
        return Err(AppError::user(
            "This archive is too large for the configured S3 multipart part size.",
        ));
    }
    let state = pending.s3.clone().unwrap_or_default();
    let mut upload_id = state.upload_id.clone();
    let mut parts = state.parts.clone();

    if let Some(ref id) = upload_id {
        match s3_list_parts(client, config, pending, id, credentials).await? {
            Some(server_parts) => {
                parts = server_parts;
            }
            None => {
                upload_id = None;
                parts.clear();
            }
        }
    }
    parts.retain(|part| part.number >= 1 && part.number <= total_parts);
    if upload_id.is_none() {
        upload_id = Some(s3_initiate(client, config, pending, credentials).await?);
        parts.clear();
    }
    pending.s3 = Some(S3State {
        upload_id: upload_id.clone(),
        parts: parts.clone(),
    });
    save_pending(root, pending)?;

    let upload_id_ref = upload_id.as_deref().unwrap_or_default().to_string();
    for number in 1..=total_parts {
        if parts.iter().any(|part| part.number == number) {
            continue;
        }
        let offset = (number as u64 - 1) * PART_SIZE;
        let length = PART_SIZE.min(pending.bytes.saturating_sub(offset));
        let mut last = None;
        let mut uploaded = None;
        for attempt in 0..config.retries + 1 {
            match s3_upload_part(
                client,
                config,
                pending,
                path,
                number,
                offset,
                length,
                &upload_id_ref,
                credentials,
            )
            .await
            {
                Ok(etag) => {
                    uploaded = Some(etag);
                    break;
                }
                Err(error) => {
                    last = Some(error);
                    if attempt < config.retries {
                        tokio::time::sleep(retry_delay(attempt)).await;
                    }
                }
            }
        }
        let etag = uploaded.ok_or_else(|| {
            last.unwrap_or_else(|| {
                AppError::user("An S3 part could not be uploaded.").with_code("remote.s3")
            })
        })?;
        parts.retain(|part| part.number != number);
        parts.push(CompletedPart {
            number,
            etag,
            bytes: length,
        });
        parts.sort_by_key(|part| part.number);
        pending.s3 = Some(S3State {
            upload_id: upload_id.clone(),
            parts: parts.clone(),
        });
        save_pending(root, pending)?;
    }
    let ordered = parts
        .iter()
        .filter(|part| part.number >= 1 && part.number <= total_parts)
        .cloned()
        .collect::<Vec<_>>();
    if ordered.len() != total_parts as usize {
        return Err(
            AppError::user("The S3 multipart upload is missing a completed part.")
                .with_code("remote.s3"),
        );
    }
    s3_complete(
        client,
        config,
        pending,
        &upload_id_ref,
        &ordered,
        credentials,
    )
    .await?;
    pending.s3 = None;
    save_pending(root, pending)?;
    Ok(())
}

async fn s3_upload(
    client: &reqwest::Client,
    config: &RemoteConfig,
    root: &Path,
    pending: &mut Pending,
    path: &Path,
    secret_key: &str,
) -> Result<&'static str> {
    let credentials = S3Creds {
        access: &config.access_key,
        secret: secret_key,
        region: &config.region,
    };
    if pending.bytes > MULTIPART_THRESHOLD {
        s3_multipart(client, config, root, pending, path, credentials).await?;
    } else {
        s3_put_single(client, config, pending, path, credentials).await?;
    }
    s3_verify(client, config, pending, credentials).await
}

fn find_rclone(config: &RemoteConfig) -> Option<PathBuf> {
    let custom = config.rclone_path.trim();
    if !custom.is_empty() {
        let path = PathBuf::from(custom);
        return path.is_file().then_some(path);
    }
    let names: &[&str] = if cfg!(windows) {
        &["rclone.exe", "rclone"]
    } else {
        &["rclone", "rclone.exe"]
    };
    for directory in std::env::split_paths(&std::env::var_os("PATH")?) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
fn rclone_executable(config: &RemoteConfig) -> Result<PathBuf> {
    find_rclone(config).ok_or_else(|| {
        AppError::user(
            "rclone was not found. Install it, or set the full path to rclone.exe in the remote target settings.",
        )
        .with_code("remote.rclone")
    })
}
fn rclone_join(base: &str, suffix: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with(':') {
        format!("{base}{suffix}")
    } else {
        format!("{base}/{suffix}")
    }
}
fn rclone_target(config: &RemoteConfig, object: &str) -> String {
    rclone_join(&config.remote, object)
}
fn rclone_dir(config: &RemoteConfig) -> String {
    if config.prefix.is_empty() {
        config.remote.clone()
    } else {
        rclone_join(&config.remote, &config.prefix)
    }
}
async fn rclone_command(executable: &Path, arguments: &[String]) -> Result<std::process::Output> {
    let mut command = tokio::process::Command::new(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let child = command
        .spawn()
        .map_err(|e| AppError::new("Could not start rclone.", e))?;
    tokio::time::timeout(TRANSFER_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            AppError::user("rclone did not finish within six hours.").with_code("remote.rclone")
        })?
        .map_err(|e| AppError::new("rclone could not be completed.", e))
}
fn rclone_failure(context: &str, output: &std::process::Output) -> AppError {
    let stderr = sanitize(String::from_utf8_lossy(&output.stderr), 600);
    let stdout = sanitize(String::from_utf8_lossy(&output.stdout), 300);
    let details = if stderr.is_empty() { stdout } else { stderr };
    AppError::new(
        format!(
            "{context} failed with exit code {}.",
            output.status.code().unwrap_or(-1)
        ),
        details,
    )
    .with_code("remote.rclone")
}
async fn rclone_run(
    executable: &Path,
    arguments: &[String],
    context: &str,
) -> Result<std::process::Output> {
    let output = rclone_command(executable, arguments).await?;
    if !output.status.success() {
        return Err(rclone_failure(context, &output));
    }
    Ok(output)
}
async fn rclone_size(executable: &Path, target: &str, retries: u32) -> Result<u64> {
    let arguments = vec![
        "size".to_string(),
        target.to_string(),
        "--json".to_string(),
        "--retries".to_string(),
        retries.to_string(),
        "--low-level-retries".to_string(),
        retries.to_string(),
    ];
    let output = rclone_run(executable, &arguments, "Checking the rclone object").await?;
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let value: serde_json::Value = serde_json::from_str(text.trim()).map_err(|e| {
        AppError::new(
            "rclone did not return a readable size.",
            format!("{e}; {}", sanitize(&text, 300)),
        )
        .with_code("remote.rclone")
    })?;
    value.get("bytes").and_then(|v| v.as_u64()).ok_or_else(|| {
        AppError::new(
            "rclone did not return a size for the uploaded object.",
            sanitize(&text, 300),
        )
        .with_code("remote.verify")
    })
}
async fn hash_stream<R: AsyncRead + Unpin>(mut reader: R) -> Result<(u64, String)> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|e| AppError::new("Could not read the uploaded stream.", e))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((total, hex(&hash.finalize())))
}
async fn rclone_hash(executable: &Path, target: &str) -> Result<(u64, String)> {
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("cat")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|e| AppError::new("Could not start rclone.", e))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::new("Could not read rclone output.", "No stdout pipe."))?;
    let mut stderr = child.stderr.take();
    let error_task = tokio::spawn(async move {
        let mut text = Vec::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.take(8192).read_to_end(&mut text).await;
        }
        String::from_utf8_lossy(&text).to_string()
    });
    let hashed = tokio::time::timeout(TRANSFER_TIMEOUT, hash_stream(stdout)).await;
    let (bytes, sha256) = hashed.map_err(|_| {
        AppError::user("rclone did not finish within six hours.").with_code("remote.rclone")
    })??;
    let status = tokio::time::timeout(TRANSFER_TIMEOUT, child.wait())
        .await
        .map_err(|_| {
            AppError::user("rclone did not finish within six hours.").with_code("remote.rclone")
        })?
        .map_err(|e| AppError::new("rclone could not be completed.", e))?;
    let stderr = error_task.await.unwrap_or_default();
    if !status.success() {
        let error = std::process::Output {
            status,
            stdout: Vec::new(),
            stderr: stderr.into_bytes(),
        };
        return Err(rclone_failure(
            "Downloading the rclone object for verification",
            &error,
        ));
    }
    Ok((bytes, sha256))
}
async fn rclone_upload(
    config: &RemoteConfig,
    pending: &Pending,
    path: &Path,
) -> Result<&'static str> {
    let executable = rclone_executable(config)?;
    let target = rclone_target(config, &pending.object);
    let directory = rclone_dir(config);
    let retries = config.retries.max(1).to_string();
    if !directory.ends_with(':') {
        rclone_run(
            &executable,
            &["mkdir".into(), directory.clone()],
            "Creating the rclone folder",
        )
        .await?;
    }
    let arguments = vec![
        "copyto".to_string(),
        path.to_string_lossy().into_owned(),
        target.clone(),
        "--checksum".to_string(),
        "--retries".to_string(),
        retries.clone(),
        "--low-level-retries".to_string(),
        retries.clone(),
        "--stats".to_string(),
        "0".to_string(),
    ];
    rclone_run(&executable, &arguments, "Uploading with rclone").await?;
    let size = rclone_size(&executable, &target, config.retries).await?;
    if size != pending.bytes {
        return Err(AppError::new(
            "The uploaded rclone object has the wrong size.",
            format!("expected {} bytes, found {size}", pending.bytes),
        )
        .with_code("remote.verify"));
    }
    if config.verify_content {
        let (bytes, sha256) = rclone_hash(&executable, &target).await?;
        if bytes != pending.bytes || sha256 != pending.sha256 {
            return Err(AppError::user(
                "The uploaded rclone object did not match its local checksum.",
            )
            .with_code("remote.verify"));
        }
        return Ok("sha256");
    }
    Ok("size")
}

fn progress(m: &Manager, id: &str, backup_id: &str, phase: &str, sent: u64, total: u64) {
    m.emit(
        "remote-upload-progress",
        serde_json::json!({
            "id": id,
            "backupId": backup_id,
            "phase": phase,
            "sentBytes": sent,
            "totalBytes": total,
        }),
    );
}

pub async fn test(m: Arc<Manager>, id: String) -> Result<()> {
    let root = remote_root(&m, &id)?;
    let config = load_config(&root)?.ok_or_else(|| {
        AppError::user("Configure a remote backup target first.").with_code("remote.config")
    })?;
    let client = upload_client()?;
    match config.kind {
        RemoteKind::Webdav => {
            let password = if secrets::available() {
                secrets::load(&m, &secret_name(&id, "password"))?
            } else {
                None
            };
            dav_ensure_dirs(&client, &config, password.as_deref()).await?;
            let probe = object_key(
                &config.prefix,
                &format!(".easylauncher-probe-{}", uuid::Uuid::new_v4()),
            );
            let base = parse_base_url(&config.endpoint, "WebDAV URL")?;
            let url = join_key(&base, &probe)?;
            let response = dav_request(
                &client,
                Method::PUT,
                url.clone(),
                &config,
                password.as_deref(),
            )
            .header(CONTENT_LENGTH, 0)
            .send()
            .await
            .map_err(network_error)?;
            if !response.status().is_success() {
                return Err(response_error("Writing to the WebDAV target", response).await);
            }
            let response = dav_request(&client, Method::DELETE, url, &config, password.as_deref())
                .send()
                .await
                .map_err(network_error)?;
            if !response.status().is_success() && response.status() != StatusCode::NOT_FOUND {
                m.logger.log(
                    "WARN",
                    &format!(
                        "WebDAV connection test could not remove its probe file (HTTP {}).",
                        response.status().as_u16()
                    ),
                );
            }
            Ok(())
        }
        RemoteKind::S3 => {
            let secret = if secrets::available() {
                secrets::load(&m, &secret_name(&id, "secret"))?
            } else {
                None
            }
            .ok_or_else(|| {
                AppError::user("Save the S3 secret key before testing the target.")
                    .with_code("remote.auth")
            })?;
            let credentials = S3Creds {
                access: &config.access_key,
                secret: &secret,
                region: &config.region,
            };
            let key = object_key(
                &config.prefix,
                &format!("easylauncher-probe-{}", uuid::Uuid::new_v4()),
            );
            let url = s3_url(&config, &key, &[("uploads", "")])?;
            let response = s3_request(&client, Method::POST, url, EMPTY_SHA256, &[], credentials)?
                .header(CONTENT_LENGTH, 0)
                .send()
                .await
                .map_err(network_error)?;
            if !response.status().is_success() {
                return Err(
                    s3_response_error("Starting a test S3 multipart upload", response).await,
                );
            }
            let xml = response.text().await.map_err(network_error)?;
            let upload_id = xml_tag(&xml, "UploadId").ok_or_else(|| {
                AppError::new(
                    "The S3 target did not return a test upload identifier.",
                    sanitize(xml, 400),
                )
                .with_code("remote.s3")
            })?;
            let url = s3_url(&config, &key, &[("uploadId", &upload_id)])?;
            let response =
                s3_request(&client, Method::DELETE, url, EMPTY_SHA256, &[], credentials)?
                    .send()
                    .await;
            if let Ok(response) = response {
                if !response.status().is_success() {
                    m.logger.log(
                        "WARN",
                        &format!(
                            "S3 connection test could not abort its probe upload (HTTP {}).",
                            response.status().as_u16()
                        ),
                    );
                }
            }
            Ok(())
        }
        RemoteKind::Rclone => {
            let executable = rclone_executable(&config)?;
            let directory = rclone_dir(&config);
            if !directory.ends_with(':') {
                rclone_run(
                    &executable,
                    &["mkdir".into(), directory],
                    "Creating the rclone folder",
                )
                .await?;
            }
            Ok(())
        }
    }
}

pub async fn upload(
    m: Arc<Manager>,
    id: String,
    backup_id: String,
    confirmed: bool,
) -> Result<RemoteUploadOutcome> {
    if !confirmed {
        return Err(AppError::user(
            "Confirm that this backup may be uploaded to the remote target.",
        ));
    }
    let _serial = UPLOAD_LOCK.lock().await;
    let root = remote_root(&m, &id)?;
    let config = load_config(&root)?.ok_or_else(|| {
        AppError::user("Configure a remote backup target first.").with_code("remote.config")
    })?;
    let runtime = m.runtime(&id)?;
    let prepared = {
        let _gate = runtime.gate.lock().await;
        let manager = m.clone();
        let server_id = id.clone();
        let target = config.clone();
        let backup = backup_id.clone();
        tokio::task::spawn_blocking(move || prepare(&manager, &server_id, &target, &backup))
            .await
            .map_err(|e| AppError::new("Could not prepare the remote backup archive.", e))??
    };
    progress(&m, &id, &backup_id, "upload", 0, prepared.pending.bytes);
    let client = upload_client()?;
    let result = match config.kind {
        RemoteKind::Webdav => {
            let password = if secrets::available() {
                secrets::load(&m, &secret_name(&id, "password"))?
            } else {
                None
            };
            webdav_upload(
                &client,
                &config,
                &prepared.pending,
                &prepared.path,
                password.as_deref(),
            )
            .await
        }
        RemoteKind::S3 => {
            let secret = if secrets::available() {
                secrets::load(&m, &secret_name(&id, "secret"))?
            } else {
                None
            }
            .ok_or_else(|| {
                AppError::user("Save the S3 secret key before uploading.").with_code("remote.auth")
            })?;
            let mut pending = prepared.pending.clone();
            s3_upload(
                &client,
                &config,
                &root,
                &mut pending,
                &prepared.path,
                &secret,
            )
            .await
        }
        RemoteKind::Rclone => rclone_upload(&config, &prepared.pending, &prepared.path).await,
    };
    let verification = match result {
        Ok(verification) => verification,
        Err(error) => {
            progress(&m, &id, &backup_id, "failed", 0, prepared.pending.bytes);
            return Err(error);
        }
    };
    progress(
        &m,
        &id,
        &backup_id,
        "verify",
        prepared.pending.bytes,
        prepared.pending.bytes,
    );
    let backup = &prepared.backup;
    let archive = RemoteArchive {
        backup_id: backup.id.clone(),
        object: prepared.pending.object.clone(),
        bytes: prepared.pending.bytes,
        sha256: prepared.pending.sha256.clone(),
        encrypted: prepared.pending.encrypted,
        uploaded_at: Utc::now().to_rfc3339(),
        backup_created_at: backup.created_at.clone(),
        label: backup.label.clone(),
        kind: config.kind,
        verification: verification.to_string(),
    };
    append_history(&root, &archive)?;
    clear_pending(&root)?;
    let _ = remove_staging_file(&root, &prepared.pending.staging);
    progress(
        &m,
        &id,
        &backup_id,
        "complete",
        prepared.pending.bytes,
        prepared.pending.bytes,
    );
    Ok(RemoteUploadOutcome {
        archive,
        verification: verification.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backup(id: &str) -> Backup {
        Backup {
            id: id.into(),
            created_at: "2026-10-20T12:34:56Z".into(),
            size_bytes: 10,
            worlds: vec!["world".into()],
            safety: false,
            reason: crate::models::BackupReason::Manual,
            label: None,
            minecraft_version: None,
            server_type: None,
            format: BackupFormat::Chunked,
            stored_bytes: 10,
            chunk_count: 1,
            file_count: 1,
        }
    }

    #[test]
    fn object_names_and_folders_are_safe() {
        let b = backup("12345678-abcd-4000-8000-000000000000");
        assert_eq!(
            object_name(&b, true),
            "easylauncher-20261020-123456-12345678.zip.age"
        );
        assert_eq!(
            object_name(&b, false),
            "easylauncher-20261020-123456-12345678.zip"
        );
        assert_eq!(object_key("", "a.zip"), "a.zip");
        assert_eq!(object_key("mc/backups", "a.zip"), "mc/backups/a.zip");
    }

    #[test]
    fn configuration_validation_catches_unsafe_targets() {
        let mut config = RemoteConfig::default();
        config.endpoint = "https://dav.example.com/backups".into();
        assert!(normalize_config(&mut config).is_ok());
        config.endpoint = "https://user:secret@dav.example.com".into();
        assert!(normalize_config(&mut config).is_err());
        config.endpoint = "https://dav.example.com".into();
        config.prefix = "../escape".into();
        assert!(normalize_config(&mut config).is_err());
        config.prefix = "mc backups".into();
        assert!(normalize_config(&mut config).is_err());
        let mut s3 = RemoteConfig {
            kind: RemoteKind::S3,
            bucket: "My_Bucket".into(),
            access_key: "key".into(),
            ..RemoteConfig::default()
        };
        assert!(normalize_config(&mut s3).is_err());
        s3.bucket = "mc-backups".into();
        assert!(normalize_config(&mut s3).is_ok());
    }

    #[test]
    fn sigv4_matches_the_aws_s3_example() {
        let url = Url::parse("https://examplebucket.s3.amazonaws.com/test.txt").unwrap();
        let time = DateTime::parse_from_rfc3339("2013-05-24T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let authorization = sigv4_authorization(
            "GET",
            &url,
            EMPTY_SHA256,
            &[("range", "bytes=0-9")],
            time,
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "us-east-1",
        )
        .unwrap();
        assert_eq!(
            authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn s3_urls_support_path_and_virtual_hosts() {
        let mut config = RemoteConfig {
            kind: RemoteKind::S3,
            bucket: "mc-backups".into(),
            access_key: "key".into(),
            ..RemoteConfig::default()
        };
        let url = s3_url(&config, "folder/a.zip", &[("uploads", "")]).unwrap();
        assert_eq!(
            url.as_str(),
            "https://mc-backups.s3.us-east-1.amazonaws.com/folder/a.zip?uploads="
        );
        config.endpoint = "http://127.0.0.1:9000".into();
        let url = s3_url(&config, "folder/a.zip", &[]).unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:9000/mc-backups/folder/a.zip"
        );
        config.path_style = false;
        config.endpoint = "http://minio.local:9000".into();
        let url = s3_url(&config, "folder/a.zip", &[]).unwrap();
        assert_eq!(
            url.as_str(),
            "http://mc-backups.minio.local:9000/folder/a.zip"
        );
    }

    #[test]
    fn parses_s3_multipart_xml() {
        let xml = r#"<?xml version="1.0"?><ListPartsResult><IsTruncated>false</IsTruncated><Part><PartNumber>1</PartNumber><ETag>"abc"</ETag><Size>100</Size></Part><Part><PartNumber>2</PartNumber><ETag>&quot;def&quot;</ETag><Size>50</Size></Part></ListPartsResult>"#;
        let parts = parse_parts(xml);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].number, 1);
        assert_eq!(parts[0].etag, "\"abc\"");
        assert_eq!(parts[1].etag, "\"def\"");
        assert_eq!(
            xml_tag(&xml.replace("false", "true"), "IsTruncated").as_deref(),
            Some("true")
        );
    }

    #[test]
    fn archive_contains_manifest_and_referenced_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("backups");
        let id = "12345678-abcd-4000-8000-000000000000";
        let source = root.join(id);
        fs::create_dir_all(source.join("chunks")).unwrap();
        let chunk = b"compressed-world";
        let hash = hex(&Sha256::digest(chunk));
        let chunk_path = root
            .join("chunks")
            .join(&hash[..2])
            .join(format!("{hash}.zst"));
        fs::create_dir_all(chunk_path.parent().unwrap()).unwrap();
        fs::write(&chunk_path, chunk).unwrap();
        let manifest = format!(
            r#"{{"version":1,"chunkSize":1048576,"worlds":["world"],"files":{{"world/level.dat":{{"bytes":{},"sha256":"{}","chunks":[{{"sha256":"{}","bytes":{}}}]}}}}}}"#,
            chunk.len(),
            hash,
            hash,
            chunk.len()
        );
        fs::write(source.join("manifest.json"), manifest).unwrap();
        let b = backup(id);
        storage::write_json(&source.join("backup.json"), &b).unwrap();
        let dest = temp.path().join("archive.zip");
        build_archive(&root, &source, &b, &dest).unwrap();
        let file = fs::File::open(&dest).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let names = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect::<Vec<_>>();
        assert!(names.contains(&"backup.json".to_string()));
        assert!(names.contains(&"manifest.json".to_string()));
        assert!(names.contains(&format!("chunks/{}/{hash}.zst", &hash[..2])));
        let mut data = Vec::new();
        zip.by_name(&format!("chunks/{}/{hash}.zst", &hash[..2]))
            .unwrap()
            .read_to_end(&mut data)
            .unwrap();
        assert_eq!(data, chunk);
    }

    #[test]
    fn age_encryption_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let plain = temp.path().join("plain.bin");
        let encrypted = temp.path().join("plain.age");
        fs::write(&plain, b"world data that should be private").unwrap();
        encrypt_archive(&plain, &encrypted, "a long test passphrase").unwrap();
        let ciphertext = fs::read(&encrypted).unwrap();
        assert!(ciphertext.starts_with(b"age-encryption.org/v1"));
        assert!(!ciphertext.windows(11).any(|w| w == b"world data"));
        let decryptor = age::Decryptor::new(&ciphertext[..]).unwrap();
        let identity = age::scrypt::Identity::new(age::secrecy::SecretString::from(
            "a long test passphrase".to_owned(),
        ));
        let mut reader = decryptor.decrypt(std::iter::once(&identity as _)).unwrap();
        let mut decrypted = Vec::new();
        reader.read_to_end(&mut decrypted).unwrap();
        assert_eq!(decrypted, b"world data that should be private");
    }

    #[test]
    fn webdav_folder_urls_keep_the_base_path() {
        let base = Url::parse("https://example.com/dav/").unwrap();
        let url = dir_url(&base, "mc/backups").unwrap();
        assert_eq!(url.as_str(), "https://example.com/dav/mc/backups/");
        let file = join_key(&base, "mc/backups/a.zip").unwrap();
        assert_eq!(file.as_str(), "https://example.com/dav/mc/backups/a.zip");
        assert!(file.as_str().starts_with(url.as_str()));
        let root = Url::parse("https://example.com/").unwrap();
        assert_eq!(
            join_key(&root, "a.zip").unwrap().as_str(),
            "https://example.com/a.zip"
        );
    }

    #[test]
    fn staging_names_are_restricted() {
        assert!(safe_file_name("upload-1234.zip.age"));
        assert!(!safe_file_name("../escape.zip"));
        assert!(!safe_file_name("C:escape.zip"));
        assert!(!safe_file_name(""));
        assert!(safe_segment("mc-backups_1.2"));
        assert!(!safe_segment(".."));
    }
}
