use std::path::Path;

use crate::command::{run_command_env, trim_output};
use crate::resolve::{resolve_in_dir, resolve_tool};
use crate::runtime::prepended_path;

use super::manage::acquire_install_lock;
use crate::lsp::manage::NPM_INSTALL_LOCK;
use super::recipes::INSTALL_RECIPES;
use super::InstallMethod;
use super::managed_bin_dirs;

/// `npm install` into the shared `<lsp>/npm` prefix, after ensuring Node is
/// available (auto-provisions managed Node if missing).
pub async fn install_npm(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    root: &Path,
    spec: &str,
    on_event: &(dyn Fn(super::LspInstallEvent) + Sync),
) -> Result<String, String> {
    let npm = if let Some(npm) = resolve_tool(data_dir, "npm") {
        npm
    } else {
        on_event(super::LspInstallEvent::Progress {
            language_id: language_id.to_string(),
            percent: 8,
            step: "Setting up Node.js".to_string(),
        });
        crate::runtime::ensure_runtime(data_dir, crate::runtime::Runtime::Node, &|e| {
            on_event(super::LspInstallEvent::from_runtime_event(
                language_id, &e,
            ));
        })
        .await
        .map_err(|e| format!("npm is required and Node.js auto-setup failed: {e}"))?;
        resolve_tool(data_dir, "npm")
            .ok_or_else(|| "Node.js was set up but npm is still not resolvable.".to_string())?
    };
    let prefix = root.join("npm");
    tokio::fs::create_dir_all(&prefix)
        .await
        .map_err(|e| e.to_string())?;
    let _npm_guard = acquire_install_lock(data_dir, language_id, &NPM_INSTALL_LOCK, "npm", on_event).await;

    let mut args = vec![
        "install".to_string(),
        "--prefix".to_string(),
        prefix.to_string_lossy().to_string(),
        "--no-audit".to_string(),
        "--no-fund".to_string(),
        "--loglevel".to_string(),
        "error".to_string(),
    ];
    args.extend(spec.split_whitespace().map(str::to_string));
    let env: Vec<(String, String)> = prepended_path(data_dir).into_iter().collect();
    let step = run_command_env(&npm, &args, None, &env).await?;
    if !step.success {
        return Err(trim_output(&step.output, "npm install failed"));
    }
    finalize(data_dir, command)
}

/// `npm uninstall` from the shared `<lsp>/npm` prefix.
pub async fn uninstall_npm(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    root: &Path,
    spec: &str,
    on_event: &(dyn Fn(super::LspInstallEvent) + Sync),
) -> Result<String, String> {
    let npm = resolve_tool(data_dir, "npm")
        .ok_or_else(|| "npm is not resolvable; cannot uninstall.".to_string())?;
    let prefix = root.join("npm");
    let _guard = acquire_install_lock(data_dir, language_id, &NPM_INSTALL_LOCK, "npm", on_event).await;

    let mut args = vec![
        "uninstall".to_string(),
        "--prefix".to_string(),
        prefix.to_string_lossy().to_string(),
    ];
    args.extend(spec.split_whitespace().map(str::to_string));
    let env: Vec<(String, String)> = prepended_path(data_dir).into_iter().collect();
    let step = run_command_env(&npm, &args, None, &env).await?;
    if !step.success {
        return Err(trim_output(&step.output, "npm uninstall failed"));
    }

    let shared: Vec<&str> = INSTALL_RECIPES
        .iter()
        .filter(|r| r.language_id != language_id)
        .filter_map(|r| match r.method {
            InstallMethod::Npm(other_spec) if other_spec == spec => Some(r.language_id),
            _ => None,
        })
        .collect();
    Ok(if shared.is_empty() {
        format!("Uninstalled {command}.")
    } else {
        format!(
            "Uninstalled {command}. This package is shared with {}, which {} now uninstalled too.",
            shared.join(", "),
            if shared.len() == 1 { "is" } else { "are" }
        )
    })
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
