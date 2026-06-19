import { useCallback, useMemo, useState } from "react";
import {
  createComposerFileAttachment,
  createComposerMentionAttachment,
  createComposerSelectionAttachment,
  revokeComposerAttachmentPreview,
  type ComposerAttachment,
} from "./aiChatComposerAttachments";
import { setComposerAttachments } from "./aiChatComposerSession";
import { documentDisplayPath, documentTitle } from "./documents";
import { formatSelectionLabel, getEditorSelectionSnapshot } from "./editorSelectionBridge";
import type { AiMentionCandidate } from "./aiChatMentions";
import type { DocumentSnapshot } from "./types";

/**
 * Owns the composer's attachment list and the actions that mutate it (file drops,
 * @-mentions, editor selections, pinned editor tabs). Each mutation persists the
 * new set into the per-session composer store so it survives session switches.
 *
 * Returns `attachments`/`setAttachments` (the latter for the panel's hydrate and
 * post-send clear), the derived `pinnedEditorPaths` for context reporting, and the
 * attach/remove helpers wired straight to the composer.
 */
export function useAiChatComposerAttachments({
  sessionId,
  openDocuments,
}: {
  sessionId: string;
  openDocuments: DocumentSnapshot[];
}) {
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([]);

  const pinnedEditorPaths = useMemo(
    () => attachments.flatMap((attachment) => {
      if (attachment.kind === "editor") {
        const document = openDocuments.find((candidate) => candidate.id === attachment.documentId);
        return document ? [documentDisplayPath(document)] : [attachment.name];
      }
      if (attachment.kind === "mention" && attachment.path) return [attachment.path];
      if (attachment.kind === "selection") return [attachment.path];
      return [];
    }),
    [attachments, openDocuments],
  );

  const attachFiles = (files: FileList | File[] | null) => {
    if (!files || files.length === 0) return;
    // Mint each blob preview URL exactly once in the event handler, never inside
    // the updater — React may invoke an updater more than once (StrictMode/dev or
    // a discarded concurrent render), which would leak the discarded URL.
    const incoming = Array.from(files).map((file) => createComposerFileAttachment(file));
    setAttachments((current) => {
      const byId = new Map(current.map((attachment) => [attachment.id, attachment]));
      for (const next of incoming) {
        const existing = byId.get(next.id);
        if (existing?.kind === "file") revokeComposerAttachmentPreview(existing);
        byId.set(next.id, next);
      }
      const nextAttachments = [...byId.values()];
      setComposerAttachments(sessionId, nextAttachments);
      return nextAttachments;
    });
  };

  const attachMention = useCallback((candidate: AiMentionCandidate) => {
    const next = createComposerMentionAttachment({
      mentionType: candidate.kind,
      name: candidate.label,
      path: candidate.path,
      symbolName: candidate.symbolName,
      line: candidate.line,
      column: candidate.column,
    });
    setAttachments((current) => {
      const byId = new Map(current.map((attachment) => [attachment.id, attachment]));
      byId.set(next.id, next);
      const nextAttachments = [...byId.values()];
      setComposerAttachments(sessionId, nextAttachments);
      return nextAttachments;
    });
  }, [sessionId]);

  const attachSelection = useCallback((selection = getEditorSelectionSnapshot()) => {
    if (!selection) return false;
    const next = createComposerSelectionAttachment({
      documentId: selection.documentId,
      name: formatSelectionLabel(selection),
      path: selection.path,
      text: selection.text,
      startLine: selection.startLine,
      endLine: selection.endLine,
      startColumn: selection.startColumn,
      endColumn: selection.endColumn,
      languageId: selection.languageId,
    });
    setAttachments((current) => {
      const byId = new Map(current.map((attachment) => [attachment.id, attachment]));
      byId.set(next.id, next);
      const nextAttachments = [...byId.values()];
      setComposerAttachments(sessionId, nextAttachments);
      return nextAttachments;
    });
    return true;
  }, [sessionId]);

  const attachEditorDocument = useCallback((documentId: string) => {
    const document = openDocuments.find((candidate) => candidate.id === documentId);
    if (!document) return;
    const id = `editor:${documentId}`;
    const name = documentTitle(document);
    setAttachments((current) => {
      const byId = new Map(current.map((attachment) => [attachment.id, attachment]));
      byId.set(id, { kind: "editor", documentId, id, name, size: document.text.length });
      const nextAttachments = [...byId.values()];
      setComposerAttachments(sessionId, nextAttachments);
      return nextAttachments;
    });
  }, [sessionId, openDocuments]);

  const removeAttachment = (id: string) => {
    setAttachments((current) => {
      const removed = current.find((attachment) => attachment.id === id);
      if (removed) revokeComposerAttachmentPreview(removed);
      const nextAttachments = current.filter((attachment) => attachment.id !== id);
      setComposerAttachments(sessionId, nextAttachments);
      return nextAttachments;
    });
  };

  return {
    attachments,
    setAttachments,
    pinnedEditorPaths,
    attachFiles,
    attachMention,
    attachSelection,
    attachEditorDocument,
    removeAttachment,
  };
}
