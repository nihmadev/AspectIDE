export const SPREADSHEET_EDIT_FORMAT = "lux-spreadsheet/v1";

export type SpreadsheetEditSheet = {
  name: string;
  rows: string[][];
};

export type SpreadsheetEditDocument = {
  format: string;
  workbookType: string;
  truncated: boolean;
  sheets: SpreadsheetEditSheet[];
};

export function parseSpreadsheetDocument(text: string): SpreadsheetEditDocument | null {
  try {
    const parsed = JSON.parse(text) as SpreadsheetEditDocument;
    if (parsed.format !== SPREADSHEET_EDIT_FORMAT || !Array.isArray(parsed.sheets)) return null;
    return parsed;
  } catch {
    return null;
  }
}

export function serializeSpreadsheetDocument(document: SpreadsheetEditDocument) {
  return JSON.stringify(document, null, 2);
}

export function columnLabel(index: number) {
  let column = index + 1;
  let label = "";
  while (column > 0) {
    const remainder = (column - 1) % 26;
    label = String.fromCharCode(65 + remainder) + label;
    column = Math.floor((column - 1) / 26);
  }
  return label;
}

export function sheetColumnCount(sheet: SpreadsheetEditSheet) {
  return sheet.rows.reduce((max, row) => Math.max(max, row.length), 0);
}

export function normalizeSheetRows(sheet: SpreadsheetEditSheet, minColumns = 1, minRows = 1) {
  const columns = Math.max(sheetColumnCount(sheet), minColumns);
  const rows = [...sheet.rows];
  while (rows.length < minRows) rows.push([]);
  return {
    ...sheet,
    rows: rows.map((row) => {
      const next = [...row];
      while (next.length < columns) next.push("");
      return next;
    }),
  };
}

export function updateSpreadsheetCell(
  document: SpreadsheetEditDocument,
  sheetIndex: number,
  rowIndex: number,
  columnIndex: number,
  value: string,
): SpreadsheetEditDocument {
  const sheets = document.sheets.map((sheet, index) => {
    if (index !== sheetIndex) return sheet;
    const normalized = normalizeSheetRows(sheet, columnIndex + 1, rowIndex + 1);
    const rows = normalized.rows.map((row, rIndex) => {
      if (rIndex !== rowIndex) return row;
      const next = [...row];
      next[columnIndex] = value;
      return next;
    });
    return { ...normalized, rows };
  });
  return { ...document, sheets };
}

export function addSpreadsheetRow(document: SpreadsheetEditDocument, sheetIndex: number) {
  const sheets = document.sheets.map((sheet, index) => {
    if (index !== sheetIndex) return sheet;
    const normalized = normalizeSheetRows(sheet);
    const columns = sheetColumnCount(normalized);
    return {
      ...normalized,
      rows: [...normalized.rows, Array.from({ length: Math.max(columns, 1) }, () => "")],
    };
  });
  return { ...document, sheets };
}

export function addSpreadsheetColumn(document: SpreadsheetEditDocument, sheetIndex: number) {
  const sheets = document.sheets.map((sheet, index) => {
    if (index !== sheetIndex) return sheet;
    const normalized = normalizeSheetRows(sheet);
    return {
      ...normalized,
      rows: normalized.rows.map((row) => [...row, ""]),
    };
  });
  return { ...document, sheets };
}