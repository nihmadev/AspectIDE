//! Managed install of popular language servers (VS Code / Zed style).
//!
//! Servers are NOT bundled in the installer (rust-analyzer / gopls / clangd are
//! hundreds of MB combined and platform-specific). Instead the IDE installs them
//! on demand into a managed directory under the app data dir and resolves them
//! from there — so `lux-lsp` discovery reports them Available without the user
//! ever touching PATH. Progress streams to the UI on `lux://lsp-install`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const INSTALL_EVENT: &str = "lux://lsp-install";
const INSTALL_TIMEOUT_SECS: u64 = 900;

/// Serializes npm-based installs. They all share one install prefix
/// (`<root>/npm`), so concurrent `npm install`s race on shared transitive deps
/// (vscode-uri, vscode-jsonrpc, vscode-nls, …) and fail with ENOTEMPTY /
/// "file in use" on Windows. Non-npm methods (go/rustup/pip) use distinct dirs
/// and stay parallel, so the lock is scoped to npm only.
static NPM_INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Per-target install locks for the non-npm methods. Each writes a distinct shared
/// directory/toolchain that a concurrent run of the SAME method would corrupt
/// (file-in-use / ENOTEMPTY / partial binary): `install_go` → `<lsp>/go/bin`,
/// `install_pip` → `<lsp>/pip`, `install_rustup` → the shared rustup toolchain.
/// Distinct methods still run in parallel (separate locks); only same-method
/// concurrency (double-click, two Settings panels, AI-driven retries) is serialized.
static GO_INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PIP_INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static RUSTUP_INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Serializes GitHub-release installs (lua-language-server, clangd). They each own
/// a distinct `<lsp>/gh/<command>/` directory, but share one lock (like the simpler
/// `NPM_INSTALL_LOCK`) rather than per-target locks — there are only two of them and
/// both stage-then-swap into place, so the extra parallelism a per-target lock would
/// buy is not worth another static.
static GH_INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How a given language server is obtained. Each variant maps to a concrete
/// package-manager invocation in `install_server`.
// No `PartialEq`/`Eq`: `GithubRelease` carries a `fn` pointer (`GithubReleaseSpec::
// asset_for`), and rustc's `unpredictable_function_pointer_comparisons` lint flags a
// derived `==` over one for good reason (addresses aren't guaranteed unique after
// inlining/dedup). Nothing compares `InstallMethod` values as a whole; the one place
// that compares recipes (`uninstall_npm`'s shared-package detection) matches down to
// the `&str` spec instead.
#[derive(Debug, Clone, Copy)]
pub enum InstallMethod {
    /// `npm install <pkg>` into the managed prefix; binary lands in node bin dir.
    Npm(&'static str),
    /// `go install <pkg>@latest` with GOBIN set to the managed bin dir.
    GoInstall(&'static str),
    /// `pip install <pkg>` with `--target`; console script lands in the bin dir.
    /// Used by `ty` (Astral's Python language server), distributed as a prebuilt
    /// wheel — no Rust/C build needed, just a managed Python to host pip.
    Pip(&'static str),
    /// `rustup component add <name>` — installs into the active rust toolchain.
    RustupComponent(&'static str),
    /// Download a prebuilt binary directly from a GitHub Releases page — no host
    /// toolchain required. Used for lua-language-server and clangd, whose upstreams
    /// publish per-platform archives with no npm/pip/go packaging at all.
    GithubRelease(GithubReleaseSpec),
    /// No automated installer (toolchain-specific); UI shows a manual hint. Currently
    /// unused — lua/clangd (the last two recipes that needed it) moved to
    /// `GithubRelease` above — but kept as the escape hatch for a future language
    /// server with no automatable install path; `method_label`/the catalog/uninstall
    /// dispatch all still handle it.
    #[allow(dead_code)]
    Manual(&'static str),
}

/// Host OS bucket for selecting a GitHub release asset. Deliberately coarser than
/// `std::env::consts::OS` — every upstream here only ships windows/linux/macos builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhOs {
    Windows,
    Linux,
    Macos,
}

/// CPU-architecture bucket for selecting a GitHub release asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhArch {
    X64,
    Arm64,
}

const fn current_gh_os() -> GhOs {
    if cfg!(windows) {
        GhOs::Windows
    } else if cfg!(target_os = "macos") {
        GhOs::Macos
    } else {
        GhOs::Linux
    }
}

fn current_gh_arch() -> Option<GhArch> {
    match std::env::consts::ARCH {
        "x86_64" => Some(GhArch::X64),
        "aarch64" => Some(GhArch::Arm64),
        _ => None,
    }
}

/// A managed install sourced directly from a GitHub Releases page (no host
/// toolchain, no package manager). `asset_for` picks the release asset name for the
/// resolved OS/arch/version (`None` when upstream ships no build for that platform);
/// `bin_subdirs` are the directory/directories — relative to the extracted, top-level-
/// stripped release tree — that hold the resolvable executable. Directories (not exact
/// file paths) because resolution reuses `managed_bin_dirs`/`resolve_in_dir`, which
/// already knows how to find a named command inside a directory (Windows extensions
/// included) — a separate exact-path lookup would just duplicate that.
#[derive(Debug, Clone, Copy)]
pub struct GithubReleaseSpec {
    /// `"<owner>/<repo>"` for the GitHub releases API and download URLs.
    pub repo: &'static str,
    /// Pin to a specific release tag; `None` resolves `/releases/latest`.
    pub version_tag: Option<&'static str>,
    /// Selects the release asset file name; `None` means no build for this platform.
    pub asset_for: fn(GhOs, GhArch, &str) -> Option<String>,
    pub bin_subdirs: &'static [&'static str],
}

fn lua_language_server_asset(os: GhOs, arch: GhArch, version: &str) -> Option<String> {
    let v = version.trim_start_matches('v');
    let name = match (os, arch) {
        (GhOs::Windows, GhArch::X64) => format!("lua-language-server-{v}-win32-x64.zip"),
        (GhOs::Linux, GhArch::X64) => format!("lua-language-server-{v}-linux-x64.tar.gz"),
        (GhOs::Linux, GhArch::Arm64) => format!("lua-language-server-{v}-linux-arm64.tar.gz"),
        (GhOs::Macos, GhArch::X64) => format!("lua-language-server-{v}-darwin-x64.tar.gz"),
        (GhOs::Macos, GhArch::Arm64) => format!("lua-language-server-{v}-darwin-arm64.tar.gz"),
        // LuaLS ships no win32-arm64 build.
        (GhOs::Windows, GhArch::Arm64) => return None,
    };
    Some(name)
}

fn clangd_asset(os: GhOs, arch: GhArch, version: &str) -> Option<String> {
    let v = version.trim_start_matches('v');
    match (os, arch) {
        (GhOs::Windows, GhArch::X64) => Some(format!("clangd-windows-{v}.zip")),
        (GhOs::Linux, GhArch::X64) => Some(format!("clangd-linux-{v}.zip")),
        // clangd's mac build is a universal (x86_64 + arm64) binary — one asset covers both.
        (GhOs::Macos, _) => Some(format!("clangd-mac-{v}.zip")),
        // No windows-arm64 or linux-arm64 build is published.
        (GhOs::Windows | GhOs::Linux, GhArch::Arm64) => None,
    }
}

const LUA_LANGUAGE_SERVER_RELEASE: GithubReleaseSpec = GithubReleaseSpec {
    repo: "LuaLS/lua-language-server",
    version_tag: None,
    asset_for: lua_language_server_asset,
    bin_subdirs: &["bin"],
};

const CLANGD_RELEASE: GithubReleaseSpec = GithubReleaseSpec {
    repo: "clangd/clangd",
    version_tag: None,
    asset_for: clangd_asset,
    bin_subdirs: &["bin"],
};

/// Install recipe for one catalog server, keyed by `language_id` (matches the
/// `lux-lsp` `BUILTIN_SERVERS` catalog so discovery + install stay in lockstep).
#[derive(Debug, Clone, Copy)]
pub struct InstallRecipe {
    pub language_id: &'static str,
    pub method: InstallMethod,
}

/// Install recipes for the popular-language catalog. Order is the display order
/// in Settings; npm-based servers first (fast, cross-platform), then toolchain
/// ones. Every `language_id` here MUST exist in `lux_lsp::BUILTIN_SERVERS`.
pub const INSTALL_RECIPES: &[InstallRecipe] = &[
    InstallRecipe {
        language_id: "typescript",
        method: InstallMethod::Npm("typescript-language-server typescript"),
    },
    InstallRecipe {
        language_id: "python",
        method: InstallMethod::Pip("ty"),
    },
    InstallRecipe {
        language_id: "json",
        method: InstallMethod::Npm("vscode-langservers-extracted"),
    },
    InstallRecipe {
        language_id: "html",
        method: InstallMethod::Npm("vscode-langservers-extracted"),
    },
    InstallRecipe {
        language_id: "css",
        method: InstallMethod::Npm("vscode-langservers-extracted"),
    },
    InstallRecipe {
        language_id: "yaml",
        method: InstallMethod::Npm("yaml-language-server"),
    },
    InstallRecipe {
        language_id: "bash",
        method: InstallMethod::Npm("bash-language-server"),
    },
    InstallRecipe {
        language_id: "go",
        method: InstallMethod::GoInstall("golang.org/x/tools/gopls"),
    },
    InstallRecipe {
        language_id: "rust",
        method: InstallMethod::RustupComponent("rust-analyzer"),
    },
    InstallRecipe {
        language_id: "lua",
        method: InstallMethod::GithubRelease(LUA_LANGUAGE_SERVER_RELEASE),
    },
    InstallRecipe {
        language_id: "cpp",
        method: InstallMethod::GithubRelease(CLANGD_RELEASE),
    },
];

#[must_use]
pub fn recipe_for(language_id: &str) -> Option<&'static InstallRecipe> {
    INSTALL_RECIPES
        .iter()
        .find(|recipe| recipe.language_id == language_id)
}

/// The managed LSP root: `<app_data>/lsp`. `bin/` holds resolvable executables,
/// `npm/` is the npm install prefix, `pip/` the pip --target, `go/` GOPATH.
pub fn managed_root(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join("lsp"))
}

