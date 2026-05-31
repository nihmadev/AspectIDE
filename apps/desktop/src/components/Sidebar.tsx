import * as Dialog from "@radix-ui/react-dialog";
import {
  AlertTriangle,
  Bug,
  CaseSensitive,
  ChevronDown,
  ChevronRight,
  Copy,
  FilePlus2,
  Files,
  Folder,
  FolderOpen,
  FolderPlus,
  GitBranch,
  Loader2,
  Package,
  Play,
  Pin,
  Regex,
  RefreshCw,
  Replace,
  ReplaceAll,
  Search,
  SearchX,
  ShieldAlert,
  Trash2,
  WholeWord,
} from "lucide-react";
import type { CSSProperties, DragEvent, KeyboardEvent as ReactKeyboardEvent, MouseEvent, ReactNode } from "react";
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { useEditorCloseGuard } from "./EditorCloseGuard";
import { closedDocumentIdsForAllDocuments } from "../lib/editorCloseTargets";
import { fileIconForName } from "../lib/fileIcons";
import { displayPath, joinPath, normalizePath, parentPath } from "../lib/fileTree";
import { useLuxStore, type Activity } from "../lib/store";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
import type { MessageKey } from "../lib/i18n";
import { luxCommands } from "../lib/tauri";
import type { DebugAdapterInfo, DebugConfiguration, DebugWorkspaceInfo, ExtensionInfo, FsEntry, GitFileStatus, LspWorkspaceEdit, SearchHit, SearchOptions, WorkspaceInfo } from "../lib/types";

const explorerActivities: Array<{ id: Activity; label: MessageKey; shortcut: string; icon: ReactNode }> = [
  { id: "explorer", label: "sidebar.explorer.title", shortcut: "Ctrl+Shift+E", icon: <Files size={18} strokeWidth={1.8} /> },
  { id: "search", label: "sidebar.search.title", shortcut: "Ctrl+Shift+F", icon: <Search size={18} strokeWidth={1.8} /> },
  { id: "git", label: "sidebar.git.title", shortcut: "Ctrl+Shift+G", icon: <GitBranch size={18} strokeWidth={1.8} /> },
  { id: "runDebug", label: "sidebar.runDebug.title", shortcut: "Ctrl+Shift+D", icon: <Bug size={18} strokeWidth={1.8} /> },
  { id: "extensions", label: "sidebar.extensions.title", shortcut: "Ctrl+Shift+X", icon: <Package size={18} strokeWidth={1.8} /> },
];

const pinnedExplorerActivityIds = new Set<Activity>(["explorer", "search"]);

type PendingCreate = {
  kind: "file" | "directory";
  parentPath: string;
};

type PendingRename = {
  entry: FsEntry;
};

type PendingDelete = {
  entry: FsEntry;
};

type ClipboardEntry = {
  entry: FsEntry;
  operation: "copy" | "cut";
};

type DraggedEntry = {
  entry: FsEntry;
};

type ContextMenuState = {
  entry: FsEntry;
  source: "row" | "blank";
  x: number;
  y: number;
};

type TreeAction = {
  label: string;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  onClick: () => void;
};

type PanelAction = {
  label: string;
  icon: ReactNode;
  onClick: () => void;
  disabled?: boolean;
};

export function Sidebar({ side = "left" }: { side?: "left" | "right" }) {
  const activeActivity = useLuxStore((state) => state.activeActivity);

  return (
    <aside className="sidebar" data-side={side}>
      <div className="sidebar-surface">
        <SidebarViewSwitcher />
        {activeActivity === "explorer" && <ExplorerPanel />}
        {activeActivity === "search" && <SearchPanel />}
        {activeActivity === "git" && <GitPanel />}
        {activeActivity === "runDebug" && <RunDebugPanel />}
        {activeActivity === "extensions" && <ExtensionsPanel />}
      </div>
    </aside>
  );
}

