import { Circle, FileCode2, Minus, Plus, RotateCcw, Save, SaveAll, SquareSplitHorizontal, X } from "lucide-react";
import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { Panel, Group, Separator } from "react-resizable-panels";
import { useMutation } from "@tanstack/react-query";
import type { OnMount } from "@monaco-editor/react";
import type { editor, IRange, languages, Position } from "monaco-editor";
import { useEditorCloseGuard } from "./EditorCloseGuard";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
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
import { useLuxStore, type Activity, type EditorGroup } from "../lib/store";
import { luxCommands } from "../lib/tauri";
import type {
  DiagnosticSeverity,
  DocumentSnapshot,
  LspCodeAction,
  LspCodeActionDiagnostic,
  LspCompletionItem,
  LspCompletionItemKind,
  LspDocumentSymbol,
  LspFoldingRangeKind,
  LspFormattingOptions,
  LspLocation,
  LspRange,
  LspSemanticTokens,
  LspSignatureHelp,
  LspSymbolKind,
  TextEdit,
  WorkspaceDiagnostic,
} from "../lib/types";

const MonacoEditor = lazy(() => import("@monaco-editor/react"));
const noDiagnostics: WorkspaceDiagnostic[] = [];
const semanticTokenTypes = [
  "namespace",
  "type",
  "class",
  "enum",
  "interface",
  "struct",
  "typeParameter",
  "parameter",
  "variable",
  "property",
  "enumMember",
  "event",
  "function",
  "method",
  "macro",
  "keyword",
  "modifier",
  "comment",
  "string",
  "number",
  "regexp",
  "operator",
  "decorator",
];
const semanticTokenModifiers = [
  "declaration",
  "definition",
  "readonly",
  "static",
  "deprecated",
  "abstract",
  "async",
  "modification",
  "documentation",
  "defaultLibrary",
];

type MonacoEditorInstance = Parameters<OnMount>[0];
type MonacoInstance = Parameters<OnMount>[1];
type MonacoDisposable = { dispose: () => void };

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
  const pendingEditorReveal = useLuxStore((state) => state.pendingEditorReveal);
  const setPendingEditorReveal = useLuxStore((state) => state.setPendingEditorReveal);
  const consumePendingEditorReveal = useLuxStore((state) => state.consumePendingEditorReveal);
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
              applyEditorChanges={applyEditorChanges}
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
  applyEditorChanges,
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
  applyEditorChanges: (id: string, event: editor.IModelContentChangedEvent, value: string | undefined) => void;
  upsertDocument: ReturnType<typeof useLuxStore.getState>["upsertDocument"];
  workspaceRoot: string | null;
}) {
  const { t } = useTranslation();
  const isActiveGroup = activeEditorGroupId === group.id;
  const editorRef = useRef<MonacoEditorInstance | null>(null);
  const monacoRef = useRef<MonacoInstance | null>(null);
  const lspProviderDisposablesRef = useRef<MonacoDisposable[]>([]);
  const diagnostics = activeDocument.path ? diagnosticsByPath[normalizePath(activeDocument.path)] ?? noDiagnostics : noDiagnostics;

  useEffect(() => () => {
    disposeLspProviders(lspProviderDisposablesRef.current);
    lspProviderDisposablesRef.current = [];
  }, []);

  useEffect(() => {
    applyDiagnosticsMarkers(editorRef.current, monacoRef.current, group.id, diagnostics);
  }, [activeDocument.path, diagnostics, group.id]);

  useEffect(() => {
    if (pendingEditorReveal?.documentId !== activeDocument.id) return;
    revealEditorTarget(editorRef.current, consumePendingEditorReveal(activeDocument.id));
  }, [activeDocument.id, consumePendingEditorReveal, pendingEditorReveal]);

  useEffect(() => {
    if (!editorRef.current || !monacoRef.current) return;
    disposeLspProviders(lspProviderDisposablesRef.current);
    lspProviderDisposablesRef.current = registerLspProviders({
      document: activeDocument,
      editor: editorRef.current,
      monaco: monacoRef.current,
      setPendingEditorReveal,
      upsertDocument,
      t,
    });
  }, [activeDocument, setPendingEditorReveal, upsertDocument, t]);

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
              onClick={() => saveDocument(activeDocument.id)}
            >
              <Save size={15} />
            </button>
          </div>
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
                renderWhitespace: editorPreferences.renderWhitespace,
                smoothScrolling: editorPreferences.smoothScrolling,
                scrollBeyondLastLine: false,
                tabSize: editorPreferences.tabSize,
                unicodeHighlight: { ambiguousCharacters: editorPreferences.unicodeHighlightAmbiguousCharacters },
                wordWrap: editorPreferences.wordWrap,
                renderLineHighlight: "all",
              }}
              onChange={(value, event) => {
                applyEditorChanges(activeDocument.id, event, value);
              }}
              onMount={(editor, monaco) => {
                editorRef.current = editor;
                monacoRef.current = monaco;
                disposeLspProviders(lspProviderDisposablesRef.current);
                lspProviderDisposablesRef.current = registerLspProviders({
                  document: activeDocument,
                  editor,
                  monaco,
                  setPendingEditorReveal,
                  upsertDocument,
                  t,
                });
                applyDiagnosticsMarkers(editor, monaco, group.id, diagnostics);
                revealEditorTarget(editor, consumePendingEditorReveal(activeDocument.id));
              }}
            />
          </Suspense>
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
      { label: t("common.save"), shortcut: "Ctrl+S", disabled: !document.is_dirty, onClick: () => saveDocument(document.id) },
      { label: t("common.saveAs"), shortcut: "Ctrl+Shift+S", onClick: () => saveDocumentAs(document.id) },
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

