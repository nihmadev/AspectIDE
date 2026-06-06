import { Database, Play, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { documentDisplayPath } from "../lib/documents";
import { useTranslation } from "../lib/i18n/useTranslation";
import { luxCommands } from "../lib/tauri";
import type { DatabaseTablePreview, DocumentSnapshot } from "../lib/types";

const dbOptions = { maxTextBytes: 1_000_000, maxRows: 200, maxColumns: 48, maxArchiveEntries: 0 };

type TableView = {
  name: string;
  kind: string;
  columns: string[];
  rowids: number[];
  rows: string[][];
};

type DatabaseEditorPaneProps = {
  document: DocumentSnapshot;
};

export function DatabaseEditorPane({ document }: DatabaseEditorPaneProps) {
  const { t } = useTranslation();
  const path = document.path;
  const [tables, setTables] = useState<DatabaseTablePreview[]>([]);
  const [activeTable, setActiveTable] = useState<string | null>(null);
  const [tableView, setTableView] = useState<TableView | null>(null);
  const [sql, setSql] = useState("SELECT name, type FROM sqlite_schema WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name;");
  const [resultMessage, setResultMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const loadTables = useCallback(async () => {
    if (!path) return;
    setLoading(true);
    setError(null);
    try {
      const next = await luxCommands.databaseListTables(path, dbOptions);
      setTables(next);
      if (!activeTable && next[0]) setActiveTable(next[0].name);
    } catch (reason) {
      setError(readError(reason));
    } finally {
      setLoading(false);
    }
  }, [activeTable, path]);

  const loadTableView = useCallback(async (tableName: string, tableKind = "table") => {
    if (!path) return;
    setLoading(true);
    setError(null);
    try {
      const quoted = quoteIdent(tableName);
      const result = await luxCommands.databaseExecuteSql(path, {
        sql: `SELECT rowid, * FROM ${quoted} LIMIT ${dbOptions.maxRows}`,
      });
      const rowids: number[] = [];
      const rows: string[][] = [];
      const columns = result.columns.filter((column) => column !== "rowid");
      for (const row of result.rows) {
        const rowid = Number(row[0]);
        if (!Number.isFinite(rowid)) continue;
        rowids.push(rowid);
        rows.push(row.slice(1));
      }
      setTableView({
        name: tableName,
        kind: tableKind,
        columns,
        rowids,
        rows,
      });
      setResultMessage(result.message);
    } catch (reason) {
      setError(readError(reason));
    } finally {
      setLoading(false);
    }
  }, [path]);

  useEffect(() => {
    void loadTables();
  }, [loadTables]);

  useEffect(() => {
    if (!activeTable) return;
    const kind = tables.find((table) => table.name === activeTable)?.kind ?? "table";
    void loadTableView(activeTable, kind);
  }, [activeTable, loadTableView, tables]);

  const columnCount = useMemo(
    () => Math.max(tableView?.columns.length ?? 0, ...(tableView?.rows.map((row) => row.length) ?? [0]), 1),
    [tableView],
  );

  const runSql = async () => {
    if (!path) return;
    setLoading(true);
    setError(null);
    try {
      const result = await luxCommands.databaseExecuteSql(path, { sql });
      setResultMessage(result.message);
      if (result.columns.length > 0) {
        setTableView({
          name: t("databaseEditor.queryResult"),
          kind: "query",
          columns: result.columns,
          rowids: [],
          rows: result.rows,
        });
      } else {
        await loadTables();
        if (activeTable) await loadTableView(activeTable);
      }
    } catch (reason) {
      setError(readError(reason));
    } finally {
      setLoading(false);
    }
  };

  const updateCell = async (rowIndex: number, columnIndex: number, value: string) => {
    if (!path || !tableView || !activeTable || tableView.kind === "query") return;
    const rowid = tableView.rowids[rowIndex];
    const column = tableView.columns[columnIndex];
    if (!column || rowid === undefined) return;
    const nextRows = tableView.rows.map((row, index) =>
      index === rowIndex ? row.map((cell, col) => (col === columnIndex ? value : cell)) : row,
    );
    setTableView({ ...tableView, rows: nextRows });
    try {
      await luxCommands.databaseUpdateCell(path, {
        table: activeTable,
        rowid,
        column,
        value,
      });
      setResultMessage(t("databaseEditor.cellSaved"));
    } catch (reason) {
      setError(readError(reason));
      await loadTableView(activeTable);
    }
  };

  if (!path) {
    return <div className="database-editor-empty">{t("databaseEditor.noPath")}</div>;
  }

  return (
    <div className="database-editor-pane">
      <div className="database-editor-toolbar">
        <div className="database-editor-title">
          <Database size={17} />
          <div>
            <strong>{documentDisplayPath(document)}</strong>
            <span>{t("databaseEditor.subtitle")}</span>
          </div>
        </div>
        <button className="icon-button compact" type="button" title={t("databaseEditor.refresh")} onClick={() => void loadTables()} disabled={loading}>
          <RefreshCw size={14} />
        </button>
      </div>
      {error && <div className="database-editor-error">{error}</div>}
      {resultMessage && <div className="database-editor-status">{resultMessage}</div>}
      <div className="database-editor-layout">
        <aside className="database-editor-sidebar">
          <h3>{t("databaseEditor.tables")}</h3>
          <ul>
            {tables.map((table) => (
              <li key={table.name}>
                <button
                  type="button"
                  className="database-editor-table-button"
                  data-active={table.name === activeTable}
                  onClick={() => setActiveTable(table.name)}
                >
                  <span>{table.name}</span>
                  <small>{table.kind}</small>
                </button>
              </li>
            ))}
          </ul>
        </aside>
        <section className="database-editor-main">
          <div className="database-editor-sql">
            <textarea value={sql} onChange={(event) => setSql(event.target.value)} spellCheck={false} />
            <button className="secondary-button compact" type="button" onClick={() => void runSql()} disabled={loading}>
              <Play size={14} />
              {t("databaseEditor.runSql")}
            </button>
          </div>
          {tableView && (
            <div className="database-editor-grid-wrap">
              <h3>{tableView.name}</h3>
              <table className="database-editor-grid">
                <thead>
                  <tr>
                    {Array.from({ length: columnCount }, (_, index) => (
                      <th key={index}>{tableView.columns[index] ?? t("databaseEditor.column", { index: index + 1 })}</th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {tableView.rows.map((row, rowIndex) => (
                    <tr key={rowIndex}>
                      {Array.from({ length: columnCount }, (_, columnIndex) => (
                        <td key={columnIndex}>
                          <input
                            value={row[columnIndex] ?? ""}
                            readOnly={tableView.kind === "query"}
                            onChange={(event) => void updateCell(rowIndex, columnIndex, event.target.value)}
                          />
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {loading && <div className="database-editor-loading">{t("databaseEditor.loading")}</div>}
        </section>
      </div>
    </div>
  );
}

function quoteIdent(value: string) {
  return `"${value.replaceAll("\"", "\"\"")}"`;
}

function readError(reason: unknown) {
  if (reason instanceof Error) return reason.message;
  return String(reason);
}