//! Windows shell integration: a `lux` launcher on the user PATH and an
//! Explorer right-click "Open with Lux IDE" verb for folders.
//!
//! Everything is per-user (HKCU + %LOCALAPPDATA%): no elevation, safe to apply
//! silently, and auto-update never breaks it because the verbs point at the
//! stable installed exe path.

static STARTUP_OPEN_PATH: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Records a folder passed on the command line (Explorer verb / `lux .`) so the
/// frontend can open it as the workspace once it boots.
pub fn capture_startup_path() {
    let candidate = std::env::args_os().skip(1).find_map(|arg| {
        let text = arg.to_string_lossy().into_owned();
        if text.starts_with('-') {
            return None;
        }
        let path = std::path::PathBuf::from(&text);
        if !path.exists() {
            return None;
        }
        // A file argument opens its containing folder as the workspace.
        let dir = if path.is_dir() {
            path
        } else {
            path.parent()?.to_path_buf()
        };
        let resolved = dir.canonicalize().unwrap_or(dir);
        Some(dunce::simplified(&resolved).to_string_lossy().into_owned())
    });
    if let Ok(mut slot) = STARTUP_OPEN_PATH.lock() {
        *slot = candidate;
    }
}

/// One-shot: the frontend consumes the pending startup folder exactly once, so
/// a webview reload can't re-trigger a workspace switch.
#[tauri::command]
pub fn startup_open_path() -> Option<String> {
    STARTUP_OPEN_PATH.lock().ok()?.take()
}

/// Always-on shell integration: applied on every launch of an installed build.
/// Idempotent (PATH entry is deduped, registry writes are upserts), so this also
/// self-heals after the install location changes or a registry cleanup.
#[cfg(windows)]
pub fn apply_default_integration() {
    if cfg!(debug_assertions) {
        // Dev builds would register target/debug/lux-desktop.exe — never do that.
        return;
    }
    let _ = imp::set_path(true);
    let _ = imp::set_context_menu(true);
}