pub fn managed_bin_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let Ok(root) = managed_root(app) else {
        return Vec::new();
    };
    // Search every place an install method can drop an executable.
    let mut dirs = vec![
        root.join("bin"),
        root.join("npm").join("node_modules").join(".bin"),
        root.join("pip").join("bin"),
        root.join("pip").join("Scripts"),
        root.join("go").join("bin"),
    ];
    // GithubRelease servers keep their extracted tree intact under `<lsp>/gh/<command>/`
    // instead of a copied/symlinked binary (LuaLS needs its `main.lua`/`meta` beside the
    // exe; clangd needs its bundled clang resource headers) — so their bin dir(s) are
    // searched here rather than landing in the flat `bin/` above. Cheap: a handful of
    // extra `PathBuf`s: existence is checked downstream, by whoever resolves a command
    // in each dir, not here.
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

/// The catalog `BuiltinServer.command` for a `language_id`, or `None` if unknown.
fn command_for(language_id: &str) -> Option<&'static str> {
    lux_lsp::BUILTIN_SERVERS
        .iter()
        .find(|s| s.language_id == language_id)
        .map(|s| s.command)
}

// ── Status / catalog (Rust → UI) ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspCatalogEntry {
    pub language_id: String,
    pub name: String,
    pub command: String,
    pub extensions: Vec<String>,
    /// How it installs: "npm" | "go" | "pip" | "rustup" | "github" | "manual".
    pub install_method: String,
    /// Manual-install guidance (non-empty only for the "manual" method).
    pub manual_hint: String,
    /// True when the server resolves in the managed dir or PATH right now.
    pub installed: bool,
    /// Resolved absolute path when installed, else null.
    pub path: Option<String>,
    /// True when found specifically in the managed dir (vs. user's PATH).
    pub managed: bool,
}

const fn method_label(method: InstallMethod) -> &'static str {
    match method {
        InstallMethod::Npm(_) => "npm",
        InstallMethod::GoInstall(_) => "go",
        InstallMethod::Pip(_) => "pip",
        InstallMethod::RustupComponent(_) => "rustup",
        InstallMethod::GithubRelease(_) => "github",
        InstallMethod::Manual(_) => "manual",
    }
}

