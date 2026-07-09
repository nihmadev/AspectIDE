use std::path::Path;

use crate::command::{run_command_env, trim_output};
use crate::resolve::resolve_tool;
use crate::runtime::{is_managed_path, managed_rust_env};

use super::manage::acquire_install_lock;
use crate::lsp::manage::RUSTUP_INSTALL_LOCK;
use super::{managed_bin_dirs, LspInstallEvent};

/// `rustup component add <name>`. If rustup is not available, auto-provisions
/// the managed Rust toolchain first (which already includes rust-analyzer).
pub async fn install_rustup(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    component: &str,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let _guard = acquire_install_lock(data_dir, language_id, &RUSTUP_INSTALL_LOCK, command, on_event).await;
    if let Some(path) = already_installed(data_dir, command) {
        return Ok(path);
    }

    let rustup = if let Some(rustup) = resolve_tool(data_dir, "rustup") {
        rustup
    } else {
        on_event(LspInstallEvent::Progress {
            language_id: language_id.to_string(),
            percent: 8,
            step: "Setting up Rust toolchain".to_string(),
        });
        let path = crate::runtime::ensure_runtime(data_dir, crate::runtime::Runtime::Rust, &|e| {
            on_event(LspInstallEvent::from_runtime_event(language_id, &e));
        })
        .await
        .map_err(|e| format!("Rust auto-setup failed: {e}"))?;

        if let Some(found) = resolve_tool(data_dir, command) {
            let _ = path;
            return Ok(found.to_string_lossy().to_string());
        }
        resolve_tool(data_dir, "rustup")
            .ok_or_else(|| "Rust was set up but rustup is still not resolvable.".to_string())?
    };

    let env = if is_managed_path(data_dir, &rustup) {
        managed_rust_env(data_dir)
    } else {
        Vec::new()
    };
    let step = run_command_env(
        &rustup,
        &["component".to_string(), "add".to_string(), component.to_string()],
        None,
        &env,
    )
    .await?;
    if !step.success {
        return Err(trim_output(&step.output, "rustup component add failed"));
    }

    resolve_tool(data_dir, command)
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| "rustup reported success but rust-analyzer is not resolvable.".to_string())
}

/// rust-analyzer ships as a rustup component; no standalone uninstall.
pub fn uninstall_rustup(component: &str) -> Result<String, String> {
    Err(format!(
        "{component} ships with the managed Rust toolchain and can't be uninstalled on its own — removing it would require uninstalling the whole managed Rust runtime."
    ))
}

fn already_installed(data_dir: &Path, command: &str) -> Option<String> {
    use crate::resolve::resolve_in_dir;
    managed_bin_dirs(data_dir)
        .iter()
        .find_map(|dir| resolve_in_dir(dir, command))
        .map(|path| path.to_string_lossy().to_string())
}
