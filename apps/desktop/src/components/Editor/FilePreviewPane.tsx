import { Copy, Database, ExternalLink, FileArchive, FileText, ImageIcon, Music, RefreshCw, Table2, Video } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { documentDisplayPath } from '../../lib/editor/documents/documents';
import { useTranslation } from '../../lib/i18n/useTranslation';
import { MediaAssetView } from "../Preview/MediaAssetView";
import { useFileAssetUrl } from '../../lib/hooks/use-file-asset-url';
import { luxCommands } from '../../lib/tauri/commands';
import type { DocumentSnapshot, FileInspection, FilePreview } from '../../lib/types/index';

type FilePreviewPaneProps = {
  document: DocumentSnapshot;
  variant?: "editor" | "inline";
};

const previewOptions = {
  maxTextBytes: 1_000_000,
  maxRows: 120,
  maxColumns: 32,
  maxArchiveEntries: 500,
};

export function FilePreviewPane({ document, variant = "editor" }: FilePreviewPaneProps) {
  const { t } = useTranslation();
  const [inspection, setInspection] = useState<FileInspection | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [reloadToken, setReloadToken] = useState(0);
  const path = document.path;

  useEffect(() => {
    if (!path) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    setInspection(null);
    void luxCommands.fileInspect(path, previewOptions)
      .then((result) => {
        if (!cancelled) setInspection(result);
      })
      .catch((reason) => {
        if (!cancelled) setError(readError(reason));
      })
      .finally(() => {
      if (!cancelled) setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [path, reloadToken]);

  const title = inspection?.title ?? document.title;
  const descriptor = inspection?.descriptor ?? document.view;
  const metaItems = useMemo(() => [
    descriptor.displayName,
    inspection ? formatBytes(inspection.metadata.size) : null,
    inspection?.truncated ? t("filePreview.meta.truncated") : null,
    descriptor.aiReadable ? t("filePreview.meta.aiReadable") : t("filePreview.meta.metadataOnly"),
  ].filter(Boolean), [descriptor, inspection, t]);

  if (!path) return <div className="file-preview-empty">{t("filePreview.empty.noPath")}</div>;

  return (
    <div className="file-preview-pane" data-variant={variant}>
      <div className="file-preview-toolbar">
        <div className="file-preview-title">
          <PreviewIcon kind={inspection?.preview.kind ?? strategyToKind(descriptor.strategy)} />
          <div>
            <strong>{title}</strong>
            <span>{metaItems.join(` ${t("filePreview.meta.separator")} `)}</span>
          </div>
        </div>
        <div className="file-preview-actions">
          <button className="icon-button compact" type="button" title={t("filePreview.action.copyAiContext")} disabled={!inspection} onClick={() => inspection && void luxCommands.clipboardWriteText(inspection.aiContext).catch(() => undefined)}>
            <Copy size={14} />
          </button>
          <button className="icon-button compact" type="button" title={t("filePreview.action.refresh")} onClick={() => setReloadToken((value) => value + 1)} disabled={loading}>
            <RefreshCw size={14} />
          </button>
          <button className="icon-button compact" type="button" title={t("filePreview.action.openExternal")} onClick={() => void luxCommands.fileOpenExternal(path).catch(() => undefined)}>
            <ExternalLink size={14} />
          </button>
        </div>
      </div>
      {error && <div className="file-preview-error">{error}</div>}
      {loading && !inspection ? <div className="file-preview-loading">{t("filePreview.status.loading")}</div> : null}
      {inspection ? <PreviewBody inspection={inspection} fallbackPath={documentDisplayPath(document)} reloadKey={reloadToken} /> : null}
    </div>
  );
}

function PreviewBody({ fallbackPath, inspection, reloadKey }: { fallbackPath: string; inspection: FileInspection; reloadKey: number }) {
  const { t } = useTranslation();

  switch (inspection.preview.kind) {
    case "text":
      return <pre className="file-preview-text">{inspection.preview.text}</pre>;
    case "table":
      return <TablePreview headers={inspection.preview.headers} rows={inspection.preview.rows} rowCount={inspection.preview.row_count} truncated={inspection.preview.truncated} />;
    case "spreadsheet":
      return <SpreadsheetPreview preview={inspection.preview} />;
    case "database":
      return <DatabasePreview preview={inspection.preview} />;
    case "pdf":
      return <PdfPreview path={inspection.path} preview={inspection.preview} reloadKey={reloadKey} />;
    case "office":
      return <OfficePreview preview={inspection.preview} />;
    case "image":
      return <MediaAssetView alt={fallbackPath} kind="image" path={inspection.path} reloadKey={reloadKey} />;
    case "audio":
      return <MediaAssetView alt={fallbackPath} kind="audio" path={inspection.path} reloadKey={reloadKey} />;
    case "video":
      return <MediaAssetView alt={fallbackPath} kind="video" path={inspection.path} reloadKey={reloadKey} />;
    case "archive":
      return <ArchivePreview preview={inspection.preview} />;
    case "notebook":
      return <NotebookPreview preview={inspection.preview} />;
    case "binary":
      return <BinaryPreview preview={inspection.preview} />;
    case "external":
      return <PreviewNote note={inspection.preview.reason} openPath={inspection.path} />;
    default:
      return <PreviewNote note={t("filePreview.fallback.noPreview")} />;
  }
}

function TablePreview({ headers, rowCount, rows, truncated }: { headers: string[]; rows: string[][]; rowCount: number; truncated: boolean }) {
  const { t } = useTranslation();
  const columnCount = Math.max(headers.length, ...rows.map((row) => row.length), 1);
  const summary = `${t("filePreview.table.rowCount", { count: rowCount })}${truncated ? ` вЂ” ${t("filePreview.table.truncatedSuffix")}` : ""}`;
  return (
    <div className="file-preview-table-wrap">
      <PreviewSummary text={summary} />
      <table className="file-preview-table">
        {headers.length > 0 && (
          <thead>
            <tr>
              {Array.from({ length: columnCount }, (_, index) => (
                <th key={index}>{headers[index] ?? t("filePreview.table.columnFallback", { index: index + 1 })}</th>
              ))}
            </tr>
          </thead>
        )}
        <tbody>
          {rows.map((row, rowIndex) => (
            <tr key={rowIndex}>
              {Array.from({ length: columnCount }, (_, index) => (
                <td key={index}>{row[index] ?? ""}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function SpreadsheetPreview({ preview }: { preview: Extract<FilePreview, { kind: "spreadsheet" }> }) {
  return (
    <div className="file-preview-sections">
      {preview.sheets.map((sheet) => (
        <section className="file-preview-section" key={sheet.name}>
          <h3>{sheet.name}</h3>
          <TablePreview headers={sheet.headers} rows={sheet.rows} rowCount={sheet.rowCount} truncated={sheet.truncated} />
        </section>
      ))}
    </div>
  );
}

function DatabasePreview({ preview }: { preview: Extract<FilePreview, { kind: "database" }> }) {
  return (
    <div className="file-preview-sections">
      {preview.tables.map((table) => (
        <section className="file-preview-section" key={table.name}>
          <h3>{table.kind}: {table.name}</h3>
          <div className="file-preview-columns">
            {table.columns.map((column) => (
              <span key={column.name}>
                {column.name} <small>{column.typeName || "any"}</small>
              </span>
            ))}
          </div>
          <TablePreview headers={table.columns.map((column) => column.name)} rows={table.rows} rowCount={table.rowCount ?? table.rows.length} truncated={table.truncated} />
        </section>
      ))}
    </div>
  );
}

function PdfPreview({ path, preview, reloadKey = 0 }: { path: string; preview: Extract<FilePreview, { kind: "pdf" }>; reloadKey?: number }) {
  const { t } = useTranslation();
  const { error, loading, url } = useFileAssetUrl(path, reloadKey);
  return (
    <div className="file-preview-pdf">
      {url ? <iframe title={t("filePreview.pdf.title")} src={url} /> : null}
      {loading && !url ? <div className="file-preview-loading">{t("filePreview.status.loading")}</div> : null}
      {error && <div className="file-preview-error">{error}</div>}
      {preview.text ? <pre>{preview.text}</pre> : !loading && !error ? <pre>{t("filePreview.pdf.emptyText")}</pre> : null}
    </div>
  );
}

function OfficePreview({ preview }: { preview: Extract<FilePreview, { kind: "office" }> }) {
  const { t } = useTranslation();
  return (
    <div className="file-preview-office">
      <pre>{preview.text || t("filePreview.office.emptyText")}</pre>
      <ArchiveEntryList entries={preview.parts} />
    </div>
  );
}

function ArchivePreview({ preview }: { preview: Extract<FilePreview, { kind: "archive" }> }) {
  const { t } = useTranslation();
  const summary = `${t("filePreview.archive.entryCount", { count: preview.total_entries })}${preview.truncated ? ` вЂ” ${t("filePreview.table.truncatedSuffix")}` : ""}`;
  return (
    <div className="file-preview-archive">
      <PreviewSummary text={summary} />
      <ArchiveEntryList entries={preview.entries} />
    </div>
  );
}

function ArchiveEntryList({ entries }: { entries: Array<{ path: string; compressedSize: number; uncompressedSize: number; isDir: boolean }> }) {
  const { t } = useTranslation();
  return (
    <div className="file-preview-entry-list">
      {entries.map((entry, index) => (
        <div className="file-preview-entry" key={`${index}:${entry.path}`}>
          <span>{entry.path}</span>
          <small>{entry.isDir ? t("filePreview.archive.folder") : formatBytes(entry.uncompressedSize)}</small>
        </div>
      ))}
    </div>
  );
}

function NotebookPreview({ preview }: { preview: Extract<FilePreview, { kind: "notebook" }> }) {
  const { t } = useTranslation();
  return (
    <div className="file-preview-sections">
      <PreviewSummary text={t("filePreview.notebook.cellCount", { count: preview.cell_count })} />
      {preview.cells.map((cell) => (
        <section className="file-preview-section" key={cell.index}>
          <h3>{t("filePreview.notebook.cellTitle", { index: cell.index + 1, cellType: cell.cellType })}</h3>
          <pre>{cell.text}</pre>
          {cell.outputText && <pre>{cell.outputText}</pre>}
        </section>
      ))}
    </div>
  );
}

function BinaryPreview({ preview }: { preview: Extract<FilePreview, { kind: "binary" }> }) {
  return (
    <div className="file-preview-binary">
      <pre>{preview.hex}</pre>
      <pre>{preview.ascii}</pre>
    </div>
  );
}

function PreviewNote({ note, openPath }: { note: string; openPath?: string }) {
  const { t } = useTranslation();
  return (
    <div className="file-preview-note">
      <FileText size={18} />
      <span>{note}</span>
      {openPath && (
        <button className="secondary-button compact" type="button" onClick={() => void luxCommands.fileOpenExternal(openPath).catch(() => undefined)}>
          {t("filePreview.action.openExternal")}
        </button>
      )}
    </div>
  );
}

function PreviewSummary({ text }: { text: string }) {
  return <div className="file-preview-summary">{text}</div>;
}

function PreviewIcon({ kind }: { kind: string }) {
  const Icon = kind === "database" ? Database : kind === "table" || kind === "spreadsheet" ? Table2 : kind === "archive" ? FileArchive : kind === "image" ? ImageIcon : kind === "audio" ? Music : kind === "video" ? Video : FileText;
  return <Icon size={17} />;
}

function strategyToKind(strategy: string) {
  if (strategy === "spreadsheetEditor") return "spreadsheet";
  return strategy.replace(/Preview$/, "").replace(/^monacoText$/, "text");
}

function readError(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function formatBytes(bytes: bigint | number) {
  const value = typeof bytes === "bigint" ? Number(bytes) : bytes;
  if (!Number.isFinite(value)) return String(bytes);
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}