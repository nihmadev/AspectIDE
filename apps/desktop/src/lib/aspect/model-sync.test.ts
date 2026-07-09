import { describe, expect, it } from "vitest";

import { standardReasoningEfforts, type AiModelConfig, type AiPreferences } from "../aspector/utils/preferences";
import { normalizeAiPreferences } from "../aspector/utils/preferences";
import { formatCompactTokens, formatAspectUsageLabel, reconcileAspectModels } from "./model-sync";

function model(id: string): AiModelConfig {
  return { id, name: id, alias: id, contextTokens: null, effortLevels: standardReasoningEfforts() };
}

function prefsWithAspect(modelIds: string[], selectedModelId = modelIds[0]): AiPreferences {
  const base = normalizeAiPreferences({}, { preserveText: true });
  const provider = { ...base.providers[0], id: "aspect", providerType: "aspect" as const, models: modelIds.map(model) };
  return { ...base, providers: [provider], selectedProviderId: "aspect", selectedModelId };
}

describe("reconcileAspectModels", () => {
  it("returns null for an empty fetch (never strands the user with no models)", () => {
    const prefs = prefsWithAspect(["glm-4.7", "MiniMax-M2.7"]);
    expect(reconcileAspectModels(prefs, "aspect", [])).toBeNull();
  });

  it("returns null when the fetched list is identical (no store churn)", () => {
    const prefs = prefsWithAspect(["glm-4.7", "MiniMax-M2.7"]);
    const fetched = [model("glm-4.7"), model("MiniMax-M2.7")];
    expect(reconcileAspectModels(prefs, "aspect", fetched)).toBeNull();
  });

  it("returns null for an unknown provider id", () => {
    const prefs = prefsWithAspect(["glm-4.7"]);
    expect(reconcileAspectModels(prefs, "nope", [model("glm-4.7"), model("x")])).toBeNull();
  });

  it("drops a disabled model while keeping the selection on a surviving model", () => {
    const prefs = prefsWithAspect(["glm-4.7", "MiniMax-M2.7"], "MiniMax-M2.7");
    const next = reconcileAspectModels(prefs, "aspect", [model("MiniMax-M2.7"), model("Kimi-K2.6")]);
    expect(next).not.toBeNull();
    expect(next!.providers[0].models.map((m) => m.id)).toEqual(["MiniMax-M2.7", "Kimi-K2.6"]);
    expect(next!.selectedModelId).toBe("MiniMax-M2.7");
  });

  it("resets selection to the first model when the selected one disappears", () => {
    const prefs = prefsWithAspect(["glm-4.7", "MiniMax-M2.7"], "glm-4.7");
    const next = reconcileAspectModels(prefs, "aspect", [model("Kimi-K2.6"), model("MiniMax-M3")]);
    expect(next).not.toBeNull();
    expect(next!.selectedModelId).toBe("Kimi-K2.6");
    expect(next!.selectedEffortId).toBe(standardReasoningEfforts()[0].id);
  });

  it("preserves selection across the one-time lowercase-id → alias migration", () => {
    const prefs = prefsWithAspect(["minimax-m2.7"], "minimax-m2.7");
    const next = reconcileAspectModels(prefs, "aspect", [model("MiniMax-M2.7")]);
    expect(next).not.toBeNull();
    expect(next!.selectedModelId).toBe("MiniMax-M2.7");
  });

  it("does not touch selection when a non-active provider refreshes", () => {
    const prefs = { ...prefsWithAspect(["glm-4.7"]), selectedProviderId: "other", selectedModelId: "keep-me" };
    const next = reconcileAspectModels(prefs, "aspect", [model("glm-4.7"), model("Kimi-K2.6")]);
    expect(next).not.toBeNull();
    expect(next!.selectedModelId).toBe("keep-me");
  });
});

describe("formatCompactTokens", () => {
  it("formats across units", () => {
    expect(formatCompactTokens(0)).toBe("0");
    expect(formatCompactTokens(999)).toBe("999");
    expect(formatCompactTokens(1_200)).toBe("1.2k");
    expect(formatCompactTokens(100_000)).toBe("100k");
    expect(formatCompactTokens(1_500_000)).toBe("1.5M");
    expect(formatCompactTokens(500_000_000)).toBe("500M");
    expect(formatCompactTokens(2_000_000_000)).toBe("2B");
  });

  it("rolls a round-up to the next unit instead of emitting 1000k / 1000M", () => {
    expect(formatCompactTokens(999_999_999)).toBe("1B");
    expect(formatCompactTokens(999_950)).toBe("1M");
  });
});

describe("formatAspectUsageLabel", () => {
  it("returns null when nothing to show", () => {
    expect(formatAspectUsageLabel(null)).toBeNull();
    expect(formatAspectUsageLabel({ total: 0, windows: [{ window: "day", used: 0, cap: 0 }] })).toBeNull();
  });

  it("shows all-time total when uncapped but spent", () => {
    expect(formatAspectUsageLabel({ total: 45_000_000, windows: [{ window: "day", used: 12_000, cap: 0 }] }))
      .toBe("Σ 45M");
  });

  it("shows each capped window in order plus the all-time total", () => {
    const label = formatAspectUsageLabel({
      total: 45_000_000,
      windows: [
        { window: "5h", used: 12_000, cap: 100_000 },
        { window: "day", used: 0, cap: 0 },
        { window: "week", used: 1_200_000, cap: 2_000_000 },
      ],
    });
    expect(label).toBe("5h 12k/100k · wk 1.2M/2M · Σ 45M");
  });
});
