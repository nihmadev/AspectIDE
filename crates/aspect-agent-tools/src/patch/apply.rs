use std::path::Path;

use crate::types::file_patch::*;
use crate::atomic_write::ai_atomic_write;

pub async fn apply_patch_to_disk(
    operations: &[AiPreparedPatchOperation],
    save_to_disk: bool,
    rollback: &mut Vec<AiPatchRollbackEntry>,
) -> Result<(), String> {
    if !save_to_disk {
        return Ok(());
    }

    for operation in operations {
        let previous_bytes = read_backup_bytes(&operation.path).await;
        rollback.push(AiPatchRollbackEntry {
            path: operation.path.clone(),
            previous_bytes,
        });

        match operation.kind {
            AiPreparedPatchKind::Create
            | AiPreparedPatchKind::Rewrite
            | AiPreparedPatchKind::Replace => {
                let text = operation.after_text.as_deref().unwrap_or_default();
                if let Some(parent) = operation.path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|error| error.to_string())?;
                }
                ai_atomic_write(&operation.path, text.as_bytes().to_vec()).await?;
            }
            AiPreparedPatchKind::Delete => {
                tokio::fs::remove_file(&operation.path)
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

async fn read_backup_bytes(path: &Path) -> Option<Vec<u8>> {
    tokio::fs::read(path).await.ok()
}