/// Shell integration is Windows-only; other platforms get a no-op.
#[cfg(not(windows))]
pub const fn apply_default_integration() {}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::path::PathBuf;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_EXPAND_SZ};
    use winreg::{RegKey, RegValue};

    const MENU_KEY_DIR: &str = r"Software\Classes\Directory\shell\LuxIDE";
    const MENU_KEY_BG: &str = r"Software\Classes\Directory\Background\shell\LuxIDE";
    const MENU_LABEL: &str = "Open with Lux IDE";

    fn exe_path() -> Result<PathBuf, String> {
        let exe = std::env::current_exe()
            .map_err(|error| format!("cannot resolve executable path: {error}"))?;
        Ok(dunce::simplified(&exe).to_path_buf())
    }

    /// `%LOCALAPPDATA%\Lux IDE\bin` — holds the `lux.cmd` shim the PATH entry
    /// points at, so PATH itself never has to change across app updates.
    fn shim_dir() -> Result<PathBuf, String> {
        let base = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| "LOCALAPPDATA is not set".to_string())?;
        Ok(PathBuf::from(base).join("Lux IDE").join("bin"))
    }

    pub fn set_path(enable: bool) -> Result<(), String> {
        debug_assert!(
            enable,
            "PATH integration is always-on; disable path removed"
        );
        let dir = shim_dir()?;
        // Rewrite the shim every launch so it always points at the current exe;
        // the PATH entry itself is added once and deduped after that, so the
        // environment broadcast only fires when something actually changed.
        write_shim(&dir)?;
        if add_to_user_path(&dir.to_string_lossy())? {
            broadcast_environment_change();
        }
        Ok(())
    }

    fn write_shim(dir: &std::path::Path) -> Result<(), String> {
        let exe = exe_path()?;
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        // `start "" ...` detaches the IDE from the console; %* forwards the
        // folder/file arguments so `lux .` opens the current directory.
        let script = format!("@echo off\r\nstart \"\" \"{}\" %*\r\n", exe.display());
        std::fs::write(dir.join("lux.cmd"), script)
            .map_err(|error| format!("cannot write lux.cmd: {error}"))
    }

    fn environment_key(write: bool) -> io::Result<RegKey> {
        let flags = if write {
            KEY_READ | KEY_SET_VALUE
        } else {
            KEY_READ
        };
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Environment", flags)
    }

    fn current_user_path() -> Option<String> {
        environment_key(false)
            .ok()?
            .get_value::<String, _>("Path")
            .ok()
    }

    fn split_path(value: &str) -> Vec<String> {
        value
            .split(';')
            .filter(|entry| !entry.trim().is_empty())
            .map(str::to_string)
            .collect()
    }

    fn paths_equal(a: &str, b: &str) -> bool {
        a.trim_end_matches('\\')
            .eq_ignore_ascii_case(b.trim_end_matches('\\'))
    }

    /// Writes Path back as `REG_EXPAND_SZ` (the conventional type for user Path;
    /// preserves any %VAR% entries other software may have added).
    fn write_user_path(entries: &[String]) -> Result<(), String> {
        let key = environment_key(true)
            .map_err(|error| format!("cannot open HKCU\\Environment: {error}"))?;
        let joined = entries.join(";");
        let mut data: Vec<u8> = Vec::with_capacity((joined.len() + 1) * 2);
        for unit in joined.encode_utf16().chain(std::iter::once(0)) {
            data.extend_from_slice(&unit.to_le_bytes());
        }
        key.set_raw_value(
            "Path",
            &RegValue {
                bytes: data,
                vtype: REG_EXPAND_SZ,
            },
        )
        .map_err(|error| format!("cannot write user Path: {error}"))
    }

    /// Returns true when the entry was actually added (false = already present).
    fn add_to_user_path(dir: &str) -> Result<bool, String> {
        let mut entries = split_path(&current_user_path().unwrap_or_default());
        if entries.iter().any(|entry| paths_equal(entry, dir)) {
            return Ok(false);
        }
        entries.push(dir.to_string());
        write_user_path(&entries)?;
        Ok(true)
    }

    pub fn set_context_menu(enable: bool) -> Result<(), String> {
        debug_assert!(enable, "context menu is always-on; disable path removed");
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let exe = exe_path()?;
        let exe = exe.display().to_string();
        // %1 = the right-clicked folder; %V = the folder whose background
        // was right-clicked (no selection).
        register_verb(&root, MENU_KEY_DIR, &format!("\"{exe}\" \"%1\""), &exe)?;
        register_verb(&root, MENU_KEY_BG, &format!("\"{exe}\" \"%V\""), &exe)
    }

    fn register_verb(
        root: &RegKey,
        key_path: &str,
        command: &str,
        exe: &str,
    ) -> Result<(), String> {
        let (key, _) = root
            .create_subkey(key_path)
            .map_err(|error| format!("cannot create {key_path}: {error}"))?;
        key.set_value("", &MENU_LABEL)
            .map_err(|error| format!("cannot label {key_path}: {error}"))?;
        key.set_value("Icon", &format!("\"{exe}\""))
            .map_err(|error| format!("cannot set icon on {key_path}: {error}"))?;
        let (command_key, _) = key
            .create_subkey("command")
            .map_err(|error| format!("cannot create {key_path}\\command: {error}"))?;
        command_key
            .set_value("", &command)
            .map_err(|error| format!("cannot set command on {key_path}: {error}"))
    }

    /// Explorer and new consoles only see the PATH change after this broadcast.
    fn broadcast_environment_change() {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "user32")]
        extern "system" {
            fn SendMessageTimeoutW(
                hwnd: isize,
                msg: u32,
                wparam: usize,
                lparam: isize,
                flags: u32,
                timeout: u32,
                result: *mut usize,
            ) -> isize;
        }
        const HWND_BROADCAST: isize = 0xffff;
        const WM_SETTINGCHANGE: u32 = 0x001a;
        const SMTO_ABORTIFHUNG: u32 = 0x0002;
        let environment: Vec<u16> = std::ffi::OsStr::new("Environment")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut result = 0usize;
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                environment.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                2_000,
                &raw mut result,
            );
        }
    }
}
