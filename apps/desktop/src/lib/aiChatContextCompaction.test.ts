import { beforeEach, describe, expect, it, vi } from "vitest";

// `./tauri` reads `window.__TAURI_INTERNALS__` at call time, which is absent under
// the node test runner. Mock the seam so the compaction code can exercise its
// Tauri-runtime branch deterministically.
const aiCompactionSummary = vi.fn();
vi.mock("./tauri", () => ({
  isTauriRuntime: () => true,
  luxCommands: { aiCompactionSummary: (...args: unknown[]) => aiCompactionSummary(...args) },
}));

import {
  compactChatHistory,
  isCompactionCheckpointMessage,
  type CompactChatHistoryInput,
} from "./aiChatContextCompaction";
import type { AiChatMessage } from "./aiChatTypes";
import type { AiModelConfig, AiProviderConfig } from "./aiPreferences";

const SMALL_CONTEXT_TOKENS = 8_000;
const LOW_THRESHOLD = 0.5;
// Comfortably above the non-forced preserve window (10) so the auto path compacts
// enough older turns to clear the 8% min-reduction guard, not just a token or two.
const OVER_THRESHOLD_MESSAGE_COUNT = 30;
const LARGE_CONTENT = "a".repeat(1_600); // ~432 estimated tokens per message

const model: AiModelConfig = {
  id: "test-model",
  name: "Test Model",
  alias: "",
  contextTokens: SMALL_CONTEXT_TOKENS,
  effortLevels: [],
};

const provider: AiProviderConfig = {
  id: "test-provider",
  name: "Test Provider",
  providerType: "openai",
  protocol: "openai-compatible",
  baseUrl: "https://example.test",
  apiKey: "",
  localHost: "",
  localPort: "",
  localPath: "",
  models: [model],
  embeddingModel: "",
};

function makeMessages(count: number): AiChatMessage[] {
  return Array.from({ length: count }, (_unused, index) => ({
    id: `m-${index}`,
    role: index % 2 === 0 ? "user" : "assistant",
    content: `${LARGE_CONTENT} #${index}`,
    timestamp: index,
  }));
}

function baseInput(overrides: Partial<CompactChatHistoryInput>): CompactChatHistoryInput {
  return {
    messages: makeMessages(OVER_THRESHOLD_MESSAGE_COUNT),
    compactionState: null,
    model,
    provider,
    selectedEffortId: "",
    threshold: LOW_THRESHOLD,
    autoCompactEnabled: true,
    ...overrides,
  };
}

const FULL_SUMMARY = [
  "## Task goal",
  "Continue the task.",
  "## Latest user direction",
  "Keep going.",
  "## Open tasks",
  "- none",
  "## Progress",
  "- did things",
  "## Key decisions / constraints",
  "- none",
  "## Files and tools",
  "- none",
  "## Errors / blockers",
  "- none",
  "## Critical preserved facts",
  "- none",
  "## Open items / next step",
  "Resume.",
].join("\n");

beforeEach(() => {
  aiCompactionSummary.mockReset();
  aiCompactionSummary.mockResolvedValue(FULL_SUMMARY);
});

describe("compactChatHistory disabled-auto guard", () => {
  it("never summarizes an over-threshold transcript when auto-compaction is disabled and not forced", async () => {
    const input = baseInput({ autoCompactEnabled: false, force: false });

    const result = await compactChatHistory(input);

    // The bug was that an over-threshold transcript fell through the disabled guard
    // and compacted anyway. With the fix it must be left untouched.
    expect(result.compacted).toBe(false);
    expect(result.reason).toBe("below-threshold");
    expect(result.messages).toBe(input.messages);
    expect(aiCompactionSummary).not.toHaveBeenCalled();
  });

  it("still forces compaction even when auto-compaction is disabled", async () => {
    const input = baseInput({ autoCompactEnabled: false, force: true });

    const result = await compactChatHistory(input);

    expect(result.compacted).toBe(true);
    expect(aiCompactionSummary).toHaveBeenCalledTimes(1);
    expect(result.messages.length).toBeLessThan(input.messages.length);
    const [checkpoint] = result.messages;
    expect(checkpoint).toBeDefined();
    expect(isCompactionCheckpointMessage(checkpoint!)).toBe(true);
    expect(result.compactionState?.droppedItems?.length).toBeGreaterThan(0);
  });
});

describe("compactChatHistory auto over-threshold", () => {
  it("creates a durable checkpoint when enabled and over the trigger", async () => {
    const input = baseInput({ autoCompactEnabled: true, force: false });

    const result = await compactChatHistory(input);

    expect(result.compacted).toBe(true);
    expect(aiCompactionSummary).toHaveBeenCalledTimes(1);
    expect(result.compactionState?.generation).toBe(1);
    expect(isCompactionCheckpointMessage(result.messages[0]!)).toBe(true);
  });
});

describe("compactChatHistory forced on short chats", () => {
  it("compacts a single-message chat when forced", async () => {
    const input = baseInput({ messages: makeMessages(1), force: true });

    const result = await compactChatHistory(input);

    expect(result.compacted).toBe(true);
    expect(aiCompactionSummary).toHaveBeenCalledTimes(1);
    expect(result.messages).toHaveLength(1);
    expect(isCompactionCheckpointMessage(result.messages[0]!)).toBe(true);
  });

  it("compacts exactly-preserve-window-sized chats instead of bailing already-checkpoint-only", async () => {
    // Old cliff: 5 eligible messages passed the forced floor (5) but the fixed
    // forced preserve window (5) swallowed the whole transcript, leaving nothing
    // older to summarize — forced compaction silently did nothing.
    const input = baseInput({ messages: makeMessages(5), force: true });

    const result = await compactChatHistory(input);

    expect(result.compacted).toBe(true);
    expect(result.reason).toBeUndefined();
    expect(isCompactionCheckpointMessage(result.messages[0]!)).toBe(true);
    expect(result.messages.length).toBeLessThanOrEqual(input.messages.length);
  });

  it("re-compacts a checkpoint followed by one new message when forced", async () => {
    const [checkpointSource] = makeMessages(1);
    const checkpoint: AiChatMessage = {
      ...checkpointSource!,
      id: "ckpt-1",
      kind: "compaction-checkpoint",
      content: "[Lux · context compacted]\ncovered_messages=4\nPrior summary body.",
    };
    const follow = { ...makeMessages(2)[1]!, id: "m-follow" };
    const input = baseInput({ messages: [checkpoint, follow], force: true });

    const result = await compactChatHistory(input);

    expect(result.compacted).toBe(true);
    expect(result.messages).toHaveLength(1);
    expect(isCompactionCheckpointMessage(result.messages[0]!)).toBe(true);
    // The new checkpoint merges the prior one's covered count instead of dropping it.
    expect(result.messages[0]!.content).toContain("covered_messages=5");
  });

  it("still refuses a chat with nothing eligible to compact", async () => {
    const [checkpointSource] = makeMessages(1);
    const checkpoint: AiChatMessage = {
      ...checkpointSource!,
      id: "ckpt-only",
      kind: "compaction-checkpoint",
      content: "[Lux · context compacted]\ncovered_messages=4\nPrior summary body.",
    };
    const input = baseInput({ messages: [checkpoint], force: true });

    const result = await compactChatHistory(input);

    expect(result.compacted).toBe(false);
    expect(result.reason).toBe("too-few-messages");
    expect(aiCompactionSummary).not.toHaveBeenCalled();
  });
});