function applyDiagnosticsMarkers(
  editorInstance: MonacoEditorInstance | null,
  monaco: MonacoInstance | null,
  owner: string,
  diagnostics: WorkspaceDiagnostic[],
) {
  const model = editorInstance?.getModel();
  if (!model || !monaco) return;
  monaco.editor.setModelMarkers(
    model,
    `lux-lsp:${owner}`,
    diagnostics.map((diagnostic) => toMonacoMarker(monaco, diagnostic)),
  );
}

function registerLspProviders({
  document,
  editor,
  monaco,
  setPendingEditorReveal,
  upsertDocument,
  t,
}: {
  document: DocumentSnapshot;
  editor: MonacoEditorInstance;
  monaco: MonacoInstance;
  setPendingEditorReveal: ReturnType<typeof useLuxStore.getState>["setPendingEditorReveal"];
  upsertDocument: ReturnType<typeof useLuxStore.getState>["upsertDocument"];
  t: TranslateFn;
}): MonacoDisposable[] {
  const model = editor.getModel();
  if (!model || !document.path) return [];
  const selector = { pattern: model.uri.toString() };
  const applyCodeActionCommandId = `lux.applyCodeAction.${document.id}`;
  const applyCodeActionCommand = monaco.editor.registerCommand(applyCodeActionCommandId, (_accessor: unknown, action: LspCodeAction) => {
    if (!action.edit) return;
    void luxCommands.editorApplyWorkspaceEdit(action.edit)
      .then((result) => useLuxStore.getState().updateOpenDocuments(result.edited_documents))
      .catch(() => undefined);
  });

  return [
    applyCodeActionCommand,
    monaco.languages.registerHoverProvider(selector, {
      provideHover: async (_model: editor.ITextModel, position: Position) => {
        const hover = await luxCommands.lspHover(document.id, position.lineNumber, position.column).catch(() => null);
        if (!hover || hover.contents.length === 0) return null;
        return {
          range: hover.range ? toMonacoRange(monaco, hover.range) : undefined,
          contents: hover.contents.map((value) => ({ value })),
        };
      },
    }),
    monaco.languages.registerDefinitionProvider(selector, {
      provideDefinition: async (_model: editor.ITextModel, position: Position) => {
        const locations = await luxCommands.lspDefinition(document.id, position.lineNumber, position.column).catch(() => []);
        if (locations.length === 0) return [];
        const results: languages.Location[] = [];
        for (const location of locations) {
          const target = await openLspLocation(location, monaco, upsertDocument, setPendingEditorReveal);
          if (target) results.push(target);
        }
        return results;
      },
    }),
    monaco.languages.registerReferenceProvider(selector, {
      provideReferences: async (_model: editor.ITextModel, position: Position) => {
        const locations = await luxCommands.lspReferences(document.id, position.lineNumber, position.column).catch(() => []);
        if (locations.length === 0) return [];
        const results: languages.Location[] = [];
        for (const location of locations) {
          const target = await openLspLocation(location, monaco, upsertDocument, setPendingEditorReveal);
          if (target) results.push(target);
        }
        return results;
      },
    }),
    monaco.languages.registerDocumentSymbolProvider(selector, {
      displayName: "Lux LSP",
      provideDocumentSymbols: async () => {
        const symbols = await luxCommands.lspDocumentSymbols(document.id).catch(() => []);
        return symbols.map((symbol) => toMonacoDocumentSymbol(monaco, symbol));
      },
    }),
    monaco.languages.registerFoldingRangeProvider(selector, {
      provideFoldingRanges: async () => {
        const ranges = await luxCommands.lspFoldingRanges(document.id).catch(() => []);
        return ranges.map((range) => ({
          start: range.start_line,
          end: range.end_line,
          kind: range.kind ? toMonacoFoldingRangeKind(monaco, range.kind) : undefined,
        }));
      },
    }),
    monaco.languages.registerInlayHintsProvider(selector, {
      displayName: "Lux LSP",
      provideInlayHints: async (_model: editor.ITextModel, range: IRange) => {
        const hints = await luxCommands.lspInlayHints(document.id, lspRangeFromMonacoRange(range)).catch(() => []);
        return {
          hints: hints.map((hint) => ({
            label: hint.label,
            tooltip: hint.tooltip ? { value: hint.tooltip } : undefined,
            position: { lineNumber: hint.line, column: hint.column },
            kind: hint.kind === "type" ? monaco.languages.InlayHintKind.Type : hint.kind === "parameter" ? monaco.languages.InlayHintKind.Parameter : undefined,
            paddingLeft: hint.padding_left || undefined,
            paddingRight: hint.padding_right || undefined,
          })),
          dispose: () => undefined,
        };
      },
    }),
    monaco.languages.registerDocumentSemanticTokensProvider(selector, {
      getLegend: () => ({ tokenTypes: semanticTokenTypes, tokenModifiers: semanticTokenModifiers }),
      provideDocumentSemanticTokens: async () => toMonacoSemanticTokens(await luxCommands.lspSemanticTokens(document.id).catch(() => null)),
      releaseDocumentSemanticTokens: () => undefined,
    }),
    monaco.languages.registerRenameProvider(selector, {
      provideRenameEdits: async (_model: editor.ITextModel, position: Position, newName: string) => {
        const result = await luxCommands.lspRename(document.id, position.lineNumber, position.column, newName).catch(() => null);
        if (!result) return { edits: [], rejectReason: t("editor.status.renameFailed") };
        useLuxStore.getState().updateOpenDocuments(result.edited_documents);
        return { edits: [] };
      },
    }),
    monaco.languages.registerCompletionItemProvider(selector, {
      triggerCharacters: [".", ":", "<", "\"", "'", "/", "@", "#"],
      provideCompletionItems: async (model: editor.ITextModel, position: Position) => {
        const completion = await luxCommands.lspCompletion(document.id, position.lineNumber, position.column).catch(() => ({
          is_incomplete: false,
          items: [] as LspCompletionItem[],
        }));
        if (completion.items.length === 0) return { suggestions: [], incomplete: completion.is_incomplete };
        const word = model.getWordUntilPosition(position);
        const fallbackRange = new monaco.Range(position.lineNumber, word.startColumn, position.lineNumber, word.endColumn);
        return {
          incomplete: completion.is_incomplete,
          suggestions: completion.items.map((item) => toMonacoCompletionItem(monaco, item, fallbackRange)),
        };
      },
    }),
    monaco.languages.registerSignatureHelpProvider(selector, {
      signatureHelpTriggerCharacters: ["(", ",", "<"],
      signatureHelpRetriggerCharacters: [",", ")", ">"],
      provideSignatureHelp: async (_model: editor.ITextModel, position: Position) => {
        const signatureHelp = await luxCommands.lspSignatureHelp(document.id, position.lineNumber, position.column).catch(() => null);
        if (!signatureHelp || signatureHelp.signatures.length === 0) return null;
        return {
          value: toMonacoSignatureHelp(signatureHelp),
          dispose: () => undefined,
        };
      },
    }),
    monaco.languages.registerCodeActionProvider(selector, {
      provideCodeActions: async (_model: editor.ITextModel, range: IRange, context: languages.CodeActionContext) => {
        const actions = await luxCommands.lspCodeActions(
          document.id,
          lspRangeFromMonacoRange(range),
          context.markers.map(lspCodeActionDiagnosticFromMarker),
          context.only ? [context.only] : null,
          context.trigger === monaco.languages.CodeActionTriggerType.Auto ? "automatic" : "invoke",
        ).catch(() => [] as LspCodeAction[]);
        return {
          actions: actions.map((action) => toMonacoCodeAction(action, applyCodeActionCommandId)),
          dispose: () => undefined,
        };
      },
    }, { providedCodeActionKinds: ["quickfix", "refactor", "source", "source.organizeImports", "source.fixAll"] }),
    monaco.languages.registerDocumentFormattingEditProvider(selector, {
      displayName: "Lux LSP",
      provideDocumentFormattingEdits: async (_model: editor.ITextModel, options: languages.FormattingOptions) => {
        const edits = await luxCommands.lspFormatDocument(document.id, lspFormattingOptionsFromMonaco(options)).catch(() => []);
        return edits.map((edit) => ({ range: toMonacoRange(monaco, edit.range), text: edit.text }));
      },
    }),
    monaco.languages.registerDocumentRangeFormattingEditProvider(selector, {
      displayName: "Lux LSP",
      provideDocumentRangeFormattingEdits: async (_model: editor.ITextModel, range: IRange, options: languages.FormattingOptions) => {
        const edits = await luxCommands.lspFormatRange(document.id, lspRangeFromMonacoRange(range), lspFormattingOptionsFromMonaco(options)).catch(() => []);
        return edits.map((edit) => ({ range: toMonacoRange(monaco, edit.range), text: edit.text }));
      },
    }),
  ];
}

