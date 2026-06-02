import { Copy, Database, ExternalLink, FileArchive, FileText, ImageIcon, Music, RefreshCw, Table2, Video } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { documentDisplayPath } from "../lib/documents";
import { luxCommands, type FileAssetResponse } from "../lib/tauri";
import type { DocumentSnapshot, FileInspection, FilePreview } from "../lib/types";

type FilePreviewPaneProps = {
  document: DocumentSnapshot;
};

const previewOptions = {
  maxTextBytes: 1_000_000n,
  maxRows: 120,
  maxColumns: 32,
  maxArchiveEntries: 500,
};

export function FilePreviewPane({ document }: FilePreviewPaneProps) {
  const [inspection, setInspection] = useState<FileInspection | null>(null);
  const [asset, setAsset] = useState<FileAssetResponse | null>(null);
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
    setAsset(null);
    void Promise.allSettled([
      luxCommands.fileInspect(path, previewOptions),
      needsAsset(document) ? luxCommands.fileAssetData(path) : Promise.resolve(null),
    ]).then(([inspectionResult, assetResult]) => {
      if (cancelled) return;
      if (inspectionResult.status === "fulfilled") setInspection(inspectionResult.value);
      else setError(readError(inspectionResult.reason));
      if (assetResult.status === "fulfilled") setAsset(assetResult.value);
      else setError(readError(assetResult.reason));
    }).finally(() => {
      if (!cancelled) setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [document, path, reloadToken]);

  const title = inspection?.title ?? document.title;
  const descriptor = inspection?.descriptor ?? document.view;
  const metaItems = useMemo(() => [
    descriptor.displayName,
    inspection ? formatBytes(inspection.metadata.size) : null,
    inspection?.truncated ? "truncated" : null,
    descriptor.aiReadable ? "AI readable" : "metadata only",
  ].filter(Boolean), [descriptor, inspection]);

  if (!path) return <div className="file-preview-empty">No file path.</div>;

  return (
    <div className="file-preview-pane">
      <div className="file-preview-toolbar">
        <div className="file-preview-title">
          <PreviewIcon kind={inspection?.preview.kind ?? strategyToKind(descriptor.strategy)} />
          <div>
            <strong>{title}</strong>
            <span>{metaItems.join(" Â· ")}</span>
          </div>
        </div>
        <div className="file-preview-actions">
          <button className="icon-button compact" type="button" title="Copy AI context" disabled={!inspection} onClick={() => inspection && void luxCommands.clipboardWriteText(inspection.aiContext).catch(() => undefined)}>
            <Copy size={14} />
          </button>
          <button className="icon-button compact" type="button" title="Refresh preview" onClick={() => setReloadToken((value) => value + 1)} disabled={loading}>
            <RefreshCw size={14} />
          </button>
          <button className="icon-button compact" type="button" title="Open in system app" onClick={() => void luxCommands.fileOpenExternal(path).catch(() => undefined)}>
            <ExternalLink size={14} />
          </button>
        </div>
      </div>
      {error && <div className="file-preview-error">{error}</div>}
      {loading && !inspection ? <div className="file-preview-loading">Loading preview...</div> : null}
      {inspection ? <PreviewBody inspection={inspection} asset={asset} fallbackPath={documentDisplayPath(document)} /> : null}
    </div>
  );
}

function PreviewBody({ asset, fallbackPath, inspection }: { asset: FileAssetResponse | null; fallbackPath: string; inspection: FileInspection }) {
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
      return <PdfPreview asset={asset} preview={inspection.preview} />;
    case "office":
      return <OfficePreview preview={inspection.preview} />;
    case "image":
      return asset ? <div className="file-preview-media image"><img src={asset.dataUrl} alt={fallbackPath} /></div> : <PreviewNote note={inspection.preview.note} />;
    case "audio":
      return asset ? <div className="file-preview-media"><audio controls src={asset.dataUrl} /></div> : <PreviewNote note={inspection.preview.note} />;
    case "video":
      return asset ? <div className="file-preview-media video"><video controls src={asset.dataUrl} /></div> : <PreviewNote note={inspection.preview.note} />;
    case "archive":
      return <ArchivePreview preview={inspection.preview} />;
    case "notebook":
      return <NotebookPreview preview={inspection.preview} />;
    case "binary":
      return <BinaryPreview preview={inspection.preview} />;
    case "external":
      return <PreviewNote note={inspection.preview.reason} />;
    default:
      return <PreviewNote note="No preview available." />;
  }
}