/// Full catalog with live installed-state for the Settings UI. Pure filesystem +
/// PATH probing — never launches a server, so it is cheap and safe to poll.
#[tauri::command]
// Tauri command: the Result is kept for IPC error-channel symmetry with the rest.
#[allow(clippy::unnecessary_wraps)]
pub fn lsp_server_catalog(app: AppHandle) -> Result<Vec<LspCatalogEntry>, String> {
    let bin_dirs = managed_bin_dirs(&app);
    let mut entries = Vec::with_capacity(lux_lsp::BUILTIN_SERVERS.len());
    for server in lux_lsp::BUILTIN_SERVERS {
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
    Ok(entries)
}

// ── Install progress events (Rust → UI) ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LspInstallEvent {
    /// Install started for a language.
    #[serde(rename_all = "camelCase")]
    Started { language_id: String, name: String },
    /// Coarse progress (0–100) plus a human step label. Package managers don't
    /// report fine-grained progress, so this is staged (resolve→download→link).
    #[serde(rename_all = "camelCase")]
    Progress {
        language_id: String,
        percent: u8,
        step: String,
    },
    /// Finished: success with resolved path, or failure with an error message.
    #[serde(rename_all = "camelCase")]
    Finished {
        language_id: String,
        success: bool,
        path: Option<String>,
        error: Option<String>,
    },
}

fn emit_install(app: &AppHandle, event: &LspInstallEvent) {
    let _ = app.emit(INSTALL_EVENT, event);
}

fn progress(app: &AppHandle, language_id: &str, percent: u8, step: &str) {
    emit_install(
        app,
        &LspInstallEvent::Progress {
            language_id: language_id.to_string(),
            percent,
            step: step.to_string(),
        },
    );
}

/// Install (or reinstall) the language server for `language_id` into the managed
/// directory, streaming progress on `lux://lsp-install`. Returns the resolved
/// executable path on success. Idempotent: re-running upgrades/repairs.
#[tauri::command]
pub async fn lsp_install_server(app: AppHandle, language_id: String) -> Result<String, String> {
    let Some(server) = lux_lsp::BUILTIN_SERVERS
        .iter()
        .find(|s| s.language_id == language_id)
    else {
        return Err(format!("Unknown language server: {language_id}"));
    };
    let Some(recipe) = recipe_for(&language_id) else {
        return Err(format!("No install recipe for {language_id}"));
    };

    emit_install(
        &app,
        &LspInstallEvent::Started {
            language_id: language_id.clone(),
            name: server.name.to_string(),
        },
    );
    progress(&app, &language_id, 5, "Preparing");

    let result = run_install(&app, &language_id, server.command, recipe.method).await;

    match &result {
        Ok(path) => emit_install(
            &app,
            &LspInstallEvent::Finished {
                language_id: language_id.clone(),
                success: true,
                path: Some(path.clone()),
                error: None,
            },
        ),
        Err(error) => emit_install(
            &app,
            &LspInstallEvent::Finished {
                language_id: language_id.clone(),
                success: false,
                path: None,
                error: Some(error.clone()),
            },
        ),
    }
    result
}

/// Remove a managed install for `language_id`, streaming progress on the same
/// `lux://lsp-install` channel as install (with a distinguishable "Uninstalling"
/// step) so the Settings store refreshes the same way it does after an install.
/// Managed-only: refuses with a clear error for anything that resolves only on the
/// system PATH (or isn't installed at all) — a system install is never touched.
#[tauri::command]
pub async fn lsp_uninstall_server(app: AppHandle, language_id: String) -> Result<String, String> {
    let Some(server) = lux_lsp::BUILTIN_SERVERS
        .iter()
        .find(|s| s.language_id == language_id)
    else {
        return Err(format!("Unknown language server: {language_id}"));
    };
    let Some(recipe) = recipe_for(&language_id) else {
        return Err(format!("No install recipe for {language_id}"));
    };

    let managed_path = managed_bin_dirs(&app)
        .iter()
        .find_map(|dir| resolve_in_dir(dir, server.command));
    managed_uninstall_guard(server.name, managed_path, resolve_on_path(server.command))?;

    emit_install(
        &app,
        &LspInstallEvent::Started {
            language_id: language_id.clone(),
            name: server.name.to_string(),
        },
    );
    progress(&app, &language_id, 20, "Uninstalling");

    let result = run_uninstall(&app, &language_id, server.command, recipe.method).await;

    match &result {
        Ok(_) => emit_install(
            &app,
            &LspInstallEvent::Finished {
                language_id: language_id.clone(),
                success: true,
                path: None,
                error: None,
            },
        ),
        Err(error) => emit_install(
            &app,
            &LspInstallEvent::Finished {
                language_id: language_id.clone(),
                success: false,
                path: None,
                error: Some(error.clone()),
            },
        ),
    }
    result
}

/// Pure guard behind `lsp_uninstall_server`'s managed-only refusal, factored out so it
/// is testable without an `AppHandle`. `Err` carries the exact message the command
/// returns; `Ok(())` means the server has a managed install and uninstall may proceed.
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
            "{name} resolves from your system PATH, not a Lux-managed install — uninstall it via your package manager instead."
        )
    } else {
        format!("{name} is not installed.")
    })
}

async fn run_uninstall(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    method: InstallMethod,
) -> Result<String, String> {
    let root = managed_root(app)?;
    match method {
        InstallMethod::Npm(spec) => uninstall_npm(app, language_id, command, &root, spec).await,
        InstallMethod::GoInstall(_) => uninstall_go(app, language_id, command, &root).await,
        InstallMethod::Pip(_) => uninstall_pip(app, language_id, command, &root).await,
        InstallMethod::GithubRelease(_) => {
            uninstall_github_release(app, language_id, command, &root).await
        }
        InstallMethod::RustupComponent(name) => uninstall_rustup(name),
        InstallMethod::Manual(_) => {
            Err("This server has no managed install to uninstall.".to_string())
        }
    }
}

