import type { DocumentSnapshot, FileOpenMode } from "./../../types/index";
import { displayPath, normalizePath } from "./../../explorer/file-tree";

export function documentTitle(document: DocumentSnapshot) {
  return document.title || (document.path ? displayPath(document.path).split("/").pop() : null) || "Untitled";
}

export function documentDisplayPath(document: DocumentSnapshot) {
  return document.path ? displayPath(document.path) : document.title;
}

export function documentParentLabel(document: DocumentSnapshot) {
  if (!document.path) return "Unsaved editor";
  const normalized = displayPath(document.path);
  const index = normalized.lastIndexOf("/");
  return index === -1 ? normalized : normalized.slice(0, index);
}

export function documentRelativePath(document: DocumentSnapshot, root: string | null) {
  if (!document.path) return document.title;
  if (!root) return displayPath(document.path);
  const normalizedRoot = displayPath(root).replace(/\/+$/, "");
  const normalizedPath = displayPath(document.path);
  return normalizePath(normalizedPath).startsWith(`${normalizePath(normalizedRoot)}/`)
    ? normalizedPath.slice(normalizedRoot.length + 1)
    : normalizedPath;
}

export function isEditorTextMode(mode: FileOpenMode) {
  return mode === "editableText" || mode === "readOnlyText";
}

export function isEditorTextDocument(document: Pick<DocumentSnapshot, "view">) {
  return isEditorTextMode(document.view.mode);
}

/** Monaco-backed text editor (excludes spreadsheet workbooks and markdown split view). */
export function isMonacoTextDocument(document: Pick<DocumentSnapshot, "view">) {
  return isEditorTextMode(document.view.mode)
    && document.view.strategy !== "spreadsheetEditor"
    && document.view.strategy !== "tableEditor"
    && document.view.strategy !== "markdownPreview"
    && document.view.strategy !== "diagramPreview";
}

export function isSpreadsheetEditorDocument(document: Pick<DocumentSnapshot, "view">) {
  return document.view.strategy === "spreadsheetEditor";
}

export function isTableEditorDocument(document: Pick<DocumentSnapshot, "view">) {
  return document.view.strategy === "tableEditor";
}

export function isDatabaseEditorDocument(document: Pick<DocumentSnapshot, "view">) {
  return document.view.strategy === "databaseEditor";
}

export function isDiagramPreviewDocument(document: Pick<DocumentSnapshot, "view">) {
  return document.view.strategy === "diagramPreview";
}

export function isEditableTextDocument(document: Pick<DocumentSnapshot, "view">) {
  return document.view.mode === "editableText";
}

export function isImagePreviewDocument(document: Pick<DocumentSnapshot, "view">) {
  return document.view.strategy === "imagePreview";
}

export function isPdfPreviewDocument(document: Pick<DocumentSnapshot, "view">) {
  return document.view.strategy === "pdfPreview";
}

export function isMarkdownPreviewDocument(document: Pick<DocumentSnapshot, "view">) {
  return document.view.strategy === "markdownPreview";
}

export function isPreviewDocument(document: Pick<DocumentSnapshot, "view">) {
  return document.view.mode === "preview" || document.view.mode === "external";
}