#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

mod concurrency;
pub use concurrency::{resolve_scan_threads, scan_threads, set_scan_concurrency, ScanConcurrency};

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("glob error: {0}")]
    Glob(#[from] globset::Error),
    #[error("regex error: {0}")]
    Regex(#[from] regex::Error),
    #[error("service error: {0}")]
    Service(String),
}

impl From<AppError> for String {
    fn from(value: AppError) -> Self {
        value.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    pub name: String,
    pub root: PathBuf,
    pub opened_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecentWorkspace {
    pub name: String,
    pub root: PathBuf,
    pub last_opened_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceId(pub Uuid);

impl WorkspaceId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkspaceId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: FsEntryKind,
    #[ts(type = "number")]
    pub size: u64,
    pub modified_at: Option<DateTime<Utc>>,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FsEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BufferId(pub Uuid);

impl BufferId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BufferId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DocumentSnapshot {
    pub id: BufferId,
    pub path: Option<PathBuf>,
    pub title: String,
    pub language_id: String,
    pub text: String,
    pub view: FileViewDescriptor,
    #[ts(type = "number")]
    pub version: u64,
    pub is_dirty: bool,
    pub is_untitled: bool,
    pub opened_at: DateTime<Utc>,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TextEdit {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DocumentEditResult {
    pub id: BufferId,
    pub path: Option<PathBuf>,
    pub title: String,
    #[ts(type = "number")]
    pub version: u64,
    pub is_dirty: bool,
    pub is_untitled: bool,
}

impl From<&DocumentSnapshot> for DocumentEditResult {
    fn from(document: &DocumentSnapshot) -> Self {
        Self {
            id: document.id,
            path: document.path.clone(),
            title: document.title.clone(),
            version: document.version,
            is_dirty: document.is_dirty,
            is_untitled: document.is_untitled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[allow(clippy::struct_excessive_bools)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
    pub include_hidden: bool,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub max_results: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            whole_word: false,
            use_regex: false,
            include_hidden: false,
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
            max_results: 250,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchHit {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub match_length: usize,
    pub match_text: String,
    pub preview: String,
    pub preview_match_start: usize,
    pub preview_match_length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchResponse {
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub truncated: bool,
    #[ts(type = "number")]
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TerminalSessionInfo {
    pub id: Uuid,
    pub shell: String,
    pub cwd: PathBuf,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitFileStatus {
    pub path: PathBuf,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitStatus {
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<GitFileStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitDiffFile {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitDiff {
    pub files: Vec<GitDiffFile>,
    pub additions: u32,
    pub deletions: u32,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SettingsScope {
    User,
    Workspace(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SettingValue {
    pub key: String,
    #[ts(type = "unknown")]
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Keybinding {
    pub command: String,
    pub key: String,
    pub when: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KeybindingProfile {
    pub id: String,
    pub name: String,
    pub bindings: Vec<Keybinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub wasm_module: PathBuf,
    #[serde(default)]
    pub permissions: Vec<ExtensionHostPermission>,
    pub contributes: Vec<String>,
    #[serde(default)]
    pub commands: Vec<ExtensionCommandContribution>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionStatus {
    Discovered,
    Active,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionContributionKind {
    Commands,
    Themes,
    Keybindings,
    Languages,
    Grammars,
    Snippets,
    Views,
    Menus,
    Settings,
    Debuggers,
    Tasks,
    ProblemMatchers,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionContributionPoint {
    pub id: String,
    pub kind: ExtensionContributionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionCommandContribution {
    pub id: String,
    pub title: String,
    pub category: Option<String>,
    pub handler: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub wasm_module: PathBuf,
    pub permissions: Vec<ExtensionHostPermission>,
    pub contributes: Vec<String>,
    pub contribution_points: Vec<ExtensionContributionPoint>,
    pub commands: Vec<ExtensionCommandContribution>,
    pub status: ExtensionStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionWasmPreflight {
    pub module_path: PathBuf,
    #[ts(type = "number")]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionHostPermission {
    WorkspaceRead,
    WorkspaceWrite,
    NetworkAccess,
    ProcessSpawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionWasmImportKind {
    Function,
    Table,
    Memory,
    Global,
    Tag,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionWasmImport {
    pub module: String,
    pub name: String,
    pub kind: ExtensionWasmImportKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionWasmAbi {
    pub version: u32,
    pub entrypoint: String,
    pub required_exports: Vec<String>,
    pub optional_exports: Vec<String>,
    pub imports: Vec<ExtensionWasmImport>,
    pub exports_memory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionHostLimits {
    pub max_memory_pages: u32,
    #[ts(type = "number")]
    pub activation_timeout_ms: u64,
    #[ts(type = "number")]
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionHostActivationContract {
    pub abi: ExtensionWasmAbi,
    pub permissions: Vec<ExtensionHostPermission>,
    pub limits: ExtensionHostLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivationCandidate {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub wasm_module: PathBuf,
    pub contribution_points: Vec<ExtensionContributionPoint>,
    pub commands: Vec<ExtensionCommandContribution>,
    pub wasm_preflight: ExtensionWasmPreflight,
    pub host_contract: ExtensionHostActivationContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivationBlocked {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub wasm_module: PathBuf,
    pub contribution_points: Vec<ExtensionContributionPoint>,
    pub commands: Vec<ExtensionCommandContribution>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivationPlan {
    pub candidates: Vec<ExtensionActivationCandidate>,
    pub blocked: Vec<ExtensionActivationBlocked>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivated {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub wasm_module: PathBuf,
    #[ts(type = "number")]
    pub fuel_consumed: u64,
    #[ts(type = "number")]
    pub fuel_remaining: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivationFailed {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub wasm_module: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionActivationReport {
    pub plan: ExtensionActivationPlan,
    pub activated: Vec<ExtensionActivated>,
    pub failed: Vec<ExtensionActivationFailed>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionContributionRegistration {
    pub extension_id: String,
    pub extension_name: String,
    pub extension_version: String,
    pub contribution: ExtensionContributionPoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionContributionUnavailable {
    pub extension_id: String,
    pub extension_name: String,
    pub extension_version: String,
    pub contribution: ExtensionContributionPoint,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionContributionRegistry {
    pub activation: ExtensionActivationReport,
    pub registered: Vec<ExtensionContributionRegistration>,
    pub unavailable: Vec<ExtensionContributionUnavailable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionCommandRoute {
    pub id: String,
    pub title: String,
    pub category: Option<String>,
    pub handler: String,
    pub extension_id: String,
    pub extension_name: String,
    pub extension_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionCommandExecutionStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionCommandExecutionPhase {
    Routing,
    Activation,
    Handler,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionCommandExecution {
    pub command_id: String,
    pub route: Option<ExtensionCommandRoute>,
    pub status: ExtensionCommandExecutionStatus,
    pub phase: ExtensionCommandExecutionPhase,
    pub reason: Option<String>,
    #[ts(type = "number")]
    pub duration_ms: u64,
    #[ts(type = "number | null")]
    pub activation_fuel_consumed: Option<u64>,
    #[ts(type = "number | null")]
    pub activation_fuel_remaining: Option<u64>,
    #[ts(type = "number | null")]
    pub handler_fuel_consumed: Option<u64>,
    #[ts(type = "number | null")]
    pub handler_fuel_remaining: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugAdapterStatus {
    Available,
    Missing,
    NotConfigured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugConfigurationRequest {
    Launch,
    Attach,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugAdapterTransport {
    Stdio,
    TcpServer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugSessionStatus {
    Starting,
    Running,
    Paused,
    Stopping,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugExecutionAction {
    Continue,
    StepOver,
    StepIn,
    StepOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugEvaluateContext {
    Repl,
    Watch,
    Hover,
    Clipboard,
    Variables,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugSourceBreakpoint {
    pub path: PathBuf,
    #[ts(type = "number")]
    pub line: u64,
    #[ts(type = "number | null")]
    pub column: Option<u64>,
    pub condition: Option<String>,
    pub log_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugResolvedBreakpoint {
    #[ts(type = "number | null")]
    pub id: Option<u64>,
    pub path: PathBuf,
    #[ts(type = "number")]
    pub line: u64,
    #[ts(type = "number | null")]
    pub column: Option<u64>,
    pub verified: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugBreakpointsUpdate {
    pub session_id: Uuid,
    pub path: PathBuf,
    pub breakpoints: Vec<DebugResolvedBreakpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugAdapterInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub configuration_types: Vec<String>,
    pub transport: DebugAdapterTransport,
    pub workspace_root: PathBuf,
    pub status: DebugAdapterStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugConfiguration {
    pub name: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub adapter_type: String,
    pub request: DebugConfigurationRequest,
    #[ts(type = "Record<string, unknown>")]
    pub raw: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugWorkspaceInfo {
    pub adapters: Vec<DebugAdapterInfo>,
    pub configurations: Vec<DebugConfiguration>,
    pub launch_json_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugSessionInfo {
    pub id: Uuid,
    pub configuration_name: String,
    pub adapter_id: String,
    pub adapter_name: String,
    pub workspace_root: PathBuf,
    pub status: DebugSessionStatus,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    #[ts(type = "number | null")]
    pub active_thread_id: Option<u64>,
    pub last_event: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugThreadInfo {
    #[ts(type = "number")]
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugStackFrame {
    #[ts(type = "number")]
    pub id: u64,
    pub name: String,
    pub source_path: Option<PathBuf>,
    #[ts(type = "number")]
    pub line: u64,
    #[ts(type = "number")]
    pub column: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugStackTrace {
    pub session_id: Uuid,
    pub thread: DebugThreadInfo,
    pub frames: Vec<DebugStackFrame>,
    #[ts(type = "number | null")]
    pub total_frames: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugScopeInfo {
    pub name: String,
    #[ts(type = "number")]
    pub variables_reference: u64,
    pub expensive: bool,
    #[ts(type = "number | null")]
    pub named_variables: Option<u64>,
    #[ts(type = "number | null")]
    pub indexed_variables: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugVariableInfo {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    #[ts(type = "number")]
    pub variables_reference: u64,
    pub evaluate_name: Option<String>,
    #[ts(type = "number | null")]
    pub named_variables: Option<u64>,
    #[ts(type = "number | null")]
    pub indexed_variables: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugFrameScopes {
    pub session_id: Uuid,
    #[ts(type = "number")]
    pub frame_id: u64,
    pub scopes: Vec<DebugScopeInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugVariables {
    pub session_id: Uuid,
    #[ts(type = "number")]
    pub variables_reference: u64,
    pub variables: Vec<DebugVariableInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugEvaluateResult {
    pub session_id: Uuid,
    pub expression: String,
    pub result: String,
    pub type_name: Option<String>,
    #[ts(type = "number")]
    pub variables_reference: u64,
    #[ts(type = "number | null")]
    pub named_variables: Option<u64>,
    #[ts(type = "number | null")]
    pub indexed_variables: Option<u64>,
    pub memory_reference: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LanguageServerStatus {
    Available,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LanguageServerInfo {
    pub language_id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub workspace_root: PathBuf,
    pub status: LanguageServerStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceDiagnostic {
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspRange {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspHover {
    pub contents: Vec<String>,
    pub range: Option<LspRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspLocation {
    pub path: PathBuf,
    pub range: LspRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspSymbolKind {
    File,
    Module,
    Namespace,
    Package,
    Class,
    Method,
    Property,
    Field,
    Constructor,
    Enum,
    Interface,
    Function,
    Variable,
    Constant,
    String,
    Number,
    Boolean,
    Array,
    Object,
    Key,
    Null,
    EnumMember,
    Struct,
    Event,
    Operator,
    TypeParameter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspDocumentSymbol {
    pub name: String,
    pub detail: Option<String>,
    pub kind: LspSymbolKind,
    pub range: LspRange,
    pub selection_range: LspRange,
    pub children: Vec<Self>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspWorkspaceSymbol {
    pub name: String,
    pub kind: LspSymbolKind,
    pub location: LspLocation,
    pub container_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspFoldingRangeKind {
    Comment,
    Imports,
    Region,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspFoldingRange {
    pub start_line: u32,
    pub end_line: u32,
    pub start_column: Option<u32>,
    pub end_column: Option<u32>,
    pub kind: Option<LspFoldingRangeKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspInlayHintKind {
    Type,
    Parameter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspInlayHint {
    pub label: String,
    pub tooltip: Option<String>,
    pub line: u32,
    pub column: u32,
    pub kind: Option<LspInlayHintKind>,
    pub padding_left: bool,
    pub padding_right: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspSemanticTokens {
    pub result_id: Option<String>,
    pub data: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspCompletionItemKind {
    Text,
    Method,
    Function,
    Constructor,
    Field,
    Variable,
    Class,
    Interface,
    Module,
    Property,
    Unit,
    Value,
    Enum,
    Keyword,
    Snippet,
    Color,
    File,
    Reference,
    Folder,
    EnumMember,
    Constant,
    Struct,
    Event,
    Operator,
    TypeParameter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspInsertTextFormat {
    PlainText,
    Snippet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspCompletionItem {
    pub label: String,
    pub kind: Option<LspCompletionItemKind>,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub insert_text: String,
    pub insert_text_format: LspInsertTextFormat,
    pub filter_text: Option<String>,
    pub sort_text: Option<String>,
    pub range: Option<LspRange>,
    pub commit_characters: Vec<String>,
    pub preselect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspCompletionList {
    pub is_incomplete: bool,
    pub items: Vec<LspCompletionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspCodeActionDiagnostic {
    pub range: LspRange,
    pub severity: Option<DiagnosticSeverity>,
    pub source: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspCodeAction {
    pub title: String,
    pub kind: Option<String>,
    pub is_preferred: bool,
    pub disabled_reason: Option<String>,
    pub edit: Option<LspWorkspaceEdit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspCodeActionTrigger {
    Invoke,
    Automatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspFormattingOptions {
    pub tab_size: u32,
    pub insert_spaces: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspSignatureParameter {
    pub label: String,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspSignatureInformation {
    pub label: String,
    pub documentation: Option<String>,
    pub parameters: Vec<LspSignatureParameter>,
    pub active_parameter: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspSignatureHelp {
    pub signatures: Vec<LspSignatureInformation>,
    pub active_signature: Option<u32>,
    pub active_parameter: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspTextEdit {
    pub range: LspRange,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspWorkspaceEditFile {
    pub path: PathBuf,
    pub edits: Vec<LspTextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspWorkspaceEdit {
    pub files: Vec<LspWorkspaceEditFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceEditResult {
    pub edited_documents: Vec<DocumentSnapshot>,
    pub changed_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum LuxEvent {
    WorkspaceChanged {
        workspace: Option<WorkspaceInfo>,
    },
    FsChanged {
        path: PathBuf,
    },
    EditorDocumentClosed {
        document: DocumentSnapshot,
    },
    EditorDocumentChanged {
        document: DocumentSnapshot,
    },
    EditorDocumentsChanged {
        documents: Vec<DocumentSnapshot>,
    },
    EditorDocumentEdited {
        document: DocumentEditResult,
    },
    EditorDiagnosticsChanged {
        path: PathBuf,
        diagnostics: Vec<WorkspaceDiagnostic>,
    },
    SearchProgress {
        query: String,
        indexed_files: usize,
    },
    TerminalOutput {
        session_id: Uuid,
        data: String,
    },
    GitStatusChanged {
        status: GitStatus,
    },
    DebugSessionChanged {
        session: DebugSessionInfo,
    },
    DebugBreakpointsChanged {
        update: DebugBreakpointsUpdate,
    },
    SettingsChanged {
        key: String,
    },
}
