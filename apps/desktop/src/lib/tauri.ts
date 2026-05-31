import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  BufferId,
  DebugWorkspaceInfo,
  DocumentEditResult,
  DocumentSnapshot,
  ExtensionInfo,
  FsEntry,
  GitDiff,
  GitStatus,
  KeybindingProfile,
  LanguageServerInfo,
  LspCodeAction,
  LspCodeActionDiagnostic,
  LspCodeActionTrigger,
  LspCompletionList,
  LspDocumentSymbol,
  LspFoldingRange,
  LspFormattingOptions,
  LspHover,
  LspInlayHint,
  LspLocation,
  LspRange,
  LspSemanticTokens,
  LspSignatureHelp,
  LspTextEdit,
  LspWorkspaceSymbol,
  LspWorkspaceEdit,
  LuxEvent,
  RecentWorkspace,
  SearchOptions,
  SearchResponse,
  SettingValue,
  SettingsScope,
  TerminalSessionInfo,
  TextEdit,
  WorkspaceDiagnostic,
  WorkspaceEditResult,
  WorkspaceInfo,
} from "./types";

export type VoiceInputProviderStatus = {
  provider: string;
  available: boolean;
  detail: string;
  command: string | null;
  modelPath: string | null;
};

export type VoiceTranscriptionRequest = {
  provider: "local";
  audioBase64: string;
  mimeType: string;
  language?: string | null;
  command?: string | null;
  modelPath?: string | null;
};

export type VoiceTranscriptionResult = {
  text: string;
};

export type AiChatCompletionRequest = {
  baseUrl: string;
  apiKey?: string | null;
  payload: unknown;
};

export type AiChatCompletionResponse = {
  status: number;
  body: unknown;
};

export type AiChatCompletionStreamRequest = AiChatCompletionRequest & {
  streamId?: string;
};

export type AiChatCompletionStreamResponse = {
  streamId: string;
};

export type AiChatStreamEvent = {
  streamId: string;
  kind: "chunk" | "done" | "error" | "cancelled" | string;
  data?: unknown;
  error?: string | null;
};

export type WebFetchResponse = {
  url: string;
  finalUrl: string;
  status: number;
  contentType: string | null;
  title: string | null;
  text: string;
  bytesRead: number;
  truncated: boolean;
  elapsedMs: number;
};

export type FsReadTextResponse = {
  path: string;
  text: string;
  truncated: boolean;
  size: number;
};

export type TestHealthResponse = {
  workspaceRoot: string;
  status: "passed" | "failed" | "timeout" | "error" | string;
  summary: TestHealthSummary;
  runners: TestHealthRunnerResult[];
  language: string;
  framework: string;
  command: string;
  exitCode: number | null;
  durationMs: number;
  stdout: string;
  stderr: string;
  timedOut: boolean;
};

export type TestHealthSummary = {
  total: number;
  passed: number;
  failed: number;
  timedOut: number;
  errored: number;
  skipped: number;
  durationMs: number;
};

export type TestHealthRunnerResult = {
  id: string;
  workspaceRelativePath: string;
  status: "passed" | "failed" | "timeout" | "error" | string;
  kind: "test" | "typecheck" | "lint" | "build" | "check" | string;
  language: string;
  framework: string;
  command: string;
  exitCode: number | null;
  durationMs: number;
  stdout: string;
  stderr: string;
  timedOut: boolean;
};

export type AiFileOperationStats = {
  linesAdded: number;
  linesRemoved: number;
  filesChanged: number;
  filesCreated: number;
  filesDeleted: number;
};

export type AiFileOperationResult = {
  operation: string;
  path: string;
  savedToDisk: boolean;
  changedPaths: string[];
  editedDocuments: DocumentSnapshot[];
  stats: AiFileOperationStats;
  message: string;
};

export type AiFilePatchOperation = {
  action: "create" | "rewrite" | "replace" | "delete" | string;
  path: string;
  text?: string;
  oldText?: string;
  newText?: string;
  expectedReplacements?: number;
  overwrite?: boolean;
};

