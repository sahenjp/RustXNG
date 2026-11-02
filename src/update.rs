//! Explicit self-update from official GitHub releases. Search TLS settings never
//! weaken this client's certificate validation.
use std::{io::Write, time::Duration};

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const API: &str = "https://api.github.com/repos/sahenjp/RustXNG/releases/latest";
const DOWNLOAD_PREFIX: &str = "https://github.com/sahenjp/RustXNG/releases/download/";
const MAX_BINARY: usize = 256 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn release_version(tag: &str) -> Result<Version, String> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).map_err(|e| e.to_string())
}

fn asset_name(tag: &str) -> String {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    format!("rustxng-{tag}-{}{suffix}", env!("RUSTXNG_BUILD_TARGET"))
}

fn asset_url<'a>(release: &'a Release, name: &str) -> Result<&'a str, String> {
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| format!("release has no asset {name}; build from source instead"))?;
    let expected = format!("{DOWNLOAD_PREFIX}{}/{name}", release.tag_name);
    if asset.browser_download_url != expected {
        return Err(format!("unexpected download URL for {name}"));
    }
    Ok(&asset.browser_download_url)
}

async fn download(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("update download exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn verify(bytes: &[u8], checksum: &str, name: &str) -> Result<(), String> {
    let parts: Vec<_> = checksum.split_whitespace().collect();
    if parts.len() != 2
        || parts[1].trim_start_matches('*') != name
        || parts[0].len() != 64
        || !parts[0].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("invalid release checksum sidecar".into());
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(parts[0]) {
        return Err("SHA-256 mismatch; executable was not changed".into());
    }
    if bytes.is_empty() {
        return Err("empty update executable".into());
    }
    Ok(())
}

/// Check or install the newest stable release without loading server settings.
pub async fn run(install: bool) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .user_agent(concat!("rustxng/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    let metadata = download(&client, API, 4 * 1024 * 1024).await?;
    let release: Release = serde_json::from_slice(&metadata).map_err(|e| e.to_string())?;
    let latest = release_version(&release.tag_name)?;
    if release.draft || release.prerelease || !latest.pre.is_empty() {
        return Err("expected a stable, published release".into());
    }
    let current = release_version(env!("CARGO_PKG_VERSION"))?;
    if latest <= current {
        println!("RustXNG {current} is up to date (latest: {latest}).");
        return Ok(());
    }
    let name = asset_name(&release.tag_name);
    let binary_url = asset_url(&release, &name)?;
    let checksum_url = asset_url(&release, &format!("{name}.sha256"))?;
    println!("RustXNG {current} -> {latest}");
    if !install {
        println!("Run rustxng --update to install.");
        return Ok(());
    }
    let checksum = download(&client, checksum_url, 4096).await?;
    let bytes = download(&client, binary_url, MAX_BINARY).await?;
    verify(
        &bytes,
        std::str::from_utf8(&checksum).map_err(|e| e.to_string())?,
        &name,
    )?;
    let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    self_replace::self_replace(file.path())
        .map_err(|e| format!("could not replace executable: {e}"))?;
    println!("Installed RustXNG {latest}. Settings were preserved. Restart the server to use it.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_rejects_corruption_and_wrong_asset() {
        let name = "rustxng-v1.0.0-test";
        let checksum = format!("{:x}  {name}\n", Sha256::digest(b"binary"));
        assert!(verify(b"binary", &checksum, name).is_ok());
        assert!(verify(b"corrupt", &checksum, name).is_err());
        assert!(verify(b"binary", &checksum, "other").is_err());
        assert!(verify(b"binary", "bad", name).is_err());
    }

    #[test]
    fn versions_are_numeric_and_assets_match_release_workflow() {
        assert!(release_version("v0.26.10").unwrap() > release_version("v0.26.9").unwrap());
        assert!(release_version("../../bad").is_err());
        assert!(asset_name("v1.0.0").contains(env!("RUSTXNG_BUILD_TARGET")));
    }

    #[test]
    fn refuses_nonofficial_asset_url() {
        let mut release = Release {
            tag_name: "v1.0.0".into(),
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                name: "binary".into(),
                browser_download_url: "https://example.org/binary".into(),
            }],
        };
        assert!(asset_url(&release, "binary").is_err());
        release.assets[0].browser_download_url = format!("{DOWNLOAD_PREFIX}v1.0.0/binary");
        assert!(asset_url(&release, "binary").is_ok());
        assert!(asset_url(&release, "missing").is_err());
    }
}
