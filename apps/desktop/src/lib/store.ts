import { create } from "zustand";
import { defaultAiPreferences, mergeAiPreferences, type AiPreferences } from "./aiPreferences";
import type { AiChatContextBudgetReport } from "./aiChatContextReport";
import type { ContextCompactionState } from "./aiChatContextCompaction";
import type { PersistedPendingFileReview } from "./aiPendingFileReview";
import type { AiSessionTodo } from "./aiSessionTodos";
import { disposeAiChatSessionSideState } from "./aiChatSessionLifecycle";
import { normalizeAiMessageReasoning } from "./aiChatReasoning";
import type { AiChatMessage, AiMessageSegment } from "./aiChatTypes";
import type { AiProjectIndexBucket, AiProjectIndexFile, AiProjectIndexQuality, AiProjectIndexSource } from "./aiProjectIndex";
import { defaultEditorPreferences, mergeEditorPreferences, type EditorPreferences } from "./editorPreferences";
import { normalizePath, sameWorkspaceRoot, type FileTreeDirectories } from "./fileTree";
import { DEFAULT_LOCALE, type Locale } from "./i18n";
import { defaultKeybindingProfile } from "./keybindings";
import { applyTextEdits } from "./documentEdits";
import {
  fileTreeDirectoriesEqual,
  flattenDiagnostics,
  fsEntriesEqual,
  gitStatusEqual,
  languageServersEqual,
  searchResponsesEqual,
} from "./storeEquality";
import { appendTerminalChunks, emptyTerminalBuffer, terminalOutputCoalescer } from "./terminalOutput";
import type { TerminalOutputBuffer } from "./terminalTypes";
import type { DebugBreakpointsUpdate, DebugResolvedBreakpoint, DebugSourceBreakpoint, DocumentEditResult, DocumentSnapshot, FsEntry, GitStatus, KeybindingProfile, LanguageServerInfo, SearchResponse, TerminalSessionInfo, TextEdit, WorkspaceDiagnostic, WorkspaceInfo } from "./types";

export type Activity = "explorer" | "search" | "git" | "runDebug" | "extensions";
export type BottomPanelTab = "problems" | "output" | "terminal";
export type AiIndexStatus = "disabled" | "idle" | "indexing" | "ready";
export type WorkspaceMode = "agent" | "workspace";
export type AiChatSessionStatus = "idle" | "thinking" | "streaming" | "preparing" | "running-tools" | "waiting-approval" | "error";
export type ProjectLoadStage = "idle" | "opening" | "files" | "services" | "indexing" | "ready" | "error";

export type ProjectLoadState = {
  active: boolean;
  error: string | null;
  progress: number;
  root: string | null;
  stage: ProjectLoadStage;
  workspaceName: string | null;
};

export type AiIndexState = {
  status: AiIndexStatus;
  progress: number;
  indexedFiles: number;
  totalFiles: number;
  ignoredFiles: number;
  truncatedFiles: number;
  totalBytes: number;
  sourceFiles: number;
  testFiles: number;
  configFiles: number;
  rulesFiles: number;
  docsFiles: number;
  memoryFiles: number;
  durationMs: number | null;
  scanLimit: number | null;
  scanTruncated: boolean;
  source: AiProjectIndexSource;
  lastError: string | null;
  quality: AiProjectIndexQuality;
  languageCounts: AiProjectIndexBucket[];
  topDirectories: AiProjectIndexBucket[];
  importantFiles: AiProjectIndexFile[];
  workspaceRoot: string | null;
  updatedAt: string | null;
};

export type EditorGroup = {
  id: string;
  documentIds: string[];
  activeDocumentId: string | null;
};

export type EditorRevealTarget = {
  documentId: string;
  line: number;
  column: number;
};

/** One failed attempt shown in the error card's history disclosure; consecutive
 *  identical failures collapse into a single entry with a bumped `count`. */
export type AiChatErrorHistoryEntry = {
  message: string;
  timestamp: number;
  count: number;
};

export type AiChatSession = {
  id: string;
  title: string;
  workspaceRoot: string | null;
  messages: AiChatMessage[];
  contextCompaction?: ContextCompactionState | null;
  contextBudgetReport?: AiChatContextBudgetReport | null;
  sessionTodos?: AiSessionTodo[];
  sessionGoal?: string;
  pendingFileReviews?: PersistedPendingFileReview[];
  pinned?: boolean;
  status: AiChatSessionStatus;
  lastError: string | null;
  /** Failures accumulated across the current retry ladder; cleared on a clean idle. */
  errorHistory?: AiChatErrorHistoryEntry[];
  closedAt: number | null;
  createdAt: number;
  updatedAt: number;
};

export type AiChatSessionState = {
  activeSessionId: string;
  sessions: AiChatSession[];
};

export type CreateAiChatSessionResult = {
  id: string;
  reused: boolean;
};

export function isAiChatSessionBusyStatus(status: AiChatSessionStatus) {
  return status === "thinking" || status === "streaming" || status === "preparing" || status === "running-tools" || status === "waiting-approval";
}

const DEFAULT_EDITOR_GROUP_ID = "editor-group-1";
const createEditorGroupId = () => `editor-group-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
const createEmptyEditorGroup = (id = DEFAULT_EDITOR_GROUP_ID): EditorGroup => ({ id, documentIds: [], activeDocumentId: null });
const createAiChatSessionId = () => `chat-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

export function createEmptyAiIndexState(status: AiIndexStatus = "idle"): AiIndexState {
  return {
    status,
    progress: 0,
    indexedFiles: 0,
    totalFiles: 0,
    ignoredFiles: 0,
    truncatedFiles: 0,
    totalBytes: 0,
    sourceFiles: 0,
    testFiles: 0,
    configFiles: 0,
    rulesFiles: 0,
    docsFiles: 0,
    memoryFiles: 0,
    durationMs: null,
    scanLimit: null,
    scanTruncated: false,
    source: "file-tree",
    lastError: null,
    quality: "empty",
    languageCounts: [],
    topDirectories: [],
    importantFiles: [],
    workspaceRoot: null,
    updatedAt: null,
  };
}

export function createIdleProjectLoadState(): ProjectLoadState {
  return {
    active: false,
    error: null,
    progress: 0,
    root: null,
    stage: "idle",
    workspaceName: null,
  };
}

