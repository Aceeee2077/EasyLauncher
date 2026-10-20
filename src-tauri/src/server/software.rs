use crate::{
    error::{AppError, Result},
    java::version::required_java,
    minecraft::versions::{self, fabric_meta, json, Version},
    models::{DownloadSource, ServerType},
};
use reqwest::Client;
use std::{collections::BTreeSet, future::Future, pin::Pin};
pub struct Artifact {
    pub url: String,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub size: Option<u64>,
    pub label: String,
    pub java: u32,
}
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One adapter per server software. Each adapter declares where its versions
/// and artifacts come from, how they are verified, and which Java major it
/// needs. Adding a software family should not require touching the installer's
/// download or verification code.
pub trait SoftwareAdapter: Sync {
    fn kind(&self) -> ServerType;
    fn supports_snapshots(&self) -> bool {
        false
    }
    fn list<'a>(
        &'a self,
        client: &'a Client,
        source: DownloadSource,
        snapshots: bool,
    ) -> BoxFuture<'a, Result<Vec<Version>>>;
    fn resolve<'a>(
        &'a self,
        client: &'a Client,
        version: &'a str,
        source: DownloadSource,
    ) -> BoxFuture<'a, Result<Artifact>>;
}

fn java_for(software: ServerType, version: &str, details: &serde_json::Value) -> u32 {
    required_java(
        version,
        software,
        details["javaVersion"]["majorVersion"]
            .as_u64()
            .map(|value| value as u32),
    )
}

/// Mojang's manifest goes back much further than the launcher supports. The
/// 1.7 era is the oldest family that still runs from a plain server JAR and
/// uses the world layout this application manages.
fn supported_release(id: &str) -> bool {
    let numbers: Vec<u32> = id.split('.').filter_map(|part| part.parse().ok()).collect();
    numbers.first().copied().unwrap_or(0) >= 26
        || (numbers.first() == Some(&1) && numbers.get(1).copied().unwrap_or(0) >= 7)
}

fn required(value: &serde_json::Value, name: &str) -> Result<String> {
    value.as_str().map(str::to_owned).ok_or_else(|| {
        AppError::user(format!(
            "The upstream service did not return a valid {name}. Try again later."
        ))
    })
}

struct Vanilla;
impl SoftwareAdapter for Vanilla {
    fn kind(&self) -> ServerType {
        ServerType::Vanilla
    }
    fn supports_snapshots(&self) -> bool {
        true
    }
    fn list<'a>(
        &'a self,
        client: &'a Client,
        source: DownloadSource,
        snapshots: bool,
    ) -> BoxFuture<'a, Result<Vec<Version>>> {
        Box::pin(async move {
            let mut versions = versions::manifest_versions(client, source).await?;
            versions.retain(|entry| match entry.kind.as_str() {
                "release" => supported_release(&entry.id),
                "snapshot" => snapshots,
                _ => false,
            });
            Ok(versions)
        })
    }
    fn resolve<'a>(
        &'a self,
        client: &'a Client,
        version: &'a str,
        source: DownloadSource,
    ) -> BoxFuture<'a, Result<Artifact>> {
        Box::pin(async move {
            let details = versions::details(client, version, source).await?;
            let download = &details["downloads"]["server"];
            Ok(Artifact {
                url: required(&download["url"], "Mojang server download")?,
                sha1: download["sha1"].as_str().map(str::to_owned),
                sha256: None,
                size: download["size"].as_u64(),
                label: format!("Vanilla {version}"),
                java: java_for(ServerType::Vanilla, version, &details),
            })
        })
    }
}

