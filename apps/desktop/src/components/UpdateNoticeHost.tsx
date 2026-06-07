import { useEffect } from "react";
import { useLuxStore } from "../lib/store";
import { useUpdater } from "../lib/useUpdater";
import { UpdateNotice } from "./UpdateNotice";

/**
 * Owns the auto-update lifecycle for the whole app: runs the periodic check,
 * renders the bottom-right {@link UpdateNotice}, and mirrors "update available"
 * into the store so the title-bar badge stays in sync. Mounted once at the app
 * root so a single updater drives every surface.
 */
export function UpdateNoticeHost() {
  const { state, install, dismiss, check } = useUpdater();
  const setUpdateAvailable = useLuxStore((store) => store.setUpdateAvailable);

  useEffect(() => {
    setUpdateAvailable(state.status === "available");
  }, [state.status, setUpdateAvailable]);

  return (
    <UpdateNotice
      state={state}
      onInstall={install}
      onDismiss={dismiss}
      onRetry={() => void check()}
    />
  );
}
