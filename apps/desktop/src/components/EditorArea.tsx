import { Circle, FileCode2, Minus, Plus, RotateCcw, Save, SaveAll, SquareSplitHorizontal, X } from "lucide-react";
import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { Panel, Group, Separator } from "react-resizable-panels";
import { useMutation } from "@tanstack/react-query";
import type { editor } from "monaco-editor";
import { useEditorCloseGuard } from "./EditorCloseGuard";
import { FilePreviewPane } from "./FilePreviewPane";
import { useTranslation } from "../lib/i18n/useTranslation";
import { resetEditorFontZoom, updateEditorFontSize, zoomEditorFontIn, zoomEditorFontOut } from "../lib/editorPreferenceCommands";
import {
  closedDocumentIdsForAllDocuments,
  closedDocumentIdsForDocumentInGroup,
  closedDocumentIdsForDocumentsToRightInGroup,
  closedDocumentIdsForEditorGroup,
  closedDocumentIdsForOtherDocuments,
  closedDocumentIdsForOtherDocumentsInGroup,
} from "../lib/editorCloseTargets";
import { documentDisplayPath, documentRelativePath, documentTitle } from "../lib/documents";
import { normalizePath, parentPath } from "../lib/fileTree";
import { applyDebugBreakpointDecorations, registerDebugBreakpointGutter } from "../lib/monacoDebugAdapters";
import {
  applyDiagnosticsMarkers,
  disposeLspProviders,
  registerLspProviders,
  revealEditorTarget,
  textEditsFromMonacoEvent,
  type MonacoDisposable,
  type MonacoEditorInstance,
  type MonacoInstance,
} from "../lib/monacoLspAdapters";
import { useLuxStore, type Activity, type EditorGroup } from "../lib/store";
import { luxCommands } from "../lib/tauri";
import type {
  DocumentSnapshot,
  WorkspaceDiagnostic,
} from "../lib/types";

const MonacoEditor = lazy(() => import("@monaco-editor/react"));
const noDiagnostics: WorkspaceDiagnostic[] = [];

type EditorTabMenuAction = {
  label: string;
  shortcut?: string;
  disabled?: boolean;
  onClick: () => void;
};

