#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

mod discovery;
mod helpers;
mod manager;
mod protocol;
mod results;
mod session;
mod transport;
mod types;

pub use discovery::{
    language_server_diagnostics, workspace_language_servers, workspace_language_servers_with_dirs,
    BuiltinServer, BUILTIN_SERVERS,
};
pub use manager::LspManager;
pub use protocol::{
    code_action_request, completion_request, definition_request, did_change_edits_notification,
    did_change_notification, did_close_notification, did_open_notification, did_save_notification,
    document_symbol_request, exit_notification, folding_range_request, formatting_request,
    hover_request, initialize_request, initialized_notification, inlay_hint_request,
    path_to_file_uri, range_formatting_request, references_request, rename_request,
    semantic_tokens_full_request, shutdown_request, signature_help_request,
    workspace_symbol_request,
};
pub use results::{
    diagnostics_update_from_publish, parse_code_action_result, parse_completion_result,
    parse_definition_result, parse_document_symbol_result, parse_folding_range_result,
    parse_hover_result, parse_inlay_hint_result, parse_semantic_token_legend_from_initialize,
    parse_semantic_tokens_result, parse_signature_help_result, parse_text_document_sync_kind,
    parse_text_edits_result, parse_workspace_edit_result, parse_workspace_symbol_result,
    workspace_diagnostics_from_publish,
};
pub use transport::{
    drain_lsp_frames, encode_lsp_message, parse_lsp_notification, parse_lsp_response, LspFrame,
    LspNotification,
};
pub use types::{
    DiagnosticsUpdate, LanguageServerDefinition, SemanticTokenLegend, TextDocumentSyncKind,
    CLIENT_SEMANTIC_TOKEN_MODIFIERS, CLIENT_SEMANTIC_TOKEN_TYPES,
};
