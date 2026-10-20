//! Signed application updates.
//!
//! A channel is one HTTPS URL that serves `latest.json`; its detached Ed25519
//! signature is served from `<url>.sig` as lowercase or uppercase hex. The
//! public key is compiled into the app (`updater.pub`), so a compromised host
//! cannot forge a manifest. The signature is verified over the exact bytes
//! before the manifest is parsed.
//!
//! The app only checks and downloads. A download is streamed into the app data
//! directory and verified against the signed size and SHA-256; the installer
//! is launched only by a separate, explicit user action. Nothing is installed
//! silently, and no signature key or private material ships with the binary.

use crate::{
    error::{AppError, Result},
    server::manager::Manager,
    storage,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

pub const MANIFEST_LIMIT: usize = 256 * 1024;
pub const SIGNATURE_LIMIT: usize = 1024;
pub const MAX_ASSET_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const RECORD: &str = "installer.json";

/// One downloadable platform build, as listed in the signed manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    /// `windows-x86_64`, `linux-x86_64`, `macos-aarch64`, ...
    pub platform: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex SHA-256.
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pub_date: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssetInfo {
    pub platform: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

impl From<&Asset> for AssetInfo {
    fn from(asset: &Asset) -> Self {
        Self {
            platform: asset.platform.clone(),
            url: asset.url.clone(),
            size: asset.size,
            sha256: asset.sha256.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// False when no channel URL is configured.
    pub configured: bool,
    pub channel: Option<String>,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub available: bool,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub asset: Option<AssetInfo>,
    /// Present when a verified installer for this version is already on disk.
    pub downloaded_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    version: String,
    file_name: String,
    sha256: String,
    size: u64,
}

fn err(code: &str, message: impl Into<String>, details: impl ToString) -> AppError {
    AppError::new(message, details).with_code(code)
}

fn network_error(e: impl ToString) -> AppError {
    err(
        "update.network",
        "Could not reach the update channel. Check the URL, proxy and connection.",
        e,
    )
}

/// Manifest versions are strict `major.minor.patch` numbers. Pre-release
/// ordering is intentionally not guessed; publish a new numeric version.
pub fn valid_version(text: &str) -> bool {
    let parts = text.trim().split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.len() <= 9 && part.bytes().all(|b| b.is_ascii_digit())
        })
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    if !valid_version(latest) || !valid_version(current) {
        return false;
    }
    let parse = |value: &str| -> (u64, u64, u64) {
        let mut parts = value.trim().split('.');
        let mut next = || {
            parts
                .next()
                .and_then(|part| part.parse::<u64>().ok())
                .unwrap_or(0)
        };
        (next(), next(), next())
    };
    parse(latest) > parse(current)
}

pub fn current_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// A channel must be a clean HTTPS URL with no query or fragment so the
/// detached signature URL can be derived by appending `.sig`.
pub fn valid_channel(channel: &str) -> bool {
    let trimmed = channel.trim();
    trimmed.len() <= 2048
        && trimmed.starts_with("https://")
        && trimmed.len() > "https://".len()
        && !trimmed.contains([' ', '\t', '\n', '\r', '?', '#'])
}

pub fn signature_url(channel: &str) -> String {
    format!("{}.sig", channel.trim())
}

fn public_key() -> Result<Vec<u8>> {
    let text = include_str!("../updater.pub").trim();
    let key = hex::decode(text).map_err(|e| {
        err(
            "update.notConfigured",
            "This build has no usable update signing key.",
            e,
        )
    })?;
    if key.len() != 32 {
        return Err(err(
            "update.notConfigured",
            "This build has no usable update signing key.",
            format!("expected 32 bytes, found {}", key.len()),
        ));
    }
    Ok(key)
}

/// Verifies a hex-encoded detached Ed25519 signature over `data`.
pub fn verify_signature(public: &[u8], data: &[u8], signature_hex: &str) -> bool {
    let signature = match hex::decode(signature_hex.trim()) {
        Ok(bytes) if bytes.len() == 64 => bytes,
        _ => return false,
    };
    let key = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public);
    key.verify(data, &signature).is_ok()
}

fn validate_asset(asset: &Asset) -> Result<()> {
    if asset.platform.is_empty()
        || asset.platform.len() > 64
        || !asset
            .platform
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(err(
            "update.invalidManifest",
            "The update manifest has an invalid platform name.",
            &asset.platform,
        ));
    }
    if asset.url.len() > 2048
        || !asset.url.starts_with("https://")
        || asset.url.contains([' ', '\t', '\n', '\r'])
    {
        return Err(err(
            "update.invalidManifest",
            "The update manifest has an insecure asset URL.",
            &asset.url,
        ));
    }
    if asset.size == 0 || asset.size > MAX_ASSET_BYTES {
        return Err(err(
            "update.invalidManifest",
            "The update asset size is outside the accepted range.",
            asset.size,
        ));
    }
    if asset.sha256.len() != 64 || !asset.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(err(
            "update.invalidManifest",
            "The update asset is missing a valid SHA-256.",
            &asset.sha256,
        ));
    }
    Ok(())
}

