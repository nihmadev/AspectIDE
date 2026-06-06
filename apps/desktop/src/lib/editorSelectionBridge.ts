/** Live Monaco selection snapshot for AI composer context. */

export type EditorSelectionSnapshot = {
  documentId: string;
  path: string;
  languageId: string;
  startLine: number;
  endLine: number;
  startColumn: number;
  endColumn: number;
  text: string;
};

type EditorSelectionListener = () => void;

let snapshot: EditorSelectionSnapshot | null = null;
const listeners = new Set<EditorSelectionListener>();

export function getEditorSelectionSnapshot() {
  return snapshot;
}

export function subscribeEditorSelection(listener: EditorSelectionListener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function setEditorSelectionSnapshot(next: EditorSelectionSnapshot | null) {
  if (selectionSnapshotsEqual(snapshot, next)) return;
  snapshot = next;
  for (const listener of listeners) listener();
}

export function formatSelectionLabel(selection: EditorSelectionSnapshot) {
  const file = selection.path.split(/[/\\]/).pop() || selection.path || "selection";
  if (selection.startLine === selection.endLine) {
    return `${file}:${selection.startLine}`;
  }
  return `${file}:${selection.startLine}-${selection.endLine}`;
}

function selectionSnapshotsEqual(left: EditorSelectionSnapshot | null, right: EditorSelectionSnapshot | null) {
  if (left === right) return true;
  if (!left || !right) return false;
  return left.documentId === right.documentId
    && left.path === right.path
    && left.startLine === right.startLine
    && left.endLine === right.endLine
    && left.startColumn === right.startColumn
    && left.endColumn === right.endColumn
    && left.text === right.text;
}