function toMonacoCodeAction(action: LspCodeAction, applyCodeActionCommandId: string): languages.CodeAction {
  return {
    title: action.title,
    kind: action.kind ?? undefined,
    isPreferred: action.is_preferred || undefined,
    disabled: action.disabled_reason ?? undefined,
    command: action.edit
      ? {
          id: applyCodeActionCommandId,
          title: action.title,
          arguments: [action],
        }
      : undefined,
  };
}

function toMonacoSemanticTokens(tokens: LspSemanticTokens | null): languages.SemanticTokens | null {
  if (!tokens || tokens.data.length === 0) return null;
  return {
    resultId: tokens.result_id ?? undefined,
    data: new Uint32Array(tokens.data),
  };
}

function toMonacoDocumentSymbol(monaco: MonacoInstance, symbol: LspDocumentSymbol): languages.DocumentSymbol {
  return {
    name: symbol.name,
    detail: symbol.detail ?? "",
    kind: toMonacoSymbolKind(monaco, symbol.kind),
    tags: [],
    range: toMonacoRange(monaco, symbol.range),
    selectionRange: toMonacoRange(monaco, symbol.selection_range),
    children: symbol.children.length > 0 ? symbol.children.map((child) => toMonacoDocumentSymbol(monaco, child)) : undefined,
  };
}