pub fn validate_manifest(manifest: &Manifest) -> Result<()> {
    if !valid_version(&manifest.version) {
        return Err(err(
            "update.invalidManifest",
            "The update manifest version must be major.minor.patch.",
            &manifest.version,
        ));
    }
    if manifest.notes.chars().count() > 20_000 {
        return Err(err(
            "update.invalidManifest",
            "The update notes are too long.",
            manifest.notes.chars().count(),
        ));
    }
    if manifest.assets.is_empty() {
        return Err(err(
            "update.invalidManifest",
            "The update manifest lists no assets.",
            "",
        ));
    }
    let mut platforms = std::collections::HashSet::new();
    for asset in &manifest.assets {
        validate_asset(asset)?;
        if !platforms.insert(asset.platform.to_ascii_lowercase()) {
            return Err(err(
                "update.invalidManifest",
                "The update manifest lists a platform twice.",
                &asset.platform,
            ));
        }
    }
    Ok(())
}

pub fn asset_for<'a>(manifest: &'a Manifest, platform: &str) -> Option<&'a Asset> {
    manifest
        .assets
        .iter()
        .find(|asset| asset.platform.eq_ignore_ascii_case(platform))
}

/// Extracts a plain file name from the signed asset URL. Non-alphanumeric
/// characters are replaced, dot names are rejected and length is bounded.
pub fn file_name(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let raw = path.rsplit('/').next()?;
    let decoded = percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .ok()?;
    let name = decoded
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if name.is_empty() || name.len() > 120 || name.starts_with('.') || name == "." || name == ".." {
        return None;
    }
    Some(name)
}

