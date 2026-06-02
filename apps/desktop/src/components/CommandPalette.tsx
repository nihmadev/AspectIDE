import { Command } from "cmdk";
import {
  Bug,
  CircleX,
  FileCode2,
  FolderOpen,
  GitBranch,
  LayoutPanelLeft,
  ListTree,
  ServerCog,
  PanelBottom,
  RotateCcw,
  Sparkles,
  Save,
  SaveAll,
  Search,
  Settings,
  SquareSplitHorizontal,
  TerminalSquare,
  WrapText,
  PlugZap,
} from "lucide-react";
import * as Dialog from "@radix-ui/react-dialog";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { useEditorCloseGuard } from "./EditorCloseGuard";
import { documentDisplayPath } from "../lib/documents";
import { resetEditorFontZoom, toggleEditorMinimap, toggleEditorWordWrap, zoomEditorFontIn, zoomEditorFontOut } from "../lib/editorPreferenceCommands";
import {
  closedDocumentIdsForAllDocuments,
  closedDocumentIdsForDocumentInGroup,
  closedDocumentIdsForOtherDocuments,
} from "../lib/editorCloseTargets";
import { formatKeybindingForDisplay } from "../lib/keybindings";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
import { displayPath as formatPath, normalizePath } from "../lib/fileTree";
import { useLuxStore, type Activity } from "../lib/store";
import { luxCommands } from "../lib/tauri";
import { pickAndOpenWorkspace, reloadWorkspace } from "../lib/workspaceActions";
import type { ExtensionCommandExecution, ExtensionCommandRoute, FsEntry, LspWorkspaceSymbol } from "../lib/types";

const MAX_QUICK_OPEN_FILES = 2_500;

type PaletteCommand = {
  id: string;
  label: string;
  detail: string;
  shortcut?: string;
  icon: typeof FolderOpen;
  run: () => void;
  closeOnRun?: boolean;
};

