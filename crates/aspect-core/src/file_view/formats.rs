use std::path::Path;

use crate::{
    FileOpenMode, FileViewCategory, FileViewDescriptor, FileViewStrategy,
    FileFormatSupport,
};

struct FileFormatSpec {
    display_name: &'static str,
    category: FileViewCategory,
    strategy: FileViewStrategy,
    mode: FileOpenMode,
    mime_type: Option<&'static str>,
    extensions: &'static [&'static str],
    exact_names: &'static [&'static str],
    binary: bool,
    ai_readable: bool,
    notes: &'static [&'static str],
}

impl FileFormatSpec {
    fn descriptor(&self) -> FileViewDescriptor {
        FileViewDescriptor {
            category: self.category,
            strategy: self.strategy,
            mode: self.mode,
            display_name: self.display_name.to_string(),
            mime_type: self.mime_type.map(ToOwned::to_owned),
            extensions: self
                .extensions
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            editable: self.mode == FileOpenMode::EditableText,
            previewable: self.strategy != FileViewStrategy::ExternalOnly,
            ai_readable: self.ai_readable,
            binary: self.binary,
            max_inline_bytes: if self.binary { None } else { Some(1_000_000) },
            notes: self
                .notes
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        }
    }
}

const FILE_FORMATS: &[FileFormatSpec] = &[
    spec("Plain text", FileViewCategory::Text, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("text/plain"), &["txt", "text", "log", "out", "err", "diff", "patch"], &[], false, true, &[]),
    spec("Markdown", FileViewCategory::Markdown, FileViewStrategy::MarkdownPreview, FileOpenMode::EditableText, Some("text/markdown"), &["md", "mdx", "markdown", "rst", "org"], &["readme", "license", "notice", "changelog"], false, true, &[]),
    spec("JSON", FileViewCategory::Config, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("application/json"), &["json", "jsonc", "json5", "jsonl", "ndjson", "geojson", "webmanifest"], &[], false, true, &[]),
    spec("YAML", FileViewCategory::Config, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("application/yaml"), &["yaml", "yml"], &[], false, true, &[]),
    spec("TOML", FileViewCategory::Config, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("application/toml"), &["toml", "tml"], &[], false, true, &[]),
    spec("Configuration", FileViewCategory::Config, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("text/plain"), &["ini", "cfg", "conf", "config", "properties", "props", "env", "editorconfig", "npmrc", "yarnrc", "prettierrc", "eslintrc", "babelrc", "browserslistrc", "gitignore", "gitattributes", "gitmodules", "lock", "mod", "sum"], &["dockerfile", "makefile", "rakefile", "gemfile", "procfile", "containerfile", "dockerignore", ".editorconfig", ".gitignore", ".gitattributes", ".gitmodules", ".npmrc", ".yarnrc", ".prettierrc", ".eslintrc", ".babelrc", ".browserslistrc", ".env", ".env.example", ".env.local", "cargo.lock", "pnpm-lock.yaml", "package-lock.json", "yarn.lock"], false, true, &[]),
    spec("XML", FileViewCategory::Data, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("application/xml"), &["xml", "xsd", "xsl", "xslt"], &[], false, true, &[]),
    spec("Delimited table", FileViewCategory::Spreadsheet, FileViewStrategy::TableEditor, FileOpenMode::EditableText, Some("text/csv"), &["csv", "tsv", "psv"], &[], false, true, &[]),
    spec("SQL", FileViewCategory::Data, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("application/sql"), &["sql", "ddl", "dml"], &[], false, true, &[]),
    spec("JavaScript", FileViewCategory::Code, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("text/javascript"), &["js", "jsx", "mjs", "cjs"], &[], false, true, &[]),
    spec("TypeScript", FileViewCategory::Code, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("text/typescript"), &["ts", "tsx", "mts", "cts", "d.ts", "d.mts", "d.cts"], &[], false, true, &[]),
    spec("Web", FileViewCategory::Code, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("text/html"), &["html", "htm", "css", "scss", "sass", "less", "vue", "svelte", "astro"], &[], false, true, &[]),
    spec("Shell", FileViewCategory::Code, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("text/x-shellscript"), &["sh", "bash", "zsh", "fish", "ksh", "ps1", "psm1", "psd1", "bat", "cmd"], &[], false, true, &[]),
    spec("Programming language", FileViewCategory::Code, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("text/plain"), &["rs", "go", "py", "pyw", "java", "kt", "kts", "c", "h", "cpp", "cc", "cxx", "hpp", "hh", "hxx", "cs", "fs", "fsx", "php", "rb", "lua", "swift", "scala", "dart", "ex", "exs", "erl", "hrl", "zig", "nim", "r", "jl", "hs", "lhs", "pl", "pm", "perl", "clj", "cljs", "cljc", "edn", "elm", "sol", "v", "sv", "svh", "asm", "s", "m", "mm", "gradle", "cmake"], &[], false, true, &[]),
    spec("Schema", FileViewCategory::Data, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("text/plain"), &["graphql", "gql", "proto", "prisma", "avsc", "thrift", "capnp"], &[], false, true, &[]),
    spec("Infrastructure", FileViewCategory::Config, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("text/plain"), &["tf", "tfvars", "hcl", "nomad", "dockerignore", "containerfile"], &[], false, true, &[]),
    spec("Spreadsheet", FileViewCategory::Spreadsheet, FileViewStrategy::SpreadsheetEditor, FileOpenMode::EditableText, None, &["xls", "xlsx", "xlsm", "xlsb", "ods", "fods"], &[], true, true, &["Legacy .xls files are edited as cell data and saved in the modern workbook format supported by the engine."]),
    spec("Database", FileViewCategory::Database, FileViewStrategy::DatabaseEditor, FileOpenMode::Preview, Some("application/vnd.sqlite3"), &["db", "db3", "sqlite", "sqlite2", "sqlite3", "s3db", "duckdb"], &[], true, true, &["SQLite databases open in the built-in table/SQL editor; DuckDB files are identified but require an external tool."]),
    spec("PDF", FileViewCategory::Pdf, FileViewStrategy::PdfPreview, FileOpenMode::Preview, Some("application/pdf"), &["pdf"], &[], true, true, &[]),
    spec("Office document", FileViewCategory::Office, FileViewStrategy::OfficePreview, FileOpenMode::Preview, None, &["doc", "docx", "docm", "dot", "dotx", "dotm", "rtf", "odt", "ott"], &[], true, true, &[]),
    spec("Presentation", FileViewCategory::Office, FileViewStrategy::OfficePreview, FileOpenMode::Preview, None, &["ppt", "pptx", "pptm", "pot", "potx", "potm", "pps", "ppsx", "odp", "otp"], &[], true, true, &[]),
    spec("Image", FileViewCategory::Image, FileViewStrategy::ImagePreview, FileOpenMode::Preview, None, &["png", "jpg", "jpeg", "jpe", "webp", "gif", "bmp", "dib", "ico", "icns", "avif", "tif", "tiff", "heic", "heif", "apng", "psd", "ai", "eps", "svg"], &[], true, true, &[]),
    spec("Audio", FileViewCategory::Audio, FileViewStrategy::AudioPreview, FileOpenMode::Preview, None, &["mp3", "wav", "flac", "ogg", "oga", "m4a", "aac", "opus", "wma", "aiff", "aif", "mid", "midi"], &[], true, true, &[]),
    spec("Video", FileViewCategory::Video, FileViewStrategy::VideoPreview, FileOpenMode::Preview, None, &["mp4", "m4v", "webm", "mov", "mkv", "avi", "wmv", "mpeg", "mpg", "3gp", "ogv"], &[], true, true, &[]),
    spec("Archive", FileViewCategory::Archive, FileViewStrategy::ArchivePreview, FileOpenMode::Preview, None, &["zip", "rar", "7z", "tar", "tar.gz", "tar.bz2", "tar.xz", "gz", "tgz", "tbz2", "txz", "bz2", "xz", "zst", "br", "jar", "war", "ear", "apk", "aab", "ipa", "vsix", "nupkg", "crate", "whl", "gem"], &[], true, true, &[]),
    spec("Notebook", FileViewCategory::Notebook, FileViewStrategy::MonacoText, FileOpenMode::EditableText, Some("application/x-ipynb+json"), &["ipynb"], &[], false, true, &[]),
    spec("Diagram", FileViewCategory::Diagram, FileViewStrategy::DiagramPreview, FileOpenMode::EditableText, Some("text/plain"), &["drawio", "dio", "excalidraw", "mermaid", "mmd", "puml", "plantuml", "dot", "gv", "vsdx"], &[], false, true, &[]),
    spec("Font", FileViewCategory::Font, FileViewStrategy::BinaryPreview, FileOpenMode::Preview, None, &["ttf", "otf", "woff", "woff2", "eot", "fon"], &[], true, false, &[]),
    spec("Executable", FileViewCategory::Executable, FileViewStrategy::BinaryPreview, FileOpenMode::Preview, None, &["exe", "dll", "so", "dylib", "bin", "msi", "deb", "rpm", "dmg", "iso", "appimage", "class", "o", "obj", "pdb", "lib"], &[], true, false, &[]),
    spec("Binary", FileViewCategory::Binary, FileViewStrategy::BinaryPreview, FileOpenMode::Preview, Some("application/octet-stream"), &["wasm", "dat", "pak", "blob", "cache", "tmp"], &[], true, false, &[]),
    spec("Certificate/key", FileViewCategory::Config, FileViewStrategy::MonacoText, FileOpenMode::ReadOnlyText, Some("text/plain"), &["pem", "crt", "cer", "csr", "key", "pub", "asc", "gpg", "pgp", "p12", "pfx"], &[], false, true, &["Sensitive material is opened read-only by default."]),
];

