use std::path::Path;

use serde_json::{json, Value};

use crate::types::{LanguageServerDefinition, CLIENT_SEMANTIC_TOKEN_MODIFIERS, CLIENT_SEMANTIC_TOKEN_TYPES};

use super::uri::path_to_file_uri;

const CLIENT_SYMBOL_KIND_VALUE_SET: &[u8] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
];

const CLIENT_COMPLETION_ITEM_KIND_VALUE_SET: &[u8] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
];

#[must_use]
pub fn initialize_request(id: u64, definition: &LanguageServerDefinition) -> Value {
    let root_uri = path_to_file_uri(&definition.workspace_root);
    let mut params = json!({
        "processId": std::process::id(),
        "rootUri": root_uri,
        "capabilities": initialize_capabilities(),
        "workspaceFolders": [{
            "uri": root_uri,
            "name": definition.workspace_root.file_name().and_then(|name| name.to_str()).unwrap_or("workspace")
        }]
    });

    if definition.language_id == "typescript" {
        params["initializationOptions"] = typescript_initialization_options(definition);
    }

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": params
    })
}

fn typescript_initialization_options(definition: &LanguageServerDefinition) -> Value {
    let mut options = json!({
        "typescript": {
            "preferences": {
                "importModuleSpecifierEnding": "js",
                "includePackageJsonAutoImports": "auto"
            }
        },
        "javascript": {
            "preferences": {
                "importModuleSpecifierEnding": "js"
            }
        }
    });

    if let Some(tsdk) = resolve_typescript_sdk(definition) {
        options["typescript"]["tsdk"] = json!(tsdk);
    }

    options
}

fn resolve_typescript_sdk(definition: &LanguageServerDefinition) -> Option<String> {
    let workspace_tsdk = definition.workspace_root.join("node_modules/typescript/lib");
    if workspace_tsdk.join("typescript.js").is_file() {
        return Some(workspace_tsdk.to_string_lossy().to_string());
    }

    let cmd = Path::new(&definition.command);
    if cmd.is_absolute() {
        if let Some(bin_dir) = cmd.parent() {
            if let Some(node_modules_dir) = bin_dir.parent() {
                if node_modules_dir.ends_with("node_modules") {
                    let managed_tsdk = node_modules_dir.join("typescript/lib");
                    if managed_tsdk.join("typescript.js").is_file() {
                        return Some(managed_tsdk.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    None
}

fn initialize_capabilities() -> Value {
    json!({
        "textDocument": text_document_capabilities(),
        "workspace": workspace_capabilities(),
    })
}

fn text_document_capabilities() -> Value {
    json!({
        "publishDiagnostics": {
            "relatedInformation": true,
            "versionSupport": true,
            "codeDescriptionSupport": true,
            "dataSupport": true,
        },
        "synchronization": {
            "dynamicRegistration": false,
            "didSave": true,
        },
        "hover": {
            "dynamicRegistration": false,
            "contentFormat": ["markdown", "plaintext"],
        },
        "definition": {
            "dynamicRegistration": false,
            "linkSupport": true,
        },
        "references": {
            "dynamicRegistration": false,
        },
        "documentSymbol": document_symbol_capabilities(),
        "foldingRange": {
            "dynamicRegistration": false,
            "lineFoldingOnly": true,
        },
        "inlayHint": inlay_hint_capabilities(),
        "semanticTokens": semantic_token_capabilities(),
        "rename": {
            "dynamicRegistration": false,
            "prepareSupport": false,
        },
        "codeAction": code_action_capabilities(),
        "formatting": {
            "dynamicRegistration": false,
        },
        "rangeFormatting": {
            "dynamicRegistration": false,
        },
        "completion": completion_capabilities(),
        "signatureHelp": signature_help_capabilities(),
    })
}

fn document_symbol_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "hierarchicalDocumentSymbolSupport": true,
        "symbolKind": {
            "valueSet": CLIENT_SYMBOL_KIND_VALUE_SET,
        },
    })
}

fn inlay_hint_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "resolveSupport": {
            "properties": ["tooltip", "textEdits", "label.tooltip", "label.location", "label.command"],
        },
    })
}

fn semantic_token_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "requests": {
            "range": false,
            "full": true,
        },
        "tokenTypes": CLIENT_SEMANTIC_TOKEN_TYPES,
        "tokenModifiers": CLIENT_SEMANTIC_TOKEN_MODIFIERS,
        "formats": ["relative"],
        "overlappingTokenSupport": false,
        "multilineTokenSupport": true,
        "serverCancelSupport": false,
        "augmentsSyntaxTokens": true,
    })
}

fn code_action_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "isPreferredSupport": true,
        "disabledSupport": true,
        "dataSupport": false,
        "codeActionLiteralSupport": {
            "codeActionKind": {
                "valueSet": ["", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source", "source.organizeImports", "source.fixAll"],
            },
        },
    })
}

fn completion_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "completionItem": {
            "snippetSupport": true,
            "commitCharactersSupport": true,
            "documentationFormat": ["markdown", "plaintext"],
            "deprecatedSupport": true,
            "preselectSupport": true,
            "tagSupport": {
                "valueSet": [1],
            },
        },
        "completionItemKind": {
            "valueSet": CLIENT_COMPLETION_ITEM_KIND_VALUE_SET,
        },
        "contextSupport": true,
    })
}

fn signature_help_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "signatureInformation": {
            "documentationFormat": ["markdown", "plaintext"],
            "parameterInformation": {
                "labelOffsetSupport": true,
            },
            "activeParameterSupport": true,
        },
        "contextSupport": true,
    })
}

fn workspace_capabilities() -> Value {
    json!({
        "symbol": {
            "dynamicRegistration": false,
            "symbolKind": {
                "valueSet": CLIENT_SYMBOL_KIND_VALUE_SET,
            },
        },
        "workspaceFolders": false,
        "configuration": false,
    })
}