type LuxState = {
  workspaceMode: WorkspaceMode;
  activeActivity: Activity;
  sidebarVisible: boolean;
  aiChatOpen: boolean;
  settingsOpen: boolean;
  settingsInitialSection: string | null;
  /** True when a newer signed build is available (drives the title-bar badge). */
  updateAvailable: boolean;
  locale: Locale;
  aiPreferences: AiPreferences;
  aiIndex: AiIndexState;
  aiChatSessions: AiChatSession[];
  activeAiChatSessionId: string;
  editorPreferences: EditorPreferences;
  keybindingProfile: KeybindingProfile;
  workspace: WorkspaceInfo | null;
  workspaceFolders: WorkspaceInfo[];
  projectLoad: ProjectLoadState;
  fileEntries: FsEntry[];
  fileTreeDirectories: FileTreeDirectories;
  fileTreeLoading: boolean;
  fileTreeError: string | null;
  explorerExpandedPaths: string[];
  openDocuments: DocumentSnapshot[];
  activeDocumentId: string | null;
  pendingEditorReveal: EditorRevealTarget | null;
  editorGroups: EditorGroup[];
  activeEditorGroupId: string;
  searchResponse: SearchResponse | null;
  terminal: TerminalSessionInfo | null;
  terminalSessions: TerminalSessionInfo[];
  activeTerminalId: string | null;
  terminalOutputBuffers: Record<string, TerminalOutputBuffer>;
  gitStatus: GitStatus | null;
  languageServers: LanguageServerInfo[];
  languageServersLoading: boolean;
  diagnosticsByPath: Record<string, WorkspaceDiagnostic[]>;
  debugSourceBreakpointsByPath: Record<string, DebugSourceBreakpoint[]>;
  debugResolvedBreakpointsByPath: Record<string, DebugResolvedBreakpoint[]>;
  commandPaletteOpen: boolean;
  bottomPanelOpen: boolean;
  bottomPanelTab: BottomPanelTab;
  setWorkspaceMode: (mode: WorkspaceMode) => void;
  setActiveActivity: (activity: Activity) => void;
  setSidebarVisible: (visible: boolean) => void;
  toggleSidebar: () => void;
  setAiChatOpen: (open: boolean) => void;
  toggleAiChat: () => void;
  setSettingsOpen: (open: boolean) => void;
  openSettingsSection: (sectionId: string) => void;
  setUpdateAvailable: (available: boolean) => void;
  setLocale: (locale: Locale) => void;
  updateAiPreferences: (preferences: Partial<AiPreferences>) => void;
  setAiPreferences: (preferences: AiPreferences) => void;
  setAiIndex: (index: Partial<AiIndexState>) => void;
  setAiChatSessionContextBudgetReport: (sessionId: string, report: AiChatContextBudgetReport | null) => void;
  setAiChatSessions: (state: AiChatSessionState) => void;
  createAiChatSession: (workspaceRoot?: string | null) => string;
  ensureAiChatSession: (workspaceRoot?: string | null) => CreateAiChatSessionResult;
  setActiveAiChatSession: (sessionId: string) => void;
  closeAiChatSession: (sessionId: string) => void;
  restoreAiChatSession: (sessionId: string) => void;
  renameAiChatSession: (sessionId: string, title: string) => void;
  pinAiChatSession: (sessionId: string, pinned: boolean) => void;
  deleteAiChatSession: (sessionId: string) => void;
  appendAiChatMessage: (sessionId: string, message: AiChatMessage) => void;
  updateAiChatMessage: (sessionId: string, messageId: string, patch: Partial<AiChatMessage>) => void;
  replaceAiChatMessages: (sessionId: string, messages: AiChatMessage[], options?: { contextCompaction?: ContextCompactionState | null }) => void;
  setAiChatSessionStatus: (sessionId: string, status: AiChatSessionStatus, lastError?: string | null) => void;
  clearAiChatErrorHistory: (sessionId: string) => void;
  appendAiChatSessionError: (sessionId: string, message: string) => void;
  updateEditorPreferences: (preferences: Partial<EditorPreferences>) => void;
  setEditorPreferences: (preferences: EditorPreferences) => void;
  setKeybindingProfile: (profile: KeybindingProfile) => void;
  setWorkspace: (workspace: WorkspaceInfo | null) => void;
  setProjectLoad: (state: Partial<ProjectLoadState>) => void;
  addWorkspaceFolder: (workspace: WorkspaceInfo) => void;
  removeWorkspaceFolder: (root: string) => void;
  setFileEntries: (entries: FsEntry[]) => void;
  setFileTreeDirectories: (directories: FileTreeDirectories) => void;
  setFileTreeLoading: (loading: boolean) => void;
  setFileTreeError: (error: string | null) => void;
  setExplorerExpandedPaths: (paths: Iterable<string>) => void;
  ensureExplorerExpandedPath: (path: string) => void;
  toggleExplorerExpandedPath: (path: string) => void;
  upsertDocument: (document: DocumentSnapshot) => void;
  updateOpenDocuments: (documents: DocumentSnapshot[]) => void;
  replaceDocumentSnapshot: (document: DocumentSnapshot) => void;
  applyDocumentEdits: (documentId: string, edits: TextEdit[], result: DocumentEditResult) => void;
  setActiveDocument: (id: string) => void;
  setActiveEditorGroup: (id: string) => void;
  setActiveDocumentInGroup: (groupId: string, documentId: string) => void;
  setPendingEditorReveal: (target: EditorRevealTarget | null) => void;
  consumePendingEditorReveal: (documentId: string) => EditorRevealTarget | null;
  splitActiveEditor: () => void;
  splitDocumentInGroup: (groupId: string, documentId: string, side: "left" | "right") => void;
  closeEditorGroup: (groupId: string) => void;
  closeDocumentInGroup: (groupId: string, documentId: string) => void;
  closeOtherDocumentsInGroup: (groupId: string, documentId: string) => void;
  closeDocumentsToRightInGroup: (groupId: string, documentId: string) => void;
  closeSavedDocumentsInGroup: (groupId: string) => void;
  closeDocumentInActiveGroup: () => void;
  closeDocument: (id: string) => void;
  closeOtherDocuments: (id: string) => void;
  closeAllDocuments: () => void;
  selectNextDocument: () => void;
  selectPreviousDocument: () => void;
  setSearchResponse: (response: SearchResponse | null) => void;
  setTerminal: (terminal: TerminalSessionInfo | null) => void;
  upsertTerminalSession: (terminal: TerminalSessionInfo, makeActive?: boolean) => void;
  setActiveTerminal: (terminalId: string) => void;
  closeTerminalSession: (terminalId: string) => void;
  closeAllTerminalSessions: () => void;
  appendTerminalOutput: (terminalId: string, data: string) => void;
  /** Internal: commits a coalesced batch of terminal chunks (one frame's worth). */
  commitTerminalOutput: (pending: ReadonlyMap<string, string[]>) => void;
  clearTerminalOutput: (terminalId: string) => void;
  clearAllTerminalOutput: () => void;
  setGitStatus: (status: GitStatus | null) => void;
  setLanguageServers: (servers: LanguageServerInfo[]) => void;
  setLanguageServersLoading: (loading: boolean) => void;
  setDiagnosticsForPath: (path: string, diagnostics: WorkspaceDiagnostic[]) => void;
  clearDiagnostics: () => void;
  toggleDebugSourceBreakpoint: (path: string, line: number) => void;
  setDebugResolvedBreakpoints: (update: DebugBreakpointsUpdate) => void;
  clearDebugBreakpoints: () => void;
  setCommandPaletteOpen: (open: boolean) => void;
  openBottomPanel: (tab: BottomPanelTab) => void;
  toggleBottomPanel: (tab: BottomPanelTab) => void;
  setBottomPanelOpen: (open: boolean) => void;
  setBottomPanelTab: (tab: BottomPanelTab) => void;
};

