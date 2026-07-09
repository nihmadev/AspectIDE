import type { DocumentSnapshot } from "../types/index";
import { useLuxStore } from "../store/index";

export const AGENT_BROWSER_PREVIEW_PATH_PREFIX = "lux://agent-browser-preview/";

export function agentBrowserPreviewPath(chatSessionId: string) {
  return `${AGENT_BROWSER_PREVIEW_PATH_PREFIX}${chatSessionId}`;
}

export function chatSessionIdFromBrowserPreviewPath(path: string | null) {
  if (!path?.startsWith(AGENT_BROWSER_PREVIEW_PATH_PREFIX)) return null;
  const sessionId = path.slice(AGENT_BROWSER_PREVIEW_PATH_PREFIX.length).trim();
  return sessionId || null;
}

export function agentBrowserPreviewDocumentId(chatSessionId: string) {
  return `browser-preview-${chatSessionId}`;
}

export function isAgentBrowserPreviewDocument(document: Pick<DocumentSnapshot, "path">) {
  return chatSessionIdFromBrowserPreviewPath(document.path) !== null;
}

export function createAgentBrowserPreviewDocument(chatSessionId: string, title: string): DocumentSnapshot {
  return {
    id: agentBrowserPreviewDocumentId(chatSessionId),
    path: agentBrowserPreviewPath(chatSessionId),
    title,
    language_id: "plaintext",
    text: "",
    view: {
      category: "unknown",
      strategy: "externalOnly",
      mode: "preview",
      displayName: "Browser preview",
      mimeType: null,
      extensions: [],
      editable: false,
      previewable: true,
      aiReadable: false,
      binary: false,
      maxInlineBytes: null,
      notes: [],
    },
    version: 0,
    is_dirty: false,
    is_untitled: false,
    opened_at: new Date().toISOString(),
  };
}

export function openAgentBrowserPreviewTab(chatSessionId: string, title: string) {
  useLuxStore.getState().upsertDocument(createAgentBrowserPreviewDocument(chatSessionId, title));
}