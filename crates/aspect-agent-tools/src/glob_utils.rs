use std::path::Path;

use crate::types::glob_result::AiGlobResult;

pub async fn ai_glob(
    root: &Path,
    pattern: &str,
    max_results: Option<usize>,
) -> Result<AiGlobResult, String> {
    let max = max_results.unwrap_or(80).clamp(1, 500);
    let pattern = pattern.trim().replace('\\', "/");
    if pattern.is_empty() {
        return Ok(AiGlobResult {
            pattern,
            count: 0,
            files: Vec::new(),
            truncated: false,
        });
    }

    let has_glob_meta = pattern.contains(['*', '?', '[', ']', '{', '}']);
    let glob_matcher = if has_glob_meta {
        let mut builder = globset::GlobSetBuilder::new();
        let add = |b: &mut globset::GlobSetBuilder, raw: &str| -> Result<(), String> {
            let glob = globset::GlobBuilder::new(raw)
                .case_insensitive(true)
                .literal_separator(false)
                .build()
                .map_err(|e| format!("Invalid glob pattern `{raw}`: {e}"))?;
            b.add(glob);
            Ok(())
        };
        add(&mut builder, &pattern)?;
        if !pattern.contains('/') {
            add(&mut builder, &format!("**/{pattern}"))?;
        }
        Some(
            builder
                .build()
                .map_err(|e| format!("Invalid glob pattern: {e}"))?,
        )
    } else {
        None
    };

    let root_for_walk = root.to_path_buf();
    let substring_pattern = pattern.to_lowercase();
    let files: Vec<std::path::PathBuf> = tokio::task::spawn_blocking(move || {
        aspect_fs::list_files_matching(
            root_for_walk.clone(),
            move |path| {
                let relative = path
                    .strip_prefix(&root_for_walk)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");
                glob_matcher.as_ref().map_or_else(
                    || relative.to_lowercase().contains(&substring_pattern),
                    |matcher| matcher.is_match(relative.as_str()),
                )
            },
            max,
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?
    .into_iter()
    .map(|e| e.path)
    .collect();

    Ok(AiGlobResult {
        count: files.len(),
        truncated: files.len() >= max,
        files,
        pattern,
    })
}
