use crate::{
    error::{AppError, Result},
    java::detector::JavaRuntime,
    server::manager::Manager,
    storage,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;

const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 30_000;
const MANIFEST: &str = "minehost-java.json";
const IMAGE_TYPE: &str = "jre";
const MIRROR_BASE: &str = "https://mirrors.tuna.tsinghua.edu.cn/Adoptium";
const CHUNK_TIMEOUT: Duration = Duration::from_secs(30);
const SLOW_AFTER: Duration = Duration::from_secs(12);
const SLOW_BYTES_PER_SEC: f64 = 64.0 * 1024.0;

pub const SUPPORTED_MAJORS: [u32; 4] = [8, 17, 21, 25];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedJava {
    pub id: String,
    pub major: u32,
    pub vendor: String,
    pub version: String,
    pub path: String,
    pub installed_at: String,
    pub size_bytes: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaProgress {
    pub id: String,
    pub stage: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub attempt: u32,
}

pub fn list(root: &Path) -> Vec<ManagedJava> {
    let mut items = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return items;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !dir.is_dir() || name.starts_with('.') || name.starts_with("deleted-") {
            continue;
        }
        if storage::validate_id(&name).is_err() {
            continue;
        }
        let Ok(bytes) = fs::read(dir.join(MANIFEST)) else {
            continue;
        };
        let Ok(item) = serde_json::from_slice::<ManagedJava>(&bytes) else {
            continue;
        };
        if item.id == name && Path::new(&item.path).is_file() {
            items.push(item);
        }
    }
    items.sort_by(|a, b| {
        b.major
            .cmp(&a.major)
            .then_with(|| b.version.cmp(&a.version))
    });
    items
}

pub fn executable(runtime: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "java.exe" } else { "java" };
    for relative in [
        format!("bin/{name}"),
        format!("Contents/Home/bin/{name}"),
        format!("jre/bin/{name}"),
    ] {
        let p = runtime.join(&relative);
        if p.is_file() {
            return Some(p);
        }
    }
    find(&runtime.to_path_buf(), name, 0)
}

fn find(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth > 3 {
        return None;
    }
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find(&path, name, depth + 1) {
                return Some(found);
            }
        } else if path.file_name().and_then(|s| s.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}

fn platform() -> Result<(&'static str, &'static str)> {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "linux" => "linux",
        "macos" => "mac",
        other => {
            return Err(AppError::user(format!(
                "Managed Java installation is not available on {other}."
            )))
        }
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "aarch64",
        "x86" => "x86",
        "arm" => "arm",
        other => {
            return Err(AppError::user(format!(
                "Managed Java installation is not available for {other}."
            )))
        }
    };
    Ok((os, arch))
}

struct Release {
    sources: Vec<String>,
    sha256: String,
    size: Option<u64>,
    semver: String,
}

/// Builds the Tsinghua mirror URL for an Adoptium asset. The checked-in checksum
/// from the official API still verifies whatever the mirror serves.
fn mirror_url(major: u32, name: &str) -> Option<String> {
    let (os, arch) = platform().ok()?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return None;
    }
    Some(format!(
        "{MIRROR_BASE}/{major}/{IMAGE_TYPE}/{arch}/{os}/{name}"
    ))
}

async fn resolve(client: &reqwest::Client, major: u32) -> Result<Release> {
    let (os, arch) = platform()?;
    let url = format!("https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={arch}&image_type={IMAGE_TYPE}&jvm_impl=hotspot&os={os}&vendor=eclipse");
    let response = client.get(url).send().await?.error_for_status()?;
    let assets: serde_json::Value = response.json().await?;
    let asset = assets.as_array().and_then(|a| a.first()).ok_or_else(|| {
        AppError::user(format!(
            "No Eclipse Temurin {major} build is available for this system."
        ))
    })?;
    let package = &asset["binary"]["package"];
    let link = package["link"].as_str().ok_or_else(|| {
        AppError::user("The Java provider did not return a download link. Try again later.")
    })?;
    if !link.starts_with("https://") {
        return Err(AppError::user(
            "The Java provider returned an insecure download link.",
        ));
    }
    let sha256 = package["checksum"].as_str().ok_or_else(|| {
        AppError::user("The Java provider did not return a checksum. Try again later.")
    })?;
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::user(
            "The Java provider returned an invalid checksum.",
        ));
    }
    let name = package["name"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| link.rsplit('/').next().map(str::to_owned))
        .unwrap_or_default();
    let mut sources = Vec::with_capacity(2);
    if let Some(mirror) = mirror_url(major, &name) {
        sources.push(mirror);
    }
    sources.push(link.into());
    Ok(Release {
        sources,
        sha256: sha256.to_ascii_lowercase(),
        size: package["size"].as_u64(),
        semver: asset["version"]["semver"]
            .as_str()
            .or_else(|| asset["release_name"].as_str())
            .unwrap_or("unknown")
            .to_string(),
    })
}

