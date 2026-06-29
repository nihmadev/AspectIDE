//! File-view catalog: how each file format is presented, edited, previewed, and
//! exposed to the AI.
//!
//! This module owns the static format table ([`FILE_FORMATS`]) and the descriptor
//! types ([`FileViewDescriptor`], [`FilePreview`], the `*Preview` rows, …) plus the
//! path→view resolution helpers. It was extracted from the crate root so the schema
//! god-file no longer mixes file-format logic with workspace/git/lsp/debug types;
//! the public API is unchanged because `lib.rs` re-exports this module's items
//! (`pub use file_view::*;`).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Typed IPC descriptor mirrors independent frontend capabilities."
)]
pub struct FileViewDescriptor {
    pub category: FileViewCategory,
    pub strategy: FileViewStrategy,
    pub mode: FileOpenMode,
    pub display_name: String,
    pub mime_type: Option<String>,
    pub extensions: Vec<String>,
    pub editable: bool,
    pub previewable: bool,
    pub ai_readable: bool,
    pub binary: bool,
    #[ts(type = "number | null")]
    pub max_inline_bytes: Option<u64>,
    pub notes: Vec<String>,
}

impl Default for FileViewDescriptor {
    fn default() -> Self {
        Self {
            category: FileViewCategory::Text,
            strategy: FileViewStrategy::MonacoText,
            mode: FileOpenMode::EditableText,
            display_name: "Text".to_string(),
            mime_type: Some("text/plain".to_string()),
            extensions: Vec::new(),
            editable: true,
            previewable: true,
            ai_readable: true,
            binary: false,
            max_inline_bytes: Some(1_000_000),
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileViewCategory {
    Text,
    Code,
    Markdown,
    Config,
    Data,
    Spreadsheet,
    Database,
    Pdf,
    Office,
    Image,
    Audio,
    Video,
    Archive,
    Notebook,
    Diagram,
    Font,
    Executable,
    Binary,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileViewStrategy {
    MonacoText,
    MarkdownPreview,
    TablePreview,
    SpreadsheetPreview,
    SpreadsheetEditor,
    TableEditor,
    DatabasePreview,
    DatabaseEditor,
    PdfPreview,
    OfficePreview,
    ImagePreview,
    AudioPreview,
    VideoPreview,
    ArchivePreview,
    NotebookPreview,
    DiagramPreview,
    BinaryPreview,
    ExternalOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileOpenMode {
    EditableText,
    ReadOnlyText,
    Preview,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Typed IPC catalog row exposes independent frontend capability flags."
)]
pub struct FileFormatSupport {
    pub extension: String,
    pub category: FileViewCategory,
    pub strategy: FileViewStrategy,
    pub mode: FileOpenMode,
    pub display_name: String,
    pub mime_type: Option<String>,
    pub editable: bool,
    pub previewable: bool,
    pub ai_readable: bool,
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileInspectionOptions {
    #[ts(type = "number")]
    pub max_text_bytes: u64,
    pub max_rows: usize,
    pub max_columns: usize,
    pub max_archive_entries: usize,
}

impl Default for FileInspectionOptions {
    fn default() -> Self {
        Self {
            max_text_bytes: 1_000_000,
            max_rows: 80,
            max_columns: 24,
            max_archive_entries: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileInspection {
    pub path: PathBuf,
    pub title: String,
    pub descriptor: FileViewDescriptor,
    pub metadata: FileMetadata,
    pub preview: FilePreview,
    pub ai_context: String,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileMetadata {
    #[ts(type = "number")]
    pub size: u64,
    pub modified_at: Option<DateTime<Utc>>,
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum FilePreview {
    Text {
        language_id: String,
        text: String,
        line_count: usize,
        truncated: bool,
    },
    Table {
        delimiter: String,
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        row_count: usize,
        truncated: bool,
    },
    Spreadsheet {
        sheets: Vec<SpreadsheetSheetPreview>,
        workbook_type: String,
        truncated: bool,
    },
    Database {
        tables: Vec<DatabaseTablePreview>,
        truncated: bool,
    },
    Pdf {
        text: String,
        page_count: Option<usize>,
        truncated: bool,
    },
    Office {
        text: String,
        parts: Vec<ArchiveEntryPreview>,
        truncated: bool,
    },
    Image {
        note: String,
    },
    Audio {
        note: String,
    },
    Video {
        note: String,
    },
    Archive {
        entries: Vec<ArchiveEntryPreview>,
        total_entries: usize,
        truncated: bool,
    },
    Notebook {
        cells: Vec<NotebookCellPreview>,
        cell_count: usize,
        truncated: bool,
    },
    Binary {
        hex: String,
        ascii: String,
        truncated: bool,
    },
    External {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpreadsheetSheetPreview {
    pub name: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub row_count: usize,
    pub column_count: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DatabaseTablePreview {
    pub name: String,
    pub kind: String,
    pub columns: Vec<DatabaseColumnPreview>,
    pub rows: Vec<Vec<String>>,
    pub row_count: Option<usize>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DatabaseColumnPreview {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub primary_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchiveEntryPreview {
    pub path: String,
    #[ts(type = "number")]
    pub compressed_size: u64,
    #[ts(type = "number")]
    pub uncompressed_size: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NotebookCellPreview {
    pub index: usize,
    pub cell_type: String,
    pub text: String,
    pub output_text: String,
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

#[cfg(test)]
mod file_view_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn delimited_tables_open_in_table_editor() {
        for file in ["report.csv", "data.tsv", "pipe.psv"] {
            let descriptor = file_view_descriptor_for_path(Path::new(file));
            assert_eq!(descriptor.strategy, FileViewStrategy::TableEditor);
            assert_eq!(descriptor.mode, FileOpenMode::EditableText);
            assert!(descriptor.editable);
        }
    }

    #[test]
    fn structured_text_formats_use_monaco_mode_not_preview() {
        for file in [
            "query.sql",
            "app.log",
            "config.yaml",
            "Cargo.toml",
            "schema.graphql",
            "diagram.mmd",
            ".env",
        ] {
            let descriptor = file_view_descriptor_for_path(Path::new(file));
            assert!(
                is_editor_text_mode(descriptor.mode),
                "{} should be editor text, got {:?}",
                file,
                descriptor.mode
            );
        }
    }

    #[test]
    fn spreadsheet_workbooks_open_as_editable() {
        for file in ["book.xlsx", "model.xlsm", "data.ods"] {
            let descriptor = file_view_descriptor_for_path(Path::new(file));
            assert_eq!(descriptor.strategy, FileViewStrategy::SpreadsheetEditor);
            assert_eq!(descriptor.mode, FileOpenMode::EditableText);
            assert!(descriptor.editable);
        }
    }

    #[test]
    fn binary_formats_stay_preview_only() {
        let descriptor = file_view_descriptor_for_path(Path::new("photo.png"));
        assert_eq!(descriptor.mode, FileOpenMode::Preview);
    }

    #[test]
    fn vector_images_open_as_image_preview() {
        let descriptor = file_view_descriptor_for_path(Path::new("icon.svg"));
        assert_eq!(descriptor.strategy, FileViewStrategy::ImagePreview);
        assert_eq!(descriptor.category, FileViewCategory::Image);
    }

    #[test]
    fn preview_media_and_documents_use_dedicated_strategies() {
        let cases = [
            (
                "readme.pdf",
                FileViewStrategy::PdfPreview,
                FileOpenMode::Preview,
            ),
            (
                "photo.webp",
                FileViewStrategy::ImagePreview,
                FileOpenMode::Preview,
            ),
            (
                "clip.mp4",
                FileViewStrategy::VideoPreview,
                FileOpenMode::Preview,
            ),
            (
                "song.mp3",
                FileViewStrategy::AudioPreview,
                FileOpenMode::Preview,
            ),
            (
                "app.db",
                FileViewStrategy::DatabaseEditor,
                FileOpenMode::Preview,
            ),
            (
                "workflow.mmd",
                FileViewStrategy::DiagramPreview,
                FileOpenMode::EditableText,
            ),
            (
                "bundle.tar.gz",
                FileViewStrategy::ArchivePreview,
                FileOpenMode::Preview,
            ),
            (
                "bundle.zip",
                FileViewStrategy::ArchivePreview,
                FileOpenMode::Preview,
            ),
            (
                "letter.docx",
                FileViewStrategy::OfficePreview,
                FileOpenMode::Preview,
            ),
            (
                "app.wasm",
                FileViewStrategy::BinaryPreview,
                FileOpenMode::Preview,
            ),
        ];
        for (file, strategy, mode) in cases {
            let descriptor = file_view_descriptor_for_path(Path::new(file));
            assert_eq!(descriptor.strategy, strategy, "{file}");
            assert_eq!(descriptor.mode, mode, "{file}");
        }
    }

    #[test]
    fn compound_typescript_extension_is_recognized() {
        let extension = file_extension_for_path(Path::new("component.d.ts"));
        assert_eq!(extension, "d.ts");
        let language = monaco_language_id_for_path(Path::new("component.d.ts"));
        assert_eq!(language, "typescript");
    }
}