export type AiShellResponse = {
  workspaceRoot: string;
  cwd: string;
  command: string;
  exitCode: number | null;
  durationMs: number;
  stdout: string;
  stderr: string;
  timedOut: boolean;
};

export type AiSymbolContextResponse = {
  workspaceRoot: string;
  query: string;
  path: string | null;
  position: { line: number; column: number } | null;
  workspaceSymbols: LspWorkspaceSymbol[];
  documentSymbols: LspDocumentSymbol[];
  hover: LspHover | null;
  definitions: LspLocation[];
  references: LspLocation[];
  signatureHelp: LspSignatureHelp | null;
  diagnostics: WorkspaceDiagnostic[];
  notes: string[];
};

export const isTauriRuntime = () => Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);

async function invokeRequired<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    throw new Error(`Command ${command} requires the Lux desktop runtime`);
  }
  return invoke<T>(command, args);
}

async function invokeOptional<T>(command: string, args: Record<string, unknown> | undefined, fallback: () => T): Promise<T> {
  if (!isTauriRuntime()) return fallback();
  return invoke<T>(command, args);
}

const browserSettings = new Map<string, SettingValue>();

export const luxCommands = {
  workspaceOpen: (path: string) => invokeRequired<WorkspaceInfo>("workspace_open", { path }),
  workspaceClose: () => invokeRequired<void>("workspace_close"),
  workspacePickFolder: () => invokeOptional<WorkspaceInfo | null>("workspace_pick_folder", undefined, () => null),
  fsReadDir: (path: string) => invokeRequired<FsEntry[]>("fs_read_dir", { path }),
  fsReadTree: (path: string) => invokeRequired<FsEntry[]>("fs_read_tree", { path }),
  fsReadText: (path: string, maxBytes?: number) => invokeRequired<FsReadTextResponse>("fs_read_text", { path, maxBytes }),
  fsListFiles: (maxResults = 2_500) => invokeRequired<FsEntry[]>("fs_list_files", { maxResults }),
  fsCreateFile: (path: string) => invokeRequired<void>("fs_create_file", { path }),
  fsCreateDir: (path: string) => invokeRequired<void>("fs_create_dir", { path }),
  fsRename: (from: string, to: string) => invokeRequired<void>("fs_rename", { from, to }),
  fsCopy: (from: string, to: string) => invokeRequired<void>("fs_copy", { from, to }),
  fsDelete: (path: string) => invokeRequired<void>("fs_delete", { path }),
  fsRevealInFileExplorer: (path: string) => invokeRequired<void>("fs_reveal_in_file_explorer", { path }),
  clipboardWriteText: (text: string) => navigator.clipboard.writeText(text),
  editorNewFile: () => invokeOptional<DocumentSnapshot>("editor_new_file", undefined, createBrowserUntitledDocument),
  editorOpenFile: (path: string) => invokeRequired<DocumentSnapshot>("editor_open_file", { path }),
  editorUpdateText: (bufferId: BufferId, text: string) => invokeRequired<DocumentSnapshot>("editor_update_text", { bufferId, text }),
  editorApplyEdits: (bufferId: BufferId, edits: TextEdit[]) => invokeRequired<DocumentEditResult>("editor_apply_edits", { bufferId, edits }),
  editorApplyWorkspaceEdit: (edit: LspWorkspaceEdit) => invokeRequired<WorkspaceEditResult>("editor_apply_workspace_edit", { edit }),
  editorSaveFile: (bufferId: BufferId) => invokeOptional<DocumentSnapshot>("editor_save_file", { bufferId }, () => saveBrowserDocument(bufferId)),
  editorSaveFileAs: (bufferId: BufferId) => invokeOptional<DocumentSnapshot>("editor_save_file_as", { bufferId }, () => saveBrowserDocumentAs(bufferId)),
  searchQuery: (query: string, options: SearchOptions) => invokeRequired<SearchResponse>("search_query", { query, options }),
  aiChatCompletion: (request: AiChatCompletionRequest) => invokeRequired<AiChatCompletionResponse>("ai_chat_completion", { request }),
  aiChatCompletionStream: (request: AiChatCompletionStreamRequest) =>
    invokeRequired<AiChatCompletionStreamResponse>("ai_chat_completion_stream", { request }),
  aiChatCompletionStreamCancel: (streamId: string) => invokeRequired<void>("ai_chat_completion_stream_cancel", { streamId }),
  webFetch: (url: string, maxBytes?: number | null, timeoutSecs?: number | null, allowPrivateHosts?: boolean | null) =>
    invokeRequired<WebFetchResponse>("web_fetch", { url, maxBytes: maxBytes ?? null, timeoutSecs: timeoutSecs ?? null, allowPrivateHosts: allowPrivateHosts ?? null }),
  testHealth: () => invokeRequired<TestHealthResponse>("test_health"),
  aiFileWrite: (path: string, text: string, overwrite = false, saveToDisk = true) =>
    invokeRequired<AiFileOperationResult>("ai_file_write", { path, text, overwrite, saveToDisk }),
  aiFileStrReplace: (path: string, oldText: string, newText: string, expectedReplacements = 1, saveToDisk = true) =>
    invokeRequired<AiFileOperationResult>("ai_file_str_replace", { path, oldText, newText, expectedReplacements, saveToDisk }),
  aiFilePatch: (operations: AiFilePatchOperation[], saveToDisk = true, dryRun = false) =>
    invokeRequired<AiFileOperationResult>("ai_file_patch", { operations, saveToDisk, dryRun }),
  aiFileDelete: (path: string) => invokeRequired<AiFileOperationResult>("ai_file_delete", { path }),
  aiShell: (command: string, cwd?: string | null, timeoutSecs?: number | null) =>
    invokeRequired<AiShellResponse>("ai_shell", { command, cwd: cwd ?? null, timeoutSecs: timeoutSecs ?? null }),
  aiSymbolContext: (query?: string | null, path?: string | null, line?: number | null, column?: number | null, maxResults?: number | null) =>
    invokeRequired<AiSymbolContextResponse>("ai_symbol_context", { query: query ?? null, path: path ?? null, line: line ?? null, column: column ?? null, maxResults: maxResults ?? null }),
  voiceInputStatus: (provider: string, command?: string | null, modelPath?: string | null) =>
    invokeOptional<VoiceInputProviderStatus>("voice_input_status", { provider, command, modelPath }, () => ({
      provider,
      available: false,
      detail: "Lux desktop runtime is required for local voice input",
      command: command ?? null,
      modelPath: modelPath ?? null,
    })),
  voiceTranscribeLocal: (request: VoiceTranscriptionRequest) =>
    invokeRequired<VoiceTranscriptionResult>("voice_transcribe_local", { request }),
  terminalCreate: (shell?: string, cwd?: string, cols = 120, rows = 30) =>
    invokeRequired<TerminalSessionInfo>("terminal_create", { shell, cwd, cols, rows }),
  terminalWrite: (sessionId: string, data: string) => invokeRequired<void>("terminal_write", { sessionId, data }),
  terminalResize: (sessionId: string, cols: number, rows: number) => invokeRequired<void>("terminal_resize", { sessionId, cols, rows }),
  terminalClose: (sessionId: string) => invokeRequired<void>("terminal_close", { sessionId }),
  terminalCloseAll: () => invokeRequired<void>("terminal_close_all"),
  gitStatus: () => invokeRequired<GitStatus>("git_status"),
  gitDiff: () => invokeRequired<GitDiff>("git_diff"),
  extensionsList: () => invokeRequired<ExtensionInfo[]>("extensions_list"),
  debugWorkspaceInfo: () => invokeRequired<DebugWorkspaceInfo>("debug_workspace_info"),
  lspServers: () => invokeRequired<LanguageServerInfo[]>("lsp_servers"),
  diagnosticsSnapshot: () => invokeRequired<WorkspaceDiagnostic[]>("diagnostics_snapshot"),
  lspHover: (bufferId: BufferId, line: number, column: number) => invokeOptional<LspHover | null>("lsp_hover", { bufferId, line, column }, () => null),
  lspDefinition: (bufferId: BufferId, line: number, column: number) => invokeOptional<LspLocation[]>("lsp_definition", { bufferId, line, column }, () => []),
  lspReferences: (bufferId: BufferId, line: number, column: number) => invokeOptional<LspLocation[]>("lsp_references", { bufferId, line, column }, () => []),
  lspDocumentSymbols: (bufferId: BufferId) => invokeOptional<LspDocumentSymbol[]>("lsp_document_symbols", { bufferId }, () => []),
  lspWorkspaceSymbols: (query: string) => invokeOptional<LspWorkspaceSymbol[]>("lsp_workspace_symbols", { query }, () => []),
  lspFoldingRanges: (bufferId: BufferId) => invokeOptional<LspFoldingRange[]>("lsp_folding_ranges", { bufferId }, () => []),
  lspInlayHints: (bufferId: BufferId, range: LspRange) => invokeOptional<LspInlayHint[]>("lsp_inlay_hints", { bufferId, range }, () => []),
  lspSemanticTokens: (bufferId: BufferId) => invokeOptional<LspSemanticTokens | null>("lsp_semantic_tokens", { bufferId }, () => null),
  lspRename: (bufferId: BufferId, line: number, column: number, newName: string) =>
    invokeOptional<WorkspaceEditResult>("lsp_rename", { bufferId, line, column, newName }, () => ({ edited_documents: [], changed_paths: [] })),
  lspCompletion: (bufferId: BufferId, line: number, column: number) =>
    invokeOptional<LspCompletionList>("lsp_completion", { bufferId, line, column }, () => ({ is_incomplete: false, items: [] })),
  lspCodeActions: (
    bufferId: BufferId,
    range: LspRange,
    diagnostics: LspCodeActionDiagnostic[],
    only: string[] | null,
    trigger: LspCodeActionTrigger,
  ) => invokeOptional<LspCodeAction[]>("lsp_code_actions", { bufferId, range, diagnostics, only, trigger }, () => []),
  lspFormatDocument: (bufferId: BufferId, options: LspFormattingOptions) =>
    invokeOptional<LspTextEdit[]>("lsp_format_document", { bufferId, options }, () => []),
  lspFormatRange: (bufferId: BufferId, range: LspRange, options: LspFormattingOptions) =>
    invokeOptional<LspTextEdit[]>("lsp_format_range", { bufferId, range, options }, () => []),
  lspSignatureHelp: (bufferId: BufferId, line: number, column: number) =>
    invokeOptional<LspSignatureHelp | null>("lsp_signature_help", { bufferId, line, column }, () => null),
  recentWorkspaces: () => invokeOptional<RecentWorkspace[]>("recent_workspaces", undefined, () => []),
  recentWorkspaceForget: (root: string) => invokeOptional<RecentWorkspace[]>("recent_workspace_forget", { root }, () => []),
  settingsGet: (scope: SettingsScope, key: string) =>
    invokeOptional<SettingValue | null>("settings_get", { scope, key }, () => browserSettings.get(key) ?? null),
  settingsSet: (scope: SettingsScope, key: string, value: unknown) =>
    invokeOptional<SettingValue>("settings_set", { scope, key, value }, () => {
      const setting = { key, value, updated_at: new Date().toISOString() };
      browserSettings.set(key, setting);
      return setting;
    }),
  keybindingsGet: () => invokeOptional<KeybindingProfile>("keybindings_get", undefined, () => defaultKeybindingProfile()),
  keybindingsSet: (profile: KeybindingProfile) => invokeOptional<KeybindingProfile>("keybindings_set", { profile }, () => profile),
};

