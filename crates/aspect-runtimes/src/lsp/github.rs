use std::path::Path;

use crate::archive::{archive_ext, extract_archive};
use crate::fs::single_child_dir;
use crate::io::{download_asset, http_client};
use crate::fs::{remove_dir_tombstoned, replace_runtime_dir, unique_scratch_path};
use crate::resolve::resolve_in_dir;

use super::manage::acquire_install_lock;
use crate::lsp::manage::GH_INSTALL_LOCK;
use crate::platform::{current_gh_arch, current_gh_os};
use super::recipes::{GithubReleaseSpec, no_asset_error};
use super::{lsp_root, managed_bin_dirs, LspInstallEvent};

#[derive(serde::Deserialize)]
struct GhReleaseResponse {
    tag_name: String,
}

async fn resolve_release_tag(
    client: &reqwest::Client,
    repo: &str,
    pinned: Option<&str>,
) -> Result<String, String> {
    if let Some(tag) = pinned {
        return Ok(tag.to_string());
    }
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let release: GhReleaseResponse = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Could not reach the GitHub releases API for {repo}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub releases API error for {repo}: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Malformed GitHub release response for {repo}: {e}"))?;
    Ok(release.tag_name)
}

async fn write_release_manifest(dest: &Path, repo: &str, tag: &str, asset: &str) {
    let manifest = serde_json::json!({ "repo": repo, "tag": tag, "asset": asset });
    if let Ok(bytes) = serde_json::to_vec_pretty(&manifest) {
        let _ = tokio::fs::write(dest.join("manifest.json"), bytes).await;
    }
}

/// Install a `GithubReleaseSpec` server — plain download-and-extract.
pub async fn install_github_release(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    spec: &GithubReleaseSpec,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let _guard = acquire_install_lock(data_dir, language_id, &GH_INSTALL_LOCK, command, on_event).await;
    if let Some(path) = already_installed(data_dir, command) {
        return Ok(path);
    }

    let root = lsp_root(data_dir);
    let gh_root = root.join("gh");
    tokio::fs::create_dir_all(&gh_root)
        .await
        .map_err(|e| e.to_string())?;

    let archive_path = unique_scratch_path(&gh_root, &format!("{command}-download"), ".part");
    let staging = unique_scratch_path(&gh_root, &format!("{command}-staging"), "");

    let result = install_inner(
        data_dir, command, spec, &gh_root, &archive_path, &staging, on_event,
    )
    .await;

    let _ = tokio::fs::remove_file(&archive_path).await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    result
}

async fn install_inner(
    data_dir: &Path,
    command: &str,
    spec: &GithubReleaseSpec,
    gh_root: &Path,
    archive_path: &Path,
    staging: &Path,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let client = http_client()?;
    let tag = resolve_release_tag(&client, spec.repo, spec.version_tag).await?;

    let os = current_gh_os();
    let Some(arch) = current_gh_arch() else {
        return Err(format!(
            "{}: unsupported CPU architecture for a GitHub-release install",
            spec.repo
        ));
    };
    let Some(asset) = (spec.asset_for)(os, arch, &tag) else {
        return Err(no_asset_error(spec.repo, os, arch));
    };
    let url = format!(
        "https://github.com/{}/releases/download/{tag}/{asset}",
        spec.repo
    );

    let downloaded = download_asset(&client, &url, archive_path, |pct| {
        on_event(LspInstallEvent::Progress {
            language_id: command.to_string(),
            percent: (15 + pct * 60 / 100) as u8,
            step: "Downloading".to_string(),
        });
    })
    .await?;
    if downloaded == 0 {
        return Err(format!("Downloaded asset {asset} was empty"));
    }

    let ext = archive_ext(&asset)?;
    let _ = tokio::fs::remove_dir_all(staging).await;
    extract_archive(archive_path, staging, ext)
        .await
        .map_err(|e| {
            format!("Downloaded archive {asset} could not be opened (likely corrupt): {e}")
        })?;

    let inner = single_child_dir(staging)
        .await?
        .unwrap_or_else(|| staging.to_path_buf());
    let dest = gh_root.join(command);
    replace_runtime_dir(&inner, &dest).await?;

    write_release_manifest(&dest, spec.repo, &tag, &asset).await;

    let _ = finalize(data_dir, command)?;
    Ok(dest.to_string_lossy().to_string())
}

/// Delete `<lsp>/gh/<command>/` via tombstone-safe removal.
pub async fn uninstall_github_release(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let _guard = acquire_install_lock(data_dir, language_id, &GH_INSTALL_LOCK, command, on_event).await;
    let root = lsp_root(data_dir);
    let dest = root.join("gh").join(command);
    if tokio::fs::metadata(&dest).await.is_err() {
        return Err(format!("{command} is not installed in the managed directory."));
    }
    remove_dir_tombstoned(&dest).await?;
    Ok(format!("Uninstalled {command}."))
}

fn finalize(data_dir: &Path, command: &str) -> Result<String, String> {
    for dir in managed_bin_dirs(data_dir) {
        if let Some(path) = resolve_in_dir(&dir, command) {
            return Ok(path.to_string_lossy().to_string());
        }
    }
    Err(format!(
        "Install completed but `{command}` was not found in the managed directory."
    ))
}

fn already_installed(data_dir: &Path, command: &str) -> Option<String> {
    managed_bin_dirs(data_dir)
        .iter()
        .find_map(|dir| resolve_in_dir(dir, command))
        .map(|path| path.to_string_lossy().to_string())
}