function toMonacoFoldingRangeKind(monaco: MonacoInstance, kind: LspFoldingRangeKind): languages.FoldingRangeKind {
  if (kind === "comment") return monaco.languages.FoldingRangeKind.Comment;
  if (kind === "imports") return monaco.languages.FoldingRangeKind.Imports;
  return monaco.languages.FoldingRangeKind.Region;
}

function toMonacoSymbolKind(monaco: MonacoInstance, kind: LspSymbolKind): languages.SymbolKind {
  if (kind === "file") return monaco.languages.SymbolKind.File;
  if (kind === "module") return monaco.languages.SymbolKind.Module;
  if (kind === "namespace") return monaco.languages.SymbolKind.Namespace;
  if (kind === "package") return monaco.languages.SymbolKind.Package;
  if (kind === "class") return monaco.languages.SymbolKind.Class;
  if (kind === "method") return monaco.languages.SymbolKind.Method;
  if (kind === "property") return monaco.languages.SymbolKind.Property;
  if (kind === "field") return monaco.languages.SymbolKind.Field;
  if (kind === "constructor") return monaco.languages.SymbolKind.Constructor;
  if (kind === "enum") return monaco.languages.SymbolKind.Enum;
  if (kind === "interface") return monaco.languages.SymbolKind.Interface;
  if (kind === "function") return monaco.languages.SymbolKind.Function;
  if (kind === "variable") return monaco.languages.SymbolKind.Variable;
  if (kind === "constant") return monaco.languages.SymbolKind.Constant;
  if (kind === "string") return monaco.languages.SymbolKind.String;
  if (kind === "number") return monaco.languages.SymbolKind.Number;
  if (kind === "boolean") return monaco.languages.SymbolKind.Boolean;
  if (kind === "array") return monaco.languages.SymbolKind.Array;
  if (kind === "object") return monaco.languages.SymbolKind.Object;
  if (kind === "key") return monaco.languages.SymbolKind.Key;
  if (kind === "null") return monaco.languages.SymbolKind.Null;
  if (kind === "enumMember") return monaco.languages.SymbolKind.EnumMember;
  if (kind === "struct") return monaco.languages.SymbolKind.Struct;
  if (kind === "event") return monaco.languages.SymbolKind.Event;
  if (kind === "operator") return monaco.languages.SymbolKind.Operator;
  if (kind === "typeParameter") return monaco.languages.SymbolKind.TypeParameter;
  return monaco.languages.SymbolKind.Variable;
}

