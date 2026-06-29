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
/// SHA-256 of `python-{PYTHON_EMBED_VERSION}-embed-amd64.zip`, from python.org's
/// signed SPDX manifest. Pinned because the embeddable zips have no `.sha256`
/// sibling; keep in lockstep with `PYTHON_EMBED_VERSION` on every bump.
const PYTHON_EMBED_SHA256_AMD64: &str =
    "8d3f33be9eb810f23c102f08475af2854e50484b8e4e06275e937be61ce3d2fb";
/// SHA-256 of `python-{PYTHON_EMBED_VERSION}-embed-arm64.zip` (same source).
const PYTHON_EMBED_SHA256_ARM64: &str =
    "d34db37675973785a2a539cd1c8dde1b6d45665f48c615ef55274b3798bf9fd3";
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
    // Reclaim any tombstoned trees a previous locked/crashed replace left behind
    // (now likely unlocked), so they don't accumulate in app data.
    if let Ok(root) = runtime_root(&app) {
        sweep_tombstones(&root).await;
        sweep_tombstones(&root.join("go")).await; // Go's SDK tombstones live under go/
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
    let file_name = format!("{stem}.{ext}");
    let url = format!("https://nodejs.org/dist/{version}/{file_name}");

    // nodejs.org publishes a signed `SHASUMS256.txt` per release listing every
    // artifact's SHA-256. Fetch it and pin to the line for *our* exact filename —
    // tracks the dynamically-resolved LTS without baking in stale hashes.
    progress(app, "node", 7, "Fetching checksums");
    let shasums = fetch_text(
        &client,
        &format!("https://nodejs.org/dist/{version}/SHASUMS256.txt"),
    )
    .await?;
    let checksum = Integrity::sha256(&shasum_for(&shasums, &file_name)?)?;

    let root = runtime_root(app)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    let archive = root.join(format!("node-download.{ext}"));
    download_to_file(app, "node", &client, &url, &archive, 8, 68, checksum).await?;

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
    // Replace atomically via tombstone — never move into a half-deleted dest.
    replace_runtime_dir(&inner, &dest).await?;
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
    /// Per-file SHA-256 (lowercase hex) published in the release index. Verified
    /// against the downloaded archive before extraction.
    #[serde(default)]
    sha256: String,
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
    // go.dev's release index ships a per-file SHA-256 — verify against it directly,
    // so the pin tracks whatever stable version we resolved (never stale).
    let checksum = Integrity::sha256(&file.sha256)?;

    let root = runtime_root(app)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    let archive = root.join(format!("go-download.{ext}"));
    download_to_file(app, "go", &client, &url, &archive, 8, 70, checksum).await?;

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
    // Replace atomically via tombstone — never move into a half-deleted dest.
    replace_runtime_dir(&inner, &sdk_dest).await?;
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
    // rustup publishes a `.sha256` sibling next to every rustup-init binary.
    // Fetch + enforce it before the installer is ever marked executable or run.
    progress(app, "rust", 5, "Fetching checksum");
    let sha_body = fetch_text(&client, &format!("{url}.sha256")).await?;
    let checksum = Integrity::sha256(&lone_sha256(&sha_body)?)?;
    let installer = root.join(bin_name);
    download_to_file(app, "rust", &client, &url, &installer, 6, 55, checksum).await?;

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
    // Pin the embeddable archive hash by arch; an unpinned arch fails closed.
    let (arch, embed_sha) = match std::env::consts::ARCH {
        "aarch64" => ("arm64", PYTHON_EMBED_SHA256_ARM64),
        "x86_64" => ("amd64", PYTHON_EMBED_SHA256_AMD64),
        other => return Err(format!("No pinned embeddable Python for arch {other}")),
    };
    let checksum = Integrity::sha256(embed_sha)?;
    let client = http_client()?;
    let root = runtime_root(app)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;

    let file = format!("python-{PYTHON_EMBED_VERSION}-embed-{arch}.zip");
    let url = format!("{PYTHON_FTP_BASE}/{PYTHON_EMBED_VERSION}/{file}");
    let archive = root.join("python-download.zip");
    download_to_file(app, "python", &client, &url, &archive, 8, 60, checksum).await?;

    progress(app, "python", 64, "Extracting");
    let dest = python_dir(app)?;
    // Extract into a staging dir, then atomically swap it into place — never extract
    // over a half-deleted old tree (the ENOTEMPTY/mixed-version footgun on Windows).
    let staging = root.join("python-staging");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    extract_archive(&archive, &staging, "zip").await?;
    let _ = tokio::fs::remove_file(&archive).await;

    // Embeddable Python disables `site` (and thus pip) by default. Uncomment the
    // `import site` line in the `pythonXY._pth` file so pip/installed packages work.
    progress(app, "python", 78, "Enabling pip");
    enable_embeddable_site(&staging).await?;
    // pip is part of the managed-Python contract: the `ty` language server installs
    // through it, so a Python with no pip is a broken runtime that later fails with no
    // self-repair. Bootstrap into the staging tree and FAIL provisioning if it can't
    // be made available, rather than marking python.exe ready and breaking ty later.
    if let Err(why) = bootstrap_pip(&client, &staging).await {
        return Err(format!("Python pip bootstrap failed: {why}"));
    }

    replace_runtime_dir(&staging, &dest).await?;
    let _ = tokio::fs::remove_dir_all(&staging).await;

    progress(app, "python", 96, "Verifying");
    finalize_marker(app, Runtime::Python)
}

