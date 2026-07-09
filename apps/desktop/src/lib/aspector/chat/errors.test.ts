import { describe, expect, it } from "vitest";
import { classifyAiChatError } from "./errors";
import type { TranslateFn } from "../../i18n/useTranslation";

// The translator is irrelevant to classification; echo the key.
const t = ((key: string) => key) as unknown as TranslateFn;

describe("classifyAiChatError context overflow", () => {
  it("classifies provider context-length errors as context-overflow", () => {
    const messages = [
      "This model's maximum context length is 128000 tokens, however you requested 490000",
      "context_length_exceeded",
      "prompt is too long: 480000 tokens",
      "Please reduce the length of the messages",
      "input is too long for the requested model",
    ];
    for (const message of messages) {
      const result = classifyAiChatError(new Error(message), t);
      expect(result.kind, message).toBe("context-overflow");
      expect(result.canRetry).toBe(true);
    }
  });

  it("does not misclassify unrelated errors as context-overflow", () => {
    expect(classifyAiChatError(new Error("429 too many requests"), t).kind).toBe("rate-limit");
    expect(classifyAiChatError(new Error("connection reset by peer"), t).kind).not.toBe("context-overflow");
    expect(classifyAiChatError(new Error("401 unauthorized"), t).kind).toBe("auth");
  });
});
