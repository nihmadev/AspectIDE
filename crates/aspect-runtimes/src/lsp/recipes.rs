use super::InstallMethod;
use crate::platform::{GhArch, GhOs};

/// A managed install sourced directly from a GitHub Releases page.
#[derive(Debug, Clone, Copy)]
pub struct GithubReleaseSpec {
    pub repo: &'static str,
    pub version_tag: Option<&'static str>,
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
        (GhOs::Windows, GhArch::Arm64) => return None,
    };
    Some(name)
}

fn clangd_asset(os: GhOs, arch: GhArch, version: &str) -> Option<String> {
    let v = version.trim_start_matches('v');
    match (os, arch) {
        (GhOs::Windows, GhArch::X64) => Some(format!("clangd-windows-{v}.zip")),
        (GhOs::Linux, GhArch::X64) => Some(format!("clangd-linux-{v}.zip")),
        (GhOs::Macos, _) => Some(format!("clangd-mac-{v}.zip")),
        (GhOs::Windows | GhOs::Linux, GhArch::Arm64) => None,
    }
}

pub const LUA_LANGUAGE_SERVER_RELEASE: GithubReleaseSpec = GithubReleaseSpec {
    repo: "LuaLS/lua-language-server",
    version_tag: None,
    asset_for: lua_language_server_asset,
    bin_subdirs: &["bin"],
};

pub const CLANGD_RELEASE: GithubReleaseSpec = GithubReleaseSpec {
    repo: "clangd/clangd",
    version_tag: None,
    asset_for: clangd_asset,
    bin_subdirs: &["bin"],
};

/// Install recipe for one catalog server, keyed by `language_id`.
#[derive(Debug, Clone, Copy)]
pub struct InstallRecipe {
    pub language_id: &'static str,
    pub method: InstallMethod,
}

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

pub fn recipe_for(language_id: &str) -> Option<&'static InstallRecipe> {
    INSTALL_RECIPES
        .iter()
        .find(|recipe| recipe.language_id == language_id)
}

pub fn command_for(language_id: &str) -> Option<&'static str> {
    aspect_lsp::BUILTIN_SERVERS
        .iter()
        .find(|s| s.language_id == language_id)
        .map(|s| s.command)
}

pub fn no_asset_error(repo: &str, os: GhOs, arch: GhArch) -> String {
    if os == GhOs::Windows && arch == GhArch::Arm64 {
        format!(
            "{repo} publishes no native Windows-arm64 build. Run AspectIDE under x64 emulation (Windows 11's built-in x86-64 emulation for Arm) to install it, or install it manually."
        )
    } else {
        format!("{repo} publishes no release asset for this platform.")
    }
}

