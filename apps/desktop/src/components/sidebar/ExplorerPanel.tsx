import * as Dialog from "@radix-ui/react-dialog";
import { AlertTriangle, ChevronDown, ChevronRight, Copy, FilePlus2, Folder, FolderOpen, FolderPlus, Loader2, RefreshCw, Trash2 } from "lucide-react";
import type { CSSProperties, DragEvent, KeyboardEvent as ReactKeyboardEvent, MouseEvent } from "react";
import { Fragment, memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEditorCloseGuard } from "../EditorCloseGuard";
import { fileIconForName } from "../../lib/fileIcons";
import { displayPath, joinPath, normalizePath, parentPath } from "../../lib/fileTree";
import { useTranslation } from "../../lib/i18n/useTranslation";
import { useLuxStore } from "../../lib/store";
import { luxCommands } from "../../lib/tauri";
import { externalFilesFromDrop, importExternalFiles, isExternalFileDrag } from "../../lib/explorerImport";
import type { FsEntry } from "../../lib/types";
import {
  buildDirectories,
  buildGitDecorations,
  countDescendants,
  flattenVisibleRows,
  gitDecoBadge,
  type GitDecoStatus,
  deleteDialogDescription,
  deleteDialogTitle,
  deleteOpenDocumentsMessage,
  findEntryByNormalizedPath,
  formatItemCount,
  formatWorkspaceRootLabel,
  pathIsInsideEntry,
  uniqueDestinationPath,
  validateMoveTarget,
  workspaceToEntry,
} from "./ExplorerHelpers";
import type { ClipboardEntry, ContextMenuState, DraggedEntry, PendingCreate, PendingDelete, PendingRename, TreeAction } from "./ExplorerTypes";
import { PanelHeader, readErrorMessage, relativePath, TreeMessage } from "./SidebarShared";

// Fixed row height of a tree row (.file-row / header / create / rename), in px.
// Used as the virtualizer size estimate; `measureElement` corrects any drift.
const EXPLORER_ROW_HEIGHT = 24;
// Below this visible-row count the tree renders in normal flow — the virtual
// layout's absolute positioning and measurement overhead is not worth it.
const EXPLORER_VIRTUALIZE_THRESHOLD = 80;
// Extra rows rendered above/below the viewport so fast scrolls stay smooth.
const EXPLORER_OVERSCAN = 12;