struct Paper;
impl SoftwareAdapter for Paper {
    fn kind(&self) -> ServerType {
        ServerType::Paper
    }
    fn list<'a>(
        &'a self,
        client: &'a Client,
        source: DownloadSource,
        _snapshots: bool,
    ) -> BoxFuture<'a, Result<Vec<Version>>> {
        Box::pin(async move {
            let mut versions = versions::manifest_versions(client, source).await?;
            versions.retain(|entry| entry.kind == "release" && supported_release(&entry.id));
            let project = json(client, "https://fill.papermc.io/v3/projects/paper").await?;
            let supported: BTreeSet<&str> = project["versions"]
                .as_object()
                .map(|object| {
                    object
                        .values()
                        .filter_map(|value| value.as_array())
                        .flatten()
                        .filter_map(|value| value.as_str())
                        .collect()
                })
                .unwrap_or_default();
            versions.retain(|entry| supported.contains(entry.id.as_str()));
            Ok(versions)
        })
    }
    fn resolve<'a>(
        &'a self,
        client: &'a Client,
        version: &'a str,
        source: DownloadSource,
    ) -> BoxFuture<'a, Result<Artifact>> {
        Box::pin(async move {
            let details = versions::details(client, version, source).await?;
            let builds = json(
                client,
                &format!("https://fill.papermc.io/v3/projects/paper/versions/{version}/builds"),
            )
            .await?;
            let build = builds
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .filter(|item| item["channel"] == "STABLE")
                        .max_by_key(|item| item["id"].as_u64().unwrap_or(0))
                })
                .ok_or_else(|| {
                    AppError::user(
                        "Paper has no stable build for this release. Choose another version or Vanilla.",
                    )
                })?;
            let download = &build["downloads"]["server:default"];
            Ok(Artifact {
                url: required(&download["url"], "Paper download")?,
                sha1: None,
                sha256: Some(required(
                    &download["checksums"]["sha256"],
                    "Paper checksum",
                )?),
                size: download["size"].as_u64(),
                label: format!("Paper {version}, build {}", build["id"]),
                java: java_for(ServerType::Paper, version, &details),
            })
        })
    }
}

struct Purpur;
impl SoftwareAdapter for Purpur {
    fn kind(&self) -> ServerType {
        ServerType::Purpur
    }
    fn list<'a>(
        &'a self,
        client: &'a Client,
        source: DownloadSource,
        _snapshots: bool,
    ) -> BoxFuture<'a, Result<Vec<Version>>> {
        Box::pin(async move {
            let mut versions = versions::manifest_versions(client, source).await?;
            versions.retain(|entry| entry.kind == "release" && supported_release(&entry.id));
            let project = json(client, "https://api.purpurmc.org/v2/purpur").await?;
            let supported: BTreeSet<&str> = project["versions"]
                .as_array()
                .map(|items| items.iter().filter_map(|item| item.as_str()).collect())
                .unwrap_or_default();
            versions.retain(|entry| supported.contains(entry.id.as_str()));
            Ok(versions)
        })
    }
    fn resolve<'a>(
        &'a self,
        client: &'a Client,
        version: &'a str,
        source: DownloadSource,
    ) -> BoxFuture<'a, Result<Artifact>> {
        Box::pin(async move {
            let details = versions::details(client, version, source).await?;
            let latest = json(
                client,
                &format!("https://api.purpurmc.org/v2/purpur/{version}/latest"),
            )
            .await?;
            if latest["result"].as_str() != Some("SUCCESS") {
                return Err(AppError::user(
                    "Purpur has no successful build for this release. Choose another version.",
                ));
            }
            let build = latest["build"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| latest["build"].as_u64().map(|value| value.to_string()))
                .ok_or_else(|| {
                    AppError::user("Purpur did not report a build number. Try again later.")
                })?;
            Ok(Artifact {
                url: format!("https://api.purpurmc.org/v2/purpur/{version}/latest/download"),
                sha1: None,
                sha256: None,
                size: None,
                label: format!("Purpur {version}, build {build}"),
                java: java_for(ServerType::Purpur, version, &details),
            })
        })
    }
}