async fn fetch_limited(
    client: &reqwest::Client,
    url: &str,
    limit: usize,
    timeout_secs: u64,
) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(timeout_secs))
        .send()
        .await
        .map_err(network_error)?;
    if !response.status().is_success() {
        return Err(err(
            "update.network",
            "The update channel returned an error.",
            format!("{} {}", response.status(), url),
        ));
    }
    if response.content_length().unwrap_or(0) > limit as u64 {
        return Err(err(
            "update.tooLarge",
            "The update response is larger than expected.",
            response.content_length().unwrap_or_default(),
        ));
    }
    let mut data = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(network_error)?;
        if data.len() + chunk.len() > limit {
            return Err(err(
                "update.tooLarge",
                "The update response is larger than expected.",
                data.len() + chunk.len(),
            ));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

fn empty_status(channel: Option<String>) -> Status {
    Status {
        configured: channel.is_some(),
        channel,
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        latest_version: None,
        available: false,
        notes: None,
        pub_date: None,
        asset: None,
        downloaded_path: None,
    }
}

fn update_dir(root: &Path, version: &str) -> Result<PathBuf> {
    if !valid_version(version) {
        return Err(err(
            "update.invalidManifest",
            "The update version is invalid.",
            version,
        ));
    }
    let updates = storage::child(root, "updates")?;
    storage::child(&updates, version)
}

fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 120
        && !name.starts_with('.')
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn downloaded_path(root: &Path, version: &str, asset: &Asset) -> Option<String> {
    let dir = update_dir(root, version).ok()?;
    let record: Record = serde_json::from_slice(&fs::read(dir.join(RECORD)).ok()?).ok()?;
    if record.version != version
        || !record.sha256.eq_ignore_ascii_case(&asset.sha256)
        || record.size != asset.size
        || !plain_name(&record.file_name)
    {
        return None;
    }
    let file = dir.join(&record.file_name);
    file.is_file().then(|| file.display().to_string())
}

async fn check_channel(
    client: &reqwest::Client,
    root: &Path,
    channel: Option<&str>,
) -> Result<Status> {
    let Some(channel) = channel.map(str::trim).filter(|url| !url.is_empty()) else {
        return Ok(empty_status(None));
    };
    if !valid_channel(channel) {
        return Err(err(
            "update.invalidChannel",
            "Enter a direct HTTPS URL to the signed update manifest.",
            channel,
        ));
    }
    let public = public_key()?;
    let raw = fetch_limited(client, channel, MANIFEST_LIMIT, 20).await?;
    let signature = fetch_limited(client, &signature_url(channel), SIGNATURE_LIMIT, 20).await?;
    let signature_text = std::str::from_utf8(&signature).map_err(|e| {
        err(
            "update.badSignature",
            "The update signature is not valid hex text.",
            e,
        )
    })?;
    if !verify_signature(&public, &raw, signature_text) {
        return Err(err(
            "update.badSignature",
            "The update manifest signature does not match this app's signing key.",
            channel,
        ));
    }
    let manifest: Manifest = serde_json::from_slice(&raw).map_err(|e| {
        err(
            "update.invalidManifest",
            "The signed update manifest could not be parsed.",
            e,
        )
    })?;
    validate_manifest(&manifest)?;
    let current = env!("CARGO_PKG_VERSION");
    let available = is_newer(&manifest.version, current);
    let found = asset_for(&manifest, &current_platform()).cloned();
    let downloaded = if available {
        found
            .as_ref()
            .and_then(|asset| downloaded_path(root, &manifest.version, asset))
    } else {
        None
    };
    let asset = found.map(|asset| AssetInfo::from(&asset));
    Ok(Status {
        configured: true,
        channel: Some(channel.to_string()),
        current_version: current.to_string(),
        latest_version: Some(manifest.version),
        available,
        notes: Some(manifest.notes).filter(|notes| !notes.trim().is_empty()),
        pub_date: manifest.pub_date,
        asset,
        downloaded_path: downloaded,
    })
}

/// Fetches, verifies and compares the signed manifest. No file is written.
pub async fn check(m: &Manager) -> Result<Status> {
    let settings = m.settings()?;
    check_channel(&m.client(), &m.root, settings.update_channel.as_deref()).await
}

/// Downloads the installer for the currently signed release into the app data
/// directory, verifying its size and SHA-256. It never runs the file.
pub async fn download(m: &Manager, version: &str) -> Result<Status> {
    let mut status = check(m).await?;
    if !status.available || status.latest_version.as_deref() != Some(version) {
        return Err(err(
            "update.changed",
            "The release changed or is no longer newer. Check again.",
            version,
        ));
    }
    let asset = status.asset.clone().ok_or_else(|| {
        err(
            "update.noAsset",
            "This release has no build for this platform.",
            current_platform(),
        )
    })?;
    let name = file_name(&asset.url).ok_or_else(|| {
        err(
            "update.invalidManifest",
            "The installer URL does not end in a usable file name.",
            &asset.url,
        )
    })?;
    let dir = update_dir(&m.root, version)?;
    fs::create_dir_all(&dir)?;
    storage::no_links(&dir)?;
    let target = dir.join(&name);
    storage::no_links(&target)?;
    let temp = dir.join(format!(".{}.part", uuid::Uuid::new_v4()));

    let response = m
        .client()
        .get(&asset.url)
        .timeout(Duration::from_secs(1800))
        .send()
        .await
        .map_err(network_error)?;
    if !response.status().is_success() {
        return Err(err(
            "update.network",
            "The installer download returned an error.",
            format!("{} {}", response.status(), asset.url),
        ));
    }
    if let Some(length) = response.content_length() {
        if length != asset.size {
            return Err(err(
                "update.sizeMismatch",
                "The server reported a different installer size than the signed manifest.",
                format!("{length} != {}", asset.size),
            ));
        }
    }

    let result = async {
        let mut file = tokio::fs::File::create(&temp).await?;
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(network_error)?;
            total = total.saturating_add(chunk.len() as u64);
            if total > asset.size || total > MAX_ASSET_BYTES {
                return Err(err(
                    "update.sizeMismatch",
                    "The installer is larger than the signed size.",
                    total,
                ));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        if total != asset.size {
            return Err(err(
                "update.sizeMismatch",
                "The installer size does not match the signed manifest.",
                format!("{total} != {}", asset.size),
            ));
        }
        let digest = hex::encode(hasher.finalize());
        if !digest.eq_ignore_ascii_case(&asset.sha256) {
            return Err(err(
                "update.hashMismatch",
                "The installer checksum does not match the signed manifest.",
                digest,
            ));
        }
        tokio::fs::rename(&temp, &target).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
        return result.map(|_| status);
    }

    let record = Record {
        version: version.to_string(),
        file_name: name,
        sha256: asset.sha256.to_ascii_lowercase(),
        size: asset.size,
    };
    storage::write_json(&dir.join(RECORD), &record)?;
    status.downloaded_path = Some(target.display().to_string());
    Ok(status)
}

/// Re-verifies the downloaded installer's SHA-256 and opens it with the OS.
/// This is the only point where an installer is launched, and only after an
/// explicit request.
pub fn open_installer(m: &Manager, version: &str) -> Result<()> {
    let dir = update_dir(&m.root, version)?;
    let record_file = dir.join(RECORD);
    storage::no_links(&record_file)?;
    let record: Record = serde_json::from_slice(&fs::read(&record_file)?)?;
    if record.version != version || !plain_name(&record.file_name) {
        return Err(err(
            "update.invalidRecord",
            "The downloaded installer record is invalid.",
            &record.file_name,
        ));
    }
    let file = dir.join(&record.file_name);
    storage::no_links(&file)?;
    let metadata = fs::metadata(&file).map_err(|e| {
        err(
            "update.notDownloaded",
            "The installer is no longer on disk. Download it again.",
            e,
        )
    })?;
    if !metadata.is_file() || metadata.len() != record.size {
        return Err(err(
            "update.hashMismatch",
            "The installer changed after download. Download it again.",
            file.display(),
        ));
    }
    let mut hasher = Sha256::new();
    let mut source = fs::File::open(&file)?;
    std::io::copy(&mut source, &mut hasher)?;
    if !hex::encode(hasher.finalize()).eq_ignore_ascii_case(&record.sha256) {
        return Err(err(
            "update.hashMismatch",
            "The installer changed after download. Download it again.",
            file.display(),
        ));
    }
    open::that(&file).map_err(|e| {
        err(
            "update.openFailed",
            "The installer could not be opened. Open it from the app data folder.",
            e,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::{
        rand::SystemRandom,
        signature::{Ed25519KeyPair, KeyPair},
    };

    fn asset(platform: &str) -> Asset {
        Asset {
            platform: platform.into(),
            url: "https://github.com/example/releases/download/v0.2.0/EasyLauncher_0.2.0_x64-setup.exe"
                .into(),
            size: 48_000_000,
            sha256: "a".repeat(64),
        }
    }

    fn manifest() -> Manifest {
        Manifest {
            version: "0.2.0".into(),
            notes: "Fixes and improvements.".into(),
            pub_date: Some("2026-10-20T00:00:00Z".into()),
            assets: vec![asset("windows-x86_64"), asset("linux-x86_64")],
        }
    }

    #[test]
    fn versions_are_strict_and_ordered_numerically() {
        assert!(valid_version("0.2.0"));
        assert!(valid_version("10.20.300"));
        assert!(!valid_version("0.2"));
        assert!(!valid_version("0.2.0-beta"));
        assert!(!valid_version("v0.2.0"));
        assert!(!valid_version("0.2.0.1"));
        assert!(!valid_version("0.2.x"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("10.0.0", "9.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("bad", "0.1.0"));
    }

    #[test]
    fn update_channels_require_direct_https_urls() {
        assert!(valid_channel("https://example.com/updates/latest.json"));
        assert!(!valid_channel("http://example.com/latest.json"));
        assert!(!valid_channel("https://example.com/latest.json?token=1"));
        assert!(!valid_channel("https://example.com/latest.json#top"));
        assert!(!valid_channel("https://"));
        assert!(!valid_channel("https://example.com/a b.json"));
        assert_eq!(
            signature_url("https://example.com/latest.json"),
            "https://example.com/latest.json.sig"
        );
    }

    #[test]
    fn detached_ed25519_signatures_verify_over_exact_bytes() {
        let random = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let public = pair.public_key().as_ref().to_vec();
        let signature = hex::encode(pair.sign(b"{\"version\":\"0.2.0\"}").as_ref());
        assert!(verify_signature(
            &public,
            b"{\"version\":\"0.2.0\"}",
            &signature
        ));
        assert!(!verify_signature(
            &public,
            b"{\"version\":\"0.3.0\"}",
            &signature
        ));
        assert!(!verify_signature(&public, b"{}", "not-hex"));
        assert!(!verify_signature(&public, b"{}", &signature[..64]));
        assert!(!verify_signature(&[0u8; 32], b"{}", &signature));
        assert!(verify_signature(
            &public,
            b"{\"version\":\"0.2.0\"}",
            &signature.to_ascii_uppercase()
        ));
    }

    #[test]
    fn manifests_require_valid_versions_assets_and_hashes() {
        assert!(validate_manifest(&manifest()).is_ok());

        let mut bad_version = manifest();
        bad_version.version = "0.2".into();
        assert!(validate_manifest(&bad_version).is_err());

        let mut insecure = manifest();
        insecure.assets[0].url = "http://example.com/setup.exe".into();
        assert!(validate_manifest(&insecure).is_err());

        let mut bad_hash = manifest();
        bad_hash.assets[0].sha256 = "xyz".into();
        assert!(validate_manifest(&bad_hash).is_err());

        let mut zero = manifest();
        zero.assets[0].size = 0;
        assert!(validate_manifest(&zero).is_err());

        let mut duplicate = manifest();
        duplicate.assets[1].platform = "WINDOWS-x86_64".into();
        assert!(validate_manifest(&duplicate).is_err());

        let mut empty = manifest();
        empty.assets.clear();
        assert!(validate_manifest(&empty).is_err());

        let mut long_notes = manifest();
        long_notes.notes = "x".repeat(20_001);
        assert!(validate_manifest(&long_notes).is_err());
    }

    #[test]
    fn assets_are_selected_for_the_current_platform_case_insensitively() {
        let manifest = manifest();
        assert!(asset_for(&manifest, "windows-x86_64").is_some());
        assert!(asset_for(&manifest, "WINDOWS-X86_64").is_some());
        assert!(asset_for(&manifest, "macos-aarch64").is_none());
        assert_eq!(
            AssetInfo::from(asset_for(&manifest, "linux-x86_64").unwrap()).size,
            48_000_000
        );
    }

    #[test]
    fn asset_urls_reduce_to_plain_safe_file_names() {
        assert_eq!(
            file_name(
                "https://github.com/example/releases/download/v0.2.0/EasyLauncher_0.2.0_x64-setup.exe?download=1"
            )
            .as_deref(),
            Some("EasyLauncher_0.2.0_x64-setup.exe")
        );
        assert_eq!(
            file_name("https://example.com/a/b%20c.exe").as_deref(),
            Some("b_c.exe")
        );
        assert_eq!(file_name("https://example.com/%2e%2e"), None);
        assert_eq!(file_name("https://example.com/..%2f..%2fcmd.exe"), None);
        assert_eq!(file_name("https://example.com/"), None);
        assert_eq!(file_name("https://example.com/.hidden"), None);
    }

    #[test]
    fn the_compiled_signing_key_is_a_full_ed25519_public_key() {
        let key = public_key().unwrap();
        assert_eq!(key.len(), 32);
        assert!(key.iter().any(|byte| *byte != 0));
    }
}
