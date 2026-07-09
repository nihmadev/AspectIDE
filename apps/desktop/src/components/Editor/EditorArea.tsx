import { Circle, X } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Panel, Group, Separator } from "react-resizable-panels";
import { useMutation } from "@tanstack/react-query";
import type { editor } from "monaco-editor";
import { useEditorCloseGuard } from "./EditorCloseGuard";
import { FilePreviewPane } from "./FilePreviewPane";
import { ImageEditorPane } from "./ImageEditorPane";
import { MediaEditorPane } from "./MediaEditorPane";
import { PdfEditorPane } from "./PdfEditorPane";
import { MarkdownEditorPane } from "./MarkdownEditorPane";
import { DatabaseEditorPane } from "./DatabaseEditorPane";
import { DiagramEditorPane } from "./DiagramEditorPane";
import { SpreadsheetEditorPane } from "./SpreadsheetEditorPane";
import { TableEditorPane } from "./TableEditorPane";
import { gitStatusForPath, useGitDecorations } from "../lib/explorer/git-decorations";
import { fileIconForName } from "../lib/explorer/file-icons";
import { resolveEditorPaneKind } from "../lib/editor/documents/view-routing";
import { AgentBrowserPreviewEditorPane } from "./AgentBrowserPreviewEditorPane";
import { useTranslation } from "../lib/i18n/useTranslation";
import { resetEditorFontZoom, updateEditorFontSize, zoomEditorFontIn, zoomEditorFontOut } from "../lib/editor/preference-commands";
import { DEFAULT_EDITOR_FONT_STACK, withFontFallback } from "../lib/editor/preferences";
import {
  closedDocumentIdsForAllDocuments,
  closedDocumentIdsForDocumentInGroup,
  closedDocumentIdsForDocumentsToRightInGroup,
  closedDocumentIdsForEditorGroup,
  closedDocumentIdsForOtherDocuments,
  closedDocumentIdsForOtherDocumentsInGroup,
} from "../lib/editor/close-targets";
import {
  documentDisplayPath,
  documentRelativePath,
  documentTitle,
  isEditableTextDocument,
} from "../lib/editor/documents/documents";
import { setEditorTabDragData } from "../lib/editor/chat-bridge";
import { normalizePath, parentPath } from "../lib/explorer/file-tree";
import { AiFileReviewBar } from "./ai-chat/AiFileReviewBar";
import { EditorBreadcrumb } from "./EditorBreadcrumb";
import {
  getPendingFileReviewsSnapshot,
  listPendingFileReviewsForPath,
  subscribePendingFileReviews,
} from "../lib/aspector/utils/pending-file-review";
import {
  applyAiEditDecorations,
  createEmptyAiEditDecorationState,
  type AiEditDecorationState,
} from "../lib/editor/monaco/ai-edit-decorations";
import { setEditorSelectionSnapshot } from "../lib/editor/selection-bridge";
import { applyDebugBreakpointDecorations, registerDebugBreakpointGutter } from "../lib/editor/monaco/debug-adapters";
import {
  applyDiagnosticsMarkers,
  disposeLspProviders,
  registerLspProviders,
  revealEditorTarget,
  textEditsFromMonacoEvent,
  type MonacoDisposable,
  type MonacoEditorInstance,
  type MonacoInstance,
} from "../lib/editor/monaco/lsp-adapters";
import { useAspectStore, type Activity, type EditorGroup } from "../lib/store/index";
import { aspectCommands } from "../lib/tauri/commands";
import type {
  DebugResolvedBreakpoint,
  DebugSourceBreakpoint,
  DocumentSnapshot,
  WorkspaceDiagnostic,
} from "../lib/types/index";

const MonacoEditor = lazy(() => import("@monaco-editor/react"));
const noDiagnostics: WorkspaceDiagnostic[] = [];
const noSourceBreakpoints: DebugSourceBreakpoint[] = [];
const noResolvedBreakpoints: DebugResolvedBreakpoint[] = [];

type EditorTabMenuAction = {
  label: string;
  shortcut?: string;
  disabled?: boolean;
  onClick: () => void;
};

