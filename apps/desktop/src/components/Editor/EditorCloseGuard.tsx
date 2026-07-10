import * as Dialog from "@radix-ui/react-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { AlertTriangle, FileCode2 } from "lucide-react";
import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { documentDisplayPath, documentParentLabel, documentTitle } from '../../lib/editor/documents/documents';
import { useTranslation, type TranslateFn } from '../../lib/i18n/useTranslation';
import { useLuxStore } from '../../lib/store/index';
import { isTauriRuntime, luxCommands } from '../../lib/tauri/commands';
import type { DocumentSnapshot } from '../../lib/types/index';

type CloseRequestOptions = {
  title?: string;
  message?: string;
};

type PendingCloseRequest = {
  action: () => void;
  documentIds: string[];
  documents: DocumentSnapshot[];
  message: string;
  title: string;
};

type EditorCloseGuardApi = {
  requestCloseDocuments: (documentIds: Iterable<string>, action: () => void, options?: CloseRequestOptions) => boolean;
};

const EditorCloseGuardContext = createContext<EditorCloseGuardApi | null>(null);

export function EditorCloseGuardProvider({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const replaceDocumentSnapshot = useLuxStore((state) => state.replaceDocumentSnapshot);
  const [pendingRequest, setPendingRequest] = useState<PendingCloseRequest | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pendingRequestRef = useRef<PendingCloseRequest | null>(null);
  useEffect(() => {
    pendingRequestRef.current = pendingRequest;
  }, [pendingRequest]);

  const requestCloseDocuments = useCallback<EditorCloseGuardApi["requestCloseDocuments"]>((documentIds, action, options) => {
    const uniqueDocumentIds = Array.from(new Set(Array.from(documentIds)));
    const documentsById = new Map(useLuxStore.getState().openDocuments.map((document) => [document.id, document]));
    const dirtyDocuments = uniqueDocumentIds
      .map((documentId) => documentsById.get(documentId))
      .filter((document): document is DocumentSnapshot => Boolean(document?.is_dirty));

    if (dirtyDocuments.length === 0) {
      action();
      return true;
    }

    if (pendingRequestRef.current) return false;

    const newRequest: PendingCloseRequest = {
      action,
      documentIds: uniqueDocumentIds,
      documents: dirtyDocuments,
      title: options?.title ?? t("closeGuard.title"),
      message: options?.message ?? closeMessage(dirtyDocuments.length, t),
    };
    pendingRequestRef.current = newRequest;
    setError(null);
    setPendingRequest(newRequest);
    return false;
  }, [t]);

  const cancel = useCallback(() => {
    if (saving) return;
    setError(null);
    setPendingRequest(null);
  }, [saving]);

  const discardAndClose = useCallback(() => {
    const request = pendingRequest;
    if (!request || saving) return;
    request.action();
    setError(null);
    setPendingRequest(null);
  }, [pendingRequest, saving]);

  const saveAndClose = useCallback(async () => {
    const request = pendingRequest;
    if (!request || saving) return;

    setSaving(true);
    setError(null);
    try {
      for (const document of request.documents) {
        const current = useLuxStore.getState().openDocuments.find((candidate) => candidate.id === document.id);
        if (!current?.is_dirty) continue;
        const saved = await luxCommands.editorSaveFile(document.id);
        replaceDocumentSnapshot(saved);
      }

      const latestDocumentsById = new Map(useLuxStore.getState().openDocuments.map((document) => [document.id, document]));
      const stillDirtyDocuments = request.documentIds
        .map((documentId) => latestDocumentsById.get(documentId))
        .filter((document): document is DocumentSnapshot => Boolean(document?.is_dirty));

      if (stillDirtyDocuments.length > 0) {
        setPendingRequest({ ...request, documents: stillDirtyDocuments, message: closeMessage(stillDirtyDocuments.length, t) });
        setError(t("closeGuard.changedWhileSaving"));
        return;
      }

      request.action();
      setPendingRequest(null);
    } catch (error) {
      setError(readErrorMessage(error, t));
    } finally {
      setSaving(false);
    }
  }, [pendingRequest, replaceDocumentSnapshot, saving, t]);

  const contextValue = useMemo(() => ({ requestCloseDocuments }), [requestCloseDocuments]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void getCurrentWindow().onCloseRequested((event) => {
      const openDocuments = useLuxStore.getState().openDocuments;
      if (!openDocuments.some((document) => document.is_dirty)) return;

      event.preventDefault();
      requestCloseDocuments(
        openDocuments.map((document) => document.id),
        () => void getCurrentWindow().destroy(),
        { title: t("closeGuard.appCloseTitle") },
      );
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch(() => undefined);

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [requestCloseDocuments, t]);

  return (
    <EditorCloseGuardContext.Provider value={contextValue}>
      {children}
      <Dialog.Root open={Boolean(pendingRequest)} onOpenChange={(open) => { if (!open) cancel(); }}>
        <Dialog.Portal>
          <Dialog.Overlay className="unsaved-overlay" />
          <Dialog.Content className="unsaved-dialog" aria-describedby="unsaved-dialog-description">
            <div className="unsaved-header">
              <span className="unsaved-icon"><AlertTriangle size={18} /></span>
              <div>
                <Dialog.Title>{pendingRequest?.title}</Dialog.Title>
                <Dialog.Description id="unsaved-dialog-description">{pendingRequest?.message}</Dialog.Description>
              </div>
            </div>
            <div className="unsaved-doc-list" role="list" aria-label={t("closeGuard.unsavedFiles")}>
              {pendingRequest?.documents.map((document) => (
                <div className="unsaved-doc-row" role="listitem" key={document.id} title={documentDisplayPath(document)}>
                  <FileCode2 size={15} />
                  <span>{documentTitle(document)}</span>
                  <small>{documentParentLabel(document)}</small>
                </div>
              ))}
            </div>
            {error ? <div className="unsaved-error">{error}</div> : null}
            <div className="unsaved-actions">
              <button className="secondary-button" type="button" disabled={saving} onClick={discardAndClose}>{t("closeGuard.dontSave")}</button>
              <button className="secondary-button" type="button" disabled={saving} onClick={cancel}>{t("common.cancel")}</button>
              <button className="primary-button" type="button" disabled={saving} onClick={() => void saveAndClose()}>{saving ? t("closeGuard.saving") : t("common.save")}</button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </EditorCloseGuardContext.Provider>
  );
}

export function useEditorCloseGuard() {
  const context = useContext(EditorCloseGuardContext);
  if (!context) throw new Error("useEditorCloseGuard must be used inside EditorCloseGuardProvider");
  return context;
}

function closeMessage(count: number, t: TranslateFn) {
  return count === 1
    ? t("closeGuard.message.single")
    : t("closeGuard.message.multiple", { count });
}

function readErrorMessage(error: unknown, t: TranslateFn) {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return t("closeGuard.saveFailed");
}
