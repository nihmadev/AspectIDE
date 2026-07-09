use crate::path::split_delims;

const TEST_SEGMENT_WORDS: &[&str] = &["test", "spec", "tests", "specs"];
const TEST_DIR_WORDS: &[&str] = &["__tests__", "test", "tests", "spec", "specs"];

const IMPORTANT_FILE_NAMES: &[&str] = &[
    "package.json", "cargo.toml", "pyproject.toml", "go.mod", "pom.xml",
    "build.gradle", "dockerfile", "makefile", ".env.example",
];

const IMPORTANT_FILE_PREFIXES: &[&str] = &["vite.config.", "tsconfig.", "jsconfig."];

pub fn is_test_file(basename_lower: &str, relative_lower: &str) -> bool {
    let base_parts = split_delims(basename_lower);
    if base_parts
        .iter()
        .any(|p| TEST_SEGMENT_WORDS.contains(&p.as_str()))
    {
        return true;
    }
    relative_lower
        .split('/')
        .any(|segment| TEST_DIR_WORDS.contains(&segment))
}

pub fn is_important_project_file(relative_lower: &str) -> bool {
    for name in IMPORTANT_FILE_NAMES {
        if relative_lower == *name || relative_lower.ends_with(&format!("/{name}")) {
            return true;
        }
    }
    for prefix in IMPORTANT_FILE_PREFIXES {
        if relative_lower.starts_with(prefix)
            || relative_lower.contains(&format!("/{prefix}"))
        {
            return true;
        }
    }
    relative_lower.contains("readme")
}
