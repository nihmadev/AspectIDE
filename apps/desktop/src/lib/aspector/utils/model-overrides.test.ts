import { describe, expect, it } from "vitest";
import {
  resolveEffectiveAutoCompactThreshold,
  resolveModelProtocol,
} from "./preferences";

describe("resolveModelProtocol", () => {
  it("prefers the model override, then the provider, then the default", () => {
    expect(resolveModelProtocol({ protocol: "openai-compatible" }, { protocol: "anthropic" })).toBe("anthropic");
    expect(resolveModelProtocol({ protocol: "google" }, { protocol: null })).toBe("google");
    expect(resolveModelProtocol({ protocol: "google" }, {})).toBe("google");
    expect(resolveModelProtocol(null, null)).toBe("openai-compatible");
  });
});

describe("resolveEffectiveAutoCompactThreshold", () => {
  it("resolves model → provider → global, treating null as inherit", () => {
    expect(resolveEffectiveAutoCompactThreshold(0.8, { contextAutoCompactThreshold: 0.6 }, { contextAutoCompactThreshold: 0.5 })).toBe(0.5);
    expect(resolveEffectiveAutoCompactThreshold(0.8, { contextAutoCompactThreshold: 0.6 }, { contextAutoCompactThreshold: null })).toBe(0.6);
    expect(resolveEffectiveAutoCompactThreshold(0.8, { contextAutoCompactThreshold: null }, {})).toBe(0.8);
    expect(resolveEffectiveAutoCompactThreshold(0.8, null, null)).toBe(0.8);
  });
});
