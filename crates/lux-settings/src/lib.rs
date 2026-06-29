#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    cmp::Reverse,
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::Utc;
use lux_core::{
    AppError, AppResult, Keybinding, KeybindingProfile, RecentWorkspace, SettingValue,
    SettingsScope, WorkspaceInfo,
};
use serde_json::Value;

const RECENT_WORKSPACES_FILE: &str = "recent-workspaces.json";
const MAX_RECENT_WORKSPACES: usize = 12;
const KEYBINDINGS_KEY: &str = "workbench.keybindings";
/// Sub-directory name used for workspace-scoped settings inside the workspace root.
const WORKSPACE_SETTINGS_DIR: &str = ".lux";
const WORKSPACE_SETTINGS_FILE: &str = "settings.json";

pub struct SettingsStore {
    /// Path to the *user* settings file; always authoritative for user-scoped calls.
    path: PathBuf,
    values: BTreeMap<String, SettingValue>,
    /// When the original file was corrupt *and* quarantine failed (rename refused),
    /// we must not overwrite the still-present corrupt bytes. All `set()` calls on
    /// a read-only store write to a fresh sibling path instead, leaving the original
    /// intact until the user can inspect and recover it manually.
    read_only_corrupt: bool,
}

impl SettingsStore {
    pub fn load(path: PathBuf) -> AppResult<Self> {
        let (values, read_only_corrupt) = if path.exists() {
            let raw = fs::read_to_string(&path)?;
            match serde_json::from_str(&raw) {
                Ok(values) => (values, false),
                // A corrupt `settings.json` must never silently collapse to an empty
                // map: the next `set()` would persist that emptiness and erase every
                // saved setting for good. Move the bad file aside (preserving it as a
                // recoverable backup) before starting from defaults.
                //
                // If the rename itself fails (permissions, collision) we mark the store
                // read-only so `set()` writes to a fresh sibling instead of overwriting
                // the still-present corrupt bytes.
                Err(error) => {
                    let quarantined = quarantine_corrupt(&path, &error);
                    (BTreeMap::new(), !quarantined)
                }
            }
        } else {
            (BTreeMap::new(), false)
        };

        Ok(Self {
            path,
            values,
            read_only_corrupt,
        })
    }

    #[must_use]
    pub fn get(&self, scope: SettingsScope, key: &str) -> Option<SettingValue> {
        match scope {
            SettingsScope::User => self.values.get(key).cloned(),
            SettingsScope::Workspace(root) => {
                // Load the workspace-local store on demand (read-only merge: workspace
                // value wins over user value when present).
                let ws_path = workspace_settings_path(&root);
                if let Ok(ws_store) = Self::load(ws_path) {
                    if let Some(val) = ws_store.values.get(key).cloned() {
                        return Some(val);
                    }
                }
                // Fall through to user-scope for keys not set at workspace level.
                self.values.get(key).cloned()
            }
        }
    }

    pub fn set(
        &mut self,
        scope: SettingsScope,
        key: String,
        value: Value,
    ) -> AppResult<SettingValue> {
        let setting = SettingValue {
            key: key.clone(),
            value,
            updated_at: Utc::now(),
        };
        match scope {
            SettingsScope::User => {
                self.values.insert(key, setting.clone());
                // Resolve the effective write path. When the original settings file is
                // corrupt and could not be quarantined (renamed aside), we must NOT
                // overwrite it — instead write to a fresh recovery sibling so the bad
                // bytes are never clobbered before the user can inspect them.
                let write_path = if self.read_only_corrupt {
                    recovery_path(&self.path)
                } else {
                    self.path.clone()
                };
                if let Some(parent) = write_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                write_atomic(
                    &write_path,
                    serde_json::to_string_pretty(&self.values)?.as_bytes(),
                )?;
            }
            SettingsScope::Workspace(root) => {
                // Write into the workspace-local settings file; never mutate the
                // user-level `self.values` map so workspace prefs stay isolated.
                let ws_path = workspace_settings_path(&root);
                let mut ws_store = Self::load(ws_path.clone()).unwrap_or_else(|_| Self {
                    path: ws_path.clone(),
                    values: BTreeMap::new(),
                    read_only_corrupt: false,
                });
                ws_store.values.insert(key, setting.clone());
                if let Some(parent) = ws_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                write_atomic(
                    &ws_path,
                    serde_json::to_string_pretty(&ws_store.values)?.as_bytes(),
                )?;
            }
        }
        Ok(setting)
    }

