//! Managed language runtimes (Node, Rust, Python) — VS Code / rustup style.
//!
//! The npm-, go- and rustup-based language servers in `lsp_install` need a host
//! toolchain (`npm`, `cargo`/`rustup`, `python`). Rather than forcing the user to
//! install those by hand, the IDE provisions them on demand into a managed dir
//! under app data and resolves them from there *before* PATH — so a clean machine
//! can bring every server online with zero manual setup. Progress streams to the
//! UI on `lux://runtime-provision`, mirroring the LSP install event shape.
//!
//! Scope of automation (the runtimes the product promises out of the box):
//!   • Node LTS  — official nodejs.org dist (zip on Windows, tar.gz elsewhere).
//!                 Unblocks all npm servers (ts, json/html/css, yaml, bash).
//!   • Rust      — rustup-init with managed `CARGO_HOME/RUSTUP_HOME`, minimal profile
//!                 plus the `rust-analyzer` component in one shot.
//!   • Python    — Windows embeddable build + pip bootstrap. Other platforms ship a
//!                 system Python, so there it is a manual hint (honest, no fake auto).
//!                 Hosts pip for the `ty` Python language server.

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const RUNTIME_EVENT: &str = "lux://runtime-provision";
/// Node release index — small JSON, lists every release newest-first with its
/// `lts` codename (string) vs `false`. We pick the newest LTS.
const NODE_INDEX_URL: &str = "https://nodejs.org/dist/index.json";
/// Go release index — JSON, newest stable first, each entry lists platform `files`.
const GO_INDEX_URL: &str = "https://go.dev/dl/?mode=json";
/// rustup bootstrap binary (cross-platform; arch/os folded into the path).
const RUSTUP_DIST_BASE: &str = "https://static.rust-lang.org/rustup/dist";
/// Python embeddable archive base (Windows only).
const PYTHON_FTP_BASE: &str = "https://www.python.org/ftp/python";
/// Pinned embeddable Python — embeddable builds exist only for specific patch
/// releases; this is a current, widely-available 3.12.x.
const PYTHON_EMBED_VERSION: &str = "3.12.8";
const GET_PIP_URL: &str = "https://bootstrap.pypa.io/get-pip.py";

/// Overall guard for a single provisioning run (download + extract + setup).
const PROVISION_TIMEOUT_SECS: u64 = 1_200;

/// Per-runtime locks. Several LSP installs can demand the same missing runtime at
/// once (e.g. 5 npm servers with no system Node) — without a lock they would all
/// download+extract into the same dir concurrently and corrupt it. Each runtime
/// gets its own lock so distinct runtimes still provision in parallel.
static NODE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static RUST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PYTHON_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static GO_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn provision_lock(runtime: Runtime) -> &'static tokio::sync::Mutex<()> {
    match runtime {
        Runtime::Node => &NODE_LOCK,
        Runtime::Rust => &RUST_LOCK,
        Runtime::Python => &PYTHON_LOCK,
        Runtime::Go => &GO_LOCK,
    }
}

// ── Identity ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Node,
    Rust,
    Python,
    Go,
}

impl Runtime {
    const fn id(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Go => "go",
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Node => "Node.js",
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::Go => "Go",
        }
    }

    /// The executable that, when resolvable, proves the runtime is installed.
    const fn marker_command(self) -> &'static str {
        match self {
            Self::Node => "npm",
            Self::Rust => "cargo",
            Self::Python => "python",
            Self::Go => "go",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        match id {
            "node" => Some(Self::Node),
            "rust" => Some(Self::Rust),
            "python" => Some(Self::Python),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    pub const fn all() -> [Self; 4] {
        [Self::Node, Self::Rust, Self::Python, Self::Go]
    }
}

// ── Managed layout ──

/// `<app_data>/runtime`. Each runtime owns a subdirectory.
pub fn runtime_root(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join("runtime"))
}

fn node_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(runtime_root(app)?.join("node"))
}