export const useLuxStore = create<LuxState>((set, get) => ({
  workspaceMode: "workspace",
  activeActivity: "explorer",
  sidebarVisible: true,
  aiChatOpen: false,
  settingsOpen: false,
  updateAvailable: false,
  settingsInitialSection: null,
  locale: DEFAULT_LOCALE,
  aiPreferences: defaultAiPreferences,
  aiIndex: createEmptyAiIndexState(),
  ...createInitialAiChatState(),
  editorPreferences: defaultEditorPreferences,
  keybindingProfile: defaultKeybindingProfile(),
  workspace: null,
  workspaceFolders: [],
  projectLoad: createIdleProjectLoadState(),
  fileEntries: [],
  fileTreeDirectories: {},
  fileTreeLoading: false,
  fileTreeError: null,
  explorerExpandedPaths: [],
  openDocuments: [],
  activeDocumentId: null,
  pendingEditorReveal: null,
  editorGroups: [createEmptyEditorGroup()],
  activeEditorGroupId: DEFAULT_EDITOR_GROUP_ID,
  searchResponse: null,
  terminal: null,
  terminalSessions: [],
  activeTerminalId: null,
  terminalOutputBuffers: {},
  gitStatus: null,
  languageServers: [],
  languageServersLoading: false,
  diagnosticsByPath: {},
  debugSourceBreakpointsByPath: {},
  debugResolvedBreakpointsByPath: {},
  commandPaletteOpen: false,
  bottomPanelOpen: false,
  bottomPanelTab: "terminal",
  setWorkspaceMode: (workspaceMode) => set({ workspaceMode }),
  setActiveActivity: (activity) => set({ activeActivity: activity }),
  setSidebarVisible: (sidebarVisible) => set({ sidebarVisible }),
  toggleSidebar: () => set((state) => ({ sidebarVisible: !state.sidebarVisible })),
  setAiChatOpen: (aiChatOpen) => set({ aiChatOpen }),
  toggleAiChat: () => set((state) => ({ aiChatOpen: !state.aiChatOpen })),
  setSettingsOpen: (settingsOpen) => set({ settingsOpen, ...(settingsOpen ? {} : { settingsInitialSection: null }) }),
  setUpdateAvailable: (updateAvailable) => set({ updateAvailable }),
  openSettingsSection: (sectionId) => set({ settingsOpen: true, settingsInitialSection: sectionId }),
  setLocale: (locale) => set({ locale }),
  updateAiPreferences: (preferences) => set((state) => ({ aiPreferences: mergeAiPreferences(state.aiPreferences, preferences) })),
  setAiPreferences: (aiPreferences) => set({ aiPreferences }),
  setAiIndex: (index) => set((state) => ({ aiIndex: { ...state.aiIndex, ...index } })),
  setAiChatSessionContextBudgetReport: (sessionId, report) =>
    set((state) => ({
      aiChatSessions: state.aiChatSessions.map((session) => session.id === sessionId
        ? { ...session, contextBudgetReport: report, updatedAt: Date.now() }
        : session),
    })),
  setAiChatSessions: (chatState) => set((state) => mergeAiChatSessionState(state, chatState)),
  createAiChatSession: (workspaceRoot = get().workspace?.root ?? null) => {
    const session = createAiChatSession(workspaceRoot);
    set((state) => ({
      aiChatSessions: [session, ...state.aiChatSessions],
      activeAiChatSessionId: session.id,
      aiChatOpen: true,
    }));
    return session.id;
  },
  ensureAiChatSession: (workspaceRoot = get().workspace?.root ?? null) => {
    const current = get();
    const reusable = current.aiChatSessions.find((session) => !session.closedAt && sameWorkspaceRoot(session.workspaceRoot, workspaceRoot) && isEmptyAiChatSession(session));
    if (reusable) {
      set({ activeAiChatSessionId: reusable.id, aiChatOpen: true });
      return { id: reusable.id, reused: true };
    }
    const session = createAiChatSession(workspaceRoot);
    set((state) => ({
      aiChatSessions: [session, ...state.aiChatSessions],
      activeAiChatSessionId: session.id,
      aiChatOpen: true,
    }));
    return { id: session.id, reused: false };
  },
  setActiveAiChatSession: (sessionId) => set((state) => {
    const target = state.aiChatSessions.find((session) => session.id === sessionId);
    if (!target) return {};
    // Selecting a session from history must always yield a usable, open chat. If the
    // target was archived (closedAt set), re-open it — otherwise it would render with
    // the composer disabled ("shows but won't open / can't do anything").
    if (target.closedAt) {
      const now = Date.now();
      return {
        activeAiChatSessionId: sessionId,
        aiChatOpen: true,
        aiChatSessions: state.aiChatSessions.map((session) =>
          session.id === sessionId ? { ...session, closedAt: null, updatedAt: now } : session),
      };
    }
    return {
      activeAiChatSessionId: sessionId,
      aiChatOpen: true,
    };
  }),
  closeAiChatSession: (sessionId) =>
    set((state) => {
      const target = state.aiChatSessions.find((session) => session.id === sessionId);
      if (!target || target.closedAt) return {};
      const now = Date.now();
      const sessions = state.aiChatSessions.map((session) => session.id === sessionId
        ? { ...session, closedAt: now, updatedAt: now }
        : session);
      if (state.activeAiChatSessionId !== sessionId) return { aiChatSessions: sessions };
      const nextActiveSession = sessions.find((session) => !session.closedAt && session.id !== sessionId);
      if (!nextActiveSession) {
        const fallback = createAiChatSession(state.workspace?.root ?? null);
        return { aiChatSessions: [fallback, ...sessions], activeAiChatSessionId: fallback.id, aiChatOpen: true };
      }
      return { aiChatSessions: sessions, activeAiChatSessionId: nextActiveSession.id };
    }),
  restoreAiChatSession: (sessionId) =>
    set((state) => {
      const target = state.aiChatSessions.find((session) => session.id === sessionId);
      if (!target) return {};
      const now = Date.now();
      return {
        activeAiChatSessionId: sessionId,
        aiChatOpen: true,
        aiChatSessions: state.aiChatSessions.map((session) => session.id === sessionId
          ? { ...session, closedAt: null, updatedAt: now }
          : session),
      };
    }),
  renameAiChatSession: (sessionId, title) =>
    set((state) => ({
      aiChatSessions: state.aiChatSessions.map((session) => session.id === sessionId ? { ...session, title: normalizeChatTitle(title), updatedAt: Date.now() } : session),
    })),
  pinAiChatSession: (sessionId, pinned) =>
    set((state) => ({
      aiChatSessions: state.aiChatSessions.map((session) => session.id === sessionId ? { ...session, pinned, updatedAt: Date.now() } : session),
    })),
  deleteAiChatSession: (sessionId) =>
    set((state) => {
      if (!state.aiChatSessions.some((session) => session.id === sessionId)) return {};
      disposeAiChatSessionSideState(sessionId);
      const aiChatSessions = state.aiChatSessions.filter((session) => session.id !== sessionId);
      if (aiChatSessions.length === 0) {
        const fallback = createAiChatSession(state.workspace?.root ?? null);
        return { aiChatSessions: [fallback], activeAiChatSessionId: fallback.id, aiChatOpen: true };
      }
      if (state.activeAiChatSessionId !== sessionId) return { aiChatSessions };
      const nextOpenSession = aiChatSessions.find((session) => !session.closedAt);
      if (nextOpenSession) return { aiChatSessions, activeAiChatSessionId: nextOpenSession.id };
      const fallback = createAiChatSession(state.workspace?.root ?? null);
      return { aiChatSessions: [fallback, ...aiChatSessions], activeAiChatSessionId: fallback.id, aiChatOpen: true };
    }),
  appendAiChatMessage: (sessionId, message) =>
    set((state) => ({
      aiChatSessions: state.aiChatSessions.map((session) => {
        if (session.id !== sessionId) return session;
        const messages = [...session.messages, message];
        return { ...session, messages, title: nextChatSessionTitle(session, message), lastError: null, updatedAt: Date.now() };
      }),
    })),
  updateAiChatMessage: (sessionId, messageId, patch) =>
    set((state) => {
      // Token streaming calls this for every delta. Locate the target session and
      // message by index and bail early when either is missing or the patch is a
      // no-op, so a redundant patch does not wake every store subscriber or bump
      // session metadata. Only the matching session/message is rebuilt.
      const sessionIndex = state.aiChatSessions.findIndex((session) => session.id === sessionId);
      if (sessionIndex === -1) return {};
      const session = state.aiChatSessions[sessionIndex];
      const messageIndex = session.messages.findIndex((message) => message.id === messageId);
      if (messageIndex === -1) return {};
      const current = session.messages[messageIndex];
      if (isNoOpMessagePatch(current, patch)) return {};

      const messages = session.messages.slice();
      messages[messageIndex] = { ...current, ...patch };
      const aiChatSessions = state.aiChatSessions.slice();
      aiChatSessions[sessionIndex] = { ...session, messages, updatedAt: Date.now() };
      return { aiChatSessions };
    }),
  replaceAiChatMessages: (sessionId, messages, options) =>
    set((state) => ({
      aiChatSessions: state.aiChatSessions.map((session) => session.id === sessionId
        ? {
          ...session,
          messages,
          contextCompaction: options && "contextCompaction" in options ? options.contextCompaction ?? null : session.contextCompaction,
          title: titleFromMessages(messages) ?? session.title,
          updatedAt: Date.now(),
        }
        : session),
    })),
  setAiChatSessionStatus: (sessionId, status, lastError = null) =>
    set((state) => ({
      aiChatSessions: state.aiChatSessions.map((session) => {
        if (session.id !== sessionId) return session;
        // Error history feeds the error card's "previous attempts" disclosure:
        // every failure appends an entry, a clean idle end clears the ladder, and
        // intermediate statuses (e.g. "thinking" between auto-retries) keep it.
        const errorHistory = lastError
          ? appendAiChatErrorHistory(session.errorHistory, lastError)
          : status === "idle"
            ? undefined
            : session.errorHistory;
        return { ...session, status, lastError, errorHistory, updatedAt: status === "idle" ? session.updatedAt : Date.now() };
      }),
    })),
  // A fresh (non-retry) send starts a new logical turn — the previous failure's
  // retry ladder must not leak under a later, unrelated error card.
  clearAiChatErrorHistory: (sessionId) =>
    set((state) => ({
      aiChatSessions: state.aiChatSessions.map((session) =>
        session.id === sessionId && session.errorHistory !== undefined
          ? { ...session, errorHistory: undefined }
          : session),
    })),
  // Record a transient failure in the session's error history WITHOUT changing
  // status/lastError — used for the backend's in-turn retry notices (rate limit,
  // provider hiccups) so the "Retrying" banner's error-history disclosure fills
  // up live during the ladder, before the turn has ultimately failed.
  appendAiChatSessionError: (sessionId, message) =>
    set((state) => ({
      aiChatSessions: state.aiChatSessions.map((session) =>
        session.id === sessionId
          ? { ...session, errorHistory: appendAiChatErrorHistory(session.errorHistory, message) }
          : session),
    })),
  updateEditorPreferences: (preferences) => set((state) => ({ editorPreferences: mergeEditorPreferences(state.editorPreferences, preferences) })),
  setEditorPreferences: (editorPreferences) => set({ editorPreferences }),
  setKeybindingProfile: (keybindingProfile) => set({ keybindingProfile }),
  setWorkspace: (workspace) =>
    set((state) => {
      const sameWorkspace = Boolean(workspace && sameWorkspaceRoot(state.workspace?.root, workspace.root));
      const workspaceChatSession = workspace
        ? state.aiChatSessions.find((session) => !session.closedAt && sameWorkspaceRoot(session.workspaceRoot, workspace.root))
        : state.aiChatSessions.find((session) => !session.closedAt && session.workspaceRoot === null) ?? null;
      const fallbackChatSession = workspace ? createAiChatSession(workspace.root) : createAiChatSession(null);
      const aiChatSessions = sameWorkspace || !fallbackChatSession
        ? state.aiChatSessions
        : workspaceChatSession
          ? state.aiChatSessions
          : [fallbackChatSession, ...state.aiChatSessions];
      const activeAiChatSessionId = sameWorkspace
        ? state.activeAiChatSessionId
        : workspaceChatSession?.id ?? fallbackChatSession?.id ?? state.activeAiChatSessionId;
      return {
      workspace,
      projectLoad: workspace ? state.projectLoad : createIdleProjectLoadState(),
      aiChatOpen: workspace ? (sameWorkspace ? state.aiChatOpen : true) : false,
      aiChatSessions,
      activeAiChatSessionId,
      workspaceFolders: workspace
        ? state.workspaceFolders.some((folder) => folder.root === workspace.root)
          ? state.workspaceFolders
          : [workspace]
        : [],
      sidebarVisible: workspace ? true : false,
      bottomPanelOpen: false,
      fileEntries: workspace && sameWorkspace ? state.fileEntries : [],
      fileTreeDirectories: workspace && sameWorkspace ? state.fileTreeDirectories : {},
      fileTreeLoading: false,
      fileTreeError: null,
      openDocuments: workspace && sameWorkspace ? state.openDocuments : [],
      activeDocumentId: workspace && sameWorkspace ? state.activeDocumentId : null,
      pendingEditorReveal: null,
      editorGroups: workspace && sameWorkspace ? state.editorGroups : [createEmptyEditorGroup()],
      activeEditorGroupId: workspace && sameWorkspace ? state.activeEditorGroupId : DEFAULT_EDITOR_GROUP_ID,
      terminal: null,
      terminalSessions: [],
      activeTerminalId: null,
      terminalOutputBuffers: {},
      languageServers: workspace && sameWorkspace ? state.languageServers : [],
      languageServersLoading: false,
      diagnosticsByPath: workspace && sameWorkspace ? state.diagnosticsByPath : {},
      debugSourceBreakpointsByPath: workspace && sameWorkspace ? state.debugSourceBreakpointsByPath : {},
      debugResolvedBreakpointsByPath: workspace && sameWorkspace ? state.debugResolvedBreakpointsByPath : {},
      explorerExpandedPaths: workspace
        ? sameWorkspace && state.explorerExpandedPaths.length > 0
          ? state.explorerExpandedPaths
          : [normalizePath(workspace.root)]
        : [],
      };
    }),
  setProjectLoad: (projectLoad) => set((state) => ({ projectLoad: { ...state.projectLoad, ...projectLoad } })),
  addWorkspaceFolder: (workspace) =>
    set((state) => ({
      workspaceFolders: state.workspaceFolders.some((folder) => folder.root === workspace.root)
        ? state.workspaceFolders
        : [...state.workspaceFolders, workspace],
    })),
  removeWorkspaceFolder: (root) =>
    set((state) => ({
      workspaceFolders: state.workspaceFolders.filter((folder) => folder.root !== root),
    })),
  // Equality-aware: explorer/source-control/search/LSP snapshots are frequently
  // re-sent unchanged by polling/events; skipping the write avoids re-rendering
  // large UI regions for a semantically identical payload (new reference only).
  setFileEntries: (fileEntries) =>
    set((state) => fsEntriesEqual(state.fileEntries, fileEntries) ? {} : { fileEntries }),
  setFileTreeDirectories: (fileTreeDirectories) =>
    set((state) => fileTreeDirectoriesEqual(state.fileTreeDirectories, fileTreeDirectories) ? {} : { fileTreeDirectories }),
  setFileTreeLoading: (fileTreeLoading) => set({ fileTreeLoading }),
  setFileTreeError: (fileTreeError) => set({ fileTreeError }),
  setExplorerExpandedPaths: (paths) => set({ explorerExpandedPaths: normalizePathList(paths) }),
  ensureExplorerExpandedPath: (path) =>
    set((state) => {
      const normalizedPath = normalizePath(path);
      if (state.explorerExpandedPaths.includes(normalizedPath)) return {};
      return { explorerExpandedPaths: [...state.explorerExpandedPaths, normalizedPath] };
    }),
  toggleExplorerExpandedPath: (path) =>
    set((state) => {
      const normalizedPath = normalizePath(path);
      return {
        explorerExpandedPaths: state.explorerExpandedPaths.includes(normalizedPath)
          ? state.explorerExpandedPaths.filter((candidate) => candidate !== normalizedPath)
          : [...state.explorerExpandedPaths, normalizedPath],
      };
    }),
  upsertDocument: (document) =>
    set((state) => {
      const exists = state.openDocuments.some((candidate) => candidate.id === document.id);
      const editorGroups = ensureEditorGroups(state.editorGroups);
      const activeGroup = editorGroups.find((group) => group.id === state.activeEditorGroupId) ?? editorGroups[0];
      return {
        openDocuments: exists
          ? state.openDocuments.map((candidate) => (candidate.id === document.id ? document : candidate))
          : [...state.openDocuments, document],
        editorGroups: editorGroups.map((group) =>
          group.id === activeGroup.id
            ? {
                ...group,
                documentIds: group.documentIds.includes(document.id) ? group.documentIds : [...group.documentIds, document.id],
                activeDocumentId: document.id,
              }
            : group,
        ),
        activeEditorGroupId: activeGroup.id,
        activeDocumentId: document.id,
      };
    }),
  updateOpenDocuments: (documents) =>
    set((state) => {
      if (documents.length === 0) return {};
      const byId = new Map(documents.map((document) => [document.id, document]));
      return {
        openDocuments: state.openDocuments.map((document) => byId.get(document.id) ?? document),
      };
    }),
  replaceDocumentSnapshot: (document) =>
    set((state) => {
      if (!state.openDocuments.some((candidate) => candidate.id === document.id)) return {};
      return {
        openDocuments: state.openDocuments.map((candidate) => (candidate.id === document.id ? document : candidate)),
      };
    }),
  applyDocumentEdits: (documentId, edits, result) =>
    set((state) => ({
      openDocuments: state.openDocuments.map((document) => {
        if (document.id !== documentId) return document;
        return {
          ...document,
          text: applyTextEdits(document.text, edits),
          version: result.version,
          is_dirty: result.is_dirty,
        };
      }),
    })),
  setActiveDocument: (activeDocumentId) =>
    set((state) => {
      if (!state.openDocuments.some((document) => document.id === activeDocumentId)) return {};
      const editorGroups = ensureEditorGroups(state.editorGroups);
      const activeGroup = editorGroups.find((group) => group.id === state.activeEditorGroupId) ?? editorGroups[0];
      return {
        activeDocumentId,
        activeEditorGroupId: activeGroup.id,
        editorGroups: editorGroups.map((group) =>
          group.id === activeGroup.id
            ? {
                ...group,
                documentIds: group.documentIds.includes(activeDocumentId) ? group.documentIds : [...group.documentIds, activeDocumentId],
                activeDocumentId,
              }
            : group,
        ),
      };
    }),
  setActiveEditorGroup: (activeEditorGroupId) =>
    set((state) => {
      const editorGroups = ensureEditorGroups(state.editorGroups);
      const activeGroup = editorGroups.find((group) => group.id === activeEditorGroupId);
      if (!activeGroup) return {};
      return { activeEditorGroupId, activeDocumentId: activeGroup.activeDocumentId ?? activeGroup.documentIds[0] ?? null };
    }),
  setActiveDocumentInGroup: (groupId, documentId) =>
    set((state) => {
      if (!state.openDocuments.some((document) => document.id === documentId)) return {};
      const editorGroups = ensureEditorGroups(state.editorGroups);
      if (!editorGroups.some((group) => group.id === groupId)) return {};
      return {
        activeDocumentId: documentId,
        activeEditorGroupId: groupId,
        editorGroups: editorGroups.map((group) =>
          group.id === groupId
            ? {
                ...group,
                documentIds: group.documentIds.includes(documentId) ? group.documentIds : [...group.documentIds, documentId],
                activeDocumentId: documentId,
              }
            : group,
        ),
      };
    }),
  setPendingEditorReveal: (pendingEditorReveal) => set({ pendingEditorReveal }),
  consumePendingEditorReveal: (documentId) => {
    const target = get().pendingEditorReveal;
    if (!target || target.documentId !== documentId) return null;
    set({ pendingEditorReveal: null });
    return target;
  },
  splitActiveEditor: () =>
    set((state) => {
      if (!state.activeDocumentId) return {};
      const editorGroups = ensureEditorGroups(state.editorGroups);
      const activeIndex = Math.max(0, editorGroups.findIndex((group) => group.id === state.activeEditorGroupId));
      const newGroup: EditorGroup = { id: createEditorGroupId(), documentIds: [state.activeDocumentId], activeDocumentId: state.activeDocumentId };
      return {
        editorGroups: [...editorGroups.slice(0, activeIndex + 1), newGroup, ...editorGroups.slice(activeIndex + 1)],
        activeEditorGroupId: newGroup.id,
      };
    }),
  splitDocumentInGroup: (groupId, documentId, side) =>
    set((state) => {
      if (!state.openDocuments.some((document) => document.id === documentId)) return {};
      const editorGroups = ensureEditorGroups(state.editorGroups);
      const sourceIndex = editorGroups.findIndex((group) => group.id === groupId && group.documentIds.includes(documentId));
      if (sourceIndex === -1) return {};
      const newGroup: EditorGroup = { id: createEditorGroupId(), documentIds: [documentId], activeDocumentId: documentId };
      const insertIndex = side === "left" ? sourceIndex : sourceIndex + 1;
      return {
        activeDocumentId: documentId,
        activeEditorGroupId: newGroup.id,
        editorGroups: [...editorGroups.slice(0, insertIndex), newGroup, ...editorGroups.slice(insertIndex)],
      };
    }),
  closeEditorGroup: (groupId) =>
    set((state) => {
      const editorGroups = ensureEditorGroups(state.editorGroups);
      if (editorGroups.length <= 1) return {};
      const closingIndex = editorGroups.findIndex((group) => group.id === groupId);
      if (closingIndex === -1) return {};
      const remainingGroups = editorGroups.filter((group) => group.id !== groupId);
      const referencedIds = new Set(remainingGroups.flatMap((group) => group.documentIds));
      const openDocuments = state.openDocuments.filter((document) => referencedIds.has(document.id));
      return normalizeEditorGroupState(
        remainingGroups,
        openDocuments,
        state.activeEditorGroupId === groupId ? remainingGroups[Math.min(closingIndex, remainingGroups.length - 1)]?.id : state.activeEditorGroupId,
      );
    }),
  closeDocumentInGroup: (groupId, documentId) =>
    set((state) => closeDocumentInGroupState(state, groupId, documentId)),
  closeOtherDocumentsInGroup: (groupId, documentId) =>
    set((state) => {
      const document = state.openDocuments.find((candidate) => candidate.id === documentId);
      if (!document) return {};
      const editorGroups = ensureEditorGroups(state.editorGroups);
      const targetGroup = editorGroups.find((group) => group.id === groupId);
      if (!targetGroup?.documentIds.includes(documentId)) return {};
      const nextGroups = editorGroups.map((group) => group.id === groupId ? { ...group, documentIds: [documentId], activeDocumentId: documentId } : group);
      const referencedIds = new Set(nextGroups.flatMap((group) => group.documentIds));
      const openDocuments = state.openDocuments.filter((candidate) => referencedIds.has(candidate.id));
      return {
        openDocuments,
        editorGroups: nextGroups,
        activeDocumentId: documentId,
        activeEditorGroupId: groupId,
      };
    }),
  closeDocumentsToRightInGroup: (groupId, documentId) =>
    set((state) => {
      const editorGroups = ensureEditorGroups(state.editorGroups);
      const targetGroup = editorGroups.find((group) => group.id === groupId);
      if (!targetGroup) return {};
      const documentIndex = targetGroup.documentIds.indexOf(documentId);
      if (documentIndex === -1 || documentIndex === targetGroup.documentIds.length - 1) return {};
      const nextDocumentIds = targetGroup.documentIds.slice(0, documentIndex + 1);
      const nextGroups = editorGroups.map((group) => group.id === groupId ? { ...group, documentIds: nextDocumentIds, activeDocumentId: documentId } : group);
      const referencedIds = new Set(nextGroups.flatMap((group) => group.documentIds));
      const openDocuments = state.openDocuments.filter((document) => referencedIds.has(document.id));
      return {
        openDocuments,
        editorGroups: nextGroups,
        activeDocumentId: documentId,
        activeEditorGroupId: groupId,
      };
    }),
  closeSavedDocumentsInGroup: (groupId) =>
    set((state) => {
      const editorGroups = ensureEditorGroups(state.editorGroups);
      const targetGroup = editorGroups.find((group) => group.id === groupId);
      if (!targetGroup) return {};
      const dirtyIds = new Set(state.openDocuments.filter((document) => document.is_dirty).map((document) => document.id));
      const nextDocumentIds = targetGroup.documentIds.filter((documentId) => dirtyIds.has(documentId));
      const nextGroups = editorGroups
        .map((group) => {
          if (group.id !== groupId) return group;
          const activeDocumentId = group.activeDocumentId && nextDocumentIds.includes(group.activeDocumentId)
            ? group.activeDocumentId
            : nextDocumentIds[0] ?? null;
          return { ...group, documentIds: nextDocumentIds, activeDocumentId };
        })
        .filter((group) => group.documentIds.length > 0);
      const referencedIds = new Set(nextGroups.flatMap((group) => group.documentIds));
      const openDocuments = state.openDocuments.filter((document) => referencedIds.has(document.id));
      return normalizeEditorGroupState(nextGroups, openDocuments, state.activeEditorGroupId);
    }),
  closeDocumentInActiveGroup: () =>
    set((state) => {
      if (!state.activeDocumentId) return {};
      return closeDocumentInGroupState(state, state.activeEditorGroupId, state.activeDocumentId);
    }),
  closeDocument: (id) =>
    set((state) => {
      const openDocuments = state.openDocuments.filter((document) => document.id !== id);
      if (openDocuments.length === state.openDocuments.length) return {};
      const editorGroups = ensureEditorGroups(state.editorGroups)
        .map((group) => {
          const documentIds = group.documentIds.filter((documentId) => documentId !== id);
          return {
            ...group,
            documentIds,
            activeDocumentId: group.activeDocumentId === id ? documentIds[0] ?? null : group.activeDocumentId,
          };
        })
        .filter((group) => group.documentIds.length > 0);
      return normalizeEditorGroupState(editorGroups, openDocuments, state.activeEditorGroupId);
    }),
  closeOtherDocuments: (id) =>
    set((state) => {
      const document = state.openDocuments.find((candidate) => candidate.id === id);
      if (!document) return {};
      const groupId = state.activeEditorGroupId || DEFAULT_EDITOR_GROUP_ID;
      return {
        activeDocumentId: id,
        activeEditorGroupId: groupId,
        editorGroups: [{ id: groupId, documentIds: [id], activeDocumentId: id }],
        openDocuments: [document],
      };
    }),
  closeAllDocuments: () => set({ activeDocumentId: null, activeEditorGroupId: DEFAULT_EDITOR_GROUP_ID, editorGroups: [createEmptyEditorGroup()], openDocuments: [], pendingEditorReveal: null }),
  selectNextDocument: () =>
    set((state) => {
      const editorGroups = ensureEditorGroups(state.editorGroups);
      const activeGroup = editorGroups.find((group) => group.id === state.activeEditorGroupId) ?? editorGroups[0];
      if (activeGroup.documentIds.length < 2) return {};
      const activeIndex = Math.max(0, activeGroup.documentIds.findIndex((documentId) => documentId === activeGroup.activeDocumentId));
      const activeDocumentId = activeGroup.documentIds[(activeIndex + 1) % activeGroup.documentIds.length];
      return {
        activeDocumentId,
        editorGroups: editorGroups.map((group) => group.id === activeGroup.id ? { ...group, activeDocumentId } : group),
      };
    }),
  selectPreviousDocument: () =>
    set((state) => {
      const editorGroups = ensureEditorGroups(state.editorGroups);
      const activeGroup = editorGroups.find((group) => group.id === state.activeEditorGroupId) ?? editorGroups[0];
      if (activeGroup.documentIds.length < 2) return {};
      const activeIndex = Math.max(0, activeGroup.documentIds.findIndex((documentId) => documentId === activeGroup.activeDocumentId));
      const activeDocumentId = activeGroup.documentIds[(activeIndex - 1 + activeGroup.documentIds.length) % activeGroup.documentIds.length];
      return {
        activeDocumentId,
        editorGroups: editorGroups.map((group) => group.id === activeGroup.id ? { ...group, activeDocumentId } : group),
      };
    }),
  setSearchResponse: (searchResponse) =>
    set((state) => searchResponsesEqual(state.searchResponse, searchResponse) ? {} : { searchResponse }),
  setTerminal: (terminal) => set((state) => {
    if (terminal) return upsertTerminalState(state, terminal, true);
    terminalOutputCoalescer.discard();
    return { terminal: null, terminalSessions: [], activeTerminalId: null, terminalOutputBuffers: {} };
  }),
  upsertTerminalSession: (terminal, makeActive = true) => set((state) => upsertTerminalState(state, terminal, makeActive)),
  setActiveTerminal: (terminalId) =>
    set((state) => {
      const terminal = state.terminalSessions.find((session) => session.id === terminalId);
      if (!terminal) return {};
      return { activeTerminalId: terminal.id, terminal };
    }),
  closeTerminalSession: (terminalId) =>
    set((state) => {
      const terminalSessions = state.terminalSessions.filter((session) => session.id !== terminalId);
      if (terminalSessions.length === state.terminalSessions.length) return {};
      // Drop any chunks still queued for the closed terminal so a pending frame
      // flush cannot recreate its buffer after removal.
      terminalOutputCoalescer.discard(terminalId);
      const terminalOutputBuffers = { ...state.terminalOutputBuffers };
      delete terminalOutputBuffers[terminalId];
      const activeStillExists = terminalSessions.find((session) => session.id === state.activeTerminalId) ?? null;
      const terminal = activeStillExists ?? terminalSessions[0] ?? null;
      return {
        terminal,
        terminalSessions,
        activeTerminalId: terminal?.id ?? null,
        terminalOutputBuffers,
      };
    }),
  closeAllTerminalSessions: () => {
    terminalOutputCoalescer.discard();
    set({ terminal: null, terminalSessions: [], activeTerminalId: null, terminalOutputBuffers: {} });
  },
  // High-volume PTY/tool output is coalesced: chunks are queued and committed once
  // per animation frame (see terminalOutput.ts) instead of doing O(buffer-size)
  // string work and waking every store subscriber on each individual byte.
  appendTerminalOutput: (terminalId, data) => terminalOutputCoalescer.enqueue(terminalId, data),
  commitTerminalOutput: (pending) =>
    set((state) => {
      let changed = false;
      const terminalOutputBuffers = { ...state.terminalOutputBuffers };
      for (const [terminalId, chunks] of pending) {
        const next = appendTerminalChunks(terminalOutputBuffers[terminalId], chunks);
        if (next !== terminalOutputBuffers[terminalId]) {
          terminalOutputBuffers[terminalId] = next;
          changed = true;
        }
      }
      return changed ? { terminalOutputBuffers } : {};
    }),
  clearTerminalOutput: (terminalId) =>
    set((state) => {
      if (!terminalId || !state.terminalOutputBuffers[terminalId]) return {};
      // Flush-proof the clear by dropping queued chunks for this terminal first.
      terminalOutputCoalescer.discard(terminalId);
      return { terminalOutputBuffers: { ...state.terminalOutputBuffers, [terminalId]: emptyTerminalBuffer() } };
    }),
  clearAllTerminalOutput: () => {
    terminalOutputCoalescer.discard();
    set({ terminalOutputBuffers: {} });
  },
  setGitStatus: (gitStatus) =>
    set((state) => gitStatusEqual(state.gitStatus, gitStatus) ? {} : { gitStatus }),
  setLanguageServers: (languageServers) =>
    set((state) => languageServersEqual(state.languageServers, languageServers) ? {} : { languageServers }),
  setLanguageServersLoading: (languageServersLoading) => set({ languageServersLoading }),
  setDiagnosticsForPath: (path, diagnostics) =>
    set((state) => ({
      diagnosticsByPath: {
        ...state.diagnosticsByPath,
        [normalizePath(path)]: diagnostics,
      },
    })),
  clearDiagnostics: () => set({ diagnosticsByPath: {} }),
  toggleDebugSourceBreakpoint: (path, line) =>
    set((state) => {
      const normalizedPath = normalizePath(path);
      const current = state.debugSourceBreakpointsByPath[normalizedPath] ?? [];
      const existing = current.some((breakpoint) => breakpoint.line === line);
      const nextBreakpoints = existing
        ? current.filter((breakpoint) => breakpoint.line !== line)
        : [...current, { path: normalizedPath, line, column: null, condition: null, log_message: null }].sort((left, right) => left.line - right.line);
      const debugSourceBreakpointsByPath = { ...state.debugSourceBreakpointsByPath };
      const debugResolvedBreakpointsByPath = { ...state.debugResolvedBreakpointsByPath };
      if (nextBreakpoints.length > 0) {
        debugSourceBreakpointsByPath[normalizedPath] = nextBreakpoints;
      } else {
        delete debugSourceBreakpointsByPath[normalizedPath];
      }
      delete debugResolvedBreakpointsByPath[normalizedPath];
      return { debugSourceBreakpointsByPath, debugResolvedBreakpointsByPath };
    }),
  setDebugResolvedBreakpoints: (update) =>
    set((state) => {
      const normalizedPath = normalizePath(update.path);
      const debugResolvedBreakpointsByPath = { ...state.debugResolvedBreakpointsByPath };
      if (update.breakpoints.length > 0) {
        debugResolvedBreakpointsByPath[normalizedPath] = update.breakpoints;
      } else {
        delete debugResolvedBreakpointsByPath[normalizedPath];
      }
      return { debugResolvedBreakpointsByPath };
    }),
  clearDebugBreakpoints: () => set({ debugSourceBreakpointsByPath: {}, debugResolvedBreakpointsByPath: {} }),
  setCommandPaletteOpen: (commandPaletteOpen) => set({ commandPaletteOpen }),
  openBottomPanel: (bottomPanelTab) => set({ bottomPanelOpen: true, bottomPanelTab }),
  toggleBottomPanel: (bottomPanelTab) =>
    set((state) => ({
      bottomPanelOpen: state.bottomPanelOpen && state.bottomPanelTab === bottomPanelTab ? false : true,
      bottomPanelTab,
    })),
  setBottomPanelOpen: (bottomPanelOpen) => set({ bottomPanelOpen }),
  setBottomPanelTab: (bottomPanelTab) => set({ bottomPanelTab }),
}));