let browserUntitledCounter = 0;
const browserDocuments = new Map<string, DocumentSnapshot>();

function createBrowserUntitledDocument(): DocumentSnapshot {
  browserUntitledCounter += 1;
  const title = `Untitled-${browserUntitledCounter}`;
  const document = {
    id: crypto.randomUUID(),
    path: null,
    title,
    language_id: "plaintext",
    text: "",
    version: 1,
    is_dirty: true,
    is_untitled: true,
    opened_at: new Date().toISOString(),
  };
  browserDocuments.set(document.id, document);
  return document;
}

function saveBrowserDocument(bufferId: BufferId): DocumentSnapshot {
  const existing = browserDocuments.get(bufferId);
  if (!existing) throw new Error(`No browser document ${bufferId}`);
  const saved = { ...existing, is_dirty: false, version: existing.version + 1 };
  browserDocuments.set(bufferId, saved);
  return saved;
}

function saveBrowserDocumentAs(bufferId: BufferId): DocumentSnapshot {
  const existing = browserDocuments.get(bufferId);
  if (!existing) throw new Error(`No browser document ${bufferId}`);
  const saved = {
    ...existing,
    path: existing.path ?? `browser://${existing.title}`,
    is_dirty: false,
    is_untitled: false,
    version: existing.version + 1,
  };
  browserDocuments.set(bufferId, saved);
  return saved;
}