export function EditorArea() {
  const openDocuments = useAspectStore((state) => state.openDocuments);
  const editorGroups = useAspectStore((state) => state.editorGroups);
  const activeEditorGroupId = useAspectStore((state) => state.activeEditorGroupId);
  const setActiveEditorGroup = useAspectStore((state) => state.setActiveEditorGroup);
  const setActiveDocumentInGroup = useAspectStore((state) => state.setActiveDocumentInGroup);
  const splitActiveEditor = useAspectStore((state) => state.splitActiveEditor);
  const splitDocumentInGroup = useAspectStore((state) => state.splitDocumentInGroup);
  const closeEditorGroup = useAspectStore((state) => state.closeEditorGroup);
  const closeDocumentInGroup = useAspectStore((state) => state.closeDocumentInGroup);
  const closeOtherDocumentsInGroup = useAspectStore((state) => state.closeOtherDocumentsInGroup);
  const closeDocumentsToRightInGroup = useAspectStore((state) => state.closeDocumentsToRightInGroup);
  const closeSavedDocumentsInGroup = useAspectStore((state) => state.closeSavedDocumentsInGroup);
  const closeOtherDocuments = useAspectStore((state) => state.closeOtherDocuments);
  const closeAllDocuments = useAspectStore((state) => state.closeAllDocuments);
  const ensureExplorerExpandedPath = useAspectStore((state) => state.ensureExplorerExpandedPath);
  const upsertDocument = useAspectStore((state) => state.upsertDocument);
  const applyDocumentEdits = useAspectStore((state) => state.applyDocumentEdits);
  const workspace = useAspectStore((state) => state.workspace);
  const setActiveActivity = useAspectStore((state) => state.setActiveActivity);
  const setSidebarVisible = useAspectStore((state) => state.setSidebarVisible);
  const editorPreferences = useAspectStore((state) => state.editorPreferences);
  const aiPreferences = useAspectStore((state) => state.aiPreferences);
  const diagnosticsByPath = useAspectStore((state) => state.diagnosticsByPath);
  const debugSourceBreakpointsByPath = useAspectStore((state) => state.debugSourceBreakpointsByPath);
  const debugResolvedBreakpointsByPath = useAspectStore((state) => state.debugResolvedBreakpointsByPath);
  const toggleDebugSourceBreakpoint = useAspectStore((state) => state.toggleDebugSourceBreakpoint);
  const pendingEditorReveal = useAspectStore((state) => state.pendingEditorReveal);
  const setPendingEditorReveal = useAspectStore((state) => state.setPendingEditorReveal);
  const consumePendingEditorReveal = useAspectStore((state) => state.consumePendingEditorReveal);
  const updateOpenDocuments = useAspectStore((state) => state.updateOpenDocuments);
  const { requestCloseDocuments } = useEditorCloseGuard();
  const { t } = useTranslation();

  useEffect(() => {
    let lastZoomAt = 0;
    const zoomFromWheel = (event: WheelEvent) => {
      if (!(event.ctrlKey || event.metaKey)) return;
      if (!useAspectStore.getState().editorPreferences.mouseWheelZoom) return;
      if (!(event.target instanceof Element) || !event.target.closest(".editor-area")) return;

      event.preventDefault();
      event.stopPropagation();
      const now = Date.now();
      if (now - lastZoomAt < 45) return;
      lastZoomAt = now;
      updateEditorFontSize(event.deltaY < 0 ? 1 : -1);
    };
    window.addEventListener("wheel", zoomFromWheel, { capture: true, passive: false });
    return () => window.removeEventListener("wheel", zoomFromWheel, { capture: true });
  }, []);

  const documentsById = new Map(openDocuments.map((document) => [document.id, document]));
  const visibleGroups = editorGroups
    .map((group) => ({
      ...group,
      documentIds: group.documentIds.filter((documentId) => documentsById.has(documentId)),
    }))
    .filter((group) => group.documentIds.length > 0);

  const saveMutation = useMutation({
    mutationFn: aspectCommands.editorSaveFile,
    onSuccess: upsertDocument,
  });

  const saveAsMutation = useMutation({
    mutationFn: aspectCommands.editorSaveFileAs,
    onSuccess: upsertDocument,
  });

  const editorOperationQueues = useRef(new Map<string, Promise<void>>());
  const enqueueEditorOperation = (documentId: string, operation: () => Promise<void>) => {
    const previous = editorOperationQueues.current.get(documentId) ?? Promise.resolve();
    const next = previous.catch(() => undefined).then(operation);
    editorOperationQueues.current.set(documentId, next);
    void next.finally(() => {
      if (editorOperationQueues.current.get(documentId) === next) editorOperationQueues.current.delete(documentId);
    });
  };

  const applySpreadsheetChanges = (id: string, text: string) => {
    enqueueEditorOperation(id, async () => {
      const document = await aspectCommands.editorUpdateText(id, text);
      upsertDocument(document);
    });
  };

  const applyEditorChanges = (id: string, event: editor.IModelContentChangedEvent, value: string | undefined) => {
    if (typeof value !== "string") return;

    if (event.isFlush || event.isEolChange) {
      enqueueEditorOperation(id, async () => {
        const document = await aspectCommands.editorUpdateText(id, value);
        upsertDocument(document);
      });
      return;
    }

    const edits = textEditsFromMonacoEvent(event);
    if (edits.length === 0) return;
    enqueueEditorOperation(id, async () => {
      try {
        const result = await aspectCommands.editorApplyEdits(id, edits);
        applyDocumentEdits(id, edits, result);
      } catch {
        const document = await aspectCommands.editorUpdateText(id, value);
        upsertDocument(document);
      }
    });
  };

  const saveAllOpenDocuments = () => {
    for (const document of openDocuments) {
      if (document.is_dirty) saveMutation.mutate(document.id);
    }
  };

  if (visibleGroups.length === 0) {
    return <section className="editor-empty" aria-label={t("editor.aria.emptyEditor")} />;
  }

  return (
    <section className="editor-area" data-groups={visibleGroups.length}>
      <Group orientation="horizontal" className="editor-groups">
        {visibleGroups.map((group, index) => {
          const activeDocument = documentsById.get(group.activeDocumentId ?? "") ?? documentsById.get(group.documentIds[0]);
          if (!activeDocument) return null;

          return (
            <EditorGroupPane
              activeDocument={activeDocument}
              activeEditorGroupId={activeEditorGroupId}
              closeDocumentInGroup={(groupId, documentId) => {
                requestCloseDocuments(
                  closedDocumentIdsForDocumentInGroup(openDocuments, editorGroups, groupId, documentId),
                  () => closeDocumentInGroup(groupId, documentId),
                );
              }}
              closeEditorGroup={(groupId) => {
                requestCloseDocuments(
                  closedDocumentIdsForEditorGroup(openDocuments, editorGroups, groupId),
                  () => closeEditorGroup(groupId),
                  { title: t("editor.confirm.closeGroup") },
                );
              }}
              closeDocumentsToRightInGroup={(groupId, documentId) => {
                requestCloseDocuments(
                  closedDocumentIdsForDocumentsToRightInGroup(openDocuments, editorGroups, groupId, documentId),
                  () => closeDocumentsToRightInGroup(groupId, documentId),
                );
              }}
              closeOtherDocuments={(documentId) => {
                requestCloseDocuments(
                  closedDocumentIdsForOtherDocuments(openDocuments, documentId),
                  () => closeOtherDocuments(documentId),
                  { title: t("editor.confirm.closeOtherEditors") },
                );
              }}
              closeOtherDocumentsInGroup={(groupId, documentId) => {
                requestCloseDocuments(
                  closedDocumentIdsForOtherDocumentsInGroup(openDocuments, editorGroups, groupId, documentId),
                  () => closeOtherDocumentsInGroup(groupId, documentId),
                  { title: t("editor.confirm.closeOtherEditors") },
                );
              }}
              closeSavedDocumentsInGroup={closeSavedDocumentsInGroup}
              closeAllDocuments={() => {
                requestCloseDocuments(
                  closedDocumentIdsForAllDocuments(openDocuments),
                  closeAllDocuments,
                  { title: t("editor.confirm.closeAllEditors") },
                );
              }}
              documents={group.documentIds.map((documentId) => documentsById.get(documentId)).filter(Boolean) as DocumentSnapshot[]}
              diagnosticsByPath={diagnosticsByPath}
              debugResolvedBreakpointsByPath={debugResolvedBreakpointsByPath}
              debugSourceBreakpointsByPath={debugSourceBreakpointsByPath}
              editorPreferences={editorPreferences}
              aiPreferences={aiPreferences}
              pendingEditorReveal={pendingEditorReveal}
              setPendingEditorReveal={setPendingEditorReveal}
              consumePendingEditorReveal={consumePendingEditorReveal}
              ensureExplorerExpandedPath={ensureExplorerExpandedPath}
              group={group}
              groupCount={visibleGroups.length}
              index={index}
              key={group.id}
              saveAllOpenDocuments={saveAllOpenDocuments}
              saveDocument={(id) => saveMutation.mutate(id)}
              saveDocumentAs={(id) => saveAsMutation.mutate(id)}
              setActiveDocumentInGroup={setActiveDocumentInGroup}
              setActiveActivity={setActiveActivity}
              setActiveEditorGroup={setActiveEditorGroup}
              setSidebarVisible={setSidebarVisible}
              splitActiveEditor={splitActiveEditor}
              splitDocumentInGroup={splitDocumentInGroup}
              toggleDebugSourceBreakpoint={toggleDebugSourceBreakpoint}
              applyEditorChanges={applyEditorChanges}
              applySpreadsheetChanges={applySpreadsheetChanges}
              updateOpenDocuments={updateOpenDocuments}
              upsertDocument={upsertDocument}
              workspaceRoot={workspace?.root ?? null}
            />
          );
        })}
      </Group>
    </section>
  );
}