/// `npm uninstall` from the shared `<lsp>/npm` prefix. `vscode-langservers-extracted`
/// backs three catalog entries (json/html/css) with one package — uninstalling it
/// uninstalls all three, so the response names the others affected.
async fn uninstall_npm(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    root: &Path,
    spec: &str,
) -> Result<String, String> {
    let npm = resolve_tool(app, "npm")
        .ok_or_else(|| "npm is not resolvable; cannot uninstall.".to_string())?;
    let prefix = root.join("npm");
    let _guard = acquire_install_lock(app, language_id, &NPM_INSTALL_LOCK, "npm").await;
    progress(app, language_id, 50, "Uninstalling");
    let mut args = vec![
        "uninstall".to_string(),
        "--prefix".to_string(),
        prefix.to_string_lossy().to_string(),
    ];
    args.extend(spec.split_whitespace().map(str::to_string));
    let env: Vec<(String, String)> = crate::runtime_provision::prepended_path(app)
        .into_iter()
        .collect();
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

/// Delete the `gopls` binary from `<lsp>/go/bin` — the managed Go SDK itself is left
/// alone (it may back other tooling), only the LSP install is removed.
async fn uninstall_go(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    root: &Path,
) -> Result<String, String> {
    let _guard = acquire_install_lock(app, language_id, &GO_INSTALL_LOCK, command).await;
    let gobin = root.join("go").join("bin");
    let Some(path) = resolve_in_dir(&gobin, command) else {
        return Err(format!(
            "{command} is not installed in the managed Go bin directory."
        ));
    };
    tokio::fs::remove_file(&path)
        .await
        .map_err(|e| format!("Could not remove {command}: {e}"))?;
    Ok(format!("Uninstalled {command}."))
}

/// `pip uninstall -y <pkg>` best-effort (a `--target` install isn't tracked in the
/// interpreter's own registry, so pip may report "not installed" and do nothing),
/// then authoritatively delete the console script(s) pip dropped into the managed
/// target's bin dirs — that's what `resolve_in_dir`/discovery actually looks at.
async fn uninstall_pip(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    root: &Path,
) -> Result<String, String> {
    let _guard = acquire_install_lock(app, language_id, &PIP_INSTALL_LOCK, command).await;
    let Some(recipe) = recipe_for(language_id) else {
        return Err(format!("No install recipe for {language_id}"));
    };
    let InstallMethod::Pip(pkg) = recipe.method else {
        return Err(format!("{language_id} is not a pip-installed server"));
    };
    if let Some(python) = resolve_tool(app, "python3").or_else(|| resolve_tool(app, "python")) {
        let env: Vec<(String, String)> = crate::runtime_provision::prepended_path(app)
            .into_iter()
            .collect();
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
        Err(format!(
            "{command} is not installed in the managed pip directory."
        ))
    }
}

/// Delete `<lsp>/gh/<command>/` (the whole extracted release tree) via the same
/// tombstone-safe removal `runtime_provision` uses when replacing a runtime dir.
async fn uninstall_github_release(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    root: &Path,
) -> Result<String, String> {
    let _guard = acquire_install_lock(app, language_id, &GH_INSTALL_LOCK, command).await;
    let dest = root.join("gh").join(command);
    if tokio::fs::metadata(&dest).await.is_err() {
        return Err(format!(
            "{command} is not installed in the managed directory."
        ));
    }
    crate::runtime_provision::remove_dir_tombstoned(&dest).await?;
    Ok(format!("Uninstalled {command}."))
}

/// rust-analyzer ships as a rustup component of the managed Rust toolchain — it has no
/// standalone uninstall; removing it means removing the whole managed Rust runtime.
fn uninstall_rustup(component: &str) -> Result<String, String> {
    Err(format!(
        "{component} ships with the managed Rust toolchain and can't be uninstalled on its own — removing it would require uninstalling the whole managed Rust runtime."
    ))
}

async fn run_install(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    method: InstallMethod,
) -> Result<String, String> {
    let root = managed_root(app)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| format!("Could not create managed LSP dir: {e}"))?;

    match method {
        InstallMethod::Npm(spec) => install_npm(app, language_id, command, &root, spec).await,
        InstallMethod::GoInstall(pkg) => install_go(app, language_id, command, &root, pkg).await,
        InstallMethod::Pip(pkg) => install_pip(app, language_id, command, &root, pkg).await,
        InstallMethod::RustupComponent(name) => {
            install_rustup(app, language_id, command, name).await
        }
        InstallMethod::GithubRelease(spec) => {
            install_github_release(app, language_id, command, &root, &spec).await
        }
        InstallMethod::Manual(hint) => Err(hint.to_string()),
    }
}

/// Acquire a per-target install lock, surfacing a "waiting" progress step if it is
/// already held so the UI explains the pause instead of looking stuck.
async fn acquire_install_lock<'lock>(
    app: &AppHandle,
    language_id: &str,
    lock: &'lock tokio::sync::Mutex<()>,
    command: &str,
) -> tokio::sync::MutexGuard<'lock, ()> {
    if let Ok(guard) = lock.try_lock() {
        return guard;
    }
    progress(
        app,
        language_id,
        15,
        &format!("Waiting for another {command} install to finish"),
    );
    lock.lock().await
}

/// If `command` already resolves in the managed dir, return its path — used after
/// acquiring an install lock to skip redundant work a concurrent install just did.
fn already_installed(app: &AppHandle, command: &str) -> Option<String> {
    managed_bin_dirs(app)
        .iter()
        .find_map(|dir| resolve_in_dir(dir, command))
        .map(|path| path.to_string_lossy().to_string())
}

async fn install_npm(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    root: &Path,
    spec: &str,
) -> Result<String, String> {
    // Bring up managed Node on a machine with no system npm, then resolve it.
    let npm = if let Some(npm) = resolve_tool(app, "npm") {
        npm
    } else {
        progress(app, language_id, 8, "Setting up Node.js");
        crate::runtime_provision::ensure_runtime(app, crate::runtime_provision::Runtime::Node)
            .await
            .map_err(|e| format!("npm is required and Node.js auto-setup failed: {e}"))?;
        resolve_tool(app, "npm")
            .ok_or_else(|| "Node.js was set up but npm is still not resolvable.".to_string())?
    };
    let prefix = root.join("npm");
    tokio::fs::create_dir_all(&prefix)
        .await
        .map_err(|e| e.to_string())?;
    // Serialize concurrent npm installs into the shared prefix (see NPM_INSTALL_LOCK).
    // Held across `npm install` + `finalize` so no two npm processes ever overlap.
    let _npm_guard = acquire_install_lock(app, language_id, &NPM_INSTALL_LOCK, "npm").await;
    progress(app, language_id, 25, "Downloading via npm");
    // `--prefix` installs into <prefix>/node_modules and <prefix>/node_modules/.bin.
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
    // Prepend managed runtime bins so npm's own node resolves to the managed one.
    let env: Vec<(String, String)> = crate::runtime_provision::prepended_path(app)
        .into_iter()
        .collect();
    let step = run_command_env(&npm, &args, None, &env).await?;
    if !step.success {
        return Err(trim_output(&step.output, "npm install failed"));
    }
    progress(app, language_id, 90, "Linking");
    finalize(app, command)
}