function lspCodeActionDiagnosticFromMarker(marker: editor.IMarkerData): LspCodeActionDiagnostic {
  return {
    range: {
      start_line: marker.startLineNumber,
      start_column: marker.startColumn,
      end_line: marker.endLineNumber,
      end_column: marker.endColumn,
    },
    severity: lspSeverityFromMonacoMarkerSeverity(marker.severity),
    source: marker.source ?? null,
    message: marker.message,
  };
}

function lspSeverityFromMonacoMarkerSeverity(severity: number): DiagnosticSeverity | null {
  if (severity === 8) return "error";
  if (severity === 4) return "warning";
  if (severity === 2) return "information";
  if (severity === 1) return "hint";
  return null;
}

function lspFormattingOptionsFromMonaco(options: languages.FormattingOptions): LspFormattingOptions {
  return {
    tab_size: options.tabSize,
    insert_spaces: options.insertSpaces,
  };
}

function lspRangeFromMonacoRange(range: IRange): LspRange {
  return {
    start_line: range.startLineNumber,
    start_column: range.startColumn,
    end_line: range.endLineNumber,
    end_column: range.endColumn,
  };
}

function toMonacoSignatureHelp(signatureHelp: LspSignatureHelp): languages.SignatureHelp {
  return {
    activeSignature: signatureHelp.active_signature ?? 0,
    activeParameter: signatureHelp.active_parameter ?? 0,
    signatures: signatureHelp.signatures.map((signature) => ({
      label: signature.label,
      documentation: signature.documentation ? { value: signature.documentation } : undefined,
      parameters: signature.parameters.map((parameter) => ({
        label: parameter.label,
        documentation: parameter.documentation ? { value: parameter.documentation } : undefined,
      })),
      activeParameter: signature.active_parameter ?? undefined,
    })),
  };
}

function toMonacoCompletionItem(monaco: MonacoInstance, item: LspCompletionItem, fallbackRange: languages.CompletionItemRanges): languages.CompletionItem {
  return {
    label: item.label,
    kind: toMonacoCompletionKind(monaco, item.kind),
    detail: item.detail ?? undefined,
    documentation: item.documentation ? { value: item.documentation } : undefined,
    insertText: item.insert_text,
    insertTextRules: item.insert_text_format === "snippet" ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet : undefined,
    range: item.range ? toMonacoRange(monaco, item.range) : fallbackRange,
    filterText: item.filter_text ?? undefined,
    sortText: item.sort_text ?? undefined,
    commitCharacters: item.commit_characters.length > 0 ? item.commit_characters : undefined,
    preselect: item.preselect || undefined,
  };
}

