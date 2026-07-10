import { useTranslation } from '../../lib/i18n/useTranslation';
import { useFileAssetUrl } from '../../lib/hooks/use-file-asset-url';

type ResolvedAsset = {
  error: string | null;
  loading: boolean;
  url: string | null;
};

type MediaAssetViewProps = {
  alt: string;
  kind: "audio" | "image" | "video";
  path: string;
  reloadKey?: number;
  // Pre-resolved asset state. When supplied, MediaAssetView does NOT fetch the file
  // itself — this avoids the duplicate full-file base64 transfer + main-thread decode
  // that happens when both the owning pane and this view independently load the asset.
  // The owner fetches once via useFileAssetUrl and passes the result down.
  asset?: ResolvedAsset;
};

export function MediaAssetView({ alt, kind, path, reloadKey = 0, asset }: MediaAssetViewProps) {
  const { t } = useTranslation();
  // Fetch internally only when the owner did not provide an asset. Passing a null path
  // makes the hook a no-op, keeping hook order stable regardless of the `asset` prop.
  const own = useFileAssetUrl(asset ? null : path, reloadKey);
  const { error, loading, url } = asset ?? own;

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
