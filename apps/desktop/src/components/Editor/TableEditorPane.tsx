import { AlertTriangle, Plus, Table2 } from "lucide-react";
import { useCallback, useMemo } from "react";
import { documentDisplayPath } from "../lib/editor/documents/documents";
import { useTranslation } from "../lib/i18n/useTranslation";
import {
  addTableColumn,
  addTableRow,
  parseTableDocument,
  serializeTableDocument,
  tableColumnCount,
  updateTableCell,
} from "../lib/editor/documents/table-document";
import type { DocumentSnapshot } from "../lib/types/index";

type TableEditorPaneProps = {
  document: DocumentSnapshot;
  onChange: (text: string) => void;
};

export function TableEditorPane({ document, onChange }: TableEditorPaneProps) {
  const { t } = useTranslation();
  const parsed = useMemo(() => parseTableDocument(document.text), [document.text]);

  const commit = useCallback((next: NonNullable<ReturnType<typeof parseTableDocument>>) => {
    onChange(serializeTableDocument(next));
  }, [onChange]);

  if (!parsed) {
    return (
      <div className="spreadsheet-editor-empty">
        <Table2 size={18} />
        <span>{t("tableEditor.parseError")}</span>
      </div>
    );
  }

  const columnCount = tableColumnCount(parsed);

  return (
    <div className="spreadsheet-editor-pane table-editor-pane">
      <div className="spreadsheet-editor-toolbar">
        <div className="spreadsheet-editor-title">
          <Table2 size={17} />
          <div>
            <strong>{documentDisplayPath(document)}</strong>
            <span>{t("tableEditor.fileType", { type: parsed.fileType.toUpperCase() })}</span>
          </div>
        </div>
        <div className="spreadsheet-editor-actions">
          <button className="secondary-button compact" type="button" onClick={() => commit(addTableRow(parsed))}>
            <Plus size={14} />
            {t("tableEditor.addRow")}
          </button>
          <button className="secondary-button compact" type="button" onClick={() => commit(addTableColumn(parsed))}>
            <Plus size={14} />
            {t("tableEditor.addColumn")}
          </button>
        </div>
      </div>
      {parsed.truncated && (
        <div className="spreadsheet-editor-banner">
          <AlertTriangle size={15} />
          <span>{t("tableEditor.truncated")}</span>
        </div>
      )}
      <div className="spreadsheet-editor-grid-wrap">
        <table className="spreadsheet-editor-grid">
          <thead>
            <tr>
              {Array.from({ length: columnCount }, (_, index) => (
                <th key={index}>
                  <input
                    value={parsed.headers[index] ?? ""}
                    onChange={(event) => {
                      const headers = [...parsed.headers];
                      headers[index] = event.target.value;
                      commit({ ...parsed, headers });
                    }}
                  />
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {parsed.rows.map((row, rowIndex) => (
              <tr key={rowIndex}>
                {Array.from({ length: columnCount }, (_, columnIndex) => (
                  <td key={columnIndex}>
                    <input
                      value={row[columnIndex] ?? ""}
                      onChange={(event) => commit(updateTableCell(parsed, rowIndex, columnIndex, event.target.value))}
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