/// Make `pip` available in the managed Python so the `ty` language server can be
/// installed. Prefers the bundled, network-free `ensurepip` (no remote code), and
/// only falls back to the network `get-pip.py` bootstrap when `ensurepip` is absent
/// (the case for Windows embeddable builds, which ship without it).
///
/// SECURITY: `get-pip.py` is a rolling, unversioned bootstrap with no stable
/// published hash, so a hard SHA pin would break on every upstream refresh. We
/// minimise exposure by (1) trying `ensurepip` first so the remote path is skipped
/// whenever possible, and (2) fetching over HTTPS and executing only inside the
/// isolated managed runtime dir. A fuller supply-chain fix (bundling verified
/// pip/setuptools/wheel wheels and installing them with `--require-hashes`) is
/// tracked as a followup.
async fn bootstrap_pip(client: &reqwest::Client, dest: &Path) -> Result<(), String> {
    // 1) Network-free path: `python -m ensurepip --upgrade`. Skips remote code
    //    entirely on any managed Python that bundles ensurepip.
    if let Some(py) = crate::lsp_install::resolve_in_dir(dest, "python") {
        let ensurepip = run_command_env(
            &py,
            &[
                "-m".to_string(),
                "ensurepip".to_string(),
                "--upgrade".to_string(),
            ],
            Some(dest),
            &[],
        )
        .await;
        if ensurepip.is_ok_and(|step| step.success) {
            return Ok(());
        }
    }

    // 2) Fallback: fetch + run get-pip.py (embeddable Python lacks ensurepip).
    let body = client
        .get(GET_PIP_URL)
        .send()
        .await
        .map_err(|e| format!("could not reach {GET_PIP_URL}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("get-pip.py download error: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("get-pip.py read failed: {e}"))?;
    if body.is_empty() {
        return Err("get-pip.py download was empty".to_string());
    }

    let script = dest.join("get-pip.py");
    tokio::fs::write(&script, &body)
        .await
        .map_err(|e| format!("could not write get-pip.py: {e}"))?;
    let py = crate::lsp_install::resolve_in_dir(dest, "python")
        .ok_or("managed python.exe not found for pip bootstrap")?;
    let result = run_command_env(
        &py,
        &[
            script.to_string_lossy().to_string(),
            "--no-warn-script-location".to_string(),
        ],
        Some(dest),
        &[],
    )
    .await;
    let _ = tokio::fs::remove_file(&script).await;

    let step = result?;
    if step.success {
        Ok(())
    } else {
        Err(trim_output(&step.output, "get-pip.py exited with an error"))
    }
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

/// Integrity expectation for a download. Every provisioned runtime must declare
/// one — a managed toolchain is prepended to PATH and (for rustup/get-pip) even
/// executed, so unverified bytes are never extracted or run.
enum Integrity {
    /// Vendor-published SHA-256 (lowercase hex, 64 chars), enforced byte-for-byte.
    Sha256(String),
}

impl Integrity {
    /// Parse + normalize an expected hex digest, rejecting anything that is not a
    /// well-formed SHA-256 so a malformed/empty pin fails closed (never skips).
    fn sha256(expected: &str) -> Result<Self, String> {
        let hex = expected.trim().to_ascii_lowercase();
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "Refusing download: malformed SHA-256 checksum ({} chars)",
                hex.len()
            ));
        }
        Ok(Self::Sha256(hex))
    }

    /// Compare the digest of the fully-downloaded bytes against the expectation.
    fn verify(&self, actual: &[u8; 32]) -> Result<(), String> {
        let Self::Sha256(expected) = self;
        let actual_hex = to_hex(actual);
        // Constant-time-ish: both sides are fixed-width lowercase hex of equal len.
        if actual_hex == *expected {
            Ok(())
        } else {
            Err(format!(
                "checksum mismatch — expected SHA-256 {expected}, got {actual_hex}"
            ))
        }
    }
}

