import { describe, expect, it } from "vitest";
import { appendTerminalChunks, emptyTerminalBuffer, MAX_TERMINAL_BUFFER_CHARS, terminalOutputCoalescer } from "./output";

const nextFlush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

describe("terminalOutputCoalescer", () => {
  it("delivers enqueued chunks to the sink (v1.0.17 clear-before-sink regression)", async () => {
    const seen: Array<Map<string, string[]>> = [];
    terminalOutputCoalescer.setSink((pending) => seen.push(new Map(pending)));

    terminalOutputCoalescer.enqueue("t1", "hello ");
    terminalOutputCoalescer.enqueue("t1", "world");
    terminalOutputCoalescer.enqueue("t2", "other");
    await nextFlush();

    // The batch handed to the sink must still CONTAIN the chunks — flushing by
    // clear()ing the shared map handed the sink an empty map, so every terminal
    // rendered nothing while the PTY kept streaming.
    expect(seen).toHaveLength(1);
    expect(seen[0].get("t1")).toEqual(["hello ", "world"]);
    expect(seen[0].get("t2")).toEqual(["other"]);
  });

  it("keeps post-flush chunks isolated from the delivered batch", async () => {
    const seen: Array<Map<string, string[]>> = [];
    terminalOutputCoalescer.setSink((pending) => seen.push(new Map(pending)));

    terminalOutputCoalescer.enqueue("t1", "first");
    await nextFlush();
    terminalOutputCoalescer.enqueue("t1", "second");
    await nextFlush();

    expect(seen).toHaveLength(2);
    expect(seen[0].get("t1")).toEqual(["first"]);
    expect(seen[1].get("t1")).toEqual(["second"]);
  });

  it("discard drops queued chunks before they reach the sink", async () => {
    const seen: Array<Map<string, string[]>> = [];
    terminalOutputCoalescer.setSink((pending) => seen.push(new Map(pending)));

    terminalOutputCoalescer.enqueue("t1", "doomed");
    terminalOutputCoalescer.enqueue("t2", "survives");
    terminalOutputCoalescer.discard("t1");
    await nextFlush();

    expect(seen).toHaveLength(1);
    expect(seen[0].has("t1")).toBe(false);
    expect(seen[0].get("t2")).toEqual(["survives"]);
  });
});

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
