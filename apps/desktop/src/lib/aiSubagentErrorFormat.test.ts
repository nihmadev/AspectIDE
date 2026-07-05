import { describe, expect, it } from "vitest";
import { compactSubagentError } from "./aiNativeOrchestrationBridge";

describe("compactSubagentError", () => {
  it("extracts the message from provider JSON error envelopes", () => {
    expect(compactSubagentError('{"error":{"message":"402 Insufficient balance","type":"x"}}'))
      .toBe("402 Insufficient balance");
    expect(compactSubagentError('{"error":"rate limited"}')).toBe("rate limited");
    expect(compactSubagentError('{"message":"bad request"}')).toBe("bad request");
  });

  it("collapses whitespace and passes through plain text", () => {
    expect(compactSubagentError("connection   reset\n  by peer")).toBe("connection reset by peer");
  });

  it("clamps very long messages", () => {
    const long = "e".repeat(500);
    const out = compactSubagentError(long);
    expect(out.length).toBeLessThanOrEqual(240);
    expect(out.endsWith("…")).toBe(true);
  });

  it("falls back for empty input", () => {
    expect(compactSubagentError("")).toBe("Subagent failed");
    expect(compactSubagentError("   ")).toBe("Subagent failed");
  });
});