async fn install_go(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    root: &Path,
    pkg: &str,
) -> Result<String, String> {
    // Serialize same-target installs into `<lsp>/go/bin`; if a queued duplicate
    // already produced the binary, return it without re-running `go install`.
    let _guard = acquire_install_lock(app, language_id, &GO_INSTALL_LOCK, command).await;
    if let Some(path) = already_installed(app, command) {
        return Ok(path);
    }
    // On a clean machine, provision the managed Go SDK first, then resolve it.
    let go = if let Some(go) = resolve_tool(app, "go") {
        go
    } else {
        progress(app, language_id, 8, "Setting up Go toolchain");
        crate::runtime_provision::ensure_runtime(app, crate::runtime_provision::Runtime::Go)
            .await
            .map_err(|e| format!("Go is required and auto-setup failed: {e}"))?;
        resolve_tool(app, "go").ok_or_else(|| {
            "Go was set up but the `go` command is still not resolvable.".to_string()
        })?
    };
    // gopls must land where LSP discovery looks (`<lsp>/go/bin`), so GOBIN/GOPATH
    // point there regardless of which `go` we used.
    let gobin = root.join("go").join("bin");
    tokio::fs::create_dir_all(&gobin)
        .await
        .map_err(|e| e.to_string())?;
    progress(app, language_id, 25, "Building via go install");
    let mut env = vec![
        ("GOBIN".to_string(), gobin.to_string_lossy().to_string()),
        (
            "GOPATH".to_string(),
            root.join("go").to_string_lossy().to_string(),
        ),
    ];
    // A managed `go` needs its GOROOT (and runtime bins on PATH) to find its stdlib;
    // a system `go` already knows its own.
    if crate::runtime_provision::is_managed_path(app, &go) {
        for (key, value) in crate::runtime_provision::managed_go_env(app) {
            // Keep our GOBIN/GOPATH (above); only add GOROOT from the managed env.
            if key == "GOROOT" {
                env.push((key, value));
            }
        }
        if let Some(path) = crate::runtime_provision::prepended_path(app) {
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
    progress(app, language_id, 90, "Linking");
    finalize(app, command)
}

async fn install_pip(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    root: &Path,
    pkg: &str,
) -> Result<String, String> {
    // Serialize same-target installs into `<lsp>/pip`; a queued duplicate that
    // already produced the console script short-circuits before re-running pip.
    let _guard = acquire_install_lock(app, language_id, &PIP_INSTALL_LOCK, command).await;
    if let Some(path) = already_installed(app, command) {
        return Ok(path);
    }
    // Bring up managed Python on a machine with no system Python, then resolve it.
    // Mirrors the npm/go/rustup installers so a clean box can install ty unaided.
    let python = if let Some(python) =
        resolve_tool(app, "python3").or_else(|| resolve_tool(app, "python"))
    {
        python
    } else {
        progress(app, language_id, 8, "Setting up Python");
        crate::runtime_provision::ensure_runtime(app, crate::runtime_provision::Runtime::Python)
            .await
            .map_err(|e| format!("Python is required and auto-setup failed: {e}"))?;
        resolve_tool(app, "python3")
            .or_else(|| resolve_tool(app, "python"))
            .ok_or_else(|| "Python was set up but is still not resolvable.".to_string())?
    };
    // Self-repair: a managed Python provisioned before pip was a hard requirement (or
    // whose bootstrap was skipped) may have no pip. Ensure it before the pip install
    // so `ty` doesn't fail with an unrepairable "No module named pip".
    if crate::runtime_provision::is_managed_path(app, &python) {
        crate::runtime_provision::ensure_managed_pip(app).await?;
    }
    let target = root.join("pip");
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| e.to_string())?;
    progress(app, language_id, 25, "Downloading via pip");
    let args = vec![
        "-m".to_string(),
        "pip".to_string(),
        "install".to_string(),
        "--upgrade".to_string(),
        "--target".to_string(),
        target.to_string_lossy().to_string(),
        pkg.to_string(),
    ];
    // Prepend managed runtime bins so a managed Python finds its own pip/scripts.
    let env: Vec<(String, String)> = crate::runtime_provision::prepended_path(app)
        .into_iter()
        .collect();
    let step = run_command_env(&python, &args, None, &env).await?;
    if !step.success {
        return Err(trim_output(&step.output, "pip install failed"));
    }
    progress(app, language_id, 90, "Linking");
    finalize(app, command)
}

async fn install_rustup(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    component: &str,
) -> Result<String, String> {
    // Serialize concurrent rustup component installs (they mutate one shared
    // toolchain); a queued duplicate that already added the component returns early.
    let _guard = acquire_install_lock(app, language_id, &RUSTUP_INSTALL_LOCK, command).await;
    if let Some(path) = already_installed(app, command) {
        return Ok(path);
    }
    // On a clean machine, provision the managed Rust toolchain — rustup-init already
    // installs the `rust-analyzer` component, so this single step can fully satisfy us.
    let rustup = if let Some(rustup) = resolve_tool(app, "rustup") {
        rustup
    } else {
        progress(app, language_id, 8, "Setting up Rust toolchain");
        let path =
            crate::runtime_provision::ensure_runtime(app, crate::runtime_provision::Runtime::Rust)
                .await
                .map_err(|e| format!("Rust auto-setup failed: {e}"))?;
        // rustup-init pulled rust-analyzer in the same shot; if it already resolves
        // in the managed toolchain we are done.
        if let Some(found) = resolve_tool(app, command) {
            let _ = path;
            progress(app, language_id, 90, "Resolving");
            return Ok(found.to_string_lossy().to_string());
        }
        resolve_tool(app, "rustup")
            .ok_or_else(|| "Rust was set up but rustup is still not resolvable.".to_string())?
    };
    progress(app, language_id, 25, "Adding rustup component");
    // Use the managed Rust home when rustup is the managed one, so the component
    // lands in the self-contained toolchain rather than the user's ~/.rustup.
    let env = if crate::runtime_provision::is_managed_path(app, &rustup) {
        crate::runtime_provision::managed_rust_env(app)
    } else {
        Vec::new()
    };
    let step = run_command_env(
        &rustup,
        &[
            "component".to_string(),
            "add".to_string(),
            component.to_string(),
        ],
        None,
        &env,
    )
    .await?;
    if !step.success {
        return Err(trim_output(&step.output, "rustup component add failed"));
    }
    progress(app, language_id, 90, "Resolving");
    // Prefer the managed toolchain bin, then PATH (rustup shims) for a system install.
    resolve_tool(app, command)
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| "rustup reported success but rust-analyzer is not resolvable.".to_string())
}