    /// Returns the filesystem path for workspace-scoped settings inside `root`.
    #[must_use]
    pub fn workspace_settings_path(root: &Path) -> PathBuf {
        workspace_settings_path(root)
    }

    pub fn keybinding_profile(&self) -> KeybindingProfile {
        self.values
            .get(KEYBINDINGS_KEY)
            .and_then(|setting| serde_json::from_value(setting.value.clone()).ok())
            .map_or_else(default_keybinding_profile, normalize_keybinding_profile)
    }

    pub fn set_keybinding_profile(
        &mut self,
        profile: KeybindingProfile,
    ) -> AppResult<KeybindingProfile> {
        let profile = normalize_keybinding_profile(profile);
        self.set(
            SettingsScope::User,
            KEYBINDINGS_KEY.to_string(),
            serde_json::to_value(&profile)?,
        )?;
        Ok(profile)
    }

    pub fn recent_workspaces(&self) -> AppResult<Vec<RecentWorkspace>> {
        let path = self.recent_workspaces_path()?;
        if !path.exists() {
            return Ok(Vec::new());
        }

        let raw = fs::read_to_string(&path)?;
        let mut workspaces: Vec<RecentWorkspace> = match serde_json::from_str(&raw) {
            Ok(workspaces) => workspaces,
            // Same invariant as `load`: never let a corrupt recents file silently
            // become an empty list that the next write then makes permanent.
            Err(error) => {
                quarantine_corrupt(&path, &error);
                Vec::new()
            }
        };
        workspaces.sort_by_key(|workspace| Reverse(workspace.last_opened_at));
        Ok(workspaces)
    }

    pub fn record_recent_workspace(
        &self,
        workspace: &WorkspaceInfo,
    ) -> AppResult<Vec<RecentWorkspace>> {
        let mut workspaces = self.recent_workspaces()?;
        let workspace_root = workspace
            .root
            .canonicalize()
            .unwrap_or_else(|_| workspace.root.clone());
        workspaces.retain(|candidate| candidate.root != workspace_root);
        workspaces.insert(
            0,
            RecentWorkspace {
                name: workspace.name.clone(),
                root: workspace_root,
                last_opened_at: Utc::now(),
            },
        );
        workspaces.truncate(MAX_RECENT_WORKSPACES);
        self.write_recent_workspaces(&workspaces)?;
        Ok(workspaces)
    }

    pub fn forget_recent_workspace(&self, root: PathBuf) -> AppResult<Vec<RecentWorkspace>> {
        let root = root.canonicalize().unwrap_or(root);
        let mut workspaces = self.recent_workspaces()?;
        workspaces.retain(|candidate| candidate.root != root);
        self.write_recent_workspaces(&workspaces)?;
        Ok(workspaces)
    }

    fn write_recent_workspaces(&self, workspaces: &[RecentWorkspace]) -> AppResult<()> {
        let path = self.recent_workspaces_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_atomic(&path, serde_json::to_string_pretty(workspaces)?.as_bytes())?;
        Ok(())
    }

    fn recent_workspaces_path(&self) -> AppResult<PathBuf> {
        let parent = self.path.parent().ok_or_else(|| {
            AppError::InvalidPath(format!("{} has no parent directory", self.path.display()))
        })?;
        Ok(parent.join(RECENT_WORKSPACES_FILE))
    }
}

/// Returns the path for workspace-scoped settings: `<root>/.lux/settings.json`.
fn workspace_settings_path(root: &Path) -> PathBuf {
    root.join(WORKSPACE_SETTINGS_DIR)
        .join(WORKSPACE_SETTINGS_FILE)
}

