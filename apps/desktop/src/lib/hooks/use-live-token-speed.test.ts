import { describe, expect, it } from "vitest";

import type { AiChatMessage } from "./../aspector/chat/types";
import { formatTokenSpeed, latestAssistantMessage } from "./use-live-token-speed";

const message = (over: Partial<AiChatMessage>): AiChatMessage => ({
  id: over.id ?? "m",
  role: over.role ?? "assistant",
  content: over.content ?? "",
  timestamp: over.timestamp ?? 0,
  ...over,
});

describe("latestAssistantMessage", () => {
  it("returns the last assistant message even when user messages follow it", () => {
    // Mid-turn "recommendation" injections append a user message AFTER the
    // streaming assistant message, which keeps being patched in place by id.
    const streaming = message({ id: "a1", content: "streaming…" });
    const history = [
      message({ id: "u1", role: "user", content: "do it" }),
      streaming,
      message({ id: "u2", role: "user", content: "injected note" }),
    ];
    expect(latestAssistantMessage(history)).toBe(streaming);
  });

  it("returns the trailing assistant message in the common case", () => {
    const tail = message({ id: "a2", content: "answer" });
    const history = [message({ id: "u1", role: "user" }), message({ id: "a1" }), tail];
    expect(latestAssistantMessage(history)).toBe(tail);
  });

  it("returns undefined when the session has no assistant message yet", () => {
    expect(latestAssistantMessage([message({ role: "user", content: "hi" })])).toBeUndefined();
    expect(latestAssistantMessage([])).toBeUndefined();
  });
});

describe("formatTokenSpeed", () => {
  it("uses one decimal under 10 and integers above", () => {
    expect(formatTokenSpeed(3.42)).toBe("3.4");
    expect(formatTokenSpeed(28.4)).toBe("28");
  });

  it("clamps negatives to zero", () => {
    expect(formatTokenSpeed(-1)).toBe("0.0");
  });
});