// Wire the terminal-output coalescer to flush batched PTY chunks into the store.
terminalOutputCoalescer.setSink((pending) => useLuxStore.getState().commitTerminalOutput(pending));

export const selectActiveDocument = (state: LuxState) =>
  state.openDocuments.find((document) => document.id === state.activeDocumentId) ?? null;

export const selectActiveAiChatSession = (state: LuxState) =>
  state.aiChatSessions.find((session) => session.id === state.activeAiChatSessionId) ?? state.aiChatSessions[0] ?? null;

// Stable, memoized flatten: returns the same array reference until diagnosticsByPath
// changes, so diagnostics-bound UI no longer re-renders on unrelated store writes
// (AI token deltas, terminal output) the way `Object.values(...).flat()` forced.
export const selectDiagnostics = (state: LuxState) => flattenDiagnostics(state.diagnosticsByPath);

function createInitialAiChatState(workspaceRoot: string | null = null): Pick<LuxState, "aiChatSessions" | "activeAiChatSessionId"> {
  const session = createAiChatSession(workspaceRoot);
  return { aiChatSessions: [session], activeAiChatSessionId: session.id };
}

function createAiChatSession(workspaceRoot: string | null): AiChatSession {
  const now = Date.now();
  return {
    id: createAiChatSessionId(),
    title: "New chat",
    workspaceRoot,
    messages: [],
    contextCompaction: null,
    status: "idle",
    lastError: null,
    errorHistory: undefined,
    closedAt: null,
    createdAt: now,
    updatedAt: now,
  };
}