/// Managed Rust home. `CARGO_HOME` and `RUSTUP_HOME` live underneath so the toolchain
/// is fully self-contained and never touches the user's `~/.cargo`.
fn rust_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(runtime_root(app)?.join("rust"))
}

fn cargo_home(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(rust_dir(app)?.join("cargo"))
}

fn rustup_home(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(rust_dir(app)?.join("rustup"))
}

fn python_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(runtime_root(app)?.join("python"))
}

/// Managed Go root. The SDK unpacks to `go/sdk` (its own `bin/`); `go install`
/// targets land in `go/bin` via a GOBIN/GOPATH under here.
fn go_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(runtime_root(app)?.join("go"))
}

/// Directories to search (and prepend to PATH for child installs) so managed
/// `npm`/`node`/`cargo`/`rustup`/`python`/`pip`/`go` win over a system install.
/// Missing dirs are harmless — callers probe each for the specific tool.
pub fn runtime_bin_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(node) = node_dir(app) {
        dirs.push(node.clone());
        // npm on POSIX node tarballs lives in `bin/`.
        dirs.push(node.join("bin"));
    }
    if let Ok(cargo) = cargo_home(app) {
        dirs.push(cargo.join("bin"));
    }
    if let Ok(py) = python_dir(app) {
        dirs.push(py.clone());
        dirs.push(py.join("Scripts"));
        dirs.push(py.join("bin"));
    }
    if let Ok(go) = go_dir(app) {
        dirs.push(go.join("sdk").join("bin")); // the `go` compiler itself
        dirs.push(go.join("bin")); // `go install` outputs (gopls, etc.)
    }
    dirs
}

/// GOROOT / GOPATH / GOBIN for the managed Go SDK. Passed to managed `go`
/// invocations so they use the self-contained SDK and write tools into `go/bin`.
pub fn managed_go_env(app: &AppHandle) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Ok(go) = go_dir(app) {
        env.push((
            "GOROOT".to_string(),
            go.join("sdk").to_string_lossy().to_string(),
        ));
        env.push(("GOPATH".to_string(), go.to_string_lossy().to_string()));
        env.push((
            "GOBIN".to_string(),
            go.join("bin").to_string_lossy().to_string(),
        ));
    }
    env
}

/// `CARGO_HOME` / `RUSTUP_HOME` for the managed Rust toolchain. Must be passed to any
/// `rustup`/`cargo` invocation that uses the managed binaries, or they would write
/// to the user's `~/.rustup` instead of the self-contained managed home.
pub fn managed_rust_env(app: &AppHandle) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Ok(cargo) = cargo_home(app) {
        env.push((
            "CARGO_HOME".to_string(),
            cargo.to_string_lossy().to_string(),
        ));
    }
    if let Ok(rustup) = rustup_home(app) {
        env.push((
            "RUSTUP_HOME".to_string(),
            rustup.to_string_lossy().to_string(),
        ));
    }
    env
}

/// True when `path` lives inside the managed runtime root (vs. a system install).
pub fn is_managed_path(app: &AppHandle, path: &Path) -> bool {
    runtime_root(app)
        .ok()
        .and_then(|root| dunce::canonicalize(&root).ok().or(Some(root)))
        .zip(
            dunce::canonicalize(path)
                .ok()
                .or_else(|| Some(path.to_path_buf())),
        )
        .is_some_and(|(root, p)| p.starts_with(&root))
}

/// PATH-style env value of the managed bin dirs (joined with the OS separator),
/// prepended ahead of the inherited PATH. Used when launching package managers so
/// e.g. `go`/`rustup` invoked via npm scripts still see managed Node first.
pub fn prepended_path(app: &AppHandle) -> Option<(String, String)> {
    let dirs = runtime_bin_dirs(app);
    if dirs.is_empty() {
        return None;
    }
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut parts: Vec<PathBuf> = dirs.into_iter().filter(|d| d.is_dir()).collect();
    if parts.is_empty() {
        return None;
    }
    parts.extend(std::env::split_paths(&existing));
    let joined = std::env::join_paths(parts).ok()?;
    Some(("PATH".to_string(), joined.to_string_lossy().to_string()))
}

