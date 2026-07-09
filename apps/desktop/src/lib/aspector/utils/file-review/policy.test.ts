import { describe, expect, it } from "vitest";

import { shouldSkipEditReview } from "./policy";

describe("shouldSkipEditReview", () => {
  it("always skips in automatic mode (full autonomy, nothing to accept/reject)", () => {
    expect(shouldSkipEditReview({ agentMode: "automatic", fileEditTrustMode: "preview-before-apply" }, false)).toBe(true);
    expect(shouldSkipEditReview({ agentMode: "automatic", fileEditTrustMode: "apply-immediately" }, true)).toBe(true);
  });

  it("skips saved-to-disk edits under apply-immediately trust", () => {
    // "Принятие сразу": the user pre-accepted edits, so no Accept/Reject bar.
    expect(shouldSkipEditReview({ agentMode: "agent", fileEditTrustMode: "apply-immediately" }, false)).toBe(true);
  });

  it("keeps preview-only edits reviewable even under apply-immediately trust", () => {
    // A staged edit (saveToDisk=false) is persisted BY Accept — skipping its
    // review would silently lose the edit.
    expect(shouldSkipEditReview({ agentMode: "agent", fileEditTrustMode: "apply-immediately" }, true)).toBe(false);
  });

  it("keeps reviews in preview-before-apply trust for interactive modes", () => {
    expect(shouldSkipEditReview({ agentMode: "agent", fileEditTrustMode: "preview-before-apply" }, false)).toBe(false);
    expect(shouldSkipEditReview({ agentMode: "ask", fileEditTrustMode: "preview-before-apply" }, true)).toBe(false);
  });
});
