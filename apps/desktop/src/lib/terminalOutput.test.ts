import { describe, expect, it } from "vitest";
import { appendTerminalChunks, emptyTerminalBuffer, MAX_TERMINAL_BUFFER_CHARS } from "./terminalOutput";

describe("appendTerminalChunks", () => {
  it("returns the previous buffer unchanged for an empty batch", () => {
    const previous = emptyTerminalBuffer();
    expect(appendTerminalChunks(previous, [])).toBe(previous);
  });

  it("folds multiple chunks in one pass with correct counters", () => {
    const result = appendTerminalChunks(undefined, ["foo", "bar", "baz"]);
    expect(result.text).toBe("foobarbaz");
    expect(result.bytes).toBe(9);
    expect(result.chunks).toBe(3);
    expect(result.truncated).toBe(false);
  });

  it("accumulates across calls", () => {
    const first = appendTerminalChunks(undefined, ["a"]);
    const second = appendTerminalChunks(first, ["b", "c"]);
    expect(second.text).toBe("abc");
    expect(second.chunks).toBe(3);
  });

  it("tail-keeps and flags truncation past the cap", () => {
    const big = "x".repeat(MAX_TERMINAL_BUFFER_CHARS + 100);
    const result = appendTerminalChunks(undefined, [big]);
    expect(result.text).toHaveLength(MAX_TERMINAL_BUFFER_CHARS);
    expect(result.truncated).toBe(true);
    // Tail-keep: the newest bytes survive.
    expect(result.text.endsWith("x")).toBe(true);
    expect(result.bytes).toBe(MAX_TERMINAL_BUFFER_CHARS + 100);
  });
});