// ── Catalog (Rust → UI) ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCatalogEntry {
    pub id: String,
    pub name: String,
    /// True when the runtime's marker command resolves in the managed dir or PATH.
    pub installed: bool,
    /// True when satisfied specifically by the managed dir (vs. system PATH).
    pub managed: bool,
    /// Resolved marker path when installed.
    pub path: Option<String>,
    /// False when this platform has no automated install (UI shows the hint).
    pub can_auto: bool,
    /// Manual guidance when `can_auto` is false.
    pub manual_hint: String,
}

/// Probe a runtime's marker command in the managed dirs, then PATH. Pure FS/PATH
/// lookups — never launches anything, safe to poll from the Settings panel.
fn probe(app: &AppHandle, runtime: Runtime) -> (bool, Option<PathBuf>) {
    let marker = runtime.marker_command();
    for dir in runtime_bin_dirs(app) {
        if let Some(path) = crate::lsp_install::resolve_in_dir(&dir, marker) {
            return (true, Some(path));
        }
    }
    (false, None)
}

#[tauri::command]
// Tauri command: the Result is kept for IPC error-channel symmetry with the rest.
#[allow(clippy::unnecessary_wraps)]
pub fn runtime_catalog(app: AppHandle) -> Result<Vec<RuntimeCatalogEntry>, String> {
    let mut entries = Vec::with_capacity(Runtime::all().len());
    for runtime in Runtime::all() {
        let (managed, managed_path) = probe(&app, runtime);
        let path = managed_path
            .clone()
            .or_else(|| crate::lsp_install::resolve_on_path(runtime.marker_command()));
        let (can_auto, manual_hint) = auto_support(runtime);
        entries.push(RuntimeCatalogEntry {
            id: runtime.id().to_string(),
            name: runtime.name().to_string(),
            installed: path.is_some(),
            managed,
            path: path.map(|p| p.to_string_lossy().to_string()),
            can_auto,
            manual_hint,
        });
    }
    Ok(entries)
}

/// Whether a runtime can be auto-provisioned on the current platform.
fn auto_support(runtime: Runtime) -> (bool, String) {
    match runtime {
        Runtime::Node | Runtime::Rust | Runtime::Go => {
            if arch_tag().is_some() {
                (true, String::new())
            } else {
                (
                    false,
                    format!(
                        "No prebuilt {} download for this CPU architecture; install it from your package manager.",
                        runtime.name()
                    ),
                )
            }
        }
        Runtime::Python => {
            if cfg!(windows) && arch_tag().is_some() {
                (true, String::new())
            } else {
                (
                    false,
                    "Python ships with macOS/Linux; install it from python.org or your package manager and ensure it is on PATH.".to_string(),
                )
            }
        }
    }
}

// ── Provision events (Rust → UI) ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RuntimeProvisionEvent {
    #[serde(rename_all = "camelCase")]
    Started { id: String, name: String },
    #[serde(rename_all = "camelCase")]
    Progress {
        id: String,
        percent: u8,
        step: String,
    },
    #[serde(rename_all = "camelCase")]
    Finished {
        id: String,
        success: bool,
        path: Option<String>,
        error: Option<String>,
    },
}

fn emit_runtime(app: &AppHandle, event: &RuntimeProvisionEvent) {
    let _ = app.emit(RUNTIME_EVENT, event);
}

fn progress(app: &AppHandle, id: &str, percent: u8, step: &str) {
    emit_runtime(
        app,
        &RuntimeProvisionEvent::Progress {
            id: id.to_string(),
            percent,
            step: step.to_string(),
        },
    );
}