function EditorGroupPane({
  activeDocument,
  activeEditorGroupId,
  closeAllDocuments,
  closeDocumentInGroup,
  closeEditorGroup,
  closeDocumentsToRightInGroup,
  closeOtherDocuments,
  closeOtherDocumentsInGroup,
  closeSavedDocumentsInGroup,
  documents,
  diagnosticsByPath,
  debugResolvedBreakpointsByPath,
  debugSourceBreakpointsByPath,
  editorPreferences,
  aiPreferences,
  pendingEditorReveal,
  setPendingEditorReveal,
  consumePendingEditorReveal,
  ensureExplorerExpandedPath,
  group,
  groupCount,
  index,
  saveAllOpenDocuments,
  saveDocumentAs,
  saveDocument,
  setActiveDocumentInGroup,
  setActiveActivity,
  setActiveEditorGroup,
  setSidebarVisible,
  splitActiveEditor,
  splitDocumentInGroup,
  toggleDebugSourceBreakpoint,
  applyEditorChanges,
  applySpreadsheetChanges,
  updateOpenDocuments,
  upsertDocument,
  workspaceRoot,
}: {
  activeDocument: DocumentSnapshot;
  activeEditorGroupId: string;
  closeAllDocuments: () => void;
  closeDocumentInGroup: (groupId: string, documentId: string) => void;
  closeEditorGroup: (groupId: string) => void;
  closeDocumentsToRightInGroup: (groupId: string, documentId: string) => void;
  closeOtherDocuments: (documentId: string) => void;
  closeOtherDocumentsInGroup: (groupId: string, documentId: string) => void;
  closeSavedDocumentsInGroup: (groupId: string) => void;
  documents: DocumentSnapshot[];
  diagnosticsByPath: Record<string, WorkspaceDiagnostic[]>;
  debugResolvedBreakpointsByPath: ReturnType<typeof useAspectStore.getState>["debugResolvedBreakpointsByPath"];
  debugSourceBreakpointsByPath: ReturnType<typeof useAspectStore.getState>["debugSourceBreakpointsByPath"];
  editorPreferences: typeof useAspectStore.getState extends () => infer T ? T extends { editorPreferences: infer P } ? P : never : never;
  aiPreferences: typeof useAspectStore.getState extends () => infer T ? T extends { aiPreferences: infer P } ? P : never : never;
  pendingEditorReveal: ReturnType<typeof useAspectStore.getState>["pendingEditorReveal"];
  setPendingEditorReveal: ReturnType<typeof useAspectStore.getState>["setPendingEditorReveal"];
  consumePendingEditorReveal: ReturnType<typeof useAspectStore.getState>["consumePendingEditorReveal"];
  ensureExplorerExpandedPath: (path: string) => void;
  group: EditorGroup;
  groupCount: number;
  index: number;
  saveAllOpenDocuments: () => void;
  saveDocumentAs: (id: string) => void;
  saveDocument: (id: string) => void;
  setActiveDocumentInGroup: (groupId: string, documentId: string) => void;
  setActiveActivity: (activity: Activity) => void;
  setActiveEditorGroup: (groupId: string) => void;
  setSidebarVisible: (visible: boolean) => void;
  splitActiveEditor: () => void;
  splitDocumentInGroup: (groupId: string, documentId: string, side: "left" | "right") => void;
  toggleDebugSourceBreakpoint: (path: string, line: number) => void;
  applyEditorChanges: (id: string, event: editor.IModelContentChangedEvent, value: string | undefined) => void;
  applySpreadsheetChanges: (id: string, text: string) => void;
  updateOpenDocuments: ReturnType<typeof useAspectStore.getState>["updateOpenDocuments"];
  upsertDocument: ReturnType<typeof useAspectStore.getState>["upsertDocument"];
  workspaceRoot: string | null;
}) {
  const { t } = useTranslation();
  const activeAiChatSessionId = useAspectStore((state) => state.activeAiChatSessionId);
  const isActiveGroup = activeEditorGroupId === group.id;
  const editorRef = useRef<MonacoEditorInstance | null>(null);
  const monacoRef = useRef<MonacoInstance | null>(null);
  const lspProviderDisposablesRef = useRef<MonacoDisposable[]>([]);
  const breakpointGutterDisposableRef = useRef<MonacoDisposable | null>(null);
  const breakpointDecorationsRef = useRef<string[]>([]);
  const aiEditDecorationsRef = useRef<AiEditDecorationState>(createEmptyAiEditDecorationState());
  const activeDocumentRef = useRef(activeDocument);
  activeDocumentRef.current = activeDocument;
  const tabsListRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const el = tabsListRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      el.scrollLeft += e.deltaY;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);
  const pendingReviews = useSyncExternalStore(
    subscribePendingFileReviews,
    getPendingFileReviewsSnapshot,
    getPendingFileReviewsSnapshot,
  );
  const activeFileReview = useMemo(() => {
    if (!activeDocument.path) return null;
    return listPendingFileReviewsForPath(activeDocument.path)[0] ?? null;
  }, [activeDocument.path, pendingReviews]);
  const diagnostics = activeDocument.path ? diagnosticsByPath[normalizePath(activeDocument.path)] ?? noDiagnostics : noDiagnostics;
  const activeDocumentPath = activeDocument.path ? normalizePath(activeDocument.path) : null;
  const sourceBreakpoints = activeDocumentPath ? debugSourceBreakpointsByPath[activeDocumentPath] ?? noSourceBreakpoints : noSourceBreakpoints;
  const resolvedBreakpoints = activeDocumentPath ? debugResolvedBreakpointsByPath[activeDocumentPath] ?? noResolvedBreakpoints : noResolvedBreakpoints;
  const editorPaneKind = resolveEditorPaneKind(activeDocument);
  const isMonacoDocument = editorPaneKind === "monaco" || editorPaneKind === "markdown" || editorPaneKind === "diagram";
  const isEditableDocument = isEditableTextDocument(activeDocument);
  // User-selected code font (Settings в†’ Fonts) ahead of the built-in mono stack.
  const editorFontFamily = withFontFallback(editorPreferences.fontFamily, DEFAULT_EDITOR_FONT_STACK);

  useEffect(() => () => {
    disposeLspProviders(lspProviderDisposablesRef.current);
    lspProviderDisposablesRef.current = [];
    breakpointGutterDisposableRef.current?.dispose();
    breakpointGutterDisposableRef.current = null;
    if (editorRef.current && monacoRef.current) {
      aiEditDecorationsRef.current = applyAiEditDecorations(
        editorRef.current,
        monacoRef.current,
        aiEditDecorationsRef.current,
        null,
      );
    }
  }, []);

  useEffect(() => {
    if (editorPaneKind !== "monaco") return;
    applyDiagnosticsMarkers(editorRef.current, monacoRef.current, group.id, diagnostics);
  }, [activeDocument.path, diagnostics, group.id, editorPaneKind]);

  useEffect(() => {
    if (editorPaneKind !== "monaco") return;
    breakpointDecorationsRef.current = applyDebugBreakpointDecorations(
      editorRef.current,
      monacoRef.current,
      breakpointDecorationsRef.current,
      activeDocumentRef.current,
      sourceBreakpoints,
      resolvedBreakpoints,
    );
  }, [activeDocument.path, editorPaneKind, resolvedBreakpoints, sourceBreakpoints]);

  useEffect(() => {
    if (!editorRef.current) return;
    if (pendingEditorReveal?.documentId !== activeDocument.id) return;
    revealEditorTarget(editorRef.current, consumePendingEditorReveal(activeDocument.id));
  }, [activeDocument.id, consumePendingEditorReveal, pendingEditorReveal]);

  useEffect(() => {
    if (!isMonacoDocument) return;
    aiEditDecorationsRef.current = applyAiEditDecorations(
      editorRef.current,
      monacoRef.current,
      aiEditDecorationsRef.current,
      activeFileReview,
    );
  }, [activeFileReview, activeDocument.text, isMonacoDocument]);

  useEffect(() => {
    if (editorPaneKind !== "monaco") return;
    if (!editorRef.current || !monacoRef.current) return;
    const document = activeDocumentRef.current;
    disposeLspProviders(lspProviderDisposablesRef.current);
    lspProviderDisposablesRef.current = registerLspProviders({
      document,
      editor: editorRef.current,
      monaco: monacoRef.current,
      setPendingEditorReveal,
      upsertDocument,
      updateOpenDocuments,
      t,
    });
    breakpointGutterDisposableRef.current?.dispose();
    breakpointGutterDisposableRef.current = registerDebugBreakpointGutter(editorRef.current, monacoRef.current, document, toggleDebugSourceBreakpoint);
  }, [activeDocument.id, activeDocument.path, activeDocument.language_id, editorPaneKind, setPendingEditorReveal, toggleDebugSourceBreakpoint, updateOpenDocuments, upsertDocument, t]);

  return (
    <>
      {index > 0 && <Separator className="resize-handle editor-group-separator" />}
      <Panel minSize="260px">
        <div className="editor-group" data-active={isActiveGroup} onPointerDown={() => setActiveEditorGroup(group.id)}>
          <div className="tabs-row">
            <div className="tabs-list" ref={tabsListRef} role="tablist" aria-label={t("editor.aria.openEditorsGroup", { groupNumber: index + 1 })}>
              {documents.map((document) => (
                <EditorTab
                  active={document.id === activeDocument.id}
                  aiPendingReview={Boolean(document.path && listPendingFileReviewsForPath(document.path).length > 0)}
                  document={document}
                  groupId={group.id}
                  key={document.id}
                  closeAllDocuments={closeAllDocuments}
                  closeDocumentsToRightInGroup={closeDocumentsToRightInGroup}
                  closeDocumentInGroup={closeDocumentInGroup}
                  closeOtherDocumentsInGroup={closeOtherDocumentsInGroup}
                  closeSavedDocumentsInGroup={closeSavedDocumentsInGroup}
                  groupDocumentCount={documents.length}
                  hasSavedDocuments={documents.some((candidate) => !candidate.is_dirty)}
                  isRightmost={documents[documents.length - 1]?.id === document.id}
                  revealInExplorer={() => {
                    if (!document.path) return;
                    setActiveActivity("explorer");
                    setSidebarVisible(true);
                    ensureExplorerExpandedPath(parentPath(document.path));
                  }}
                  splitDocumentInGroup={splitDocumentInGroup}
                  setActiveDocumentInGroup={setActiveDocumentInGroup}
                  saveDocument={saveDocument}
                  saveDocumentAs={saveDocumentAs}
                  workspaceRoot={workspaceRoot}
                />
              ))}
            </div>
          </div>
        <div className="editor-content-wrapper">
          {activeDocument.path && (
            <EditorBreadcrumb
              documentPath={activeDocument.path}
              workspaceRoot={workspaceRoot}
            />
          )}
        {editorPaneKind === "browserPreview" ? (
            <AgentBrowserPreviewEditorPane document={activeDocument} preferences={aiPreferences} />
          ) : editorPaneKind === "database" ? (
            <DatabaseEditorPane document={activeDocument} />
          ) : editorPaneKind === "spreadsheet" ? (
            <SpreadsheetEditorPane
              document={activeDocument}
              onChange={(text) => applySpreadsheetChanges(activeDocument.id, text)}
            />
          ) : editorPaneKind === "table" ? (
            <TableEditorPane
              document={activeDocument}
              onChange={(text) => applySpreadsheetChanges(activeDocument.id, text)}
            />
          ) : editorPaneKind === "diagram" ? (
            <DiagramEditorPane
              document={activeDocument}
              fontFamily={editorFontFamily}
              fontLigatures={editorPreferences.fontLigatures}
              fontSize={editorPreferences.fontSize}
              lineHeight={editorPreferences.lineHeight}
              minimap={editorPreferences.minimap}
              onChange={(value, event) => {
                if (!isEditableDocument) return;
                applyEditorChanges(activeDocument.id, event, value);
              }}
              readOnly={!isEditableDocument}
              renderWhitespace={editorPreferences.renderWhitespace}
              smoothScrolling={editorPreferences.smoothScrolling}
              tabSize={editorPreferences.tabSize}
              wordWrap={editorPreferences.wordWrap}
            />
          ) : editorPaneKind === "image" ? (
            <ImageEditorPane document={activeDocument} />
          ) : editorPaneKind === "pdf" ? (
            <PdfEditorPane document={activeDocument} />
          ) : editorPaneKind === "media" ? (
            <MediaEditorPane document={activeDocument} />
          ) : editorPaneKind === "markdown" ? (
            <MarkdownEditorPane
              document={activeDocument}
              fontFamily={editorFontFamily}
              fontLigatures={editorPreferences.fontLigatures}
              fontSize={editorPreferences.fontSize}
              lineHeight={editorPreferences.lineHeight}
              minimap={editorPreferences.minimap}
              onChange={(value, event) => {
                if (!isEditableDocument) return;
                applyEditorChanges(activeDocument.id, event, value);
              }}
              readOnly={!isEditableDocument}
              renderWhitespace={editorPreferences.renderWhitespace}
              smoothScrolling={editorPreferences.smoothScrolling}
              tabSize={editorPreferences.tabSize}
              wordWrap={editorPreferences.wordWrap}
            />
          ) : editorPaneKind === "monaco" ? (
            <>
              <AiFileReviewBar documentPath={activeDocument.path} sessionId={activeAiChatSessionId} />
              <Suspense fallback={<div className="editor-loading">{t("editor.status.loading")}</div>}>
              <MonacoEditor
                height="100%"
                theme="vs-dark"
                path={`${group.id}:${documentDisplayPath(activeDocument)}`}
                language={activeDocument.language_id}
                value={activeDocument.text}
                options={{
                  automaticLayout: true,
                  fontFamily: editorFontFamily,
                  fontLigatures: editorPreferences.fontLigatures,
                  fontSize: editorPreferences.fontSize,
                  lineHeight: editorPreferences.lineHeight,
                  minimap: { enabled: editorPreferences.minimap, scale: 0.75 },
                  mouseWheelZoom: false,
                  padding: { top: 18, bottom: 18 },
                  readOnly: !isEditableDocument,
                  renderWhitespace: editorPreferences.renderWhitespace,
                  smoothScrolling: editorPreferences.smoothScrolling,
                  scrollBeyondLastLine: false,
                  tabSize: editorPreferences.tabSize,
                  unicodeHighlight: { ambiguousCharacters: editorPreferences.unicodeHighlightAmbiguousCharacters },
                  wordWrap: editorPreferences.wordWrap,
                  renderLineHighlight: "all",
                  glyphMargin: true,
                  lineNumbersMinChars: 2,
                }}
                onChange={(value, event) => {
                  if (!isEditableDocument) return;
                  applyEditorChanges(activeDocument.id, event, value);
                }}
                onMount={(editor, monaco) => {
                  editorRef.current = editor;
                  monacoRef.current = monaco;
                  editor.onDidChangeCursorSelection(() => {
                    const doc = activeDocumentRef.current;
                    const selection = editor.getSelection();
                    const model = editor.getModel();
                    if (!selection || !model || !doc.path) {
                      setEditorSelectionSnapshot(null);
                      return;
                    }
                    const selectedText = model.getValueInRange(selection);
                    if (!selectedText.trim()) {
                      setEditorSelectionSnapshot(null);
                      return;
                    }
                    setEditorSelectionSnapshot({
                      documentId: doc.id,
                      path: doc.path,
                      languageId: doc.language_id,
                      startLine: selection.startLineNumber,
                      endLine: selection.endLineNumber,
                      startColumn: selection.startColumn,
                      endColumn: selection.endColumn,
                      text: selectedText,
                    });
                  });
                  disposeLspProviders(lspProviderDisposablesRef.current);
                  lspProviderDisposablesRef.current = isEditableDocument ? registerLspProviders({
                    document: activeDocument,
                    editor,
                    monaco,
                    setPendingEditorReveal,
                    upsertDocument,
                    updateOpenDocuments,
                    t,
                  }) : [];
                  breakpointGutterDisposableRef.current?.dispose();
                  breakpointGutterDisposableRef.current = isEditableDocument ? registerDebugBreakpointGutter(editor, monaco, activeDocument, toggleDebugSourceBreakpoint) : null;
                  applyDiagnosticsMarkers(editor, monaco, group.id, diagnostics);
                  breakpointDecorationsRef.current = applyDebugBreakpointDecorations(editor, monaco, breakpointDecorationsRef.current, activeDocument, sourceBreakpoints, resolvedBreakpoints);
                  revealEditorTarget(editor, consumePendingEditorReveal(activeDocument.id));
                  aiEditDecorationsRef.current = applyAiEditDecorations(
                    editor,
                    monaco,
                    aiEditDecorationsRef.current,
                    activeFileReview,
                  );
                }}
              />
            </Suspense>
            </>
          ) : <FilePreviewPane document={activeDocument} variant="editor" />}
        </div>
        </div>
      </Panel>
    </>
  );
}