const AI_CHAT_ERROR_HISTORY_LIMIT = 8;

/** Append a failure to the session's retry-error history. Consecutive identical
 *  messages collapse into one entry with a bumped count (an auto-retry ladder
 *  hitting the same 403 eight times reads as one line, not eight); the list is
 *  capped so a marathon of distinct failures cannot grow unbounded. */
export function appendAiChatErrorHistory(
  history: AiChatErrorHistoryEntry[] | undefined,
  message: string,
): AiChatErrorHistoryEntry[] {
  const entries = history ?? [];
  const last = entries[entries.length - 1];
  if (last && last.message === message) {
    return [...entries.slice(0, -1), { ...last, count: last.count + 1, timestamp: Date.now() }];
  }
  return [...entries, { message, timestamp: Date.now(), count: 1 }].slice(-AI_CHAT_ERROR_HISTORY_LIMIT);
}

function normalizeAiChatSessionState(chatState: AiChatSessionState, fallbackRoot: string | null = null): Pick<LuxState, "aiChatSessions" | "activeAiChatSessionId"> {
  let sessions = chatState.sessions.length > 0 ? chatState.sessions.map((session) => normalizeAiChatSession(session)) : [createAiChatSession(fallbackRoot)];
  let activeAiChatSessionId = sessions.some((session) => session.id === chatState.activeSessionId) ? chatState.activeSessionId : sessions[0].id;
  const activeSession = sessions.find((session) => session.id === activeAiChatSessionId);
  if (!activeSession || activeSession.closedAt) {
    const openSession = sessions.find((session) => !session.closedAt);
    if (openSession) {
      activeAiChatSessionId = openSession.id;
    } else {
      // Scope a freshly-minted fallback to the open workspace, not the global (null)
      // root, so it stays a project chat instead of leaking across every project.
      const fallback = createAiChatSession(fallbackRoot);
      sessions = [fallback, ...sessions];
      activeAiChatSessionId = fallback.id;
    }
  }
  return { aiChatSessions: sessions, activeAiChatSessionId };
}