async fn fetch(
    m: &Manager,
    client: &reqwest::Client,
    release: &Release,
    url: &str,
    dest: &Path,
    id: &str,
    attempt: u32,
) -> Result<u64> {
    let response = client.get(url).send().await?.error_for_status()?;
    let total = release.size.or(response.content_length());
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut tick = Instant::now() - Duration::from_secs(1);
    let started = Instant::now();
    while let Some(chunk) = tokio::time::timeout(CHUNK_TIMEOUT, stream.next())
        .await
        .map_err(|_| AppError::user("The Java download stalled. Please retry."))?
    {
        let chunk = chunk?;
        bytes += chunk.len() as u64;
        if bytes > MAX_ARCHIVE {
            return Err(AppError::user(
                "The Java archive exceeded the 512 MB safety limit.",
            ));
        }
        hash.update(&chunk);
        file.write_all(&chunk).await?;
        let elapsed = started.elapsed();
        if elapsed >= SLOW_AFTER && bytes as f64 / elapsed.as_secs_f64() < SLOW_BYTES_PER_SEC {
            return Err(AppError::user(
                "The Java download source is too slow. Trying another source.",
            ));
        }
        if tick.elapsed() > Duration::from_millis(150) {
            m_emit_progress(m, id, "downloading", bytes, total, attempt);
            tick = Instant::now();
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    if total.map(|n| n != bytes).unwrap_or(false) {
        return Err(AppError::user(
            "The Java download is incomplete. Please retry.",
        ));
    }
    if format!("{:x}", hash.finalize()) != release.sha256 {
        return Err(AppError::user(
            "Java checksum verification failed. The file was discarded.",
        ));
    }
    Ok(bytes)
}

fn m_emit_progress(
    m: &Manager,
    id: &str,
    stage: &str,
    downloaded: u64,
    total: Option<u64>,
    attempt: u32,
) {
    m.emit(
        "java-progress",
        JavaProgress {
            id: id.into(),
            stage: stage.into(),
            downloaded_bytes: downloaded,
            total_bytes: total,
            attempt,
        },
    );
}

async fn download(m: &Manager, release: &Release, dest: &Path, id: &str) -> Result<u64> {
    storage::no_links(dest)?;
    let client = m.client();
    let mut last = AppError::user("Download failed.");
    let mut attempt = 0;
    for url in &release.sources {
        for _ in 0..2 {
            attempt += 1;
            match fetch(m, &client, release, url, dest, id, attempt).await {
                Ok(bytes) => {
                    m.emit(
                        "java-progress",
                        JavaProgress {
                            id: id.into(),
                            stage: "verifying".into(),
                            downloaded_bytes: bytes,
                            total_bytes: Some(bytes),
                            attempt,
                        },
                    );
                    m.logger
                        .log("INFO", &format!("Downloaded the Java archive from {url}"));
                    return Ok(bytes);
                }
                Err(e) => {
                    last = e;
                    let _ = tokio::fs::remove_file(dest).await;
                }
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }
    Err(last)
}

fn zip_error(e: zip::result::ZipError) -> AppError {
    AppError::new("The Java archive could not be read safely.", e)
}

fn extract(archive_path: &Path, dest: &Path) -> Result<u64> {
    use zip::ZipArchive;
    storage::no_links(archive_path)?;
    fs::create_dir_all(dest)?;
    let file = fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).map_err(zip_error)?;
    if archive.len() > MAX_ENTRIES {
        return Err(AppError::user(
            "The Java archive contains too many entries.",
        ));
    }
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(AppError::user(
                "The Java archive contains an unsafe path and was discarded.",
            ));
        };
        if entry
            .unix_mode()
            .map(|m| m & 0o170000 == 0o120000)
            .unwrap_or(false)
        {
            return Err(AppError::user(
                "The Java archive contains a link and was discarded.",
            ));
        }
        let out = dest.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > MAX_EXTRACTED {
            return Err(AppError::user(
                "The Java archive exceeded the 1 GB extraction limit.",
            ));
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        storage::no_links(&out)?;
        let mut output = fs::File::create(&out)?;
        let copied = std::io::copy(&mut entry, &mut output)?;
        if copied != entry.size() {
            return Err(AppError::user(
                "The Java archive ended unexpectedly and was discarded.",
            ));
        }
        output.sync_all()?;
    }
    Ok(total)
}

pub async fn install(m: &Manager, major: u32) -> Result<ManagedJava> {
    if !SUPPORTED_MAJORS.contains(&major) {
        return Err(AppError::user(
            "Choose Java 8, 17, 21 or 25 for a managed runtime.",
        ));
    }
    let _gate = m.java_gate.lock().await;
    storage::no_links(&m.java)?;
    fs::create_dir_all(&m.java)?;
    let id = uuid::Uuid::new_v4().to_string();
    m.emit(
        "java-progress",
        JavaProgress {
            id: id.clone(),
            stage: "resolving".into(),
            downloaded_bytes: 0,
            total_bytes: None,
            attempt: 1,
        },
    );
    let release = resolve(&m.client(), major).await?;
    let staging = storage::child(&m.java, &format!(".staging-{id}"))?;
    let final_dir = storage::managed(&m.java, &id)?;
    let result = async {
        fs::create_dir(&staging)?;
        let archive = staging.join("temurin.zip");
        download(m, &release, &archive, &id).await?;
        m.emit(
            "java-progress",
            JavaProgress {
                id: id.clone(),
                stage: "extracting".into(),
                downloaded_bytes: 0,
                total_bytes: None,
                attempt: 1,
            },
        );
        let runtime = staging.join("runtime");
        let size = tokio::task::spawn_blocking({
            let archive = archive.clone();
            let runtime = runtime.clone();
            move || extract(&archive, &runtime)
        })
        .await
        .map_err(|e| AppError::new("Java extraction was interrupted.", e))??;
        let exe = executable(&runtime).ok_or_else(|| {
            AppError::user("The extracted Java archive did not contain a runtime.")
        })?;
        let inspected = crate::java::detector::inspect(&exe.to_string_lossy()).await?;
        if inspected.major != major {
            return Err(AppError::user(format!(
                "The downloaded runtime is Java {}, not Java {major}.",
                inspected.major
            )));
        }
        let relative = exe
            .strip_prefix(&runtime)
            .map_err(|_| AppError::user("The extracted Java runtime has an invalid layout."))?
            .to_path_buf();
        let item = ManagedJava {
            id: id.clone(),
            major,
            vendor: "Eclipse Temurin".into(),
            version: release.semver.clone(),
            path: final_dir.join(relative).to_string_lossy().into(),
            installed_at: chrono::Utc::now().to_rfc3339(),
            size_bytes: size,
        };
        storage::write_json(&staging.join(MANIFEST), &item)?;
        fs::remove_file(&archive)?;
        fs::rename(&staging, &final_dir)?;
        Ok(item)
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    match result {
        Ok(item) => {
            m.logger.log(
                "INFO",
                &format!("Installed managed Java {} ({})", item.major, item.id),
            );
            Ok(item)
        }
        Err(e) => {
            m.emit(
                "java-progress",
                JavaProgress {
                    id,
                    stage: "failed".into(),
                    downloaded_bytes: 0,
                    total_bytes: None,
                    attempt: 1,
                },
            );
            Err(e)
        }
    }
}

pub async fn delete(m: &Manager, id: &str, confirmed: bool) -> Result<()> {
    if !confirmed {
        return Err(AppError::user(
            "Confirm removal of this Java runtime first.",
        ));
    }
    let _gate = m.java_gate.lock().await;
    let dir = storage::managed(&m.java, id)?;
    if !dir.join(MANIFEST).is_file() {
        return Err(AppError::user(
            "This managed Java runtime could not be found.",
        ));
    }
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    if let Some(server) = in_use(m, &canonical)? {
        return Err(AppError::user(format!(
            "“{server}” is configured to use this Java runtime. Choose another runtime for that server first."
        )));
    }
    storage::validate_tree(&dir)?;
    let trash = storage::child(&m.java, &format!("deleted-{id}"))?;
    fs::rename(&dir, &trash)?;
    fs::remove_dir_all(trash)?;
    m.logger.log("INFO", &format!("Removed managed Java {id}"));
    Ok(())
}

fn in_use(m: &Manager, runtime: &Path) -> Result<Option<String>> {
    for view in m.list()? {
        let Some(path) = view.server.java_path else {
            continue;
        };
        let path = PathBuf::from(path);
        let path = path.canonicalize().unwrap_or(path);
        if path.starts_with(runtime) {
            return Ok(Some(view.server.name));
        }
    }
    Ok(None)
}

pub fn mark(runtime: &JavaRuntime, managed_root: &Path) -> bool {
    let path = PathBuf::from(&runtime.path);
    let root = managed_root
        .canonicalize()
        .unwrap_or_else(|_| managed_root.to_path_buf());
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture_executable(runtime: &Path) -> PathBuf {
        let name = if cfg!(windows) { "java.exe" } else { "java" };
        runtime.join("bin").join(name)
    }

    #[test]
    fn manifest_roundtrip_and_listing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let id = uuid::Uuid::new_v4().to_string();
        let dir = root.join(&id);
        let exe = fixture_executable(&dir.join("runtime"));
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"fixture").unwrap();
        let item = ManagedJava {
            id: id.clone(),
            major: 21,
            vendor: "Eclipse Temurin".into(),
            version: "21.0.5+11".into(),
            path: exe.to_string_lossy().into(),
            installed_at: "2026-01-01".into(),
            size_bytes: 7,
        };
        storage::write_json(&dir.join(MANIFEST), &item).unwrap();
        let listed = list(root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].major, 21);
        assert_eq!(executable(&dir.join("runtime")), Some(exe));
    }

    #[test]
    fn corrupt_manifest_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MANIFEST), b"not json").unwrap();
        assert!(list(temp.path()).is_empty());
    }

    #[test]
    fn mirror_url_follows_adoptium_layout() {
        let url = mirror_url(25, "OpenJDK25U-jre_x64_windows_hotspot_25.0.4.1_1.zip").unwrap();
        assert!(url.starts_with(MIRROR_BASE));
        assert!(url.contains("/25/jre/"));
        assert!(url.ends_with("/OpenJDK25U-jre_x64_windows_hotspot_25.0.4.1_1.zip"));
        assert!(mirror_url(25, "../escape.zip").is_none());
        assert!(mirror_url(25, "").is_none());
    }

    #[test]
    fn extraction_rejects_traversal() {
        use zip::write::SimpleFileOptions;
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("unsafe.zip");
        {
            let file = fs::File::create(&archive).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("../escape", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"x").unwrap();
            writer.finish().unwrap();
        }
        let dest = temp.path().join("out");
        assert!(extract(&archive, &dest).is_err());
        assert!(!temp.path().join("escape").exists());
    }

    #[test]
    fn extraction_writes_files() {
        use zip::write::SimpleFileOptions;
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("safe.zip");
        {
            let file = fs::File::create(&archive).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file(
                    format!(
                        "jdk/bin/{}",
                        if cfg!(windows) { "java.exe" } else { "java" }
                    ),
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(b"fixture").unwrap();
            writer.finish().unwrap();
        }
        let dest = temp.path().join("out");
        assert_eq!(extract(&archive, &dest).unwrap(), 7);
        assert_eq!(
            fs::read(fixture_executable(&dest.join("jdk"))).unwrap(),
            b"fixture"
        );
    }
}
