import { AlertTriangle, Plus, Table2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { documentDisplayPath } from "../lib/documents";
import { useTranslation } from "../lib/i18n/useTranslation";
import {
  addSpreadsheetColumn,
  addSpreadsheetRow,
  columnLabel,
  normalizeSheetRows,
  parseSpreadsheetDocument,
  serializeSpreadsheetDocument,
  sheetColumnCount,
  updateSpreadsheetCell,
  type SpreadsheetEditDocument,
} from "../lib/spreadsheetDocument";
import type { DocumentSnapshot } from "../lib/types";

type SpreadsheetEditorPaneProps = {
  document: DocumentSnapshot;
  onChange: (text: string) => void;
};

export function SpreadsheetEditorPane({ document, onChange }: SpreadsheetEditorPaneProps) {
  const { t } = useTranslation();
  const parsed = useMemo(() => parseSpreadsheetDocument(document.text), [document.text]);
  const [activeSheetIndex, setActiveSheetIndex] = useState(0);

  useEffect(() => {
    setActiveSheetIndex(0);
  }, [document.id]);

  useEffect(() => {
    if (!parsed) return;
    if (activeSheetIndex < parsed.sheets.length) return;
    setActiveSheetIndex(0);
  }, [activeSheetIndex, parsed]);

  const commit = useCallback((next: SpreadsheetEditDocument) => {
    onChange(serializeSpreadsheetDocument(next));
  }, [onChange]);

  if (!parsed || parsed.sheets.length === 0) {
    return (
      <div className="spreadsheet-editor-empty">
        <Table2 size={18} />
        <span>{t("spreadsheetEditor.parseError")}</span>
      </div>
    );
  }

  const sheet = normalizeSheetRows(parsed.sheets[activeSheetIndex] ?? parsed.sheets[0]);
  const columnCount = Math.max(sheetColumnCount(sheet), 1);
  const saveNote = parsed.workbookType === "xls" ? t("spreadsheetEditor.saveHint.xls") : null;

  return (
    <div className="spreadsheet-editor-pane">
      <div className="spreadsheet-editor-toolbar">
        <div className="spreadsheet-editor-title">
          <Table2 size={17} />
          <div>
            <strong>{documentDisplayPath(document)}</strong>
            <span>{t("spreadsheetEditor.workbookType", { type: parsed.workbookType.toUpperCase() })}</span>
          </div>
        </div>
        <div className="spreadsheet-editor-actions">
          <button
            className="secondary-button compact"
            type="button"
            onClick={() => commit(addSpreadsheetRow(parsed, activeSheetIndex))}
          >
            <Plus size={14} />
            {t("spreadsheetEditor.addRow")}
          </button>
          <button
            className="secondary-button compact"
            type="button"
            onClick={() => commit(addSpreadsheetColumn(parsed, activeSheetIndex))}
          >
            <Plus size={14} />
            {t("spreadsheetEditor.addColumn")}
          </button>
        </div>
      </div>
      {parsed.truncated && (
        <div className="spreadsheet-editor-banner">
          <AlertTriangle size={15} />
          <span>{t("spreadsheetEditor.truncated")}</span>
        </div>
      )}
      {saveNote && (
        <div className="spreadsheet-editor-banner subtle">
          <span>{saveNote}</span>
        </div>
      )}
      <div className="spreadsheet-editor-sheet-tabs" role="tablist" aria-label={t("spreadsheetEditor.sheetTabs")}>
        {parsed.sheets.map((candidate, index) => (
          <button
            className="spreadsheet-editor-sheet-tab"
            data-active={index === activeSheetIndex}
            key={`${candidate.name}-${index}`}
            role="tab"
            type="button"
            aria-selected={index === activeSheetIndex}
            onClick={() => setActiveSheetIndex(index)}
          >
            {candidate.name.trim() || t("spreadsheetEditor.unnamedSheet", { index: index + 1 })}
          </button>
        ))}
      </div>
      <div className="spreadsheet-editor-grid-wrap">
        <table className="spreadsheet-editor-grid">
          <thead>
            <tr>
              <th className="spreadsheet-editor-corner" />
              {Array.from({ length: columnCount }, (_, index) => (
                <th key={index}>{columnLabel(index)}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {sheet.rows.map((row, rowIndex) => (
              <tr key={rowIndex}>
                <th className="spreadsheet-editor-row-index">{rowIndex + 1}</th>
                {Array.from({ length: columnCount }, (_, columnIndex) => (
                  <td key={columnIndex}>
                    <input
                      className="spreadsheet-editor-cell"
                      value={row[columnIndex] ?? ""}
                      spellCheck={false}
                      onChange={(event) => {
                        commit(updateSpreadsheetCell(parsed, activeSheetIndex, rowIndex, columnIndex, event.target.value));
                      }}
                    />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}