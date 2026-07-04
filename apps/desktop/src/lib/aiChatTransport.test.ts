import { describe, expect, it } from "vitest";

import { reasoningPayload, resolveWireReasoningEffort } from "./aiChatTransport";
import type { AiModelConfig, AiProviderConfig } from "./aiPreferences";

function provider(overrides: Partial<AiProviderConfig> = {}): AiProviderConfig {
  return {
    providerType: "openai",
    protocol: "openai",
    baseUrl: "https://api.example.com/v1",
    apiKey: "key",
    ...overrides,
  } as AiProviderConfig;
}

function model(effortLevels: AiModelConfig["effortLevels"]): Pick<AiModelConfig, "effortLevels"> {
  return { effortLevels };
}

describe("resolveWireReasoningEffort", () => {
  it("passes preset effort ids through unchanged", () => {
    for (const id of ["minimal", "low", "medium", "high", "xhigh", "max"]) {
      expect(resolveWireReasoningEffort(id)).toBe(id);
    }
  });

  it("returns empty string for empty effort id", () => {
    expect(resolveWireReasoningEffort("")).toBe("");
  });

  it("drops custom effort ids that are not known wire variants", () => {
    // createAiEffortConfig generates ids like "effort"/"effort-2"; strict providers
    // reject them with 400 "unknown variant `effort`".
    expect(resolveWireReasoningEffort("effort")).toBe("");
    expect(resolveWireReasoningEffort("effort-2", model([{ id: "effort-2", label: "хуи" }]))).toBe("");
  });

  it("maps custom efforts by label when the label names a real level", () => {
    expect(resolveWireReasoningEffort("effort", model([{ id: "effort", label: "Max" }]))).toBe("max");
    expect(resolveWireReasoningEffort("effort-3", model([{ id: "effort-3", label: "  NONE  " }]))).toBe("none");
  });

  it("normalizes casing and whitespace on the id itself", () => {
    expect(resolveWireReasoningEffort(" High ")).toBe("high");
  });
});

describe("reasoningPayload", () => {
  it("sends reasoning_effort for known preset ids", () => {
    expect(reasoningPayload("medium", provider())).toEqual({ reasoning_effort: "medium" });
  });

  it("omits the reasoning field entirely for unknown custom effort ids", () => {
    expect(reasoningPayload("effort", provider())).toEqual({});
    expect(reasoningPayload("effort", provider(), model([{ id: "effort", label: "хуи" }]))).toEqual({});
  });

  it("downgrades xhigh/max to high for non-local-proxy providers", () => {
    expect(reasoningPayload("xhigh", provider())).toEqual({ reasoning_effort: "high" });
    expect(reasoningPayload("max", provider())).toEqual({ reasoning_effort: "high" });
    expect(reasoningPayload("xhigh", provider({ protocol: "local-proxy" } as Partial<AiProviderConfig>))).toEqual({
      reasoning_effort: "xhigh",
    });
    expect(reasoningPayload("max", provider({ protocol: "local-proxy" } as Partial<AiProviderConfig>))).toEqual({
      reasoning_effort: "max",
    });
  });

  it("wraps the effort for openrouter providers", () => {
    expect(reasoningPayload("low", provider({ providerType: "openrouter" } as Partial<AiProviderConfig>))).toEqual({
      reasoning: { effort: "low" },
    });
  });

  it("resolves label-named custom efforts before wrapping", () => {
    expect(reasoningPayload("effort", provider(), model([{ id: "effort", label: "Minimal" }]))).toEqual({
      reasoning_effort: "minimal",
    });
  });
});