/// Per-process sequence making each temporary file name unique (see [`write_atomic`]).
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Writes `contents` to `path` atomically by streaming to a sibling temporary
/// file, flushing it to disk, then renaming over the target. Same-directory
/// rename replaces the destination atomically (Windows `MoveFileEx` with
/// `MOVEFILE_REPLACE_EXISTING`), so an interrupted write can never leave the
/// target truncated.
///
/// The temp file name is unique per writer (`<path>.<pid>.<seq>.tmp`): a fixed
/// `.tmp` would let two concurrent instances writing the same settings clobber
/// each other's in-progress file and install a torn result. Each writer now owns
/// its own scratch file and the rename makes last-write-win cleanly.
fn write_atomic(path: &Path, contents: &[u8]) -> AppResult<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Moves a corrupt JSON file aside to `<path>.corrupt-<unix_ts>-<pid>-<seq>` so
/// a parse failure can never trigger silent, permanent data loss: the original
/// bytes are preserved for recovery, and the caller starts from a clean default
/// that will be written to the now-vacant original path.
///
/// Returns `true` if the rename succeeded (caller may write new data to `path`),
/// `false` if it failed (caller must treat the file as read-only to avoid clobbering
/// the still-present corrupt bytes).
fn quarantine_corrupt(path: &Path, error: &serde_json::Error) -> bool {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    // Include PID + sequence to avoid collisions (same-second crashes, parallel
    // processes, or a previous rename that also failed at the same timestamp).
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();

    let mut backup = path.as_os_str().to_owned();
    backup.push(OsString::from(format!(".corrupt-{ts}-{pid}-{seq}")));
    let backup = PathBuf::from(backup);

    match fs::rename(path, &backup) {
        Ok(()) => {
            eprintln!(
                "lux-settings: {} is corrupt ({error}); backed up to {} and reset to defaults",
                path.display(),
                backup.display()
            );
            true
        }
        Err(rename_error) => {
            eprintln!(
                "lux-settings: {} is corrupt ({error}) and could not be backed up ({rename_error}); \
                 leaving it untouched — writes this session will go to a recovery sibling",
                path.display()
            );
            false
        }
    }
}

/// Fresh sibling path used when the original settings file is corrupt and
/// could not be quarantined. Writes land here rather than clobbering the bad file.
fn recovery_path(path: &Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = path.as_os_str().to_owned();
    name.push(OsString::from(format!(".recovery-{ts}-{seq}")));
    PathBuf::from(name)
}

#[must_use]
pub fn default_keybinding_profile() -> KeybindingProfile {
    KeybindingProfile {
        id: "default".to_string(),
        name: "Default".to_string(),
        bindings: vec![
            binding("workbench.action.showCommands", "Ctrl+Shift+P", None),
            binding("workbench.action.quickOpen", "Ctrl+P", None),
            binding("workbench.action.files.newUntitledFile", "Ctrl+N", None),
            binding("workbench.action.openSettings", "Ctrl+,", None),
            binding("workbench.action.openFolder", "Ctrl+O", None),
            binding(
                "workbench.action.toggleSidebar",
                "Ctrl+B",
                Some("workspace"),
            ),
            binding("workbench.view.explorer", "Ctrl+Shift+E", Some("workspace")),
            binding("workbench.view.search", "Ctrl+Shift+F", Some("workspace")),
            binding("workbench.view.scm", "Ctrl+Shift+G", Some("workspace")),
            binding("workbench.view.debug", "Ctrl+Shift+D", Some("workspace")),
            binding(
                "workbench.view.extensions",
                "Ctrl+Shift+X",
                Some("workspace"),
            ),
            binding("workbench.action.chat.toggle", "Ctrl+L", Some("workspace")),
            binding(
                "workbench.action.terminal.toggleTerminal",
                "Ctrl+`",
                Some("workspace"),
            ),
            binding("editor.action.toggleWordWrap", "Alt+Z", Some("editor")),
            binding(
                "editor.action.toggleMinimap",
                "Ctrl+M Ctrl+M",
                Some("editor"),
            ),
            binding("editor.action.fontZoomIn", "Ctrl+=", Some("editor")),
            binding("editor.action.fontZoomIn", "Ctrl+Shift+=", Some("editor")),
            binding("editor.action.fontZoomOut", "Ctrl+-", Some("editor")),
            binding("editor.action.fontZoomReset", "Ctrl+0", Some("editor")),
            binding("workbench.action.files.save", "Ctrl+S", Some("editor")),
            binding(
                "workbench.action.files.saveAs",
                "Ctrl+Shift+S",
                Some("editor"),
            ),
            binding(
                "workbench.action.files.saveAll",
                "Ctrl+K Ctrl+S",
                Some("dirtyEditors"),
            ),
            binding(
                "workbench.action.closeActiveEditor",
                "Ctrl+W",
                Some("editor"),
            ),
            binding(
                "workbench.action.splitEditorRight",
                "Ctrl+\\",
                Some("editor"),
            ),
            binding(
                "workbench.action.nextEditor",
                "Ctrl+PageDown",
                Some("editor"),
            ),
            binding(
                "workbench.action.previousEditor",
                "Ctrl+PageUp",
                Some("editor"),
            ),
        ],
    }
}

