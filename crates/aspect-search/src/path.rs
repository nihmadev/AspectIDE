const IGNORED_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", "out", "coverage", ".git",
    ".next", ".turbo", "vendor", "venv", ".venv", "__pycache__",
];

const BINARY_EXTS: &[&str] = &[
    ".7z", ".avi", ".bmp", ".class", ".db", ".dll", ".dmg", ".exe", ".gif",
    ".gz", ".ico", ".jar", ".jpeg", ".jpg", ".lockb", ".mov", ".mp3", ".mp4",
    ".o", ".obj", ".pdf", ".png", ".rar", ".so", ".tar", ".ttf", ".webm",
    ".webp", ".woff", ".woff2", ".zip",
];

const SOURCE_EXTS: &[&str] = &[
    ".astro", ".c", ".cc", ".cpp", ".cs", ".css", ".cxx", ".go", ".graphql",
    ".gql", ".h", ".hpp", ".html", ".java", ".js", ".json", ".jsx", ".kt",
    ".kts", ".less", ".md", ".mdx", ".mjs", ".mts", ".php", ".proto", ".py",
    ".rb", ".rs", ".sass", ".scss", ".sql", ".svelte", ".swift", ".toml",
    ".ts", ".tsx", ".vue", ".xml", ".yaml", ".yml",
];

const FAMILY_SUFFIXES: &[&str] = &[
    "test", "spec", "stories", "story", "module", "types", "schema", "route",
    "routes", "model", "models", "entity", "entities", "service", "controller",
    "view", "styles", "style", "component", "page", "layout", "hook", "hooks",
    "util", "utils", "helper", "helpers",
];

pub fn normalize_slashes(value: &str) -> String {
    value.replace('\\', "/")
}

pub fn file_extension(basename_lower: &str) -> String {
    for special in [".d.ts", ".d.mts", ".d.cts"] {
        if basename_lower.ends_with(special) {
            return special.to_string();
        }
    }
    match basename_lower.rfind('.') {
        Some(dot) if dot > 0 => basename_lower[dot..].to_string(),
        _ => String::new(),
    }
}

pub fn family_stem(basename: &str) -> String {
    let lower = basename.to_lowercase();
    let ext = file_extension(&lower);
    let src: &str = if basename.len() == lower.len() {
        basename
    } else {
        &lower
    };
    let mut stem = src[..src.len().saturating_sub(ext.len())].to_string();
    for _ in 0..2 {
        let stem_lower = stem.to_lowercase();
        let mut stripped = false;
        for suffix in FAMILY_SUFFIXES {
            for delim in ['.', '-', '_'] {
                let tail = format!("{delim}{suffix}");
                if stem_lower.ends_with(&tail) {
                    stem = stem[..stem.len() - tail.len()].to_string();
                    stripped = true;
                    break;
                }
            }
            if stripped {
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    stem
}

pub fn score_path(path: &str) -> i64 {
    let lower = path.to_lowercase().replace('\\', "/");
    let mut score = 0i64;
    if lower.ends_with("package.json")
        || lower.ends_with("cargo.toml")
        || lower.contains("vite.config.")
        || lower.contains("tsconfig.")
        || lower.contains("readme")
        || lower.contains("src/app.")
        || lower.contains("src/main.")
        || lower.contains("src-tauri/src/lib.rs")
    {
        score += 100;
    }
    let in_src = lower == "src"
        || lower.starts_with("src/")
        || lower.contains("/src/")
        || lower.contains("/src-tauri/src/");
    if in_src {
        score += 25;
    }
    if lower.contains("/components/") || lower.starts_with("components/") {
        score += 10;
    }
    let is_artifact = lower.contains("/node_modules/")
        || lower.starts_with("node_modules/")
        || lower.contains("/target/")
        || lower.starts_with("target/")
        || lower.contains("/dist/")
        || lower.starts_with("dist/");
    if is_artifact {
        score -= 200;
    }
    score
}

pub fn is_low_signal_path(path: &str) -> bool {
    let lower = normalize_slashes(path).to_lowercase();
    if lower
        .split('/')
        .any(|segment| IGNORED_DIRS.contains(&segment))
    {
        return true;
    }
    if BINARY_EXTS.iter().any(|ext| lower.ends_with(ext)) {
        return true;
    }
    !is_source_path(&lower) && !is_extensionless_project_file(&lower)
}

fn is_source_path(lower: &str) -> bool {
    SOURCE_EXTS.iter().any(|ext| lower.ends_with(ext))
}

fn is_extensionless_project_file(lower_path: &str) -> bool {
    let basename = lower_path.rsplit('/').next().unwrap_or(lower_path);
    matches!(
        basename,
        "dockerfile"
            | "makefile"
            | "readme"
            | "license"
            | "notice"
            | "procfile"
            | "gemfile"
            | "rakefile"
    )
}

pub fn language_for_path(lower: &str) -> String {
    let lang = if ends_with_any(lower, &[".tsx", ".ts", ".mts", ".cts"]) {
        "typescript"
    } else if ends_with_any(lower, &[".jsx", ".js", ".mjs", ".cjs"]) {
        "javascript"
    } else if ends_with_any(lower, &[".rs"]) {
        "rust"
    } else if ends_with_any(lower, &[".py"]) {
        "python"
    } else if ends_with_any(lower, &[".go"]) {
        "go"
    } else if ends_with_any(lower, &[".java", ".kt", ".kts"]) {
        "jvm"
    } else if ends_with_any(lower, &[".cs"]) {
        "csharp"
    } else if ends_with_any(lower, &[".css", ".scss", ".sass", ".less"]) {
        "styles"
    } else if ends_with_any(lower, &[".json", ".yaml", ".yml", ".toml", ".xml"]) {
        "config-data"
    } else if ends_with_any(lower, &[".md", ".mdx"])
        || lower.contains("readme")
        || lower.contains("license")
        || lower.contains("notice")
    {
        "docs"
    } else if ends_with_any(lower, &[".html", ".vue", ".svelte", ".astro"]) {
        "web"
    } else if ends_with_any(lower, &[".sql", ".graphql", ".gql", ".proto"]) {
        "schema"
    } else {
        "other"
    };
    lang.to_string()
}

fn ends_with_any(value: &str, exts: &[&str]) -> bool {
    exts.iter().any(|ext| value.ends_with(ext))
}

pub fn split_delims(value: &str) -> Vec<String> {
    value
        .split(['.', '_', '-'])
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