function SidebarViewSwitcher() {
  const { t } = useTranslation();
  const activeActivity = useLuxStore((state) => state.activeActivity);
  const setActiveActivity = useLuxStore((state) => state.setActiveActivity);
  const [menuOpen, setMenuOpen] = useState(false);
  const switcherRef = useRef<HTMLDivElement | null>(null);
  const pinnedActivities = explorerActivities.filter((activity) => pinnedExplorerActivityIds.has(activity.id));
  const overflowActivities = explorerActivities.filter((activity) => !pinnedExplorerActivityIds.has(activity.id));
  const overflowActive = overflowActivities.some((activity) => activity.id === activeActivity);

  useEffect(() => {
    if (!menuOpen) return;
    const closeIfOutside = (event: PointerEvent) => {
      if (switcherRef.current?.contains(event.target as Node)) return;
      setMenuOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };
    window.addEventListener("pointerdown", closeIfOutside);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", closeIfOutside);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [menuOpen]);

  return (
    <div className="sidebar-view-switcher" ref={switcherRef}>
      <nav className="explorer-activity-strip" aria-label={t("sidebar.aria.panelViews")}>
        {pinnedActivities.map((activity) => (
          <button
            className="explorer-activity-button"
            data-active={activeActivity === activity.id}
            type="button"
            aria-label={t(activity.label)}
            title={t(activity.label)}
            key={activity.id}
            onClick={() => setActiveActivity(activity.id)}
          >
            {activity.icon}
          </button>
        ))}
        <button
          className="explorer-activity-button view-menu-toggle"
          data-active={menuOpen || overflowActive}
          type="button"
          aria-label={t("sidebar.views.more")}
          aria-expanded={menuOpen}
          title={t("sidebar.views.more")}
          onClick={() => setMenuOpen((open) => !open)}
        >
          <ChevronDown size={15} strokeWidth={1.9} />
        </button>
      </nav>
      {menuOpen && (
        <div className="view-switcher-menu">
          {overflowActivities.map((activity) => (
            <button
              className="view-switcher-item"
              data-active={activeActivity === activity.id}
              type="button"
              key={activity.id}
              onClick={() => {
                setActiveActivity(activity.id);
                setMenuOpen(false);
              }}
            >
              {activity.icon}
              <span>{t(activity.label)}</span>
              <kbd>{activity.shortcut}</kbd>
              <Pin size={13} />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function PanelHeader({ actions = [], title }: { actions?: PanelAction[]; title: string }) {
  return (
    <div className="panel-header">
      <span>{title}</span>
      {actions.length > 0 && (
        <div className="panel-actions">
          {actions.map((action) => (
            <button
              className="icon-button compact"
              type="button"
              aria-label={action.label}
              title={action.label}
              disabled={action.disabled}
              key={action.label}
              onClick={action.onClick}
            >
              {action.icon}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function ExplorerPanel() {
  const { t } = useTranslation();
  const workspace = useLuxStore((state) => state.workspace);
  const workspaceFolders = useLuxStore((state) => state.workspaceFolders);
  const setWorkspace = useLuxStore((state) => state.setWorkspace);
  const addWorkspaceFolder = useLuxStore((state) => state.addWorkspaceFolder);
  const removeWorkspaceFolder = useLuxStore((state) => state.removeWorkspaceFolder);
  const fileEntries = useLuxStore((state) => state.fileEntries);
  const fileTreeDirectories = useLuxStore((state) => state.fileTreeDirectories);
  const fileTreeLoading = useLuxStore((state) => state.fileTreeLoading);
  const fileTreeError = useLuxStore((state) => state.fileTreeError);
  const setFileEntries = useLuxStore((state) => state.setFileEntries);
  const setFileTreeDirectories = useLuxStore((state) => state.setFileTreeDirectories);
  const setFileTreeLoading = useLuxStore((state) => state.setFileTreeLoading);
  const setFileTreeError = useLuxStore((state) => state.setFileTreeError);
  const activeDocument = useLuxStore((state) => state.openDocuments.find((document) => document.id === state.activeDocumentId) ?? null);
  const openDocuments = useLuxStore((state) => state.openDocuments);
  const closeDocument = useLuxStore((state) => state.closeDocument);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);
  const openBottomPanel = useLuxStore((state) => state.openBottomPanel);
  const setCommandPaletteOpen = useLuxStore((state) => state.setCommandPaletteOpen);
  const setSettingsOpen = useLuxStore((state) => state.setSettingsOpen);
  const terminal = useLuxStore((state) => state.terminal);
  const setTerminal = useLuxStore((state) => state.setTerminal);
  const explorerExpandedPaths = useLuxStore((state) => state.explorerExpandedPaths);
  const setExplorerExpandedPaths = useLuxStore((state) => state.setExplorerExpandedPaths);
  const ensureExplorerExpandedPath = useLuxStore((state) => state.ensureExplorerExpandedPath);
  const toggleExplorerExpandedPath = useLuxStore((state) => state.toggleExplorerExpandedPath);

  const [pendingCreate, setPendingCreate] = useState<PendingCreate | null>(null);
  const [pendingRename, setPendingRename] = useState<PendingRename | null>(null);
  const [pendingDelete, setPendingDelete] = useState<PendingDelete | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [clipboardEntry, setClipboardEntry] = useState<ClipboardEntry | null>(null);
  const [draggedEntry, setDraggedEntry] = useState<DraggedEntry | null>(null);
  const [dropTargetPath, setDropTargetPath] = useState<string | null>(null);
  const [selectedEntryPath, setSelectedEntryPath] = useState<string | null>(null);
  const [deleteInFlight, setDeleteInFlight] = useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const draggedEntryRef = useRef<DraggedEntry | null>(null);
  const { requestCloseDocuments } = useEditorCloseGuard();
  const expandedPaths = useMemo(() => new Set(explorerExpandedPaths), [explorerExpandedPaths]);

  const workspaceRoots = useMemo(
    () => (workspaceFolders.length > 0 ? workspaceFolders : workspace ? [workspace] : []),
    [workspace, workspaceFolders],
  );
  const primaryRoot = workspaceRoots[0] ?? workspace;
  const rootPath = primaryRoot?.root ?? "";
  const rootKey = normalizePath(rootPath);
  const rootEntry: FsEntry | null = primaryRoot
    ? { name: primaryRoot.name, path: primaryRoot.root, kind: "directory", size: 0, modified_at: null, is_hidden: false }
    : null;
  const rootEntries = fileTreeDirectories[rootKey] ?? fileEntries;
  const rootPendingCreate = pendingCreate && normalizePath(pendingCreate.parentPath) === rootKey ? pendingCreate : null;

  const openFileMutation = useMutation({
    mutationFn: luxCommands.editorOpenFile,
    onSuccess: upsertDocument,
    onError: (error) => setOperationError(readErrorMessage(error, t)),
  });

  const refreshTree = useCallback(async () => {
    if (!workspace) return;
    setFileTreeLoading(true);
    setFileTreeError(null);
    setOperationError(null);
    try {
      const pairs = await Promise.all(workspaceRoots.map(async (folder) => [folder, await luxCommands.fsReadTree(folder.root)] as const));
      const directories = pairs.reduce<Record<string, FsEntry[]>>((merged, [folder, entries]) => ({
        ...merged,
        ...buildDirectories(folder.root, entries),
      }), {});
      setFileTreeDirectories(directories);
      setFileEntries(directories[normalizePath(workspace.root)] ?? []);
    } catch (error) {
      setFileTreeError(readErrorMessage(error, t));
    } finally {
      setFileTreeLoading(false);
    }
  }, [setFileEntries, setFileTreeDirectories, setFileTreeError, setFileTreeLoading, t, workspace, workspaceRoots]);

  const loadWorkspaceRoot = useCallback(async (folder: typeof workspaceRoots[number]) => {
    setFileTreeLoading(true);
    setFileTreeError(null);
    setOperationError(null);
    try {
      const entries = await luxCommands.fsReadTree(folder.root);
      const directories = buildDirectories(folder.root, entries);
      setFileTreeDirectories({ ...useLuxStore.getState().fileTreeDirectories, ...directories });
      if (workspace?.root === folder.root) setFileEntries(directories[normalizePath(folder.root)] ?? []);
      ensureExplorerExpandedPath(folder.root);
    } catch (error) {
      setFileTreeError(readErrorMessage(error, t));
    } finally {
      setFileTreeLoading(false);
    }
  }, [ensureExplorerExpandedPath, setFileEntries, setFileTreeDirectories, setFileTreeError, setFileTreeLoading, t, workspace?.root]);

  useEffect(() => {
    setPendingCreate(null);
    setPendingRename(null);
    setPendingDelete(null);
    setContextMenu(null);
    setDraggedEntry(null);
    setDropTargetPath(null);
    setSelectedEntryPath(null);
    setDeleteInFlight(false);
    setOperationError(null);
  }, [workspace?.root]);

  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [contextMenu]);

  const toggleDirectory = useCallback((entry: FsEntry) => {
    toggleExplorerExpandedPath(entry.path);
  }, [toggleExplorerExpandedPath]);

  const createEntry = useCallback(
    async (parent: string, name: string, kind: PendingCreate["kind"]) => {
      const trimmed = name.trim();
      if (!trimmed || !workspace) {
        setPendingCreate(null);
        return;
      }

      const targetPath = joinPath(parent, trimmed);
      try {
        setOperationError(null);
        if (kind === "file") await luxCommands.fsCreateFile(targetPath);
        else await luxCommands.fsCreateDir(targetPath);
        await refreshTree();
        ensureExplorerExpandedPath(parent);
        if (kind === "file") openFileMutation.mutate(targetPath);
      } catch (error) {
        setOperationError(readErrorMessage(error, t));
      } finally {
        setPendingCreate(null);
      }
    },
    [ensureExplorerExpandedPath, openFileMutation, refreshTree, t, workspace],
  );

  const renameEntry = useCallback(
    async (entry: FsEntry, name: string) => {
      const trimmed = name.trim();
      if (!trimmed || trimmed === entry.name) {
        setPendingRename(null);
        return;
      }

      try {
        setOperationError(null);
        await luxCommands.fsRename(entry.path, joinPath(parentPath(entry.path), trimmed));
        await refreshTree();
      } catch (error) {
        setOperationError(readErrorMessage(error, t));
      } finally {
        setPendingRename(null);
      }
    },
    [refreshTree, t],
  );

  const deleteEntry = useCallback(
    async (entry: FsEntry) => {
      if (deleteInFlight) return;
      try {
        setDeleteInFlight(true);
        setPendingDelete(null);
        setOperationError(null);
        await luxCommands.fsDelete(entry.path);
        for (const document of useLuxStore.getState().openDocuments) {
          if (document.path && pathIsInsideEntry(entry.path, document.path)) closeDocument(document.id);
        }
        await refreshTree();
        setSelectedEntryPath(null);
        setClipboardEntry((current) => current && pathIsInsideEntry(entry.path, current.entry.path) ? null : current);
        setPendingDelete(null);
      } catch (error) {
        setOperationError(readErrorMessage(error, t));
      } finally {
        setDeleteInFlight(false);
      }
    },
    [closeDocument, deleteInFlight, refreshTree, t],
  );

  const confirmDeleteEntry = useCallback((entry: FsEntry) => {
    if (deleteInFlight) return;
    const affectedDocumentIds = openDocuments
      .filter((document) => document.path && pathIsInsideEntry(entry.path, document.path))
      .map((document) => document.id);
    setPendingDelete(null);
    requestCloseDocuments(
      affectedDocumentIds,
      () => void deleteEntry(entry),
      {
        title: t("sidebar.explorer.delete.saveChangesTitle", { name: entry.name }),
        message: deleteOpenDocumentsMessage(entry, affectedDocumentIds.length, t),
      },
    );
  }, [deleteEntry, deleteInFlight, openDocuments, requestCloseDocuments, t]);

  const requestDeleteEntry = useCallback((entry: FsEntry) => {
    setContextMenu(null);
    setSelectedEntryPath(normalizePath(entry.path));
    setOperationError(null);
    setPendingDelete({ entry });
  }, []);

  const pasteInto = useCallback(
    async (targetDirectory: string) => {
      if (!clipboardEntry) return;
      const destination = uniqueDestinationPath(targetDirectory, clipboardEntry.entry.name, fileTreeDirectories, t);
      try {
        setOperationError(null);
        if (clipboardEntry.operation === "cut") {
          await luxCommands.fsRename(clipboardEntry.entry.path, destination);
          setClipboardEntry(null);
        } else {
          await luxCommands.fsCopy(clipboardEntry.entry.path, destination);
        }
        await refreshTree();
        ensureExplorerExpandedPath(targetDirectory);
      } catch (error) {
        setOperationError(readErrorMessage(error, t));
      }
    },
    [clipboardEntry, ensureExplorerExpandedPath, fileTreeDirectories, refreshTree, t],
  );

  const moveEntryInto = useCallback(
    async (entry: FsEntry, targetDirectory: string) => {
      if (entry.kind === "directory") ensureExplorerExpandedPath(targetDirectory);
      const validationError = validateMoveTarget(entry, targetDirectory, fileTreeDirectories, t);
      if (validationError) {
        if (validationError !== "same-directory") setOperationError(validationError);
        draggedEntryRef.current = null;
        setDraggedEntry(null);
        setDropTargetPath(null);
        return;
      }

      try {
        setOperationError(null);
        await luxCommands.fsRename(entry.path, joinPath(targetDirectory, entry.name));
        await refreshTree();
        ensureExplorerExpandedPath(targetDirectory);
      } catch (error) {
        setOperationError(readErrorMessage(error, t));
      } finally {
        draggedEntryRef.current = null;
        setDraggedEntry(null);
        setDropTargetPath(null);
      }
    },
    [ensureExplorerExpandedPath, fileTreeDirectories, refreshTree, t],
  );

  const copyAbsolutePath = useCallback(async (entry: FsEntry) => {
    try {
      await luxCommands.clipboardWriteText(entry.path);
    } catch (error) {
      setOperationError(readErrorMessage(error, t));
    }
  }, [t]);

  const copyRelativePath = useCallback(async (entry: FsEntry) => {
    if (!workspace) return;
    try {
      await luxCommands.clipboardWriteText(relativePath(workspace.root, entry.path));
    } catch (error) {
      setOperationError(readErrorMessage(error, t));
    }
  }, [t, workspace]);

  const openEntryTerminal = useCallback(
    async (entry: FsEntry) => {
      try {
        const cwd = entry.kind === "directory" ? entry.path : parentPath(entry.path);
        openBottomPanel("terminal");
        if (terminal) await luxCommands.terminalClose(terminal.id).catch(() => undefined);
        const createdTerminal = await luxCommands.terminalCreate(undefined, cwd);
        setTerminal(createdTerminal);
      } catch (error) {
        setOperationError(readErrorMessage(error, t));
      }
    },
    [openBottomPanel, setTerminal, t, terminal],
  );

  const addFolderToWorkspace = useCallback(async () => {
    try {
      const picked = await luxCommands.workspacePickFolder();
      if (!picked) return;
      addWorkspaceFolder(picked);
      await loadWorkspaceRoot(picked);
    } catch (error) {
      setOperationError(readErrorMessage(error, t));
    }
  }, [addWorkspaceFolder, loadWorkspaceRoot, t]);

  const removeFolderFromWorkspace = useCallback((root: string) => {
    requestCloseDocuments(closedDocumentIdsForAllDocuments(openDocuments), () => {
      const remainingFolders = workspaceRoots.filter((folder) => normalizePath(folder.root) !== normalizePath(root));
      removeWorkspaceFolder(root);
      if (remainingFolders.length === 0) {
        void luxCommands.workspaceClose().then(() => setWorkspace(null)).catch(() => setWorkspace(null));
        return;
      }
      if (workspace && normalizePath(workspace.root) === normalizePath(root)) {
        setWorkspace(remainingFolders[0]);
      }
      const normalizedRoot = normalizePath(root);
      setFileTreeDirectories(Object.fromEntries(Object.entries(useLuxStore.getState().fileTreeDirectories).filter(([key]) => key !== normalizedRoot && !key.startsWith(`${normalizedRoot}/`))));
    }, { title: t("sidebar.explorer.removeWorkspaceFolder.saveChangesTitle") });
  }, [openDocuments, removeWorkspaceFolder, requestCloseDocuments, setFileTreeDirectories, setWorkspace, t, workspace, workspaceRoots]);

  const startEntryDrag = useCallback((entry: FsEntry) => {
    const nextDraggedEntry = { entry };
    draggedEntryRef.current = nextDraggedEntry;
    setDraggedEntry(nextDraggedEntry);
    setDropTargetPath(null);
    setOperationError(null);
  }, []);

  const endEntryDrag = useCallback(() => {
    draggedEntryRef.current = null;
    setDraggedEntry(null);
    setDropTargetPath(null);
  }, []);

  const dragOverDirectory = useCallback(
    (targetDirectory: string) => {
      const currentDraggedEntry = draggedEntryRef.current ?? draggedEntry;
      if (!currentDraggedEntry || validateMoveTarget(currentDraggedEntry.entry, targetDirectory, fileTreeDirectories, t)) {
        setDropTargetPath(null);
        return false;
      }
      setDropTargetPath(normalizePath(targetDirectory));
      ensureExplorerExpandedPath(targetDirectory);
      return true;
    },
    [draggedEntry, ensureExplorerExpandedPath, fileTreeDirectories, t],
  );

  const dragLeaveDirectory = useCallback((targetDirectory: string) => {
    setDropTargetPath((current) => current === normalizePath(targetDirectory) ? null : current);
  }, []);

  const dropEntryIntoDirectory = useCallback(
    async (targetDirectory: string) => {
      const currentDraggedEntry = draggedEntryRef.current ?? draggedEntry;
      if (!currentDraggedEntry) return;
      await moveEntryInto(currentDraggedEntry.entry, targetDirectory);
    },
    [draggedEntry, moveEntryInto],
  );

  const selectedEntry = useMemo(
    () => selectedEntryPath ? findEntryByNormalizedPath(fileTreeDirectories, selectedEntryPath) : null,
    [fileTreeDirectories, selectedEntryPath],
  );

  useEffect(() => {
    if (!selectedEntryPath || selectedEntry) return;
    setSelectedEntryPath(null);
  }, [selectedEntry, selectedEntryPath]);

  const handleExplorerKeyDown = useCallback((event: ReactKeyboardEvent) => {
    if (event.key !== "Delete" || !selectedEntry) return;
    if (pendingCreate || pendingRename || pendingDelete) return;
    event.preventDefault();
    event.stopPropagation();
    requestDeleteEntry(selectedEntry);
  }, [pendingCreate, pendingDelete, pendingRename, requestDeleteEntry, selectedEntry]);

  const contextActions = useMemo(() => {
    if (!contextMenu || !workspace) return [];
    const entry = contextMenu.entry;
    const targetDirectory = entry.kind === "directory" ? entry.path : parentPath(entry.path);
    if (contextMenu.source === "blank") {
      return [
        [
          { label: t("sidebar.explorer.contextMenu.newFile"), onClick: () => { ensureExplorerExpandedPath(targetDirectory); setPendingCreate({ kind: "file", parentPath: targetDirectory }); } },
          { label: t("sidebar.explorer.contextMenu.newFolder"), onClick: () => { ensureExplorerExpandedPath(targetDirectory); setPendingCreate({ kind: "directory", parentPath: targetDirectory }); } },
          { label: t("sidebar.explorer.contextMenu.revealInFileExplorer"), shortcut: "Shift+Alt+R", onClick: () => void luxCommands.fsRevealInFileExplorer(targetDirectory).catch((error) => setOperationError(readErrorMessage(error, t))) },
          { label: t("sidebar.explorer.contextMenu.openInIntegratedTerminal"), onClick: () => void openEntryTerminal(entry) },
        ],
        [
          { label: t("sidebar.explorer.contextMenu.addFolderToWorkspace"), onClick: () => void addFolderToWorkspace() },
          { label: t("sidebar.explorer.contextMenu.openFolderSettings"), onClick: () => setSettingsOpen(true) },
          { label: t("sidebar.explorer.contextMenu.removeFolderFromWorkspace"), onClick: () => removeFolderFromWorkspace(targetDirectory) },
        ],
        [
          { label: t("sidebar.explorer.contextMenu.findInFolder"), shortcut: "Shift+Alt+F", onClick: () => { useLuxStore.getState().setActiveActivity("search"); setCommandPaletteOpen(true); } },
        ],
        [
          { label: t("common.paste"), shortcut: "Ctrl+V", disabled: !clipboardEntry, onClick: () => void pasteInto(targetDirectory) },
        ],
        [
          { label: t("sidebar.explorer.contextMenu.copyPath"), shortcut: "Shift+Alt+C", onClick: () => void copyAbsolutePath(entry) },
          { label: t("sidebar.explorer.contextMenu.copyRelativePath"), shortcut: "Ctrl+M Ctrl+Shift+C", onClick: () => void copyRelativePath(entry) },
        ],
      ] satisfies TreeAction[][];
    }

    return [
      [
        { label: t("sidebar.explorer.contextMenu.newFile"), onClick: () => { ensureExplorerExpandedPath(targetDirectory); setPendingCreate({ kind: "file", parentPath: targetDirectory }); } },
        { label: t("sidebar.explorer.contextMenu.newFolder"), onClick: () => { ensureExplorerExpandedPath(targetDirectory); setPendingCreate({ kind: "directory", parentPath: targetDirectory }); } },
        { label: t("sidebar.explorer.contextMenu.revealInFileExplorer"), shortcut: "Shift+Alt+R", onClick: () => void luxCommands.fsRevealInFileExplorer(entry.path).catch((error) => setOperationError(readErrorMessage(error, t))) },
        { label: t("sidebar.explorer.contextMenu.openInIntegratedTerminal"), onClick: () => void openEntryTerminal(entry) },
      ],
      [
        { label: t("sidebar.explorer.contextMenu.findInFolder"), shortcut: "Shift+Alt+F", onClick: () => { useLuxStore.getState().setActiveActivity("search"); setCommandPaletteOpen(true); } },
      ],
      [
        { label: t("common.cut"), shortcut: "Ctrl+X", onClick: () => setClipboardEntry({ entry, operation: "cut" }) },
        { label: t("common.copy"), shortcut: "Ctrl+C", onClick: () => setClipboardEntry({ entry, operation: "copy" }) },
        { label: t("common.paste"), shortcut: "Ctrl+V", disabled: !clipboardEntry, onClick: () => void pasteInto(targetDirectory) },
      ],
      [
        { label: t("sidebar.explorer.contextMenu.copyPath"), shortcut: "Shift+Alt+C", onClick: () => void copyAbsolutePath(entry) },
        { label: t("sidebar.explorer.contextMenu.copyRelativePath"), shortcut: "Ctrl+M Ctrl+Shift+C", onClick: () => void copyRelativePath(entry) },
      ],
      [
        { label: t("common.renameWithEllipsis"), shortcut: "F2", onClick: () => setPendingRename({ entry }) },
        { label: t("common.delete"), shortcut: "Delete", danger: true, onClick: () => requestDeleteEntry(entry) },
      ],
    ] satisfies TreeAction[][];
  }, [addFolderToWorkspace, clipboardEntry, contextMenu, copyAbsolutePath, copyRelativePath, ensureExplorerExpandedPath, openEntryTerminal, pasteInto, removeFolderFromWorkspace, requestDeleteEntry, setCommandPaletteOpen, setSettingsOpen, t, workspace, workspaceRoots.length]);

  const actions = workspace
    ? [
        {
          label: t("sidebar.explorer.actions.newFile"),
          icon: <FilePlus2 size={15} />,
          onClick: () => setPendingCreate({ kind: "file", parentPath: workspace.root }),
        },
        {
          label: t("sidebar.explorer.actions.newFolder"),
          icon: <FolderPlus size={15} />,
          onClick: () => setPendingCreate({ kind: "directory", parentPath: workspace.root }),
        },
        {
          label: t("sidebar.explorer.actions.refresh"),
          icon: fileTreeLoading ? <Loader2 size={14} className="spin-icon" /> : <RefreshCw size={14} />,
          onClick: () => void refreshTree(),
          disabled: fileTreeLoading,
        },
        {
          label: t("sidebar.explorer.actions.collapseAll"),
          icon: <Copy size={14} />,
          onClick: () => setExplorerExpandedPaths([rootKey]),
        },
      ]
    : [];

  if (!workspace) {
    return <div className="panel-content empty-panel"><PanelHeader title={t("sidebar.explorer.title")} /><span>{t("sidebar.explorer.empty.noWorkspace")}</span></div>;
  }

  return (
    <div className="panel-content explorer-panel-content">
      <section
        className="tree-section"
        onContextMenu={(event) => {
          event.preventDefault();
          if (rootEntry) setContextMenu({ entry: rootEntry, source: "blank", x: event.clientX, y: event.clientY });
        }}
      >
        <div className="tree-root-header">
          <button
            className="tree-section-title"
            type="button"
            data-drop-target={dropTargetPath === rootKey}
            onClick={() => toggleExplorerExpandedPath(rootKey)}
            onDragEnter={(event) => {
              if (!rootEntry || !dragOverDirectory(rootEntry.path)) return;
              event.preventDefault();
            }}
            onDragOver={(event) => {
              if (!rootEntry || !dragOverDirectory(rootEntry.path)) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
            }}
            onDrop={(event) => {
              if (!rootEntry) return;
              event.preventDefault();
              event.stopPropagation();
              void dropEntryIntoDirectory(rootEntry.path);
            }}
            onContextMenu={(event) => {
              if (!rootEntry) return;
              event.preventDefault();
              setContextMenu({ entry: rootEntry, source: "blank", x: event.clientX, y: event.clientY });
            }}
          >
            {expandedPaths.has(rootKey) ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
            <span>{formatWorkspaceRootLabel(workspace.name)}</span>
          </button>
          <div className="panel-actions tree-root-actions">
            {actions.map((action) => (
              <button className="icon-button compact" type="button" aria-label={action.label} title={action.label} disabled={action.disabled} key={action.label} onClick={action.onClick}>
                {action.icon}
              </button>
            ))}
          </div>
        </div>
        <div
          className="file-tree"
          role="tree"
          tabIndex={0}
          onKeyDown={handleExplorerKeyDown}
          onContextMenu={(event) => { event.preventDefault(); if (rootEntry) setContextMenu({ entry: rootEntry, source: "blank", x: event.clientX, y: event.clientY }); }}
          onDragOver={(event) => {
            if (!rootEntry || !dragOverDirectory(rootEntry.path)) return;
            event.preventDefault();
            event.stopPropagation();
            event.dataTransfer.dropEffect = "move";
          }}
          onDrop={(event) => {
            if (!rootEntry) return;
            event.preventDefault();
            event.stopPropagation();
            void dropEntryIntoDirectory(rootEntry.path);
          }}
        >
          {fileTreeError && <TreeMessage depth={0} tone="error" text={fileTreeError} />}
          {operationError && <TreeMessage depth={0} tone="error" text={operationError} />}
          {rootPendingCreate && <CreateRow create={createEntry} onCancel={() => setPendingCreate(null)} pendingCreate={rootPendingCreate} />}
          {workspaceRoots.map((folder) => {
            const folderKey = normalizePath(folder.root);
            const folderEntries = fileTreeDirectories[folderKey] ?? (folder.root === rootPath ? rootEntries : []);
            return (
              <Fragment key={folder.root}>
                {workspaceRoots.length > 1 && (
                  <button
                    className="tree-section-title workspace-folder-title"
                    type="button"
                    data-drop-target={dropTargetPath === folderKey}
                    onClick={() => toggleExplorerExpandedPath(folderKey)}
                    onDragEnter={(event) => {
                      if (!dragOverDirectory(folder.root)) return;
                      event.preventDefault();
                    }}
                    onDragOver={(event) => {
                      if (!dragOverDirectory(folder.root)) return;
                      event.preventDefault();
                      event.stopPropagation();
                      event.dataTransfer.dropEffect = "move";
                    }}
                    onDrop={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      void dropEntryIntoDirectory(folder.root);
                    }}
                    onContextMenu={(event) => {
                      event.preventDefault();
                      setContextMenu({ entry: workspaceToEntry(folder), source: "blank", x: event.clientX, y: event.clientY });
                    }}
                  >
                    {expandedPaths.has(folderKey) ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
                    <span>{folder.name}</span>
                  </button>
                )}
                {expandedPaths.has(folderKey) && folderEntries.map((entry) => (
                  <TreeEntry
                    activePath={activeDocument?.path ?? null}
                    clipboardEntry={clipboardEntry}
                    createEntry={createEntry}
                    depth={workspaceRoots.length > 1 ? 1 : 0}
                    directories={fileTreeDirectories}
                    draggedEntry={draggedEntry}
                    dragLeaveDirectory={dragLeaveDirectory}
                    dropEntryIntoDirectory={dropEntryIntoDirectory}
                    dropTargetPath={dropTargetPath}
                    dragOverDirectory={dragOverDirectory}
                    endEntryDrag={endEntryDrag}
                    entry={entry}
                    expandedPaths={expandedPaths}
                    key={entry.path}
                    openFile={(path) => openFileMutation.mutate(path)}
                    pendingCreate={pendingCreate}
                    pendingRename={pendingRename}
                    renameEntry={renameEntry}
                    requestDeleteEntry={requestDeleteEntry}
                    setContextMenu={setContextMenu}
                    selectedEntryPath={selectedEntryPath}
                    setSelectedEntryPath={setSelectedEntryPath}
                    setPendingCreate={setPendingCreate}
                    setPendingRename={setPendingRename}
                    startEntryDrag={startEntryDrag}
                    toggleDirectory={toggleDirectory}
                  />
                ))}
              </Fragment>
            );
          })}
        </div>
      </section>

      {contextMenu && <TreeContextMenu groups={contextActions} x={contextMenu.x} y={contextMenu.y} onClose={() => setContextMenu(null)} />}
      <DeleteEntryDialog
        directories={fileTreeDirectories}
        pendingDelete={pendingDelete}
        deleting={deleteInFlight}
        onCancel={() => setPendingDelete(null)}
        onConfirm={confirmDeleteEntry}
      />
    </div>
  );
}

function TreeEntry({
  activePath,
  clipboardEntry,
  createEntry,
  depth,
  directories,
  draggedEntry,
  dragLeaveDirectory,
  dropEntryIntoDirectory,
  dropTargetPath,
  dragOverDirectory,
  endEntryDrag,
  entry,
  expandedPaths,
  openFile,
  pendingCreate,
  pendingRename,
  renameEntry,
  requestDeleteEntry,
  setContextMenu,
  selectedEntryPath,
  setSelectedEntryPath,
  setPendingCreate,
  setPendingRename,
  startEntryDrag,
  toggleDirectory,
}: {
  activePath: string | null;
  clipboardEntry: ClipboardEntry | null;
  createEntry: (parentPath: string, name: string, kind: PendingCreate["kind"]) => Promise<void>;
  depth: number;
  directories: Record<string, FsEntry[]>;
  draggedEntry: DraggedEntry | null;
  dragLeaveDirectory: (targetDirectory: string) => void;
  dropEntryIntoDirectory: (targetDirectory: string) => Promise<void>;
  dropTargetPath: string | null;
  dragOverDirectory: (targetDirectory: string) => boolean;
  endEntryDrag: () => void;
  entry: FsEntry;
  expandedPaths: Set<string>;
  openFile: (path: string) => void;
  pendingCreate: PendingCreate | null;
  pendingRename: PendingRename | null;
  renameEntry: (entry: FsEntry, name: string) => Promise<void>;
  requestDeleteEntry: (entry: FsEntry) => void;
  setContextMenu: (contextMenu: ContextMenuState | null) => void;
  selectedEntryPath: string | null;
  setSelectedEntryPath: (path: string | null) => void;
  setPendingCreate: (pendingCreate: PendingCreate | null) => void;
  setPendingRename: (pendingRename: PendingRename | null) => void;
  startEntryDrag: (entry: FsEntry) => void;
  toggleDirectory: (entry: FsEntry) => void;
}) {
  const key = normalizePath(entry.path);
  const isDirectory = entry.kind === "directory";
  const isExpanded = expandedPaths.has(key);
  const children = directories[key] ?? [];
  const hasChildren = children.length > 0;
  const pendingParentKey = pendingCreate ? normalizePath(pendingCreate.parentPath) : null;
  const renameKey = pendingRename ? normalizePath(pendingRename.entry.path) : null;

  return (
    <Fragment>
      {renameKey === key ? (
        <RenameRow depth={depth} entry={entry} onCancel={() => setPendingRename(null)} rename={renameEntry} />
      ) : (
        <FileRow
          activePath={activePath}
          clipboardEntry={clipboardEntry}
          depth={depth}
          entry={entry}
          expanded={isExpanded}
          hasChildren={hasChildren}
          isDragging={draggedEntry ? normalizePath(draggedEntry.entry.path) === key : false}
          isDropTarget={dropTargetPath === key}
          isSelected={selectedEntryPath === key}
          onContextMenu={(event) => {
            event.preventDefault();
            event.stopPropagation();
            setSelectedEntryPath(key);
            setContextMenu({ entry, source: "row", x: event.clientX, y: event.clientY });
          }}
          onDelete={() => requestDeleteEntry(entry)}
          onDragEnd={endEntryDrag}
          onDragStart={(event) => {
            event.dataTransfer.effectAllowed = "move";
            event.dataTransfer.setData("text/plain", entry.path);
            startEntryDrag(entry);
          }}
          onDragEnter={isDirectory ? (event) => {
            if (!dragOverDirectory(entry.path)) return;
            event.preventDefault();
            event.stopPropagation();
            event.dataTransfer.dropEffect = "move";
          } : undefined}
          onDragOver={isDirectory ? (event) => {
            if (!dragOverDirectory(entry.path)) return;
            event.preventDefault();
            event.stopPropagation();
            event.dataTransfer.dropEffect = "move";
          } : undefined}
          onDragLeave={isDirectory ? (event) => {
            if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
            dragLeaveDirectory(entry.path);
          } : undefined}
          onDrop={isDirectory ? (event) => {
            event.preventDefault();
            event.stopPropagation();
            void dropEntryIntoDirectory(entry.path);
          } : undefined}
          onOpen={() => {
            setSelectedEntryPath(key);
            if (!isDirectory) openFile(entry.path);
            else if (hasChildren || pendingParentKey === key) toggleDirectory(entry);
          }}
        />
      )}
      {isDirectory && isExpanded && (hasChildren || pendingParentKey === key) && (
        <>
          {pendingCreate && pendingParentKey === key ? <CreateRow create={createEntry} depth={depth + 1} onCancel={() => setPendingCreate(null)} pendingCreate={pendingCreate} /> : null}
          {children.map((child) => (
            <TreeEntry
              activePath={activePath}
              clipboardEntry={clipboardEntry}
              createEntry={createEntry}
              depth={depth + 1}
              directories={directories}
              draggedEntry={draggedEntry}
              dragLeaveDirectory={dragLeaveDirectory}
              dropEntryIntoDirectory={dropEntryIntoDirectory}
              dropTargetPath={dropTargetPath}
              dragOverDirectory={dragOverDirectory}
              endEntryDrag={endEntryDrag}
              entry={child}
              expandedPaths={expandedPaths}
              key={child.path}
              openFile={openFile}
              pendingCreate={pendingCreate}
              pendingRename={pendingRename}
              renameEntry={renameEntry}
              requestDeleteEntry={requestDeleteEntry}
              setContextMenu={setContextMenu}
              selectedEntryPath={selectedEntryPath}
              setSelectedEntryPath={setSelectedEntryPath}
              setPendingCreate={setPendingCreate}
              setPendingRename={setPendingRename}
              startEntryDrag={startEntryDrag}
              toggleDirectory={toggleDirectory}
            />
          ))}
        </>
      )}
    </Fragment>
  );
}

function FileRow({
  activePath,
  clipboardEntry,
  depth,
  entry,
  expanded,
  hasChildren,
  isDragging,
  isDropTarget,
  isSelected,
  onContextMenu,
  onDelete,
  onDragEnd,
  onDragEnter,
  onDragLeave,
  onDragOver,
  onDragStart,
  onDrop,
  onOpen,
}: {
  activePath: string | null;
  clipboardEntry: ClipboardEntry | null;
  depth: number;
  entry: FsEntry;
  expanded: boolean;
  hasChildren: boolean;
  isDragging: boolean;
  isDropTarget: boolean;
  isSelected: boolean;
  onContextMenu: (event: MouseEvent<HTMLButtonElement>) => void;
  onDelete: () => void;
  onDragEnd: () => void;
  onDragEnter?: (event: DragEvent<HTMLButtonElement>) => void;
  onDragLeave?: (event: DragEvent<HTMLButtonElement>) => void;
  onDragOver?: (event: DragEvent<HTMLButtonElement>) => void;
  onDragStart: (event: DragEvent<HTMLButtonElement>) => void;
  onDrop?: (event: DragEvent<HTMLButtonElement>) => void;
  onOpen: () => void;
}) {
  const isDirectory = entry.kind === "directory";
  const iconMeta = fileIconForName(entry.name);
  const Icon = isDirectory ? (expanded ? FolderOpen : Folder) : iconMeta.Icon;
  const isCut = clipboardEntry?.operation === "cut" && normalizePath(clipboardEntry.entry.path) === normalizePath(entry.path);

  return (
    <div className="file-row-shell" style={{ "--tree-depth": depth } as CSSProperties}>
      <button
        className="file-row"
        type="button"
        draggable
        role="treeitem"
        aria-expanded={isDirectory ? expanded : undefined}
        data-active={activePath ? normalizePath(activePath) === normalizePath(entry.path) : false}
        data-cut={isCut}
        data-dragging={isDragging}
        data-drop-target={isDropTarget}
        data-selected={isSelected}
        onClick={onOpen}
        onContextMenu={onContextMenu}
        onKeyDown={(event) => {
          if (event.key !== "Delete") return;
          event.preventDefault();
          event.stopPropagation();
          onDelete();
        }}
        onDragEnd={onDragEnd}
        onDragEnter={onDragEnter}
        onDragLeave={onDragLeave}
        onDragOver={onDragOver}
        onDragStart={onDragStart}
        onDrop={onDrop}
      >
        {isDirectory && hasChildren ? <span className="tree-chevron">{expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</span> : <span className="tree-chevron" />}
        <Icon size={15} className={isDirectory ? "folder-icon" : iconMeta.className} />
        <span>{entry.name}</span>
      </button>
    </div>
  );
}

function CreateRow({
  create,
  depth = 0,
  onCancel,
  pendingCreate,
}: {
  create: (parentPath: string, name: string, kind: PendingCreate["kind"]) => Promise<void>;
  depth?: number;
  onCancel: () => void;
  pendingCreate: PendingCreate;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(pendingCreate.kind === "file" ? t("sidebar.explorer.create.defaultFileName") : t("sidebar.explorer.create.defaultFolderName"));
  const iconMeta = fileIconForName(name);
  const Icon = pendingCreate.kind === "file" ? iconMeta.Icon : Folder;

  return (
    <form
      className="create-row"
      style={{ "--tree-depth": depth } as CSSProperties}
      onSubmit={(event) => {
        event.preventDefault();
        void create(pendingCreate.parentPath, name, pendingCreate.kind);
      }}
    >
      <span className="tree-chevron" />
      <Icon size={15} className={pendingCreate.kind === "file" ? iconMeta.className : "folder-icon"} />
      <input
        autoFocus
        value={name}
        onBlur={() => {
          if (!name.trim()) onCancel();
        }}
        onChange={(event) => setName(event.target.value)}
        onFocus={(event) => event.currentTarget.select()}
        onKeyDown={(event) => {
          if (event.key === "Escape") onCancel();
        }}
        aria-label={pendingCreate.kind === "file" ? t("sidebar.explorer.aria.newFileName") : t("sidebar.explorer.aria.newFolderName")}
      />
    </form>
  );
}

function RenameRow({ depth, entry, onCancel, rename }: { depth: number; entry: FsEntry; onCancel: () => void; rename: (entry: FsEntry, name: string) => Promise<void> }) {
  const { t } = useTranslation();
  const [name, setName] = useState(entry.name);
  const iconMeta = fileIconForName(name);
  const Icon = entry.kind === "directory" ? Folder : iconMeta.Icon;

  return (
    <form
      className="create-row rename-row"
      style={{ "--tree-depth": depth } as CSSProperties}
      onSubmit={(event) => {
        event.preventDefault();
        void rename(entry, name);
      }}
    >
      <span className="tree-chevron" />
      <Icon size={15} className={entry.kind === "directory" ? "folder-icon" : iconMeta.className} />
      <input
        autoFocus
        value={name}
        onBlur={() => void rename(entry, name)}
        onChange={(event) => setName(event.target.value)}
        onFocus={(event) => event.currentTarget.select()}
        onKeyDown={(event) => {
          if (event.key === "Escape") onCancel();
        }}
        aria-label={t("sidebar.explorer.aria.renameEntry", { name: entry.name })}
      />
    </form>
  );
}

function TreeContextMenu({ groups, onClose, x, y }: { groups: TreeAction[][]; onClose: () => void; x: number; y: number }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [position, setPosition] = useState({ x, y });

  useEffect(() => {
    const menu = ref.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    setPosition({
      x: Math.min(x, window.innerWidth - rect.width - 8),
      y: Math.min(y, window.innerHeight - rect.height - 8),
    });
  }, [x, y]);

  return (
    <div className="tree-context-menu" ref={ref} style={{ left: position.x, top: position.y }} onPointerDown={(event) => event.stopPropagation()}>
      {groups.map((group, groupIndex) => (
        <div className="tree-context-menu-group" key={groupIndex}>
          {group.map((action) => (
            <button
              className="tree-context-menu-item"
              data-danger={action.danger}
              type="button"
              disabled={action.disabled}
              key={action.label}
              onClick={() => {
                if (action.disabled) return;
                action.onClick();
                onClose();
              }}
            >
              <span>{action.label}</span>
              {action.shortcut ? <kbd>{action.shortcut}</kbd> : <span />}
            </button>
          ))}
        </div>
      ))}
    </div>
  );
}

function DeleteEntryDialog({
  deleting,
  directories,
  onCancel,
  onConfirm,
  pendingDelete,
}: {
  deleting: boolean;
  directories: Record<string, FsEntry[]>;
  onCancel: () => void;
  onConfirm: (entry: FsEntry) => void;
  pendingDelete: PendingDelete | null;
}) {
  const { t } = useTranslation();
  const entry = pendingDelete?.entry ?? null;
  const isDirectory = entry?.kind === "directory";
  const childCount = entry ? countDescendants(entry.path, directories) : 0;
  const hasContents = isDirectory && childCount > 0;
  const description = entry
    ? deleteDialogDescription(entry, childCount, t)
    : t("sidebar.explorer.delete.defaultDescription");

  return (
    <Dialog.Root open={Boolean(entry)} onOpenChange={(open) => { if (!open && !deleting) onCancel(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="delete-entry-overlay" />
        <Dialog.Content className="delete-entry-dialog" aria-describedby="delete-entry-description">
          <div className="delete-entry-header">
            <span className="delete-entry-icon" data-danger={hasContents || !isDirectory}><AlertTriangle size={18} /></span>
            <div>
              <Dialog.Title>{entry ? deleteDialogTitle(entry, hasContents, t) : t("sidebar.explorer.delete.defaultTitle")}</Dialog.Title>
              <Dialog.Description id="delete-entry-description">{description}</Dialog.Description>
            </div>
          </div>
          {entry && (
            <div className="delete-entry-target" title={displayPath(entry.path)}>
              {isDirectory ? <Folder size={16} className="folder-icon" /> : <Trash2 size={16} />}
              <span>{entry.name}</span>
              <small>{displayPath(parentPath(entry.path))}</small>
            </div>
          )}
          {hasContents && (
            <div className="delete-entry-warning">
              {t("sidebar.explorer.delete.warningWithContents", { itemCount: formatItemCount(childCount, t) })}
            </div>
          )}
          <div className="delete-entry-actions">
            <button className="secondary-button" type="button" disabled={deleting} onClick={onCancel}>{t("common.cancel")}</button>
            <button className="danger-button" type="button" disabled={!entry || deleting} onClick={() => entry && onConfirm(entry)}>
              {deleting ? t("sidebar.explorer.delete.deleting") : t("common.delete")}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function TreeMessage({ depth, text, tone = "muted" }: { depth: number; text: string; tone?: "muted" | "error" }) {
  return (
    <div className="tree-message" data-tone={tone} style={{ "--tree-depth": depth } as CSSProperties}>
      {text}
    </div>
  );
}

function SearchPanel() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [replaceValue, setReplaceValue] = useState("");
  const [includePattern, setIncludePattern] = useState("");
  const [excludePattern, setExcludePattern] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  const [useRegex, setUseRegex] = useState(false);
  const [includeHidden, setIncludeHidden] = useState(false);
  const lastScheduledSearchKey = useRef("");
  const [openError, setOpenError] = useState<string | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const searchResponse = useLuxStore((state) => state.searchResponse);
  const setSearchResponse = useLuxStore((state) => state.setSearchResponse);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);
  const updateOpenDocuments = useLuxStore((state) => state.updateOpenDocuments);
  const setPendingEditorReveal = useLuxStore((state) => state.setPendingEditorReveal);
  const workspace = useLuxStore((state) => state.workspace);

  const searchOptions = useMemo<SearchOptions>(() => ({
    case_sensitive: caseSensitive,
    whole_word: wholeWord,
    use_regex: useRegex,
    include_hidden: includeHidden,
    include_globs: parseGlobList(includePattern),
    exclude_globs: parseGlobList(excludePattern),
    max_results: 500,
  }), [caseSensitive, excludePattern, includeHidden, includePattern, useRegex, wholeWord]);

  const searchMutation = useMutation({
    mutationFn: ({ options, value }: { value: string; options: SearchOptions }) => luxCommands.searchQuery(value, options),
    onSuccess: (response) => {
      setOpenError(null);
      setSearchError(null);
      setSearchResponse(response);
    },
    onError: (error) => setSearchError(readErrorMessage(error, t)),
  });

  const openSearchHitMutation = useMutation({
    mutationFn: async (hit: SearchHit) => ({ hit, document: await luxCommands.editorOpenFile(hit.path) }),
    onSuccess: ({ document, hit }) => {
      setOpenError(null);
      upsertDocument(document);
      setPendingEditorReveal({ documentId: document.id, line: hit.line, column: hit.column });
    },
    onError: (error) => setOpenError(readErrorMessage(error, t)),
  });

  const replaceMutation = useMutation({
    mutationFn: async (hits: SearchHit[]) => luxCommands.editorApplyWorkspaceEdit(buildSearchReplaceEdit(hits, query, replaceValue, { caseSensitive, useRegex })),
    onSuccess: (result) => {
      setOpenError(null);
      updateOpenDocuments(result.edited_documents);
      runSearchRef.current();
    },
    onError: (error) => setOpenError(readErrorMessage(error, t)),
  });

  const resultLabel = useMemo(() => {
    if (!searchResponse) return t("sidebar.search.noSearchExecuted");
    const truncatedIndicator = searchResponse.truncated ? "+" : "";
    return t("sidebar.search.resultCount", { count: searchResponse.hits.length, truncatedIndicator, elapsedMs: searchResponse.elapsed_ms });
  }, [searchResponse, t]);

  const groupedHits = useMemo(() => groupSearchHits(searchResponse?.hits ?? [], workspace?.root ?? null), [searchResponse?.hits, workspace?.root]);
  const canReplace = Boolean(query.trim() && searchResponse?.hits.length && !searchMutation.isPending && !replaceMutation.isPending);

  const searchKey = useMemo(() => JSON.stringify({ query, searchOptions }), [query, searchOptions]);
  const runSearchRef = useRef<() => void>(() => undefined);

  const runSearch = useCallback(() => {
    lastScheduledSearchKey.current = searchKey;
    searchMutation.mutate({ value: query, options: searchOptions });
  }, [query, searchKey, searchMutation, searchOptions]);

  useEffect(() => {
    runSearchRef.current = runSearch;
  }, [runSearch]);

  useEffect(() => {
    if (!query.trim()) return;
    if (lastScheduledSearchKey.current === searchKey) return;
    const timer = window.setTimeout(() => runSearchRef.current(), 260);
    return () => window.clearTimeout(timer);
  }, [query, searchKey]);

  return (
    <div className="panel-content utility-panel-content search-panel-content">
      <PanelHeader
        title={t("sidebar.search.title")}
        actions={[
          { label: t("sidebar.search.actions.refresh"), icon: searchMutation.isPending ? <Loader2 size={14} className="spin-icon" /> : <RefreshCw size={14} />, onClick: runSearch, disabled: searchMutation.isPending || !query.trim() },
          { label: t("sidebar.search.actions.clearResults"), icon: <SearchX size={14} />, onClick: () => setSearchResponse(null), disabled: !searchResponse },
        ]}
      />
      <form
        className="search-panel-form"
        onSubmit={(event) => {
          event.preventDefault();
          runSearch();
        }}
      >
        <div className="search-input-row">
          <Search size={14} />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("sidebar.search.title")} spellCheck={false} />
          <SearchToggle active={caseSensitive} label={t("sidebar.search.toggle.matchCase")} onClick={() => setCaseSensitive((active) => !active)}><CaseSensitive size={14} /></SearchToggle>
          <SearchToggle active={wholeWord} label={t("sidebar.search.toggle.matchWholeWord")} onClick={() => setWholeWord((active) => !active)}><WholeWord size={14} /></SearchToggle>
          <SearchToggle active={useRegex} label={t("sidebar.search.toggle.useRegularExpression")} onClick={() => setUseRegex((active) => !active)}><Regex size={14} /></SearchToggle>
        </div>
        <div className="search-input-row">
          <Replace size={14} />
          <input value={replaceValue} onChange={(event) => setReplaceValue(event.target.value)} placeholder={t("sidebar.search.replace")} spellCheck={false} />
          <button className="search-inline-button" type="button" aria-label={t("sidebar.search.replace")} title={t("sidebar.search.replace")} disabled={!canReplace} onClick={() => searchResponse?.hits[0] && replaceMutation.mutate([searchResponse.hits[0]])}><Replace size={13} /></button>
          <button className="search-inline-button" type="button" aria-label={t("sidebar.search.replaceAll")} title={t("sidebar.search.replaceAll")} disabled={!canReplace} onClick={() => searchResponse && replaceMutation.mutate(searchResponse.hits)}><ReplaceAll size={13} /></button>
        </div>
        <label className="search-filter-field">
          <span>{t("sidebar.search.filesToInclude")}</span>
          <input value={includePattern} onChange={(event) => setIncludePattern(event.target.value)} placeholder={t("sidebar.search.includePlaceholder")} spellCheck={false} />
        </label>
        <label className="search-filter-field">
          <span>{t("sidebar.search.filesToExclude")}</span>
          <input value={excludePattern} onChange={(event) => setExcludePattern(event.target.value)} placeholder={t("sidebar.search.excludePlaceholder")} spellCheck={false} />
        </label>
        <label className="search-hidden-toggle">
          <input type="checkbox" checked={includeHidden} onChange={(event) => setIncludeHidden(event.target.checked)} />
          <span>{t("sidebar.search.includeHiddenFiles")}</span>
        </label>
      </form>
      <div className="panel-caption">{replaceMutation.isPending ? t("sidebar.search.replacing") : searchMutation.isPending ? t("sidebar.search.searching") : resultLabel}</div>
      {searchError && <TreeMessage depth={0} tone="error" text={searchError} />}
      {openError && <TreeMessage depth={0} tone="error" text={openError} />}
      <div className="search-results">
        {groupedHits.map((group) => (
          <section className="search-result-group" key={group.path}>
            <div className="search-result-file">
              {(() => {
                const iconMeta = fileIconForName(group.path);
                const Icon = iconMeta.Icon;
                return <Icon size={15} className={iconMeta.className} />;
              })()}
              <span>{group.label}</span>
              <small>{group.hits.length}</small>
            </div>
            {group.hits.map((hit, index) => (
              <button
                className="search-hit"
                type="button"
                key={`${hit.path}-${hit.line}-${hit.column}-${index}`}
                onClick={() => openSearchHitMutation.mutate(hit)}
              >
                <span>{highlightPreview(hit)}</span>
                <small>{hit.line}:{hit.column}</small>
              </button>
            ))}
          </section>
        ))}
      </div>
    </div>
  );
}

function SearchToggle({ active, children, label, onClick }: { active: boolean; children: ReactNode; label: string; onClick: () => void }) {
  return (
    <button className="search-inline-button" data-active={active} type="button" aria-label={label} title={label} onClick={onClick}>
      {children}
    </button>
  );
}

type SearchResultGroup = {
  path: string;
  label: string;
  hits: SearchHit[];
};

function parseGlobList(value: string) {
  return value
    .split(",")
    .map((pattern) => pattern.trim())
    .filter(Boolean);
}

function groupSearchHits(hits: SearchHit[], workspaceRoot: string | null): SearchResultGroup[] {
  const groups = new Map<string, SearchResultGroup>();
  for (const hit of hits) {
    const path = displayPath(hit.path);
    const group = groups.get(path);
    if (group) {
      group.hits.push(hit);
      continue;
    }
    groups.set(path, { path, label: workspaceRoot ? relativePath(workspaceRoot, path) : path, hits: [hit] });
  }
  return [...groups.values()];
}

function buildSearchReplaceEdit(
  hits: SearchHit[],
  query: string,
  replacement: string,
  options: { caseSensitive: boolean; useRegex: boolean },
): LspWorkspaceEdit {
  const files = new Map<string, LspWorkspaceEdit["files"][number]>();
  for (const hit of hits) {
    const range = replacementRangeForHit(hit);
    const path = hit.path;
    const fileEdit = files.get(path) ?? { path, edits: [] };
    fileEdit.edits.push({ range, text: replacementTextForHit(hit, query, replacement, options) });
    files.set(path, fileEdit);
  }
  return { files: [...files.values()] };
}

function replacementRangeForHit(hit: SearchHit) {
  const startColumn = Math.max(1, hit.column);
  return {
    start_line: hit.line,
    start_column: startColumn,
    end_line: hit.line,
    end_column: startColumn + Math.max(0, hit.match_length),
  };
}

function replacementTextForHit(hit: SearchHit, query: string, replacement: string, options: { caseSensitive: boolean; useRegex: boolean }) {
  if (!options.useRegex) return replacement;
  try {
    return hit.match_text.replace(new RegExp(query, options.caseSensitive ? "" : "i"), replacement);
  } catch {
    return replacement;
  }
}

function highlightPreview(hit: SearchHit): ReactNode {
  const preview = hit.preview;
  const matchIndex = Math.max(0, Math.min(preview.length, hit.preview_match_start));
  const matchEnd = Math.max(matchIndex, Math.min(preview.length, hit.preview_match_start + Math.max(0, hit.preview_match_length)));
  if (matchEnd <= matchIndex) return preview;
  return (
    <>
      {preview.slice(0, matchIndex)}
      <mark>{preview.slice(matchIndex, matchEnd)}</mark>
      {preview.slice(matchEnd)}
    </>
  );
}

function GitPanel() {
  const { t } = useTranslation();
  const gitStatus = useLuxStore((state) => state.gitStatus);
  const workspace = useLuxStore((state) => state.workspace);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);
  const [openError, setOpenError] = useState<string | null>(null);

  const openGitFileMutation = useMutation({
    mutationFn: (file: GitFileStatus) => luxCommands.editorOpenFile(gitFileAbsolutePath(file, workspace?.root)),
    onSuccess: (document) => {
      setOpenError(null);
      upsertDocument(document);
    },
    onError: (error) => setOpenError(readErrorMessage(error, t)),
  });

  return (
    <div className="panel-content utility-panel-content">
      <div className="branch-summary">
        <GitBranch size={16} />
        <span>{gitStatus?.branch ?? t("sidebar.git.noRepository")}</span>
      </div>
      {openError && <TreeMessage depth={0} tone="error" text={openError} />}
      <div className="file-tree">
        {gitStatus?.files.map((file) => (
          <button className="file-row" type="button" key={file.path} onClick={() => openGitFileMutation.mutate(file)}>
            <span className="status-pill">{file.index_status.trim() || file.worktree_status.trim() || "M"}</span>
            <span>{displayPath(file.path)}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function gitFileAbsolutePath(file: GitFileStatus, workspaceRoot?: string) {
  const targetPath = gitStatusTargetPath(file.path);
  return workspaceRoot ? joinPath(workspaceRoot, targetPath) : targetPath;
}

function gitStatusTargetPath(path: string) {
  const renameSeparator = " -> ";
  const separatorIndex = path.lastIndexOf(renameSeparator);
  return separatorIndex === -1 ? path : path.slice(separatorIndex + renameSeparator.length);
}

function RunDebugPanel() {
  const { t } = useTranslation();
  const workspace = useLuxStore((state) => state.workspace);
  const [debugInfo, setDebugInfo] = useState<DebugWorkspaceInfo | null>(null);
  const [debugError, setDebugError] = useState<string | null>(null);
  const [selectedConfigName, setSelectedConfigName] = useState<string | null>(null);

  const debugMutation = useMutation({
    mutationFn: luxCommands.debugWorkspaceInfo,
    onSuccess: (info) => {
      setDebugInfo(info);
      setDebugError(null);
      setSelectedConfigName((current) => current ?? info.configurations[0]?.name ?? null);
    },
    onError: (error) => setDebugError(readErrorMessage(error, t)),
  });

  useEffect(() => {
    if (!workspace) {
      setDebugInfo(null);
      setSelectedConfigName(null);
      return;
    }
    debugMutation.mutate();
  }, [workspace?.root]);

  const selectedConfiguration = debugInfo?.configurations.find((configuration) => configuration.name === selectedConfigName) ?? debugInfo?.configurations[0] ?? null;
  const selectedAdapter = selectedConfiguration
    ? debugInfo?.adapters.find((adapter) => adapter.id === selectedConfiguration.type || adapter.command === selectedConfiguration.type) ?? null
    : null;

  return (
    <div className="panel-content utility-panel-content run-debug-panel-content">
      <PanelHeader
        title={t("sidebar.runDebug.title")}
        actions={[{ label: t("sidebar.runDebug.actions.refreshConfiguration"), icon: debugMutation.isPending ? <Loader2 size={14} className="spin-icon" /> : <RefreshCw size={14} />, onClick: () => debugMutation.mutate(), disabled: !workspace || debugMutation.isPending }]}
      />
      {!workspace ? <TreeMessage depth={0} text={t("sidebar.runDebug.empty.openWorkspace")} /> : null}
      {debugError ? <TreeMessage depth={0} tone="error" text={debugError} /> : null}
      {workspace && !debugMutation.isPending && debugInfo && (
        <>
          <DebugLaunchBlock
            adapter={selectedAdapter}
            configuration={selectedConfiguration}
            configurations={debugInfo.configurations}
            launchJsonPath={debugInfo.launch_json_path}
            setSelectedConfigName={setSelectedConfigName}
          />
          <DebugAdaptersBlock adapters={debugInfo.adapters} />
        </>
      )}
      {workspace && debugMutation.isPending ? <TreeMessage depth={0} text={t("sidebar.runDebug.scanning")} /> : null}
    </div>
  );
}

function DebugLaunchBlock({
  adapter,
  configuration,
  configurations,
  launchJsonPath,
  setSelectedConfigName,
}: {
  adapter: DebugAdapterInfo | null;
  configuration: DebugConfiguration | null;
  configurations: DebugConfiguration[];
  launchJsonPath: string | null;
  setSelectedConfigName: (name: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <section className="debug-section">
      <div className="debug-section-title">{t("sidebar.runDebug.start.heading")}</div>
      {configurations.length > 0 ? (
        <select className="debug-config-select" value={configuration?.name ?? ""} onChange={(event) => setSelectedConfigName(event.target.value)} aria-label={t("sidebar.runDebug.aria.debugConfiguration")}>
          {configurations.map((item) => (
            <option value={item.name} key={`${item.type}-${item.request}-${item.name}`}>{item.name}</option>
          ))}
        </select>
      ) : (
        <div className="debug-empty-card">
          <Bug size={16} />
          <span>{t("sidebar.runDebug.empty.noLaunchConfigurations")}</span>
        </div>
      )}
      <button className="debug-run-button" type="button" disabled title={t("sidebar.runDebug.start.disabledTitle")}>
        <Play size={15} /> {t("sidebar.runDebug.startDebugging")}
      </button>
      <div className="debug-meta-list">
        <DebugMeta label={t("sidebar.runDebug.meta.configuration")} value={configuration ? `${configuration.request} / ${configuration.type}` : t("sidebar.runDebug.meta.notConfigured")} />
        <DebugMeta label={t("sidebar.runDebug.meta.adapter")} value={adapter ? `${adapter.name} (${adapter.status})` : configuration ? t("sidebar.runDebug.meta.noMatchingAdapter") : t("sidebar.runDebug.meta.notSelected")} tone={adapter?.status === "missing" ? "warning" : undefined} />
        <DebugMeta label="launch.json" value={launchJsonPath ?? t("sidebar.runDebug.meta.missingLaunchJson")} tone={launchJsonPath ? undefined : "muted"} />
      </div>
    </section>
  );
}

function DebugAdaptersBlock({ adapters }: { adapters: DebugAdapterInfo[] }) {
  const { t } = useTranslation();
  return (
    <section className="debug-section">
      <div className="debug-section-title">{t("sidebar.runDebug.adapters.heading")}</div>
      {adapters.length === 0 ? <TreeMessage depth={0} text={t("sidebar.runDebug.adapters.empty")} /> : null}
      <div className="debug-adapter-list">
        {adapters.map((adapter) => (
          <div className="debug-adapter-row" data-status={adapter.status} key={adapter.id} title={adapter.error ?? adapter.command}>
            <span className="debug-adapter-icon"><Bug size={15} /></span>
            <span className="debug-adapter-main">
              <strong>{adapter.name}</strong>
              <small>{adapter.command}{adapter.args.length > 0 ? ` ${adapter.args.join(" ")}` : ""}</small>
            </span>
            <span className="debug-adapter-status">{adapter.status}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function DebugMeta({ label, tone, value }: { label: string; tone?: "muted" | "warning"; value: string }) {
  return (
    <div className="debug-meta-row" data-tone={tone}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function ExtensionsPanel() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [openError, setOpenError] = useState<string | null>(null);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);

  const extensionsMutation = useMutation({
    mutationFn: luxCommands.extensionsList,
  });

  const openExtensionManifestMutation = useMutation({
    mutationFn: (extension: ExtensionInfo) => luxCommands.editorOpenFile(extension.manifest_path),
    onSuccess: (document) => {
      setOpenError(null);
      upsertDocument(document);
    },
    onError: (error) => setOpenError(readErrorMessage(error, t)),
  });

  useEffect(() => {
    extensionsMutation.mutate();
  }, []);

  const extensions = extensionsMutation.data ?? [];
  const visibleExtensions = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) return extensions;
    return extensions.filter((extension) =>
      `${extension.name} ${extension.id} ${extension.version} ${extension.contributes.join(" ")} ${extension.contribution_points.map((point) => point.id).join(" ")}`
        .toLowerCase()
        .includes(normalizedQuery),
    );
  }, [extensions, query]);

  return (
    <div className="panel-content extensions-panel-content utility-panel-content">
      <form
        className="search-form extensions-search-form"
        onSubmit={(event) => {
          event.preventDefault();
          extensionsMutation.mutate();
        }}
      >
        <Search size={15} />
        <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("sidebar.extensions.search.placeholder")} />
      </form>
      <div className="panel-caption extensions-caption">
        <span>{extensionCaption(extensionsMutation.isPending, visibleExtensions.length, extensions.length, t)}</span>
        <button className="icon-button compact" type="button" aria-label={t("sidebar.extensions.actions.refresh")} title={t("sidebar.extensions.actions.refresh")} onClick={() => extensionsMutation.mutate()} disabled={extensionsMutation.isPending}>
          {extensionsMutation.isPending ? <Loader2 size={14} className="spin-icon" /> : <RefreshCw size={14} />}
        </button>
      </div>
      <div className="extensions-list" role="list" aria-label={t("sidebar.extensions.installed")}>
        {extensionsMutation.error ? <TreeMessage depth={0} tone="error" text={readErrorMessage(extensionsMutation.error, t)} /> : null}
        {openError ? <TreeMessage depth={0} tone="error" text={openError} /> : null}
        {!extensionsMutation.isPending && visibleExtensions.length === 0 ? (
          <div className="extensions-empty-state">
            <Package size={17} />
            <span>{query.trim() ? t("sidebar.extensions.empty.noSearchMatches") : t("sidebar.extensions.empty.noManifests")}</span>
          </div>
        ) : null}
        {visibleExtensions.map((extension) => (
          <ExtensionRow
            extension={extension}
            key={`${extension.root}-${extension.id}`}
            openManifest={() => openExtensionManifestMutation.mutate(extension)}
          />
        ))}
      </div>
    </div>
  );
}

function ExtensionRow({ extension, openManifest }: { extension: ExtensionInfo; openManifest: () => void }) {
  const { t } = useTranslation();
  const invalid = extension.status === "invalid";
  const active = extension.status === "active";

  return (
    <button className="extension-row" type="button" role="listitem" data-invalid={invalid} data-active={active} title={extension.error ?? extension.manifest_path} onClick={openManifest}>
      <span className="extension-row-icon">{invalid ? <ShieldAlert size={16} /> : <Package size={16} />}</span>
      <span className="extension-row-main">
        <span className="extension-row-title">
          <strong>{extension.name}</strong>
          <small>{extension.version}</small>
        </span>
        <span className="extension-row-id">{extension.id}</span>
        <span className="extension-row-contributes">{formatExtensionContributes(extension, t)}</span>
      </span>
      <span className="extension-status" data-invalid={invalid} data-active={active}>{extensionStatusLabel(extension.status, t)}</span>
    </button>
  );
}

function extensionStatusLabel(status: ExtensionInfo["status"], t: TranslateFn) {
  if (status === "active") return t("sidebar.extensions.status.active");
  if (status === "invalid") return t("sidebar.extensions.status.invalid");
  return t("sidebar.extensions.status.discovered");
}

function extensionCaption(loading: boolean, visibleCount: number, totalCount: number, t: TranslateFn) {
  if (loading) return t("sidebar.extensions.scanning");
  if (totalCount === 0) return t("sidebar.extensions.installed");
  if (visibleCount === totalCount) return t("sidebar.extensions.installedCount", { count: totalCount });
  return t("sidebar.extensions.visibleCount", { visibleCount, totalCount });
}

function formatExtensionContributes(extension: ExtensionInfo, t: TranslateFn) {
  if (extension.error) return extension.error;
  if (extension.contribution_points.length === 0) return t("sidebar.extensions.noContributionPoints");
  return extension.contribution_points.map((point) => point.id).join(", ");
}

function buildDirectories(root: string, entries: FsEntry[]) {
  const directories: Record<string, FsEntry[]> = { [normalizePath(root)]: [] };

  for (const entry of entries) {
    const parentKey = normalizePath(parentPath(entry.path));
    directories[parentKey] ??= [];
    directories[parentKey].push(entry);
    if (entry.kind === "directory") directories[normalizePath(entry.path)] ??= [];
  }

  for (const key of Object.keys(directories)) {
    directories[key] = [...directories[key]].sort((left, right) => {
      const leftRank = left.kind === "directory" ? 0 : 1;
      const rightRank = right.kind === "directory" ? 0 : 1;
      if (leftRank !== rightRank) return leftRank - rightRank;
      return left.name.localeCompare(right.name, undefined, { numeric: true, sensitivity: "base" });
    });
  }

  return directories;
}

function workspaceToEntry(workspace: WorkspaceInfo): FsEntry {
  return {
    name: workspace.name,
    path: workspace.root,
    kind: "directory",
    size: 0,
    modified_at: null,
    is_hidden: false,
  };
}

function formatWorkspaceRootLabel(name: string) {
  return (name.startsWith("!") ? name : `!${name}`).toUpperCase();
}

function uniqueDestinationPath(targetDirectory: string, name: string, directories: Record<string, FsEntry[]>, t: TranslateFn) {
  const existing = new Set((directories[normalizePath(targetDirectory)] ?? []).map((entry) => normalizePath(entry.name)));
  if (!existing.has(normalizePath(name))) return joinPath(targetDirectory, name);

  const dotIndex = name.lastIndexOf(".");
  const base = dotIndex > 0 ? name.slice(0, dotIndex) : name;
  const extension = dotIndex > 0 ? name.slice(dotIndex) : "";
  let index = 1;
  let candidate = t("sidebar.explorer.duplicateName.copy", { base, extension });

  while (existing.has(normalizePath(candidate))) {
    index += 1;
    candidate = t("sidebar.explorer.duplicateName.copyIndexed", { base, index, extension });
  }

  return joinPath(targetDirectory, candidate);
}

function findEntryByNormalizedPath(directories: Record<string, FsEntry[]>, normalizedPath: string) {
  for (const entries of Object.values(directories)) {
    const found = entries.find((entry) => normalizePath(entry.path) === normalizedPath);
    if (found) return found;
  }
  return null;
}

function countDescendants(path: string, directories: Record<string, FsEntry[]>) {
  const rootKey = normalizePath(path);
  let count = 0;
  const stack = [...(directories[rootKey] ?? [])];
  while (stack.length > 0) {
    const entry = stack.pop();
    if (!entry) continue;
    count += 1;
    if (entry.kind === "directory") stack.push(...(directories[normalizePath(entry.path)] ?? []));
  }
  return count;
}

function deleteDialogTitle(entry: FsEntry, hasContents: boolean, t: TranslateFn) {
  if (entry.kind === "directory") return hasContents ? t("sidebar.explorer.delete.folderTitle", { name: entry.name }) : t("sidebar.explorer.delete.emptyFolderTitle", { name: entry.name });
  return t("sidebar.explorer.delete.fileTitle", { name: entry.name });
}

function deleteDialogDescription(entry: FsEntry, childCount: number, t: TranslateFn) {
  if (entry.kind === "directory" && childCount > 0) {
    return t("sidebar.explorer.delete.nonEmptyFolderDescription", { name: entry.name });
  }
  if (entry.kind === "directory") return t("sidebar.explorer.delete.emptyFolderDescription", { name: entry.name });
  return t("sidebar.explorer.delete.fileDescription", { name: entry.name });
}

function deleteOpenDocumentsMessage(entry: FsEntry, affectedDocumentCount: number, t: TranslateFn) {
  if (entry.kind !== "directory") {
    return t("sidebar.explorer.delete.unsavedOpenFile", { name: entry.name });
  }
  if (affectedDocumentCount <= 1) {
    return t("sidebar.explorer.delete.unsavedOpenFileInsideFolder", { name: entry.name });
  }
  return t("sidebar.explorer.delete.unsavedOpenFilesInsideFolder", { count: affectedDocumentCount, name: entry.name });
}

function formatItemCount(count: number, t: TranslateFn) {
  return t("sidebar.explorer.itemCount", { count });
}

function pathIsInsideEntry(entryPath: string, candidatePath: string) {
  const normalizedEntryPath = normalizePath(entryPath);
  const normalizedCandidatePath = normalizePath(candidatePath);
  return normalizedCandidatePath === normalizedEntryPath || normalizedCandidatePath.startsWith(`${normalizedEntryPath}/`);
}

function validateMoveTarget(entry: FsEntry, targetDirectory: string, directories: Record<string, FsEntry[]>, t: TranslateFn) {
  const entryPath = normalizePath(entry.path);
  const entryParent = normalizePath(parentPath(entry.path));
  const targetPath = normalizePath(targetDirectory);

  if (entryParent === targetPath) return "same-directory";
  if (entry.kind === "directory" && (targetPath === entryPath || targetPath.startsWith(`${entryPath}/`))) {
    return t("sidebar.explorer.move.cannotMoveFolderIntoItself");
  }

  const siblingExists = (directories[targetPath] ?? []).some((candidate) => normalizePath(candidate.name) === normalizePath(entry.name));
  if (siblingExists) return t("sidebar.explorer.move.nameAlreadyExists", { name: entry.name });
  return null;
}

function relativePath(root: string, path: string) {
  const normalizedRoot = displayPath(root).replace(/\/+$/, "");
  const normalizedPath = displayPath(path);
  return normalizedPath.toLowerCase().startsWith(`${normalizedRoot.toLowerCase()}/`)
    ? normalizedPath.slice(normalizedRoot.length + 1)
    : normalizedPath;
}

function readErrorMessage(error: unknown, t: TranslateFn) {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return t("sidebar.error.operationFailed");
}
