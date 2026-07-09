use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSemanticResult {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub source: &'static str,
    pub score: i64,
    pub path: String,
    pub relative_path: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub name: Option<String>,
    #[serde(rename = "kind", skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_name: Option<String>,
    pub preview: Option<String>,
    pub match_text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSemanticSearchResponse {
    pub workspace_root: PathBuf,
    pub query: String,
    pub path_filter: Option<String>,
    pub count: usize,
    pub truncated: bool,
    pub partial: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partial_reasons: Vec<String>,
    pub results: Vec<AiSemanticResult>,
}
