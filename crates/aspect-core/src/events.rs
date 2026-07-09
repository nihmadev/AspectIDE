use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    DebugBreakpointsUpdate, DebugSessionInfo, DocumentEditResult, DocumentSnapshot, GitStatus,
    WorkspaceDiagnostic, WorkspaceInfo,
};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum AspectEvent {
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
    AiShellOutput {
        data: String,
        tool_call_id: Option<String>,
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
