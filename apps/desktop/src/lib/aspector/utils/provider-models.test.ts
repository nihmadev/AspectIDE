import { describe, expect, it } from "vitest";
import { isFreeModelId, mergeRefreshedModels, resolveProviderDiscoveryProfile } from "./provider-models";
import type { AiModelConfig, AiProviderConfig } from "./preferences";

describe("resolveProviderDiscoveryProfile", () => {
  it("only OpenCode Zen treats a -free suffix as a free model", () => {
    expect(resolveProviderDiscoveryProfile("opencode-zen").isFreeModel("deepseek-v4-flash-free")).toBe(true);
    expect(resolveProviderDiscoveryProfile("openai").isFreeModel("deepseek-v4-flash-free")).toBe(false);
    expect(resolveProviderDiscoveryProfile("openrouter").isFreeModel("anything-free")).toBe(false);
  });

  it("grants free OpenCode Zen models the 1M window, else infers by model family", () => {
    const zen = resolveProviderDiscoveryProfile("opencode-zen");
    expect(zen.contextTokensFor("mystery-model-free")).toBe(1_000_000);
    expect(zen.contextTokensFor("deepseek-chat")).toBe(128_000);
  });

  it("never coerces a vendor-neutral provider's -free id to a 1M window", () => {
    const openrouter = resolveProviderDiscoveryProfile("openrouter");
    // The free 1M override is OpenCode-only; here the deepseek family still wins.
    expect(openrouter.contextTokensFor("deepseek-chat-free")).toBe(128_000);
    // Unknown family => null (auto-detect downstream), NOT a bogus 1M window.
    expect(openrouter.contextTokensFor("acme-mystery-model")).toBeNull();
  });

  it("sorts free-first only for OpenCode Zen; vendor-neutral is alphabetical", () => {
    const ids = ["zeta-model", "alpha-free", "beta-model", "gamma-free"];
    const zen = [...ids].sort(resolveProviderDiscoveryProfile("opencode-zen").compareIds);
    const neutral = [...ids].sort(resolveProviderDiscoveryProfile("custom").compareIds);
    expect(zen).toEqual(["alpha-free", "gamma-free", "beta-model", "zeta-model"]);
    expect(neutral).toEqual(["alpha-free", "beta-model", "gamma-free", "zeta-model"]);
  });

  it("labels free OpenCode models, plain title-cases everywhere else", () => {
    expect(resolveProviderDiscoveryProfile("opencode-zen").humanize("deepseek-v4-flash-free")).toBe("Deepseek V4 Flash (Free)");
    expect(resolveProviderDiscoveryProfile("anthropic").humanize("claude-sonnet-4-5")).toBe("Claude Sonnet 4 5");
  });
});

describe("isFreeModelId", () => {
  it("matches the -free suffix case-insensitively", () => {
    expect(isFreeModelId("model-FREE")).toBe(true);
    expect(isFreeModelId("model-paid")).toBe(false);
  });
});

describe("mergeRefreshedModels", () => {
  const makeProvider = (models: AiModelConfig[]): AiProviderConfig => ({
    id: "p",
    name: "P",
    providerType: "custom",
    protocol: "openai-compatible",
    baseUrl: "",
    apiKey: "",
    localHost: "",
    localPort: "",
    localPath: "",
    models,
    embeddingModel: "",
  });

  it("keeps user manual prices but takes the freshly-fetched name/context", () => {
    const prior = makeProvider([
      { id: "m", name: "old", alias: "m", contextTokens: 999, inputPricePerMillion: 5, outputPricePerMillion: 7, effortLevels: [] },
    ]);
    const fetched: AiModelConfig[] = [{ id: "m", name: "new", alias: "m", contextTokens: 128_000, effortLevels: [] }];
    const merged = mergeRefreshedModels(prior, fetched);
    expect(merged.models[0].name).toBe("new");
    expect(merged.models[0].contextTokens).toBe(128_000);
    expect(merged.models[0].inputPricePerMillion).toBe(5);
    expect(merged.models[0].outputPricePerMillion).toBe(7);
  });

  it("returns the provider unchanged when the fetch produced no models", () => {
    const provider = makeProvider([{ id: "m", name: "n", alias: "m", effortLevels: [] }]);
    expect(mergeRefreshedModels(provider, [])).toBe(provider);
  });
});
