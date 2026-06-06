import { useTranslation } from "../../lib/i18n/useTranslation";
import { useFileAssetUrl } from "../../lib/useFileAssetUrl";

type MediaAssetViewProps = {
  alt: string;
  kind: "audio" | "image" | "video";
  path: string;
  reloadKey?: number;
};

export function MediaAssetView({ alt, kind, path, reloadKey = 0 }: MediaAssetViewProps) {
  const { t } = useTranslation();
  const { error, loading, url } = useFileAssetUrl(path, reloadKey);

  if (loading && !url) {
    return <div className="file-preview-loading">{t("filePreview.status.loading")}</div>;
  }
  if (error) {
    return <div className="file-preview-error">{error}</div>;
  }
  if (!url) {
    return <div className="file-preview-note"><span>{t("filePreview.fallback.noPreview")}</span></div>;
  }

  if (kind === "image") {
    return (
      <div className="file-preview-media image">
        <img src={url} alt={alt} />
      </div>
    );
  }
  if (kind === "audio") {
    return (
      <div className="file-preview-media">
        <audio controls src={url} />
      </div>
    );
  }
  return (
    <div className="file-preview-media video">
      <video controls src={url} />
    </div>
  );
}