function mergeAiChatSessionState(state: LuxState, chatState: AiChatSessionState): Pick<LuxState, "aiChatSessions" | "activeAiChatSessionId"> {
  const incoming = normalizeAiChatSessionState(chatState, state.workspace?.root ?? null);
  const byId = new Map<string, AiChatSession>();

  for (const session of incoming.aiChatSessions) {
    byId.set(session.id, session);
  }

  for (const current of state.aiChatSessions.map((session) => normalizeAiChatSession(session, { preserveRuntimeStatus: true }))) {
    const persisted = byId.get(current.id);
    byId.set(current.id, persisted ? chooseHydratedAiChatSession(current, persisted) : current);
  }

  const aiChatSessions = sortAiChatSessionsForStore([...byId.values()]);
  const activeAiChatSessionId = resolveHydratedActiveSessionId(state.activeAiChatSessionId, incoming.activeAiChatSessionId, aiChatSessions);
  return { aiChatSessions, activeAiChatSessionId };
}

function chooseHydratedAiChatSession(current: AiChatSession, persisted: AiChatSession): AiChatSession {
  if (isAiChatSessionBusyStatus(current.status)) {
    if (persisted.messages.length > current.messages.length) {
      return { ...current, ...persisted, status: current.status, lastError: current.lastError, errorHistory: current.errorHistory };
    }
    return current;
  }
  if (current.updatedAt > persisted.updatedAt) return current;
  if (current.messages.length > persisted.messages.length) return current;
  if (current.updatedAt === persisted.updatedAt && sessionSegmentScore(current) >= sessionSegmentScore(persisted)) return current;
  return persisted;
}

