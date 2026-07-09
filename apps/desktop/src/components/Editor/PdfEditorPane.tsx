import { ChevronDown, ChevronUp, ExternalLink, FileText, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { documentDisplayPath } from "../lib/editor/documents/documents";
import { useTranslation } from "../lib/i18n/useTranslation";
import { useFileAssetUrl } from "../lib/hooks/use-file-asset-url";
import { aspectCommands } from "../lib/tauri/commands";
import type { DocumentSnapshot, FileInspection } from "../lib/types/index";

type PdfEditorPaneProps = {
  document: DocumentSnapshot;
};

const inspectOptions = {
  maxTextBytes: 48_000,
  maxRows: 40,
  maxColumns: 24,
  maxArchiveEntries: 80,
};

export function PdfEditorPane({ document }: PdfEditorPaneProps) {
  const { t } = useTranslation();
  const path = document.path;
  const [reloadToken, setReloadToken] = useState(0);
  const [inspection, setInspection] = useState<FileInspection | null>(null);
  const [inspectError, setInspectError] = useState<string | null>(null);
  const [textOpen, setTextOpen] = useState(false);
  const { error, loading, mimeType, size, url } = useFileAssetUrl(path, reloadToken);

  useEffect(() => {
    if (!path) return;
    let cancelled = false;
    setInspectError(null);
    void aspectCommands.fileInspect(path, inspectOptions)
      .then((result) => {
        if (!cancelled) setInspection(result);
      })
      .catch((reason) => {
        if (!cancelled) setInspectError(reason instanceof Error ? reason.message : String(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [path, reloadToken]);

  if (!path) {
    return <div className="pdf-editor-empty">{t("pdfEditor.empty.noPath")}</div>;
  }

  const pdfPreview = inspection?.preview.kind === "pdf" ? inspection.preview : null;
  const extractedText = pdfPreview?.text?.trim() ?? "";

  return (
    <div className="pdf-editor-pane">
      <div className="pdf-editor-toolbar">
        <div className="pdf-editor-title">
          <FileText size={17} />
          <div>
            <strong>{documentDisplayPath(document)}</strong>
            <span>
              {[
                mimeType ?? "application/pdf",
                size != null ? formatBytes(size, t) : null,
                pdfPreview?.page_count != null ? t("pdfEditor.pageCount", { count: pdfPreview.page_count }) : null,
              ].filter(Boolean).join(" В· ")}
            </span>
          </div>
        </div>
        <div className="pdf-editor-actions">
          {extractedText && (
            <button
              className="secondary-button compact"
              type="button"
              onClick={() => setTextOpen((open) => !open)}
            >
              {textOpen ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
              {t("pdfEditor.action.toggleText")}
            </button>
          )}
          <button className="icon-button compact" type="button" title={t("pdfEditor.action.refresh")} onClick={() => setReloadToken((value) => value + 1)}>
            <RefreshCw size={14} />
          </button>
          <button className="icon-button compact" type="button" title={t("pdfEditor.action.openExternal")} onClick={() => void aspectCommands.fileOpenExternal(path).catch(() => undefined)}>
            <ExternalLink size={14} />
          </button>
        </div>
      </div>
      {(error || inspectError) && (
        <div className="pdf-editor-error">{error ?? inspectError}</div>
      )}
      <div className="pdf-editor-body" data-text-open={textOpen}>
        <div className="pdf-editor-viewport">
          {loading && !url ? <div className="pdf-editor-loading">{t("pdfEditor.status.loading")}</div> : null}
          {url ? (
            <iframe className="pdf-editor-frame" title={t("pdfEditor.frameTitle")} src={url} />
          ) : !loading && !error ? (
            <div className="pdf-editor-loading">{t("pdfEditor.status.unavailable")}</div>
          ) : null}
        </div>
        {textOpen && extractedText && (
          <pre className="pdf-editor-text">{extractedText}</pre>
        )}
      </div>
    </div>
  );
}

function formatBytes(bytes: number, t: ReturnType<typeof useTranslation>["t"]) {
  if (bytes < 1024) return t("common.fileSize.bytes", { bytes });
  if (bytes < 1024 * 1024) return t("common.fileSize.kilobytes", { kilobytes: (bytes / 1024).toFixed(1) });
  return t("common.fileSize.megabytes", { megabytes: (bytes / (1024 * 1024)).toFixed(1) });
}