fn normalize_keybinding_profile(profile: KeybindingProfile) -> KeybindingProfile {
    let mut bindings = Vec::new();
    for binding in profile.bindings {
        let command = binding.command.trim();
        let key = normalize_key_sequence(&binding.key);
        if command.is_empty() || key.is_empty() {
            continue;
        }
        bindings.push(Keybinding {
            command: command.to_string(),
            key,
            when: binding.when.and_then(|value| {
                let value = value.trim();
                (!value.is_empty()).then(|| value.to_string())
            }),
        });
    }

    if bindings.is_empty() {
        return default_keybinding_profile();
    }

    if profile.id.trim().is_empty() || profile.id.trim() == "default" {
        for default_binding in default_keybinding_profile().bindings {
            if !bindings.iter().any(|binding| {
                binding.command == default_binding.command && binding.key == default_binding.key
            }) {
                bindings.push(default_binding);
            }
        }
    }

    KeybindingProfile {
        id: if profile.id.trim().is_empty() {
            "custom".to_string()
        } else {
            profile.id.trim().to_string()
        },
        name: if profile.name.trim().is_empty() {
            "Custom".to_string()
        } else {
            profile.name.trim().to_string()
        },
        bindings,
    }
}

fn normalize_key_sequence(value: &str) -> String {
    value
        .split_whitespace()
        .filter_map(normalize_key_chord)
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_key_chord(value: &str) -> Option<String> {
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut meta = false;
    let mut key = None;

    for part in value.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" | "win" | "super" => meta = true,
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" | "option" => alt = true,
            _ => key = Some(normalize_key_name(part)),
        }
    }

    let key = key?;
    let mut parts = Vec::with_capacity(5);
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if meta {
        parts.push("Meta".to_string());
    }
    parts.push(key);
    Some(parts.join("+"))
}

fn normalize_key_name(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "esc" => "Escape".to_string(),
        "space" => "Space".to_string(),
        "pgup" => "PageUp".to_string(),
        "pgdn" => "PageDown".to_string(),
        "left" => "ArrowLeft".to_string(),
        "right" => "ArrowRight".to_string(),
        "up" => "ArrowUp".to_string(),
        "down" => "ArrowDown".to_string(),
        key if key.len() == 1 => key.to_ascii_uppercase(),
        _ => {
            let mut chars = value.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        }
    }
}

