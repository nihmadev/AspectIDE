use std::path::{Path, PathBuf};

/// If `dir` contains exactly one entry and it is a directory, return it (used to
/// strip the single top-level folder inside Node archives).
pub async fn single_child_dir(dir: &Path) -> Result<Option<PathBuf>, String> {
    let mut entries = tokio::fs::read_dir(dir).await.map_err(|e| e.to_string())?;
    let mut found: Option<PathBuf> = None;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        if found.is_some() {
            return Ok(None);
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

/// A per-run-unique scratch name (`<stem>-<pid>-<nanos><suffix>`) for a download or
/// staging path under the runtime root.
pub fn unique_scratch_path(root: &Path, stem: &str, suffix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let pid = std::process::id();
    root.join(format!("{stem}-{pid}-{nanos}{suffix}"))
}

/// Atomically replace the managed directory `dest` with the freshly-staged
/// `staged` tree, never writing into a half-deleted destination.
pub async fn replace_runtime_dir(staged: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    if tokio::fs::metadata(dest).await.is_ok() {
        let tombstone = tombstone_path(dest);
        move_aside_with_retry(dest, &tombstone).await?;
        let _ = tokio::fs::remove_dir_all(&tombstone).await;
    }
    move_dir(staged, dest).await
}

fn tombstone_path(dest: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let name = dest.file_name().map_or_else(
        || "runtime".to_string(),
        |n| n.to_string_lossy().to_string(),
    );
    dest.with_file_name(format!(".{name}.tombstone-{nanos}"))
}

/// Rename `from` → `to`, retrying with exponential backoff to ride out transient
/// Windows file locks.
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
pub async fn sweep_tombstones(root: &Path) {
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

/// Tombstone-safe recursive removal: moves `dir` aside first, retrying through
/// transient Windows locks, rather than calling `remove_dir_all` directly.
pub async fn remove_dir_tombstoned(dir: &Path) -> Result<(), String> {
    if tokio::fs::metadata(dir).await.is_err() {
        return Ok(());
    }
    let tombstone = tombstone_path(dir);
    move_aside_with_retry(dir, &tombstone).await?;
    let _ = tokio::fs::remove_dir_all(&tombstone).await;
    Ok(())
}

/// Move `from` to `to`, falling back to recursive copy when a plain rename is not
/// possible (e.g. across volumes).
async fn move_dir(from: &Path, to: &Path) -> Result<(), String> {
    if tokio::fs::rename(from, to).await.is_ok() {
        return Ok(());
    }
    copy_dir_recursive(from, to).await
}

async fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
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
