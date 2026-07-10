import { ExternalLink, ImageIcon, Minus, Plus, RefreshCw, RotateCcw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { documentDisplayPath } from '../../lib/editor/documents/documents';
import { useTranslation } from '../../lib/i18n/useTranslation';
import { useFileAssetUrl } from '../../lib/hooks/use-file-asset-url';
import { luxCommands } from '../../lib/tauri/commands';
import type { DocumentSnapshot } from '../../lib/types/index';

type ImageEditorPaneProps = {
  document: DocumentSnapshot;
};

const zoomSteps = [25, 50, 75, 100, 125, 150, 200, 300, 400];

export function ImageEditorPane({ document }: ImageEditorPaneProps) {
  const { t } = useTranslation();
  const path = document.path;
  const [zoom, setZoom] = useState(100);
  const [fitMode, setFitMode] = useState<"fit" | "actual">("fit");
  const [dimensions, setDimensions] = useState<{ height: number; width: number } | null>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const { error, loading, mimeType, size, url } = useFileAssetUrl(path, reloadToken);
  const viewportRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    setZoom(100);
    setFitMode("fit");
    setDimensions(null);
  }, [document.id, reloadToken]);

  const stepZoom = useCallback((direction: -1 | 1) => {
    setFitMode("actual");
    setZoom((current) => {
      const index = zoomSteps.findIndex((value) => value >= current);
      const baseIndex = index === -1 ? zoomSteps.length - 1 : index;
      const nextIndex = Math.min(zoomSteps.length - 1, Math.max(0, baseIndex + direction));
      return zoomSteps[nextIndex] ?? current;
    });
  }, []);

  if (!path) {
    return <div className="image-editor-empty">{t("imageEditor.empty.noPath")}</div>;
  }

  const isVector = mimeType === "image/svg+xml" || path.toLowerCase().endsWith(".svg");

  return (
    <div className="image-editor-pane">
      <div className="image-editor-toolbar">
        <div className="image-editor-title">
          <ImageIcon size={17} />
          <div>
            <strong>{documentDisplayPath(document)}</strong>
            <span>
              {[
                mimeType,
                size != null ? formatBytes(size, t) : null,
                dimensions ? t("imageEditor.dimensions", { width: dimensions.width, height: dimensions.height }) : null,
              ].filter(Boolean).join(" В· ")}
            </span>
          </div>
        </div>
        <div className="image-editor-actions">
          <button className="icon-button compact" type="button" title={t("imageEditor.action.zoomOut")} disabled={zoom <= zoomSteps[0]} onClick={() => stepZoom(-1)}>
            <Minus size={14} />
          </button>
          <button className="secondary-button compact" type="button" title={t("imageEditor.action.resetZoom")} onClick={() => { setFitMode("fit"); setZoom(100); }}>
            {fitMode === "fit" ? t("imageEditor.zoom.fit") : t("imageEditor.zoom.percent", { percent: zoom })}
          </button>
          <button className="icon-button compact" type="button" title={t("imageEditor.action.zoomIn")} disabled={zoom >= zoomSteps[zoomSteps.length - 1]} onClick={() => stepZoom(1)}>
            <Plus size={14} />
          </button>
          <button className="icon-button compact" type="button" title={t("imageEditor.action.actualSize")} onClick={() => { setFitMode("actual"); setZoom(100); }}>
            <RotateCcw size={14} />
          </button>
          <button className="icon-button compact" type="button" title={t("imageEditor.action.refresh")} onClick={() => setReloadToken((value) => value + 1)}>
            <RefreshCw size={14} />
          </button>
          <button className="icon-button compact" type="button" title={t("imageEditor.action.openExternal")} onClick={() => void luxCommands.fileOpenExternal(path).catch(() => undefined)}>
            <ExternalLink size={14} />
          </button>
        </div>
      </div>
      {error && <div className="image-editor-error">{error}</div>}
      <div className="image-editor-viewport" ref={viewportRef} data-fit={fitMode} key={`${path}:${reloadToken}`}>
        {loading && !url ? <div className="image-editor-loading">{t("imageEditor.status.loading")}</div> : null}
        {url ? (
          isVector ? (
            <object className="image-editor-vector" data={url} type="image/svg+xml" aria-label={documentDisplayPath(document)} />
          ) : (
            <img
              className="image-editor-image"
              src={url}
              alt={documentDisplayPath(document)}
              style={fitMode === "actual" ? { width: `${zoom}%`, maxWidth: "none" } : undefined}
              onLoad={(event) => {
                const image = event.currentTarget;
                setDimensions({ width: image.naturalWidth, height: image.naturalHeight });
              }}
            />
          )
        ) : null}
      </div>
      {isVector && (
        <div className="image-editor-hint">{t("imageEditor.hint.svg")}</div>
      )}
    </div>
  );
}

function formatBytes(bytes: number, t: ReturnType<typeof useTranslation>["t"]) {
  if (bytes < 1024) return t("common.fileSize.bytes", { bytes });
  if (bytes < 1024 * 1024) return t("common.fileSize.kilobytes", { kilobytes: (bytes / 1024).toFixed(1) });
  return t("common.fileSize.megabytes", { megabytes: (bytes / (1024 * 1024)).toFixed(1) });
}