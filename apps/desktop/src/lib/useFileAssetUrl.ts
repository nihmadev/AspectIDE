import { useEffect, useState } from "react";
import { luxCommands } from "./tauri";

type FileAssetState = {
  error: string | null;
  loading: boolean;
  mimeType: string | null;
  size: number | null;
  url: string | null;
};

function dataUrlToBlobUrl(dataUrl: string, mimeType: string) {
  const commaIndex = dataUrl.indexOf(",");
  if (commaIndex < 0) throw new Error("Invalid asset data URL");
  const base64 = dataUrl.slice(commaIndex + 1);
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  const blob = new Blob([bytes], { type: mimeType });
  return URL.createObjectURL(blob);
}

export function useFileAssetUrl(path: string | null, reloadKey = 0) {
  const [state, setState] = useState<FileAssetState>({
    error: null,
    loading: Boolean(path),
    mimeType: null,
    size: null,
    url: null,
  });

  useEffect(() => {
    if (!path) {
      setState({ error: null, loading: false, mimeType: null, size: null, url: null });
      return;
    }

    let objectUrl: string | null = null;
    let cancelled = false;
    setState({ error: null, loading: true, mimeType: null, size: null, url: null });

    void luxCommands.fileAssetData(path)
      .then((asset) => {
        if (cancelled) return;
        objectUrl = dataUrlToBlobUrl(asset.dataUrl, asset.mimeType);
        setState({
          error: null,
          loading: false,
          mimeType: asset.mimeType,
          size: Number(asset.size),
          url: objectUrl,
        });
      })
      .catch((error) => {
        if (cancelled) return;
        setState({
          error: error instanceof Error ? error.message : String(error),
          loading: false,
          mimeType: null,
          size: null,
          url: null,
        });
      });

    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [path, reloadKey]);

  return state;
}