function defaultKeybindingProfile(): KeybindingProfile {
  return {
    id: "default",
    name: "Default",
    bindings: [
      { command: "workbench.action.showCommands", key: "Ctrl+Shift+P", when: null },
      { command: "workbench.action.quickOpen", key: "Ctrl+P", when: null },
      { command: "workbench.action.files.newUntitledFile", key: "Ctrl+N", when: null },
      { command: "workbench.action.openSettings", key: "Ctrl+,", when: null },
      { command: "workbench.action.openFolder", key: "Ctrl+O", when: null },
      { command: "workbench.action.toggleSidebar", key: "Ctrl+B", when: "workspace" },
      { command: "workbench.view.explorer", key: "Ctrl+Shift+E", when: "workspace" },
      { command: "workbench.view.search", key: "Ctrl+Shift+F", when: "workspace" },
      { command: "workbench.view.scm", key: "Ctrl+Shift+G", when: "workspace" },
      { command: "workbench.view.debug", key: "Ctrl+Shift+D", when: "workspace" },
      { command: "workbench.view.extensions", key: "Ctrl+Shift+X", when: "workspace" },
      { command: "workbench.action.chat.toggle", key: "Ctrl+L", when: "workspace" },
      { command: "workbench.action.terminal.toggleTerminal", key: "Ctrl+`", when: "workspace" },
      { command: "editor.action.toggleWordWrap", key: "Alt+Z", when: "editor" },
      { command: "editor.action.toggleMinimap", key: "Ctrl+M Ctrl+M", when: "editor" },
      { command: "editor.action.fontZoomIn", key: "Ctrl+=", when: "editor" },
      { command: "editor.action.fontZoomIn", key: "Ctrl+Shift+=", when: "editor" },
      { command: "editor.action.fontZoomOut", key: "Ctrl+-", when: "editor" },
      { command: "editor.action.fontZoomReset", key: "Ctrl+0", when: "editor" },
      { command: "workbench.action.files.save", key: "Ctrl+S", when: "editor" },
      { command: "workbench.action.files.saveAs", key: "Ctrl+Shift+S", when: "editor" },
      { command: "workbench.action.files.saveAll", key: "Ctrl+K Ctrl+S", when: "dirtyEditors" },
      { command: "workbench.action.closeActiveEditor", key: "Ctrl+W", when: "editor" },
      { command: "workbench.action.splitEditorRight", key: "Ctrl+\\", when: "editor" },
      { command: "workbench.action.nextEditor", key: "Ctrl+PageDown", when: "editor" },
      { command: "workbench.action.previousEditor", key: "Ctrl+PageUp", when: "editor" },
    ],
  };
}

export async function subscribeLuxEvents(handler: (event: LuxEvent) => void) {
  if (!isTauriRuntime()) {
    return () => undefined;
  }
  return listen<LuxEvent>("lux://event", (event) => handler(event.payload));
}

export async function subscribeAiChatStream(handler: (event: AiChatStreamEvent) => void) {
  if (!isTauriRuntime()) {
    return () => undefined;
  }
  return listen<AiChatStreamEvent>("lux://ai-chat-stream", (event) => handler(event.payload));
}