/// Stream a download to `dest`, emitting coarse progress between `from`..`to`
/// percent based on Content-Length (falls back to an indeterminate label), and
/// verifying its integrity before the bytes are ever exposed for extraction/exec.
///
/// The bytes land in a sibling `.part` file and are SHA-256'd inline (single pass,
/// no extra read). Only after the digest matches `integrity` is the temp atomically
/// renamed onto `dest`; any failure deletes the temp so a partial/forged download
/// can never be promoted (defends both the no-integrity gap and partial-promotion).
// Each argument is a distinct, meaningful input (progress identity, HTTP client,
// source URL, destination, progress range, expected digest); a wrapper struct would
// only relocate the same fields without adding clarity.
#[allow(clippy::too_many_arguments)]
async fn download_to_file(
    app: &AppHandle,
    id: &str,
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    from: u8,
    to: u8,
    integrity: Integrity,
) -> Result<(), String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed ({url}): {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download error ({url}): {e}"))?;
    let total = response.content_length();
    progress(app, id, from, "Downloading");

    let part = dest.with_extension("part");
    // A stale `.part` from a previous aborted run must not be appended to.
    let _ = tokio::fs::remove_file(&part).await;

    // Hash + write streamed bytes together; bail (and clean up) on any error so we
    // never leave a half-written temp that a later run might mistake for complete.
    let result = stream_to_part(app, id, response, &part, total, from, to).await;
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

    // Verified — promote atomically into place.
    let _ = tokio::fs::remove_file(dest).await;
    tokio::fs::rename(&part, dest).await.map_err(|e| {
        format!(
            "Could not finalize verified download {}: {e}",
            dest.display()
        )
    })
}

