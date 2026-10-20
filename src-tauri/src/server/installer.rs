use crate::{
    error::{AppError, Result},
    models::{DownloadSource, ServerType},
};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Serialize;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;

pub use super::software::Artifact;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: String,
    pub stage: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub attempt: u32,
}
// Each adapter resolves an immutable artifact for exactly the requested release.
pub struct ServerInstaller;
pub async fn cancelled(cancel: &AtomicBool) {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}
impl ServerInstaller {
    /// Resolves the artifact through the selected software adapter. Download
    /// verification below is shared by every adapter.
    pub async fn resolve(
        client: &Client,
        version: &str,
        software: ServerType,
        source: DownloadSource,
    ) -> Result<Artifact> {
        super::software::resolve(client, software, version, source).await
    }
}
pub async fn download(
    client: &Client,
    artifact: &Artifact,
    dest: &Path,
    cancel: &AtomicBool,
    progress: impl Fn(Progress),
    id: &str,
) -> Result<()> {
    let part = dest.with_extension("jar.part");
    crate::storage::no_links(&part)?;
    crate::storage::no_links(dest)?;
    let mut last = AppError::user("Download failed.");
    for attempt in 1..=3 {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::user(
                "Installation cancelled. You can retry from the server card.",
            ));
        }
        let result = async {
            let response = client.get(&artifact.url).send().await?.error_for_status()?;
            let total = artifact.size.or(response.content_length());
            let mut bytes = 0u64;
            let mut h1 = Sha1::new();
            let mut h256 = Sha256::new();
            let mut header: Vec<u8> = Vec::new();
            let mut file = tokio::fs::File::create(&part).await?;
            let mut stream = response.bytes_stream();
            let mut tick = Instant::now() - Duration::from_secs(1);
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(AppError::user(
                        "Installation cancelled. You can retry from the server card.",
                    ));
                }
                let chunk = tokio::time::timeout(Duration::from_secs(30), stream.next())
                    .await
                    .map_err(|_| AppError::user("The download stalled. Please retry."))?;
                let Some(chunk) = chunk else { break };
                let chunk = chunk?;
                bytes += chunk.len() as u64;
                if bytes > 1024 * 1024 * 1024 {
                    return Err(AppError::user(
                        "The server artifact exceeded the 1 GB safety limit.",
                    ));
                }
                if header.len() < 4 {
                    header.extend(chunk.iter().take(4 - header.len()));
                }
                h1.update(&chunk);
                h256.update(&chunk);
                file.write_all(&chunk).await?;
                if tick.elapsed() > Duration::from_millis(150) {
                    progress(Progress {
                        id: id.into(),
                        stage: format!("Downloading {}", artifact.label),
                        downloaded_bytes: bytes,
                        total_bytes: total,
                        attempt,
                    });
                    tick = Instant::now();
                }
            }
            if bytes < 4 || header != b"PK\x03\x04" || total.map(|n| n != bytes).unwrap_or(false) {
                return Err(AppError::user(
                    "The downloaded JAR is incomplete or invalid. Please retry.",
                ));
            }
            if artifact
                .sha1
                .as_ref()
                .map(|s| *s != format!("{:x}", h1.finalize()))
                .unwrap_or(false)
                || artifact
                    .sha256
                    .as_ref()
                    .map(|s| *s != format!("{:x}", h256.finalize()))
                    .unwrap_or(false)
            {
                return Err(AppError::user(
                    "Download checksum verification failed. The file was discarded.",
                ));
            }
            file.flush().await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&part, dest).await?;
            progress(Progress {
                id: id.into(),
                stage: "Download verified".into(),
                downloaded_bytes: bytes,
                total_bytes: Some(bytes),
                attempt,
            });
            Ok(())
        }
        .await;
        match result {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = e;
                let _ = tokio::fs::remove_file(&part).await;
            }
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn serve(body: &'static [u8], requests: usize) -> String {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/artifact", socket.local_addr().unwrap());
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in socket.incoming().take(requests) {
                let mut s = stream.unwrap();
                let mut buf = [0; 4096];
                let _ = s.read(&mut buf);
                write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                s.write_all(body).unwrap();
            }
        });
        url
    }
    #[tokio::test]
    async fn checksum_and_atomic_download() {
        let body = b"PK\x03\x04test fixture payload";
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("server.jar");
        let client = Client::new();
        let cancel = AtomicBool::new(false);
        let artifact = Artifact {
            url: serve(body, 1),
            sha1: Some(format!("{:x}", Sha1::digest(body))),
            sha256: None,
            size: Some(body.len() as u64),
            label: "fixture".into(),
            java: 21,
        };
        download(&client, &artifact, &dest, &cancel, |_| {}, "fixture")
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!dest.with_extension("jar.part").exists());
        let failed = temp.path().join("failed.jar");
        let artifact = Artifact {
            url: serve(body, 3),
            sha1: Some("invalid-checksum".into()),
            ..artifact
        };
        assert!(
            download(&client, &artifact, &failed, &cancel, |_| {}, "fixture")
                .await
                .is_err()
        );
        assert!(!failed.exists());
        assert!(!failed.with_extension("jar.part").exists());
        cancel.store(true, Ordering::Relaxed);
        assert!(
            download(&client, &artifact, &failed, &cancel, |_| {}, "fixture")
                .await
                .is_err()
        );
    }
    #[tokio::test]
    #[ignore = "Read-only upstream network verification; does not download or launch Minecraft"]
    async fn upstream_adapter_resolution() {
        let client = Client::builder()
            .user_agent(concat!("EasyLauncher/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(45))
            .build()
            .unwrap();
        for software in [
            ServerType::Vanilla,
            ServerType::Paper,
            ServerType::Purpur,
            ServerType::Fabric,
        ] {
            let versions = crate::server::software::versions(
                &client,
                software,
                DownloadSource::Official,
                false,
            )
            .await
            .unwrap();
            assert!(versions.iter().any(|v| v.id == "1.21.4"));
            let artifact =
                ServerInstaller::resolve(&client, "1.21.4", software, DownloadSource::Official)
                    .await
                    .unwrap();
            assert!(artifact.url.starts_with("https://"));
            assert_eq!(artifact.java, 21);
            println!("{software:?}: {}", artifact.label);
        }
    }
}
