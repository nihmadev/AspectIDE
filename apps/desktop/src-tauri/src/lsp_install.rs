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

/// How a given language server is obtained. Each variant maps to a concrete
/// package-manager invocation in `install_server`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// No automated installer (toolchain-specific); UI shows a manual hint.
    Manual(&'static str),
}

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
    InstallRecipe { language_id: "typescript", method: InstallMethod::Npm("typescript-language-server typescript") },
    InstallRecipe { language_id: "python", method: InstallMethod::Pip("ty") },
    InstallRecipe { language_id: "json", method: InstallMethod::Npm("vscode-langservers-extracted") },
    InstallRecipe { language_id: "html", method: InstallMethod::Npm("vscode-langservers-extracted") },
    InstallRecipe { language_id: "css", method: InstallMethod::Npm("vscode-langservers-extracted") },
    InstallRecipe { language_id: "yaml", method: InstallMethod::Npm("yaml-language-server") },
    InstallRecipe { language_id: "bash", method: InstallMethod::Npm("bash-language-server") },
    InstallRecipe { language_id: "go", method: InstallMethod::GoInstall("golang.org/x/tools/gopls") },
    InstallRecipe { language_id: "rust", method: InstallMethod::RustupComponent("rust-analyzer") },
    InstallRecipe { language_id: "lua", method: InstallMethod::Manual("Install lua-language-server from your package manager (brew install lua-language-server, or download from the LuaLS releases) and ensure it is on PATH.") },
    InstallRecipe { language_id: "cpp", method: InstallMethod::Manual("Install clangd via your LLVM toolchain (apt install clangd, brew install llvm, or the LLVM releases) and ensure it is on PATH.") },
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
    vec![
        root.join("bin"),
        root.join("npm").join("node_modules").join(".bin"),
        root.join("pip").join("bin"),
        root.join("pip").join("Scripts"),
        root.join("go").join("bin"),
    ]
}

// ── Status / catalog (Rust → UI) ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspCatalogEntry {
    pub language_id: String,
    pub name: String,
    pub command: String,
    pub extensions: Vec<String>,
    /// How it installs: "npm" | "go" | "pip" | "rustup" | "manual".
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