function normalizeAiChatSession(session: AiChatSession, options: { preserveRuntimeStatus?: boolean } = {}): AiChatSession {
  const status = options.preserveRuntimeStatus && isAiChatSessionBusyStatus(session.status)
    ? session.status
    : session.status === "error"
      ? "error"
      : "idle";
  return {
    ...session,
    title: normalizeChatTitle(session.title || titleFromMessages(session.messages) || "New chat"),
    status,
    lastError: session.lastError ?? null,
    errorHistory: Array.isArray(session.errorHistory) ? session.errorHistory : undefined,
    closedAt: Number.isFinite(session.closedAt) ? session.closedAt : null,
    messages: Array.isArray(session.messages) ? session.messages.map(normalizeAiChatMessage) : [],
    contextCompaction: session.contextCompaction ?? null,
    createdAt: Number.isFinite(session.createdAt) ? session.createdAt : Date.now(),
    updatedAt: Number.isFinite(session.updatedAt) ? session.updatedAt : Date.now(),
  };
}

function normalizeAiChatMessage(message: AiChatMessage): AiChatMessage {
  const normalized = normalizeAiMessageReasoning(message);
  return {
    ...normalized,
    segments: normalizeAiMessageSegments(normalized),
  };
}

function normalizeAiMessageSegments(message: AiChatMessage): AiMessageSegment[] | undefined {
  if (Array.isArray(message.segments) && message.segments.length > 0) {
    return message.segments.map((segment, index) => {
      if (segment.kind === "tool") return { ...segment, id: segment.id || segment.toolCall.id || `tool-${index}` };
      return { ...segment, id: segment.id || `${segment.kind}-${index}` };
    });
  }

  if (message.role !== "assistant") return message.segments;
  const segments: AiMessageSegment[] = [];
  if (message.reasoning?.trim()) segments.push({ kind: "reasoning", id: `${message.id}-reasoning`, text: message.reasoning });
  if (message.toolCalls?.length) {
    for (const [index, toolCall] of message.toolCalls.entries()) {
      const id = toolCall.id || `${message.id}-tool-${index}`;
      segments.push({ kind: "tool", id, toolCall: { ...toolCall, id } });
    }
  }
  if (message.content.trim()) segments.push({ kind: "text", id: `${message.id}-text`, text: message.content });
  return segments.length > 0 ? segments : message.segments;
}

