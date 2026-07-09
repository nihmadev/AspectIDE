/// Node/Python download arch tag (`x64` / `arm64`), or None if unsupported.
pub fn arch_tag() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// Rust target-triple arch component.
pub fn rustup_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        _ => None,
    }
}

/// Go release `arch` value (go.dev uses GOARCH names).
pub fn go_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("amd64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// Host OS bucket for selecting a GitHub release asset.
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

pub const fn current_gh_os() -> GhOs {
    if cfg!(windows) {
        GhOs::Windows
    } else if cfg!(target_os = "macos") {
        GhOs::Macos
    } else {
        GhOs::Linux
    }
}

pub fn current_gh_arch() -> Option<GhArch> {
    match std::env::consts::ARCH {
        "x86_64" => Some(GhArch::X64),
        "aarch64" => Some(GhArch::Arm64),
        _ => None,
    }
}

pub const fn host_os_tag() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}
