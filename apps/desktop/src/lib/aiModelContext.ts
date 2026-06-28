import type { AiModelConfig } from "./aiPreferences";

export const DEFAULT_MODEL_CONTEXT_TOKENS = 200_000;
export const DEFAULT_CONTEXT_AUTO_COMPACT_THRESHOLD = 0.8;
export const MIN_CONTEXT_AUTO_COMPACT_THRESHOLD = 0.5;
export const MAX_CONTEXT_AUTO_COMPACT_THRESHOLD = 0.95;
export const MIN_MODEL_CONTEXT_TOKENS = 8_000;
export const MAX_MODEL_CONTEXT_TOKENS = 2_000_000;

const MODEL_CONTEXT_HINTS: ReadonlyArray<{ pattern: RegExp; tokens: number }> = [
  { pattern: /gpt-5\.5|gpt-5-pro|gpt-5(?![\w-])/i, tokens: 400_000 },
  { pattern: /gpt-5-mini|gpt-5-nano/i, tokens: 400_000 },
  { pattern: /gpt-4\.1|gpt-4o|o3|o4/i, tokens: 128_000 },
  { pattern: /claude-opus-4|claude-sonnet-4|claude-3-7|claude-3-5/i, tokens: 200_000 },
  { pattern: /claude-3-5-haiku|claude-haiku/i, tokens: 200_000 },
  { pattern: /gemini-2\.5-pro|gemini-2\.5-flash/i, tokens: 1_048_576 },
  { pattern: /gemini-2\.0|gemini-1\.5-pro/i, tokens: 1_048_576 },
  { pattern: /gemini/i, tokens: 128_000 },
  { pattern: /deepseek/i, tokens: 128_000 },
  { pattern: /mistral-large|codestral/i, tokens: 128_000 },
  { pattern: /llama-3\.3|llama3/i, tokens: 128_000 },
  { pattern: /qwen/i, tokens: 128_000 },
];

export function clampContextAutoCompactThreshold(value: number) {
  if (!Number.isFinite(value)) return DEFAULT_CONTEXT_AUTO_COMPACT_THRESHOLD;
  return Math.min(MAX_CONTEXT_AUTO_COMPACT_THRESHOLD, Math.max(MIN_CONTEXT_AUTO_COMPACT_THRESHOLD, value));
}

export function clampModelContextTokens(value: number) {
  if (!Number.isFinite(value)) return DEFAULT_MODEL_CONTEXT_TOKENS;
  return Math.min(MAX_MODEL_CONTEXT_TOKENS, Math.max(MIN_MODEL_CONTEXT_TOKENS, Math.round(value)));
}

export function inferContextTokensFromModelRef(modelRef: string) {
  const haystack = modelRef.trim().toLowerCase();
  if (!haystack) return null;
  for (const hint of MODEL_CONTEXT_HINTS) {
    if (hint.pattern.test(haystack)) return hint.tokens;
  }
  return null;
}

function inferKnownModelContextTokens(model: AiModelConfig) {
  return inferContextTokensFromModelRef([
    model.alias,
    model.id,
    model.name,
  ].filter(Boolean).join(" "));
}

export function resolveModelContextTokens(model: AiModelConfig | null | undefined) {
  if (!model) return DEFAULT_MODEL_CONTEXT_TOKENS;

  const inferred = inferKnownModelContextTokens(model);
  const configured = typeof model.contextTokens === "number" && model.contextTokens > 0
    ? clampModelContextTokens(model.contextTokens)
    : null;

  // Provider discovery may return stale or rounded limits (e.g. 239K for a known
  // 400K model). Never let that make the UI claim "100% Full" while known model
  // metadata says there is still room. Keep larger configured values so custom
  // endpoints with extended context still work.
  if (inferred != null && configured != null) return Math.max(inferred, configured);
  return inferred ?? configured ?? DEFAULT_MODEL_CONTEXT_TOKENS;
}

export function resolveContextCompactTriggerTokens(model: AiModelConfig | null | undefined, threshold: number) {
  const budget = resolveModelContextTokens(model);
  const ratio = clampContextAutoCompactThreshold(threshold);
  return Math.floor(budget * ratio);
}
