import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import type { PanelImperativeHandle } from "react-resizable-panels";
import { AgentWorkspace } from "./components/AgentWorkspace";
import { useEditorCloseGuard } from "./components/EditorCloseGuard";
import { LazyAiChatPanel } from "./components/LazyAiChatPanel";
import { ProjectLoadingStatus } from "./components/ProjectLoadingStatus";
import { StatusBar } from "./components/StatusBar";
import { TitleBar } from "./components/TitleBar";
import { UpdateNoticeHost } from "./components/UpdateNoticeHost";
import { WelcomeScreen } from "./components/WelcomeScreen";
import { saveChatCheckpointStore } from "./lib/aiChatCheckpointStore";
import { loadAiChatHistory, resetAiChatPersistDigest, saveAiChatHistory } from "./lib/aiChatHistory";
import { ensureBundledAgentBrowserLatest } from "./lib/agentBrowserAutoUpdate";
import { AI_PREFERENCES_KEY, normalizeAiPreferences } from "./lib/aiPreferences";
import { buildAiProjectIndexSnapshot } from "./lib/aiProjectIndex";
import { closedDocumentIdsForAllDocuments, closedDocumentIdsForDocumentInGroup } from "./lib/editorCloseTargets";
import { normalizeLocale, UI_LOCALE_KEY } from "./lib/i18n";
import { resetEditorFontZoom, toggleEditorMinimap, toggleEditorWordWrap, zoomEditorFontIn, zoomEditorFontOut } from "./lib/editorPreferenceCommands";
import { EDITOR_PREFERENCES_KEY, normalizeEditorPreferences } from "./lib/editorPreferences";
import { buildFileTreeDirectories, normalizePath } from "./lib/fileTree";
import { createKeybindingDispatcher, KEYBINDINGS_SETTINGS_KEY } from "./lib/keybindings";
import { buildProjectLoadSummary } from "./lib/projectLoadPresentation";
import { maybeAutoInstallLanguageServers, resetLspAutoInstallAttempts } from "./lib/lspAutoInstall";
import { bootstrapManagedRuntimes } from "./lib/runtimeBootstrap";
import { createEmptyAiIndexState, createIdleProjectLoadState, isAiChatSessionBusyStatus, useLuxStore, type Activity } from "./lib/store";
import { luxCommands, subscribeLuxEvents } from "./lib/tauri";
import { pickAndOpenWorkspace } from "./lib/workspaceActions";
import type { RecentWorkspace, WorkspaceInfo } from "./lib/types";

const BottomPanel = lazy(() => import("./components/BottomPanel").then((module) => ({ default: module.BottomPanel })));
const CommandPalette = lazy(() => import("./components/CommandPalette").then((module) => ({ default: module.CommandPalette })));
const EditorArea = lazy(() => import("./components/EditorArea").then((module) => ({ default: module.EditorArea })));
const SettingsDialog = lazy(() => import("./components/SettingsDialog").then((module) => ({ default: module.SettingsDialog })));
const Sidebar = lazy(() => import("./components/Sidebar").then((module) => ({ default: module.Sidebar })));