/// Provision (or repair) a managed runtime, streaming progress on
/// `lux://runtime-provision`. Returns the resolved marker path on success.
/// Idempotent — re-running repairs/updates in place.
#[tauri::command]
pub async fn runtime_provision(app: AppHandle, id: String) -> Result<String, String> {
    let Some(runtime) = Runtime::from_id(&id) else {
        return Err(format!("Unknown runtime: {id}"));
    };

    // Serialize concurrent provisions of the same runtime (see provision_lock).
    let _guard = provision_lock(runtime).lock().await;
    // Double-checked: a concurrent caller we queued behind may have already done it.
    if let (true, Some(path)) = probe(&app, runtime) {
        return Ok(path.to_string_lossy().to_string());
    }

    emit_runtime(
        &app,
        &RuntimeProvisionEvent::Started {
            id: id.clone(),
            name: runtime.name().to_string(),
        },
    );
    progress(&app, &id, 3, "Preparing");

    let work = async {
        match runtime {
            Runtime::Node => provision_node(&app).await,
            Runtime::Rust => provision_rust(&app).await,
            Runtime::Python => provision_python(&app).await,
            Runtime::Go => provision_go(&app).await,
        }
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(PROVISION_TIMEOUT_SECS), work)
        .await
        .unwrap_or_else(|_| {
            Err(format!(
                "{} setup timed out after {PROVISION_TIMEOUT_SECS}s",
                runtime.name()
            ))
        });

    match &result {
        Ok(path) => emit_runtime(
            &app,
            &RuntimeProvisionEvent::Finished {
                id: id.clone(),
                success: true,
                path: Some(path.clone()),
                error: None,
            },
        ),
        Err(error) => emit_runtime(
            &app,
            &RuntimeProvisionEvent::Finished {
                id: id.clone(),
                success: false,
                path: None,
                error: Some(error.clone()),
            },
        ),
    }
    result
}

/// Ensure a runtime is present, provisioning it if missing. Returns the marker
/// path. Used both by the explicit command and as a prerequisite step before an
/// LSP install that needs the corresponding host toolchain.
pub async fn ensure_runtime(app: &AppHandle, runtime: Runtime) -> Result<String, String> {
    if let (true, Some(path)) = probe(app, runtime) {
        return Ok(path.to_string_lossy().to_string());
    }
    // Defer to the command path so progress + finished events still stream.
    runtime_provision(app.clone(), runtime.id().to_string()).await
}

// ── Node ──

#[derive(serde::Deserialize)]
struct NodeRelease {
    version: String,
    /// `lts` is the codename string for LTS releases, or `false` otherwise.
    #[serde(default)]
    lts: serde_json::Value,
}

async fn provision_node(app: &AppHandle) -> Result<String, String> {
    let arch = arch_tag().ok_or("Unsupported CPU architecture for Node download")?;
    let client = http_client()?;

    progress(app, "node", 6, "Resolving latest LTS");
    let releases: Vec<NodeRelease> = client
        .get(NODE_INDEX_URL)
        .send()
        .await
        .map_err(|e| format!("Could not reach nodejs.org: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Node release index error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Malformed Node release index: {e}"))?;
    let lts = releases
        .iter()
        .find(|r| r.lts.is_string())
        .ok_or("No LTS Node release found in index")?;
    let version = lts.version.trim().to_string(); // e.g. "v20.18.1"

    let (os_tag, ext) = if cfg!(windows) {
        ("win", "zip")
    } else if cfg!(target_os = "macos") {
        ("darwin", "tar.gz")
    } else {
        ("linux", "tar.gz")
    };
    let stem = format!("node-{version}-{os_tag}-{arch}");
    let url = format!("https://nodejs.org/dist/{version}/{stem}.{ext}");

    let root = runtime_root(app)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    let archive = root.join(format!("node-download.{ext}"));
    download_to_file(app, "node", &client, &url, &archive, 8, 68).await?;

    progress(app, "node", 70, "Extracting");
    let staging = root.join("node-staging");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    extract_archive(&archive, &staging, ext).await?;
    let _ = tokio::fs::remove_file(&archive).await;

    // Node archives contain a single top-level `node-<ver>-<os>-<arch>/` dir.
    // Promote its contents to the stable managed `node/` dir atomically-ish.
    progress(app, "node", 88, "Installing");
    let inner = single_child_dir(&staging)
        .await?
        .unwrap_or_else(|| staging.clone());
    let dest = node_dir(app)?;
    let _ = tokio::fs::remove_dir_all(&dest).await;
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    move_dir(&inner, &dest).await?;
    let _ = tokio::fs::remove_dir_all(&staging).await;

    progress(app, "node", 96, "Verifying");
    finalize_marker(app, Runtime::Node)
}

