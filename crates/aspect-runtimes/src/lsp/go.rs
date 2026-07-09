use std::path::Path;

use crate::command::{run_command_env, trim_output};
use crate::resolve::{resolve_in_dir, resolve_tool};
use crate::runtime::{is_managed_path, managed_go_env, prepended_path};

use super::manage::acquire_install_lock;
use crate::lsp::manage::GO_INSTALL_LOCK;
use super::{lsp_root, managed_bin_dirs, LspInstallEvent};

/// `go install <pkg>@latest` with GOBIN set to the managed bin dir.
pub async fn install_go(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    pkg: &str,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let _guard = acquire_install_lock(data_dir, language_id, &GO_INSTALL_LOCK, command, on_event).await;
    if let Some(path) = already_installed(data_dir, command) {
        return Ok(path);
    }

    let go = if let Some(go) = resolve_tool(data_dir, "go") {
        go
    } else {
        on_event(LspInstallEvent::Progress {
            language_id: language_id.to_string(),
            percent: 8,
            step: "Setting up Go toolchain".to_string(),
        });
        crate::runtime::ensure_runtime(data_dir, crate::runtime::Runtime::Go, &|e| {
            on_event(LspInstallEvent::from_runtime_event(language_id, &e));
        })
        .await
        .map_err(|e| format!("Go is required and auto-setup failed: {e}"))?;
        resolve_tool(data_dir, "go")
            .ok_or_else(|| "Go was set up but the `go` command is still not resolvable.".to_string())?
    };

    let root = lsp_root(data_dir);
    let gobin = root.join("go").join("bin");
    tokio::fs::create_dir_all(&gobin)
        .await
        .map_err(|e| e.to_string())?;

    let mut env = vec![
        ("GOBIN".to_string(), gobin.to_string_lossy().to_string()),
        ("GOPATH".to_string(), root.join("go").to_string_lossy().to_string()),
    ];
    if is_managed_path(data_dir, &go) {
        for (key, value) in managed_go_env(data_dir) {
            if key == "GOROOT" {
                env.push((key, value));
            }
        }
        if let Some(path) = prepended_path(data_dir) {
            env.push(path);
        }
    }

    let step = run_command_env(
        &go,
        &["install".to_string(), format!("{pkg}@latest")],
        None,
        &env,
    )
    .await?;
    if !step.success {
        return Err(trim_output(&step.output, "go install failed"));
    }
    finalize(data_dir, command)
}

/// Delete the binary from `<lsp>/go/bin`.
pub async fn uninstall_go(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let _guard = acquire_install_lock(data_dir, language_id, &GO_INSTALL_LOCK, command, on_event).await;
    let root = lsp_root(data_dir);
    let gobin = root.join("go").join("bin");
    let Some(path) = resolve_in_dir(&gobin, command) else {
        return Err(format!("{command} is not installed in the managed Go bin directory."));
    };
    tokio::fs::remove_file(&path)
        .await
        .map_err(|e| format!("Could not remove {command}: {e}"))?;
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