/// Install a `GithubReleaseSpec` server. Needs no host toolchain at all (unlike the
/// npm/go/pip/rustup methods, there is no `ensure_runtime` step) — it is a plain
/// download-and-extract straight from the upstream's GitHub Releases page.
async fn install_github_release(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    root: &Path,
    spec: &GithubReleaseSpec,
) -> Result<String, String> {
    // Serialize concurrent GitHub-release installs (see GH_INSTALL_LOCK); a queued
    // duplicate that already produced the binary short-circuits before re-downloading.
    let _guard = acquire_install_lock(app, language_id, &GH_INSTALL_LOCK, command).await;
    if let Some(path) = already_installed(app, command) {
        return Ok(path);
    }

    let gh_root = root.join("gh");
    tokio::fs::create_dir_all(&gh_root)
        .await
        .map_err(|e| e.to_string())?;
    // PID+nanos-unique scratch names (see `unique_scratch_path`) so a concurrent second
    // app instance installing the same server can't collide on a fixed name mid-extract.
    let archive_path = crate::runtime_provision::unique_scratch_path(
        &gh_root,
        &format!("{command}-download"),
        ".part",
    );
    let staging =
        crate::runtime_provision::unique_scratch_path(&gh_root, &format!("{command}-staging"), "");

    let result = install_github_release_inner(
        app,
        language_id,
        command,
        spec,
        &gh_root,
        &archive_path,
        &staging,
    )
    .await;

    // Always sweep scratch artifacts — success already moved what it needed out of
    // `staging` (see `single_child_dir`), and a failure must never leave a partial
    // download or half-extracted tree behind for a later re-install to trip over.
    let _ = tokio::fs::remove_file(&archive_path).await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    result
}

async fn install_github_release_inner(
    app: &AppHandle,
    language_id: &str,
    command: &str,
    spec: &GithubReleaseSpec,
    gh_root: &Path,
    archive_path: &Path,
    staging: &Path,
) -> Result<String, String> {
    progress(app, language_id, 8, "Resolving latest release");
    let client = crate::runtime_provision::http_client()?;
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

    // GitHub releases here publish no companion sha256 manifest (unlike nodejs.org's
    // SHASUMS256.txt or go.dev's per-file digests, which `runtime_provision` pins
    // against). Integrity instead comes from the archive successfully opening right
    // after — a truncated/corrupt download fails loudly at `extract_archive` rather
    // than silently installing a broken tree.
    progress(app, language_id, 15, "Downloading");
    let downloaded = download_asset(app, language_id, &client, &url, archive_path, 15, 75).await?;
    if downloaded == 0 {
        return Err(format!("Downloaded asset {asset} was empty"));
    }

    progress(app, language_id, 78, "Extracting");
    let ext = archive_ext(&asset)?;
    let _ = tokio::fs::remove_dir_all(staging).await;
    crate::runtime_provision::extract_archive(archive_path, staging, ext)
        .await
        .map_err(|e| {
            format!("Downloaded archive {asset} could not be opened (likely corrupt): {e}")
        })?;

    // Normalize the top-level dir away when the archive wraps everything in one (e.g.
    // clangd's `clangd_<ver>/`); LuaLS's archives have no such wrapper, so `None` here
    // just means "the staging root is already the install tree".
    progress(app, language_id, 90, "Installing");
    let inner = crate::runtime_provision::single_child_dir(staging)
        .await?
        .unwrap_or_else(|| staging.to_path_buf());
    let dest = gh_root.join(command);
    // Tombstone-swap into place — same atomic replace `runtime_provision` uses for
    // Node/Go/Python, so a re-install over a broken/partial `dest` always succeeds.
    crate::runtime_provision::replace_runtime_dir(&inner, &dest).await?;

    write_release_manifest(&dest, spec.repo, &tag, &asset).await;

    progress(app, language_id, 96, "Verifying");
    finalize(app, command)
}

#[derive(serde::Deserialize)]
struct GhReleaseResponse {
    tag_name: String,
}

/// Resolve the release tag to install: the pinned `version_tag`, or GitHub's
/// `/releases/latest` when unpinned.
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

/// A clear, actionable error for a platform with no published release asset.
fn no_asset_error(repo: &str, os: GhOs, arch: GhArch) -> String {
    if os == GhOs::Windows && arch == GhArch::Arm64 {
        format!(
            "{repo} publishes no native Windows-arm64 build. Run Lux IDE under x64 emulation (Windows 11's built-in x86-64 emulation for Arm) to install it, or install it manually."
        )
    } else {
        format!("{repo} publishes no release asset for this platform.")
    }
}

/// Derive the archive kind from a release asset's file name.
fn archive_ext(asset: &str) -> Result<&'static str, String> {
    let path = Path::new(asset);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if ext.eq_ignore_ascii_case("zip") || ext.eq_ignore_ascii_case("tgz") {
        return Ok(if ext.eq_ignore_ascii_case("zip") {
            "zip"
        } else {
            "tar.gz"
        });
    }
    // `.tar.gz` is a compound extension: `Path::extension()` only sees the trailing
    // `gz`, so also check the stem's own extension is `tar`.
    if ext.eq_ignore_ascii_case("gz")
        && Path::new(path.file_stem().unwrap_or_default())
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("tar"))
    {
        return Ok("tar.gz");
    }
    Err(format!(
        "Unsupported archive format for release asset {asset}"
    ))
}

/// Record which release is installed at `dest` (repo/tag/asset) so the install can
/// be inspected later — informational only, never consulted by `finalize`/resolution.
async fn write_release_manifest(dest: &Path, repo: &str, tag: &str, asset: &str) {
    let manifest = serde_json::json!({ "repo": repo, "tag": tag, "asset": asset });
    if let Ok(bytes) = serde_json::to_vec_pretty(&manifest) {
        let _ = tokio::fs::write(dest.join("manifest.json"), bytes).await;
    }
}