export function ExplorerPanel() {
  const { t } = useTranslation();
  const workspace = useLuxStore((state) => state.workspace);
  const workspaceFolders = useLuxStore((state) => state.workspaceFolders);
  const setWorkspace = useLuxStore((state) => state.setWorkspace);
  const addWorkspaceFolder = useLuxStore((state) => state.addWorkspaceFolder);
  const removeWorkspaceFolder = useLuxStore((state) => state.removeWorkspaceFolder);
  const fileEntries = useLuxStore((state) => state.fileEntries);
  const gitStatus = useLuxStore((state) => state.gitStatus);
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
  const upsertTerminalSession = useLuxStore((state) => state.upsertTerminalSession);
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
  // Git status → tree decorations (green added, gold modified, red deleted, …),
  // refreshed automatically by the workspace watcher's gitStatusChanged events.
  const gitDecorations = useMemo(
    () => buildGitDecorations(gitStatus?.files ?? [], rootPath),
    [gitStatus, rootPath],
  );

  const openFileMutation = useMutation({
    mutationFn: luxCommands.editorOpenFile,
    onSuccess: upsertDocument,
    onError: (error) => setOperationError(readErrorMessage(error, t)),
  });

  // Retargets open documents whose path is at or inside `sourcePath` to the
  // equivalent path under `destinationPath` after a rename/move operation.
  // Closes the stale tab and reopens the file at the new location. Dirty
  // content is discarded — the renamed file on disk already holds the last
  // saved state, which is the correct ground-truth after a FS rename.
  const retargetMovedDocuments = useCallback(async (sourcePath: string, destinationPath: string) => {
    const { openDocuments: docs } = useLuxStore.getState();
    const affected = docs.filter((doc) => doc.path && pathIsInsideEntry(sourcePath, doc.path));
    if (affected.length === 0) return;
    const sourceNorm = normalizePath(sourcePath);
    await Promise.all(affected.map(async (doc) => {
      if (!doc.path) return;
      const docNorm = normalizePath(doc.path);
      const newPath =
        docNorm === sourceNorm
          ? destinationPath
          : joinPath(destinationPath, docNorm.slice(sourceNorm.length + 1));
      // Close stale tab before opening the retargeted one
      closeDocument(doc.id);
      try {
        upsertDocument(await luxCommands.editorOpenFile(newPath));
      } catch {
        // New path unreachable (e.g. directory that contains this file was
        // not fully moved yet) — leave tab closed instead of keeping stale path.
      }
    }));
  }, [closeDocument, upsertDocument]);

  const refreshSeq = useRef(0);
  const refreshTree = useCallback(async () => {
    if (!workspace) return;
    const seq = ++refreshSeq.current;
    setFileTreeLoading(true);
    setFileTreeError(null);
    setOperationError(null);
    try {
      const pairs = await Promise.all(workspaceRoots.map(async (folder) => [folder, await luxCommands.fsReadTree(folder.root)] as const));
      const directories = pairs.reduce<Record<string, FsEntry[]>>((merged, [folder, entries]) => ({
        ...merged,
        ...buildDirectories(folder.root, entries),
      }), {});
      if (refreshSeq.current !== seq) return;
      setFileTreeDirectories(directories);
      setFileEntries(directories[normalizePath(workspace.root)] ?? []);
    } catch (error) {
      if (refreshSeq.current === seq) setFileTreeError(readErrorMessage(error, t));
    } finally {
      if (refreshSeq.current === seq) setFileTreeLoading(false);
    }
  }, [setFileEntries, setFileTreeDirectories, setFileTreeError, setFileTreeLoading, t, workspace, workspaceRoots]);

  const loadWorkspaceRoot = useCallback(async (folder: typeof workspaceRoots[number]) => {
    const seq = ++refreshSeq.current;
    setFileTreeLoading(true);
    setFileTreeError(null);
    setOperationError(null);
    try {
      const entries = await luxCommands.fsReadTree(folder.root);
      const directories = buildDirectories(folder.root, entries);
      if (refreshSeq.current !== seq) return;
      setFileTreeDirectories({ ...useLuxStore.getState().fileTreeDirectories, ...directories });
      if (workspace?.root === folder.root) setFileEntries(directories[normalizePath(folder.root)] ?? []);
      ensureExplorerExpandedPath(folder.root);
    } catch (error) {
      if (refreshSeq.current === seq) setFileTreeError(readErrorMessage(error, t));
    } finally {
      if (refreshSeq.current === seq) setFileTreeLoading(false);
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
        const destination = joinPath(parentPath(entry.path), trimmed);
        await luxCommands.fsRename(entry.path, destination);
        // Retarget any open editors that were pointing at the old path so
        // subsequent saves and AI tool calls reach the correct location.
        await retargetMovedDocuments(entry.path, destination);
        await refreshTree();
      } catch (error) {
        setOperationError(readErrorMessage(error, t));
      } finally {
        setPendingRename(null);
      }
    },
    [refreshTree, retargetMovedDocuments, t],
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
      if (clipboardEntry.entry.kind === "directory") {
        const source = normalizePath(clipboardEntry.entry.path);
        const target = normalizePath(targetDirectory);
        if (target === source || target.startsWith(`${source}/`)) {
          setOperationError(t("sidebar.explorer.move.cannotMoveFolderIntoItself"));
          return;
        }
      }
      const destination = uniqueDestinationPath(targetDirectory, clipboardEntry.entry.name, fileTreeDirectories, t);
      try {
        setOperationError(null);
        if (clipboardEntry.operation === "cut") {
          await luxCommands.fsRename(clipboardEntry.entry.path, destination);
          // Retarget open editors that referred to the cut source path.
          await retargetMovedDocuments(clipboardEntry.entry.path, destination);
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
    [clipboardEntry, ensureExplorerExpandedPath, fileTreeDirectories, refreshTree, retargetMovedDocuments, t],
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
        const destination = joinPath(targetDirectory, entry.name);
        await luxCommands.fsRename(entry.path, destination);
        // Retarget open editors that referred to the dragged entry's old path.
        await retargetMovedDocuments(entry.path, destination);
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
    [ensureExplorerExpandedPath, fileTreeDirectories, refreshTree, retargetMovedDocuments, t],
  );

  const copyAbsolutePath = useCallback(async (entry: FsEntry) => {
    try {
      await luxCommands.clipboardWriteText(entry.path);
    } catch (error) {
      setOperationError(readErrorMessage(error, t));
    }
  }, [t]);

  const copyRelativePath = useCallback(async (entry: FsEntry) => {
    const owningRoot = workspaceRoots
      .filter((folder) => pathIsInsideEntry(folder.root, entry.path))
      .sort((a, b) => normalizePath(b.root).length - normalizePath(a.root).length)[0] ?? workspace;
    if (!owningRoot) return;
    try {
      await luxCommands.clipboardWriteText(relativePath(owningRoot.root, entry.path));
    } catch (error) {
      setOperationError(readErrorMessage(error, t));
    }
  }, [t, workspace, workspaceRoots]);

  const openEntryTerminal = useCallback(
    async (entry: FsEntry) => {
      try {
        const cwd = entry.kind === "directory" ? entry.path : parentPath(entry.path);
        openBottomPanel("terminal");
        const createdTerminal = await luxCommands.terminalCreate(undefined, cwd);
        upsertTerminalSession(createdTerminal, true);
      } catch (error) {
        setOperationError(readErrorMessage(error, t));
      }
    },
    [openBottomPanel, t, upsertTerminalSession],
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
    // Only close documents that belong to the folder being removed — leave
    // documents from other workspace roots (and untitled buffers) untouched.
    const affectedDocumentIds = openDocuments
      .filter((doc) => doc.path && pathIsInsideEntry(root, doc.path))
      .map((doc) => doc.id);
    requestCloseDocuments(affectedDocumentIds, () => {
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
    (targetDirectory: string, external = false) => {
      if (external) {
        // OS file drag: any workspace directory is a valid copy target.
        setDropTargetPath(normalizePath(targetDirectory));
        ensureExplorerExpandedPath(targetDirectory);
        return true;
      }
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
    async (targetDirectory: string, externalFiles?: File[]) => {
      // OS files dropped from outside the app: copy them into the directory.
      if (externalFiles && externalFiles.length > 0) {
        setDropTargetPath(null);
        setOperationError(null);
        try {
          await importExternalFiles(targetDirectory, externalFiles);
          ensureExplorerExpandedPath(targetDirectory);
        } catch (error) {
          setOperationError(readErrorMessage(error, t));
        }
        return;
      }
      const currentDraggedEntry = draggedEntryRef.current ?? draggedEntry;
      if (!currentDraggedEntry) return;
      await moveEntryInto(currentDraggedEntry.entry, targetDirectory);
    },
    [draggedEntry, ensureExplorerExpandedPath, moveEntryInto, t],
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

  // Flatten the expanded tree into a bounded, depth-tagged visible-row array so
  // large monorepos render through a virtual window instead of mounting every
  // node. Recomputed only when the loaded directories, expansion set, or the
  // active create-row parent change — not on selection/drag/git ticks.
  const pendingCreateParentKey = pendingCreate ? normalizePath(pendingCreate.parentPath) : null;
  const visibleRows = useMemo(
    () => flattenVisibleRows(workspaceRoots, fileTreeDirectories, expandedPaths, pendingCreateParentKey, rootEntries, rootPath),
    [workspaceRoots, fileTreeDirectories, expandedPaths, pendingCreateParentKey, rootEntries, rootPath],
  );

  const treeScrollRef = useRef<HTMLDivElement | null>(null);
  const treeVirtualizer = useVirtualizer({
    count: visibleRows.length,
    getScrollElement: () => treeScrollRef.current,
    getItemKey: (index) => {
      const row = visibleRows[index];
      if (!row) return index;
      return row.kind === "entry" ? row.entryKey : row.kind === "folder-header" ? `header:${row.folderKey}` : `create:${row.parentPath}`;
    },
    estimateSize: () => EXPLORER_ROW_HEIGHT,
    overscan: EXPLORER_OVERSCAN,
  });
  const useVirtualTree = visibleRows.length >= EXPLORER_VIRTUALIZE_THRESHOLD;

  // Renders one flattened row. Directory children are already laid out as
  // sibling rows by `flattenVisibleRows`, so an entry row only needs to know its
  // own expansion/child state for the chevron and inline create/rename slots.
  const renderTreeRow = useCallback((row: typeof visibleRows[number]) => {
    if (row.kind === "folder-header") {
      const folderKey = row.folderKey;
      const folder = workspaceRoots.find((candidate) => normalizePath(candidate.root) === folderKey);
      if (!folder) return null;
      return (
        <button
          className="tree-section-title workspace-folder-title"
          type="button"
          data-drop-target={dropTargetPath === folderKey}
          onClick={() => toggleExplorerExpandedPath(folderKey)}
          onDragEnter={(event) => { if (!dragOverDirectory(folder.root, isExternalFileDrag(event.dataTransfer))) return; event.preventDefault(); }}
          onDragOver={(event) => {
            const external = isExternalFileDrag(event.dataTransfer);
            if (!dragOverDirectory(folder.root, external)) return;
            event.preventDefault();
            event.stopPropagation();
            event.dataTransfer.dropEffect = external ? "copy" : "move";
          }}
          onDrop={(event) => { event.preventDefault(); event.stopPropagation(); void dropEntryIntoDirectory(folder.root, externalFilesFromDrop(event.dataTransfer)); }}
          onContextMenu={(event) => { event.preventDefault(); setContextMenu({ entry: workspaceToEntry(folder), source: "blank", x: event.clientX, y: event.clientY }); }}
        >
          {expandedPaths.has(folderKey) ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
          <span>{folder.name}</span>
        </button>
      );
    }
    if (row.kind === "create") {
      return <CreateRow create={createEntry} depth={row.depth} onCancel={() => setPendingCreate(null)} pendingCreate={pendingCreate ?? { kind: "file", parentPath: row.parentPath }} />;
    }
    const entry = row.entry;
    const key = row.entryKey;
    if (pendingRename && normalizePath(pendingRename.entry.path) === key) {
      return <RenameRow depth={row.depth} entry={entry} onCancel={() => setPendingRename(null)} rename={renameEntry} />;
    }
    const isDirectory = entry.kind === "directory";
    const children = fileTreeDirectories[key] ?? [];
    return (
      <FileRow
        activePath={activeDocument?.path ?? null}
        clipboardEntry={clipboardEntry}
        depth={row.depth}
        entry={entry}
        expanded={expandedPaths.has(key)}
        gitStatus={gitDecorations.get(key) ?? null}
        hasChildren={children.length > 0}
        isDirectory={isDirectory}
        isDragging={draggedEntry ? normalizePath(draggedEntry.entry.path) === key : false}
        isDropTarget={dropTargetPath === key}
        isSelected={selectedEntryPath === key}
        pendingParentKey={pendingCreateParentKey}
        rowKey={key}
        dragOverDirectory={dragOverDirectory}
        dragLeaveDirectory={dragLeaveDirectory}
        dropEntryIntoDirectory={dropEntryIntoDirectory}
        endEntryDrag={endEntryDrag}
        openFile={(path) => openFileMutation.mutate(path)}
        requestDeleteEntry={requestDeleteEntry}
        setContextMenu={setContextMenu}
        setSelectedEntryPath={setSelectedEntryPath}
        startEntryDrag={startEntryDrag}
        toggleDirectory={toggleDirectory}
      />
    );
  }, [activeDocument?.path, clipboardEntry, createEntry, draggedEntry, dragLeaveDirectory, dragOverDirectory, dropEntryIntoDirectory, dropTargetPath, endEntryDrag, expandedPaths, fileTreeDirectories, gitDecorations, openFileMutation, pendingCreate, pendingCreateParentKey, pendingRename, renameEntry, requestDeleteEntry, selectedEntryPath, startEntryDrag, toggleDirectory, toggleExplorerExpandedPath, workspaceRoots]);

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
              if (!rootEntry || !dragOverDirectory(rootEntry.path, isExternalFileDrag(event.dataTransfer))) return;
              event.preventDefault();
            }}
            onDragOver={(event) => {
              const external = isExternalFileDrag(event.dataTransfer);
              if (!rootEntry || !dragOverDirectory(rootEntry.path, external)) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = external ? "copy" : "move";
            }}
            onDrop={(event) => {
              if (!rootEntry) return;
              event.preventDefault();
              event.stopPropagation();
              void dropEntryIntoDirectory(rootEntry.path, externalFilesFromDrop(event.dataTransfer));
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
          ref={treeScrollRef}
          role="tree"
          tabIndex={0}
          onKeyDown={handleExplorerKeyDown}
          onContextMenu={(event) => { event.preventDefault(); if (rootEntry) setContextMenu({ entry: rootEntry, source: "blank", x: event.clientX, y: event.clientY }); }}
          onDragOver={(event) => {
            const external = isExternalFileDrag(event.dataTransfer);
            if (!rootEntry || !dragOverDirectory(rootEntry.path, external)) return;
            event.preventDefault();
            event.stopPropagation();
            event.dataTransfer.dropEffect = external ? "copy" : "move";
          }}
          onDrop={(event) => {
            if (!rootEntry) return;
            event.preventDefault();
            event.stopPropagation();
            void dropEntryIntoDirectory(rootEntry.path, externalFilesFromDrop(event.dataTransfer));
          }}
        >
          {fileTreeError && <TreeMessage depth={0} tone="error" text={fileTreeError} />}
          {operationError && <TreeMessage depth={0} tone="error" text={operationError} />}
          {useVirtualTree ? (
            <div className="file-tree-virtual" style={{ height: treeVirtualizer.getTotalSize() }}>
              {treeVirtualizer.getVirtualItems().map((item) => {
                const row = visibleRows[item.index];
                if (!row) return null;
                return (
                  <div
                    key={item.key}
                    className="file-tree-virtual-row"
                    data-index={item.index}
                    ref={treeVirtualizer.measureElement}
                    style={{ transform: `translateY(${item.start}px)` }}
                  >
                    {renderTreeRow(row)}
                  </div>
                );
              })}
            </div>
          ) : (
            visibleRows.map((row, index) => (
              <Fragment key={row.kind === "entry" ? row.entryKey : row.kind === "folder-header" ? `header:${row.folderKey}` : `create:${row.parentPath}:${index}`}>
                {renderTreeRow(row)}
              </Fragment>
            ))
          )}
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

const FileRow = memo(function FileRow({
  activePath,
  clipboardEntry,
  depth,
  entry,
  expanded,
  gitStatus,
  hasChildren,
  isDirectory,
  isDragging,
  isDropTarget,
  isSelected,
  pendingParentKey,
  rowKey,
  dragOverDirectory,
  dragLeaveDirectory,
  dropEntryIntoDirectory,
  endEntryDrag,
  openFile,
  requestDeleteEntry,
  setContextMenu,
  setSelectedEntryPath,
  startEntryDrag,
  toggleDirectory,
}: {
  activePath: string | null;
  clipboardEntry: ClipboardEntry | null;
  depth: number;
  entry: FsEntry;
  expanded: boolean;
  gitStatus: GitDecoStatus | null;
  hasChildren: boolean;
  isDirectory: boolean;
  isDragging: boolean;
  isDropTarget: boolean;
  isSelected: boolean;
  pendingParentKey: string | null;
  rowKey: string;
  dragOverDirectory: (targetDirectory: string, external?: boolean) => boolean;
  dragLeaveDirectory: (targetDirectory: string) => void;
  dropEntryIntoDirectory: (targetDirectory: string, externalFiles?: File[]) => Promise<void>;
  endEntryDrag: () => void;
  openFile: (path: string) => void;
  requestDeleteEntry: (entry: FsEntry) => void;
  setContextMenu: (contextMenu: ContextMenuState | null) => void;
  setSelectedEntryPath: (path: string | null) => void;
  startEntryDrag: (entry: FsEntry) => void;
  toggleDirectory: (entry: FsEntry) => void;
}) {
  const iconMeta = fileIconForName(entry.name);
  const Icon = isDirectory ? (expanded ? FolderOpen : Folder) : iconMeta.Icon;
  const isCut = clipboardEntry?.operation === "cut" && normalizePath(clipboardEntry.entry.path) === rowKey;

  // Handlers are built here from already-stable parent callbacks + this row's own
  // `entry`, so they are recreated only when this row actually re-renders — which,
  // thanks to the memo wrapper, happens only when one of this row's own props
  // changes. A git/selection/drag change elsewhere in the tree no longer rebuilds
  // every row's closures or re-renders unaffected rows.
  const onOpen = () => {
    setSelectedEntryPath(rowKey);
    if (!isDirectory) openFile(entry.path);
    else if (hasChildren || pendingParentKey === rowKey) toggleDirectory(entry);
  };
  const onContextMenu = (event: MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    setSelectedEntryPath(rowKey);
    setContextMenu({ entry, source: "row", x: event.clientX, y: event.clientY });
  };
  const onDragStart = (event: DragEvent<HTMLButtonElement>) => {
    event.dataTransfer.effectAllowed = "copyMove";
    event.dataTransfer.setData("text/plain", entry.path);
    // Precise marker so drop targets (e.g. the AI composer) can accept a workspace
    // file/folder drag without clobbering ordinary text drops, plus the entry kind
    // so a directory is attached as a folder mention, not run through the file path.
    event.dataTransfer.setData("application/x-lux-path", entry.path);
    event.dataTransfer.setData("application/x-lux-kind", entry.kind);
    startEntryDrag(entry);
  };
  const onDragEnter = isDirectory
    ? (event: DragEvent<HTMLButtonElement>) => {
        const external = isExternalFileDrag(event.dataTransfer);
        if (!dragOverDirectory(entry.path, external)) return;
        event.preventDefault();
        event.stopPropagation();
        event.dataTransfer.dropEffect = external ? "copy" : "move";
      }
    : undefined;
  const onDragOver = isDirectory
    ? (event: DragEvent<HTMLButtonElement>) => {
        const external = isExternalFileDrag(event.dataTransfer);
        if (!dragOverDirectory(entry.path, external)) return;
        event.preventDefault();
        event.stopPropagation();
        event.dataTransfer.dropEffect = external ? "copy" : "move";
      }
    : undefined;
  const onDragLeave = isDirectory
    ? (event: DragEvent<HTMLButtonElement>) => {
        if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
        dragLeaveDirectory(entry.path);
      }
    : undefined;
  const onDrop = isDirectory
    ? (event: DragEvent<HTMLButtonElement>) => {
        event.preventDefault();
        event.stopPropagation();
        void dropEntryIntoDirectory(entry.path, externalFilesFromDrop(event.dataTransfer));
      }
    : undefined;

  return (
    <div className="file-row-shell" style={{ "--tree-depth": depth } as CSSProperties}>
      <button
        className="file-row"
        type="button"
        draggable
        role="treeitem"
        aria-expanded={isDirectory ? expanded : undefined}
        data-active={activePath ? normalizePath(activePath) === rowKey : false}
        data-cut={isCut}
        data-dragging={isDragging}
        data-drop-target={isDropTarget}
        data-selected={isSelected}
        data-git={gitStatus ?? undefined}
        data-git-dir={gitStatus && isDirectory ? "" : undefined}
        onClick={onOpen}
        onContextMenu={onContextMenu}
        onKeyDown={(event) => {
          if (event.key !== "Delete") return;
          event.preventDefault();
          event.stopPropagation();
          requestDeleteEntry(entry);
        }}
        onDragEnd={endEntryDrag}
        onDragEnter={onDragEnter}
        onDragLeave={onDragLeave}
        onDragOver={onDragOver}
        onDragStart={onDragStart}
        onDrop={onDrop}
      >
        {isDirectory && hasChildren ? <span className="tree-chevron">{expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</span> : <span className="tree-chevron" />}
        <Icon size={15} className={isDirectory ? "folder-icon" : iconMeta.className} />
        <span className="file-row-name">{entry.name}</span>
        {gitStatus && !isDirectory && <span className="git-badge" title={gitStatus}>{gitDecoBadge(gitStatus)}</span>}
      </button>
    </div>
  );
});

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
  const committedRef = useRef(false);
  const commit = () => {
    if (committedRef.current) return;
    committedRef.current = true;
    void rename(entry, name);
  };

  return (
    <form
      className="create-row rename-row"
      style={{ "--tree-depth": depth } as CSSProperties}
      onSubmit={(event) => {
        event.preventDefault();
        commit();
      }}
    >
      <span className="tree-chevron" />
      <Icon size={15} className={entry.kind === "directory" ? "folder-icon" : iconMeta.className} />
      <input
        autoFocus
        value={name}
        onBlur={commit}
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
