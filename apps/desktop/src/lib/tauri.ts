import { invoke } from "@tauri-apps/api/core";
import { decodeIpcResult, expectArray, type IpcDecoder } from "./tauriDecode";
import { readStringField, safeListen } from "./tauriEvents";
import {
  createDesktopRuntimeError,
  desktopRuntimeRequiredMessage,
  isBrowserPreviewRuntime,
  isTauriRuntime,
} from "./tauriRuntime";
import type {
  BufferId,
  DebugBreakpointsUpdate,
  DebugConfiguration,
  DebugEvaluateContext,
  DebugEvaluateResult,
  DebugExecutionAction,
  DebugFrameScopes,
  DebugSourceBreakpoint,
  DebugStackTrace,
  DebugSessionInfo,
  DebugVariables,
  DebugWorkspaceInfo,
  DocumentEditResult,
  DocumentSnapshot,
  ExtensionActivationReport,
  ExtensionActivationPlan,
  ExtensionCommandExecution,
  ExtensionCommandRoute,
  ExtensionContributionRegistry,
  ExtensionInfo,
  FileFormatSupport,
  DatabaseTablePreview,
  FileInspection,
  FileInspectionOptions,
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

/** HEAD vs working-tree text for one file (powers the Git diff view). */
export type GitFileDiff = {
  path: string;
  headText: string;
  workingText: string;
};

// ── Per-project memory (lux-memory) ──

/** One durable memory entry as stored by the per-project memory backend. */
export type MemoryRecord = {
  id: string;
  category: string;
  content: string;
  metadata: Record<string, unknown>;
  importance: number;
  pinned: boolean;
  source?: string;
  createdAt: number;
  updatedAt: number;
  lastAccessedAt: number;
  accessCount: number;
  hasEmbedding: boolean;
  /** Set when a newer memory superseded this one (near-duplicate replacement
   *  or a contradiction sweep). Excluded from search/list unless
   *  `includeSuperseded` is set. */
  superseded: boolean;
  /** TTL cutoff (epoch millis); absent means no expiry. */
  forgetAfter?: number;
};

/** A memory with its blended retrieval score (Rust flattens the record in). */
export type ScoredMemory = MemoryRecord & { score: number; lexical: number };

export type MemoryCategoryCount = { category: string; count: number };
export type MemoryStats = {
  total: number;
  pinned: number;
  byCategory: MemoryCategoryCount[];
  lastUpdatedAt?: number;
};

export type MemorySortOrder = "relevance" | "recent" | "importance" | "oldest";

export type MemorySearchOptions = {
  category?: string | null;
  limit?: number;
  offset?: number;
  minScore?: number | null;
  recencyHalfLifeDays?: number;
  sort?: MemorySortOrder;
  includePinned?: boolean;
  touch?: boolean;
  /** Include rows marked `superseded` (near-duplicate replacement or a
   *  contradiction sweep). Off by default. */
  includeSuperseded?: boolean;
};

export type NewMemoryInput = {
  category: string;
  content: string;
  metadata?: Record<string, unknown>;
  importance?: number;
  pinned?: boolean;
  source?: string;
  id?: string;
  /** Time-to-live in days; the memory is hard-deleted by prune once it expires
   *  (pinned status still wins over an expired TTL). */
  ttlDays?: number;
};

export type MemoryPatch = {
  content?: string;
  category?: string;
  metadata?: Record<string, unknown>;
  importance?: number;
  pinned?: boolean;
  source?: string;
};

/** Outcome of `memory_create`: the written memory plus the ids of any older,
 *  same-category memories it superseded (near-duplicate replacement). */
export type MemoryCreateOutcome = MemoryRecord & { supersededIds: string[] };

/** Kind of edge in the knowledge-graph-lite `memory_relations` table. */
export type MemoryRelationKind = "supersedes" | "extends" | "derives" | "contradicts" | "related";

/** A directed edge between two memories in the knowledge-graph-lite. */
export type MemoryRelation = {
  id: string;
  sourceId: string;
  targetId: string;
  relation: MemoryRelationKind;
  confidence: number;
  createdAt: number;
};

/** One hop-reachable memory returned by `memory_related`. */
export type RelatedMemory = MemoryRecord & { hops: number; pathConfidence: number };

/** Aggregate retention-tier counts for the whole store (Ebbinghaus-style
 *  forgetting curve), for a Settings UI health card. */
export type MemoryRetentionReport = {
  hot: number;
  warm: number;
  cold: number;
  evictable: number;
};

// ── Web research (lux-research) ──

export type ResearchFocus = "web" | "academic" | "news" | "social" | "video" | "code";

export type ResearchOptions = {
  focus?: ResearchFocus;
  maxSources?: number;
  maxCharsPerSource?: number;
};

export type RankedSource = {
  rank: number;
  url: string;
  title: string;
  snippet: string;
  content: string;
  relevance: number;
  engine: string;
};

export type ResearchResponse = {
  query: string;
  focus: ResearchFocus;
  provider: string;
  sourceCount: number;
  sources: RankedSource[];
  notes: string[];
};

// ── SSH (lux-ssh) ──

export type SshTransferDirection = "upload" | "download";

export type SshTarget = {
  host: string;
  user?: string;
  port?: number;
  identityFile?: string;
};

export type SshSession = {
  id: string;
  label: string;
  target: SshTarget;
  cwd: string;
  system?: string;
  remoteUser?: string;
  createdAt: string;
};

export type SshConfigHost = {
  alias: string;
  hostname?: string;
  user?: string;
  port?: number;
  identityFile?: string;
};

export type SshConnectResult = {
  session: SshSession;
  note: string;
};

export type SshExecResult = {
  sessionId: string;
  command: string;
  cwd: string;
  exitCode: number | null;
  durationMs: number;
  stdout: string;
  stderr: string;
  timedOut: boolean;
  warnings: string[];
};

export type SshTransferResult = {
  sessionId: string;
  direction: SshTransferDirection;
  localPath: string;
  remotePath: string;
  recursive: boolean;
  success: boolean;
  exitCode: number | null;
  durationMs: number;
  stdout: string;
  stderr: string;
  timedOut: boolean;
};

export type SshOverview = {
  available: boolean;
  version: string | null;
  sessions: SshSession[];
  configHosts: SshConfigHost[];
  strictHostKey: boolean;
  connectTimeoutSecs: number;
};

export type SshDisconnectResult = {
  closed: number;
  remaining: number;
};

// ── Skills (lux-skills) ──

export type SkillScope = "project" | "global";

export type Skill = {
  slug: string;
  name: string;
  title?: string;
  description: string;
  whenToUse?: string;
  allowedTools: string[];
  tags: string[];
  enabled: boolean;
  scope: SkillScope;
  path: string;
  body: string;
};

/** A skill with its relevance score (Rust flattens the skill in). */
export type ScoredSkill = Skill & { score: number };

/** A skill found in another agent's folder (Claude/Codex), offered for import. */
export type ImportableSkill = {
  source: string;
  slug: string;
  name: string;
  description: string;
  scopeHint: SkillScope;
  path: string;
  content: string;
};

export type SkillDraft = {
  name: string;
  title?: string;
  description: string;
  whenToUse?: string;
  allowedTools: string[];
  tags: string[];
  enabled: boolean;
  body: string;
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

export type DatabaseExecuteRequest = {
  sql: string;
};

export type DatabaseExecuteResult = {
  rowsAffected: number;
  lastInsertRowid: number;
  columns: string[];
  /** Cell is `null` for SQL NULL, distinguishing it from an empty-TEXT `""`. */
  rows: (string | null)[][];
  message: string;
};

export type DatabaseCellUpdate = {
  table: string;
  rowid: number;
  column: string;
  /** `null` writes SQL NULL; `""` writes empty TEXT. */
  value: string | null;
};

export type AgentBrowserStatusRequest = {
  commandPath?: string | null;
  /** Skip npm check and auto-upgrade (faster probe). Default false for bundled CLI. */
  skipAutoUpdate?: boolean | null;
  /** Version-only probe: no doctor, no session list, no Chromium. */
  lightweight?: boolean | null;
};

export type AgentBrowserStatusResponse = {
  available: boolean;
  commandPath: string | null;
  version: string | null;
  latestVersion: string | null;
  updatePerformed: boolean;
  updateDetail: string | null;
  detail: string;
  sessions: string[];
  doctor: unknown;
};

export type AgentBrowserInvokeRequest = {
  session: string;
  args: string[];
  headed?: boolean | null;
  allowedDomains?: string | null;
  maxOutput?: number | null;
  timeoutSecs?: number | null;
  commandPath?: string | null;
  sessionName?: string | null;
  profile?: string | null;
  statePath?: string | null;
  contentBoundaries?: boolean | null;
  ignoreHttpsErrors?: boolean | null;
  allowFileAccess?: boolean | null;
  provider?: string | null;
  proxy?: string | null;
};

export type AgentBrowserInvokeResponse = {
  session: string;
  command: string;
  success: boolean;
  data: unknown;
  text: string;
  elapsedMs: number;
  truncated: boolean;
  exitCode: number | null;
};

export type AgentBrowserInstallRequest = {
  commandPath?: string | null;
  withDeps?: boolean | null;
};

export type AgentBrowserInstallStep = {
  name: string;
  success: boolean;
  output: string;
  elapsedMs: number;
};

export type AgentBrowserInstallResponse = {
  success: boolean;
  commandPath: string | null;
  steps: AgentBrowserInstallStep[];
  detail: string;
};

export type AgentBrowserReadImageResponse = {
  path: string;
  dataUrl: string;
  bytes: number;
  mimeType: string;
};

export type AgentBrowserStreamStatusRequest = {
  session: string;
  commandPath?: string | null;
  enable?: boolean | null;
  port?: number | null;
};

export type AgentBrowserStreamStatusResponse = {
  session: string;
  enabled: boolean;
  port: number | null;
  websocketUrl: string | null;
  data: unknown;
};

export type AgentBrowserDashboardRequest = {
  action: string;
  port?: number | null;
  commandPath?: string | null;
};

export type AgentBrowserDashboardResponse = {
  action: string;
  success: boolean;
  port: number | null;
  url: string | null;
  detail: string;
  data: unknown;
};

export type AgentBrowserSkillsRequest = {
  name?: string | null;
  all?: boolean | null;
  commandPath?: string | null;
};

export type AgentBrowserSkillsResponse = {
  success: boolean;
  content: string;
  data: unknown;
};

export type AiChatCompletionRequest = {
  baseUrl: string;
  apiKey?: string | null;
  payload: unknown;
  /** Provider wire protocol. Omit or `openai-compatible` for the OpenAI Chat
   *  Completions API; `anthropic` selects the Anthropic Messages API transport. */
  protocol?: string;
};

export type AiChatCompletionResponse = {
  status: number;
  body: unknown;
};

export type AiChatHistorySaveRequest = {
  activeSessionId: string;
  sessions: unknown[];
};

export type AiChatHistoryResponse = AiChatHistorySaveRequest & {
  schemaVersion: number;
  path: string;
  recovered: boolean;
};

export type AiProviderDiagnosticResponse = {
  ok: boolean;
  status: number | null;
  latencyMs: number;
  error: string | null;
  model: string;
  baseUrl: string;
};

export type AiChatCompletionStreamRequest = AiChatCompletionRequest & {
  streamId?: string;
};

export type AiChatCompletionStreamResponse = {
  streamId: string;
};

/** Closed discriminant for the streaming chat-completion channel. Mirrors the
 *  exact `kind` strings emitted by ai_chat_backend.rs (chunk/done/error/cancelled)
 *  so handlers stay exhaustive instead of collapsing to `string`. */
export type AiChatStreamEventKind = "chunk" | "done" | "error" | "cancelled";

export type AiChatStreamEvent = {
  streamId: string;
  kind: AiChatStreamEventKind;
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

export type FileAssetResponse = {
  path: string;
  mimeType: string;
  dataUrl: string;
  size: number;
};

export type UpdateCheckResult = {
  /** Whether a newer signed build is available at the configured endpoints. */
  available: boolean;
  /** Currently running version. */
  currentVersion: string;
  /** Available version, when `available`. */
  version: string | null;
  /** Release notes for the available version, when provided. */
  notes: string | null;
};

/** Download/apply progress emitted on `lux://update` during `updateInstall`. */
export type UpdateProgress =
  | { kind: "started"; contentLength: number | null }
  | { kind: "progress"; downloaded: number; contentLength: number | null }
  | { kind: "finished" };

export type VisionEncodeResponse = {
  /** `data:<mime>;base64,<...>` ready for an `image_url` content part. */
  dataUrl: string;
  /** Produced MIME type (`image/webp`, `image/png`, or original on passthrough). */
  mimeType: string;
  /** Encoded byte length (pre-base64). */
  size: number;
  /** Output width in pixels, when known. */
  width: number | null;
  /** Output height in pixels, when known. */
  height: number | null;
  /** True when original bytes were forwarded unchanged (undecodable / smallest-wins). */
  passthrough: boolean;
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
  warnings?: string[];
  readOnly?: boolean;
};

export type AiShellClassification = {
  blocked: string | null;
  warnings: string[];
  readOnly: boolean;
};

export type AiBlackboardEntry = {
  id: string;
  author: string;
  topic: string;
  content: string;
  timestampMs: number;
};

export type AiPermissionDecision = "allow" | "deny" | "ask" | "default";

export type AiPermissionEvaluation = {
  decision: AiPermissionDecision;
  matchedRule: string | null;
};

export type AiSemanticResult = {
  type: "symbol" | "text" | "file";
  source: string;
  score: number;
  path: string;
  relativePath: string;
  line?: number;
  column?: number;
  name?: string;
  kind?: string;
  containerName?: string;
  preview?: string;
  matchText?: string;
};

export type AiSemanticSearchResponse = {
  workspaceRoot: string;
  query: string;
  pathFilter: string | null;
  count: number;
  results: AiSemanticResult[];
};

export type AiRelatedFilesResponse = {
  workspaceRoot: string;
  target: { path: string; relativePath: string; basename: string; familyStem: string } | null;
  query: string;
  scanned: number;
  count: number;
  files: {
    path: string;
    relativePath: string;
    relations: string[];
    queryHits: string[];
    score: number;
  }[];
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

// Runtime guards now live in tauriRuntime.ts (dependency-free, shared with the
// event layer). Re-exported here so every existing `from "./tauri"` import is
// unaffected by the split.
export { createDesktopRuntimeError, desktopRuntimeRequiredMessage, isBrowserPreviewRuntime, isTauriRuntime };

async function invokeRequired<T>(command: string, args?: Record<string, unknown>, decode?: IpcDecoder<T>): Promise<T> {
  if (!isTauriRuntime()) {
    throw createDesktopRuntimeError(`Command ${command}`);
  }
  const raw = await invoke<T>(command, args);
  // `T` is erased at runtime; when a decoder is supplied, validate/normalize the
  // payload at the boundary so backend drift can't corrupt trusted frontend state.
  return decode ? decodeIpcResult(command, raw, decode) : raw;
}

async function invokeOptional<T>(command: string, args: Record<string, unknown> | undefined, fallback: () => T, decode?: IpcDecoder<T>): Promise<T> {
  if (!isTauriRuntime()) {
    if (isBrowserPreviewRuntime()) return fallback();
    throw createDesktopRuntimeError(`Command ${command}`);
  }
  const raw = await invoke<T>(command, args);
  return decode ? decodeIpcResult(command, raw, decode) : raw;
}

const browserSettings = new Map<string, SettingValue>();
let browserAiChatHistory: AiChatHistoryResponse = {
  schemaVersion: 1,
  activeSessionId: "",
  sessions: [],
  path: "browser-memory://ai-chat-history",
  recovered: false,
};

/** Settings key holding the configured MCP servers (mirrors mcp.rs MCP_SERVERS_KEY). */
export const MCP_SERVERS_KEY = "ai.mcp.servers";

export type McpServerConfig = {
  id: string;
  name: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  enabled: boolean;
};

export type McpToolInfo = {
  name: string;
  description: string;
  inputSchema: unknown;
};

export type McpServerStatus = {
  id: string;
  name: string;
  state: "connected" | "connecting" | "error" | "disconnected";
  error?: string;
  tools: McpToolInfo[];
};

export const luxCommands = {
  workspaceOpen: (path: string) => invokeRequired<WorkspaceInfo>("workspace_open", { path }),
  workspaceClose: () => invokeRequired<void>("workspace_close"),
  // Best-effort release of a disposed chat session's native goals/todos/read-set
  // maps. Fire-and-forget: failure (e.g. command unavailable in a stripped build)
  // must never block JS-side session teardown.
  aiSessionDispose: (sessionId: string) => invokeOptional<void>("ai_session_dispose", { sessionId }, () => undefined),
  // Mirror checkpoint-restored goal/tasks into the native session store so the
  // next turn's FastContext doesn't re-inject rolled-back orchestration state.
  aiSessionGoalSet: (sessionId: string, goal: string) => invokeOptional<void>("ai_session_goal_set", { sessionId, goal }, () => undefined),
  aiSessionTodosSet: (sessionId: string, items: Array<{ id: string; content: string; status: string; priority: string; notes?: string }>) =>
    invokeOptional<void>("ai_session_todos_set", { sessionId, items }, () => undefined),
  workspacePickFolder: () => invokeOptional<WorkspaceInfo | null>("workspace_pick_folder", undefined, () => null),
  // The file-tree reads feed trusted explorer/AI-index state, so they validate the
  // payload shape at the IPC boundary (expectArray) instead of trusting the erased
  // generic. A non-array here is a real backend contract break, not a soft-fail.
  fsReadDir: (path: string) => invokeRequired<FsEntry[]>("fs_read_dir", { path }, expectArray<FsEntry>),
  fsReadTree: (path: string) => invokeRequired<FsEntry[]>("fs_read_tree", { path }, expectArray<FsEntry>),
  fsReadText: (path: string, maxBytes?: number) => invokeRequired<FsReadTextResponse>("fs_read_text", { path, maxBytes }),
  fsListFiles: (maxResults = 2_500) => invokeRequired<FsEntry[]>("fs_list_files", { maxResults }, expectArray<FsEntry>),
  fsCreateFile: (path: string) => invokeRequired<void>("fs_create_file", { path }),
  fsCreateDir: (path: string) => invokeRequired<void>("fs_create_dir", { path }),
  fsRename: (from: string, to: string) => invokeRequired<void>("fs_rename", { from, to }),
  fsCopy: (from: string, to: string) => invokeRequired<void>("fs_copy", { from, to }),
  fsDelete: (path: string) => invokeRequired<void>("fs_delete", { path }),
  fsRevealInFileExplorer: (path: string) => invokeRequired<void>("fs_reveal_in_file_explorer", { path }),
  fileSupportedFormats: () => invokeRequired<FileFormatSupport[]>("file_supported_formats"),
  fileInspect: (path: string, options?: Partial<FileInspectionOptions>) => invokeRequired<FileInspection>("file_inspect", { path, options: options ?? null }),
  fileMediaAiContext: (request: {
    path: string;
    sttCommand?: string | null;
    sttModelPath?: string | null;
    language?: string | null;
    maxFrames?: number;
  }) => invokeRequired<{
    transcript: string | null;
    frameDataUrls: string[];
    notes: string[];
  }>("file_media_ai_context", { request }),
  fileAssetData: (path: string) => invokeRequired<FileAssetResponse>("file_asset_data", { path }),
  aiVisionEncode: (request: {
    path?: string | null;
    dataUrl?: string | null;
    format?: "webp" | "png" | "auto";
    maxDimension?: number;
  }) => invokeRequired<VisionEncodeResponse>("ai_vision_encode", { request }),
  setScanConcurrency: (mode: "auto" | "all" | "half") =>
    invokeOptional<void>("set_scan_concurrency", { mode }, () => undefined),
  fileOpenExternal: (path: string) => invokeRequired<void>("file_open_external", { path }),
  databaseListTables: (path: string, options?: Partial<FileInspectionOptions>) =>
    invokeRequired<DatabaseTablePreview[]>("database_list_tables", { path, options: options ?? null }),
  databaseExecuteSql: (path: string, request: DatabaseExecuteRequest) =>
    invokeRequired<DatabaseExecuteResult>("database_execute_sql", { path, request }),
  databaseUpdateCell: (path: string, update: DatabaseCellUpdate) =>
    invokeRequired<void>("database_update_cell", { path, update }),
  agentBrowserStatus: (request?: AgentBrowserStatusRequest) =>
    invokeRequired<AgentBrowserStatusResponse>("agent_browser_status", { request: request ?? null }),
  agentBrowserInvoke: (request: AgentBrowserInvokeRequest) =>
    invokeRequired<AgentBrowserInvokeResponse>("agent_browser_invoke", { request }),
  agentBrowserInstall: (request?: AgentBrowserInstallRequest) =>
    invokeRequired<AgentBrowserInstallResponse>("agent_browser_install", { request: request ?? null }),
  agentBrowserReadImage: (path: string) =>
    invokeRequired<AgentBrowserReadImageResponse>("agent_browser_read_image", { request: { path } }),
  agentBrowserStreamStatus: (request: AgentBrowserStreamStatusRequest) =>
    invokeRequired<AgentBrowserStreamStatusResponse>("agent_browser_stream_status", { request }),
  agentBrowserDashboard: (request: AgentBrowserDashboardRequest) =>
    invokeRequired<AgentBrowserDashboardResponse>("agent_browser_dashboard", { request }),
  agentBrowserSkills: (request?: AgentBrowserSkillsRequest) =>
    invokeRequired<AgentBrowserSkillsResponse>("agent_browser_skills", { request: request ?? null }),
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
  aiChatHistoryLoad: () => invokeOptional<AiChatHistoryResponse>("ai_chat_history_load", undefined, () => browserAiChatHistory),
  aiChatHistorySave: (request: AiChatHistorySaveRequest) =>
    invokeOptional<AiChatHistoryResponse>("ai_chat_history_save", { request }, () => {
      browserAiChatHistory = { ...browserAiChatHistory, ...request, schemaVersion: 1 };
      return browserAiChatHistory;
    }),
  aiProviderDiagnostic: (request: AiChatCompletionRequest) => invokeRequired<AiProviderDiagnosticResponse>("ai_provider_diagnostic", { request }),
  aiChatCompletionStream: (request: AiChatCompletionStreamRequest) =>
    invokeRequired<AiChatCompletionStreamResponse>("ai_chat_completion_stream", { request }),
  aiChatCompletionStreamCancel: (streamId: string) => invokeRequired<void>("ai_chat_completion_stream_cancel", { streamId }),
  webFetch: (url: string, maxBytes?: number | null, timeoutSecs?: number | null) =>
    invokeRequired<WebFetchResponse>("web_fetch", { url, maxBytes: maxBytes ?? null, timeoutSecs: timeoutSecs ?? null }),
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
  aiShellClassify: (command: string) =>
    invokeRequired<AiShellClassification>("ai_shell_classify", { command }),
  aiBlackboardPost: (sessionId: string, author: string, topic: string, content: string) =>
    invokeRequired<AiBlackboardEntry>("ai_blackboard_post", { sessionId, author, topic, content }),
  aiBlackboardRead: (sessionId: string, topic?: string | null, limit?: number | null) =>
    invokeRequired<AiBlackboardEntry[]>("ai_blackboard_read", { sessionId, topic: topic ?? null, limit: limit ?? null }),
  aiBlackboardClear: (sessionId: string) =>
    invokeRequired<null>("ai_blackboard_clear", { sessionId }),
  aiPermissionDecide: (tool: string, input: string, rules: string[]) =>
    invokeRequired<AiPermissionEvaluation>("ai_permission_decide", { tool, input, rules }),
  aiSemanticSearch: (query: string, path?: string | null, maxResults?: number | null, maxFiles?: number | null) =>
    invokeRequired<AiSemanticSearchResponse>("ai_semantic_search", { query, path: path ?? null, maxResults: maxResults ?? null, maxFiles: maxFiles ?? null }),
  aiRelatedFiles: (path?: string | null, query?: string | null, maxResults?: number | null, maxFiles?: number | null) =>
    invokeRequired<AiRelatedFilesResponse>("ai_related_files", { path: path ?? null, query: query ?? null, maxResults: maxResults ?? null, maxFiles: maxFiles ?? null }),
  aiRepoMap: (maxFiles?: number | null) =>
    invokeRequired<{ totalListed: number; files: { path: string; size: number; modifiedAt: string | null }[] }>("ai_repo_map", { maxFiles: maxFiles ?? null }),
  aiBuildSystemPrompt: (input: {
    agentMode: string; agentName: string; agentInstructions: string;
    globalInstructions: string; projectInstructions: string; projectAgentsSnip: string;
    toolApprovalMode: string; toolRoundLimit: number | null;
    selectedEffortId: string; selectedModelAlias: string;
    providerName: string; providerProtocol: string;
    workspaceRoot: string; runtimeToolsAvailable: boolean; agentBrowserEnabled: boolean;
    tokenEconomy: boolean; customPromptEnabled: boolean; customPrompt: string;
  }) => invokeRequired<string>("ai_build_system_prompt", { input }),
  aiRunTurn: (input: AiRunTurnInput) => invokeRequired<null>("ai_run_turn", { input }),
  /** Fetch a provider's available model ids from its OpenAI-compatible /models endpoint. */
  aiListProviderModels: (baseUrl: string, apiKey: string | null) =>
    invokeRequired<string[]>("ai_list_provider_models", { baseUrl, apiKey }),
  aiResolveTurnApproval: (turnId: string, requestId: string, decision: "approved" | "rejected") =>
    invokeRequired<null>("ai_resolve_turn_approval", { turnId, requestId, decision }),
  /** Deliver a human answer to a pending AskUser question (UI → Rust). */
  aiResolveTurnQuestion: (turnId: string, requestId: string, answer: { answer: string; cancelled: boolean }) =>
    invokeRequired<null>("ai_resolve_turn_question", { turnId, requestId, answer }),
  aiCancelTurn: (turnId: string) => invokeRequired<null>("ai_cancel_turn", { turnId }),
  /** Stage a user message into a running turn. Scoped by session+turn so a
   *  restarted turn can't drain input meant for an earlier one (must match the
   *  `ai_inject_message` handler signature: session_id, turn_id, text). */
  aiInjectMessage: (sessionId: string, turnId: string, text: string) =>
    invokeRequired<null>("ai_inject_message", { sessionId, turnId, text }),
  mcpConnectAll: () => invokeRequired<McpServerStatus[]>("mcp_connect_all"),
  mcpConnect: (config: McpServerConfig) => invokeRequired<McpServerStatus>("mcp_connect", { config }),
  mcpDisconnect: (id: string) => invokeRequired<null>("mcp_disconnect", { id }),
  mcpStatus: () => invokeRequired<McpServerStatus[]>("mcp_status"),
  mcpCall: (serverId: string, tool: string, args?: Record<string, unknown>) =>
    invokeRequired<string>("mcp_call", { serverId, tool, arguments: args ?? {} }),
  mcpAdd: (config: McpServerConfig) => invokeRequired<McpServerStatus>("mcp_add", { config }),
  mcpRemove: (id: string) => invokeRequired<null>("mcp_remove", { id }),
  mcpEnable: (id: string, enabled: boolean) => invokeRequired<null>("mcp_enable", { id, enabled }),
  aiGoalEvalVerdict: (input: {
    condition: string; transcript: string; openTodoSummaries: string[];
    baseUrl: string; apiKey: string | null; model: string; protocol: string; reasoning: Record<string, unknown>;
  }) => invokeRequired<{ satisfied: boolean; blocked: boolean; reason: string; source: string } | null>("ai_goal_eval_verdict", { input }),
  aiCompactionSummary: (input: {
    transcript: string; previousSummary: string; pinnedGoal: string; openTasks: string[];
    baseUrl: string; apiKey: string | null; model: string; protocol: string; reasoning: Record<string, unknown>;
  }) => invokeRequired<string>("ai_compaction_summary", { input }),
  aiCheckpoint: (action: string, options: {
    id?: string; label?: string; paths?: string[]; maxFiles?: number;
    maxBytesPerFile?: number; saveToDisk?: boolean; dryRun?: boolean; nowMs: number;
  }) => invokeRequired<unknown>("ai_checkpoint", {
    action,
    id: options.id ?? null,
    label: options.label ?? null,
    paths: options.paths ?? null,
    maxFiles: options.maxFiles ?? null,
    maxBytesPerFile: options.maxBytesPerFile ?? null,
    saveToDisk: options.saveToDisk ?? null,
    dryRun: options.dryRun ?? null,
    nowMs: options.nowMs,
  }),
  aiWorkspaceIndex: (maxFiles?: number | null, maxScan?: number | null) =>
    invokeRequired<{
      workspaceRoot: string; scanned: number; indexedFiles: number; truncated: boolean;
      byLanguage: { key: string; count: number }[];
      byDirectory: { key: string; count: number }[];
      important: { path: string; relativePath: string; language: string; size: number }[];
      tests: { path: string; relativePath: string; language: string; size: number }[];
      source: { path: string; relativePath: string; language: string; size: number }[];
      entrypoints: { path: string; relativePath: string; language: string; size: number }[];
      largest: { path: string; relativePath: string; language: string; size: number }[];
    }>("ai_workspace_index", { maxFiles: maxFiles ?? null, maxScan: maxScan ?? null }),
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
  gitStage: (paths: string[]) => invokeRequired<GitStatus>("git_stage", { paths }),
  gitUnstage: (paths: string[]) => invokeRequired<GitStatus>("git_unstage", { paths }),
  gitDiscard: (paths: string[]) => invokeRequired<GitStatus>("git_discard", { paths }),
  gitCommit: (message: string) => invokeRequired<GitStatus>("git_commit", { message }),
  gitPush: () => invokeRequired<GitStatus>("git_push"),
  gitPull: () => invokeRequired<GitStatus>("git_pull"),
  gitBranches: () => invokeRequired<string[]>("git_branches"),
  gitCheckoutBranch: (name: string) => invokeRequired<GitStatus>("git_checkout_branch", { name }),
  gitCreateBranch: (name: string) => invokeRequired<GitStatus>("git_create_branch", { name }),
  gitFileDiff: (path: string) => invokeRequired<GitFileDiff>("git_file_diff", { path }),
  memoryCreate: (input: NewMemoryInput) => invokeRequired<MemoryCreateOutcome>("memory_create", { input }),
  memorySearch: (query: string, options?: MemorySearchOptions) => invokeRequired<ScoredMemory[]>("memory_search", { query, options }),
  memoryGet: (id: string) => invokeRequired<MemoryRecord | null>("memory_get", { id }),
  memoryUpdate: (id: string, patch: MemoryPatch) => invokeRequired<MemoryRecord>("memory_update", { id, patch }),
  memoryDelete: (id: string) => invokeRequired<boolean>("memory_delete", { id }),
  memoryList: (options?: MemorySearchOptions) => invokeRequired<MemoryRecord[]>("memory_list", { options }),
  memoryStats: () => invokeRequired<MemoryStats>("memory_stats"),
  memoryWipe: (category?: string | null) => invokeRequired<number>("memory_wipe", { category: category ?? null }),
  memoryPrune: () => invokeRequired<number>("memory_prune"),
  memoryRelate: (sourceId: string, targetId: string, relation: MemoryRelationKind, confidence?: number) =>
    invokeRequired<MemoryRelation>("memory_relate", { sourceId, targetId, relation, confidence: confidence ?? null }),
  memoryUnrelate: (relationId: string) => invokeRequired<boolean>("memory_unrelate", { relationId }),
  memoryRelations: (memoryId: string) => invokeRequired<MemoryRelation[]>("memory_relations", { memoryId }),
  memoryRelated: (memoryId: string, maxHops?: number, minConfidence?: number) =>
    invokeRequired<RelatedMemory[]>("memory_related", { memoryId, maxHops: maxHops ?? null, minConfidence: minConfidence ?? null }),
  memoryRetention: () => invokeRequired<MemoryRetentionReport>("memory_retention"),
  skillsList: () => invokeRequired<Skill[]>("skills_list"),
  skillsGet: (slug: string) => invokeRequired<Skill | null>("skills_get", { slug }),
  skillsMatch: (query: string, limit?: number) => invokeRequired<ScoredSkill[]>("skills_match", { query, limit }),
  skillsSave: (scope: SkillScope, slug: string, draft: SkillDraft) => invokeRequired<Skill>("skills_save", { scope, slug, draft }),
  skillsDelete: (scope: SkillScope, slug: string) => invokeRequired<boolean>("skills_delete", { scope, slug }),
  skillsSetEnabled: (scope: SkillScope, slug: string, enabled: boolean) => invokeRequired<boolean>("skills_set_enabled", { scope, slug, enabled }),
  skillsDiscoverImportable: () => invokeRequired<ImportableSkill[]>("skills_discover_importable"),
  skillsImport: (scope: SkillScope, slug: string, content: string) => invokeRequired<Skill>("skills_import", { scope, slug, content }),
  webResearch: (query: string, options?: ResearchOptions) => invokeRequired<ResearchResponse>("web_research", { query, options }),
  sshConnect: (host: string, user?: string | null, port?: number | null, identityFile?: string | null, label?: string | null) =>
    invokeRequired<SshConnectResult>("ssh_connect", { host, user: user ?? null, port: port ?? null, identityFile: identityFile ?? null, label: label ?? null }),
  sshExec: (sessionId: string, command: string, cwd?: string | null, timeoutSecs?: number | null) =>
    invokeRequired<SshExecResult>("ssh_exec", { sessionId, command, cwd: cwd ?? null, timeoutSecs: timeoutSecs ?? null }),
  sshTransfer: (sessionId: string, direction: SshTransferDirection, localPath: string, remotePath: string, recursive?: boolean | null) =>
    invokeRequired<SshTransferResult>("ssh_transfer", { sessionId, direction, localPath, remotePath, recursive: recursive ?? null }),
  sshList: () => invokeRequired<SshOverview>("ssh_list"),
  sshDisconnect: (sessionId?: string | null, all?: boolean | null) =>
    invokeRequired<SshDisconnectResult>("ssh_disconnect", { sessionId: sessionId ?? null, all: all ?? null }),
  extensionsList: () => invokeRequired<ExtensionInfo[]>("extensions_list"),
  extensionsActivationPlan: () => invokeRequired<ExtensionActivationPlan>("extensions_activation_plan"),
  extensionsActivate: () => invokeRequired<ExtensionActivationReport>("extensions_activate"),
  extensionsContributionRegistry: () => invokeRequired<ExtensionContributionRegistry>("extensions_contribution_registry"),
  extensionsCommandRoutes: () => invokeRequired<ExtensionCommandRoute[]>("extensions_command_routes"),
  extensionsExecuteCommand: (commandId: string) => invokeRequired<ExtensionCommandExecution>("extensions_execute_command", { commandId }),
  debugWorkspaceInfo: () => invokeRequired<DebugWorkspaceInfo>("debug_workspace_info"),
  debugStart: (configuration: DebugConfiguration, breakpoints: DebugSourceBreakpoint[] = []) => invokeRequired<DebugSessionInfo>("debug_start", { configuration, breakpoints }),
  debugStop: (sessionId: string) => invokeRequired<DebugSessionInfo>("debug_stop", { sessionId }),
  debugSessions: () => invokeRequired<DebugSessionInfo[]>("debug_sessions"),
  debugStackTrace: (sessionId: string) => invokeRequired<DebugStackTrace>("debug_stack_trace", { sessionId }),
  debugScopes: (sessionId: string, frameId: number) => invokeRequired<DebugFrameScopes>("debug_scopes", { sessionId, frameId }),
  debugVariables: (sessionId: string, variablesReference: number) => invokeRequired<DebugVariables>("debug_variables", { sessionId, variablesReference }),
  debugEvaluate: (sessionId: string, expression: string, frameId: number | null, context: DebugEvaluateContext = "repl") =>
    invokeRequired<DebugEvaluateResult>("debug_evaluate", { sessionId, expression, frameId, context }),
  debugExecute: (sessionId: string, action: DebugExecutionAction) => invokeRequired<DebugSessionInfo>("debug_execute", { sessionId, action }),
  debugSetBreakpoints: (sessionId: string, path: string, breakpoints: DebugSourceBreakpoint[]) => invokeRequired<DebugBreakpointsUpdate>("debug_set_breakpoints", { sessionId, path, breakpoints }),
  lspServers: () => invokeRequired<LanguageServerInfo[]>("lsp_servers"),
  /** Popular-language server catalog with live installed-state (managed dir + PATH). */
  lspServerCatalog: () => invokeRequired<LspCatalogEntry[]>("lsp_server_catalog"),
  /** Install (or reinstall) a language server into the managed dir; streams lux://lsp-install. */
  lspInstallServer: (languageId: string) => invokeRequired<string>("lsp_install_server", { languageId }),
  /** Remove a managed language-server install; streams lux://lsp-install (managed-only, refuses PATH installs). */
  lspUninstallServer: (languageId: string) => invokeRequired<string>("lsp_uninstall_server", { languageId }),
  /** Managed language runtimes (Node/Rust/Python) with live installed-state. */
  runtimeCatalog: () => invokeRequired<RuntimeCatalogEntry[]>("runtime_catalog"),
  /** Provision (or repair) a managed runtime; streams lux://runtime-provision. */
  runtimeProvision: (id: string) => invokeRequired<string>("runtime_provision", { id }),
  /** Build (or rebuild) the code graph for the current workspace; streams lux://code-graph. */
  codeGraphBuild: () => invokeRequired<CodeGraphSummary>("code_graph_build"),
  /** Current code-graph status (ready flag + node/edge/file counts). */
  codeGraphStatus: () => invokeRequired<CodeGraphStatus>("code_graph_status"),
  /** Query the code graph by symbol name (definition + callers/callees/neighbors). */
  codeGraphQuery: (symbol: string) => invokeRequired<CodeGraphQueryResult>("code_graph_query", { symbol }),
  /** Export an interactive code-graph.html under .lux/, returning the written path. */
  codeGraphExportHtml: () => invokeRequired<string>("code_graph_export_html"),
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
  // System font families for the appearance pickers. Browser preview has no OS
  // font scan, so the pickers degrade to the "default" entry there.
  listSystemFontFamilies: () => invokeOptional<string[]>("list_system_font_families", undefined, () => []),
  keybindingsGet: () => invokeOptional<KeybindingProfile>("keybindings_get", undefined, () => defaultKeybindingProfile()),
  keybindingsSet: (profile: KeybindingProfile) => invokeOptional<KeybindingProfile>("keybindings_set", { profile }, () => profile),
  // Auto-update. In non-desktop/browser-preview runtimes there is no updater, so
  // `updateCheck` reports "up to date" and `updateInstall` is unavailable.
  updateCheck: () => invokeOptional<UpdateCheckResult>("update_check", undefined, () => ({
    available: false,
    currentVersion: "",
    version: null,
    notes: null,
  })),
  updateInstall: () => invokeRequired<void>("update_install"),
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
    view: defaultTextView(),
    version: 1,
    is_dirty: true,
    is_untitled: true,
    opened_at: new Date().toISOString(),
  };
  browserDocuments.set(document.id, document);
  return document;
}

function defaultTextView() {
  return {
    category: "text" as const,
    strategy: "monacoText" as const,
    mode: "editableText" as const,
    displayName: "Text",
    mimeType: "text/plain",
    extensions: [],
    editable: true,
    previewable: true,
    aiReadable: true,
    binary: false,
    maxInlineBytes: 1_000_000,
    notes: [],
  };
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
  return safeListen<LuxEvent>("lux://event", handler, { label: "lux://event" });
}

export async function subscribeAiChatStream(handler: (event: AiChatStreamEvent) => void) {
  return safeListen<AiChatStreamEvent>("lux://ai-chat-stream", handler, {
    label: "lux://ai-chat-stream",
    // A thrown handler or malformed chunk must not leave the stream wedged: emit a
    // synthetic `error` event so the transport promise rejects and the chat recovers.
    onError: (error, raw) => {
      const streamId = readStringField(raw, "streamId");
      if (streamId) handler({ streamId, kind: "error", error: error instanceof Error ? error.message : String(error) });
    },
  });
}

export async function subscribeUpdateProgress(handler: (event: UpdateProgress) => void) {
  // Update progress is non-essential telemetry: stay silent (no throw) off-desktop.
  if (!isTauriRuntime()) return () => undefined;
  return safeListen<UpdateProgress>("lux://update", handler, { label: "lux://update" });
}

/** Input for the native Rust turn-loop (`ai_run_turn`). */
export type AiRunTurnInput = {
  turnId: string;
  messageId: string;
  sessionId: string;
  message: string;
  /** Fully assembled user content: plain string or OpenAI-style content-part array
   *  (text + `image_url` vision parts). Carries attachments/vision to the native turn. */
  userContent?: unknown;
  history: Array<{ role: string; content: string }>;
  baseUrl: string;
  apiKey: string | null;
  model: string;
  /** Optional embeddings model id for the active provider's `/embeddings`
   *  endpoint (semantic memory search). Empty/omitted skips embedding
   *  generation — RememberMemory/RecallMemory still work via lexical/graph
   *  search. Never used when the provider protocol is `anthropic`. */
  embeddingModel?: string | null;
  agentMode: string;
  toolRoundLimit: number | null;
  toolApprovalMode: string;
  /** Authoritative deny/ask/allow permission rules. Evaluated in the native loop
   *  before any approval prompt; deny is a hard block even in full-access/automatic. */
  toolPermissionRules: string[];
  /** Provider reasoning payload — a single field chosen per provider
   *  (`reasoning_effort` for OpenAI-compatible, `reasoning.effort` for OpenRouter),
   *  or {} when the model has no effort levels. The native turn-loop merges it into
   *  the outgoing request and omits `temperature` whenever it is non-empty. */
  reasoning: Record<string, unknown>;
  /** True for Claude-family models: the native turn-loop tags the system prompt with a
   *  cache_control breakpoint for Anthropic prompt caching. */
  anthropicCache: boolean;
  promptInput: {
    agentMode: string; agentName: string; agentInstructions: string;
    globalInstructions: string; projectInstructions: string; projectAgentsSnip: string;
    toolApprovalMode: string; toolRoundLimit: number | null;
    selectedEffortId: string; selectedModelAlias: string;
    providerName: string; providerProtocol: string;
    workspaceRoot: string; runtimeToolsAvailable: boolean; agentBrowserEnabled: boolean;
    tokenEconomy: boolean; customPromptEnabled: boolean; customPrompt: string;
  };
  agentBrowserEnabled: boolean;
  activeDocumentPath: string | null;
  openDocumentPaths: string[];
  terminalContext: unknown | null;
  /** Id of the pre-turn file checkpoint created before this turn started (see
   *  `ai_checkpoint`). When present, every native file-mutating tool call
   *  (Write/StrReplace/Delete/PatchEngine) augments this checkpoint with a
   *  pre-edit snapshot of the path(s) it is about to touch — so a later Rollback
   *  can restore files the model never explicitly ran the Checkpoint tool on. */
  fileCheckpointId?: string;
};

/** One suggested answer to an AskUser question. */
export type AiTurnQuestionOption = { label: string; description: string };
/** One step of a PresentPlan proposal. */
export type AiTurnPlanStep = { title: string; detail: string; file: string };
/** A key design decision in a PresentPlan proposal: chosen approach + tradeoff. */
export type AiPlanDecision = { option: string; tradeoff: string };

/** Status phases emitted by statusChange (mirrors ai_turn.rs StatusChange.phase). */
export type AiTurnPhase = "thinking" | "streaming" | "running-tools" | "waiting-approval" | "building-tools";
/** Terminal status of a completed tool call (mirrors ai_turn.rs ToolCallCompleted.status). */
export type AiTurnToolStatus = "success" | "error";

/** Native turn-loop events emitted by the Rust `ai_run_turn` command. */
export type AiTurnEvent =
  | { kind: "assistantCreated"; turnId: string; messageId: string }
  | { kind: "streamDelta"; turnId: string; content: string; reasoning: string }
  | { kind: "statusChange"; turnId: string; phase: AiTurnPhase }
  | { kind: "userMessageInjected"; turnId: string; text: string }
  | { kind: "toolCallStarted"; turnId: string; callId: string; tool: string; input: string }
  | { kind: "toolCallCompleted"; turnId: string; callId: string; status: AiTurnToolStatus; output: string; error: string | null }
  | { kind: "approvalRequired"; turnId: string; requestId: string; tool: string; title: string; summary: string; preview: string; risk: string }
  | { kind: "questionRequired"; turnId: string; requestId: string; question: string; detail: string; options: AiTurnQuestionOption[]; multiSelect: boolean; allowCustom: boolean; htmlPreview: string }
  | { kind: "planProposed"; turnId: string; planId: string; title: string; summary: string; steps: AiTurnPlanStep[]; alternatives: AiPlanDecision[]; risks: string[]; verification: string[]; quality: number; coaching: string[]; autoStart: boolean }
  | { kind: "turnUsage"; turnId: string; promptTokens: number; completionTokens: number; totalTokens: number; cachedPromptTokens?: number }
  | { kind: "turnDone"; turnId: string; messageId: string; content: string; durationMs: number }
  | { kind: "turnError"; turnId: string; error: string }
  | { kind: "turnRetry"; turnId: string; attempt: number; maxAttempts: number; reason: string; detail: string; delayMs: number }
  | { kind: "turnCancelled"; turnId: string };

export async function subscribeAiTurn(handler: (event: AiTurnEvent) => void) {
  return safeListen<AiTurnEvent>("lux://ai-turn", handler, {
    label: "lux://ai-turn",
    // A throwing handler mid-turn (e.g. during streamDelta/approvalRequired) must not
    // strand the turn UI: synthesize a turnError so the loop surfaces a real failure.
    onError: (error, raw) => {
      const turnId = readStringField(raw, "turnId");
      if (turnId) handler({ kind: "turnError", turnId, error: error instanceof Error ? error.message : String(error) });
    },
  });
}

/** One language server in the managed-install catalog, with live installed-state. */
export type LspCatalogEntry = {
  languageId: string;
  name: string;
  command: string;
  extensions: string[];
  /** "npm" | "go" | "pip" | "rustup" | "github" | "manual". */
  installMethod: string;
  /** Manual-install guidance (non-empty only for installMethod === "manual"). */
  manualHint: string;
  installed: boolean;
  path: string | null;
  /** True when found in the IDE's managed dir (vs. the user's own PATH). */
  managed: boolean;
};

/** Progress events emitted by `lsp_install_server` on lux://lsp-install. */
export type LspInstallEvent =
  | { kind: "started"; languageId: string; name: string }
  | { kind: "progress"; languageId: string; percent: number; step: string }
  | { kind: "finished"; languageId: string; success: boolean; path: string | null; error: string | null };

export async function subscribeLspInstall(handler: (event: LspInstallEvent) => void) {
  return safeListen<LspInstallEvent>("lux://lsp-install", handler, { label: "lux://lsp-install" });
}

/** One managed language runtime (Node/Rust/Python), with live installed-state. */
export type RuntimeCatalogEntry = {
  id: string;
  name: string;
  installed: boolean;
  /** True when satisfied by the IDE's managed dir (vs. the user's own PATH). */
  managed: boolean;
  path: string | null;
  /** False when this platform has no automated install (UI shows manualHint). */
  canAuto: boolean;
  manualHint: string;
};

/** Progress events emitted by `runtime_provision` on lux://runtime-provision. */
export type RuntimeProvisionEvent =
  | { kind: "started"; id: string; name: string }
  | { kind: "progress"; id: string; percent: number; step: string }
  | { kind: "finished"; id: string; success: boolean; path: string | null; error: string | null };

export async function subscribeRuntimeProvision(handler: (event: RuntimeProvisionEvent) => void) {
  return safeListen<RuntimeProvisionEvent>("lux://runtime-provision", handler, { label: "lux://runtime-provision" });
}

/** Summary returned by code_graph_build / reflected in code_graph_status. */
export type CodeGraphSummary = {
  nodeCount: number;
  edgeCount: number;
  fileCount: number;
};

/** Live code-graph status — ready flag plus counts. */
export type CodeGraphStatus = {
  ready: boolean;
  nodeCount: number;
  edgeCount: number;
  fileCount: number;
};

/** A definition node referenced in a code-graph query result. */
export type CodeGraphNode = {
  name: string;
  file: string;
  line: number;
};

/** One neighbor connection with its relation and direction. */
export type CodeGraphConnection = {
  name: string;
  file: string;
  line: number;
  relation: string;
  direction: string;
};

/** Result of code_graph_query for a single symbol. */
export type CodeGraphQueryResult = {
  found: boolean;
  node: CodeGraphNode | null;
  callers: CodeGraphNode[];
  callees: CodeGraphNode[];
  neighbors: CodeGraphConnection[];
  explanation: {
    kind: string;
    degree: number;
    totalConnections: number;
    connections: CodeGraphConnection[];
  } | null;
};

/** Progress events emitted by code_graph_build on lux://code-graph. */
export type CodeGraphEvent =
  | { kind: "started"; path: string }
  | { kind: "progress"; percent: number; step: string }
  | { kind: "finished"; success: boolean; nodeCount: number; edgeCount: number; error: string | null }
  | { kind: "updated"; nodeCount: number; edgeCount: number };

export async function subscribeCodeGraph(handler: (event: CodeGraphEvent) => void) {
  return safeListen<CodeGraphEvent>("lux://code-graph", handler, { label: "lux://code-graph" });
}

