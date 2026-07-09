import type { DocumentSnapshot, FileViewStrategy } from "./../../types/index";
import { isAgentBrowserPreviewDocument } from "./../../agent-browser/preview-document";
import {
  isDatabaseEditorDocument,
  isDiagramPreviewDocument,
  isEditableTextDocument,
  isImagePreviewDocument,
  isMarkdownPreviewDocument,
  isMonacoTextDocument,
  isPdfPreviewDocument,
  isSpreadsheetEditorDocument,
  isTableEditorDocument,
} from "./documents";

export type EditorPaneKind =
  | "monaco"
  | "markdown"
  | "diagram"
  | "spreadsheet"
  | "table"
  | "database"
  | "image"
  | "pdf"
  | "media"
  | "browserPreview"
  | "structuredPreview";

export function isMediaPreviewDocument(document: Pick<DocumentSnapshot, "view">) {
  const strategy = document.view.strategy;
  return strategy === "audioPreview" || strategy === "videoPreview";
}

/** Database, office, archive, binary, and other non-media preview tabs. */
export function isStructuredPreviewDocument(document: Pick<DocumentSnapshot, "view">) {
  if (isImagePreviewDocument(document) || isPdfPreviewDocument(document) || isMediaPreviewDocument(document)) {
    return false;
  }
  if (
    isSpreadsheetEditorDocument(document)
    || isTableEditorDocument(document)
    || isDatabaseEditorDocument(document)
    || isMonacoTextDocument(document)
    || isMarkdownPreviewDocument(document)
    || isDiagramPreviewDocument(document)
  ) {
    return false;
  }
  return document.view.mode === "preview" || document.view.mode === "external";
}

export function resolveEditorPaneKind(document: Pick<DocumentSnapshot, "view" | "path">): EditorPaneKind {
  if (isAgentBrowserPreviewDocument(document)) return "browserPreview";
  if (isDatabaseEditorDocument(document)) return "database";
  if (isSpreadsheetEditorDocument(document)) return "spreadsheet";
  if (isTableEditorDocument(document)) return "table";
  if (isDiagramPreviewDocument(document)) return "diagram";
  if (isMarkdownPreviewDocument(document)) return "markdown";
  if (isImagePreviewDocument(document)) return "image";
  if (isPdfPreviewDocument(document)) return "pdf";
  if (isMediaPreviewDocument(document)) return "media";
  if (isMonacoTextDocument(document)) return "monaco";
  return "structuredPreview";
}

export function strategyLabel(strategy: FileViewStrategy) {
  return strategy.replace(/Preview$/, "").replace(/^monacoText$/, "text");
}

export function isEditableInEditor(document: Pick<DocumentSnapshot, "view">) {
  return isEditableTextDocument(document);
}