export function App() {
  const setWorkspace = useLuxStore((state) => state.setWorkspace);
  const setFileEntries = useLuxStore((state) => state.setFileEntries);
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
  const setProjectLoad = useLuxStore((state) => state.setProjectLoad);
  const setDiagnosticsForPath = useLuxStore((state) => state.setDiagnosticsForPath);
  const appendTerminalOutput = useLuxStore((state) => state.appendTerminalOutput);
  const clearDiagnostics = useLuxStore((state) => state.clearDiagnostics);
  const setEditorPreferences = useLuxStore((state) => state.setEditorPreferences);
  const setLocale = useLuxStore((state) => state.setLocale);
  const locale = useLuxStore((state) => state.locale);
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const aiIndex = useLuxStore((state) => state.aiIndex);
  const aiChatSessions = useLuxStore((state) => state.aiChatSessions);
  const activeAiChatSessionId = useLuxStore((state) => state.activeAiChatSessionId);
  const setAiPreferences = useLuxStore((state) => state.setAiPreferences);
  const setAiIndex = useLuxStore((state) => state.setAiIndex);
  const setAiChatSessions = useLuxStore((state) => state.setAiChatSessions);
  const setKeybindingProfile = useLuxStore((state) => state.setKeybindingProfile);
  const keybindingProfile = useLuxStore((state) => state.keybindingProfile);
  const workspace = useLuxStore((state) => state.workspace);
  const workspaceMode = useLuxStore((state) => state.workspaceMode);
  const projectLoad = useLuxStore((state) => state.projectLoad);
  const fileTreeLoading = useLuxStore((state) => state.fileTreeLoading);
  const languageServersLoading = useLuxStore((state) => state.languageServersLoading);
  const bottomPanelOpen = useLuxStore((state) => state.bottomPanelOpen);
  const sidebarVisible = useLuxStore((state) => state.sidebarVisible);
  const aiChatOpen = useLuxStore((state) => state.aiChatOpen);
  const commandPaletteOpen = useLuxStore((state) => state.commandPaletteOpen);
  const settingsOpen = useLuxStore((state) => state.settingsOpen);
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
  const [aiIndexRefreshToken, setAiIndexRefreshToken] = useState(0);
  const keybindingDispatcherRef = useRef(createKeybindingDispatcher(keybindingProfile));
  const bottomPanelRef = useRef<PanelImperativeHandle | null>(null);
  const aiChatHistoryLoadedRef = useRef(false);
  const skipNextAiChatPersistRef = useRef(true);
  const aiChatPersistTimerRef = useRef<number | null>(null);
  const fileEntryCount = useLuxStore((state) => state.fileEntries.length);
  const languageServerCount = useLuxStore((state) => state.languageServers.length);
  const projectLoadSummary = useMemo(
    () =>
      buildProjectLoadSummary({
        aiIndex: {
          indexedFiles: aiIndex.indexedFiles,
          progress: aiIndex.progress,
          status: aiIndex.status,
          totalFiles: aiIndex.totalFiles,
        },
        aiIndexStatus: aiIndex.status,
        fileEntryCount,
        fileTreeLoading,
        languageServerCount,
        languageServersLoading,
        projectIndexingEnabled: aiPreferences.projectIndexingEnabled,
        projectLoad,
      }),
    [
      aiIndex.indexedFiles,
      aiIndex.progress,
      aiIndex.status,
      aiIndex.totalFiles,
      aiPreferences.projectIndexingEnabled,
      fileEntryCount,
      fileTreeLoading,
      languageServerCount,
      languageServersLoading,
      projectLoad,
    ],
  );
  const dismissProjectLoadError = () => setProjectLoad(createIdleProjectLoadState());

  const openProject = () => {
    requestCloseDocuments(closedDocumentIdsForAllDocuments(openDocuments), () => {
      setProjectLoad({ active: true, error: null, progress: 4, root: null, stage: "opening", workspaceName: null });
      void pickAndOpenWorkspace().then((workspace) => {
        if (workspace) {
          setProjectLoad({ active: true, error: null, progress: 12, root: workspace.root, stage: "opening", workspaceName: workspace.name });
          setWorkspace(workspace);
          refreshRecentWorkspaces(setRecentWorkspaces);
          return;
        }
        setProjectLoad(createIdleProjectLoadState());
      }).catch((error) => setProjectLoad({ active: false, error: readErrorMessage(error), progress: 0, root: null, stage: "error", workspaceName: null }));
    }, { title: "Save changes before opening another folder?" });
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
    requestCloseDocuments(closedDocumentIdsForAllDocuments(openDocuments), () => {
      setProjectLoad({ active: true, error: null, progress: 8, root, stage: "opening", workspaceName: null });
      void luxCommands.workspaceOpen(root).then((workspace) => {
        setProjectLoad({ active: true, error: null, progress: 12, root: workspace.root, stage: "opening", workspaceName: workspace.name });
        setWorkspace(workspace);
        refreshRecentWorkspaces(setRecentWorkspaces);
      }).catch(() => {
        setProjectLoad({ active: false, error: "Failed to open recent project.", progress: 0, root, stage: "error", workspaceName: null });
        void luxCommands.recentWorkspaceForget(root).then(setRecentWorkspaces).catch(() => undefined);
      });
    }, { title: "Save changes before switching folders?" });
  };

  const forgetRecentWorkspace = (root: string) => {
    void luxCommands.recentWorkspaceForget(root).then(setRecentWorkspaces).catch(() => undefined);
  };

  useEffect(() => {
    refreshRecentWorkspaces(setRecentWorkspaces);
    // Provision the baseline managed runtime (Node) in the background at startup, so
    // the common language servers work out of the box even with no system toolchain.
    bootstrapManagedRuntimes();
  }, []);

  const agentBrowserAutoUpdateKeyRef = useRef<string | null>(null);
  useEffect(() => {
    if (!aiPreferences.agentBrowserEnabled) return;
    const key = aiPreferences.agentBrowserCommand.trim();
    if (agentBrowserAutoUpdateKeyRef.current === key) return;
    agentBrowserAutoUpdateKeyRef.current = key;
    void ensureBundledAgentBrowserLatest(aiPreferences);
  }, [aiPreferences.agentBrowserEnabled, aiPreferences.agentBrowserCommand]);

  useEffect(() => {
    if (aiChatHistoryLoadedRef.current) return;
    aiChatHistoryLoadedRef.current = true;
    let cancelled = false;
    void loadAiChatHistory().then((history) => {
      if (cancelled) return;
      // Loaded state is the new persistence baseline — force the next save to write so
      // the digest can't carry over a stale value and skip the first real change.
      resetAiChatPersistDigest();
      if (history && history.sessions.length > 0) setAiChatSessions(history);
    }).catch((error) => {
      if (cancelled) return;
      const store = useLuxStore.getState();
      store.setAiChatSessionStatus(store.activeAiChatSessionId, "error", readErrorMessage(error));
    }).finally(() => {
      if (!cancelled) skipNextAiChatPersistRef.current = false;
    });
    return () => {
      cancelled = true;
    };
  }, [setAiChatSessions]);

  useEffect(() => {
    if (!aiChatHistoryLoadedRef.current || skipNextAiChatPersistRef.current) return;
    if (aiChatPersistTimerRef.current !== null) {
      window.clearTimeout(aiChatPersistTimerRef.current);
      aiChatPersistTimerRef.current = null;
    }
    if (aiChatSessions.some((session) => isAiChatSessionBusyStatus(session.status))) return;

    aiChatPersistTimerRef.current = window.setTimeout(() => {
      aiChatPersistTimerRef.current = null;
      void saveAiChatHistory({
        activeSessionId: activeAiChatSessionId,
        sessions: aiChatSessions,
      }).then(() => saveChatCheckpointStore()).catch(reportAiChatHistoryError);
    }, 450);

    return () => {
      if (aiChatPersistTimerRef.current !== null) {
        window.clearTimeout(aiChatPersistTimerRef.current);
        aiChatPersistTimerRef.current = null;
      }
    };
  }, [activeAiChatSessionId, aiChatSessions]);

  useEffect(() => {
    void luxCommands.settingsGet("user", AI_PREFERENCES_KEY)
      .then((setting) => {
        // preserveText: this is the user's saved prefs, not display defaults — keep
        // multi-line editable bodies (custom system prompt, instructions) verbatim.
        const prefs = setting ? normalizeAiPreferences(setting.value, { preserveText: true }) : null;
        if (prefs) setAiPreferences(prefs);
        // Apply the persisted (or default) scan/search CPU budget to the Rust
        // worker pools on startup so the policy is in effect before any scan.
        void luxCommands.setScanConcurrency(prefs?.scanConcurrency ?? "auto");
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
    const scanLimit = aiIndexScanLimit(aiPreferences.maxIndexedFiles);
    if (!workspace) {
      setAiIndex(createEmptyAiIndexState(aiPreferences.projectIndexingEnabled ? "idle" : "disabled"));
      return;
    }
    if (!aiPreferences.projectIndexingEnabled) {
      setAiIndex({ ...createEmptyAiIndexState("disabled"), scanLimit, source: "workspace-scan", workspaceRoot: workspace.root });
      return;
    }

    let cancelled = false;
    const startedAtMs = performance.now();
    setAiIndex({
      ...createEmptyAiIndexState("indexing"),
      progress: 8,
      scanLimit,
      source: "workspace-scan",
      workspaceRoot: workspace.root,
    });
    const indexTimer = window.setTimeout(() => {
      if (cancelled) return;
      void buildWorkspaceAiIndex(workspace, aiPreferences.includeImages, aiPreferences.maxIndexedFiles, scanLimit, startedAtMs)
        .then((snapshot) => {
          if (cancelled) return;
          setAiIndex({
            ...snapshot,
            status: "ready",
            progress: 100,
            updatedAt: new Date().toISOString(),
          });
        })
        .catch((error) => {
          if (cancelled) return;
          setAiIndex({
            ...createEmptyAiIndexState("idle"),
            lastError: readErrorMessage(error),
            scanLimit,
            source: "workspace-scan",
            workspaceRoot: workspace.root,
          });
          // Indexing error is handled by setAiIndex above
        });
    }, 60);

    return () => {
      cancelled = true;
      window.clearTimeout(indexTimer);
    };
  }, [aiIndexRefreshToken, aiPreferences.includeImages, aiPreferences.maxIndexedFiles, aiPreferences.projectIndexingEnabled, setAiIndex, setProjectLoad, workspace]);

  useEffect(() => {
    if (!workspace || projectLoad.stage !== "indexing") return;
    if (aiIndex.status !== "indexing") {
      setProjectLoad({
        active: false,
        progress: 100,
        root: workspace.root,
        stage: "ready",
        workspaceName: workspace.name,
      });
    }
  }, [aiIndex.status, projectLoad.stage, setProjectLoad, workspace]);

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
    setProjectLoad({ active: true, error: null, progress: 28, root: workspace.root, stage: "files", workspaceName: workspace.name });
    setBottomPanelMaximized(false);
    resetLspAutoInstallAttempts();
    let cancelled = false;
    clearDiagnostics();
    setFileTreeLoading(true);
    setFileTreeError(null);

    void refreshWorkspaceFileTree(workspace, true)
      .catch((error) => {
        if (!cancelled) setProjectLoad({ active: false, error: readErrorMessage(error), progress: 100, root: workspace.root, stage: "error", workspaceName: workspace.name });
      })
      .finally(() => {
        if (cancelled) return;
        setFileTreeLoading(false);
        // Files are ready ⇒ the project is ready (or moves to indexing). Language
        // servers are NOT a gate: they load in the background and can never hold
        // the splash. This is what kills the "Starting language services" hang.
        if (useLuxStore.getState().projectLoad.stage !== "error") {
          const indexingEnabled = useLuxStore.getState().aiPreferences.projectIndexingEnabled;
          const indexBusy = useLuxStore.getState().aiIndex.status === "indexing";
          setProjectLoad({
            active: indexingEnabled && indexBusy,
            progress: indexingEnabled && indexBusy ? 78 : 100,
            root: workspace.root,
            stage: indexingEnabled && indexBusy ? "indexing" : "ready",
            workspaceName: workspace.name,
          });
        }
      });
    luxCommands.gitStatus().then((s) => { if (!cancelled) setGitStatus(s); }).catch(() => { if (!cancelled) setGitStatus(null); });
    // Language-server discovery/startup runs fully in the background, detached from
    // the loading screen. The loading flag still drives the (non-blocking) status
    // chip, but its resolution never affects whether the project is "ready".
    setLanguageServersLoading(true);
    luxCommands.lspServers()
      .then((servers) => {
        if (cancelled) return;
        setLanguageServers(servers);
        // Auto-install missing servers for languages actually present in this
        // workspace (discovery only returns detected languages). Background,
        // gated by the setting; the install store streams progress to Settings,
        // and we re-pull the server list when each finishes so features light up.
        maybeAutoInstallLanguageServers(servers, () => {
          if (!cancelled) luxCommands.lspServers().then((next) => { if (!cancelled) setLanguageServers(next); }).catch(() => undefined);
        });
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
  }, [aiPreferences.projectIndexingEnabled, clearDiagnostics, setDiagnosticsForPath, setFileEntries, setFileTreeDirectories, setFileTreeError, setFileTreeLoading, setGitStatus, setLanguageServers, setLanguageServersLoading, setProjectLoad, workspace]);

  useEffect(() => {
    let active = true;
    let dispose: (() => void) | undefined;
    let fsRefreshTimer: number | undefined;
    let aiIndexRefreshTimer: number | undefined;
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

    const scheduleAiIndexRefresh = () => {
      const preferences = useLuxStore.getState().aiPreferences;
      if (!preferences.projectIndexingEnabled || !preferences.realtimeIndexing) return;
      if (aiIndexRefreshTimer !== undefined) window.clearTimeout(aiIndexRefreshTimer);
      aiIndexRefreshTimer = window.setTimeout(() => {
        aiIndexRefreshTimer = undefined;
        setAiIndexRefreshToken((token) => token + 1);
      }, 650);
    };

    void subscribeLuxEvents((event) => {
      if (event.type === "workspaceChanged") setWorkspace(event.workspace);
      if (event.type === "fsChanged") {
        const touchedRoots = workspaceRootsForChangedPath(event.path);
        for (const root of touchedRoots) pendingFsRefreshRoots.set(normalizePath(root.root), root);
        if (touchedRoots.length > 0) {
          scheduleFsRefresh();
          scheduleAiIndexRefresh();
        }
      }
      if (event.type === "editorDocumentClosed") closeDocument(event.document.id);
      if (event.type === "editorDocumentChanged") upsertDocument(event.document);
      if (event.type === "editorDocumentsChanged") updateOpenDocuments(event.documents);
      if (event.type === "editorDocumentEdited") applyDocumentEdits(event.document.id, [], event.document);
      if (event.type === "editorDiagnosticsChanged") setDiagnosticsForPath(event.path, event.diagnostics);
      if (event.type === "gitStatusChanged") setGitStatus(event.status);
      if (event.type === "terminalOutput") appendTerminalOutput(event.session_id, event.data);
      if (event.type === "settingsChanged" && event.key === KEYBINDINGS_SETTINGS_KEY) {
        void luxCommands.keybindingsGet().then(setKeybindingProfile).catch(() => undefined);
      }
    }).then((unlisten) => {
      if (!active) unlisten();
      else dispose = unlisten;
    });
    return () => {
      active = false;
      if (fsRefreshTimer !== undefined) window.clearTimeout(fsRefreshTimer);
      if (aiIndexRefreshTimer !== undefined) window.clearTimeout(aiIndexRefreshTimer);
      dispose?.();
    };
  }, [appendTerminalOutput, applyDocumentEdits, closeDocument, setDiagnosticsForPath, setGitStatus, setKeybindingProfile, setWorkspace, updateOpenDocuments, upsertDocument]);

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
            projectLoad={projectLoadSummary}
            onOpenProject={openProject}
          />
          <ProjectLoadingStatus summary={projectLoadSummary} onDismissError={dismissProjectLoadError} />
          <DeferredCommandPalette open={commandPaletteOpen} />
          <DeferredSettingsDialog open={settingsOpen} />
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
                  <DeferredEditorArea />
                ) : (
                  <WelcomeScreen
                    loading={projectLoadSummary.active}
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
                    <LazyAiChatPanel presentation="panel" />
                  </Panel>
                </>
              )}
            </Group>
          </div>
          {bottomPanelOpen && <DeferredBottomPanel isMaximized={bottomPanelMaximized} onToggleMaximized={toggleBottomPanelMaximized} />}
        </div>
        <ProjectLoadingStatus summary={projectLoadSummary} onDismissError={dismissProjectLoadError} />
        <StatusBar />
        <DeferredCommandPalette open={commandPaletteOpen} />
        <DeferredSettingsDialog open={settingsOpen} />
        <UpdateNoticeHost />
      </div>
    );
  }

  if (workspaceMode === "agent") {
    return (
      <div className="app-shell agent-shell">
        <TitleBar />
        <AgentWorkspace
          projectLoad={projectLoadSummary}
          onOpenProject={openProject}
        />
        <ProjectLoadingStatus summary={projectLoadSummary} onDismissError={dismissProjectLoadError} />
        <DeferredCommandPalette open={commandPaletteOpen} />
        <DeferredSettingsDialog open={settingsOpen} />
        <UpdateNoticeHost />
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
                <DeferredEditorArea />
              </Panel>
              {bottomPanelOpen && (
                <>
                  <Separator className="resize-handle horizontal" />
                  <Panel defaultSize="210px" minSize="150px" maxSize="70%" panelRef={bottomPanelRef}>
                    <DeferredBottomPanel isMaximized={bottomPanelMaximized} onToggleMaximized={toggleBottomPanelMaximized} />
                  </Panel>
                </>
              )}
            </Group>
          </Panel>
          {aiChatOpen && (
            <>
              <Separator className="resize-handle editor-group-separator" />
              <Panel defaultSize="32%" minSize="300px" maxSize="48%">
                <LazyAiChatPanel presentation="panel" />
              </Panel>
            </>
          )}
        </Group>
      </div>
      <ProjectLoadingStatus summary={projectLoadSummary} onDismissError={dismissProjectLoadError} />
      <StatusBar />
      <DeferredCommandPalette open={commandPaletteOpen} />
      <DeferredSettingsDialog open={settingsOpen} />
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

function aiIndexScanLimit(maxIndexedFiles: number) {
  return Math.min(50_000, Math.max(maxIndexedFiles * 2, maxIndexedFiles + 1_000, 2_500));
}

async function buildWorkspaceAiIndex(workspace: WorkspaceInfo, includeImages: boolean, maxIndexedFiles: number, scanLimit: number, startedAtMs: number) {
  const entries = await luxCommands.fsListFiles(scanLimit);
  return buildAiProjectIndexSnapshot(entries, {
    finishedAtMs: performance.now(),
    includeImages,
    maxIndexedFiles,
    scanLimit,
    source: "workspace-scan",
    startedAtMs,
    workspaceRoot: workspace.root,
  });
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

function readErrorMessage(error: unknown) {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Failed to load project file tree.";
}

function reportAiChatHistoryError(error: unknown) {
  console.error("AI chat history sync failed", error);
}

function DeferredEditorArea() {
  return (
    <Suspense fallback={<section className="editor-empty" aria-busy="true" />}>
      <EditorArea />
    </Suspense>
  );
}

function DeferredBottomPanel({ isMaximized, onToggleMaximized }: { isMaximized: boolean; onToggleMaximized: () => void }) {
  return (
    <Suspense fallback={<section className="bottom-panel" data-maximized={isMaximized} aria-busy="true" />}>
      <BottomPanel isMaximized={isMaximized} onToggleMaximized={onToggleMaximized} />
    </Suspense>
  );
}

function DeferredCommandPalette({ open }: { open: boolean }) {
  if (!open) return null;
  return (
    <Suspense fallback={null}>
      <CommandPalette />
    </Suspense>
  );
}

function DeferredSettingsDialog({ open }: { open: boolean }) {
  if (!open) return null;
  return (
    <Suspense fallback={null}>
      <SettingsDialog />
    </Suspense>
  );
}

function ExplorerPanelSlot() {
  return (
    <>
      <Panel defaultSize="288px" minSize="220px" maxSize="430px">
        <Suspense fallback={<aside className="sidebar" data-side="left" aria-busy="true" />}>
          <Sidebar side="left" />
        </Suspense>
      </Panel>
      <Separator className="resize-handle" />
    </>
  );
}
