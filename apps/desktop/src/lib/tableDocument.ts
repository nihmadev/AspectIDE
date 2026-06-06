import type { TableEditDocument } from "./types";

export const TABLE_EDIT_FORMAT = "lux-table/v1";

export function parseTableDocument(text: string): TableEditDocument | null {
  if (!text.trim()) return null;
  try {
    const parsed = JSON.parse(text) as TableEditDocument;
    if (parsed.format !== TABLE_EDIT_FORMAT) return null;
    return parsed;
  } catch {
    return null;
  }
}

export function serializeTableDocument(document: TableEditDocument) {
  return JSON.stringify(document, null, 2);
}

export function updateTableCell(
  document: TableEditDocument,
  rowIndex: number,
  columnIndex: number,
  value: string,
): TableEditDocument {
  const rows = document.rows.map((row, index) =>
    index === rowIndex
      ? row.map((cell, col) => (col === columnIndex ? value : cell))
      : row,
  );
  return { ...document, rows };
}

export function addTableRow(document: TableEditDocument): TableEditDocument {
  const width = Math.max(document.headers.length, ...document.rows.map((row) => row.length), 1);
  return {
    ...document,
    rows: [...document.rows, Array.from({ length: width }, () => "")],
  };
}

export function addTableColumn(document: TableEditDocument): TableEditDocument {
  const nextIndex = document.headers.length + 1;
  const header = `Column ${nextIndex}`;
  return {
    ...document,
    headers: [...document.headers, header],
    rows: document.rows.map((row) => [...row, ""]),
  };
}

export function tableColumnCount(document: TableEditDocument) {
  return Math.max(document.headers.length, ...document.rows.map((row) => row.length), 1);
}