fn binding(command: &str, key: &str, when: Option<&str>) -> Keybinding {
    Keybinding {
        command: command.to_string(),
        key: normalize_key_sequence(key),
        when: when.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lux_core::{WorkspaceId, WorkspaceInfo};
    use tempfile::tempdir;

    #[test]
    fn record_recent_workspace_deduplicates_and_orders_by_last_opened() {
        let temp = tempdir().expect("temp dir should be created");
        let first_root = temp.path().join("first");
        let second_root = temp.path().join("second");
        fs::create_dir_all(&first_root).expect("first root should be created");
        fs::create_dir_all(&second_root).expect("second root should be created");
        let store =
            SettingsStore::load(temp.path().join("settings.json")).expect("settings should load");

        store
            .record_recent_workspace(&workspace("first", first_root.clone()))
            .expect("first workspace should record");
        store
            .record_recent_workspace(&workspace("second", second_root))
            .expect("second workspace should record");
        let workspaces = store
            .record_recent_workspace(&workspace("first", first_root.clone()))
            .expect("first workspace should update");

        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0].name, "first");
        assert_eq!(
            workspaces[0].root,
            first_root
                .canonicalize()
                .expect("first root should canonicalize")
        );
        assert_eq!(workspaces[1].name, "second");
    }

    #[test]
    fn forget_recent_workspace_removes_only_matching_root() {
        let temp = tempdir().expect("temp dir should be created");
        let first_root = temp.path().join("first");
        let second_root = temp.path().join("second");
        fs::create_dir_all(&first_root).expect("first root should be created");
        fs::create_dir_all(&second_root).expect("second root should be created");
        let store =
            SettingsStore::load(temp.path().join("settings.json")).expect("settings should load");
        store
            .record_recent_workspace(&workspace("first", first_root.clone()))
            .expect("first workspace should record");
        store
            .record_recent_workspace(&workspace("second", second_root))
            .expect("second workspace should record");

        let workspaces = store
            .forget_recent_workspace(first_root)
            .expect("first workspace should be forgotten");

        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].name, "second");
    }

    #[test]
    fn recent_workspaces_persist_across_store_reload() {
        let temp = tempdir().expect("temp dir should be created");
        let settings_path = temp.path().join("settings.json");
        let root = temp.path().join("project");
        fs::create_dir_all(&root).expect("workspace root should be created");
        let store = SettingsStore::load(settings_path.clone()).expect("settings should load");
        store
            .record_recent_workspace(&workspace("project", root.clone()))
            .expect("workspace should record");

        let reloaded = SettingsStore::load(settings_path).expect("settings should reload");
        let workspaces = reloaded
            .recent_workspaces()
            .expect("recent workspaces should load");

        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].name, "project");
        assert_eq!(
            workspaces[0].root,
            root.canonicalize().expect("root should canonicalize")
        );
    }

    #[test]
    fn recent_workspaces_are_capped_to_limit() {
        let temp = tempdir().expect("temp dir should be created");
        let store =
            SettingsStore::load(temp.path().join("settings.json")).expect("settings should load");

        for index in 0..(MAX_RECENT_WORKSPACES + 3) {
            let root = temp.path().join(format!("project-{index}"));
            fs::create_dir_all(&root).expect("workspace root should be created");
            store
                .record_recent_workspace(&workspace(&format!("project-{index}"), root))
                .expect("workspace should record");
        }

        let workspaces = store
            .recent_workspaces()
            .expect("recent workspaces should load");

        assert_eq!(workspaces.len(), MAX_RECENT_WORKSPACES);
        assert!(workspaces
            .iter()
            .all(|workspace| workspace.name != "project-0"));
        assert!(workspaces
            .iter()
            .any(|workspace| workspace.name == format!("project-{}", MAX_RECENT_WORKSPACES + 2)));
    }

    #[test]
    fn keybinding_profile_defaults_to_cursor_like_workbench_bindings() {
        let temp = tempdir().expect("temp dir should be created");
        let store =
            SettingsStore::load(temp.path().join("settings.json")).expect("settings should load");

        let profile = store.keybinding_profile();

        assert_eq!(profile.id, "default");
        assert!(profile
            .bindings
            .iter()
            .any(|binding| binding.command == "workbench.action.quickOpen"
                && binding.key == "Ctrl+P"));
        assert!(profile.bindings.iter().any(|binding| binding.command
            == "workbench.action.toggleSidebar"
            && binding.key == "Ctrl+B"));
        assert!(profile.bindings.iter().any(|binding| binding.command
            == "workbench.action.files.saveAll"
            && binding.key == "Ctrl+K Ctrl+S"));
        assert!(profile.bindings.iter().any(|binding| binding.command
            == "editor.action.fontZoomOut"
            && binding.key == "Ctrl+-"));
    }

    #[test]
    fn keybinding_profile_persists_normalized_custom_bindings() {
        let temp = tempdir().expect("temp dir should be created");
        let settings_path = temp.path().join("settings.json");
        let mut store = SettingsStore::load(settings_path.clone()).expect("settings should load");

        store
            .set_keybinding_profile(KeybindingProfile {
                id: "  team  ".to_string(),
                name: "  Team  ".to_string(),
                bindings: vec![
                    Keybinding {
                        command: " workbench.action.quickOpen ".to_string(),
                        key: "ctrl+p".to_string(),
                        when: Some(" workspace ".to_string()),
                    },
                    Keybinding {
                        command: "workbench.action.files.saveAll".to_string(),
                        key: "ctrl+k ctrl+s".to_string(),
                        when: Some(String::new()),
                    },
                ],
            })
            .expect("profile should persist");

        let reloaded = SettingsStore::load(settings_path).expect("settings should reload");
        let profile = reloaded.keybinding_profile();

        assert_eq!(profile.id, "team");
        assert_eq!(profile.name, "Team");
        assert_eq!(profile.bindings[0].command, "workbench.action.quickOpen");
        assert_eq!(profile.bindings[0].key, "Ctrl+P");
        assert_eq!(profile.bindings[0].when.as_deref(), Some("workspace"));
        assert_eq!(profile.bindings[1].key, "Ctrl+K Ctrl+S");
        assert_eq!(profile.bindings[1].when, None);
    }

    #[test]
    fn default_keybinding_profile_merges_new_defaults_from_saved_profiles() {
        let profile = normalize_keybinding_profile(KeybindingProfile {
            id: "default".to_string(),
            name: "Default".to_string(),
            bindings: vec![Keybinding {
                command: "workbench.action.quickOpen".to_string(),
                key: "Ctrl+P".to_string(),
                when: None,
            }],
        });

        assert!(profile.bindings.iter().any(|binding| binding.command
            == "workbench.action.toggleSidebar"
            && binding.key == "Ctrl+B"));
        assert!(profile.bindings.iter().any(|binding| binding.command
            == "editor.action.fontZoomOut"
            && binding.key == "Ctrl+-"));
    }

    #[test]
    fn corrupt_settings_are_quarantined_not_silently_wiped() {
        let temp = tempdir().expect("temp dir should be created");
        let settings_path = temp.path().join("settings.json");
        fs::write(&settings_path, b"{ this is not valid json")
            .expect("corrupt settings should be written");

        // Loading must not panic, and must not leave the corrupt file in place to be
        // blindly overwritten: it is moved aside so the bytes stay recoverable.
        let store = SettingsStore::load(settings_path).expect("corrupt load should recover");
        assert!(store.get(SettingsScope::User, "any.key").is_none());

        let backups: Vec<_> = fs::read_dir(temp.path())
            .expect("temp dir should be readable")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("settings.json.corrupt-")
            })
            .collect();
        assert_eq!(
            backups.len(),
            1,
            "corrupt file should be backed up exactly once"
        );
        assert_eq!(
            fs::read(backups[0].path()).expect("backup should be readable"),
            b"{ this is not valid json",
            "backup must preserve the original corrupt bytes"
        );
    }

    #[test]
    fn corrupt_recent_workspaces_are_quarantined_not_silently_wiped() {
        let temp = tempdir().expect("temp dir should be created");
        let recents_path = temp.path().join(RECENT_WORKSPACES_FILE);
        fs::write(&recents_path, b"]not json[").expect("corrupt recents should be written");
        let store =
            SettingsStore::load(temp.path().join("settings.json")).expect("settings should load");

        let workspaces = store
            .recent_workspaces()
            .expect("corrupt recents should recover to empty");
        assert!(workspaces.is_empty());

        let backup_exists = fs::read_dir(temp.path())
            .expect("temp dir should be readable")
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("recent-workspaces.json.corrupt-")
            });
        assert!(backup_exists, "corrupt recents file should be backed up");
    }

    fn workspace(name: &str, root: PathBuf) -> WorkspaceInfo {
        WorkspaceInfo {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: name.to_string(),
            root,
            opened_at: Utc::now(),
        }
    }
}
