import type { ComposerAttachment } from "./composer-attachments";

const draftBySessionId = new Map<string, string>();
const attachmentsBySessionId = new Map<string, ComposerAttachment[]>();

export function getComposerDraft(sessionId: string) {
  return draftBySessionId.get(sessionId) ?? "";
}

export function setComposerDraft(sessionId: string, message: string) {
  if (message) draftBySessionId.set(sessionId, message);
  else draftBySessionId.delete(sessionId);
}

export function getComposerAttachments(sessionId: string) {
  return attachmentsBySessionId.get(sessionId) ?? [];
}

export function setComposerAttachments(sessionId: string, attachments: ComposerAttachment[]) {
  if (attachments.length > 0) attachmentsBySessionId.set(sessionId, attachments);
  else attachmentsBySessionId.delete(sessionId);
}

export function clearComposerSessionState(sessionId: string) {
  draftBySessionId.delete(sessionId);
  attachmentsBySessionId.delete(sessionId);
}