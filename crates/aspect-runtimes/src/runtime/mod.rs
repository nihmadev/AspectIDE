use std::path::{Path, PathBuf};

use tokio::sync::Mutex;

use crate::io::{download_to_file, fetch_text, http_client, PROVISION_TIMEOUT_SECS};
use crate::resolve::resolve_in_dir;
use crate::{arch_tag, go_arch, lone_sha256, replace_runtime_dir, rustup_arch, shasum_for};
use crate::{sweep_tombstones, unique_scratch_path, Integrity};
use crate::{extract_archive, single_child_dir};

pub(crate) mod types;
pub use crate::runtime::types::{Runtime, RuntimeCatalogEntry, RuntimeProvisionEvent};

static NODE_LOCK: Mutex<()> = Mutex::const_new(());
static RUST_LOCK: Mutex<()> = Mutex::const_new(());
static PYTHON_LOCK: Mutex<()> = Mutex::const_new(());
static GO_LOCK: Mutex<()> = Mutex::const_new(());

fn provision_lock(runtime: Runtime) -> &'static Mutex<()> {
    match runtime {
        Runtime::Node => &NODE_LOCK,
        Runtime::Rust => &RUST_LOCK,
        Runtime::Python => &PYTHON_LOCK,
        Runtime::Go => &GO_LOCK,
    }
}

/// `<app_data>/runtime`. Each runtime owns a subdirectory.
pub fn runtime_root(data_dir: &Path) -> PathBuf {
    data_dir.join("runtime")
}

fn node_dir(data_dir: &Path) -> PathBuf {
    runtime_root(data_dir).join("node")
}

fn rust_dir(data_dir: &Path) -> PathBuf {
    runtime_root(data_dir).join("rust")
}

fn cargo_home(data_dir: &Path) -> PathBuf {
    rust_dir(data_dir).join("cargo")
}

fn rustup_home(data_dir: &Path) -> PathBuf {
    rust_dir(data_dir).join("rustup")
}

fn python_dir(data_dir: &Path) -> PathBuf {
    runtime_root(data_dir).join("python")
}

fn go_dir(data_dir: &Path) -> PathBuf {
    runtime_root(data_dir).join("go")
}

/// Directories to search (and prepend to PATH) so managed binaries win.
pub fn runtime_bin_dirs(data_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    let node = node_dir(data_dir);
    dirs.push(node.clone());
    dirs.push(node.join("bin"));

    let cargo = cargo_home(data_dir);
    dirs.push(cargo.join("bin"));

    let py = python_dir(data_dir);
    dirs.push(py.clone());
    dirs.push(py.join("Scripts"));
    dirs.push(py.join("bin"));

    let go = go_dir(data_dir);
    dirs.push(go.join("sdk").join("bin"));
    dirs.push(go.join("bin"));

    dirs
}

/// GOROOT / GOPATH / GOBIN for the managed Go SDK.
pub fn managed_go_env(data_dir: &Path) -> Vec<(String, String)> {
    let go = go_dir(data_dir);
    vec![
        ("GOROOT".to_string(), go.join("sdk").to_string_lossy().to_string()),
        ("GOPATH".to_string(), go.to_string_lossy().to_string()),
        ("GOBIN".to_string(), go.join("bin").to_string_lossy().to_string()),
    ]
}

/// CARGO_HOME / RUSTUP_HOME for the managed Rust toolchain.
pub fn managed_rust_env(data_dir: &Path) -> Vec<(String, String)> {
    let mut env = Vec::new();
    let cargo = cargo_home(data_dir);
    env.push(("CARGO_HOME".to_string(), cargo.to_string_lossy().to_string()));
    let rustup = rustup_home(data_dir);
    env.push(("RUSTUP_HOME".to_string(), rustup.to_string_lossy().to_string()));
    env
}

/// True when `path` lives inside the managed runtime root.
pub fn is_managed_path(data_dir: &Path, path: &Path) -> bool {
    let root = runtime_root(data_dir);
    dunce::canonicalize(&root)
        .ok()
        .or(Some(root))
        .zip(
            dunce::canonicalize(path).ok().or_else(|| Some(path.to_path_buf())),
        )
        .is_some_and(|(r, p)| p.starts_with(&r))
}

