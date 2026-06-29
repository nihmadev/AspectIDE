import type { TextEdit } from "./types";

/** A single edit resolved to absolute [start, end) string offsets of the ORIGINAL
 *  snapshot, paired with its replacement text. */
type ResolvedEdit = {
  start: number;
  end: number;
  text: string;
};

/**
 * Apply LSP/IPC text edits using original-snapshot range semantics.
 *
 * LSP and the native edit bridge compute every range against the document as it
 * was when the edit set was produced — NOT against the text left behind by earlier
 * edits in the same batch. The previous implementation mutated the string after
 * each edit and resolved later line/column positions against the already-mutated
 * text, so any edit that changed the line/column layout silently corrupted the
 * offsets of the edits that followed it (broken AI multi-edits and LSP refactors).
 *
 * Correct algorithm:
 *   1. Resolve every edit to a [start, end) offset against the ORIGINAL text.
 *   2. Reject overlapping edits — overlapping ranges against one snapshot are
 *      ambiguous and must never be applied blindly.
 *   3. Splice the edits in descending start order so each write leaves the offsets
 *      of the not-yet-applied (lower) edits valid.
 */
export function applyTextEdits(text: string, edits: TextEdit[]): string {
  if (edits.length === 0) return text;

  // Resolve against the original text, then order by start (ties keep input order
  // so same-position insertions apply in the order the backend emitted them).
  const ordered = edits
    .map((edit, index) => ({ edit: resolveEdit(text, edit), index }))
    .sort((left, right) => left.edit.start - right.edit.start || left.index - right.index)
    .map((entry) => entry.edit);

  for (let i = 1; i < ordered.length; i += 1) {
    if (ordered[i].start < ordered[i - 1].end) {
      throw new Error("Overlapping text edits cannot be applied against a single snapshot");
    }
  }

  // Apply right-to-left: each splice happens at offsets at or beyond every
  // remaining edit, so the lower offsets we have not consumed yet stay valid.
  let nextText = text;
  for (let i = ordered.length - 1; i >= 0; i -= 1) {
    const { start, end, text: replacement } = ordered[i];
    nextText = `${nextText.slice(0, start)}${replacement}${nextText.slice(end)}`;
  }
  return nextText;
}

function resolveEdit(text: string, edit: TextEdit): ResolvedEdit {
  const start = positionToStringOffset(text, edit.start_line, edit.start_column);
  const end = positionToStringOffset(text, edit.end_line, edit.end_column);
  if (end < start) throw new Error("Text edit end position precedes its start position");
  return { start, end, text: edit.text };
}

/** Convert a 1-based line/column into a string offset, counting astral characters
 *  (code points > U+FFFF) as two UTF-16 units to match how the backend addresses
 *  the buffer. Throws when the position falls outside the document. */
export function positionToStringOffset(text: string, line: number, column: number): number {
  if (line < 1 || column < 1) throw new Error("Text edit positions are 1-based");

  let currentLine = 1;
  let currentColumn = 1;
  for (let index = 0; index < text.length;) {
    if (currentLine === line && currentColumn === column) return index;

    const codePoint = text.codePointAt(index);
    if (codePoint === undefined) break;
    const width = codePoint > 0xffff ? 2 : 1;
    const char = text.slice(index, index + width);
    index += width;

    if (char === "\n") {
      currentLine += 1;
      currentColumn = 1;
    } else {
      currentColumn += width;
    }
  }

  if (currentLine === line && currentColumn === column) return text.length;
  throw new Error(`Text edit position ${line}:${column} is outside the document`);
}
