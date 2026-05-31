import { useEffect, useRef, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import type { PanelImperativeHandle } from "react-resizable-panels";
import { AgentWorkspace } from "./components/AgentWorkspace";
import { AiChatPanel } from "./components/AiChatPanel";
import { BottomPanel } from "./components/BottomPanel";
import { CommandPalette } from "./components/CommandPalette";
import { useEditorCloseGuard } from "./components/EditorCloseGuard";
import { EditorArea } from "./components/EditorArea";
import { SettingsDialog } from "./components/SettingsDialog";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { TitleBar } from "./components/TitleBar";
import { WelcomeScreen } from "./components/WelcomeScreen";
import { AI_PREFERENCES_KEY, normalizeAiPreferences } from "./lib/aiPreferences";
import { closedDocumentIdsForAllDocuments, closedDocumentIdsForDocumentInGroup } from "./lib/editorCloseTargets";
import { normalizeLocale, UI_LOCALE_KEY } from "./lib/i18n";
import { resetEditorFontZoom, toggleEditorMinimap, toggleEditorWordWrap, zoomEditorFontIn, zoomEditorFontOut } from "./lib/editorPreferenceCommands";
import { EDITOR_PREFERENCES_KEY, normalizeEditorPreferences } from "./lib/editorPreferences";
import { buildFileTreeDirectories, normalizePath } from "./lib/fileTree";
import { createKeybindingDispatcher, KEYBINDINGS_SETTINGS_KEY } from "./lib/keybindings";
import { useLuxStore, type Activity } from "./lib/store";
import { luxCommands, subscribeLuxEvents } from "./lib/tauri";
import { pickAndOpenWorkspace } from "./lib/workspaceActions";
import type { RecentWorkspace, WorkspaceInfo } from "./lib/types";

export function App() {
  const setWorkspace = useLuxStore((state) => state.setWorkspace);
  const setFileEntries = useLuxStore((state) => state.setFileEntries);
  const fileTreeDirectories = useLuxStore((state) => state.fileTreeDirectories);
  const setFileTreeDirectories = useLuxStore((state) => state.setFileTreeDirectories);
  const setFileTreeLoading = useLuxStore((state) => state.setFileTreeLoading);
  const setFileTreeError = useLuxStore((state) => state.setFileTreeError);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);
  const updateOpenDocuments = useLuxStore((state) => state.updateOpenDocuments);
  const applyDocumentEdits = useLuxStore((state) => state.applyDocumentEdits);
  const closeDocument = useLuxStore((state) => state.closeDocument);
  const setGitStatus = useLuxStore((state) => state.setGitStatus);
  const setLanguageServers = useLuxStore((state) => state.setLanguageServers);
  const setLanguageServersLoading = useLuxStore((state) => state.setLanguageServersLoading);
  const setDiagnosticsForPath = useLuxStore((state) => state.setDiagnosticsForPath);
  const clearDiagnostics = useLuxStore((state) => state.clearDiagnostics);
  const setEditorPreferences = useLuxStore((state) => state.setEditorPreferences);
  const setLocale = useLuxStore((state) => state.setLocale);
  const locale = useLuxStore((state) => state.locale);
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const setAiPreferences = useLuxStore((state) => state.setAiPreferences);
  const setAiIndex = useLuxStore((state) => state.setAiIndex);
  const setKeybindingProfile = useLuxStore((state) => state.setKeybindingProfile);
  const keybindingProfile = useLuxStore((state) => state.keybindingProfile);
  const workspace = useLuxStore((state) => state.workspace);
  const workspaceMode = useLuxStore((state) => state.workspaceMode);
  const bottomPanelOpen = useLuxStore((state) => state.bottomPanelOpen);
  const sidebarVisible = useLuxStore((state) => state.sidebarVisible);
  const aiChatOpen = useLuxStore((state) => state.aiChatOpen);
  const setCommandPaletteOpen = useLuxStore((state) => state.setCommandPaletteOpen);
  const setSettingsOpen = useLuxStore((state) => state.setSettingsOpen);
  const setActiveActivity = useLuxStore((state) => state.setActiveActivity);
  const setSidebarVisible = useLuxStore((state) => state.setSidebarVisible);
  const toggleSidebar = useLuxStore((state) => state.toggleSidebar);
  const toggleAiChat = useLuxStore((state) => state.toggleAiChat);
  const openBottomPanel = useLuxStore((state) => state.openBottomPanel);
  const closeDocumentInActiveGroup = useLuxStore((state) => state.closeDocumentInActiveGroup);
  const splitActiveEditor = useLuxStore((state) => state.splitActiveEditor);
  const selectNextDocument = useLuxStore((state) => state.selectNextDocument);
  const selectPreviousDocument = useLuxStore((state) => state.selectPreviousDocument);
  const activeDocumentId = useLuxStore((state) => state.activeDocumentId);
  const activeEditorGroupId = useLuxStore((state) => state.activeEditorGroupId);
  const editorGroups = useLuxStore((state) => state.editorGroups);
  const openDocuments = useLuxStore((state) => state.openDocuments);
  const hasOpenDocuments = openDocuments.length > 0;
  const { requestCloseDocuments } = useEditorCloseGuard();
  const [recentWorkspaces, setRecentWorkspaces] = useState<RecentWorkspace[]>([]);
  const [bottomPanelMaximized, setBottomPanelMaximized] = useState(false);
  const keybindingDispatcherRef = useRef(createKeybindingDispatcher(keybindingProfile));
  const bottomPanelRef = useRef<PanelImperativeHandle | null>(null);

  const openProject = () => {
    requestCloseDocuments(closedDocumentIdsForAllDocuments(openDocuments), () => void pickAndOpenWorkspace().then((workspace) => {
      if (workspace) {
        setWorkspace(workspace);
        refreshRecentWorkspaces(setRecentWorkspaces);
      }
    }).catch(() => undefined), { title: "Save changes before opening another folder?" });
  };

  const newUntitledFile = () => {
    void luxCommands.editorNewFile().then(upsertDocument).catch(() => undefined);
  };

  const toggleBottomPanelMaximized = () => {
    setBottomPanelMaximized((maximized) => {
      const nextMaximized = !maximized;
      bottomPanelRef.current?.resize(nextMaximized ? "70%" : "210px");
      return nextMaximized;
    });
  };

  const openRecentWorkspace = (root: string) => {
    requestCloseDocuments(closedDocumentIdsForAllDocuments(openDocuments), () => void luxCommands.workspaceOpen(root).then((workspace) => {
      setWorkspace(workspace);
      refreshRecentWorkspaces(setRecentWorkspaces);
    }).catch(() => {
      void luxCommands.recentWorkspaceForget(root).then(setRecentWorkspaces).catch(() => undefined);
    }), { title: "Save changes before switching folders?" });
  };

  const forgetRecentWorkspace = (root: string) => {
    void luxCommands.recentWorkspaceForget(root).then(setRecentWorkspaces).catch(() => undefined);
  };

  useEffect(() => {
    refreshRecentWorkspaces(setRecentWorkspaces);
  }, []);

  useEffect(() => {
    void luxCommands.settingsGet("user", AI_PREFERENCES_KEY)
      .then((setting) => {
        if (setting) setAiPreferences(normalizeAiPreferences(setting.value));
      })
      .catch(() => undefined);

    void luxCommands.settingsGet("user", EDITOR_PREFERENCES_KEY)
      .then((setting) => {
        if (setting) setEditorPreferences(normalizeEditorPreferences(setting.value));
      })
      .catch(() => undefined);

    void luxCommands.settingsGet("user", UI_LOCALE_KEY)
      .then((setting) => {
        if (setting) setLocale(normalizeLocale(setting.value));
      })
      .catch(() => undefined);
  }, [setAiPreferences, setEditorPreferences, setLocale]);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  useEffect(() => {
    if (!workspace || !aiPreferences.projectIndexingEnabled) {
      setAiIndex({ status: aiPreferences.projectIndexingEnabled ? "idle" : "disabled", progress: 0, indexedFiles: 0, totalFiles: 0, updatedAt: null });
      return;
    }

    const files = Object.values(fileTreeDirectories)
      .flat()
      .filter((entry) => entry.kind === "file")
      .filter((entry) => aiPreferences.includeImages || !isImagePath(entry.path))
      .slice(0, aiPreferences.maxIndexedFiles);

    if (files.length === 0) {
      setAiIndex({ status: "idle", progress: 0, indexedFiles: 0, totalFiles: 0, updatedAt: null });
      return;
    }

    let cancelled = false;
    let indexedFiles = 0;
    setAiIndex({ status: "indexing", progress: 0, indexedFiles: 0, totalFiles: files.length, updatedAt: null });

    const tick = () => {
      if (cancelled) return;
      indexedFiles = Math.min(files.length, indexedFiles + Math.max(1, Math.ceil(files.length / 18)));
      setAiIndex({
        status: indexedFiles >= files.length ? "ready" : "indexing",
        progress: Math.round((indexedFiles / files.length) * 100),
        indexedFiles,
        totalFiles: files.length,
        updatedAt: indexedFiles >= files.length ? new Date().toISOString() : null,
      });
      if (indexedFiles < files.length) window.setTimeout(tick, 90);
    };

    const timer = window.setTimeout(tick, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [aiPreferences.includeImages, aiPreferences.maxIndexedFiles, aiPreferences.projectIndexingEnabled, fileTreeDirectories, setAiIndex, workspace]);

  useEffect(() => {
    void luxCommands.keybindingsGet()
      .then((profile) => {
        setKeybindingProfile(profile);
        keybindingDispatcherRef.current.setProfile(profile);
      })
      .catch(() => undefined);
  }, [setKeybindingProfile]);

  useEffect(() => {
    keybindingDispatcherRef.current.setProfile(keybindingProfile);
  }, [keybindingProfile]);

  useEffect(() => {
    if (!workspace) return;
    setBottomPanelMaximized(false);
    let cancelled = false;
    clearDiagnostics();
    setFileTreeLoading(true);
    setFileTreeError(null);

    void refreshWorkspaceFileTree(workspace, true)
      .catch(() => undefined)
      .finally(() => {
        if (!cancelled) setFileTreeLoading(false);
      });
    luxCommands.gitStatus().then(setGitStatus).catch(() => setGitStatus(null));
    setLanguageServersLoading(true);
    luxCommands.lspServers()
      .then((servers) => {
        if (!cancelled) setLanguageServers(servers);
      })
      .catch(() => {
        if (!cancelled) setLanguageServers([]);
      })
      .finally(() => {
        if (!cancelled) setLanguageServersLoading(false);
      });
    luxCommands.diagnosticsSnapshot()
      .then((diagnostics) => {
        if (cancelled) return;
        const diagnosticsByPath = new Map<string, typeof diagnostics>();
        for (const diagnostic of diagnostics) {
          const existing = diagnosticsByPath.get(diagnostic.path) ?? [];
          diagnosticsByPath.set(diagnostic.path, [...existing, diagnostic]);
        }
        for (const [path, pathDiagnostics] of diagnosticsByPath) {
          setDiagnosticsForPath(path, pathDiagnostics);
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [clearDiagnostics, setDiagnosticsForPath, setFileEntries, setFileTreeDirectories, setFileTreeError, setFileTreeLoading, setGitStatus, setLanguageServers, setLanguageServersLoading, workspace]);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    let fsRefreshTimer: number | undefined;
    let fsRefreshInFlight = false;
    let fsRefreshQueued = false;
    const pendingFsRefreshRoots = new Map<string, WorkspaceInfo>();

    const refreshAfterFsChange = () => {
      const currentWorkspace = useLuxStore.getState().workspace;
      if (!currentWorkspace) {
        pendingFsRefreshRoots.clear();
        return;
      }
      if (fsRefreshInFlight) {
        fsRefreshQueued = true;
        return;
      }
      const rootsToRefresh = [...pendingFsRefreshRoots.values()];
      pendingFsRefreshRoots.clear();
      if (rootsToRefresh.length === 0) return;

      fsRefreshInFlight = true;
      void Promise.all([
        refreshWorkspaceFileTree(currentWorkspace, false, rootsToRefresh),
        luxCommands.gitStatus().then(setGitStatus).catch(() => setGitStatus(null)),
      ]).finally(() => {
        fsRefreshInFlight = false;
        if (fsRefreshQueued || pendingFsRefreshRoots.size > 0) {
          fsRefreshQueued = false;
          scheduleFsRefresh();
        }
      });
    };

    const scheduleFsRefresh = () => {
      if (fsRefreshTimer !== undefined) window.clearTimeout(fsRefreshTimer);
      fsRefreshTimer = window.setTimeout(() => {
        fsRefreshTimer = undefined;
        refreshAfterFsChange();
      }, 180);
    };

    void subscribeLuxEvents((event) => {
      if (event.type === "workspaceChanged") setWorkspace(event.workspace);
      if (event.type === "fsChanged") {
        const touchedRoots = workspaceRootsForChangedPath(event.path);
        for (const root of touchedRoots) pendingFsRefreshRoots.set(normalizePath(root.root), root);
        if (touchedRoots.length > 0) scheduleFsRefresh();
      }
      if (event.type === "editorDocumentClosed") closeDocument(event.document.id);
      if (event.type === "editorDocumentChanged") upsertDocument(event.document);
      if (event.type === "editorDocumentsChanged") updateOpenDocuments(event.documents);
      if (event.type === "editorDocumentEdited") applyDocumentEdits(event.document.id, [], event.document);
      if (event.type === "editorDiagnosticsChanged") setDiagnosticsForPath(event.path, event.diagnostics);
      if (event.type === "gitStatusChanged") setGitStatus(event.status);
      if (event.type === "settingsChanged" && event.key === KEYBINDINGS_SETTINGS_KEY) {
        void luxCommands.keybindingsGet().then(setKeybindingProfile).catch(() => undefined);
      }
    }).then((unlisten) => {
      dispose = unlisten;
    });
    return () => {
      if (fsRefreshTimer !== undefined) window.clearTimeout(fsRefreshTimer);
      dispose?.();
    };
  }, [applyDocumentEdits, closeDocument, setDiagnosticsForPath, setGitStatus, setKeybindingProfile, setWorkspace, updateOpenDocuments, upsertDocument]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const match = keybindingDispatcherRef.current.handleKeyDown(event, {
        dirtyEditors: openDocuments.some((document) => document.is_dirty),
        editor: Boolean(activeDocumentId),
        workspace: Boolean(workspace),
      });
      if (match.preventDefault) event.preventDefault();
      if (!match.command) return;
      runKeybindingCommand(match.command, {
        activeDocumentId,
        activeEditorGroupId,
        closeDocumentInActiveGroup,
        editorGroups,
        newUntitledFile,
        openBottomPanel,
        openProject,
        openDocuments,
        requestCloseDocuments,
        selectNextDocument,
        selectPreviousDocument,
        setActiveActivity,
        setCommandPaletteOpen,
        setSettingsOpen,
        setSidebarVisible,
        splitActiveEditor,
        toggleAiChat,
        toggleSidebar,
        upsertDocument,
      });
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activeDocumentId, activeEditorGroupId, closeDocumentInActiveGroup, editorGroups, openBottomPanel, openDocuments, requestCloseDocuments, selectNextDocument, selectPreviousDocument, setActiveActivity, setCommandPaletteOpen, setSettingsOpen, setSidebarVisible, splitActiveEditor, toggleAiChat, toggleSidebar, upsertDocument, workspace]);

  if (!workspace) {
    if (workspaceMode === "agent") {
      return (
        <div className="app-shell no-project-shell agent-shell">
          <TitleBar />
          <AgentWorkspace
            onOpenProject={openProject}
          />
          <CommandPalette />
          <SettingsDialog />
        </div>
      );
    }

    return (
      <div className="app-shell no-project-shell">
        <TitleBar />
        <div className="no-project-main" data-panel-maximized={bottomPanelMaximized}>
          <div className="no-project-workbench">
            <Group orientation="horizontal" className="main-panels">
              <Panel minSize="360px">
                {hasOpenDocuments ? (
                  <EditorArea />
                ) : (
                  <WelcomeScreen
                    onForgetRecentWorkspace={forgetRecentWorkspace}
                    onOpenProject={openProject}
                    onOpenRecentWorkspace={openRecentWorkspace}
                    recentWorkspaces={recentWorkspaces}
                  />
                )}
              </Panel>
              {aiChatOpen && (
                <>
                  <Separator className="resize-handle editor-group-separator" />
                  <Panel defaultSize="32%" minSize="300px" maxSize="48%">
                    <AiChatPanel />
                  </Panel>
                </>
              )}
            </Group>
          </div>
          {bottomPanelOpen && <BottomPanel isMaximized={bottomPanelMaximized} onToggleMaximized={toggleBottomPanelMaximized} />}
        </div>
        <StatusBar />
        <CommandPalette />
        <SettingsDialog />
      </div>
    );
  }

  if (workspaceMode === "agent") {
    return (
      <div className="app-shell agent-shell">
        <TitleBar />
        <AgentWorkspace
          onOpenProject={openProject}
        />
        <CommandPalette />
        <SettingsDialog />
      </div>
    );
  }

  return (
    <div className="app-shell">
      <TitleBar />
      <div className="workbench">
        <Group orientation="horizontal" className="main-panels">
          {sidebarVisible && <ExplorerPanelSlot />}
          <Panel minSize="360px">
            <Group orientation="vertical">
              <Panel minSize="360px">
                <EditorArea />
              </Panel>
              {bottomPanelOpen && (
                <>
                  <Separator className="resize-handle horizontal" />
                  <Panel defaultSize="210px" minSize="150px" maxSize="70%" panelRef={bottomPanelRef}>
                    <BottomPanel isMaximized={bottomPanelMaximized} onToggleMaximized={toggleBottomPanelMaximized} />
                  </Panel>
                </>
              )}
            </Group>
          </Panel>
          {aiChatOpen && (
            <>
              <Separator className="resize-handle editor-group-separator" />
              <Panel defaultSize="32%" minSize="300px" maxSize="48%">
                <AiChatPanel />
              </Panel>
            </>
          )}
        </Group>
      </div>
      <StatusBar />
      <CommandPalette />
      <SettingsDialog />
    </div>
  );
}

function refreshRecentWorkspaces(setRecentWorkspaces: (workspaces: RecentWorkspace[]) => void) {
  void luxCommands.recentWorkspaces().then(setRecentWorkspaces).catch(() => setRecentWorkspaces([]));
}

type KeybindingCommandContext = {
  activeDocumentId: string | null;
  activeEditorGroupId: string;
  closeDocumentInActiveGroup: () => void;
  editorGroups: ReturnType<typeof useLuxStore.getState>["editorGroups"];
  newUntitledFile: () => void;
  openBottomPanel: ReturnType<typeof useLuxStore.getState>["openBottomPanel"];
  openProject: () => void;
  openDocuments: ReturnType<typeof useLuxStore.getState>["openDocuments"];
  requestCloseDocuments: ReturnType<typeof useEditorCloseGuard>["requestCloseDocuments"];
  selectNextDocument: () => void;
  selectPreviousDocument: () => void;
  setActiveActivity: ReturnType<typeof useLuxStore.getState>["setActiveActivity"];
  setCommandPaletteOpen: (open: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  setSidebarVisible: (visible: boolean) => void;
  splitActiveEditor: () => void;
  toggleAiChat: ReturnType<typeof useLuxStore.getState>["toggleAiChat"];
  toggleSidebar: ReturnType<typeof useLuxStore.getState>["toggleSidebar"];
  upsertDocument: ReturnType<typeof useLuxStore.getState>["upsertDocument"];
};

function runKeybindingCommand(command: string, context: KeybindingCommandContext) {
  switch (command) {
    case "workbench.action.showCommands":
    case "workbench.action.quickOpen":
      context.setCommandPaletteOpen(true);
      break;
    case "workbench.action.openSettings":
      context.setSettingsOpen(true);
      break;
    case "workbench.action.openFolder":
      context.openProject();
      break;
    case "workbench.action.files.newUntitledFile":
      context.newUntitledFile();
      break;
    case "workbench.action.files.save":
      if (context.activeDocumentId) void luxCommands.editorSaveFile(context.activeDocumentId).then(context.upsertDocument).catch(() => undefined);
      break;
    case "workbench.action.files.saveAs":
      if (context.activeDocumentId) void luxCommands.editorSaveFileAs(context.activeDocumentId).then(context.upsertDocument).catch(() => undefined);
      break;
    case "workbench.action.files.saveAll":
      for (const document of context.openDocuments) {
        if (document.is_dirty) void luxCommands.editorSaveFile(document.id).then(context.upsertDocument).catch(() => undefined);
      }
      break;
    case "editor.action.toggleWordWrap":
      toggleEditorWordWrap();
      break;
    case "editor.action.toggleMinimap":
      toggleEditorMinimap();
      break;
    case "editor.action.fontZoomIn":
      zoomEditorFontIn();
      break;
    case "editor.action.fontZoomOut":
      zoomEditorFontOut();
      break;
    case "editor.action.fontZoomReset":
      resetEditorFontZoom();
      break;
    case "workbench.action.toggleSidebar":
      context.toggleSidebar();
      break;
    case "workbench.view.explorer":
      showActivity("explorer", context.setActiveActivity, context.setSidebarVisible);
      break;
    case "workbench.view.search":
      showActivity("search", context.setActiveActivity, context.setSidebarVisible);
      break;
    case "workbench.view.scm":
      showActivity("git", context.setActiveActivity, context.setSidebarVisible);
      break;
    case "workbench.view.debug":
      showActivity("runDebug", context.setActiveActivity, context.setSidebarVisible);
      break;
    case "workbench.view.extensions":
      showActivity("extensions", context.setActiveActivity, context.setSidebarVisible);
      break;
    case "workbench.action.chat.toggle":
      context.toggleAiChat();
      break;
    case "workbench.action.terminal.toggleTerminal":
      context.openBottomPanel("terminal");
      break;
    case "workbench.action.closeActiveEditor":
      if (context.activeDocumentId) {
        context.requestCloseDocuments(
          closedDocumentIdsForDocumentInGroup(context.openDocuments, context.editorGroups, context.activeEditorGroupId, context.activeDocumentId),
          context.closeDocumentInActiveGroup,
        );
      }
      break;
    case "workbench.action.splitEditorRight":
      if (context.activeDocumentId) context.splitActiveEditor();
      break;
    case "workbench.action.nextEditor":
      context.selectNextDocument();
      break;
    case "workbench.action.previousEditor":
      context.selectPreviousDocument();
      break;
  }
}

function showActivity(activity: Activity, setActiveActivity: (activity: Activity) => void, setSidebarVisible: (visible: boolean) => void) {
  setActiveActivity(activity);
  setSidebarVisible(true);
}

async function refreshWorkspaceFileTree(workspace: WorkspaceInfo, clearTreeOnError: boolean, rootsToRefresh?: WorkspaceInfo[]) {
  const roots = workspaceRootsForRefresh(workspace, rootsToRefresh);
  try {
    const refreshedRoots = await Promise.all(
      roots.map(async (root) => [root, await luxCommands.fsReadTree(root.root)] as const),
    );
    const store = useLuxStore.getState();
    const currentWorkspace = store.workspace;
    if (!currentWorkspace || normalizePath(currentWorkspace.root) !== normalizePath(workspace.root)) return;
    const directories = mergeRefreshedWorkspaceRoots(store.fileTreeDirectories, refreshedRoots);
    store.setFileTreeDirectories(directories);
    store.setFileEntries(directories[normalizePath(currentWorkspace.root)] ?? []);
    store.setFileTreeError(null);
  } catch (error) {
    const store = useLuxStore.getState();
    const currentWorkspace = store.workspace;
    if (!currentWorkspace || normalizePath(currentWorkspace.root) !== normalizePath(workspace.root)) return;
    if (!clearTreeOnError) {
      store.setFileTreeError(readErrorMessage(error));
      return;
    }
    store.setFileEntries([]);
    store.setFileTreeDirectories({});
    store.setFileTreeError(readErrorMessage(error));
    throw error;
  }
}

function workspaceRootsForRefresh(workspace: WorkspaceInfo, rootsToRefresh?: WorkspaceInfo[]) {
  const store = useLuxStore.getState();
  const roots = rootsToRefresh && rootsToRefresh.length > 0
    ? rootsToRefresh
    : store.workspaceFolders.length > 0
      ? store.workspaceFolders
      : [workspace];
  const uniqueRoots = new Map<string, WorkspaceInfo>();
  for (const root of roots) uniqueRoots.set(normalizePath(root.root), root);
  if (!rootsToRefresh || rootsToRefresh.length === 0) uniqueRoots.set(normalizePath(workspace.root), workspace);
  return [...uniqueRoots.values()];
}

function mergeRefreshedWorkspaceRoots(
  currentDirectories: ReturnType<typeof useLuxStore.getState>["fileTreeDirectories"],
  refreshedRoots: Array<readonly [WorkspaceInfo, Awaited<ReturnType<typeof luxCommands.fsReadTree>>]>,
) {
  const directories = { ...currentDirectories };
  for (const [workspace] of refreshedRoots) {
    const rootKey = normalizePath(workspace.root);
    for (const key of Object.keys(directories)) {
      if (key === rootKey || key.startsWith(`${rootKey}/`)) delete directories[key];
    }
  }
  for (const [workspace, entries] of refreshedRoots) {
    Object.assign(directories, buildFileTreeDirectories(workspace.root, entries));
  }
  return directories;
}

function workspaceRootsForChangedPath(path: string) {
  const store = useLuxStore.getState();
  const roots = store.workspaceFolders.length > 0 ? store.workspaceFolders : store.workspace ? [store.workspace] : [];
  return roots.filter((workspace) => pathIsInsideRoot(workspace.root, path));
}

function pathIsInsideRoot(root: string, path: string) {
  const normalizedRoot = normalizePath(root);
  const normalizedPath = normalizePath(path);
  return normalizedPath === normalizedRoot || normalizedPath.startsWith(`${normalizedRoot}/`);
}

function isImagePath(path: string) {
  return /\.(avif|gif|ico|jpeg|jpg|png|svg|webp)$/i.test(path);
}

function readErrorMessage(error: unknown) {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Failed to load project file tree.";
}

function ExplorerPanelSlot() {
  return (
    <>
      <Panel defaultSize="288px" minSize="220px" maxSize="430px">
        <Sidebar side="left" />
      </Panel>
      <Separator className="resize-handle" />
    </>
  );
}