// ── Go ──

#[derive(serde::Deserialize)]
struct GoRelease {
    #[serde(default)]
    stable: bool,
    #[serde(default)]
    files: Vec<GoFile>,
}

#[derive(serde::Deserialize)]
struct GoFile {
    filename: String,
    #[serde(default)]
    os: String,
    #[serde(default)]
    arch: String,
    #[serde(default)]
    kind: String,
}

async fn provision_go(app: &AppHandle) -> Result<String, String> {
    let arch = go_arch().ok_or("Unsupported CPU architecture for Go download")?;
    let os_tag = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let client = http_client()?;

    progress(app, "go", 6, "Resolving latest stable");
    let releases: Vec<GoRelease> = client
        .get(GO_INDEX_URL)
        .send()
        .await
        .map_err(|e| format!("Could not reach go.dev: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Go release index error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Malformed Go release index: {e}"))?;
    // Newest stable release's archive for this platform.
    let file = releases
        .iter()
        .filter(|r| r.stable)
        .flat_map(|r| r.files.iter())
        .find(|f| f.os == os_tag && f.arch == arch && f.kind == "archive")
        .ok_or("No Go archive found for this platform in the release index")?;
    let ext = if std::path::Path::new(&file.filename)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        "zip"
    } else {
        "tar.gz"
    };
    let url = format!("https://go.dev/dl/{}", file.filename);

    let root = runtime_root(app)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    let archive = root.join(format!("go-download.{ext}"));
    download_to_file(app, "go", &client, &url, &archive, 8, 70).await?;

    progress(app, "go", 72, "Extracting");
    let staging = root.join("go-staging");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    extract_archive(&archive, &staging, ext).await?;
    let _ = tokio::fs::remove_file(&archive).await;

    // Go archives contain a single top-level `go/` dir — that is the SDK (GOROOT).
    // Install it as `runtime/go/sdk`, keeping `runtime/go` itself as GOPATH.
    progress(app, "go", 88, "Installing");
    let inner = single_child_dir(&staging)
        .await?
        .unwrap_or_else(|| staging.clone());
    let sdk_dest = go_dir(app)?.join("sdk");
    let _ = tokio::fs::remove_dir_all(&sdk_dest).await;
    if let Some(parent) = sdk_dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    move_dir(&inner, &sdk_dest).await?;
    let _ = tokio::fs::remove_dir_all(&staging).await;

    progress(app, "go", 96, "Verifying");
    finalize_marker(app, Runtime::Go)
}

// ── Rust ──

