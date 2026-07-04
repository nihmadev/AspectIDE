import { describe, expect, it } from "vitest";
import {
  findLastUserMessageIndex,
  isAbortError,
  messageHasAssistantWork,
  readErrorMessage,
  replaceEmptyAssistantTail,
  restoreUnansweredUserMessage,
  statusToSessionStatus,
  stripTrailingErrorBubble,
} from "./aiChatPanelTurnHelpers";
import type { AiChatMessage, AiChatToolCall } from "./aiChatTypes";

const message = (over: Partial<AiChatMessage>): AiChatMessage => ({
  id: over.id ?? "m",
  role: over.role ?? "assistant",
  content: over.content ?? "",
  timestamp: over.timestamp ?? 0,
  ...over,
});

const toolCall: AiChatToolCall = { id: "t", tool: "Read", status: "success", startTime: 0 };

describe("findLastUserMessageIndex", () => {
  it("returns the index of the last user message, or -1 when none", () => {
    const history = [message({ role: "user" }), message({ role: "assistant" }), message({ role: "user", id: "u2" })];
    expect(findLastUserMessageIndex(history)).toBe(2);
    expect(findLastUserMessageIndex([message({ role: "assistant" })])).toBe(-1);
    expect(findLastUserMessageIndex([])).toBe(-1);
  });
});

describe("messageHasAssistantWork", () => {
  it("is true when there are segments, tool calls, or reasoning", () => {
    expect(messageHasAssistantWork(message({ toolCalls: [toolCall] }))).toBe(true);
    expect(messageHasAssistantWork(message({ reasoning: "thinking…" }))).toBe(true);
    expect(messageHasAssistantWork(message({ content: "bare error" }))).toBe(false);
  });
});

describe("stripTrailingErrorBubble", () => {
  it("drops a trailing error bubble that matches lastError and carries no work", () => {
    const history = [message({ role: "user" }), message({ content: "Rate limited" })];
    expect(stripTrailingErrorBubble(history, "Rate limited")).toHaveLength(1);
  });

  it("keeps the trailing message when it carries real assistant work", () => {
    const history = [message({ role: "user" }), message({ content: "Rate limited", toolCalls: [toolCall] })];
    expect(stripTrailingErrorBubble(history, "Rate limited")).toBe(history);
  });

  it("keeps the message when the content does not match lastError", () => {
    const history = [message({ content: "different" })];
    expect(stripTrailingErrorBubble(history, "Rate limited")).toBe(history);
  });
});

describe("replaceEmptyAssistantTail", () => {
  const errorBubble = message({ id: "err", content: "boom", timestamp: 42 });

  it("folds the error into a trailing empty assistant shell", () => {
    const history = [message({ role: "user" }), message({ id: "shell" })];
    const next = replaceEmptyAssistantTail(history, errorBubble);
    expect(next).toHaveLength(2);
    expect(next[1].id).toBe("shell");
    expect(next[1].content).toBe("boom");
    expect(next[1].timestamp).toBe(42);
  });

  it("appends a new bubble when the tail already has content", () => {
    const history = [message({ content: "real answer" })];
    const next = replaceEmptyAssistantTail(history, errorBubble);
    expect(next).toHaveLength(2);
    expect(next[1]).toBe(errorBubble);
  });
});

describe("statusToSessionStatus", () => {
  it("is the identity over runtime statuses", () => {
    expect(statusToSessionStatus("streaming")).toBe("streaming");
    expect(statusToSessionStatus("waiting-approval")).toBe("waiting-approval");
  });
});

describe("isAbortError", () => {
  it("recognizes an AbortError DOMException only", () => {
    expect(isAbortError(new DOMException("aborted", "AbortError"))).toBe(true);
    expect(isAbortError(new DOMException("nope", "OtherError"))).toBe(false);
    expect(isAbortError(new Error("plain"))).toBe(false);
  });
});

describe("readErrorMessage", () => {
  it("reads Error.message, else stringifies", () => {
    expect(readErrorMessage(new Error("kaboom"))).toBe("kaboom");
    expect(readErrorMessage("just a string")).toBe("just a string");
  });
});

describe("restoreUnansweredUserMessage", () => {
  it("recovers the dangling user message and removes it from the transcript", () => {
    const earlier = [
      message({ id: "u1", role: "user", content: "first" }),
      message({ id: "a1", content: "answered" }),
    ];
    const dangling = message({ id: "u2", role: "user", content: "почини баг в поиске" });
    const result = restoreUnansweredUserMessage([...earlier, dangling], null);

    expect(result).not.toBeNull();
    expect(result!.draft).toBe("почини баг в поиске");
    expect(result!.messages).toEqual(earlier);
  });

  it("strips the trailing error bubble before recovering", () => {
    const lastError = "HTTP 429: rate limited";
    const history = [
      message({ id: "u1", role: "user", content: "stuck request" }),
      message({ id: "err", content: lastError }),
    ];
    const result = restoreUnansweredUserMessage(history, lastError);

    expect(result).not.toBeNull();
    expect(result!.draft).toBe("stuck request");
    expect(result!.messages).toEqual([]);
  });

  it("strips an empty assistant shell before recovering", () => {
    const history = [
      message({ id: "u1", role: "user", content: "hello there" }),
      message({ id: "shell", content: "" }),
    ];
    const result = restoreUnansweredUserMessage(history, null);

    expect(result).not.toBeNull();
    expect(result!.draft).toBe("hello there");
    expect(result!.messages).toEqual([]);
  });

  it("returns null when the turn produced real assistant work", () => {
    const history = [
      message({ id: "u1", role: "user", content: "do the thing" }),
      message({ id: "a1", toolCalls: [toolCall] }),
    ];
    expect(restoreUnansweredUserMessage(history, null)).toBeNull();
  });

  it("returns null when the tail assistant message has a real answer", () => {
    const history = [
      message({ id: "u1", role: "user", content: "question" }),
      message({ id: "a1", content: "a real answer" }),
    ];
    expect(restoreUnansweredUserMessage(history, null)).toBeNull();
  });

  it("returns null for special user-role messages and empty content", () => {
    expect(restoreUnansweredUserMessage(
      [message({ role: "user", content: "[Lux · context compacted]…", kind: "compaction-checkpoint" })],
      null,
    )).toBeNull();
    expect(restoreUnansweredUserMessage(
      [message({ role: "user", content: "review it", kind: "review-request" })],
      null,
    )).toBeNull();
    expect(restoreUnansweredUserMessage([message({ role: "user", content: "   " })], null)).toBeNull();
    expect(restoreUnansweredUserMessage([], null)).toBeNull();
  });

  it("leaves messages with attachments in the transcript untouched", () => {
    // The composer's attachment blobs were revoked at send time — restoring
    // only the text would silently destroy the attachment.
    const history = [
      message({
        id: "u1",
        role: "user",
        content: "look at this image",
        attachments: [{ id: "att1", kind: "image", name: "shot.png", size: 1024 }],
      }),
    ];
    expect(restoreUnansweredUserMessage(history, null)).toBeNull();
  });
});
