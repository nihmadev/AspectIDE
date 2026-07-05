import { describe, expect, it } from "vitest";
import { appendAiChatErrorHistory, type AiChatErrorHistoryEntry } from "./store";

const entry = (message: string, count = 1): AiChatErrorHistoryEntry => ({ message, timestamp: 0, count });

describe("appendAiChatErrorHistory", () => {
  it("appends a new entry to an empty or undefined history", () => {
    expect(appendAiChatErrorHistory(undefined, "boom")).toEqual([
      { message: "boom", timestamp: expect.any(Number), count: 1 },
    ]);
    expect(appendAiChatErrorHistory([], "boom")).toHaveLength(1);
  });

  it("collapses a consecutive identical failure into a bumped count", () => {
    const next = appendAiChatErrorHistory([entry("403 expired")], "403 expired");
    expect(next).toHaveLength(1);
    expect(next[0].count).toBe(2);
    expect(next[0].timestamp).toBeGreaterThan(0);
  });

  it("keeps distinct consecutive failures as separate entries", () => {
    const next = appendAiChatErrorHistory([entry("403 expired")], "timeout");
    expect(next.map((item) => item.message)).toEqual(["403 expired", "timeout"]);
  });

  it("does not collapse into a non-adjacent duplicate", () => {
    const history = [entry("403 expired"), entry("timeout")];
    const next = appendAiChatErrorHistory(history, "403 expired");
    expect(next.map((item) => item.message)).toEqual(["403 expired", "timeout", "403 expired"]);
  });

  it("caps the history at 8 entries, dropping the oldest", () => {
    let history: AiChatErrorHistoryEntry[] | undefined;
    for (let index = 0; index < 10; index += 1) {
      history = appendAiChatErrorHistory(history, `error ${index}`);
    }
    expect(history).toHaveLength(8);
    expect(history?.[0].message).toBe("error 2");
    expect(history?.[7].message).toBe("error 9");
  });
});
