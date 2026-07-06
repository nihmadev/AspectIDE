import { describe, expect, it } from "vitest";
import {
  AI_PROVIDER_PRESETS,
  defaultAiModelId,
  defaultAiProviderId,
  defaultAiProviders,
  getAiProviderPreset,
  normalizeAiPreferences,
} from "./aiPreferences";

// The bundled LuxIDE managed provider must ship first + selected on a fresh
// install, exposing exactly the free gateway models with wire-correct aliases.
describe("LuxIDE managed provider preset", () => {
  const EXPECTED_ALIASES = ["glm-4.7", "MiniMax-M2.7", "Kimi-K2.6", "step-3.7-flash", "MiniMax-M3", "Spark-X2-Flash", "Qwen3.5-397B-A17B"];

  it("is the first entry in the add-provider template list", () => {
    expect(AI_PROVIDER_PRESETS[0].id).toBe("luxide");
  });

  it("is the default selected provider + model", () => {
    expect(defaultAiProviderId).toBe("luxide");
    expect(defaultAiModelId).toBe("glm-4.7");
  });

  it("ships one keyless provider with the gateway base url", () => {
    expect(defaultAiProviders).toHaveLength(1);
    const provider = defaultAiProviders[0];
    expect(provider.providerType).toBe("luxide");
    expect(provider.protocol).toBe("openai-compatible");
    expect(provider.baseUrl).toBe("https://lux-ide.duckdns.org/v1");
    expect(provider.apiKey).toBe(""); // keyless — token is fetched via enrollment
  });

  it("exposes exactly the free models with wire-correct aliases", () => {
    const preset = getAiProviderPreset("luxide");
    expect(preset).toBeTruthy();
    expect(preset!.models.map((m) => m.alias)).toEqual(EXPECTED_ALIASES);
    expect(defaultAiProviders[0].models.map((m) => m.alias)).toEqual(EXPECTED_ALIASES);
  });

  it("survives normalization on a fresh install (empty persisted prefs)", () => {
    const prefs = normalizeAiPreferences({}, { preserveText: true });
    expect(prefs.providers[0].providerType).toBe("luxide");
    expect(prefs.selectedProviderId).toBe("luxide");
    expect(prefs.providers[0].models.map((m) => m.alias)).toEqual(EXPECTED_ALIASES);
  });
});
