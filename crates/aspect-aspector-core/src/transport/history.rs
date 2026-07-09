use std::path::{Path, PathBuf};

use chrono::Utc;

use super::types::{AiChatHistoryDocument, AiChatHistoryResponse, AiChatHistorySaveRequest};

const HISTORY_FILE: &str = "ai-chat-history.json";
const HISTORY_SCHEMA_VERSION: u32 = 1;

/// Platform abstraction for path resolution.
pub trait PathProvider {
    fn data_dir(&self) -> Result<PathBuf, String>;
}

pub fn history_load(provider: &impl PathProvider) -> Result<AiChatHistoryResponse, String> {
    let path = history_path(provider)?;
    recover_history_temp_file(&path)?;
    if !path.exists() {
        return Ok(AiChatHistoryResponse {
            schema_version: HISTORY_SCHEMA_VERSION,
            active_session_id: String::new(),
            sessions: Vec::new(),
            path,
            recovered: false,
        });
    }

    let raw = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    match serde_json::from_str::<AiChatHistoryDocument>(&raw) {
        Ok(document) => Ok(AiChatHistoryResponse {
            schema_version: document.schema_version,
            active_session_id: document.active_session_id,
            sessions: document.sessions,
            path,
            recovered: false,
        }),
        Err(error) => {
            let backup_path = path.with_extension(format!(
                "json.recovered-{}",
                Utc::now().format("%Y%m%d%H%M%S")
            ));
            let _ = std::fs::rename(&path, &backup_path);
            Err(format!(
                "AI chat history was corrupted and moved to {}: {error}",
                backup_path.display()
            ))
        }
    }
}

pub fn history_save(
    provider: &impl PathProvider,
    request: AiChatHistorySaveRequest,
) -> Result<AiChatHistoryResponse, String> {
    let path = history_path(provider)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let document = AiChatHistoryDocument {
        schema_version: HISTORY_SCHEMA_VERSION,
        active_session_id: request.active_session_id,
        sessions: request.sessions,
        updated_at: Utc::now(),
    };
    let serialized = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;

    let _guard = history_save_lock()
        .lock()
        .map_err(|_| "history save lock poisoned".to_string())?;

    let write_id = uuid::Uuid::new_v4().to_string();
    let temporary_path = history_temp_path(&path, &write_id);
    std::fs::write(&temporary_path, serialized).map_err(|error| error.to_string())?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temporary_path, &path).map_err(|error| error.to_string())?;

    Ok(AiChatHistoryResponse {
        schema_version: document.schema_version,
        active_session_id: document.active_session_id,
        sessions: document.sessions,
        path,
        recovered: false,
    })
}

fn history_path(provider: &impl PathProvider) -> Result<PathBuf, String> {
    Ok(provider.data_dir()?.join(HISTORY_FILE))
}

fn history_save_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn history_temp_path(path: &Path, id: &str) -> PathBuf {
    path.with_extension(format!("{id}.tmp"))
}

#[allow(clippy::unnecessary_wraps)]
fn recover_history_temp_file(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("ai-chat-history");
    let candidate = std::fs::read_dir(dir).ok().and_then(|entries| {
        entries
            .filter_map(std::result::Result::ok)
            .filter(|e| {
                let name = e.file_name();
                let s = name.to_string_lossy();
                s.starts_with(stem) && s.ends_with(".tmp")
            })
            .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
    });
    if let Some(entry) = candidate {
        let _ = std::fs::rename(entry.path(), path);
    }
    Ok(())
}