/// PATH-style env value of the managed bin dirs (joined with the OS separator).
pub fn prepended_path(data_dir: &Path) -> Option<(String, String)> {
    let dirs = runtime_bin_dirs(data_dir);
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

/// Probe a runtime's marker command in the managed dirs, then PATH.
pub fn probe(data_dir: &Path, runtime: Runtime) -> (bool, Option<PathBuf>) {
    let marker = runtime.marker_command();
    for dir in runtime_bin_dirs(data_dir) {
        if let Some(path) = resolve_in_dir(&dir, marker) {
            return (true, Some(path));
        }
    }
    (false, None)
}

/// Whether a runtime can be auto-provisioned on the current platform.
pub fn auto_support(runtime: Runtime) -> (bool, String) {
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

fn finalize_marker(data_dir: &Path, runtime: Runtime) -> Result<String, String> {
    match probe(data_dir, runtime) {
        (true, Some(path)) => Ok(path.to_string_lossy().to_string()),
        _ => Err(format!(
            "{} installed but `{}` was not found in the managed directory.",
            runtime.name(),
            runtime.marker_command()
        )),
    }
}

/// Full runtime catalog with live installed state.
pub fn runtime_catalog(data_dir: &Path) -> Vec<RuntimeCatalogEntry> {
    let mut entries = Vec::with_capacity(Runtime::all().len());
    for runtime in Runtime::all() {
        let (managed, managed_path) = probe(data_dir, runtime);
        let path = managed_path
            .clone()
            .or_else(|| crate::resolve::resolve_on_path(runtime.marker_command()));
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
    entries
}

/// Provision (or repair) a managed runtime, returning the resolved marker path.
pub async fn runtime_provision(
    data_dir: &Path,
    id: &str,
    on_event: &(dyn Fn(RuntimeProvisionEvent) + Sync),
) -> Result<String, String> {
    let Some(runtime) = Runtime::from_id(id) else {
        return Err(format!("Unknown runtime: {id}"));
    };

    let _guard = provision_lock(runtime).lock().await;

    if let (true, Some(path)) = probe(data_dir, runtime) {
        return Ok(path.to_string_lossy().to_string());
    }

    let root = runtime_root(data_dir);
    sweep_tombstones(&root).await;
    sweep_tombstones(&root.join("go")).await;

    on_event(RuntimeProvisionEvent::Started {
        id: id.to_string(),
        name: runtime.name().to_string(),
    });

    let work = async {
        match runtime {
            Runtime::Node => provision_node(data_dir, on_event).await,
            Runtime::Rust => provision_rust(data_dir, on_event).await,
            Runtime::Python => provision_python(data_dir, on_event).await,
            Runtime::Go => provision_go(data_dir, on_event).await,
        }
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(PROVISION_TIMEOUT_SECS),
        work,
    )
    .await
    .unwrap_or_else(|_| {
        Err(format!(
            "{} setup timed out after {PROVISION_TIMEOUT_SECS}s",
            runtime.name()
        ))
    });

    match &result {
        Ok(path) => on_event(RuntimeProvisionEvent::Finished {
            id: id.to_string(),
            success: true,
            path: Some(path.clone()),
            error: None,
        }),
        Err(error) => on_event(RuntimeProvisionEvent::Finished {
            id: id.to_string(),
            success: false,
            path: None,
            error: Some(error.clone()),
        }),
    }
    result
}

/// Ensure a runtime is present, provisioning it if missing.
pub async fn ensure_runtime(
    data_dir: &Path,
    runtime: Runtime,
    on_event: &(dyn Fn(RuntimeProvisionEvent) + Sync),
) -> Result<String, String> {
    if let (true, Some(path)) = probe(data_dir, runtime) {
        return Ok(path.to_string_lossy().to_string());
    }
    runtime_provision(data_dir, runtime.id(), on_event).await
}

// ── Node ──

#[derive(serde::Deserialize)]
struct NodeRelease {
    version: String,
    #[serde(default)]
    lts: serde_json::Value,
}

async fn provision_node(
    data_dir: &Path,
    on_event: &(dyn Fn(RuntimeProvisionEvent) + Sync),
) -> Result<String, String> {
    let arch = arch_tag().ok_or("Unsupported CPU architecture for Node download")?;
    let client = http_client()?;

    let releases: Vec<NodeRelease> = client
        .get(crate::io::NODE_INDEX_URL)
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
    let version = lts.version.trim().to_string();

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

    let shasums = fetch_text(
        &client,
        &format!("https://nodejs.org/dist/{version}/SHASUMS256.txt"),
    )
    .await?;
    let checksum = Integrity::sha256(&shasum_for(&shasums, &file_name)?)?;

    let root = runtime_root(data_dir);
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    let archive = unique_scratch_path(&root, "node-download", &format!(".{ext}"));
    download_to_file(&client, &url, &archive, checksum, |pct| {
        on_event(RuntimeProvisionEvent::Progress {
            id: "node".to_string(),
            percent: 8 + pct * 60 / 100,
            step: "Downloading".to_string(),
        });
    })
    .await?;

    let staging = unique_scratch_path(&root, "node-staging", "");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    extract_archive(&archive, &staging, ext).await?;
    let _ = tokio::fs::remove_file(&archive).await;

    let inner = single_child_dir(&staging)
        .await?
        .unwrap_or_else(|| staging.clone());
    let dest = node_dir(data_dir);
    replace_runtime_dir(&inner, &dest).await?;
    let _ = tokio::fs::remove_dir_all(&staging).await;

    finalize_marker(data_dir, Runtime::Node)
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
    #[serde(default)]
    sha256: String,
}

async fn provision_go(
    data_dir: &Path,
    on_event: &(dyn Fn(RuntimeProvisionEvent) + Sync),
) -> Result<String, String> {
    let arch = go_arch().ok_or("Unsupported CPU architecture for Go download")?;
    let os_tag = crate::platform::host_os_tag();
    let client = http_client()?;

    let releases: Vec<GoRelease> = client
        .get(crate::io::GO_INDEX_URL)
        .send()
        .await
        .map_err(|e| format!("Could not reach go.dev: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Go release index error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Malformed Go release index: {e}"))?;
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
    let checksum = Integrity::sha256(&file.sha256)?;

    let root = runtime_root(data_dir);
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    let archive = unique_scratch_path(&root, "go-download", &format!(".{ext}"));
    download_to_file(&client, &url, &archive, checksum, |pct| {
        on_event(RuntimeProvisionEvent::Progress {
            id: "go".to_string(),
            percent: 8 + pct * 62 / 100,
            step: "Downloading".to_string(),
        });
    })
    .await?;

    let staging = unique_scratch_path(&root, "go-staging", "");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    extract_archive(&archive, &staging, ext).await?;
    let _ = tokio::fs::remove_file(&archive).await;

    let inner = single_child_dir(&staging)
        .await?
        .unwrap_or_else(|| staging.clone());
    let sdk_dest = go_dir(data_dir).join("sdk");
    replace_runtime_dir(&inner, &sdk_dest).await?;
    let _ = tokio::fs::remove_dir_all(&staging).await;

    finalize_marker(data_dir, Runtime::Go)
}

// ── Rust ──

async fn provision_rust(
    data_dir: &Path,
    on_event: &(dyn Fn(RuntimeProvisionEvent) + Sync),
) -> Result<String, String> {
    let arch = rustup_arch().ok_or("Unsupported CPU architecture for Rust download")?;
    let client = http_client()?;
    let root = runtime_root(data_dir);
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
    let url = format!("{}/{triple}/{bin_name}", crate::io::RUSTUP_DIST_BASE);

    let sha_body = fetch_text(&client, &format!("{url}.sha256")).await?;
    let checksum = Integrity::sha256(&lone_sha256(&sha_body)?)?;
    let installer = root.join(bin_name);
    download_to_file(&client, &url, &installer, checksum, |pct| {
        on_event(RuntimeProvisionEvent::Progress {
            id: "rust".to_string(),
            percent: 6 + pct * 49 / 100,
            step: "Downloading".to_string(),
        });
    })
    .await?;

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

    let cargo = cargo_home(data_dir);
    let rustup = rustup_home(data_dir);
    tokio::fs::create_dir_all(&cargo)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(&rustup)
        .await
        .map_err(|e| e.to_string())?;
    let env = vec![
        ("CARGO_HOME".to_string(), cargo.to_string_lossy().to_string()),
        ("RUSTUP_HOME".to_string(), rustup.to_string_lossy().to_string()),
    ];
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
    let step = crate::command::run_command_env(&installer, &args, None, &env).await?;
    let _ = tokio::fs::remove_file(&installer).await;
    if !step.success {
        return Err(crate::command::trim_output(&step.output, "rustup-init failed"));
    }

    finalize_marker(data_dir, Runtime::Rust)
}

// ── Python (Windows embeddable) ──

async fn provision_python(
    data_dir: &Path,
    on_event: &(dyn Fn(RuntimeProvisionEvent) + Sync),
) -> Result<String, String> {
    if !cfg!(windows) {
        return Err(auto_support(Runtime::Python).1);
    }
    let (arch, embed_sha) = match std::env::consts::ARCH {
        "aarch64" => ("arm64", crate::io::PYTHON_EMBED_SHA256_ARM64),
        "x86_64" => ("amd64", crate::io::PYTHON_EMBED_SHA256_AMD64),
        other => return Err(format!("No pinned embeddable Python for arch {other}")),
    };
    let checksum = Integrity::sha256(embed_sha)?;
    let client = http_client()?;
    let root = runtime_root(data_dir);
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;

    let file = format!("python-{}-embed-{arch}.zip", crate::io::PYTHON_EMBED_VERSION);
    let url = format!("{}/{}/{}", crate::io::PYTHON_FTP_BASE, crate::io::PYTHON_EMBED_VERSION, file);
    let archive = unique_scratch_path(&root, "python-download", ".zip");
    download_to_file(&client, &url, &archive, checksum, |pct| {
        on_event(RuntimeProvisionEvent::Progress {
            id: "python".to_string(),
            percent: 8 + pct * 52 / 100,
            step: "Downloading".to_string(),
        });
    })
    .await?;

    let dest = python_dir(data_dir);
    let staging = unique_scratch_path(&root, "python-staging", "");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    extract_archive(&archive, &staging, "zip").await?;
    let _ = tokio::fs::remove_file(&archive).await;

    enable_embeddable_site(&staging).await?;
    if let Err(why) = bootstrap_pip(&client, &staging).await {
        return Err(format!("Python pip bootstrap failed: {why}"));
    }

    replace_runtime_dir(&staging, &dest).await?;
    let _ = tokio::fs::remove_dir_all(&staging).await;

    finalize_marker(data_dir, Runtime::Python)
}

async fn bootstrap_pip(client: &reqwest::Client, dest: &Path) -> Result<(), String> {
    if let Some(py) = crate::resolve::resolve_in_dir(dest, "python") {
        let ensurepip = crate::command::run_command_env(
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

    let body = client
        .get(crate::io::GET_PIP_URL)
        .send()
        .await
        .map_err(|e| format!("could not reach {}: {e}", crate::io::GET_PIP_URL))?
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
    let py = crate::resolve::resolve_in_dir(dest, "python")
        .ok_or("managed python.exe not found for pip bootstrap")?;
    let result = crate::command::run_command_env(
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
        Err(crate::command::trim_output(&step.output, "get-pip.py exited with an error"))
    }
}

/// Embeddable Python ships a `pythonXY._pth` with `#import site` commented out.
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
    Ok(())
}

/// Verify the managed Python has a working `pip`, repairing it if not.
pub async fn ensure_managed_pip(data_dir: &Path) -> Result<(), String> {
    let dest = python_dir(data_dir);
    let Some(python) = crate::resolve::resolve_in_dir(&dest, "python") else {
        return Ok(());
    };
    if pip_available(&python).await {
        return Ok(());
    }
    let _guard = PYTHON_LOCK.lock().await;
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
    crate::command::run_command_env(
        python,
        &["-m".to_string(), "pip".to_string(), "--version".to_string()],
        None,
        &[],
    )
    .await
    .is_ok_and(|result| result.success)
}