#[allow(
    clippy::too_many_arguments,
    reason = "Static file-format table entries are denser and clearer than builder boilerplate."
)]
const fn spec(
    display_name: &'static str,
    category: FileViewCategory,
    strategy: FileViewStrategy,
    mode: FileOpenMode,
    mime_type: Option<&'static str>,
    extensions: &'static [&'static str],
    exact_names: &'static [&'static str],
    binary: bool,
    ai_readable: bool,
    notes: &'static [&'static str],
) -> FileFormatSpec {
    FileFormatSpec {
        display_name,
        category,
        strategy,
        mode,
        mime_type,
        extensions,
        exact_names,
        binary,
        ai_readable,
        notes,
    }
}

fn longest_known_extension(lower_name: &str) -> Option<String> {
    lower_name.split('.').next()?;
    let parts = lower_name.split('.').collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    (1..parts.len())
        .map(|index| parts[index..].join("."))
        .find(|extension| {
            FILE_FORMATS
                .iter()
                .any(|spec| spec.extensions.contains(&extension.as_str()))
        })
}

#[must_use]
pub fn file_view_descriptor_for_path(path: &Path) -> FileViewDescriptor {
    let lower_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if let Some(spec) = FILE_FORMATS
        .iter()
        .find(|spec| spec.exact_names.contains(&lower_name.as_str()))
    {
        return spec.descriptor();
    }

    let extension = longest_known_extension(&lower_name).unwrap_or_else(|| {
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
    });
    FILE_FORMATS
        .iter()
        .find(|spec| spec.extensions.contains(&extension.as_str()))
        .map_or_else(
            || {
                let mut descriptor = FileViewDescriptor::default();
                if !extension.is_empty() {
                    descriptor.extensions = vec![extension];
                }
                descriptor
            },
            FileFormatSpec::descriptor,
        )
}

