import { ExternalLink, Music, RefreshCw, Video } from "lucide-react";
import { useState } from "react";
import { MediaAssetView } from "./preview/MediaAssetView";
import { documentDisplayPath } from "../lib/editor/documents/documents";
import { useTranslation } from "../lib/i18n/useTranslation";
import { useFileAssetUrl } from "../lib/hooks/use-file-asset-url";
import { aspectCommands } from "../lib/tauri/commands";
import type { DocumentSnapshot } from "../lib/types/index";

type MediaEditorPaneProps = {
  document: DocumentSnapshot;
};

export function MediaEditorPane({ document }: MediaEditorPaneProps) {
  const { t } = useTranslation();
  const path = document.path;
  const isVideo = document.view.strategy === "videoPreview";
  const kind = isVideo ? "video" : "audio";
  const [reloadToken, setReloadToken] = useState(0);
  // Fetch the asset exactly once here and hand the resolved URL to MediaAssetView so it
  // does not re-fetch the same file (which previously doubled the full-file transfer).
  const { error, loading, mimeType, size, url } = useFileAssetUrl(path, reloadToken);

  if (!path) {
    return <div className="media-editor-empty">{t("mediaEditor.empty.noPath")}</div>;
  }

  const Icon = isVideo ? Video : Music;

  return (
    <div className="media-editor-pane" data-kind={kind}>
      <div className="media-editor-toolbar">
        <div className="media-editor-title">
          <Icon size={17} />
          <div>
            <strong>{documentDisplayPath(document)}</strong>
            <span>{[mimeType, size != null ? formatBytes(size, t) : null].filter(Boolean).join(" В· ")}</span>
          </div>
        </div>
        <div className="media-editor-actions">
          <button className="icon-button compact" type="button" title={t("mediaEditor.action.refresh")} onClick={() => setReloadToken((value) => value + 1)}>
            <RefreshCw size={14} />
          </button>
          <button className="icon-button compact" type="button" title={t("mediaEditor.action.openExternal")} onClick={() => void aspectCommands.fileOpenExternal(path).catch(() => undefined)}>
            <ExternalLink size={14} />
          </button>
        </div>
      </div>
      {error && <div className="media-editor-error">{error}</div>}
      <div className="media-editor-viewport">
        {loading && !error ? <div className="media-editor-loading">{t("mediaEditor.status.loading")}</div> : null}
        <MediaAssetView alt={documentDisplayPath(document)} kind={kind} path={path} reloadKey={reloadToken} asset={{ error, loading, url }} />
      </div>
    </div>
  );
}

function formatBytes(bytes: number, t: ReturnType<typeof useTranslation>["t"]) {
  if (bytes < 1024) return t("common.fileSize.bytes", { bytes });
  if (bytes < 1024 * 1024) return t("common.fileSize.kilobytes", { kilobytes: (bytes / 1024).toFixed(1) });
  return t("common.fileSize.megabytes", { megabytes: (bytes / (1024 * 1024)).toFixed(1) });
}