export function EditorArea() {
  const openDocuments = useLuxStore((state) => state.openDocuments);
  const editorGroups = useLuxStore((state) => state.editorGroups);
  const activeEditorGroupId = useLuxStore((state) => state.activeEditorGroupId);
  const setActiveEditorGroup = useLuxStore((state) => state.setActiveEditorGroup);
  const setActiveDocumentInGroup = useLuxStore((state) => state.setActiveDocumentInGroup);
  const splitActiveEditor = useLuxStore((state) => state.splitActiveEditor);
  const splitDocumentInGroup = useLuxStore((state) => state.splitDocumentInGroup);
  const closeEditorGroup = useLuxStore((state) => state.closeEditorGroup);
  const closeDocumentInGroup = useLuxStore((state) => state.closeDocumentInGroup);
  const closeOtherDocumentsInGroup = useLuxStore((state) => state.closeOtherDocumentsInGroup);
  const closeDocumentsToRightInGroup = useLuxStore((state) => state.closeDocumentsToRightInGroup);
  const closeSavedDocumentsInGroup = useLuxStore((state) => state.closeSavedDocumentsInGroup);
  const closeOtherDocuments = useLuxStore((state) => state.closeOtherDocuments);
  const closeAllDocuments = useLuxStore((state) => state.closeAllDocuments);
  const ensureExplorerExpandedPath = useLuxStore((state) => state.ensureExplorerExpandedPath);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);
  const applyDocumentEdits = useLuxStore((state) => state.applyDocumentEdits);
  const workspace = useLuxStore((state) => state.workspace);
  const setActiveActivity = useLuxStore((state) => state.setActiveActivity);
  const setSidebarVisible = useLuxStore((state) => state.setSidebarVisible);
  const editorPreferences = useLuxStore((state) => state.editorPreferences);
  const diagnosticsByPath = useLuxStore((state) => state.diagnosticsByPath);
  const debugSourceBreakpointsByPath = useLuxStore((state) => state.debugSourceBreakpointsByPath);
  const debugResolvedBreakpointsByPath = useLuxStore((state) => state.debugResolvedBreakpointsByPath);
  const toggleDebugSourceBreakpoint = useLuxStore((state) => state.toggleDebugSourceBreakpoint);
  const pendingEditorReveal = useLuxStore((state) => state.pendingEditorReveal);
  const setPendingEditorReveal = useLuxStore((state) => state.setPendingEditorReveal);
  const consumePendingEditorReveal = useLuxStore((state) => state.consumePendingEditorReveal);
  const updateOpenDocuments = useLuxStore((state) => state.updateOpenDocuments);
  const { requestCloseDocuments } = useEditorCloseGuard();
  const { t } = useTranslation();

  useEffect(() => {
    let lastZoomAt = 0;
    const zoomFromWheel = (event: WheelEvent) => {
      if (!(event.ctrlKey || event.metaKey)) return;
      if (!useLuxStore.getState().editorPreferences.mouseWheelZoom) return;
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
    mutationFn: luxCommands.editorSaveFile,
    onSuccess: upsertDocument,
  });

  const saveAsMutation = useMutation({
    mutationFn: luxCommands.editorSaveFileAs,
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

  const applyEditorChanges = (id: string, event: editor.IModelContentChangedEvent, value: string | undefined) => {
    if (typeof value !== "string") return;

    if (event.isFlush || event.isEolChange) {
      enqueueEditorOperation(id, async () => {
        const document = await luxCommands.editorUpdateText(id, value);
        upsertDocument(document);
      });
      return;
    }

    const edits = textEditsFromMonacoEvent(event);
    if (edits.length === 0) return;
    enqueueEditorOperation(id, async () => {
      try {
        const result = await luxCommands.editorApplyEdits(id, edits);
        applyDocumentEdits(id, edits, result);
      } catch {
        const document = await luxCommands.editorUpdateText(id, value);
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
  debugResolvedBreakpointsByPath: ReturnType<typeof useLuxStore.getState>["debugResolvedBreakpointsByPath"];
  debugSourceBreakpointsByPath: ReturnType<typeof useLuxStore.getState>["debugSourceBreakpointsByPath"];
  editorPreferences: typeof useLuxStore.getState extends () => infer T ? T extends { editorPreferences: infer P } ? P : never : never;
  pendingEditorReveal: ReturnType<typeof useLuxStore.getState>["pendingEditorReveal"];
  setPendingEditorReveal: ReturnType<typeof useLuxStore.getState>["setPendingEditorReveal"];
  consumePendingEditorReveal: ReturnType<typeof useLuxStore.getState>["consumePendingEditorReveal"];
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
  updateOpenDocuments: ReturnType<typeof useLuxStore.getState>["updateOpenDocuments"];
  upsertDocument: ReturnType<typeof useLuxStore.getState>["upsertDocument"];
  workspaceRoot: string | null;
}) {
  const { t } = useTranslation();
  const isActiveGroup = activeEditorGroupId === group.id;
  const editorRef = useRef<MonacoEditorInstance | null>(null);
  const monacoRef = useRef<MonacoInstance | null>(null);
  const lspProviderDisposablesRef = useRef<MonacoDisposable[]>([]);
  const breakpointGutterDisposableRef = useRef<MonacoDisposable | null>(null);
  const breakpointDecorationsRef = useRef<string[]>([]);
  const diagnostics = activeDocument.path ? diagnosticsByPath[normalizePath(activeDocument.path)] ?? noDiagnostics : noDiagnostics;
  const activeDocumentPath = activeDocument.path ? normalizePath(activeDocument.path) : null;
  const sourceBreakpoints = activeDocumentPath ? debugSourceBreakpointsByPath[activeDocumentPath] ?? [] : [];
  const resolvedBreakpoints = activeDocumentPath ? debugResolvedBreakpointsByPath[activeDocumentPath] ?? [] : [];
  const isMonacoDocument = activeDocument.view.strategy === "monacoText" || activeDocument.view.strategy === "diagramPreview";
  const isEditableDocument = activeDocument.view.mode === "editableText";

  useEffect(() => () => {
    disposeLspProviders(lspProviderDisposablesRef.current);
    lspProviderDisposablesRef.current = [];
    breakpointGutterDisposableRef.current?.dispose();
    breakpointGutterDisposableRef.current = null;
  }, []);

  useEffect(() => {
    if (!isMonacoDocument) return;
    applyDiagnosticsMarkers(editorRef.current, monacoRef.current, group.id, diagnostics);
  }, [activeDocument.path, diagnostics, group.id, isMonacoDocument]);

  useEffect(() => {
    if (!isMonacoDocument) return;
    breakpointDecorationsRef.current = applyDebugBreakpointDecorations(
      editorRef.current,
      monacoRef.current,
      breakpointDecorationsRef.current,
      activeDocument,
      sourceBreakpoints,
      resolvedBreakpoints,
    );
  }, [activeDocument, isMonacoDocument, resolvedBreakpoints, sourceBreakpoints]);

  useEffect(() => {
    if (pendingEditorReveal?.documentId !== activeDocument.id) return;
    revealEditorTarget(editorRef.current, consumePendingEditorReveal(activeDocument.id));
  }, [activeDocument.id, consumePendingEditorReveal, pendingEditorReveal]);

  useEffect(() => {
    if (!isMonacoDocument) return;
    if (!editorRef.current || !monacoRef.current) return;
    disposeLspProviders(lspProviderDisposablesRef.current);
    lspProviderDisposablesRef.current = registerLspProviders({
      document: activeDocument,
      editor: editorRef.current,
      monaco: monacoRef.current,
      setPendingEditorReveal,
      upsertDocument,
      updateOpenDocuments,
      t,
    });
    breakpointGutterDisposableRef.current?.dispose();
    breakpointGutterDisposableRef.current = registerDebugBreakpointGutter(editorRef.current, monacoRef.current, activeDocument, toggleDebugSourceBreakpoint);
    breakpointDecorationsRef.current = applyDebugBreakpointDecorations(
      editorRef.current,
      monacoRef.current,
      breakpointDecorationsRef.current,
      activeDocument,
      sourceBreakpoints,
      resolvedBreakpoints,
    );
  }, [activeDocument, isMonacoDocument, resolvedBreakpoints, setPendingEditorReveal, sourceBreakpoints, toggleDebugSourceBreakpoint, updateOpenDocuments, upsertDocument, t]);

  return (
    <>
      {index > 0 && <Separator className="resize-handle editor-group-separator" />}
      <Panel minSize="260px">
        <div className="editor-group" data-active={isActiveGroup} onPointerDown={() => setActiveEditorGroup(group.id)}>
          <div className="tabs-row">
            <div className="tabs-list" role="tablist" aria-label={t("editor.aria.openEditorsGroup", { groupNumber: index + 1 })}>
              {documents.map((document) => (
                <EditorTab
                  active={document.id === activeDocument.id}
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
            <div className="editor-actions">
              <button
                className="icon-button compact"
                type="button"
                aria-label={t("editor.action.splitRight")}
                title={t("editor.action.splitRight")}
                onClick={splitActiveEditor}
              >
                <SquareSplitHorizontal size={15} />
              </button>
              <button
                className="icon-button compact"
                type="button"
                aria-label={t("editor.action.zoomFontOut")}
                title={t("editor.action.zoomFontOut")}
                disabled={editorPreferences.fontSize <= 10}
                onClick={zoomEditorFontOut}
              >
                <Minus size={14} />
              </button>
              <button
                className="editor-font-size-button"
                type="button"
                aria-label={t("editor.action.resetFontZoom")}
                title={t("editor.action.resetFontZoom")}
                onClick={resetEditorFontZoom}
              >
                {t("editor.fontSize.display", { fontSize: editorPreferences.fontSize })}
              </button>
              <button
                className="icon-button compact"
                type="button"
                aria-label={t("editor.action.zoomFontIn")}
                title={t("editor.action.zoomFontIn")}
                disabled={editorPreferences.fontSize >= 30}
                onClick={zoomEditorFontIn}
              >
                <Plus size={14} />
              </button>
              <button
                className="icon-button compact"
                type="button"
                aria-label={t("editor.action.saveAllFiles")}
                title={t("editor.action.saveAllFiles")}
              disabled={!documents.some((document) => document.is_dirty)}
                onClick={saveAllOpenDocuments}
              >
                <SaveAll size={15} />
              </button>
              <button
                className="icon-button compact"
                type="button"
                aria-label={t("editor.action.closeOtherEditors")}
                title={t("editor.action.closeOtherEditors")}
                disabled={documents.length < 2}
                onClick={() => closeOtherDocuments(activeDocument.id)}
              >
                <X size={15} />
              </button>
              <button
                className="icon-button compact"
                type="button"
                aria-label={t("editor.action.closeGroup")}
                title={t("editor.action.closeGroup")}
                disabled={groupCount < 2}
                onClick={() => closeEditorGroup(group.id)}
              >
                <X size={15} />
              </button>
            </div>
            <button
              className="icon-button compact"
              type="button"
              aria-label={t("editor.action.saveFile")}
              title={t("editor.action.saveFile")}
              disabled={!isEditableDocument && !activeDocument.is_dirty}
              onClick={() => saveDocument(activeDocument.id)}
            >
              <Save size={15} />
            </button>
          </div>
          {isMonacoDocument ? (
            <Suspense fallback={<div className="editor-loading">{t("editor.status.loading")}</div>}>
              <MonacoEditor
                height="100%"
                theme="vs-dark"
                path={`${group.id}:${documentDisplayPath(activeDocument)}`}
                language={activeDocument.language_id}
                value={activeDocument.text}
                options={{
                  automaticLayout: true,
                  fontFamily: "JetBrains Mono, Cascadia Code, Consolas, monospace",
                  fontLigatures: editorPreferences.fontLigatures,
                  fontSize: editorPreferences.fontSize,
                  lineHeight: editorPreferences.lineHeight,
                  minimap: { enabled: editorPreferences.minimap, scale: 0.75 },
                  mouseWheelZoom: false,
                  padding: { top: 18, bottom: 18 },
                  readOnly: activeDocument.view.mode === "readOnlyText",
                  renderWhitespace: editorPreferences.renderWhitespace,
                  smoothScrolling: editorPreferences.smoothScrolling,
                  scrollBeyondLastLine: false,
                  tabSize: editorPreferences.tabSize,
                  unicodeHighlight: { ambiguousCharacters: editorPreferences.unicodeHighlightAmbiguousCharacters },
                  wordWrap: editorPreferences.wordWrap,
                  renderLineHighlight: "all",
                  glyphMargin: true,
                }}
                onChange={(value, event) => {
                  if (!isEditableDocument) return;
                  applyEditorChanges(activeDocument.id, event, value);
                }}
                onMount={(editor, monaco) => {
                  editorRef.current = editor;
                  monacoRef.current = monaco;
                  disposeLspProviders(lspProviderDisposablesRef.current);
                  lspProviderDisposablesRef.current = activeDocument.view.mode === "editableText" ? registerLspProviders({
                    document: activeDocument,
                    editor,
                    monaco,
                    setPendingEditorReveal,
                    upsertDocument,
                    updateOpenDocuments,
                    t,
                  }) : [];
                  breakpointGutterDisposableRef.current?.dispose();
                  breakpointGutterDisposableRef.current = activeDocument.view.mode === "editableText" ? registerDebugBreakpointGutter(editor, monaco, activeDocument, toggleDebugSourceBreakpoint) : null;
                  applyDiagnosticsMarkers(editor, monaco, group.id, diagnostics);
                  breakpointDecorationsRef.current = applyDebugBreakpointDecorations(editor, monaco, breakpointDecorationsRef.current, activeDocument, sourceBreakpoints, resolvedBreakpoints);
                  revealEditorTarget(editor, consumePendingEditorReveal(activeDocument.id));
                }}
              />
            </Suspense>
          ) : <FilePreviewPane document={activeDocument} />}
        </div>
      </Panel>
    </>
  );
}

function EditorTab({
  active,
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
  const [menuPosition, setMenuPosition] = useState<{ x: number; y: number } | null>(null);

  const closeTab = () => closeDocumentInGroup(groupId, document.id);
  const copyPath = () => {
    if (document.path) void luxCommands.clipboardWriteText(document.path).catch(() => undefined);
  };
  const copyRelativePath = () => void luxCommands.clipboardWriteText(documentRelativePath(document, workspaceRoot)).catch(() => undefined);

  const fileBackedGroups: EditorTabMenuAction[][] = document.path
    ? [
        [
          { label: t("editor.tabMenu.copyPath"), shortcut: "Shift+Alt+C", onClick: copyPath },
          { label: t("editor.tabMenu.copyRelativePath"), shortcut: "Ctrl+M Ctrl+Shift+C", onClick: copyRelativePath },
        ],
        [
          { label: t("editor.tabMenu.revealInFileExplorer"), shortcut: "Shift+Alt+R", onClick: () => document.path && void luxCommands.fsRevealInFileExplorer(document.path).catch(() => undefined) },
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
        onClick={() => setActiveDocumentInGroup(groupId, document.id)}
      >
        <FileCode2 size={15} />
        <span>{name}</span>
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
