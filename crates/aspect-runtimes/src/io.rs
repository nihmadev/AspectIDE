use std::path::Path;

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::Integrity;

pub const NODE_INDEX_URL: &str = "https://nodejs.org/dist/index.json";
pub const GO_INDEX_URL: &str = "https://go.dev/dl/?mode=json";
pub const RUSTUP_DIST_BASE: &str = "https://static.rust-lang.org/rustup/dist";
pub const PYTHON_FTP_BASE: &str = "https://www.python.org/ftp/python";
pub const PYTHON_EMBED_VERSION: &str = "3.12.8";
pub const PYTHON_EMBED_SHA256_AMD64: &str =
    "8d3f33be9eb810f23c102f08475af2854e50484b8e4e06275e937be61ce3d2fb";
pub const PYTHON_EMBED_SHA256_ARM64: &str =
    "d34db37675973785a2a539cd1c8dde1b6d45665f48c615ef55274b3798bf9fd3";
pub const GET_PIP_URL: &str = "https://bootstrap.pypa.io/get-pip.py";
pub const PROVISION_TIMEOUT_SECS: u64 = 1_200;
pub const INSTALL_TIMEOUT_SECS: u64 = 900;

pub fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .user_agent("aspect-ide")
        .build()
        .map_err(|e| e.to_string())
}

/// Stream a download to `dest`, emitting coarse progress, and verifying
/// its integrity before the bytes are ever exposed for extraction/exec.
pub async fn download_to_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    integrity: Integrity,
    mut on_progress: impl FnMut(u8) + Send,
) -> Result<(), String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed ({url}): {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download error ({url}): {e}"))?;
    let total = response.content_length();

    let part = dest.with_extension("part");
    let _ = tokio::fs::remove_file(&part).await;

    let result = stream_to_part(response, &part, total, &mut on_progress).await;
    let digest = match result {
        Ok(digest) => digest,
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
    };

    if let Err(why) = integrity.verify(&digest) {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(format!("Refusing to install {url}: {why}"));
    }

    let _ = tokio::fs::remove_file(dest).await;
    tokio::fs::rename(&part, dest).await.map_err(|e| {
        format!(
            "Could not finalize verified download {}: {e}",
            dest.display()
        )
    })
}

async fn stream_to_part(
    response: reqwest::Response,
    part: &Path,
    total: Option<u64>,
    on_progress: &mut (dyn FnMut(u8) + Send),
) -> Result<[u8; 32], String> {
    let mut file = tokio::fs::File::create(part)
        .await
        .map_err(|e| format!("Could not create {}: {e}", part.display()))?;
    let mut hasher = crate::Sha256::new();
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_percent = 0u8;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download interrupted: {e}"))?;
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Write failed: {e}"))?;
        downloaded += chunk.len() as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            let pct = u8::try_from(downloaded.min(total) * 100 / total).unwrap_or(0);
            if pct > last_percent {
                last_percent = pct;
                on_progress(pct);
            }
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(hasher.finalize())
}

/// Fetch a small vendor text resource (a checksum sibling/manifest) as a string.
pub async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Could not fetch checksum ({url}): {e}"))?
        .error_for_status()
        .map_err(|e| format!("Checksum fetch error ({url}): {e}"))?
        .text()
        .await
        .map_err(|e| format!("Malformed checksum response ({url}): {e}"))
}

/// Stream a GitHub release asset to `dest` with unconditional progress callback
/// (0-100). Returns the number of bytes written.
pub async fn download_asset(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    mut on_progress: impl FnMut(u8) + Send,
) -> Result<u64, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed ({url}): {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download error ({url}): {e}"))?;
    let total = response.content_length();

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Could not create {}: {e}", dest.display()))?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_percent = 0u8;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download interrupted: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Write failed: {e}"))?;
        downloaded += chunk.len() as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            let pct = u8::try_from(downloaded.min(total) * 100 / total).unwrap_or(0);
            if pct > last_percent {
                last_percent = pct;
                on_progress(pct);
            }
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(downloaded)
}
