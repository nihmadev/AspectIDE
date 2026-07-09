use std::path::Path;

use aspect_core::AppResult;
use globset::{Glob, GlobSet, GlobSetBuilder};

pub fn compile_globs(patterns: &[String]) -> AppResult<Option<GlobSet>> {
    let mut builder = GlobSetBuilder::new();
    let mut has_patterns = false;
    for pattern in patterns
        .iter()
        .map(|pattern| pattern.trim())
        .filter(|pattern| !pattern.is_empty())
    {
        builder.add(Glob::new(pattern)?);
        if !pattern.contains('/') && !pattern.contains('\\') {
            builder.add(Glob::new(&format!("**/{pattern}"))?);
        }
        has_patterns = true;
    }
    Ok(if has_patterns {
        Some(builder.build()?)
    } else {
        None
    })
}

pub fn matches_glob_filters(
    root: &Path,
    path: &Path,
    include_globs: Option<&GlobSet>,
    exclude_globs: Option<&GlobSet>,
) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if exclude_globs.is_some_and(|globs| globs.is_match(relative)) {
        return false;
    }
    include_globs.is_none_or(|globs| globs.is_match(relative))
}
