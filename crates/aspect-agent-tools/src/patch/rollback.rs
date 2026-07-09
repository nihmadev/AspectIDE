use crate::types::file_patch::AiPatchRollbackEntry;
use crate::atomic_write::ai_atomic_write;

pub async fn rollback_patch(entries: Vec<AiPatchRollbackEntry>) {
    for entry in entries {
        let result = match entry.previous_bytes {
            Some(bytes) => ai_atomic_write(&entry.path, bytes).await,
            None => {
                let _ = tokio::fs::remove_file(&entry.path).await;
                Ok(())
            }
        };
        if let Err(error) = result {
            tracing::warn!(%error, path = %entry.path.display(), "patch rollback failed");
        }
    }
}
