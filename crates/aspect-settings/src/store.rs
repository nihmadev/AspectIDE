use std::{
    cmp::Reverse,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use aspect_core::{
    AppError, AppResult, KeybindingProfile, RecentWorkspace, SettingValue, SettingsScope,
    WorkspaceInfo,
};
use serde_json::Value;

use crate::io::{quarantine_corrupt, recovery_path, write_atomic};
use crate::keybindings::{default_keybinding_profile, normalize_keybinding_profile};

pub const RECENT_WORKSPACES_FILE: &str = "recent-workspaces.json";
pub const MAX_RECENT_WORKSPACES: usize = 12;
const KEYBINDINGS_KEY: &str = "workbench.keybindings";

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
                let ws_path = crate::io::workspace_settings_path(&root);
                if let Ok(ws_store) = Self::load(ws_path) {
                    if let Some(val) = ws_store.values.get(key).cloned() {
                        return Some(val);
                    }
                }
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
                let ws_path = crate::io::workspace_settings_path(&root);
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
        crate::io::workspace_settings_path(root)
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
