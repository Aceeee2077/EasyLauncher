use crate::{
    error::{AppError, Result},
    java::version::required_java,
    models::{DownloadSource, ServerType},
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const CHINA_MIRROR: &str = "https://bmclapi2.bangbang93.com";
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Version {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
}
#[derive(Deserialize)]
struct Manifest {
    versions: Vec<Version>,
}
pub fn manifest(source: DownloadSource) -> &'static str {
    match source {
        DownloadSource::Official => MANIFEST,
        DownloadSource::China => "https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json",
    }
}
pub fn fabric_meta(source: DownloadSource) -> &'static str {
    match source {
        DownloadSource::Official => "https://meta.fabricmc.net",
        DownloadSource::China => "https://bmclapi2.bangbang93.com/fabric-meta",
    }
}
/// BMCLAPI mirrors Mojang download hosts; official URLs are returned unchanged.
pub fn mirror_url(url: &str, source: DownloadSource) -> String {
    if source != DownloadSource::China {
        return url.into();
    }
    for prefix in [
        "https://launchermeta.mojang.com/",
        "https://launcher.mojang.com/",
        "https://piston-meta.mojang.com/",
    ] {
        if let Some(rest) = url.strip_prefix(prefix) {
            return format!("{CHINA_MIRROR}/{rest}");
        }
    }
    url.into()
}
/// Version metadata embeds download URLs, so rewrite them before resolving artifacts.
pub fn rewrite_urls(value: &mut Value, source: DownloadSource) {
    if source != DownloadSource::China {
        return;
    }
    match value {
        Value::String(s) => {
            let mirrored = mirror_url(s, source);
            if mirrored != *s {
                *s = mirrored;
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_urls(item, source);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                rewrite_urls(item, source);
            }
        }
        _ => {}
    }
}
pub async fn json(client: &Client, url: &str) -> Result<Value> {
    Ok(client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}
/// Every entry in Mojang's manifest, releases and snapshots alike. Adapters
/// apply their own support filter and rewrite URLs for the selected source.
pub async fn manifest_versions(client: &Client, source: DownloadSource) -> Result<Vec<Version>> {
    let manifest: Manifest = client
        .get(manifest(source))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(manifest
        .versions
        .into_iter()
        .map(|mut v| {
            v.url = mirror_url(&v.url, source);
            v
        })
        .collect())
}
pub async fn details(client: &Client, version: &str, source: DownloadSource) -> Result<Value> {
    let manifest: Manifest = client
        .get(manifest(source))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let v = manifest
        .versions
        .into_iter()
        .find(|v| v.id == version)
        .ok_or_else(|| {
            AppError::user("This Minecraft version could not be found in Mojang's manifest.")
        })?;
    let mut value = json(client, &mirror_url(&v.url, source)).await?;
    rewrite_urls(&mut value, source);
    Ok(value)
}
pub async fn requirement(
    client: &Client,
    version: &str,
    software: ServerType,
    source: DownloadSource,
) -> Result<u32> {
    let d = details(client, version, source).await?;
    Ok(required_java(
        version,
        software,
        d["javaVersion"]["majorVersion"].as_u64().map(|v| v as u32),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn china_mirror_rewrites_only_mojang_hosts() {
        assert_eq!(
            mirror_url(
                "https://launcher.mojang.com/v1/objects/abc/server.jar",
                DownloadSource::China
            ),
            "https://bmclapi2.bangbang93.com/v1/objects/abc/server.jar"
        );
        assert_eq!(
            mirror_url(
                "https://piston-meta.mojang.com/v1/packages/abc/1.21.4.json",
                DownloadSource::China
            ),
            "https://bmclapi2.bangbang93.com/v1/packages/abc/1.21.4.json"
        );
        assert_eq!(
            mirror_url(
                "https://launchermeta.mojang.com/v1/packages/abc/1.21.4.json",
                DownloadSource::Official
            ),
            "https://launchermeta.mojang.com/v1/packages/abc/1.21.4.json"
        );
        assert_eq!(
            mirror_url(
                "https://fill.papermc.io/v3/projects/paper",
                DownloadSource::China
            ),
            "https://fill.papermc.io/v3/projects/paper"
        );
    }
    #[test]
    fn rewrite_urls_walks_nested_metadata() {
        let mut v: Value = serde_json::from_str(
            r#"{"downloads":{"server":{"url":"https://launcher.mojang.com/x/server.jar"}},"list":["https://piston-meta.mojang.com/y.json"]}"#,
        )
        .unwrap();
        rewrite_urls(&mut v, DownloadSource::China);
        assert_eq!(
            v["downloads"]["server"]["url"],
            "https://bmclapi2.bangbang93.com/x/server.jar"
        );
        assert_eq!(v["list"][0], "https://bmclapi2.bangbang93.com/y.json");
    }
}