function toMonacoCompletionKind(monaco: MonacoInstance, kind: LspCompletionItemKind | null): languages.CompletionItemKind {
  if (kind === "method") return monaco.languages.CompletionItemKind.Method;
  if (kind === "function") return monaco.languages.CompletionItemKind.Function;
  if (kind === "constructor") return monaco.languages.CompletionItemKind.Constructor;
  if (kind === "field") return monaco.languages.CompletionItemKind.Field;
  if (kind === "variable") return monaco.languages.CompletionItemKind.Variable;
  if (kind === "class") return monaco.languages.CompletionItemKind.Class;
  if (kind === "interface") return monaco.languages.CompletionItemKind.Interface;
  if (kind === "module") return monaco.languages.CompletionItemKind.Module;
  if (kind === "property") return monaco.languages.CompletionItemKind.Property;
  if (kind === "unit") return monaco.languages.CompletionItemKind.Unit;
  if (kind === "value") return monaco.languages.CompletionItemKind.Value;
  if (kind === "enum") return monaco.languages.CompletionItemKind.Enum;
  if (kind === "keyword") return monaco.languages.CompletionItemKind.Keyword;
  if (kind === "snippet") return monaco.languages.CompletionItemKind.Snippet;
  if (kind === "color") return monaco.languages.CompletionItemKind.Color;
  if (kind === "file") return monaco.languages.CompletionItemKind.File;
  if (kind === "reference") return monaco.languages.CompletionItemKind.Reference;
  if (kind === "folder") return monaco.languages.CompletionItemKind.Folder;
  if (kind === "enumMember") return monaco.languages.CompletionItemKind.EnumMember;
  if (kind === "constant") return monaco.languages.CompletionItemKind.Constant;
  if (kind === "struct") return monaco.languages.CompletionItemKind.Struct;
  if (kind === "event") return monaco.languages.CompletionItemKind.Event;
  if (kind === "operator") return monaco.languages.CompletionItemKind.Operator;
  if (kind === "typeParameter") return monaco.languages.CompletionItemKind.TypeParameter;
  return monaco.languages.CompletionItemKind.Text;
}

async function openLspLocation(
  location: LspLocation,
  monaco: MonacoInstance,
  upsertDocument: ReturnType<typeof useLuxStore.getState>["upsertDocument"],
  setPendingEditorReveal: ReturnType<typeof useLuxStore.getState>["setPendingEditorReveal"],
) {
  try {
    const document = await luxCommands.editorOpenFile(location.path);
    upsertDocument(document);
    setPendingEditorReveal({
      documentId: document.id,
      line: location.range.start_line,
      column: location.range.start_column,
    });
    return {
      uri: monaco.Uri.parse(`${document.id}:${documentDisplayPath(document)}`),
      range: toMonacoRange(monaco, location.range),
    } satisfies languages.Location;
  } catch {
    return null;
  }
}

function disposeLspProviders(disposables: MonacoDisposable[]) {
  for (const disposable of disposables) disposable.dispose();
}

function toMonacoRange(monaco: MonacoInstance, range: LspRange) {
  return new monaco.Range(
    Math.max(1, range.start_line),
    Math.max(1, range.start_column),
    Math.max(1, range.end_line),
    Math.max(1, range.end_column),
  );
}

function revealEditorTarget(editorInstance: MonacoEditorInstance | null, target: ReturnType<typeof useLuxStore.getState>["pendingEditorReveal"]) {
  if (!editorInstance || !target) return;
  const position = {
    lineNumber: Math.max(1, target.line),
    column: Math.max(1, target.column),
  };
  editorInstance.setPosition(position);
  editorInstance.revealPositionInCenter(position, 0);
  editorInstance.focus();
}

function textEditsFromMonacoEvent(event: editor.IModelContentChangedEvent): TextEdit[] {
  return event.changes.map((change) => ({
    start_line: change.range.startLineNumber,
    start_column: change.range.startColumn,
    end_line: change.range.endLineNumber,
    end_column: change.range.endColumn,
    text: change.text,
  }));
}

function toMonacoMarker(monaco: MonacoInstance, diagnostic: WorkspaceDiagnostic): editor.IMarkerData {
  const startLineNumber = Math.max(1, diagnostic.line);
  const startColumn = Math.max(1, diagnostic.column);
  return {
    severity: toMonacoSeverity(monaco, diagnostic.severity),
    message: diagnostic.message,
    source: diagnostic.source,
    startLineNumber,
    startColumn,
    endLineNumber: startLineNumber,
    endColumn: startColumn + 1,
  };
}

function toMonacoSeverity(monaco: MonacoInstance, severity: DiagnosticSeverity) {
  if (severity === "error") return monaco.MarkerSeverity.Error;
  if (severity === "warning") return monaco.MarkerSeverity.Warning;
  if (severity === "hint") return monaco.MarkerSeverity.Hint;
  return monaco.MarkerSeverity.Info;
}
