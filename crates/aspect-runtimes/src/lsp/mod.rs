use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::resolve::{resolve_in_dir, resolve_on_path};
use crate::runtime::RuntimeProvisionEvent;

mod manage;

mod recipes;
pub use recipes::*;

mod npm;
mod pip;
mod go;
mod rustup;
mod github;

pub use npm::*;
pub use pip::*;
pub use go::*;
pub use rustup::*;
pub use github::*;

/// How a given language server is obtained.
#[derive(Debug, Clone, Copy)]
pub enum InstallMethod {
    Npm(&'static str),
    GoInstall(&'static str),
    Pip(&'static str),
    RustupComponent(&'static str),
    GithubRelease(GithubReleaseSpec),
    Manual(&'static str),
}

pub const fn method_label(method: InstallMethod) -> &'static str {
    match method {
        InstallMethod::Npm(_) => "npm",
        InstallMethod::GoInstall(_) => "go",
        InstallMethod::Pip(_) => "pip",
        InstallMethod::RustupComponent(_) => "rustup",
        InstallMethod::GithubRelease(_) => "github",
        InstallMethod::Manual(_) => "manual",
    }
}

// ── Event types ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LspInstallEvent {
    #[serde(rename_all = "camelCase")]
    Started { language_id: String, name: String },
    #[serde(rename_all = "camelCase")]
    Progress {
        language_id: String,
        percent: u8,
        step: String,
    },
    #[serde(rename_all = "camelCase")]
    Finished {
        language_id: String,
        success: bool,
        path: Option<String>,
        error: Option<String>,
    },
}

impl LspInstallEvent {
    /// Convert a runtime provision event into an LSP install event for chaining.
    pub fn from_runtime_event(language_id: &str, event: &RuntimeProvisionEvent) -> Self {
        match event {
            RuntimeProvisionEvent::Started { id: _, name } => Self::Progress {
                language_id: language_id.to_string(),
                percent: 10,
                step: format!("Setting up {name}"),
            },
            RuntimeProvisionEvent::Progress { id: _, percent, step } => Self::Progress {
                language_id: language_id.to_string(),
                percent: percent.saturating_add(10).min(95),
                step: step.clone(),
            },
            RuntimeProvisionEvent::Finished {
                id: _,
                success,
                path: _,
                error,
            } => {
                if *success {
                    Self::Progress {
                        language_id: language_id.to_string(),
                        percent: 95,
                        step: "Runtime ready".to_string(),
                    }
                } else {
                    Self::Progress {
                        language_id: language_id.to_string(),
                        percent: 95,
                        step: error
                            .clone()
                            .unwrap_or_else(|| "Runtime setup failed".to_string()),
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspCatalogEntry {
    pub language_id: String,
    pub name: String,
    pub command: String,
    pub extensions: Vec<String>,
    pub install_method: String,
    pub manual_hint: String,
    pub installed: bool,
    pub path: Option<String>,
    pub managed: bool,
}

// ── Managed layout ──

pub fn lsp_root(data_dir: &Path) -> PathBuf {
    data_dir.join("lsp")
}

pub fn managed_bin_dirs(data_dir: &Path) -> Vec<PathBuf> {
    let root = lsp_root(data_dir);
    let mut dirs = vec![
        root.join("bin"),
        root.join("npm").join("node_modules").join(".bin"),
        root.join("pip").join("bin"),
        root.join("pip").join("Scripts"),
        root.join("go").join("bin"),
    ];
    for recipe in INSTALL_RECIPES {
        if let InstallMethod::GithubRelease(spec) = recipe.method {
            if let Some(command) = command_for(recipe.language_id) {
                let base = root.join("gh").join(command);
                dirs.extend(spec.bin_subdirs.iter().map(|sub| base.join(sub)));
            }
        }
    }
    dirs
}

// ── Catalog ──

pub fn lsp_server_catalog(data_dir: &Path) -> Vec<LspCatalogEntry> {
    let bin_dirs = managed_bin_dirs(data_dir);
    let mut entries = Vec::with_capacity(aspect_lsp::BUILTIN_SERVERS.len());
    for server in aspect_lsp::BUILTIN_SERVERS {
        let recipe = recipe_for(server.language_id);
        let (method_str, manual_hint) = match recipe.map(|r| r.method) {
            Some(InstallMethod::Manual(hint)) => ("manual", hint.to_string()),
            Some(method) => (method_label(method), String::new()),
            None => ("manual", String::new()),
        };
        let managed_hit = bin_dirs
            .iter()
            .find_map(|dir| resolve_in_dir(dir, server.command));
        let resolved = managed_hit
            .clone()
            .or_else(|| resolve_on_path(server.command));
        entries.push(LspCatalogEntry {
            language_id: server.language_id.to_string(),
            name: server.name.to_string(),
            command: server.command.to_string(),
            extensions: server.extensions.iter().map(|e| (*e).to_string()).collect(),
            install_method: method_str.to_string(),
            manual_hint,
            installed: resolved.is_some(),
            path: resolved.as_ref().map(|p| p.to_string_lossy().to_string()),
            managed: managed_hit.is_some(),
        });
    }
    entries
}

// ── Install dispatch ──

async fn run_install(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    method: InstallMethod,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let root = lsp_root(data_dir);
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| format!("Could not create managed LSP dir: {e}"))?;

    match method {
        InstallMethod::Npm(spec) => npm::install_npm(data_dir, language_id, command, &root, spec, on_event).await,
        InstallMethod::GoInstall(pkg) => go::install_go(data_dir, language_id, command, pkg, on_event).await,
        InstallMethod::Pip(pkg) => pip::install_pip(data_dir, language_id, command, pkg, on_event).await,
        InstallMethod::RustupComponent(name) => rustup::install_rustup(data_dir, language_id, command, name, on_event).await,
        InstallMethod::GithubRelease(spec) => github::install_github_release(data_dir, language_id, command, &spec, on_event).await,
        InstallMethod::Manual(hint) => Err(hint.to_string()),
    }
}

async fn run_uninstall(
    data_dir: &Path,
    language_id: &str,
    command: &str,
    method: InstallMethod,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let root = lsp_root(data_dir);
    match method {
        InstallMethod::Npm(spec) => npm::uninstall_npm(data_dir, language_id, command, &root, spec, on_event).await,
        InstallMethod::GoInstall(_) => go::uninstall_go(data_dir, language_id, command, on_event).await,
        InstallMethod::Pip(_) => pip::uninstall_pip(data_dir, language_id, command, on_event).await,
        InstallMethod::GithubRelease(_) => github::uninstall_github_release(data_dir, language_id, command, on_event).await,
        InstallMethod::RustupComponent(name) => rustup::uninstall_rustup(name),
        InstallMethod::Manual(_) => Err("This server has no managed install to uninstall.".to_string()),
    }
}

// ── Public API ──

pub async fn lsp_install_server(
    data_dir: &Path,
    language_id: &str,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let Some(server) = aspect_lsp::BUILTIN_SERVERS
        .iter()
        .find(|s| s.language_id == language_id)
    else {
        return Err(format!("Unknown language server: {language_id}"));
    };
    let Some(recipe) = recipe_for(language_id) else {
        return Err(format!("No install recipe for {language_id}"));
    };

    on_event(LspInstallEvent::Started {
        language_id: language_id.to_string(),
        name: server.name.to_string(),
    });

    let result = run_install(data_dir, language_id, server.command, recipe.method, on_event).await;

    match &result {
        Ok(path) => on_event(LspInstallEvent::Finished {
            language_id: language_id.to_string(),
            success: true,
            path: Some(path.clone()),
            error: None,
        }),
        Err(error) => on_event(LspInstallEvent::Finished {
            language_id: language_id.to_string(),
            success: false,
            path: None,
            error: Some(error.clone()),
        }),
    }
    result
}

pub async fn lsp_uninstall_server(
    data_dir: &Path,
    language_id: &str,
    on_event: &(dyn Fn(LspInstallEvent) + Sync),
) -> Result<String, String> {
    let Some(server) = aspect_lsp::BUILTIN_SERVERS
        .iter()
        .find(|s| s.language_id == language_id)
    else {
        return Err(format!("Unknown language server: {language_id}"));
    };
    let Some(recipe) = recipe_for(language_id) else {
        return Err(format!("No install recipe for {language_id}"));
    };

    let managed_path = managed_bin_dirs(data_dir)
        .iter()
        .find_map(|dir| resolve_in_dir(dir, server.command));
    managed_uninstall_guard(server.name, managed_path, resolve_on_path(server.command))?;

    on_event(LspInstallEvent::Started {
        language_id: language_id.to_string(),
        name: server.name.to_string(),
    });

    let result = run_uninstall(data_dir, language_id, server.command, recipe.method, on_event).await;

    match &result {
        Ok(_) => on_event(LspInstallEvent::Finished {
            language_id: language_id.to_string(),
            success: true,
            path: None,
            error: None,
        }),
        Err(error) => on_event(LspInstallEvent::Finished {
            language_id: language_id.to_string(),
            success: false,
            path: None,
            error: Some(error.clone()),
        }),
    }
    result
}

/// Pure guard behind `lsp_uninstall_server`'s managed-only refusal.
fn managed_uninstall_guard(
    name: &str,
    managed_path: Option<PathBuf>,
    path_fallback: Option<PathBuf>,
) -> Result<(), String> {
    if managed_path.is_some() {
        return Ok(());
    }
    Err(if path_fallback.is_some() {
        format!(
            "{name} resolves from your system PATH, not a AspectIDE-managed install — uninstall it via your package manager instead."
        )
    } else {
        format!("{name} is not installed.")
    })
}