/// Stream `response` into `part`, returning the SHA-256 of everything written.
/// Split out so the caller can guarantee `.part` cleanup on every error path.
async fn stream_to_part(
    app: &AppHandle,
    id: &str,
    response: reqwest::Response,
    part: &Path,
    total: Option<u64>,
    from: u8,
    to: u8,
) -> Result<[u8; 32], String> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::File::create(part)
        .await
        .map_err(|e| format!("Could not create {}: {e}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let span = u64::from(to.saturating_sub(from));
    let mut last_percent = from;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download interrupted: {e}"))?;
        hasher.update(&chunk);
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
    Ok(hasher.finalize())
}

/// Fetch a small vendor text resource (a checksum sibling/manifest) as a string.
async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
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

/// Look up the SHA-256 for `file_name` in a `SHASUMS256.txt`-style manifest, whose
/// lines are `<hex>␠␠<filename>` (filenames may carry a `*` binary-mode marker).
fn shasum_for(manifest: &str, file_name: &str) -> Result<String, String> {
    manifest
        .lines()
        .filter_map(|line| {
            let (hash, name) = line.split_once(char::is_whitespace)?;
            Some((hash.trim(), name.trim().trim_start_matches('*')))
        })
        .find(|(_, name)| *name == file_name)
        .map(|(hash, _)| hash.to_string())
        .ok_or_else(|| format!("No checksum for {file_name} in vendor manifest"))
}

/// Extract the digest from a `.sha256` sibling file, which is either a bare hex
/// digest or the `<hex>␠␠<filename>` form. Returns the first whitespace-delimited
/// token, leaving final validation to [`Integrity::sha256`].
fn lone_sha256(body: &str) -> Result<String, String> {
    body.split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| "Empty .sha256 checksum file".to_string())
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

/// Atomically replace the managed runtime directory `dest` with the freshly-staged
/// `staged` tree, never writing into a half-deleted destination.
///
/// The old `let _ = remove_dir_all(dest); move_dir(staged, dest)` pattern silently
/// ignored deletion failures — common on Windows when AV/a running tool locks a file
/// (ENOTEMPTY / file-in-use) — and then merged the new install into the stale tree,
/// producing mixed-version toolchains and repeat ENOTEMPTY failures. Instead: move
/// any existing `dest` aside to a unique tombstone (with retry/backoff), failing with
/// an actionable error if it is locked, and only then move `staged` into place. The
/// tombstone is best-effort deleted; survivors are swept on the next provision.
async fn replace_runtime_dir(staged: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    if tokio::fs::metadata(dest).await.is_ok() {
        let tombstone = tombstone_path(dest);
        move_aside_with_retry(dest, &tombstone).await?;
        // Best-effort: a still-locked tombstone is cleaned by `sweep_tombstones`.
        let _ = tokio::fs::remove_dir_all(&tombstone).await;
    }
    move_dir(staged, dest).await
}

/// A unique sibling path for retiring an in-use runtime dir, e.g.
/// `node` → `.node.tombstone-<nanos>`. Hidden + timestamped so it neither collides
/// nor (being a dotfile) is picked up by tool discovery before it is swept.
fn tombstone_path(dest: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let name = dest
        .file_name()
        .map_or_else(|| "runtime".to_string(), |n| n.to_string_lossy().to_string());
    dest.with_file_name(format!(".{name}.tombstone-{nanos}"))
}

/// Rename `from` → `to`, retrying with exponential backoff to ride out transient
/// Windows file locks (AV scan, a language server/terminal still holding a handle).
/// Fails with a user-actionable message if the directory stays locked.
async fn move_aside_with_retry(from: &Path, to: &Path) -> Result<(), String> {
    const ATTEMPTS: u32 = 5;
    let mut delay = std::time::Duration::from_millis(100);
    for attempt in 1..=ATTEMPTS {
        match tokio::fs::rename(from, to).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt == ATTEMPTS => {
                return Err(format!(
                    "managed runtime at {} is in use and could not be replaced \
                     (close running terminals/language servers and try again): {error}",
                    from.display()
                ));
            }
            Err(_) => {
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }
    Ok(())
}

/// Best-effort sweep of leftover tombstones from a previous crashed/locked replace.
/// Called before provisioning so a since-unlocked stale tree is reclaimed.
async fn sweep_tombstones(root: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(root).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        if name.to_string_lossy().contains(".tombstone-") {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}

/// Verify the MANAGED Python has a working `pip`, repairing it if not.
///
/// Embeddable Python's pip bootstrap is best-effort during provisioning (python.exe
/// alone marks the runtime ready), so a later pip-dependent LSP install (`ty`) could
/// hit a managed Python with no pip and no way to self-heal. This probes `pip` and,
/// if missing, re-runs the bootstrap once under the Python lock, failing with a
/// repairable error if it still cannot be installed. No-op when there is no managed
/// Python (a system Python owns its own pip).
pub async fn ensure_managed_pip(app: &AppHandle) -> Result<(), String> {
    let dest = python_dir(app)?;
    let Some(python) = crate::lsp_install::resolve_in_dir(&dest, "python") else {
        return Ok(());
    };
    if pip_available(&python).await {
        return Ok(());
    }
    let _guard = PYTHON_LOCK.lock().await;
    // Re-check under the lock: a queued caller may have just repaired pip.
    if pip_available(&python).await {
        return Ok(());
    }
    let client = http_client()?;
    enable_embeddable_site(&dest).await?;
    bootstrap_pip(&client, &dest)
        .await
        .map_err(|why| format!("managed Python pip is missing and bootstrap failed: {why}"))?;
    if pip_available(&python).await {
        Ok(())
    } else {
        Err("managed Python pip is still unavailable after bootstrap".to_string())
    }
}

async fn pip_available(python: &Path) -> bool {
    run_command_env(
        python,
        &["-m".to_string(), "pip".to_string(), "--version".to_string()],
        None,
        &[],
    )
    .await
    .is_ok_and(|result| result.success)
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

// ── SHA-256 (FIPS 180-4) ──
//
// A tiny, dependency-free streaming SHA-256. Integrity verification is the whole
// point of this module, so the hash lives here rather than pulling `sha2` into the
// desktop crate's dependency surface just for download checks. Vetted against the
// standard test vectors (empty / "abc" / 1e6×'a') plus padding-boundary streaming.

/// Lowercase-hex encode a digest (no `hex` crate dependency).
fn to_hex(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Streaming SHA-256 state: incremental `update`, one-shot `finalize`.
struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    msg_len: u64,
}

impl Sha256 {
    const H0: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    #[rustfmt::skip]
    const K: [u32; 64] = [
        0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1, 0x923f_82a4,
        0xab1c_5ed5, 0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3, 0x72be_5d74, 0x80de_b1fe,
        0x9bdc_06a7, 0xc19b_f174, 0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f,
        0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da, 0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7,
        0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967, 0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc,
        0x5338_0d13, 0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85, 0xa2bf_e8a1, 0xa81a_664b,
        0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070, 0x19a4_c116,
        0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
        0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208, 0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7,
        0xc671_78f2,
    ];

    const fn new() -> Self {
        Self {
            state: Self::H0,
            block: [0; 64],
            block_len: 0,
            msg_len: 0,
        }
    }

    /// Absorb `data`, compressing each completed 512-bit block.
    fn update(&mut self, mut data: &[u8]) {
        self.msg_len = self.msg_len.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let take = (64 - self.block_len).min(data.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&data[..take]);
            self.block_len += take;
            data = &data[take..];
            if self.block_len == 64 {
                self.compress();
                self.block_len = 0;
            }
        }
    }

    /// Apply the standard `0x80`/zero/length padding and emit the 32-byte digest.
    fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.msg_len.wrapping_mul(8).to_be_bytes();
        self.update(&[0x80]);
        // Pad with zeros until the block has room only for the 8-byte length.
        while self.block_len != 56 {
            self.update(&[0]);
        }
        self.update(&bit_len);
        debug_assert_eq!(self.block_len, 0, "length word must close the block");

        let mut out = [0u8; 32];
        for (chunk, word) in out.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    // `a`..`h` and `w`/`s0`/`s1` are the canonical SHA-256 working-variable names
    // from FIPS 180-4; the index arithmetic (`w[i-15]`, `w[i-2]`, …) is the message
    // schedule and can't be a plain iterator — renaming/rewriting would obscure the
    // well-known algorithm.
    #[allow(clippy::many_single_char_names, clippy::needless_range_loop)]
    fn compress(&mut self) {
        let mut w = [0u32; 64];
        for (word, chunk) in w.iter_mut().zip(self.block.chunks_exact(4)) {
            *word = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(Self::K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha256_hex(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        to_hex(&h.finalize())
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            sha256_hex(&million),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn sha256_streaming_matches_oneshot_across_block_boundaries() {
        // Exercise the 0x80/length padding at and around the 56/64-byte edges.
        for n in [0usize, 1, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 1000] {
            let data = vec![0x61u8; n];
            let oneshot = sha256_hex(&data);
            let mut h = Sha256::new();
            for chunk in data.chunks(7).filter(|c| !c.is_empty()) {
                h.update(chunk);
            }
            assert_eq!(oneshot, to_hex(&h.finalize()), "stream mismatch at n={n}");
        }
    }

    #[test]
    fn integrity_normalizes_and_rejects_malformed() {
        // Uppercase + surrounding whitespace are normalized to canonical lowercase.
        let upper = "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
        let Ok(Integrity::Sha256(hex)) = Integrity::sha256(&format!("  {upper}\n")) else {
            panic!("valid digest rejected");
        };
        assert_eq!(hex, upper.to_ascii_lowercase());

        assert!(Integrity::sha256("").is_err());
        assert!(Integrity::sha256("deadbeef").is_err()); // too short
        assert!(Integrity::sha256(&"z".repeat(64)).is_err()); // non-hex
    }

    #[test]
    fn integrity_verify_detects_mismatch() {
        let empty = Sha256::new().finalize();
        let good =
            Integrity::sha256("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
                .unwrap();
        assert!(good.verify(&empty).is_ok());

        let wrong =
            Integrity::sha256("0000000000000000000000000000000000000000000000000000000000000000")
                .unwrap();
        assert!(wrong.verify(&empty).is_err());
    }

    #[test]
    fn shasum_manifest_lookup() {
        let manifest = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  node-v20.0.0-linux-x64.tar.gz
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  node-v20.0.0-win-x64.zip
";
        assert_eq!(
            shasum_for(manifest, "node-v20.0.0-win-x64.zip").unwrap(),
            "b".repeat(64)
        );
        assert!(shasum_for(manifest, "node-v20.0.0-darwin-arm64.tar.gz").is_err());
    }

    #[test]
    fn lone_sha256_accepts_bare_and_named_forms() {
        let bare = "c".repeat(64);
        assert_eq!(lone_sha256(&format!("{bare}\n")).unwrap(), bare);
        assert_eq!(
            lone_sha256(&format!("{bare}  rustup-init.exe\n")).unwrap(),
            bare
        );
        assert!(lone_sha256("   ").is_err());
    }

    #[test]
    fn pinned_python_hashes_are_well_formed() {
        // Guards against a typo when the pinned version is bumped.
        assert!(Integrity::sha256(PYTHON_EMBED_SHA256_AMD64).is_ok());
        assert!(Integrity::sha256(PYTHON_EMBED_SHA256_ARM64).is_ok());
    }
}