export function CommandPalette() {
  const { t } = useTranslation();
  const open = useLuxStore((state) => state.commandPaletteOpen);
  const setOpen = useLuxStore((state) => state.setCommandPaletteOpen);
  const workspace = useLuxStore((state) => state.workspace);
  const setWorkspace = useLuxStore((state) => state.setWorkspace);
  const setActiveActivity = useLuxStore((state) => state.setActiveActivity);
  const setSidebarVisible = useLuxStore((state) => state.setSidebarVisible);
  const sidebarVisible = useLuxStore((state) => state.sidebarVisible);
  const aiChatOpen = useLuxStore((state) => state.aiChatOpen);
  const toggleAiChat = useLuxStore((state) => state.toggleAiChat);
  const setBottomPanelOpen = useLuxStore((state) => state.setBottomPanelOpen);
  const bottomPanelOpen = useLuxStore((state) => state.bottomPanelOpen);
  const editorPreferences = useLuxStore((state) => state.editorPreferences);
  const setSettingsOpen = useLuxStore((state) => state.setSettingsOpen);
  const openBottomPanel = useLuxStore((state) => state.openBottomPanel);
  const setGitStatus = useLuxStore((state) => state.setGitStatus);
  const languageServers = useLuxStore((state) => state.languageServers);
  const setLanguageServers = useLuxStore((state) => state.setLanguageServers);
  const setLanguageServersLoading = useLuxStore((state) => state.setLanguageServersLoading);
  const upsertTerminalSession = useLuxStore((state) => state.upsertTerminalSession);
  const keybindingProfile = useLuxStore((state) => state.keybindingProfile);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);
  const setPendingEditorReveal = useLuxStore((state) => state.setPendingEditorReveal);
  const openDocuments = useLuxStore((state) => state.openDocuments);
  const activeDocumentId = useLuxStore((state) => state.activeDocumentId);
  const activeEditorGroupId = useLuxStore((state) => state.activeEditorGroupId);
  const editorGroups = useLuxStore((state) => state.editorGroups);
  const splitActiveEditor = useLuxStore((state) => state.splitActiveEditor);
  const closeDocumentInActiveGroup = useLuxStore((state) => state.closeDocumentInActiveGroup);
  const closeOtherDocuments = useLuxStore((state) => state.closeOtherDocuments);
  const closeAllDocuments = useLuxStore((state) => state.closeAllDocuments);
  const selectNextDocument = useLuxStore((state) => state.selectNextDocument);
  const selectPreviousDocument = useLuxStore((state) => state.selectPreviousDocument);
  const [search, setSearch] = useState("");
  const [files, setFiles] = useState<FsEntry[]>([]);
  const [workspaceSymbols, setWorkspaceSymbols] = useState<LspWorkspaceSymbol[]>([]);
  const [extensionCommandRoutes, setExtensionCommandRoutes] = useState<ExtensionCommandRoute[]>([]);
  const [extensionCommandRoutesLoaded, setExtensionCommandRoutesLoaded] = useState(false);
  const [indexError, setIndexError] = useState<string | null>(null);
  const [symbolsError, setSymbolsError] = useState<string | null>(null);
  const [extensionCommandError, setExtensionCommandError] = useState<string | null>(null);
  const { requestCloseDocuments } = useEditorCloseGuard();

  const fileIndexMutation = useMutation({
    mutationFn: () => luxCommands.fsListFiles(MAX_QUICK_OPEN_FILES),
    onSuccess: (entries) => {
      setFiles(entries);
      setIndexError(null);
    },
    onError: (error) => setIndexError(readErrorMessage(error, t)),
  });

  const workspaceSymbolsMutation = useMutation({
    mutationFn: luxCommands.lspWorkspaceSymbols,
    onSuccess: (symbols) => {
      setWorkspaceSymbols(symbols);
      setSymbolsError(null);
    },
    onError: (error) => setSymbolsError(readErrorMessage(error, t)),
  });

  const extensionCommandsMutation = useMutation({
    mutationFn: luxCommands.extensionsCommandRoutes,
    onSuccess: (routes) => {
      setExtensionCommandRoutes(routes);
      setExtensionCommandRoutesLoaded(true);
      setExtensionCommandError(null);
    },
    onError: (error) => {
      setExtensionCommandRoutesLoaded(true);
      setExtensionCommandError(readErrorMessage(error, t));
    },
  });

  const executeExtensionCommandMutation = useMutation({
    mutationFn: luxCommands.extensionsExecuteCommand,
    onSuccess: (report) => {
      if (report.status === "failed") {
        setExtensionCommandError(formatExtensionCommandExecutionError(report, t));
        return;
      }
      setExtensionCommandError(null);
      setOpen(false);
      setSearch("");
    },
    onError: (error) => setExtensionCommandError(readErrorMessage(error, t)),
  });

  const openFileMutation = useMutation({
    mutationFn: luxCommands.editorOpenFile,
    onSuccess: (document) => {
      upsertDocument(document);
      setOpen(false);
      setSearch("");
    },
  });

  const openSymbolMutation = useMutation({
    mutationFn: async (symbol: LspWorkspaceSymbol) => ({ symbol, document: await luxCommands.editorOpenFile(symbol.location.path) }),
    onSuccess: ({ document, symbol }) => {
      upsertDocument(document);
      setPendingEditorReveal({
        documentId: document.id,
        line: symbol.location.range.start_line,
        column: symbol.location.range.start_column,
      });
      setOpen(false);
      setSearch("");
    },
  });

  const saveFileMutation = useMutation({
    mutationFn: luxCommands.editorSaveFile,
    onSuccess: upsertDocument,
  });

  const saveAsFileMutation = useMutation({
    mutationFn: luxCommands.editorSaveFileAs,
    onSuccess: upsertDocument,
  });

  const activeDocument = openDocuments.find((document) => document.id === activeDocumentId) ?? null;
  const hasDirtyDocuments = openDocuments.some((document) => document.is_dirty);
  const saveDocument = useCallback((id: string) => saveFileMutation.mutate(id), [saveFileMutation]);
  const refreshLanguageServers = useCallback(() => {
    setLanguageServersLoading(true);
    void luxCommands.lspServers()
      .then(setLanguageServers)
      .catch(() => setLanguageServers([]))
      .finally(() => setLanguageServersLoading(false));
  }, [setLanguageServers, setLanguageServersLoading]);

  useEffect(() => {
    if (open && workspace && files.length === 0 && !fileIndexMutation.isPending) {
      fileIndexMutation.mutate();
    }
  }, [fileIndexMutation, files.length, open, workspace]);

  const commandMode = search.trimStart().startsWith(">");
  const symbolMode = search.trimStart().startsWith("@");
  const query = commandMode || symbolMode ? search.trimStart().slice(1).trim() : search.trim();

  useEffect(() => {
    if (!open || !commandMode || extensionCommandRoutesLoaded || extensionCommandsMutation.isPending) return;
    extensionCommandsMutation.mutate();
  }, [commandMode, extensionCommandRoutesLoaded, extensionCommandsMutation, open]);

  useEffect(() => {
    if (!open || !workspace || !symbolMode) return;
    if (query.length < 2) {
      setWorkspaceSymbols([]);
      setSymbolsError(null);
      return;
    }

    const handle = window.setTimeout(() => workspaceSymbolsMutation.mutate(query), 160);
    return () => window.clearTimeout(handle);
  }, [open, query, symbolMode, workspace, workspaceSymbolsMutation]);

  const commands = useMemo<PaletteCommand[]>(
    () => [
      {
        id: "file.new-untitled",
        label: t("command.title.newTextFile"),
        detail: t("command.detail.newTextFile"),
        shortcut: shortcutFor("workbench.action.files.newUntitledFile", keybindingProfile),
        icon: FileCode2,
        run: () => {
          void luxCommands.editorNewFile().then(upsertDocument).catch(() => undefined);
        },
      },
      ...(workspace
        ? [
            {
              id: "lsp.refresh",
              label: t("command.title.refreshServers"),
              detail: t("command.detail.refreshServers", { summary: formatLanguageServerSummary(languageServers, t) }),
              icon: ServerCog,
              run: refreshLanguageServers,
            },
          ]
        : []),
      {
        id: "settings.open",
        label: t("command.title.openSettings"),
        detail: t("command.detail.openSettings"),
        shortcut: shortcutFor("workbench.action.openSettings", keybindingProfile),
        icon: Settings,
        run: () => setSettingsOpen(true),
      },
      {
        id: "workspace.open-current",
        label: workspace ? t("command.title.reloadCurrentFolder") : t("command.title.openFolder"),
        detail: workspace?.root ?? t("command.detail.openFolder"),
        shortcut: shortcutFor("workbench.action.openFolder", keybindingProfile),
        icon: FolderOpen,
        run: () => {
          requestCloseDocuments(closedDocumentIdsForAllDocuments(openDocuments), () => {
            const action = workspace ? reloadWorkspace(workspace) : pickAndOpenWorkspace();
            void action.then((openedWorkspace) => {
              if (openedWorkspace) setWorkspace(openedWorkspace);
            });
          }, { title: workspace ? t("command.confirm.reloadFolder") : t("command.confirm.openAnotherFolder") });
        },
      },
      {
        id: "activity.explorer",
        label: t("command.title.showExplorer"),
        detail: t("command.detail.showExplorer"),
        shortcut: shortcutFor("workbench.view.explorer", keybindingProfile),
        icon: LayoutPanelLeft,
        run: () => showActivity("explorer", setActiveActivity, setSidebarVisible),
      },
      {
        id: "file.save",
        label: t("command.title.saveActiveEditor"),
        detail: activeDocument ? documentDisplayPath(activeDocument) : t("command.detail.noActiveEditor"),
        shortcut: shortcutFor("workbench.action.files.save", keybindingProfile),
        icon: Save,
        run: () => {
          if (activeDocument) saveDocument(activeDocument.id);
        },
      },
      {
        id: "file.save-as",
        label: t("command.title.saveActiveEditorAs"),
        detail: activeDocument ? documentDisplayPath(activeDocument) : t("command.detail.noActiveEditor"),
        shortcut: shortcutFor("workbench.action.files.saveAs", keybindingProfile),
        icon: Save,
        run: () => {
          if (activeDocument) saveAsFileMutation.mutate(activeDocument.id);
        },
      },
      {
        id: "file.save-all",
        label: t("command.title.saveAllEditors"),
        detail: hasDirtyDocuments ? t("command.detail.saveAllEditorsDirty") : t("command.detail.saveAllEditorsClean"),
        shortcut: shortcutFor("workbench.action.files.saveAll", keybindingProfile),
        icon: SaveAll,
        run: () => {
          for (const document of openDocuments) {
            if (document.is_dirty) saveDocument(document.id);
          }
        },
      },
      {
        id: "editor.toggle-word-wrap",
        label: editorPreferences.wordWrap === "on" ? t("command.title.disableWordWrap") : t("command.title.enableWordWrap"),
        detail: t("command.detail.toggleWordWrap"),
        shortcut: shortcutFor("editor.action.toggleWordWrap", keybindingProfile),
        icon: WrapText,
        run: toggleEditorWordWrap,
      },
      {
        id: "editor.toggle-minimap",
        label: editorPreferences.minimap ? t("command.title.hideMinimap") : t("command.title.showMinimap"),
        detail: t("command.detail.toggleMinimap"),
        shortcut: shortcutFor("editor.action.toggleMinimap", keybindingProfile),
        icon: LayoutPanelLeft,
        run: toggleEditorMinimap,
      },
      {
        id: "editor.font-zoom-in",
        label: t("command.title.zoomFontIn"),
        detail: t("command.detail.currentFontSize", { fontSize: editorPreferences.fontSize }),
        shortcut: shortcutFor("editor.action.fontZoomIn", keybindingProfile),
        icon: Search,
        run: zoomEditorFontIn,
      },
      {
        id: "editor.font-zoom-out",
        label: t("command.title.zoomFontOut"),
        detail: t("command.detail.currentFontSize", { fontSize: editorPreferences.fontSize }),
        shortcut: shortcutFor("editor.action.fontZoomOut", keybindingProfile),
        icon: Search,
        run: zoomEditorFontOut,
      },
      {
        id: "editor.font-zoom-reset",
        label: t("command.title.resetFontZoom"),
        detail: t("command.detail.resetFontZoom"),
        shortcut: shortcutFor("editor.action.fontZoomReset", keybindingProfile),
        icon: RotateCcw,
        run: resetEditorFontZoom,
      },
      {
        id: "editor.close-active",
        label: t("command.title.closeActiveEditor"),
        detail: activeDocument ? documentDisplayPath(activeDocument) : t("command.detail.noActiveEditor"),
        shortcut: shortcutFor("workbench.action.closeActiveEditor", keybindingProfile),
        icon: CircleX,
        run: () => {
          if (activeDocument) {
            requestCloseDocuments(
              closedDocumentIdsForDocumentInGroup(openDocuments, editorGroups, activeEditorGroupId, activeDocument.id),
              closeDocumentInActiveGroup,
            );
          }
        },
      },
      {
        id: "editor.split-right",
        label: t("command.title.splitEditorRight"),
        detail: activeDocument ? documentDisplayPath(activeDocument) : t("command.detail.noActiveEditor"),
        shortcut: shortcutFor("workbench.action.splitEditorRight", keybindingProfile),
        icon: SquareSplitHorizontal,
        run: () => {
          if (activeDocument) splitActiveEditor();
        },
      },
      {
        id: "editor.close-others",
        label: t("command.title.closeOtherEditors"),
        detail: activeDocument ? t("command.detail.closeOtherEditors") : t("command.detail.noActiveEditor"),
        icon: CircleX,
        run: () => {
          if (activeDocument) {
            requestCloseDocuments(
              closedDocumentIdsForOtherDocuments(openDocuments, activeDocument.id),
              () => closeOtherDocuments(activeDocument.id),
              { title: t("command.confirm.closeOtherEditors") },
            );
          }
        },
      },
      {
        id: "editor.close-all",
        label: t("command.title.closeAllEditors"),
        detail: t("command.detail.openEditorsCount", { count: openDocuments.length }),
        icon: CircleX,
        run: () => {
          requestCloseDocuments(
            closedDocumentIdsForAllDocuments(openDocuments),
            closeAllDocuments,
            { title: t("command.confirm.closeAllEditors") },
          );
        },
      },
      {
        id: "editor.next",
        label: t("command.title.nextEditor"),
        detail: t("command.detail.nextEditor"),
        shortcut: shortcutFor("workbench.action.nextEditor", keybindingProfile),
        icon: FileCode2,
        run: selectNextDocument,
      },
      {
        id: "editor.previous",
        label: t("command.title.previousEditor"),
        detail: t("command.detail.previousEditor"),
        shortcut: shortcutFor("workbench.action.previousEditor", keybindingProfile),
        icon: FileCode2,
        run: selectPreviousDocument,
      },
      {
        id: "activity.search",
        label: t("command.title.findInFiles"),
        detail: t("command.detail.findInFiles"),
        shortcut: shortcutFor("workbench.view.search", keybindingProfile),
        icon: Search,
        run: () => showActivity("search", setActiveActivity, setSidebarVisible),
      },
      {
        id: "activity.git",
        label: t("command.title.showSourceControl"),
        detail: t("command.detail.showSourceControl"),
        shortcut: shortcutFor("workbench.view.scm", keybindingProfile),
        icon: GitBranch,
        run: () => showActivity("git", setActiveActivity, setSidebarVisible),
      },
      {
        id: "activity.run-debug",
        label: t("command.title.showRunAndDebug"),
        detail: t("command.detail.showRunAndDebug"),
        shortcut: shortcutFor("workbench.view.debug", keybindingProfile),
        icon: Bug,
        run: () => showActivity("runDebug", setActiveActivity, setSidebarVisible),
      },
      {
        id: "git.refresh",
        label: t("command.title.refreshGitStatus"),
        detail: t("command.detail.refreshGitStatus"),
        icon: GitBranch,
        run: () => {
          void luxCommands.gitStatus().then(setGitStatus).catch(() => setGitStatus(null));
          showActivity("git", setActiveActivity, setSidebarVisible);
        },
      },
      {
        id: "terminal.new",
        label: t("command.title.createNewTerminal"),
        detail: t("command.detail.createNewTerminal"),
        shortcut: shortcutFor("workbench.action.terminal.toggleTerminal", keybindingProfile),
        icon: TerminalSquare,
        run: () => {
          openBottomPanel("terminal");
          void luxCommands.terminalCreate().then((terminal) => upsertTerminalSession(terminal, true)).catch(() => undefined);
        },
      },
      {
        id: "chat.toggle",
        label: aiChatOpen ? t("command.title.hideChat") : t("command.title.showChat"),
        detail: t("command.detail.toggleChat"),
        shortcut: shortcutFor("workbench.action.chat.toggle", keybindingProfile),
        icon: Sparkles,
        run: toggleAiChat,
      },
      {
        id: "panel.toggle",
        label: bottomPanelOpen ? t("command.title.hideBottomPanel") : t("command.title.showBottomPanel"),
        detail: t("command.detail.toggleBottomPanel"),
        icon: PanelBottom,
        run: () => setBottomPanelOpen(!bottomPanelOpen),
      },
      {
        id: "sidebar.toggle",
        label: sidebarVisible ? t("command.title.hideSideBar") : t("command.title.showSideBar"),
        detail: t("command.detail.toggleSideBar"),
        icon: LayoutPanelLeft,
        run: () => setSidebarVisible(!sidebarVisible),
      },
      ...extensionCommandRoutes.map((route) => ({
        id: `extension.${route.id}`,
        label: route.title,
        detail: route.category ? `${route.category} - ${route.extension_name}` : route.extension_name,
        icon: PlugZap,
        closeOnRun: false,
        run: () => {
          setExtensionCommandError(null);
          void executeExtensionCommandMutation.mutate(route.id);
        },
      })),
    ],
    [
      bottomPanelOpen,
      aiChatOpen,
      activeDocument,
      activeEditorGroupId,
      closeAllDocuments,
      closeDocumentInActiveGroup,
      closeOtherDocuments,
      editorGroups,
      editorPreferences.minimap,
      editorPreferences.fontSize,
      editorPreferences.wordWrap,
      hasDirtyDocuments,
      keybindingProfile,
      languageServers,
      openBottomPanel,
      openDocuments,
      refreshLanguageServers,
      requestCloseDocuments,
      saveAsFileMutation,
      saveDocument,
      selectNextDocument,
      selectPreviousDocument,
      setActiveActivity,
      setBottomPanelOpen,
      setGitStatus,
      setSidebarVisible,
      setSettingsOpen,
      upsertTerminalSession,
      setWorkspace,
      sidebarVisible,
      splitActiveEditor,
      t,
      toggleAiChat,
      extensionCommandRoutes,
      executeExtensionCommandMutation,
      workspace,
    ],
  );

  const visibleCommands = useMemo(() => {
    if (!query) return commands;
    return commands.filter((command) => fuzzyScore(`${command.label} ${command.detail}`, query) > 0);
  }, [commands, query]);

  const visibleFiles = useMemo(() => {
    if (commandMode || symbolMode) return [];
    const candidates = files.map((file) => ({ file, score: query ? fuzzyScore(displayPath(file, workspace?.root), query) : 1 }));
    return candidates
      .filter((candidate) => candidate.score > 0)
      .sort((left, right) => right.score - left.score || displayPath(left.file, workspace?.root).localeCompare(displayPath(right.file, workspace?.root)))
      .slice(0, 80)
      .map((candidate) => candidate.file);
  }, [commandMode, files, query, symbolMode, workspace?.root]);

  const visibleWorkspaceSymbols = useMemo(() => {
    if (!symbolMode) return [];
    return workspaceSymbols
      .map((symbol) => ({ symbol, score: query ? fuzzyScore(`${symbol.name} ${symbol.container_name ?? ""} ${symbol.location.path}`, query) : 1 }))
      .filter((candidate) => candidate.score > 0)
      .sort((left, right) => right.score - left.score || left.symbol.name.localeCompare(right.symbol.name))
      .slice(0, 80)
      .map((candidate) => candidate.symbol);
  }, [query, symbolMode, workspaceSymbols]);

  const runPaletteCommand = (command: PaletteCommand) => {
    command.run();
    if (command.closeOnRun === false) return;
    setOpen(false);
    setSearch("");
  };

  return (
    <Dialog.Root open={open} onOpenChange={setOpen}>
      <Dialog.Portal>
        <Dialog.Overlay className="command-overlay" />
        <Dialog.Content className="command-dialog" aria-describedby={undefined} onOpenAutoFocus={(event) => event.preventDefault()}>
          <Dialog.Title className="sr-only">{t("command.dialog.title")}</Dialog.Title>
          <Command className="command-menu" shouldFilter={false}>
            <Command.Input
              placeholder={commandMode ? t("command.placeholder.runCommand") : symbolMode ? t("command.placeholder.goToSymbol") : t("command.placeholder.openFile")}
              value={search}
              autoFocus
              onValueChange={setSearch}
            />
            <Command.List>
              <Command.Empty>{symbolMode && workspaceSymbolsMutation.isPending ? t("command.status.searchingSymbols") : fileIndexMutation.isPending ? t("command.status.indexingWorkspace") : t("command.empty.noResult")}</Command.Empty>
              {!commandMode && !symbolMode && workspace && (
                <Command.Group heading={fileIndexMutation.isPending ? t("command.heading.indexingFiles") : t("command.heading.files")}>
                  {visibleFiles.map((file) => (
                    <Command.Item
                      key={file.path}
                      value={`file:${displayPath(file, workspace.root)}`}
                      onSelect={() => openFileMutation.mutate(file.path)}
                    >
                      <FileCode2 size={16} />
                      <span>{file.name}</span>
                      <small>- {displayPath(file, workspace.root)}</small>
                    </Command.Item>
                  ))}
                  {indexError && <div className="command-inline-error">{indexError}</div>}
                </Command.Group>
              )}
              {symbolMode && workspace && (
                <Command.Group heading={workspaceSymbolsMutation.isPending ? t("command.heading.searchingSymbols") : t("command.heading.workspaceSymbols")}>
                  {visibleWorkspaceSymbols.map((symbol, index) => (
                    <Command.Item
                      key={`${symbol.location.path}:${symbol.location.range.start_line}:${symbol.location.range.start_column}:${symbol.name}:${index}`}
                      value={`symbol:${symbol.name}:${symbol.location.path}`}
                      onSelect={() => openSymbolMutation.mutate(symbol)}
                    >
                      <ListTree size={16} />
                      <span>{symbol.name}</span>
                      <small>- {formatWorkspaceSymbolDetail(symbol, workspace.root)}</small>
                    </Command.Item>
                  ))}
                  {symbolsError && <div className="command-inline-error">{symbolsError}</div>}
                </Command.Group>
              )}
              <Command.Group heading={t("command.heading.commands")}>
                {visibleCommands.map((command) => {
                  const { detail, icon: Icon, id, label, shortcut } = command;
                  return (
                    <Command.Item key={id} value={`${label} ${detail}`} onSelect={() => runPaletteCommand(command)}>
                      <Icon size={16} />
                      <span>{label}</span>
                      {shortcut ? <kbd>{shortcut}</kbd> : <small>{detail}</small>}
                    </Command.Item>
                  );
                })}
                {extensionCommandError && <div className="command-inline-error">{extensionCommandError}</div>}
              </Command.Group>
            </Command.List>
            <div className="command-footer">
              <span>{t("command.footer.commandsHint")}</span>
              <span>{t("command.footer.symbolsHint")}</span>
              <span>{t("command.footer.enterHint")}</span>
            </div>
          </Command>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function showActivity(activity: Activity, setActiveActivity: (activity: Activity) => void, setSidebarVisible: (visible: boolean) => void) {
  setActiveActivity(activity);
  setSidebarVisible(true);
}

function shortcutFor(command: string, profile: Parameters<typeof formatKeybindingForDisplay>[1]) {
  return formatKeybindingForDisplay(command, profile);
}

function displayPath(file: FsEntry, root?: string) {
  if (!root) return formatPath(file.path);
  const normalizedRoot = formatPath(root).replace(/\/+$/, "");
  const normalizedPath = formatPath(file.path);
  return normalizePath(normalizedPath).startsWith(`${normalizePath(normalizedRoot)}/`)
    ? normalizedPath.slice(normalizedRoot.length + 1)
    : normalizedPath;
}

function formatWorkspaceSymbolDetail(symbol: LspWorkspaceSymbol, root?: string) {
  const relativePath = displayPath({
    name: symbol.name,
    path: symbol.location.path,
    kind: "file",
    size: 0,
    modified_at: null,
    is_hidden: false,
  }, root);
  const container = symbol.container_name ? `${symbol.container_name} - ` : "";
  return `${container}${relativePath}:${symbol.location.range.start_line}`;
}

function fuzzyScore(value: string, query: string) {
  const source = value.toLowerCase();
  const needle = query.toLowerCase();
  if (!needle) return 1;
  if (source.includes(needle)) return needle.length * 10;

  let score = 0;
  let sourceIndex = 0;
  for (const char of needle) {
    const foundAt = source.indexOf(char, sourceIndex);
    if (foundAt === -1) return 0;
    score += foundAt === sourceIndex ? 4 : 1;
    sourceIndex = foundAt + 1;
  }
  return score;
}

function formatLanguageServerSummary(servers: Array<{ status: string }>, t: TranslateFn) {
  if (servers.length === 0) return t("command.languageServers.none");
  const available = servers.filter((server) => server.status === "available").length;
  return t("command.languageServers.available", { available, total: servers.length });
}

function formatExtensionCommandExecutionError(report: ExtensionCommandExecution, t: TranslateFn) {
  const reason = report.reason ?? t("command.error.failed");
  return `${t("command.error.failed")} [${report.phase}]: ${reason}`;
}

function readErrorMessage(error: unknown, t: TranslateFn) {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return t("command.error.failed");
}
