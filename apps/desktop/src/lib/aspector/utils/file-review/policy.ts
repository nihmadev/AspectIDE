import type { AiFileEditTrustMode } from "./../preferences";

/**
 * Central policy for whether an AI file edit should enter the pending-review
 * queue (green/red diff + Accept/Reject). A review only earns its place when a
 * human is actually expected to act on it:
 *
 * - Automatic mode is full autonomy — nothing to accept/reject, ever.
 * - "apply-immediately" trust means the user already accepted edits up front:
 *   a write that landed on disk is final, so queueing a review would resurface
 *   an Accept/Reject bar for a decision the user delegated. Turn checkpoints
 *   still cover rollback.
 * - EXCEPT preview-only edits (saveToDisk=false): those exist only in the
 *   editor buffer and are persisted BY the Accept action — skipping their
 *   review would silently strand the edit and lose it. They stay reviewable
 *   in every mode.
 */
export function shouldSkipEditReview(
  preferences: { agentMode: string; fileEditTrustMode: AiFileEditTrustMode },
  previewOnly: boolean,
): boolean {
  if (preferences.agentMode === "automatic") return true;
  return !previewOnly && preferences.fileEditTrustMode === "apply-immediately";
}