function TablePreview({ headers, rowCount, rows, truncated }: { headers: string[]; rows: string[][]; rowCount: number; truncated: boolean }) {
  const columnCount = Math.max(headers.length, ...rows.map((row) => row.length), 1);
  return (
    <div className="file-preview-table-wrap">
      <PreviewSummary text={`${rowCount} row${rowCount === 1 ? "" : "s"}${truncated ? " Â· truncated" : ""}`} />
      <table className="file-preview-table">
        {headers.length > 0 && <thead><tr>{Array.from({ length: columnCount }, (_, index) => <th key={index}>{headers[index] ?? `Column ${index + 1}`}</th>)}</tr></thead>}
        <tbody>{rows.map((row, rowIndex) => <tr key={rowIndex}>{Array.from({ length: columnCount }, (_, index) => <td key={index}>{row[index] ?? ""}</td>)}</tr>)}</tbody>
      </table>
    </div>
  );
}

function SpreadsheetPreview({ preview }: { preview: Extract<FilePreview, { kind: "spreadsheet" }> }) {
  return <div className="file-preview-sections">{preview.sheets.map((sheet) => <section className="file-preview-section" key={sheet.name}><h3>{sheet.name}</h3><TablePreview headers={sheet.headers} rows={sheet.rows} rowCount={sheet.rowCount} truncated={sheet.truncated} /></section>)}</div>;
}

function DatabasePreview({ preview }: { preview: Extract<FilePreview, { kind: "database" }> }) {
  return <div className="file-preview-sections">{preview.tables.map((table) => <section className="file-preview-section" key={table.name}><h3>{table.kind}: {table.name}</h3><div className="file-preview-columns">{table.columns.map((column) => <span key={column.name}>{column.name} <small>{column.typeName || "any"}</small></span>)}</div><TablePreview headers={table.columns.map((column) => column.name)} rows={table.rows} rowCount={table.rowCount ?? table.rows.length} truncated={table.truncated} /></section>)}</div>;
}

function PdfPreview({ asset, preview }: { asset: FileAssetResponse | null; preview: Extract<FilePreview, { kind: "pdf" }> }) {
  return <div className="file-preview-pdf">{asset && <iframe title="PDF preview" src={asset.dataUrl} />}<pre>{preview.text || "PDF rendered above. Extracted text is empty."}</pre></div>;
}

function OfficePreview({ preview }: { preview: Extract<FilePreview, { kind: "office" }> }) {
  return <div className="file-preview-office"><pre>{preview.text || "No extractable Office text found."}</pre><ArchiveEntryList entries={preview.parts} /></div>;
}

function ArchivePreview({ preview }: { preview: Extract<FilePreview, { kind: "archive" }> }) {
  return <div className="file-preview-archive"><PreviewSummary text={`${preview.total_entries} entr${preview.total_entries === 1 ? "y" : "ies"}${preview.truncated ? " Â· truncated" : ""}`} /><ArchiveEntryList entries={preview.entries} /></div>;
}

function ArchiveEntryList({ entries }: { entries: Array<{ path: string; compressedSize: number; uncompressedSize: number; isDir: boolean }> }) {
  return <div className="file-preview-entry-list">{entries.map((entry) => <div className="file-preview-entry" key={entry.path}><span>{entry.path}</span><small>{entry.isDir ? "folder" : formatBytes(entry.uncompressedSize)}</small></div>)}</div>;
}

function NotebookPreview({ preview }: { preview: Extract<FilePreview, { kind: "notebook" }> }) {
  return <div className="file-preview-sections"><PreviewSummary text={`${preview.cell_count} cell${preview.cell_count === 1 ? "" : "s"}`} />{preview.cells.map((cell) => <section className="file-preview-section" key={cell.index}><h3>Cell {cell.index + 1} Â· {cell.cellType}</h3><pre>{cell.text}</pre>{cell.outputText && <pre>{cell.outputText}</pre>}</section>)}</div>;
}

function BinaryPreview({ preview }: { preview: Extract<FilePreview, { kind: "binary" }> }) {
  return <div className="file-preview-binary"><pre>{preview.hex}</pre><pre>{preview.ascii}</pre></div>;
}

function PreviewNote({ note }: { note: string }) {
  return <div className="file-preview-note"><FileText size={18} /><span>{note}</span></div>;
}

function PreviewSummary({ text }: { text: string }) {
  return <div className="file-preview-summary">{text}</div>;
}

function PreviewIcon({ kind }: { kind: string }) {
  const Icon = kind === "database" ? Database : kind === "table" || kind === "spreadsheet" ? Table2 : kind === "archive" ? FileArchive : kind === "image" ? ImageIcon : kind === "audio" ? Music : kind === "video" ? Video : FileText;
  return <Icon size={17} />;
}

function needsAsset(document: DocumentSnapshot) {
  return ["pdfPreview", "imagePreview", "audioPreview", "videoPreview"].includes(document.view.strategy);
}

function strategyToKind(strategy: string) {
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
