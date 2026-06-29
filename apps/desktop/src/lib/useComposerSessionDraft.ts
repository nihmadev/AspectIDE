import { useEffect, useRef } from "react";
import { getComposerAttachments, getComposerDraft } from "./aiChatComposerSession";
import type { ComposerAttachment } from "./aiChatComposerAttachments";

type ComposerSessionDraftOptions = {
  /** The session whose persisted draft should populate the composer. */
  sessionId: string;
  setMessage: (message: string) => void;
  setAttachments: (attachments: ComposerAttachment[]) => void;
  /** Reset transient composer UI (context popover, drag overlay) on session change. */
  resetComposerUi: () => void;
  /** Re-measure the textarea height after the draft text is applied. */
  resizeComposerTextarea: () => void;
};

/**
 * Hydrates the composer from the active session's own persisted draft and
 * attachments whenever the active session changes.
 *
 * Hydration is deterministic: the composer always reflects the target session's
 * stored draft, never the previous session's unsaved text. Carrying text across
 * sessions could silently send the wrong prompt to the model — the outgoing draft
 * is already persisted on every keystroke (via the composer store), so nothing is
 * lost by switching. A ref guards against re-hydrating the same session on
 * unrelated re-renders, which would otherwise clobber in-progress edits.
 */
export function useComposerSessionDraft({
  sessionId,
  setMessage,
  setAttachments,
  resetComposerUi,
  resizeComposerTextarea,
}: ComposerSessionDraftOptions) {
  const hydratedSessionRef = useRef<string | null>(null);
  useEffect(() => {
    if (hydratedSessionRef.current === sessionId) return;
    hydratedSessionRef.current = sessionId;
    setMessage(getComposerDraft(sessionId));
    setAttachments(getComposerAttachments(sessionId));
    resetComposerUi();
    requestAnimationFrame(() => resizeComposerTextarea());
    // setMessage/setAttachments/resetComposerUi are stable state setters/callbacks,
    // so re-running only on a genuine session switch is intentional.
  }, [sessionId, resizeComposerTextarea]);
}