function sessionSegmentScore(session: AiChatSession) {
  return session.messages.reduce((score, message) => score + (message.segments?.length ?? 0), 0);
}

/** True when every key in `patch` already holds an identical (Object.is) value on
 *  the message. Conservative by design: it only reports a no-op when nothing would
 *  change, so it can never drop a real streaming update (new array/string refs
 *  always differ), but it does skip the redundant patches that streaming emits. */
function isNoOpMessagePatch(message: AiChatMessage, patch: Partial<AiChatMessage>): boolean {
  for (const key of Object.keys(patch) as Array<keyof AiChatMessage>) {
    if (!Object.is(message[key], patch[key])) return false;
  }
  return true;
}

function sortAiChatSessionsForStore(sessions: AiChatSession[]) {
  return sessions.sort((left, right) => right.updatedAt - left.updatedAt || right.createdAt - left.createdAt);
}

function resolveHydratedActiveSessionId(currentActiveId: string, persistedActiveId: string, sessions: AiChatSession[]) {
  const current = sessions.find((session) => session.id === currentActiveId);
  if (current && !current.closedAt) return current.id;
  const persisted = sessions.find((session) => session.id === persistedActiveId);
  if (persisted && !persisted.closedAt) return persisted.id;
  return sessions.find((session) => !session.closedAt)?.id ?? sessions[0]?.id ?? "";
}

function isEmptyAiChatSession(session: AiChatSession) {
  return session.messages.length === 0 && session.status === "idle" && !session.lastError;
}

function nextChatSessionTitle(session: AiChatSession, message: AiChatMessage) {
  if (session.title !== "New chat" || message.role !== "user") return session.title;
  return normalizeChatTitle(message.content);
}

function titleFromMessages(messages: AiChatMessage[]) {
  const firstUserMessage = messages.find((message) => message.role === "user" && message.content.trim());
  return firstUserMessage ? normalizeChatTitle(firstUserMessage.content) : null;
}

function normalizeChatTitle(value: string) {
  const normalized = value.replace(/\s+/g, " ").trim();
  if (!normalized) return "New chat";
  return normalized.length > 42 ? `${normalized.slice(0, 42).trimEnd()}...` : normalized;
}

function ensureEditorGroups(editorGroups: EditorGroup[]) {
  return editorGroups.length > 0 ? editorGroups : [createEmptyEditorGroup()];
}

function normalizeEditorGroupState(editorGroups: EditorGroup[], openDocuments: DocumentSnapshot[], preferredGroupId: string) {
  const openDocumentIds = new Set(openDocuments.map((document) => document.id));
  const groups = ensureEditorGroups(editorGroups)
    .map((group) => {
      const documentIds = group.documentIds.filter((documentId) => openDocumentIds.has(documentId));
      const activeDocumentId = group.activeDocumentId && documentIds.includes(group.activeDocumentId)
        ? group.activeDocumentId
        : documentIds[0] ?? null;
      return { ...group, documentIds, activeDocumentId };
    })
    .filter((group) => group.documentIds.length > 0);

  if (groups.length === 0) {
    return {
      activeDocumentId: null,
      activeEditorGroupId: DEFAULT_EDITOR_GROUP_ID,
      editorGroups: [createEmptyEditorGroup()],
      openDocuments,
    };
  }

  const activeGroup = groups.find((group) => group.id === preferredGroupId) ?? groups[0];
  return {
    activeDocumentId: activeGroup.activeDocumentId,
    activeEditorGroupId: activeGroup.id,
    editorGroups: groups,
    openDocuments,
  };
}

function closeDocumentInGroupState(state: LuxState, groupId: string, documentId: string) {
  const editorGroups = ensureEditorGroups(state.editorGroups);
  const targetGroup = editorGroups.find((group) => group.id === groupId);
  if (!targetGroup || !targetGroup.documentIds.includes(documentId)) return {};

  const nextGroups = editorGroups.map((group) => {
    if (group.id !== groupId) return group;
    const documentIds = group.documentIds.filter((candidate) => candidate !== documentId);
    return {
      ...group,
      documentIds,
      activeDocumentId: group.activeDocumentId === documentId ? documentIds[0] ?? null : group.activeDocumentId,
    };
  }).filter((group) => group.documentIds.length > 0);

  const referencedIds = new Set(nextGroups.flatMap((group) => group.documentIds));
  const openDocuments = state.openDocuments.filter((document) => referencedIds.has(document.id));
  return normalizeEditorGroupState(nextGroups, openDocuments, state.activeEditorGroupId === groupId ? groupId : state.activeEditorGroupId);
}

function normalizePathList(paths: Iterable<string>) {
  return Array.from(new Set(Array.from(paths, normalizePath)));
}

function upsertTerminalState(state: LuxState, terminal: TerminalSessionInfo, makeActive: boolean) {
  const exists = state.terminalSessions.some((session) => session.id === terminal.id);
  const terminalSessions = exists
    ? state.terminalSessions.map((session) => session.id === terminal.id ? terminal : session)
    : [...state.terminalSessions, terminal];
  const activeTerminalId = makeActive || !state.activeTerminalId ? terminal.id : state.activeTerminalId;
  const activeTerminal = terminalSessions.find((session) => session.id === activeTerminalId) ?? terminalSessions[0] ?? null;
  return {
    terminal: activeTerminal,
    terminalSessions,
    activeTerminalId: activeTerminal?.id ?? null,
  };
}

