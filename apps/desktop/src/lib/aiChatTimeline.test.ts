import { describe, expect, it } from "vitest";

import { createTurnTimeline } from "./aiChatTimeline";

describe("createTurnTimeline addNotice", () => {
  it("places the plaque at the current position and keeps it out of model content", () => {
    const timeline = createTurnTimeline(() => {});
    timeline.commitRound("round one answer", "");
    timeline.addNotice(
      { type: "reasoning-fallback", requested: "minimal", applied: "max" },
      'Reasoning effort "minimal" is not accepted by the provider — "max" was applied.',
    );
    timeline.beginRound();
    timeline.commitRound("round two answer", "");

    const snapshot = timeline.snapshot();
    const segments = snapshot.segments ?? [];
    // The notice sits exactly between the rounds it interrupted.
    expect(segments.map((segment) => segment.kind)).toEqual(["text", "notice", "text"]);
    const notice = segments[1];
    if (notice.kind !== "notice") throw new Error("expected a notice segment");
    expect(notice.notice.requested).toBe("minimal");
    expect(notice.notice.applied).toBe("max");
    expect(notice.text).toContain("minimal");
    // Model-visible content must not carry the plaque text.
    expect(snapshot.content).toBe("round one answer\n\nround two answer");
    expect(snapshot.reasoning).toBeUndefined();
  });
});