async fn provision_rust(app: &AppHandle) -> Result<String, String> {
    let arch = rustup_arch().ok_or("Unsupported CPU architecture for Rust download")?;
    let client = http_client()?;
    let root = runtime_root(app)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;

    let (triple, bin_name) = if cfg!(windows) {
        (format!("{arch}-pc-windows-msvc"), "rustup-init.exe")
    } else if cfg!(target_os = "macos") {
        (format!("{arch}-apple-darwin"), "rustup-init")
    } else {
        (format!("{arch}-unknown-linux-gnu"), "rustup-init")
    };
    let url = format!("{RUSTUP_DIST_BASE}/{triple}/{bin_name}");
    let installer = root.join(bin_name);
    download_to_file(app, "rust", &client, &url, &installer, 6, 55).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&installer)
            .await
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o755);
        let _ = tokio::fs::set_permissions(&installer, perms).await;
    }

    progress(app, "rust", 60, "Installing toolchain + rust-analyzer");
    let cargo = cargo_home(app)?;
    let rustup = rustup_home(app)?;
    tokio::fs::create_dir_all(&cargo)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(&rustup)
        .await
        .map_err(|e| e.to_string())?;
    let env = vec![
        (
            "CARGO_HOME".to_string(),
            cargo.to_string_lossy().to_string(),
        ),
        (
            "RUSTUP_HOME".to_string(),
            rustup.to_string_lossy().to_string(),
        ),
    ];
    // Minimal profile keeps it small; pull rust-analyzer in the same shot so the
    // Rust LSP is ready immediately.
    let args = [
        "-y".to_string(),
        "--no-modify-path".to_string(),
        "--profile".to_string(),
        "minimal".to_string(),
        "--default-toolchain".to_string(),
        "stable".to_string(),
        "-c".to_string(),
        "rust-analyzer".to_string(),
    ];
    let step = run_command_env(&installer, &args, None, &env).await?;
    let _ = tokio::fs::remove_file(&installer).await;
    if !step.success {
        return Err(trim_output(&step.output, "rustup-init failed"));
    }

    progress(app, "rust", 96, "Verifying");
    finalize_marker(app, Runtime::Rust)
}

// ── Python (Windows embeddable) ──

async fn provision_python(app: &AppHandle) -> Result<String, String> {
    if !cfg!(windows) {
        return Err(auto_support(Runtime::Python).1);
    }
    let arch = if std::env::consts::ARCH == "aarch64" {
        "arm64"
    } else {
        "amd64"
    };
    let client = http_client()?;
    let root = runtime_root(app)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;

    let file = format!("python-{PYTHON_EMBED_VERSION}-embed-{arch}.zip");
    let url = format!("{PYTHON_FTP_BASE}/{PYTHON_EMBED_VERSION}/{file}");
    let archive = root.join("python-download.zip");
    download_to_file(app, "python", &client, &url, &archive, 8, 60).await?;

    progress(app, "python", 64, "Extracting");
    let dest = python_dir(app)?;
    let _ = tokio::fs::remove_dir_all(&dest).await;
    extract_archive(&archive, &dest, "zip").await?;
    let _ = tokio::fs::remove_file(&archive).await;

    // Embeddable Python disables `site` (and thus pip) by default. Uncomment the
    // `import site` line in the `pythonXY._pth` file so pip/installed packages work.
    progress(app, "python", 78, "Enabling pip");
    enable_embeddable_site(&dest).await?;
    // Bootstrap pip; tolerate failure — python.exe alone already counts as installed.
    if let Ok(get_pip) = client.get(GET_PIP_URL).send().await {
        if let Ok(body) = get_pip.bytes().await {
            let script = dest.join("get-pip.py");
            if tokio::fs::write(&script, &body).await.is_ok() {
                if let Some(py) = crate::lsp_install::resolve_in_dir(&dest, "python") {
                    let _ = run_command_env(
                        &py,
                        &[
                            script.to_string_lossy().to_string(),
                            "--no-warn-script-location".to_string(),
                        ],
                        Some(&dest),
                        &[],
                    )
                    .await;
                }
                let _ = tokio::fs::remove_file(&script).await;
            }
        }
    }

    progress(app, "python", 96, "Verifying");
    finalize_marker(app, Runtime::Python)
}

/// Embeddable Python ships a `pythonXY._pth` with `#import site` commented out,
/// which disables site-packages (so pip can't be used). Uncomment it.
async fn enable_embeddable_site(dir: &Path) -> Result<(), String> {
    let mut entries = tokio::fs::read_dir(dir).await.map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("_pth") {
            let contents = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| e.to_string())?;
            let patched = contents.replace("#import site", "import site");
            if patched != contents {
                tokio::fs::write(&path, patched)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
    }
    // No `_pth` (non-embeddable layout) — nothing to patch.
    Ok(())
}