function EditorTab({
  active,
  aiPendingReview = false,
  closeAllDocuments,
  closeDocumentsToRightInGroup,
  closeDocumentInGroup,
  closeOtherDocumentsInGroup,
  closeSavedDocumentsInGroup,
  document,
  groupDocumentCount,
  groupId,
  hasSavedDocuments,
  isRightmost,
  revealInExplorer,
  saveDocument,
  saveDocumentAs,
  setActiveDocumentInGroup,
  splitDocumentInGroup,
  workspaceRoot,
}: {
  active: boolean;
  aiPendingReview?: boolean;
  closeAllDocuments: () => void;
  closeDocumentsToRightInGroup: (groupId: string, documentId: string) => void;
  closeDocumentInGroup: (groupId: string, documentId: string) => void;
  closeOtherDocumentsInGroup: (groupId: string, documentId: string) => void;
  closeSavedDocumentsInGroup: (groupId: string) => void;
  document: DocumentSnapshot;
  groupDocumentCount: number;
  groupId: string;
  hasSavedDocuments: boolean;
  isRightmost: boolean;
  revealInExplorer: () => void;
  saveDocument: (id: string) => void;
  saveDocumentAs: (id: string) => void;
  setActiveDocumentInGroup: (groupId: string, documentId: string) => void;
  splitDocumentInGroup: (groupId: string, documentId: string, side: "left" | "right") => void;
  workspaceRoot: string | null;
}) {
  const { t } = useTranslation();
  const name = documentTitle(document);
  const gitDecorations = useGitDecorations();
  const gitStatus = document.path ? gitStatusForPath(gitDecorations, document.path) : null;
  const [menuPosition, setMenuPosition] = useState<{ x: number; y: number } | null>(null);

  const closeTab = () => closeDocumentInGroup(groupId, document.id);
  const copyPath = () => {
    if (document.path) void aspectCommands.clipboardWriteText(document.path).catch(() => undefined);
  };
  const copyRelativePath = () => void aspectCommands.clipboardWriteText(documentRelativePath(document, workspaceRoot)).catch(() => undefined);

  const fileBackedGroups: EditorTabMenuAction[][] = document.path
    ? [
        [
          { label: t("editor.tabMenu.copyPath"), shortcut: "Shift+Alt+C", onClick: copyPath },
          { label: t("editor.tabMenu.copyRelativePath"), shortcut: "Ctrl+M Ctrl+Shift+C", onClick: copyRelativePath },
        ],
        [
          { label: t("editor.tabMenu.revealInFileExplorer"), shortcut: "Shift+Alt+R", onClick: () => document.path && void aspectCommands.fsRevealInFileExplorer(document.path).catch(() => undefined) },
          { label: t("editor.tabMenu.revealInExplorerView"), onClick: revealInExplorer },
        ],
      ]
    : [];

  const menuGroups: EditorTabMenuAction[][] = [
    [
      { label: t("common.save"), shortcut: "Ctrl+S", disabled: !document.is_dirty || !document.view.editable, onClick: () => saveDocument(document.id) },
      { label: t("common.saveAs"), shortcut: "Ctrl+Shift+S", disabled: !document.view.editable, onClick: () => saveDocumentAs(document.id) },
    ],
    [
      { label: t("common.close"), shortcut: "Ctrl+F4", onClick: closeTab },
      { label: t("editor.tabMenu.closeOthers"), disabled: groupDocumentCount < 2, onClick: () => closeOtherDocumentsInGroup(groupId, document.id) },
      { label: t("editor.tabMenu.closeToRight"), disabled: isRightmost, onClick: () => closeDocumentsToRightInGroup(groupId, document.id) },
      { label: t("editor.tabMenu.closeSaved"), shortcut: "Ctrl+M U", disabled: !hasSavedDocuments, onClick: () => closeSavedDocumentsInGroup(groupId) },
      { label: t("editor.tabMenu.closeAll"), shortcut: "Ctrl+M W", onClick: closeAllDocuments },
    ],
    ...fileBackedGroups,
    [
      { label: t("editor.tabMenu.splitLeft"), onClick: () => splitDocumentInGroup(groupId, document.id, "left") },
      { label: t("editor.tabMenu.splitRight"), shortcut: "Ctrl+M Ctrl+\\", onClick: () => splitDocumentInGroup(groupId, document.id, "right") },
    ],
  ];

  return (
    <div
      className="editor-tab"
      data-active={active}
      data-ai-pending={aiPendingReview || undefined}
      key={document.id}
      title={documentDisplayPath(document)}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
        setActiveDocumentInGroup(groupId, document.id);
        setMenuPosition({ x: event.clientX, y: event.clientY });
      }}
      onMouseDown={(event) => {
        if (event.button === 1) {
          event.preventDefault();
          closeTab();
        }
      }}
    >
      <button
        className="editor-tab-main"
        type="button"
        role="tab"
        aria-selected={active}
        draggable
        title={t("editor.tab.dragToChatHint", { name })}
        onDragStart={(event) => {
          setEditorTabDragData(event.dataTransfer, document.id);
        }}
        onClick={() => setActiveDocumentInGroup(groupId, document.id)}
      >
        {(() => {
          const iconMeta = fileIconForName(name);
          return iconMeta.imgSrc
            ? <img src={iconMeta.imgSrc} width={15} height={15} className={iconMeta.className} alt="" />
            : <iconMeta.Icon size={15} className={iconMeta.className} />;
        })()}
        <span className="editor-tab-name" data-git={gitStatus ?? undefined}>{name}</span>
        {document.is_dirty && <Circle className="dirty-dot" size={8} fill="currentColor" />}
      </button>
      <button
        className="editor-tab-close"
        type="button"
        aria-label={t("editor.tab.closeNamed", { name })}
        title={t("common.close")}
        onClick={(event) => {
          event.stopPropagation();
          closeTab();
        }}
      >
        <X size={13} />
      </button>
      {menuPosition && <EditorTabContextMenu groups={menuGroups} x={menuPosition.x} y={menuPosition.y} onClose={() => setMenuPosition(null)} />}
    </div>
  );
}

function EditorTabContextMenu({ groups, onClose, x, y }: { groups: EditorTabMenuAction[][]; onClose: () => void; x: number; y: number }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [position, setPosition] = useState({ x, y });

  useEffect(() => {
    const close = () => onClose();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  useEffect(() => {
    const menu = ref.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    setPosition({
      x: Math.max(6, Math.min(x, window.innerWidth - rect.width - 8)),
      y: Math.max(6, Math.min(y, window.innerHeight - rect.height - 8)),
    });
  }, [x, y]);

  return (
    <div className="editor-tab-context-menu" ref={ref} style={{ left: position.x, top: position.y }} onPointerDown={(event) => event.stopPropagation()}>
      {groups.map((group, groupIndex) => (
        <div className="editor-tab-context-menu-group" key={groupIndex}>
          {group.map((action) => (
            <button
              className="editor-tab-context-menu-item"
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