struct Fabric;
impl SoftwareAdapter for Fabric {
    fn kind(&self) -> ServerType {
        ServerType::Fabric
    }
    fn supports_snapshots(&self) -> bool {
        true
    }
    fn list<'a>(
        &'a self,
        client: &'a Client,
        source: DownloadSource,
        snapshots: bool,
    ) -> BoxFuture<'a, Result<Vec<Version>>> {
        Box::pin(async move {
            let games = json(client, &format!("{}/v2/versions/game", fabric_meta(source))).await?;
            let supported: BTreeSet<&str> = games
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| snapshots || item["stable"] == true)
                        .filter_map(|item| item["version"].as_str())
                        .collect()
                })
                .unwrap_or_default();
            let mut versions = versions::manifest_versions(client, source).await?;
            versions.retain(|entry| {
                supported.contains(entry.id.as_str())
                    && match entry.kind.as_str() {
                        "release" => supported_release(&entry.id),
                        "snapshot" => snapshots,
                        _ => false,
                    }
            });
            Ok(versions)
        })
    }
    fn resolve<'a>(
        &'a self,
        client: &'a Client,
        version: &'a str,
        source: DownloadSource,
    ) -> BoxFuture<'a, Result<Artifact>> {
        Box::pin(async move {
            let meta = fabric_meta(source);
            let loaders = json(client, &format!("{meta}/v2/versions/loader/{version}")).await?;
            let loader = loaders
                .as_array()
                .and_then(|items| items.iter().find(|item| item["loader"]["stable"] == true))
                .and_then(|item| item["loader"]["version"].as_str())
                .ok_or_else(|| {
                    AppError::user("No stable Fabric loader is available for this version.")
                })?;
            let installers = json(client, &format!("{meta}/v2/versions/installer")).await?;
            let installer = installers
                .as_array()
                .and_then(|items| items.iter().find(|item| item["stable"] == true))
                .and_then(|item| item["version"].as_str())
                .ok_or_else(|| AppError::user("No stable Fabric launcher is available."))?;
            let details = versions::details(client, version, source).await?;
            Ok(Artifact {
                url: format!("{meta}/v2/versions/loader/{version}/{loader}/{installer}/server/jar"),
                sha1: None,
                sha256: None,
                size: None,
                label: format!("Fabric {version}, loader {loader}, launcher {installer}"),
                java: java_for(ServerType::Fabric, version, &details),
            })
        })
    }
}

static VANILLA: Vanilla = Vanilla;
static PAPER: Paper = Paper;
static PURPUR: Purpur = Purpur;
static FABRIC: Fabric = Fabric;

pub fn adapter(software: ServerType) -> &'static dyn SoftwareAdapter {
    match software {
        ServerType::Vanilla => &VANILLA,
        ServerType::Paper => &PAPER,
        ServerType::Purpur => &PURPUR,
        ServerType::Fabric => &FABRIC,
    }
}

pub async fn versions(
    client: &Client,
    software: ServerType,
    source: DownloadSource,
    snapshots: bool,
) -> Result<Vec<Version>> {
    adapter(software).list(client, source, snapshots).await
}

pub async fn resolve(
    client: &Client,
    software: ServerType,
    version: &str,
    source: DownloadSource,
) -> Result<Artifact> {
    adapter(software).resolve(client, version, source).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_older_than_one_sixteen_are_recognized() {
        for id in ["1.7.10", "1.8.9", "1.12.2", "1.16.5", "1.21.4", "26.1"] {
            assert!(supported_release(id), "{id}");
        }
        for id in ["1.2.5", "b1.7.3", "25w45a"] {
            assert!(!supported_release(id), "{id}");
        }
    }

    #[test]
    fn adapters_declare_content_layouts() {
        assert_eq!(ServerType::Vanilla.content_folder(), None);
        assert_eq!(ServerType::Paper.content_folder(), Some("plugins"));
        assert_eq!(ServerType::Purpur.content_folder(), Some("plugins"));
        assert_eq!(ServerType::Fabric.content_folder(), Some("mods"));
        assert_eq!(ServerType::Purpur.content_loader(), Some("paper"));
        assert!(ServerType::Vanilla.supports_snapshots());
        assert!(ServerType::Fabric.supports_snapshots());
        assert!(!ServerType::Purpur.supports_snapshots());
        assert_eq!(ServerType::Purpur.label(), "purpur");
    }
}
