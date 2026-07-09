use std::path::Path;

use crate::command::{run_command_env, trim_output};
use crate::resolve::{resolve_in_dir, resolve_tool};
use crate::runtime::{is_managed_path, prepended_path};

use super::manage::acquire_install_lock;
use crate::lsp::manage::PIP_INSTALL_LOCK;
use super::{lsp_root, managed_bin_dirs, LspInstallEvent};

/// `pip install` with `--target`, after ensuring Python is available.
pub async fn install_pip(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    pkg: &str,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let _guard = acquire_install_lock(data_dir, language_id, &PIP_INSTALL_LOCK, command, on_event).await;
    if let Some(path) = already_installed(data_dir, command) {
        return Ok(path);
    }

    let python = if let Some(python) =
        resolve_tool(data_dir, "python3").or_else(|| resolve_tool(data_dir, "python"))
    {
        python
    } else {
        on_event(LspInstallEvent::Progress {
            language_id: language_id.to_string(),
            percent: 8,
            step: "Setting up Python".to_string(),
        });
        crate::runtime::ensure_runtime(data_dir, crate::runtime::Runtime::Python, &|e| {
            on_event(LspInstallEvent::from_runtime_event(language_id, &e));
        })
        .await
        .map_err(|e| format!("Python is required and auto-setup failed: {e}"))?;
        resolve_tool(data_dir, "python3")
            .or_else(|| resolve_tool(data_dir, "python"))
            .ok_or_else(|| "Python was set up but is still not resolvable.".to_string())?
    };

    if is_managed_path(data_dir, &python) {
        crate::runtime::ensure_managed_pip(data_dir).await?;
    }

    let root = lsp_root(data_dir);
    let target = root.join("pip");
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| e.to_string())?;

    let args = vec![
        "-m".to_string(),
        "pip".to_string(),
        "install".to_string(),
        "--upgrade".to_string(),
        "--target".to_string(),
        target.to_string_lossy().to_string(),
        pkg.to_string(),
    ];
    let env: Vec<(String, String)> = prepended_path(data_dir).into_iter().collect();
    let step = run_command_env(&python, &args, None, &env).await?;
    if !step.success {
        return Err(trim_output(&step.output, "pip install failed"));
    }
    finalize(data_dir, command)
}

/// `pip uninstall -y <pkg>` + authoritative binary removal.
pub async fn uninstall_pip(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let _guard = acquire_install_lock(data_dir, language_id, &PIP_INSTALL_LOCK, command, on_event).await;

    let recipe = super::recipes::recipe_for(language_id)
        .ok_or_else(|| format!("No install recipe for {language_id}"))?;
    let super::InstallMethod::Pip(pkg) = recipe.method else {
        return Err(format!("{language_id} is not a pip-installed server"));
    };

    if let Some(python) = resolve_tool(data_dir, "python3").or_else(|| resolve_tool(data_dir, "python")) {
        let env: Vec<(String, String)> = prepended_path(data_dir).into_iter().collect();
        let _ = run_command_env(
            &python,
            &[
                "-m".to_string(),
                "pip".to_string(),
                "uninstall".to_string(),
                "-y".to_string(),
                pkg.to_string(),
            ],
            None,
            &env,
        )
        .await;
    }

    let root = lsp_root(data_dir);
    let target = root.join("pip");
    let mut removed = false;
    for dir in [target.join("bin"), target.join("Scripts")] {
        if let Some(path) = resolve_in_dir(&dir, command) {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| format!("Could not remove {command}: {e}"))?;
            removed = true;
        }
    }
    if removed {
        Ok(format!("Uninstalled {command}."))
    } else {
        Err(format!("{command} is not installed in the managed pip directory."))
    }
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