#[must_use]
pub fn supported_file_formats() -> Vec<FileFormatSupport> {
    FILE_FORMATS
        .iter()
        .flat_map(|spec| {
            spec.extensions.iter().map(move |extension| {
                let descriptor = spec.descriptor();
                FileFormatSupport {
                    extension: (*extension).to_string(),
                    category: descriptor.category,
                    strategy: descriptor.strategy,
                    mode: descriptor.mode,
                    display_name: descriptor.display_name,
                    mime_type: descriptor.mime_type,
                    editable: descriptor.editable,
                    previewable: descriptor.previewable,
                    ai_readable: descriptor.ai_readable,
                    binary: descriptor.binary,
                }
            })
        })
        .collect()
}

#[must_use]
pub fn file_extension_for_path(path: &Path) -> String {
    let lower_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    longest_known_extension(&lower_name).unwrap_or_else(|| {
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
    })
}

#[must_use]
pub const fn is_editor_text_mode(mode: FileOpenMode) -> bool {
    matches!(
        mode,
        FileOpenMode::EditableText | FileOpenMode::ReadOnlyText
    )
}

#[must_use]
pub fn monaco_language_id_for_path(path: &Path) -> String {
    let descriptor = file_view_descriptor_for_path(path);
    if descriptor.category == FileViewCategory::Markdown {
        return "markdown".to_string();
    }
    let lower_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(lower_name.as_str(), "dockerfile" | "containerfile") {
        return "dockerfile".to_string();
    }
    match file_extension_for_path(path).as_str() {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" | "d.ts" | "d.mts" | "d.cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "json" | "jsonc" | "json5" | "jsonl" | "ndjson" | "webmanifest" | "geojson" | "npmrc"
        | "yarnrc" | "prettierrc" | "eslintrc" | "babelrc" | "browserslistrc" | "excalidraw" => {
            "json"
        }
        "toml" | "tml" => "toml",
        "yaml" | "yml" => "yaml",
        "css" | "scss" | "sass" | "less" => "css",
        "html" | "htm" | "vue" | "svelte" | "astro" => "html",
        "sql" | "ddl" | "dml" => "sql",
        "xml" | "xsd" | "xsl" | "xslt" | "drawio" | "dio" | "vsdx" | "svg" => "xml",
        "csv" | "tsv" | "psv" => "csv",
        "graphql" | "gql" => "graphql",
        "proto" => "proto",
        "prisma" => "prisma",
        "py" | "pyw" => "python",
        "go" | "mod" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "c" | "h" => "c",
        "cs" => "csharp",
        "fs" | "fsx" => "fsharp",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "scala" => "scala",
        "dart" => "dart",
        "lua" => "lua",
        "zig" => "zig",
        "nim" => "nim",
        "r" => "r",
        "jl" => "julia",
        "hs" | "lhs" => "haskell",
        "clj" | "cljs" | "cljc" => "clojure",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "erlang",
        "pl" | "pm" | "perl" => "perl",
        "sh" | "bash" | "zsh" | "fish" | "ksh" => "shell",
        "ps1" | "psm1" | "psd1" => "powershell",
        "bat" | "cmd" => "bat",
        "tf" | "tfvars" | "hcl" | "nomad" => "hcl",
        "dockerfile" | "containerfile" => "dockerfile",
        "md" | "mdx" | "markdown" | "rst" | "org" | "mermaid" | "mmd" => "markdown",
        "ini" | "cfg" | "conf" | "config" | "properties" | "props" | "editorconfig"
        | "gitignore" | "gitattributes" | "gitmodules" => "ini",
        "env" => "properties",
        "dot" | "gv" | "puml" | "plantuml" | "log" | "out" | "err" | "diff" | "patch" | "lock"
        | "sum" => "plaintext",
        other if !other.is_empty() => other,
        _ => "plaintext",
    }
    .to_string()
}