/// Stream a GitHub release asset to `dest`, emitting `Downloading` progress on
/// `lux://lsp-install` between `from`..`to` percent. A sibling of
/// `runtime_provision::download_to_file`, kept separate rather than reused directly:
/// that helper's progress calls are hard-wired to `lux://runtime-provision`, which
/// would misroute a GitHub-release install's progress to the wrong UI surface (and it
/// requires an `Integrity` checksum, which these assets don't publish). Returns the
/// number of bytes written.
async fn download_asset(
    app: &AppHandle,
    language_id: &str,
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    from: u8,
    to: u8,
) -> Result<u64, String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

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
    let span = u64::from(to.saturating_sub(from));
    let mut last_percent = from;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download interrupted: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Write failed: {e}"))?;
        downloaded += chunk.len() as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            let pct = from + u8::try_from(downloaded.min(total) * span / total).unwrap_or(0);
            if pct > last_percent {
                last_percent = pct;
                progress(app, language_id, pct, "Downloading");
            }
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(downloaded)
}

/// After an install that targets the managed dir, confirm the binary resolves and
/// return its absolute path. Fails loudly (never a silent success) if the expected
/// executable is not where the install method should have placed it.
fn finalize(app: &AppHandle, command: &str) -> Result<String, String> {
    for dir in managed_bin_dirs(app) {
        if let Some(path) = resolve_in_dir(&dir, command) {
            return Ok(path.to_string_lossy().to_string());
        }
    }
    Err(format!(
        "Install completed but `{command}` was not found in the managed directory. The package may use a different binary name."
    ))
}

// ── Command execution ──

struct CommandResult {
    success: bool,
    output: String,
}

