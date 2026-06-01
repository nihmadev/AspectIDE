import type { OnMount } from "@monaco-editor/react";
import type { editor, IRange, languages, Position } from "monaco-editor";
import { documentDisplayPath } from "./documents";
import type { TranslateFn } from "./i18n/useTranslation";
import { luxCommands } from "./tauri";
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
} from "./types";

export type MonacoEditorInstance = Parameters<OnMount>[0];
export type MonacoInstance = Parameters<OnMount>[1];
export type MonacoDisposable = { dispose: () => void };

export type EditorRevealTarget = {
  documentId: string;
  line: number;
  column: number;
};

export type RegisterLspProvidersInput = {
  document: DocumentSnapshot;
  editor: MonacoEditorInstance;
  monaco: MonacoInstance;
  setPendingEditorReveal: (target: EditorRevealTarget | null) => void;
  t: TranslateFn;
  updateOpenDocuments: (documents: DocumentSnapshot[]) => void;
  upsertDocument: (document: DocumentSnapshot) => void;
};

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

export function applyDiagnosticsMarkers(
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

export function registerLspProviders({
  document,
  editor,
  monaco,
  setPendingEditorReveal,
  t,
  updateOpenDocuments,
  upsertDocument,
}: RegisterLspProvidersInput): MonacoDisposable[] {
  const model = editor.getModel();
  if (!model || !document.path) return [];
  const selector = { pattern: model.uri.toString() };
  const applyCodeActionCommandId = `lux.applyCodeAction.${document.id}`;
  const applyCodeActionCommand = monaco.editor.registerCommand(applyCodeActionCommandId, (_accessor: unknown, action: LspCodeAction) => {
    if (!action.edit) return;
    void luxCommands.editorApplyWorkspaceEdit(action.edit)
      .then((result) => updateOpenDocuments(result.edited_documents))
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
        updateOpenDocuments(result.edited_documents);
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

export function disposeLspProviders(disposables: MonacoDisposable[]) {
  for (const disposable of disposables) disposable.dispose();
}

export function revealEditorTarget(editorInstance: MonacoEditorInstance | null, target: EditorRevealTarget | null) {
  if (!editorInstance || !target) return;
  const position = {
    lineNumber: Math.max(1, target.line),
    column: Math.max(1, target.column),
  };
  editorInstance.setPosition(position);
  editorInstance.revealPositionInCenter(position, 0);
  editorInstance.focus();
}

export function textEditsFromMonacoEvent(event: editor.IModelContentChangedEvent): TextEdit[] {
  return event.changes.map((change) => ({
    start_line: change.range.startLineNumber,
    start_column: change.range.startColumn,
    end_line: change.range.endLineNumber,
    end_column: change.range.endColumn,
    text: change.text,
  }));
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
  upsertDocument: (document: DocumentSnapshot) => void,
  setPendingEditorReveal: (target: EditorRevealTarget | null) => void,
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

function toMonacoRange(monaco: MonacoInstance, range: LspRange) {
  return new monaco.Range(
    Math.max(1, range.start_line),
    Math.max(1, range.start_column),
    Math.max(1, range.end_line),
    Math.max(1, range.end_column),
  );
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