// ── Shared helpers ──

fn finalize_marker(app: &AppHandle, runtime: Runtime) -> Result<String, String> {
    match probe(app, runtime) {
        (true, Some(path)) => Ok(path.to_string_lossy().to_string()),
        _ => Err(format!(
            "{} installed but `{}` was not found in the managed directory.",
            runtime.name(),
            runtime.marker_command()
        )),
    }
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .user_agent("Lux-IDE")
        .build()
        .map_err(|e| e.to_string())
}

/// Stream a download to `dest`, emitting coarse progress between `from`..`to`
/// percent based on Content-Length (falls back to an indeterminate label).
async fn download_to_file(
    app: &AppHandle,
    id: &str,
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    from: u8,
    to: u8,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed ({url}): {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download error ({url}): {e}"))?;
    let total = response.content_length();
    progress(app, id, from, "Downloading");

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
                progress(app, id, pct, "Downloading");
            }
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Extract a `.zip` or `.tar.gz` archive into `dest` (created fresh). Runs on a
/// blocking thread — the zip/tar crates are synchronous.
async fn extract_archive(archive: &Path, dest: &Path, ext: &str) -> Result<(), String> {
    tokio::fs::create_dir_all(dest)
        .await
        .map_err(|e| e.to_string())?;
    let archive = archive.to_path_buf();
    let dest = dest.to_path_buf();
    let ext = ext.to_string();
    tokio::task::spawn_blocking(move || {
        if ext == "zip" {
            extract_zip(&archive, &dest)
        } else {
            extract_tar_gz(&archive, &dest)
        }
    })
    .await
    .map_err(|e| format!("Extraction task failed: {e}"))?
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else {
            continue; // skip unsafe paths (zip-slip guard)
        };
        let outpath = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = outpath.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&outpath).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode));
            }
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    tar.unpack(dest).map_err(|e| e.to_string())
}

/// If `dir` contains exactly one entry and it is a directory, return it (used to
/// strip the single top-level folder inside Node archives).
async fn single_child_dir(dir: &Path) -> Result<Option<PathBuf>, String> {
    let mut entries = tokio::fs::read_dir(dir).await.map_err(|e| e.to_string())?;
    let mut found: Option<PathBuf> = None;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        if found.is_some() {
            return Ok(None); // more than one entry → no single root
        }
        let path = entry.path();
        if path.is_dir() {
            found = Some(path);
        } else {
            return Ok(None);
        }
    }
    Ok(found)
}

/// Move `from` to `to`, falling back to recursive copy when a plain rename is not
/// possible (e.g. across volumes / staging dir on another mount).
async fn move_dir(from: &Path, to: &Path) -> Result<(), String> {
    if tokio::fs::rename(from, to).await.is_ok() {
        return Ok(());
    }
    copy_dir_recursive(from, to).await
}

async fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    // Iterative BFS to avoid async-recursion boxing.
    tokio::fs::create_dir_all(to)
        .await
        .map_err(|e| e.to_string())?;
    let mut stack = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((src, dst)) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&src).await.map_err(|e| e.to_string())?;
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            let file_type = entry.file_type().await.map_err(|e| e.to_string())?;
            if file_type.is_dir() {
                tokio::fs::create_dir_all(&dst_path)
                    .await
                    .map_err(|e| e.to_string())?;
                stack.push((src_path, dst_path));
            } else {
                tokio::fs::copy(&src_path, &dst_path)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

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
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command
        .output()
        .await
        .map_err(|e| format!("Failed to start {}: {e}", program.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(CommandResult {
        success: output.status.success(),
        output: format!("{stdout}{stderr}").trim().to_string(),
    })
}

fn trim_output(output: &str, fallback: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
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

/// Node/Python download arch tag (`x64` / `arm64`), or None if unsupported.
fn arch_tag() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// Rust target-triple arch component.
fn rustup_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        _ => None,
    }
}

/// Go release `arch` value (go.dev uses GOARCH names).
fn go_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("amd64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}