async fn run_command_env(
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
    env: &[(String, String)],
) -> Result<CommandResult, String> {
    let mut command = tokio::process::Command::new(program);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    for (key, value) in env {
        command.env(key, value);
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(INSTALL_TIMEOUT_SECS),
        command.output(),
    )
    .await
    .map_err(|_| format!("Install timed out after {INSTALL_TIMEOUT_SECS}s"))?
    .map_err(|e| format!("Failed to start {}: {e}", program.display()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(CommandResult {
        success: output.status.success(),
        output: format!("{stdout}{stderr}").trim().to_string(),
    })
}

/// Resolve a host tool (npm/go/python/rustup), preferring a managed runtime
/// (`<app_data>/runtime`) over the system PATH so a machine with no system
/// Node/Rust still installs servers. Falls back to PATH when unmanaged.
fn resolve_tool(app: &AppHandle, tool: &str) -> Option<PathBuf> {
    for dir in crate::runtime_provision::runtime_bin_dirs(app) {
        if let Some(path) = resolve_in_dir(&dir, tool) {
            return Some(path);
        }
    }
    resolve_on_path(tool)
}

/// On Windows, the order in which executable extensions are tried. Native binaries
/// (`.com`/`.exe`) are preferred over script shims (`.bat`/`.cmd`) — mirroring the
/// default `PATHEXT` precedence — so a `.cmd`/`.bat` dropped next to a real
/// `go.exe`/`python.exe`/`rustup.exe` can't shadow it and shrink the PATH-injection
/// surface. `npm`/`npx`, which genuinely ship only as `.cmd` shims on Windows, are
/// still found because `.cmd` remains in the list (just later).
#[cfg(windows)]
const WINDOWS_EXE_EXTENSIONS: &[&str] = &[".com", ".exe", ".bat", ".cmd"];

/// Resolve a command on the system PATH, honoring Windows executable extensions in
/// native-first `PATHEXT`-style order.
pub fn resolve_on_path(command: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        for ext in WINDOWS_EXE_EXTENSIONS {
            if let Ok(path) = which::which(format!("{command}{ext}")) {
                return Some(path);
            }
        }
        // Last resort: let `which` apply the system PATHEXT itself for a bare name.
        which::which(command).ok()
    }
    #[cfg(not(windows))]
    {
        which::which(command).ok()
    }
}

/// Resolve `command` inside a specific directory (managed bin dir), applying
/// Windows executable extensions in native-first order.
pub fn resolve_in_dir(dir: &Path, command: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        // Prefer an explicit native/script extension over a bare, possibly
        // non-executable file of the same stem.
        for ext in WINDOWS_EXE_EXTENSIONS {
            let candidate = dir.join(format!("{command}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let direct = dir.join(command);
    if direct.is_file() {
        return Some(direct);
    }
    None
}

fn trim_output(output: &str, fallback: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        // Keep the tail — package managers put the actual error last.
        let tail: String = trimmed
            .chars()
            .rev()
            .take(600)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("{fallback}: {tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── GitHub-release asset selection ──

    #[test]
    fn lua_asset_selection_matrix() {
        assert_eq!(
            lua_language_server_asset(GhOs::Windows, GhArch::X64, "3.13.0").as_deref(),
            Some("lua-language-server-3.13.0-win32-x64.zip")
        );
        assert_eq!(
            lua_language_server_asset(GhOs::Linux, GhArch::X64, "3.13.0").as_deref(),
            Some("lua-language-server-3.13.0-linux-x64.tar.gz")
        );
        assert_eq!(
            lua_language_server_asset(GhOs::Linux, GhArch::Arm64, "3.13.0").as_deref(),
            Some("lua-language-server-3.13.0-linux-arm64.tar.gz")
        );
        assert_eq!(
            lua_language_server_asset(GhOs::Macos, GhArch::X64, "3.13.0").as_deref(),
            Some("lua-language-server-3.13.0-darwin-x64.tar.gz")
        );
        assert_eq!(
            lua_language_server_asset(GhOs::Macos, GhArch::Arm64, "3.13.0").as_deref(),
            Some("lua-language-server-3.13.0-darwin-arm64.tar.gz")
        );
        // No win32-arm64 build is published — must fail closed, not guess.
        assert_eq!(
            lua_language_server_asset(GhOs::Windows, GhArch::Arm64, "3.13.0"),
            None
        );
        // A leading `v` in a tag (not currently emitted by LuaLS, but defensive) is
        // stripped rather than baked into the asset name.
        assert_eq!(
            lua_language_server_asset(GhOs::Linux, GhArch::X64, "v3.13.0").as_deref(),
            Some("lua-language-server-3.13.0-linux-x64.tar.gz")
        );
    }

    #[test]
    fn clangd_asset_selection_matrix() {
        assert_eq!(
            clangd_asset(GhOs::Windows, GhArch::X64, "18.1.3").as_deref(),
            Some("clangd-windows-18.1.3.zip")
        );
        assert_eq!(
            clangd_asset(GhOs::Linux, GhArch::X64, "18.1.3").as_deref(),
            Some("clangd-linux-18.1.3.zip")
        );
        // The mac build is a universal binary — one asset for both architectures.
        assert_eq!(
            clangd_asset(GhOs::Macos, GhArch::X64, "18.1.3").as_deref(),
            Some("clangd-mac-18.1.3.zip")
        );
        assert_eq!(
            clangd_asset(GhOs::Macos, GhArch::Arm64, "18.1.3").as_deref(),
            Some("clangd-mac-18.1.3.zip")
        );
        // clangd/clangd publishes no windows-arm64 or linux-arm64 build.
        assert_eq!(clangd_asset(GhOs::Windows, GhArch::Arm64, "18.1.3"), None);
        assert_eq!(clangd_asset(GhOs::Linux, GhArch::Arm64, "18.1.3"), None);
    }

    #[test]
    fn no_asset_error_calls_out_windows_arm_emulation() {
        let msg = no_asset_error("clangd/clangd", GhOs::Windows, GhArch::Arm64);
        assert!(
            msg.contains("emulation"),
            "should suggest x64 emulation: {msg}"
        );
        let generic = no_asset_error("clangd/clangd", GhOs::Linux, GhArch::Arm64);
        assert!(
            !generic.contains("emulation"),
            "non-Windows-arm case shouldn't mention it: {generic}"
        );
    }

    #[test]
    fn archive_ext_detects_known_formats() {
        assert_eq!(archive_ext("clangd-windows-18.1.3.zip").unwrap(), "zip");
        assert_eq!(
            archive_ext("lua-language-server-3.13.0-linux-x64.tar.gz").unwrap(),
            "tar.gz"
        );
        assert!(archive_ext("lua-language-server-3.13.0.7z").is_err());
    }

    // ── Uninstall refusal (managed-only) ──

    #[test]
    fn managed_uninstall_guard_allows_managed_install() {
        let managed = Some(PathBuf::from("/managed/lsp/gh/clangd/bin/clangd"));
        assert!(managed_uninstall_guard("clangd", managed, None).is_ok());
    }

    #[test]
    fn managed_uninstall_guard_refuses_path_only_install() {
        let on_path = Some(PathBuf::from("/usr/bin/clangd"));
        let err = managed_uninstall_guard("clangd", None, on_path).unwrap_err();
        assert!(
            err.contains("system PATH"),
            "should explain it is a system install: {err}"
        );
    }

    #[test]
    fn managed_uninstall_guard_refuses_when_not_installed_at_all() {
        let err = managed_uninstall_guard("clangd", None, None).unwrap_err();
        assert!(
            err.contains("not installed"),
            "should say it isn't installed: {err}"
        );
    }

    // ── Catalog / recipe wiring ──

    #[test]
    fn method_label_reports_github_for_release_installs() {
        assert_eq!(
            method_label(InstallMethod::GithubRelease(LUA_LANGUAGE_SERVER_RELEASE)),
            "github"
        );
    }

    #[test]
    fn lua_and_cpp_recipes_use_github_release_with_no_manual_hint() {
        for language_id in ["lua", "cpp"] {
            let recipe = recipe_for(language_id).expect("recipe must exist");
            assert!(
                matches!(recipe.method, InstallMethod::GithubRelease(_)),
                "{language_id} should install via GithubRelease"
            );
        }
    }

    #[test]
    fn command_for_resolves_known_and_rejects_unknown_language_ids() {
        assert_eq!(command_for("lua"), Some("lua-language-server"));
        assert_eq!(command_for("cpp"), Some("clangd"));
        assert_eq!(command_for("not-a-real-language"), None);
    }

    // ── Archive top-level-dir normalization (via the reused `single_child_dir`) ──

    fn unique_temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lux-lsp-install-test-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    #[tokio::test]
    async fn single_child_dir_strips_a_lone_wrapper_like_clangds_archive() {
        // clangd's archives wrap everything in one `clangd_<ver>/` directory.
        let root = unique_temp_dir("wrapped");
        let wrapper = root.join("clangd_18.1.3");
        tokio::fs::create_dir_all(wrapper.join("bin"))
            .await
            .unwrap();
        tokio::fs::write(wrapper.join("bin").join("clangd"), b"stub")
            .await
            .unwrap();

        let inner = crate::runtime_provision::single_child_dir(&root)
            .await
            .unwrap();
        assert_eq!(inner, Some(wrapper));

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[tokio::test]
    async fn single_child_dir_keeps_root_for_lua_ls_unwrapped_layout() {
        // LuaLS's archives have no wrapper: bin/, locale/, LICENSE, … sit at the root.
        let root = unique_temp_dir("unwrapped");
        tokio::fs::create_dir_all(root.join("bin")).await.unwrap();
        tokio::fs::write(root.join("bin").join("lua-language-server"), b"stub")
            .await
            .unwrap();
        tokio::fs::create_dir_all(root.join("locale"))
            .await
            .unwrap();
        tokio::fs::write(root.join("LICENSE"), b"stub")
            .await
            .unwrap();

        let inner = crate::runtime_provision::single_child_dir(&root)
            .await
            .unwrap();
        assert_eq!(inner, None, "multi-entry root must not be stripped");

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    // ── manifest.json ──

    #[tokio::test]
    async fn write_release_manifest_round_trips_repo_tag_asset() {
        let dir = unique_temp_dir("manifest");
        tokio::fs::create_dir_all(&dir).await.unwrap();

        write_release_manifest(
            &dir,
            "LuaLS/lua-language-server",
            "3.13.0",
            "lua-language-server-3.13.0-linux-x64.tar.gz",
        )
        .await;

        let raw = tokio::fs::read_to_string(dir.join("manifest.json"))
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["repo"], "LuaLS/lua-language-server");
        assert_eq!(value["tag"], "3.13.0");
        assert_eq!(
            value["asset"],
            "lua-language-server-3.13.0-linux-x64.tar.gz"
        );

        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
