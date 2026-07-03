import { automaticAgentProfileInstructions } from "./aiAutomaticModeInstructions";
import {
  clampContextAutoCompactThreshold,
  clampModelContextTokens,
  DEFAULT_CONTEXT_AUTO_COMPACT_THRESHOLD,
  inferContextTokensFromModelRef,
} from "./aiModelContext";
import { defaultMaxParallelSubagents, maxParallelSubagentsMax, maxParallelSubagentsMin } from "./aiSubagentPolicy";

export const AI_PREFERENCES_KEY = "ai.preferences";

export const aiToolRoundLimitMin = 1;
export const aiToolRoundLimitMax = 200;
export const defaultLimitedAiToolRoundLimit = 100;
export const defaultAiToolRoundLimit: AiToolRoundLimit = null;

export type AiToolRoundLimit = number | null;

export type AiPreferences = {
  projectIndexingEnabled: boolean;
  realtimeIndexing: boolean;
  includeImages: boolean;
  /**
   * Encoding for vision images sent to the model. `auto` picks lossless WebP for
   * provider families known to decode it (Anthropic, OpenAI, Gemini, xAI,
   * OpenRouter) and PNG everywhere else; `webp`/`png` force the choice. Lossless
   * WebP shrinks the payload and persisted history without any pixel loss; PNG is
   * the universal fallback for models that cannot read WebP.
   */
  visionImageFormat: AiVisionImageFormatPreference;
  /**
   * CPU budget for native filesystem scans and content search. `auto` reserves
   * one logical core for the UI (`cores - 1`); `all` uses every core; `half` uses
   * half. Applied to the Rust scan/search worker pools so large operations don't
   * starve the main thread / WebView.
   */
  scanConcurrency: AiScanConcurrency;
  maxIndexedFiles: number;
  /** When true, chat history is compacted before send once estimated usage crosses the threshold. */
  contextAutoCompactEnabled: boolean;
  /** Fraction of the active model context window that triggers auto-compaction (0.5–0.95). */
  contextAutoCompactThreshold: number;
  toolRoundLimit: AiToolRoundLimit;
  /** Max concurrent Task subagents per chat session (agent-managed). */
  maxParallelSubagents: number;
  /** Goal-run token budget (null = default 200k for normal, 80k for exploratory). */
  goalRunMaxTokens: number | null;
  /** Goal-run max rounds (null = default 32, exploratory = min(maxRounds,6)). */
  goalRunMaxRounds: number | null;
  /** Automatic mode hard stop in minutes (null = unlimited). */
  automaticModeHardStopMinutes: number | null;
  /** Composite "providerId + modelId" keys hidden from the composer model picker. */
  hiddenModelIds: string[];
  /** User has seen the Telegram community welcome banner. */
  seenTelegramNotice: boolean;
  showResponseDuration: boolean;
  /**
   * Token-economy ("caveman") mode. When on, a terse-output directive is appended
   * to the system prompt: the model drops filler/pleasantries/hedging and answers
   * compactly while keeping all technical substance, code, paths, and tool work
   * intact. Trims OUTPUT tokens, not reasoning depth or correctness.
   */
  tokenEconomyEnabled: boolean;
  /**
   * When on, Lux's built-in behavioral core prompt is replaced by `customSystemPrompt`.
   * The runtime context, tool map, and a minimal safety floor are still appended so the
   * agent and tools keep working — only the behavioral body is swapped. Off → built-in core.
   */
  /** Auto-install a missing language server in the background when its language is opened. */
  lspAutoInstall: boolean;
  /** Auto-provision managed runtimes (Node baseline at startup; Rust/Python/Go on demand). */
  runtimeAutoProvision: boolean;
  customSystemPromptEnabled: boolean;
  /** User-authored system prompt body, used only when `customSystemPromptEnabled` is on. */
  customSystemPrompt: string;
  /**
   * Declarative tool permission rules, one per entry, format `[allow|deny|ask:]Tool(glob)`.
   * Examples: `allow:Bash(git *)`, `deny:Write(*.env)`, `ask:Bash(rm *)`. Evaluated in the
   * Rust permission engine before the approval prompt (deny > ask > allow).
   */
  toolPermissionRules: string[];
  globalInstructions: string;
  projectInstructionsByWorkspace: Record<string, string>;
  agentMode: AiAgentMode;
  selectedAgentId: string;
  agentProfiles: AiAgentProfile[];
  selectedProviderId: string;
  selectedModelId: string;
  selectedEffortId: string;
  toolApprovalMode: AiToolApprovalMode;
  fileEditTrustMode: AiFileEditTrustMode;
  providers: AiProviderConfig[];
  voiceInputEnabled: boolean;
  voiceInputProvider: AiVoiceInputProvider;
  voiceInputLanguage: AiVoiceInputLanguage;
  localSttCommand: string;
  localSttModelPath: string;
  agentBrowserEnabled: boolean;
  agentBrowserCommand: string;
  agentBrowserHeaded: boolean;
  agentBrowserAllowedDomains: string;
  agentBrowserMaxOutput: number;
  agentBrowserPersistSession: boolean;
  agentBrowserProfile: string;
  agentBrowserStatePath: string;
  agentBrowserContentBoundaries: boolean;
  agentBrowserIgnoreHttpsErrors: boolean;
  agentBrowserAutoStreamPreview: boolean;
  agentBrowserDashboardPort: number;
  agentBrowserAllowFileAccess: boolean;
  /** Cloud/local engine provider: chrome, browserless, browserbase, kernel, agentcore, ios, … */
  agentBrowserProvider: string;
  agentBrowserProxy: string;
};

export type AiAgentMode = "agent" | "automatic" | "plan" | "ask";

/** Display and cycle order for agent mode selectors (composer, settings, slash /agent). */
export const AI_AGENT_MODE_ORDER: readonly AiAgentMode[] = ["automatic", "agent", "plan", "ask"];

export function isFullExecutionAgentMode(mode: AiAgentMode | string | undefined): mode is "agent" | "automatic" {
  return mode === "agent" || mode === "automatic";
}

export function isReadOnlyAgentMode(mode: AiAgentMode | string | undefined): boolean {
  return mode === "plan" || mode === "ask";
}
export type AiToolApprovalMode = "default" | "full-access";
/** apply-immediately: writes go to disk at once; preview-before-apply: in-editor preview until user accepts. */
export type AiFileEditTrustMode = "apply-immediately" | "preview-before-apply";
export type AiVoiceInputLanguage = "auto" | "ru-RU" | "en-US";
export type AiVoiceInputProvider = "native-webview" | "local";
/** Vision image encoding preference. `auto` = capability-based (WebP where safe, else PNG). */
export type AiVisionImageFormatPreference = "auto" | "webp" | "png";
/** CPU budget for native FS scans/search. `auto` reserves one core for the UI. */
export type AiScanConcurrency = "auto" | "all" | "half";

export type AiProviderPresetId =
  | "openai"
  | "anthropic"
  | "openrouter"
  | "opencode-zen"
  | "google"
  | "mistral"
  | "groq"
  | "cohere"
  | "deepseek"
  | "xai"
  | "azure-openai"
  | "together"
  | "fireworks"
  | "cerebras"
  | "moonshot"
  | "zai"
  | "minimax"
  | "alibaba"
  | "huggingface"
  | "github-models"
  | "github-copilot"
  | "vercel-gateway"
  | "nvidia"
  | "deepinfra"
  | "novita"
  | "perplexity"
  | "siliconflow"
  | "nebius"
  | "baseten"
  | "venice"
  | "cloudflare-workers-ai"
  | "meta-llama"
  | "ollama-cloud"
  | "ollama"
  | "lm-studio"
  | "local-proxy"
  | "custom";

export type AiProviderProtocol = "openai-compatible" | "anthropic" | "google" | "azure-openai" | "local-proxy";

export type AiAgentProfile = {
  id: string;
  name: string;
  mode: AiAgentMode;
  instructions: string;
};

export type AiProviderConfig = {
  id: string;
  name: string;
  providerType: AiProviderPresetId;
  protocol: AiProviderProtocol;
  baseUrl: string;
  apiKey: string;
  localHost: string;
  localPort: string;
  localPath: string;
  models: AiModelConfig[];
  /** Optional embeddings model id for this provider's `/embeddings` endpoint
   *  (semantic memory search). Empty when unset — best-effort, never blocks a
   *  turn, and is never used for `anthropic`-protocol providers (no endpoint). */
  embeddingModel: string;
};

export type AiModelConfig = {
  id: string;
  name: string;
  alias: string;
  /** Max context tokens for this model. Omit or 0 to auto-detect from alias/id. */
  contextTokens?: number | null;
  /** Manual price (USD per 1M input tokens) for cost estimation. Null/0 → fall back to alias-based rates. */
  inputPricePerMillion?: number | null;
  /** Manual price (USD per 1M output tokens) for cost estimation. Null/0 → fall back to alias-based rates. */
  outputPricePerMillion?: number | null;
  effortLevels: AiEffortConfig[];
};

export type AiEffortConfig = {
  id: string;
  label: string;
};

export type AiProviderPreset = {
  id: AiProviderPresetId;
  name: string;
  description: string;
  protocol: AiProviderProtocol;
  baseUrl: string;
  localHost?: string;
  localPort?: string;
  localPath?: string;
  models: readonly AiModelTemplate[];
};

type AiModelTemplate = {
  id: string;
  name: string;
  alias: string;
  contextTokens?: number | null;
  effortLevels?: readonly AiEffortConfig[];
};

const reasoningEfforts = [
  { id: "minimal", label: "Minimal" },
  { id: "low", label: "Low" },
  { id: "medium", label: "Medium" },
  { id: "high", label: "High" },
  { id: "xhigh", label: "xHigh" },
] as const satisfies readonly AiEffortConfig[];

/** Standard reasoning-effort levels, exported for dynamically-built model configs. */
export function standardReasoningEfforts(): AiEffortConfig[] {
  return reasoningEfforts.map((effort) => ({ ...effort }));
}

export const AI_PROVIDER_PRESETS = [
  {
    id: "openai",
    name: "OpenAI",
    description: "Official OpenAI API endpoint.",
    protocol: "openai-compatible",
    baseUrl: "https://api.openai.com/v1",
    models: [
      { id: "gpt-5", name: "GPT-5", alias: "gpt-5", effortLevels: reasoningEfforts },
      { id: "gpt-5-mini", name: "GPT-5 Mini", alias: "gpt-5-mini", effortLevels: reasoningEfforts },
      { id: "gpt-5-nano", name: "GPT-5 Nano", alias: "gpt-5-nano", effortLevels: reasoningEfforts },
      { id: "gpt-4.1", name: "GPT-4.1", alias: "gpt-4.1" },
    ],
  },
  {
    id: "anthropic",
    name: "Anthropic",
    description: "Claude models through Anthropic Messages API.",
    protocol: "anthropic",
    baseUrl: "https://api.anthropic.com/v1",
    models: [
      { id: "claude-sonnet-4-5", name: "Claude Sonnet 4.5", alias: "claude-sonnet-4-5" },
      { id: "claude-opus-4-1", name: "Claude Opus 4.1", alias: "claude-opus-4-1" },
      { id: "claude-3-5-haiku-latest", name: "Claude 3.5 Haiku", alias: "claude-3-5-haiku-latest" },
    ],
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    description: "OpenAI-compatible routing for many hosted models.",
    protocol: "openai-compatible",
    baseUrl: "https://openrouter.ai/api/v1",
    models: [
      { id: "openrouter-gpt-5", name: "OpenAI GPT-5", alias: "openai/gpt-5", effortLevels: reasoningEfforts },
      { id: "openrouter-claude-sonnet", name: "Claude Sonnet", alias: "anthropic/claude-sonnet-4.5" },
      { id: "openrouter-gemini-pro", name: "Gemini Pro", alias: "google/gemini-2.5-pro" },
      { id: "openrouter-deepseek-chat", name: "DeepSeek Chat", alias: "deepseek/deepseek-chat" },
    ],
  },
  {
    id: "opencode-zen",
    name: "OpenCode Zen",
    description: "Curated coding models from the OpenCode team. The full catalog is fetched live from /models (free “–free” models listed first) and refreshes automatically; nothing here is hardcoded.",
    protocol: "openai-compatible",
    baseUrl: "https://opencode.ai/zen/v1",
    // Minimal offline bootstrap so the preset is structurally valid before the first
    // live fetch (the normalizer requires models[0]) AND usable offline. The real,
    // full catalog is pulled from GET /models (free-first) on activation/refresh —
    // see aiProviderModels.ts. This single seed is a real free model id, so the
    // provider works even if the live fetch is unavailable; it is not a fake entry.
    models: [
      { id: "opencode-zen-auto", name: "DeepSeek V4 Flash (Free)", alias: "deepseek-v4-flash-free", contextTokens: 1_000_000, effortLevels: reasoningEfforts },
    ],
  },
  {
    id: "google",
    name: "Google Gemini",
    description: "Gemini API using Google's OpenAI-compatible endpoint.",
    protocol: "google",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    models: [
      { id: "gemini-2.5-pro", name: "Gemini 2.5 Pro", alias: "gemini-2.5-pro" },
      { id: "gemini-2.5-flash", name: "Gemini 2.5 Flash", alias: "gemini-2.5-flash" },
      { id: "gemini-2.0-flash", name: "Gemini 2.0 Flash", alias: "gemini-2.0-flash" },
    ],
  },
  {
    id: "mistral",
    name: "Mistral AI",
    description: "Mistral hosted API.",
    protocol: "openai-compatible",
    baseUrl: "https://api.mistral.ai/v1",
    models: [
      { id: "mistral-large-latest", name: "Mistral Large", alias: "mistral-large-latest" },
      { id: "codestral-latest", name: "Codestral", alias: "codestral-latest" },
      { id: "ministral-8b-latest", name: "Ministral 8B", alias: "ministral-8b-latest" },
    ],
  },
  {
    id: "groq",
    name: "Groq",
    description: "Groq OpenAI-compatible inference endpoint.",
    protocol: "openai-compatible",
    baseUrl: "https://api.groq.com/openai/v1",
    models: [
      { id: "llama-3.3-70b-versatile", name: "Llama 3.3 70B", alias: "llama-3.3-70b-versatile" },
      { id: "kimi-k2-instruct", name: "Kimi K2", alias: "moonshotai/kimi-k2-instruct" },
      { id: "gpt-oss-120b", name: "GPT OSS 120B", alias: "openai/gpt-oss-120b" },
    ],
  },
  {
    id: "cohere",
    name: "Cohere",
    description: "Cohere compatibility API.",
    protocol: "openai-compatible",
    baseUrl: "https://api.cohere.com/compatibility/v1",
    models: [
      { id: "command-a-03-2025", name: "Command A", alias: "command-a-03-2025" },
      { id: "command-r-plus", name: "Command R+", alias: "command-r-plus" },
      { id: "command-r", name: "Command R", alias: "command-r" },
    ],
  },
  {
    id: "deepseek",
    name: "DeepSeek",
    description: "DeepSeek chat and reasoning models.",
    protocol: "openai-compatible",
    baseUrl: "https://api.deepseek.com/v1",
    models: [
      { id: "deepseek-chat", name: "DeepSeek Chat", alias: "deepseek-chat" },
      { id: "deepseek-reasoner", name: "DeepSeek Reasoner", alias: "deepseek-reasoner", effortLevels: reasoningEfforts },
    ],
  },
  {
    id: "xai",
    name: "xAI",
    description: "xAI Grok API.",
    protocol: "openai-compatible",
    baseUrl: "https://api.x.ai/v1",
    models: [
      { id: "grok-4", name: "Grok 4", alias: "grok-4" },
      { id: "grok-3", name: "Grok 3", alias: "grok-3" },
      { id: "grok-3-mini", name: "Grok 3 Mini", alias: "grok-3-mini" },
    ],
  },
  {
    id: "azure-openai",
    name: "Azure OpenAI",
    description: "Azure deployment endpoint; replace resource and deployment names.",
    protocol: "azure-openai",
    baseUrl: "https://YOUR-RESOURCE.openai.azure.com/openai/deployments/YOUR-DEPLOYMENT",
    models: [
      { id: "azure-gpt-5", name: "GPT-5 deployment", alias: "YOUR-DEPLOYMENT", effortLevels: reasoningEfforts },
      { id: "azure-gpt-4.1", name: "GPT-4.1 deployment", alias: "YOUR-DEPLOYMENT" },
    ],
  },
  {
    id: "ollama",
    name: "Ollama",
    description: "Local Ollama OpenAI-compatible server.",
    protocol: "local-proxy",
    baseUrl: "http://127.0.0.1:11434/v1",
    localHost: "127.0.0.1",
    localPort: "11434",
    localPath: "/v1",
    models: [
      { id: "llama3.1", name: "Llama 3.1", alias: "llama3.1" },
      { id: "qwen2.5-coder", name: "Qwen 2.5 Coder", alias: "qwen2.5-coder" },
      { id: "mistral", name: "Mistral", alias: "mistral" },
    ],
  },
  {
    id: "together",
    name: "Together AI",
    description: "Together AI serverless inference (OpenAI-compatible).",
    protocol: "openai-compatible",
    baseUrl: "https://api.together.xyz/v1",
    models: [
      { id: "together-kimi-k2", name: "Kimi K2 Instruct", alias: "moonshotai/Kimi-K2-Instruct" },
      { id: "together-qwen3-coder", name: "Qwen3 Coder 480B", alias: "Qwen/Qwen3-Coder-480B-A35B-Instruct-FP8" },
      { id: "together-deepseek-v3", name: "DeepSeek V3.1", alias: "deepseek-ai/DeepSeek-V3.1" },
    ],
  },
  {
    id: "fireworks",
    name: "Fireworks AI",
    description: "Fireworks AI fast open-model inference.",
    protocol: "openai-compatible",
    baseUrl: "https://api.fireworks.ai/inference/v1",
    models: [
      { id: "fireworks-kimi-k2", name: "Kimi K2 Instruct", alias: "accounts/fireworks/models/kimi-k2-instruct" },
      { id: "fireworks-deepseek-v3", name: "DeepSeek V3.1", alias: "accounts/fireworks/models/deepseek-v3p1" },
      { id: "fireworks-qwen3-coder", name: "Qwen3 Coder 480B", alias: "accounts/fireworks/models/qwen3-coder-480b-a35b-instruct" },
    ],
  },
  {
    id: "cerebras",
    name: "Cerebras",
    description: "Cerebras ultra-fast wafer-scale inference.",
    protocol: "openai-compatible",
    baseUrl: "https://api.cerebras.ai/v1",
    models: [
      { id: "cerebras-qwen3-coder", name: "Qwen3 Coder 480B", alias: "qwen-3-coder-480b" },
      { id: "cerebras-gpt-oss", name: "GPT OSS 120B", alias: "gpt-oss-120b" },
      { id: "cerebras-llama33", name: "Llama 3.3 70B", alias: "llama-3.3-70b" },
    ],
  },
  {
    id: "moonshot",
    name: "Moonshot AI (Kimi)",
    description: "Moonshot AI Kimi models (global endpoint).",
    protocol: "openai-compatible",
    baseUrl: "https://api.moonshot.ai/v1",
    models: [
      { id: "kimi-k2-0905", name: "Kimi K2", alias: "kimi-k2-0905-preview" },
      { id: "kimi-k2-turbo", name: "Kimi K2 Turbo", alias: "kimi-k2-turbo-preview" },
      { id: "kimi-latest", name: "Kimi Latest", alias: "kimi-latest" },
    ],
  },
  {
    id: "zai",
    name: "Z.AI (GLM)",
    description: "Z.AI GLM models (Zhipu global endpoint).",
    protocol: "openai-compatible",
    baseUrl: "https://api.z.ai/api/paas/v4",
    models: [
      { id: "glm-4.6", name: "GLM-4.6", alias: "glm-4.6", effortLevels: reasoningEfforts },
      { id: "glm-4.5", name: "GLM-4.5", alias: "glm-4.5" },
      { id: "glm-4.5-air", name: "GLM-4.5 Air", alias: "glm-4.5-air" },
    ],
  },
  {
    id: "minimax",
    name: "MiniMax",
    description: "MiniMax M2 through the Anthropic-compatible endpoint (matches opencode).",
    protocol: "anthropic",
    baseUrl: "https://api.minimax.io/anthropic/v1",
    models: [
      { id: "minimax-m2", name: "MiniMax M2", alias: "MiniMax-M2" },
      { id: "minimax-m1", name: "MiniMax M1", alias: "MiniMax-M1" },
    ],
  },
  {
    id: "alibaba",
    name: "Alibaba Qwen",
    description: "Alibaba Cloud DashScope international endpoint (Qwen models).",
    protocol: "openai-compatible",
    baseUrl: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    models: [
      { id: "qwen3-coder-plus", name: "Qwen3 Coder Plus", alias: "qwen3-coder-plus" },
      { id: "qwen3-max", name: "Qwen3 Max", alias: "qwen3-max" },
      { id: "qwen-plus", name: "Qwen Plus", alias: "qwen-plus" },
    ],
  },
  {
    id: "huggingface",
    name: "Hugging Face",
    description: "Hugging Face Inference Providers router.",
    protocol: "openai-compatible",
    baseUrl: "https://router.huggingface.co/v1",
    models: [
      { id: "hf-kimi-k2", name: "Kimi K2 Instruct", alias: "moonshotai/Kimi-K2-Instruct" },
      { id: "hf-deepseek-v3", name: "DeepSeek V3.1", alias: "deepseek-ai/DeepSeek-V3.1" },
      { id: "hf-qwen3-coder", name: "Qwen3 Coder 480B", alias: "Qwen/Qwen3-Coder-480B-A35B-Instruct" },
    ],
  },
  {
    id: "github-models",
    name: "GitHub Models",
    description: "GitHub Models inference endpoint (use a GitHub PAT as the API key).",
    protocol: "openai-compatible",
    baseUrl: "https://models.github.ai/inference",
    models: [
      { id: "gh-gpt-5", name: "GPT-5", alias: "openai/gpt-5", effortLevels: reasoningEfforts },
      { id: "gh-gpt-4.1", name: "GPT-4.1", alias: "openai/gpt-4.1" },
      { id: "gh-deepseek-v3", name: "DeepSeek V3", alias: "deepseek/DeepSeek-V3-0324" },
    ],
  },
  {
    id: "github-copilot",
    name: "GitHub Copilot",
    description: "GitHub Copilot API (requires a Copilot bearer token as the API key).",
    protocol: "openai-compatible",
    baseUrl: "https://api.githubcopilot.com",
    models: [
      { id: "copilot-gpt-5", name: "GPT-5", alias: "gpt-5", effortLevels: reasoningEfforts },
      { id: "copilot-claude-sonnet", name: "Claude Sonnet 4.5", alias: "claude-sonnet-4.5" },
    ],
  },
  {
    id: "vercel-gateway",
    name: "Vercel AI Gateway",
    description: "Vercel AI Gateway — one key for hundreds of routed models.",
    protocol: "openai-compatible",
    baseUrl: "https://ai-gateway.vercel.sh/v1",
    models: [
      { id: "vercel-claude-sonnet", name: "Claude Sonnet 4.5", alias: "anthropic/claude-sonnet-4.5" },
      { id: "vercel-gpt-5", name: "GPT-5", alias: "openai/gpt-5", effortLevels: reasoningEfforts },
      { id: "vercel-gemini-pro", name: "Gemini 2.5 Pro", alias: "google/gemini-2.5-pro" },
    ],
  },
  {
    id: "nvidia",
    name: "NVIDIA NIM",
    description: "NVIDIA NIM hosted inference (integrate.api.nvidia.com).",
    protocol: "openai-compatible",
    baseUrl: "https://integrate.api.nvidia.com/v1",
    models: [
      { id: "nvidia-deepseek-v3", name: "DeepSeek V3.1", alias: "deepseek-ai/deepseek-v3.1" },
      { id: "nvidia-kimi-k2", name: "Kimi K2 Instruct", alias: "moonshotai/kimi-k2-instruct" },
      { id: "nvidia-llama4", name: "Llama 4 Maverick", alias: "meta/llama-4-maverick-17b-128e-instruct" },
    ],
  },
  {
    id: "deepinfra",
    name: "DeepInfra",
    description: "DeepInfra pay-per-token open-model hosting.",
    protocol: "openai-compatible",
    baseUrl: "https://api.deepinfra.com/v1/openai",
    models: [
      { id: "deepinfra-deepseek-v3", name: "DeepSeek V3.1", alias: "deepseek-ai/DeepSeek-V3.1" },
      { id: "deepinfra-kimi-k2", name: "Kimi K2 Instruct", alias: "moonshotai/Kimi-K2-Instruct" },
      { id: "deepinfra-qwen3-coder", name: "Qwen3 Coder 480B", alias: "Qwen/Qwen3-Coder-480B-A35B-Instruct" },
    ],
  },
  {
    id: "novita",
    name: "Novita AI",
    description: "Novita AI open-model inference.",
    protocol: "openai-compatible",
    baseUrl: "https://api.novita.ai/openai",
    models: [
      { id: "novita-deepseek-v3", name: "DeepSeek V3.1", alias: "deepseek/deepseek-v3.1" },
      { id: "novita-qwen3-coder", name: "Qwen3 Coder 480B", alias: "qwen/qwen3-coder-480b-a35b-instruct" },
      { id: "novita-glm-4.6", name: "GLM-4.6", alias: "zai-org/glm-4.6" },
    ],
  },
  {
    id: "perplexity",
    name: "Perplexity",
    description: "Perplexity Sonar models with built-in web search.",
    protocol: "openai-compatible",
    baseUrl: "https://api.perplexity.ai",
    models: [
      { id: "sonar-pro", name: "Sonar Pro", alias: "sonar-pro" },
      { id: "sonar", name: "Sonar", alias: "sonar" },
      { id: "sonar-reasoning-pro", name: "Sonar Reasoning Pro", alias: "sonar-reasoning-pro", effortLevels: reasoningEfforts },
    ],
  },
  {
    id: "siliconflow",
    name: "SiliconFlow",
    description: "SiliconFlow international open-model endpoint.",
    protocol: "openai-compatible",
    baseUrl: "https://api.siliconflow.com/v1",
    models: [
      { id: "sf-deepseek-v3", name: "DeepSeek V3.1", alias: "deepseek-ai/DeepSeek-V3.1" },
      { id: "sf-kimi-k2", name: "Kimi K2 Instruct", alias: "moonshotai/Kimi-K2-Instruct" },
      { id: "sf-qwen3-coder", name: "Qwen3 Coder 480B", alias: "Qwen/Qwen3-Coder-480B-A35B-Instruct" },
    ],
  },
  {
    id: "nebius",
    name: "Nebius Token Factory",
    description: "Nebius Token Factory (studio) inference.",
    protocol: "openai-compatible",
    baseUrl: "https://api.tokenfactory.nebius.com/v1",
    models: [
      { id: "nebius-deepseek-v3", name: "DeepSeek V3.1", alias: "deepseek-ai/DeepSeek-V3.1" },
      { id: "nebius-qwen3-coder", name: "Qwen3 Coder 480B", alias: "Qwen/Qwen3-Coder-480B-A35B-Instruct" },
      { id: "nebius-gpt-oss", name: "GPT OSS 120B", alias: "openai/gpt-oss-120b" },
    ],
  },
  {
    id: "baseten",
    name: "Baseten",
    description: "Baseten model APIs (OpenAI-compatible).",
    protocol: "openai-compatible",
    baseUrl: "https://inference.baseten.co/v1",
    models: [
      { id: "baseten-deepseek-v3", name: "DeepSeek V3.1", alias: "deepseek-ai/DeepSeek-V3.1" },
      { id: "baseten-kimi-k2", name: "Kimi K2 Instruct", alias: "moonshotai/Kimi-K2-Instruct" },
    ],
  },
  {
    id: "venice",
    name: "Venice AI",
    description: "Venice AI privacy-first inference.",
    protocol: "openai-compatible",
    baseUrl: "https://api.venice.ai/api/v1",
    models: [
      { id: "venice-large", name: "Venice Large (Qwen3 235B)", alias: "qwen3-235b" },
      { id: "venice-llama33", name: "Llama 3.3 70B", alias: "llama-3.3-70b" },
    ],
  },
  {
    id: "cloudflare-workers-ai",
    name: "Cloudflare Workers AI",
    description: "Cloudflare Workers AI; replace the account id in the URL.",
    protocol: "openai-compatible",
    baseUrl: "https://api.cloudflare.com/client/v4/accounts/YOUR-ACCOUNT-ID/ai/v1",
    models: [
      { id: "cf-llama33", name: "Llama 3.3 70B Fast", alias: "@cf/meta/llama-3.3-70b-instruct-fp8-fast" },
      { id: "cf-qwen-coder", name: "Qwen 2.5 Coder 32B", alias: "@cf/qwen/qwen2.5-coder-32b-instruct" },
    ],
  },
  {
    id: "meta-llama",
    name: "Meta Llama API",
    description: "Meta's official Llama API (OpenAI-compatible endpoint).",
    protocol: "openai-compatible",
    baseUrl: "https://api.llama.com/compat/v1",
    models: [
      { id: "llama4-maverick", name: "Llama 4 Maverick", alias: "Llama-4-Maverick-17B-128E-Instruct-FP8" },
      { id: "llama4-scout", name: "Llama 4 Scout", alias: "Llama-4-Scout-17B-16E-Instruct-FP8" },
      { id: "llama33-70b", name: "Llama 3.3 70B", alias: "Llama-3.3-70B-Instruct" },
    ],
  },
  {
    id: "ollama-cloud",
    name: "Ollama Cloud",
    description: "Ollama's hosted cloud models (ollama.com API key).",
    protocol: "openai-compatible",
    baseUrl: "https://ollama.com/v1",
    models: [
      { id: "ollama-cloud-qwen3-coder", name: "Qwen3 Coder 480B", alias: "qwen3-coder:480b" },
      { id: "ollama-cloud-gpt-oss", name: "GPT OSS 120B", alias: "gpt-oss:120b" },
      { id: "ollama-cloud-deepseek-v3", name: "DeepSeek V3.1 671B", alias: "deepseek-v3.1:671b" },
    ],
  },
  {
    id: "lm-studio",
    name: "LM Studio",
    description: "Local LM Studio server.",
    protocol: "local-proxy",
    baseUrl: "http://127.0.0.1:1234/v1",
    localHost: "127.0.0.1",
    localPort: "1234",
    localPath: "/v1",
    models: [
      { id: "local-model", name: "Local model", alias: "local-model" },
    ],
  },
  {
    id: "local-proxy",
    name: "Local",
    description: "Custom local endpoint by IP, port, and path.",
    protocol: "local-proxy",
    baseUrl: "http://127.0.0.1:8799/v1",
    localHost: "127.0.0.1",
    localPort: "8799",
    localPath: "/v1",
    models: [
      { id: "gpt-5.5", name: "GPT 5.5", alias: "gpt-5.5", effortLevels: reasoningEfforts },
      { id: "gpt-5.4", name: "GPT 5.4", alias: "gpt-5.4", effortLevels: reasoningEfforts },
      { id: "gpt-4.8", name: "GPT 4.8", alias: "gpt-4.8", effortLevels: reasoningEfforts },
      { id: "gpt-4.7", name: "GPT 4.7", alias: "gpt-4.7", effortLevels: reasoningEfforts },
      { id: "gpt-4.6", name: "GPT 4.6", alias: "gpt-4.6", effortLevels: reasoningEfforts },
    ],
  },
  {
    id: "custom",
    name: "Custom provider",
    description: "Any OpenAI-compatible endpoint.",
    protocol: "openai-compatible",
    baseUrl: "https://api.example.com/v1",
    models: [
      { id: "custom-model", name: "Custom model", alias: "model-name" },
    ],
  },
] as const satisfies readonly AiProviderPreset[];

export const defaultAiProviderId = "local-proxy";
export const defaultAiModelId = "gpt-5.5";
export const defaultAiEffortId = "xhigh";

export const defaultAiAgentProfiles: AiAgentProfile[] = [
  { id: "automatic", name: "Automatic", mode: "automatic", instructions: automaticAgentProfileInstructions },
  { id: "agent", name: "Agent", mode: "agent", instructions: [
    "Drive the task end to end inside the current workspace: inspect evidence first, make the needed scoped edits, then verify with the narrowest meaningful checks before reporting completion.",
    "Preserve unrelated user work, dirty files, and existing architecture. Prefer existing project patterns, typed APIs, focused modules, and small reversible changes over broad rewrites.",
    "Use tools whenever the answer depends on files, diagnostics, commands, browser state, docs, or current workspace facts. Batch independent read-only context, then act sequentially where results matter.",
    "When a genuine decision can't be settled from evidence (product/UX, ambiguous scope, credentials), ask with AskUser — supply suggested options, and an htmlPreview HTML5 mockup for visual choices — rather than guessing. For multi-step work worth confirming first, propose it with PresentPlan.",
    "When changing code, keep behavior production-ready: handle errors explicitly, avoid silent fallbacks, avoid placeholder implementations, and surface real residual risk if verification cannot cover it.",
    "Final reports should be concise: what changed, what was verified, and what remains only if something genuinely remains.",
  ].join("\n") },
  { id: "plan", name: "Plan", mode: "plan", instructions: [
    "Stay read-only unless the user explicitly approves implementation. Gather only enough context to understand the task, constraints, affected files, and verification surface.",
    "Deliver the plan via the PresentPlan tool, not a prose checklist: give each step a clear title plus optional detail and the primary file it touches. PresentPlan pins the goal and task list and lets the user press Start to hand execution to Agent mode.",
    "Each step must be concrete and independently verifiable: a specific action on a named file/module with its acceptance check — never a generic phase ('set up the project', 'implement business logic', 'add documentation'). Decompose to the real edits, order them by dependency, and call out the riskiest step. If the whole plan is 3–4 phase-level bullets, it is too coarse — break it down.",
    "A complete plan covers, scaled to the task's risk: decomposition into file-level steps; the key decision/alternative where one genuinely exists (with the chosen path's rationale); the main failure mode of the riskiest step; and a final explicit verification step — the tests/build/checks that prove it works, plus a rollback trigger for risky changes. Higher-risk work (auth, payments, migrations, concurrency, data-loss, public APIs) earns more steps and explicit verification; trivial work stays terse.",
    "Front-load assumptions, ordering, risk points, and validation in the step details. When a real decision needs the user (product/UX choice, ambiguous scope), use AskUser with suggested options — and an htmlPreview HTML5 mockup when the choice is visual — instead of guessing silently.",
    "Prefer architecture-preserving plans: separate domain, runtime, infrastructure, and UI concerns; avoid hidden coupling, silent fallback behavior, and cosmetic extraction.",
    "Keep the plan compact and actionable so it can be executed without another discovery pass unless the user changes scope.",
  ].join("\n") },
  { id: "ask", name: "Ask", mode: "ask", instructions: [
    "Answer directly from the available evidence. Use read-only workspace context when it materially improves correctness, but do not edit files or run mutating commands unless asked.",
    "Explain code and tradeoffs with exact file, symbol, or command references when available. Separate confirmed facts from assumptions.",
    "For debugging questions, identify the most likely cause, the evidence behind it, and the next verification step instead of presenting speculation as certainty.",
    "Keep answers concise and useful; include examples only when they reduce ambiguity.",
  ].join("\n") },
];

export const defaultAiProviders: AiProviderConfig[] = [createProviderFromPreset(getAiProviderPreset("local-proxy")!, [])];

export const defaultAiPreferences: AiPreferences = {
  projectIndexingEnabled: true,
  realtimeIndexing: true,
  includeImages: true,
  visionImageFormat: "auto",
  scanConcurrency: "auto",
  maxIndexedFiles: 5000,
  contextAutoCompactEnabled: true,
  contextAutoCompactThreshold: DEFAULT_CONTEXT_AUTO_COMPACT_THRESHOLD,
  toolRoundLimit: defaultAiToolRoundLimit,
  maxParallelSubagents: defaultMaxParallelSubagents,
  goalRunMaxTokens: null,
  goalRunMaxRounds: null,
  // Ship 60-minute hard stop by default — unlimited execution is an explicit opt-in
  // (set to null via Settings). Prevents runaway autonomous runs from burning tokens
  // indefinitely when a task is stuck or the user walks away (F6 fix).
  automaticModeHardStopMinutes: 60,
  hiddenModelIds: [],
  seenTelegramNotice: false,
  showResponseDuration: true,
  // Token economy ships ON by default: terse "caveman" output that drops filler/
  // pleasantries to save output tokens while keeping code, paths, errors, tool work,
  // and reasoning depth exact. Users can turn it off in Settings → AI Usage.
  tokenEconomyEnabled: true,
  lspAutoInstall: true,
  runtimeAutoProvision: true,
  customSystemPromptEnabled: false,
  customSystemPrompt: "",
  toolPermissionRules: [],
  globalInstructions: "",
  projectInstructionsByWorkspace: {},
  agentMode: "agent",
  selectedAgentId: "agent",
  agentProfiles: defaultAiAgentProfiles,
  selectedProviderId: defaultAiProviderId,
  selectedModelId: defaultAiModelId,
  selectedEffortId: defaultAiEffortId,
  toolApprovalMode: "full-access",
  fileEditTrustMode: "preview-before-apply",
  providers: defaultAiProviders,
  voiceInputEnabled: true,
  voiceInputProvider: "native-webview",
  voiceInputLanguage: "auto",
  localSttCommand: "",
  localSttModelPath: "",
  agentBrowserEnabled: true,
  agentBrowserCommand: "",
  agentBrowserHeaded: false,
  agentBrowserAllowedDomains: "",
  agentBrowserMaxOutput: 50_000,
  agentBrowserPersistSession: true,
  agentBrowserProfile: "",
  agentBrowserStatePath: "",
  agentBrowserContentBoundaries: true,
  agentBrowserIgnoreHttpsErrors: false,
  agentBrowserAutoStreamPreview: true,
  agentBrowserDashboardPort: 4848,
  agentBrowserAllowFileAccess: false,
  agentBrowserProvider: "",
  agentBrowserProxy: "",
};

export function mergeAiPreferences(current: AiPreferences, patch: Partial<AiPreferences>) {
  return normalizeAiPreferences({ ...current, ...patch }, { preserveText: true });
}

type NormalizeAiPreferencesOptions = {
  // When true, user-editable text fields (names, aliases, labels, URLs) keep their raw value
  // instead of falling back to a default. This prevents the settings UI from restoring text
  // the moment a field is fully cleared while the user is still editing it.
  preserveText: boolean;
};

export function normalizeAiPreferences(value: unknown, options: NormalizeAiPreferencesOptions = { preserveText: false }): AiPreferences {
  const { preserveText } = options;
  const source = isRecord(value) ? value : {};
  const agentProfiles = normalizeAgentProfiles(source.agentProfiles, preserveText);
  const selectedAgentId = normalizeSelectedAgentId(source.selectedAgentId ?? source.agentMode, agentProfiles);
  const selectedAgent = getAiAgentProfile(agentProfiles, selectedAgentId) ?? agentProfiles[0];
  const providers = normalizeProviders(source.providers, preserveText);
  const selectedProviderId = normalizeSelectedProviderId(source.selectedProviderId, providers);
  const selectedProvider = getAiProvider(providers, selectedProviderId) ?? providers[0];
  const selectedModelId = normalizeSelectedModelId(source.selectedModelId ?? source.model, selectedProvider);
  const selectedModel = getAiModel(selectedProvider, selectedModelId) ?? selectedProvider.models[0];
  const selectedEffortId = normalizeSelectedEffortId(source.selectedEffortId ?? source.reasoningEffort, selectedModel);
  const toolApprovalMode = normalizeToolApprovalMode(source.toolApprovalMode ?? source.approvalMode);

  return {
    projectIndexingEnabled: typeof source.projectIndexingEnabled === "boolean" ? source.projectIndexingEnabled : defaultAiPreferences.projectIndexingEnabled,
    realtimeIndexing: typeof source.realtimeIndexing === "boolean" ? source.realtimeIndexing : defaultAiPreferences.realtimeIndexing,
    includeImages: typeof source.includeImages === "boolean" ? source.includeImages : defaultAiPreferences.includeImages,
    visionImageFormat: normalizeVisionImageFormat(source.visionImageFormat),
    scanConcurrency: normalizeScanConcurrency(source.scanConcurrency),
    maxIndexedFiles: clampInteger(source.maxIndexedFiles, 500, 20000, defaultAiPreferences.maxIndexedFiles),
    contextAutoCompactEnabled: typeof source.contextAutoCompactEnabled === "boolean"
      ? source.contextAutoCompactEnabled
      : defaultAiPreferences.contextAutoCompactEnabled,
    contextAutoCompactThreshold: clampContextAutoCompactThreshold(
      typeof source.contextAutoCompactThreshold === "number"
        ? source.contextAutoCompactThreshold
        : defaultAiPreferences.contextAutoCompactThreshold,
    ),
    toolRoundLimit: normalizeToolRoundLimit(resolveToolRoundLimitSource(source)),
    maxParallelSubagents: clampInteger(source.maxParallelSubagents, maxParallelSubagentsMin, maxParallelSubagentsMax, defaultMaxParallelSubagents),
    goalRunMaxTokens: typeof source.goalRunMaxTokens === "number" ? clampInteger(source.goalRunMaxTokens, 10_000, 500_000, 200_000) : source.goalRunMaxTokens === null ? null : defaultAiPreferences.goalRunMaxTokens,
    goalRunMaxRounds: typeof source.goalRunMaxRounds === "number" ? clampInteger(source.goalRunMaxRounds, 8, 80, 32) : source.goalRunMaxRounds === null ? null : defaultAiPreferences.goalRunMaxRounds,
    automaticModeHardStopMinutes: typeof source.automaticModeHardStopMinutes === "number" ? clampInteger(source.automaticModeHardStopMinutes, 15, 480, 60) : source.automaticModeHardStopMinutes === null ? null : defaultAiPreferences.automaticModeHardStopMinutes,
    hiddenModelIds: Array.isArray(source.hiddenModelIds)
      ? [...new Set(source.hiddenModelIds.filter((id): id is string => typeof id === "string" && id.length > 0))].slice(-500)
      : defaultAiPreferences.hiddenModelIds,
    seenTelegramNotice: typeof source.seenTelegramNotice === "boolean" ? source.seenTelegramNotice : defaultAiPreferences.seenTelegramNotice,
    showResponseDuration: typeof source.showResponseDuration === "boolean" ? source.showResponseDuration : defaultAiPreferences.showResponseDuration,
    tokenEconomyEnabled: typeof source.tokenEconomyEnabled === "boolean" ? source.tokenEconomyEnabled : defaultAiPreferences.tokenEconomyEnabled,
    lspAutoInstall: typeof source.lspAutoInstall === "boolean" ? source.lspAutoInstall : defaultAiPreferences.lspAutoInstall,
    runtimeAutoProvision: typeof source.runtimeAutoProvision === "boolean" ? source.runtimeAutoProvision : defaultAiPreferences.runtimeAutoProvision,
    customSystemPromptEnabled: typeof source.customSystemPromptEnabled === "boolean" ? source.customSystemPromptEnabled : defaultAiPreferences.customSystemPromptEnabled,
    customSystemPrompt: normalizeEditableText(source.customSystemPrompt, defaultAiPreferences.customSystemPrompt, preserveText),
    toolPermissionRules: Array.isArray(source.toolPermissionRules)
      ? source.toolPermissionRules
          .filter((rule): rule is string => typeof rule === "string" && rule.trim().length > 0)
          .map((rule) => rule.trim())
          .slice(0, 100)
      : defaultAiPreferences.toolPermissionRules,
    globalInstructions: normalizeEditableText(source.globalInstructions, defaultAiPreferences.globalInstructions, preserveText),
    projectInstructionsByWorkspace: normalizeProjectInstructions(source.projectInstructionsByWorkspace, preserveText),
    agentMode: selectedAgent.mode,
    selectedAgentId,
    agentProfiles,
    selectedProviderId,
    selectedModelId,
    selectedEffortId,
    toolApprovalMode,
    fileEditTrustMode: source.fileEditTrustMode === "apply-immediately" || source.fileEditTrustMode === "preview-before-apply"
      ? source.fileEditTrustMode
      : defaultAiPreferences.fileEditTrustMode,
    providers,
    voiceInputEnabled: typeof source.voiceInputEnabled === "boolean" ? source.voiceInputEnabled : defaultAiPreferences.voiceInputEnabled,
    voiceInputProvider: normalizeVoiceInputProvider(source.voiceInputProvider),
    voiceInputLanguage: normalizeVoiceInputLanguage(source.voiceInputLanguage),
    localSttCommand: normalizeEditableText(source.localSttCommand, "", preserveText),
    localSttModelPath: normalizeEditableText(source.localSttModelPath, "", preserveText),
    agentBrowserEnabled: typeof source.agentBrowserEnabled === "boolean" ? source.agentBrowserEnabled : defaultAiPreferences.agentBrowserEnabled,
    agentBrowserCommand: normalizeEditableText(source.agentBrowserCommand, "", preserveText),
    agentBrowserHeaded: typeof source.agentBrowserHeaded === "boolean" ? source.agentBrowserHeaded : defaultAiPreferences.agentBrowserHeaded,
    agentBrowserAllowedDomains: normalizeEditableText(source.agentBrowserAllowedDomains, "", preserveText),
    agentBrowserMaxOutput: clampInteger(source.agentBrowserMaxOutput, 4_000, 120_000, defaultAiPreferences.agentBrowserMaxOutput),
    agentBrowserPersistSession: typeof source.agentBrowserPersistSession === "boolean" ? source.agentBrowserPersistSession : defaultAiPreferences.agentBrowserPersistSession,
    agentBrowserProfile: normalizeEditableText(source.agentBrowserProfile, "", preserveText),
    agentBrowserStatePath: normalizeEditableText(source.agentBrowserStatePath, "", preserveText),
    agentBrowserContentBoundaries: typeof source.agentBrowserContentBoundaries === "boolean" ? source.agentBrowserContentBoundaries : defaultAiPreferences.agentBrowserContentBoundaries,
    agentBrowserIgnoreHttpsErrors: typeof source.agentBrowserIgnoreHttpsErrors === "boolean" ? source.agentBrowserIgnoreHttpsErrors : defaultAiPreferences.agentBrowserIgnoreHttpsErrors,
    agentBrowserAutoStreamPreview: typeof source.agentBrowserAutoStreamPreview === "boolean" ? source.agentBrowserAutoStreamPreview : defaultAiPreferences.agentBrowserAutoStreamPreview,
    agentBrowserDashboardPort: clampInteger(source.agentBrowserDashboardPort, 1024, 65_535, defaultAiPreferences.agentBrowserDashboardPort),
    agentBrowserAllowFileAccess: typeof source.agentBrowserAllowFileAccess === "boolean" ? source.agentBrowserAllowFileAccess : defaultAiPreferences.agentBrowserAllowFileAccess,
    agentBrowserProvider: normalizeEditableText(source.agentBrowserProvider, "", preserveText),
    agentBrowserProxy: normalizeEditableText(source.agentBrowserProxy, "", preserveText),
  };
}

export function getAiProvider(providers: AiProviderConfig[], providerId: string) {
  return providers.find((provider) => provider.id === providerId) ?? null;
}

export function getAiProviderPreset(presetId: string) {
  return AI_PROVIDER_PRESETS.find((preset) => preset.id === presetId) ?? null;
}

export function getAiAgentProfile(profiles: AiAgentProfile[], profileId: string) {
  return profiles.find((profile) => profile.id === profileId) ?? null;
}

export function getAiModel(provider: AiProviderConfig | null | undefined, modelId: string) {
  return provider?.models.find((model) => model.id === modelId) ?? null;
}

export function workspaceInstructionsKey(workspaceRoot: string | null | undefined) {
  return workspaceRoot?.trim().replaceAll("\\", "/") ?? "";
}

export function getAiProjectInstructions(preferences: AiPreferences, workspaceRoot: string | null | undefined) {
  const key = workspaceInstructionsKey(workspaceRoot);
  return key ? preferences.projectInstructionsByWorkspace[key] ?? "" : "";
}

export function createAiProviderConfig(existingProviders: AiProviderConfig[], presetId: AiProviderPresetId = "custom"): AiProviderConfig {
  return createProviderFromPreset(getAiProviderPreset(presetId) ?? getAiProviderPreset("custom")!, existingProviders);
}

export function createAiAgentProfile(existingProfiles: AiAgentProfile[]): AiAgentProfile {
  const id = uniqueConfigId("agent", existingProfiles.map((profile) => profile.id));
  return {
    id,
    name: "Custom agent",
    mode: "agent",
    instructions: "Describe how this agent should behave.",
  };
}

export function isDefaultAiAgentProfile(profileId: string) {
  return defaultAiAgentProfiles.some((profile) => profile.id === profileId);
}

export function createAiModelConfig(existingModels: AiModelConfig[]): AiModelConfig {
  const id = uniqueConfigId("model", existingModels.map((model) => model.id));
  const alias = "model-name";
  return {
    id,
    name: "New model",
    alias,
    contextTokens: inferContextTokensFromModelRef(alias),
    inputPricePerMillion: null,
    outputPricePerMillion: null,
    effortLevels: [],
  };
}

export function createAiEffortConfig(existingEfforts: AiEffortConfig[]): AiEffortConfig {
  const id = uniqueConfigId("effort", existingEfforts.map((effort) => effort.id));
  return { id, label: "New effort" };
}

export function buildLocalProxyBaseUrl(host: string, port: string, path: string) {
  const normalizedHost = host.trim() || "127.0.0.1";
  const normalizedPort = port.trim();
  const normalizedPath = normalizeApiPath(path, "/v1");
  const bracketedHost = normalizedHost.includes(":") && !normalizedHost.startsWith("[") ? `[${normalizedHost}]` : normalizedHost;
  return `http://${bracketedHost}${normalizedPort ? `:${normalizedPort}` : ""}${normalizedPath}`;
}

export function isLocalProxyProvider(provider: AiProviderConfig) {
  return provider.protocol === "local-proxy";
}

function createProviderFromPreset(preset: AiProviderPreset, existingProviders: AiProviderConfig[]): AiProviderConfig {
  const id = uniqueConfigId(preset.id, existingProviders.map((provider) => provider.id));
  const localHost = preset.localHost ?? "";
  const localPort = preset.localPort ?? "";
  const localPath = preset.localPath ?? "/v1";
  return {
    id,
    name: preset.name,
    providerType: preset.id,
    protocol: preset.protocol,
    baseUrl: preset.protocol === "local-proxy" ? buildLocalProxyBaseUrl(localHost, localPort, localPath) : preset.baseUrl,
    apiKey: "",
    localHost,
    localPort,
    localPath,
    models: cloneModelTemplates(preset.models),
    embeddingModel: "",
  };
}

function normalizeAgentProfiles(value: unknown, preserveText: boolean): AiAgentProfile[] {
  const incoming = Array.isArray(value)
    ? value.map((profile) => normalizeAgentProfile(profile, preserveText)).filter((profile): profile is AiAgentProfile => Boolean(profile))
    : [];
  const byId = new Map<string, AiAgentProfile>();
  for (const profile of cloneDefaultAgentProfiles()) byId.set(profile.id, profile);
  for (const profile of incoming) byId.set(profile.id, profile);
  return sortAgentProfilesByModeOrder(Array.from(byId.values()));
}

function sortAgentProfilesByModeOrder(profiles: AiAgentProfile[]) {
  const rank = new Map(AI_AGENT_MODE_ORDER.map((mode, index) => [mode, index]));
  return [...profiles].sort((left, right) => {
    const leftRank = rank.get(left.mode) ?? AI_AGENT_MODE_ORDER.length;
    const rightRank = rank.get(right.mode) ?? AI_AGENT_MODE_ORDER.length;
    if (leftRank !== rightRank) return leftRank - rightRank;
    return left.name.localeCompare(right.name);
  });
}

function normalizeAgentProfile(value: unknown, preserveText: boolean): AiAgentProfile | null {
  if (!isRecord(value)) return null;
  const id = normalizeIdentifier(value.id, "agent");
  return {
    id,
    name: normalizeEditableText(value.name, id, preserveText),
    mode: normalizeAgentMode(value.mode ?? id),
    instructions: normalizeEditableText(value.instructions, "", preserveText),
  };
}

function normalizeSelectedAgentId(value: unknown, profiles: AiAgentProfile[]) {
  const requested = typeof value === "string" ? normalizeAgentSelection(value) : defaultAiPreferences.selectedAgentId;
  return profiles.some((profile) => profile.id === requested) ? requested : profiles[0].id;
}

function normalizeAgentSelection(value: string) {
  return value === "edit" ? "plan" : value;
}

function normalizeProviders(value: unknown, preserveText: boolean): AiProviderConfig[] {
  const providers = Array.isArray(value)
    ? value.map((provider) => normalizeProvider(provider, preserveText)).filter((provider): provider is AiProviderConfig => Boolean(provider))
    : [];
  const normalized = dedupeById(providers);
  return normalized.length > 0 ? normalized : cloneDefaultProviders();
}

function normalizeProjectInstructions(value: unknown, preserveText: boolean): Record<string, string> {
  if (!isRecord(value)) return {};
  const instructions: Record<string, string> = {};
  for (const [workspaceRoot, text] of Object.entries(value)) {
    const key = workspaceInstructionsKey(workspaceRoot);
    if (!key || typeof text !== "string") continue;
    const normalizedText = preserveText ? text : text.trim();
    if (normalizedText) instructions[key] = normalizedText;
  }
  return instructions;
}

function normalizeProvider(value: unknown, preserveText: boolean): AiProviderConfig | null {
  if (!isRecord(value)) return null;
  const presetId = normalizeProviderPresetId(value.providerType ?? value.presetId ?? value.kind) ?? inferProviderPresetId(value);
  const preset = getAiProviderPreset(presetId) ?? getAiProviderPreset("custom")!;
  const protocol = normalizeProviderProtocol(value.protocol ?? value.providerProtocol, preset.protocol);
  const localEndpoint = normalizeLocalEndpoint(value, preset);
  const baseUrl = protocol === "local-proxy"
    ? buildLocalProxyBaseUrl(localEndpoint.localHost, localEndpoint.localPort, localEndpoint.localPath)
    : normalizeEditableText(value.baseUrl, "", preserveText);

  return {
    id: normalizeIdentifier(value.id, preset.id),
    name: normalizeProviderName(value.name, preset, preserveText),
    providerType: preset.id,
    protocol,
    baseUrl,
    apiKey: typeof value.apiKey === "string" ? value.apiKey : "",
    localHost: localEndpoint.localHost,
    localPort: localEndpoint.localPort,
    localPath: localEndpoint.localPath,
    models: normalizeModels(value.models, preserveText, preset.models),
    embeddingModel: normalizeEditableText(value.embeddingModel, "", preserveText),
  };
}

function normalizeProviderName(value: unknown, preset: AiProviderPreset, preserveText: boolean) {
  const name = normalizeEditableText(value, preset.name, preserveText);
  if (!preserveText && preset.id === "local-proxy" && name === `${preset.name} proxy`) return preset.name;
  return name;
}

function normalizeModels(value: unknown, preserveText: boolean, fallbackModels: readonly AiModelTemplate[] = []): AiModelConfig[] {
  const models = Array.isArray(value)
    ? value.map((model) => normalizeModelConfig(model, preserveText)).filter((model): model is AiModelConfig => Boolean(model))
    : [];
  const normalized = dedupeById(models);
  if (normalized.length > 0) return normalized;
  if (fallbackModels.length > 0) return cloneModelTemplates(fallbackModels);
  return [createAiModelConfig([])];
}

function normalizeModelConfig(value: unknown, preserveText: boolean): AiModelConfig | null {
  if (!isRecord(value)) return null;
  const id = normalizeIdentifier(value.id, "model");
  const alias = normalizeEditableText(value.alias, id, preserveText);
  const contextTokens = normalizeModelContextTokens(value.contextTokens, alias || id, preserveText);
  return {
    id,
    name: normalizeEditableText(value.name, id, preserveText),
    alias,
    contextTokens,
    inputPricePerMillion: normalizeModelPrice(value.inputPricePerMillion),
    outputPricePerMillion: normalizeModelPrice(value.outputPricePerMillion),
    effortLevels: normalizeEffortLevels(value.effortLevels, preserveText),
  };
}

/** Manual per-million token price: a finite, non-negative number, else null (use fallback rates). */
function normalizeModelPrice(value: unknown): number | null {
  if (value === null || value === undefined || value === "") return null;
  const parsed = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) return null;
  return Math.round(parsed * 1_000_000) / 1_000_000;
}

function normalizeModelContextTokens(value: unknown, modelRef: string, preserveText: boolean) {
  if (value === null || value === undefined || value === "") return null;
  if (typeof value === "number" && Number.isFinite(value)) {
    if (preserveText && value <= 0) return null;
    return value > 0 ? clampModelContextTokens(value) : null;
  }
  if (typeof value === "string") {
    const parsed = Number(value.trim());
    if (!Number.isFinite(parsed) || parsed <= 0) return preserveText ? null : inferContextTokensFromModelRef(modelRef);
    return clampModelContextTokens(parsed);
  }
  return null;
}

function normalizeEffortLevels(value: unknown, preserveText: boolean): AiEffortConfig[] {
  if (!Array.isArray(value)) return [];
  return dedupeById(value.map((effort) => normalizeEffortConfig(effort, preserveText)).filter((effort): effort is AiEffortConfig => Boolean(effort)));
}

function normalizeEffortConfig(value: unknown, preserveText: boolean): AiEffortConfig | null {
  if (!isRecord(value)) return null;
  const id = normalizeIdentifier(value.id, "effort");
  return {
    id,
    label: normalizeEditableText(value.label, id, preserveText),
  };
}

function normalizeSelectedProviderId(value: unknown, providers: AiProviderConfig[]) {
  const requested = typeof value === "string" ? value : defaultAiPreferences.selectedProviderId;
  return providers.some((provider) => provider.id === requested) ? requested : providers[0].id;
}

function normalizeSelectedModelId(value: unknown, provider: AiProviderConfig) {
  const requested = typeof value === "string" ? value : defaultAiPreferences.selectedModelId;
  return provider.models.some((model) => model.id === requested) ? requested : provider.models[0].id;
}

function normalizeSelectedEffortId(value: unknown, model: AiModelConfig) {
  if (model.effortLevels.length === 0) return "";
  const requested = typeof value === "string" ? value : defaultAiPreferences.selectedEffortId;
  return model.effortLevels.some((effort) => effort.id === requested) ? requested : model.effortLevels[0].id;
}

function normalizeProviderPresetId(value: unknown): AiProviderPresetId | null {
  if (typeof value !== "string") return null;
  return AI_PROVIDER_PRESETS.some((preset) => preset.id === value) ? value as AiProviderPresetId : null;
}

function normalizeProviderProtocol(value: unknown, fallback: AiProviderProtocol): AiProviderProtocol {
  if (value === "openai-compatible" || value === "anthropic" || value === "google" || value === "azure-openai" || value === "local-proxy") return value;
  return fallback;
}

function inferProviderPresetId(value: Record<string, unknown>): AiProviderPresetId {
  const haystack = [value.id, value.name, value.baseUrl]
    .map((candidate) => typeof candidate === "string" ? candidate.toLowerCase() : "")
    .join(" ");
  if (haystack.includes("anthropic") || haystack.includes("claude")) return "anthropic";
  if (haystack.includes("openrouter")) return "openrouter";
  if (haystack.includes("generativelanguage") || haystack.includes("gemini") || haystack.includes("google")) return "google";
  if (haystack.includes("mistral")) return "mistral";
  if (haystack.includes("groq")) return "groq";
  if (haystack.includes("cohere")) return "cohere";
  if (haystack.includes("deepseek")) return "deepseek";
  if (haystack.includes("x.ai") || haystack.includes("grok") || haystack.includes("xai")) return "xai";
  if (haystack.includes("azure") || haystack.includes("openai.azure.com")) return "azure-openai";
  if (haystack.includes("11434") || haystack.includes("ollama")) return "ollama";
  if (haystack.includes("1234") || haystack.includes("lm studio") || haystack.includes("lm-studio")) return "lm-studio";
  if (haystack.includes("localhost") || haystack.includes("127.0.0.1") || haystack.includes("0.0.0.0")) return "local-proxy";
  if (haystack.includes("openai")) return "openai";
  return "custom";
}

function normalizeLocalEndpoint(value: Record<string, unknown>, preset: AiProviderPreset) {
  const parsed = parseLocalEndpoint(normalizeTextSetting(value.baseUrl));
  const localHost = normalizeNullableDisplayText(value.localHost ?? value.proxyHost ?? parsed.host, preset.localHost ?? "127.0.0.1");
  const localPort = normalizePortText(value.localPort ?? value.proxyPort ?? parsed.port, preset.localPort ?? "8080");
  const localPath = normalizeApiPath(value.localPath ?? value.proxyPath ?? parsed.path, preset.localPath ?? "/v1");
  return { localHost, localPort, localPath };
}

function parseLocalEndpoint(value: string) {
  if (!value) return { host: "", port: "", path: "" };
  try {
    const url = new URL(value);
    return { host: url.hostname, port: url.port, path: url.pathname };
  } catch {
    return { host: "", port: "", path: "" };
  }
}

function normalizePortText(value: unknown, fallback: string) {
  if (typeof value !== "string" && typeof value !== "number") return fallback;
  const text = String(value).trim();
  if (!text) return typeof value === "string" ? "" : fallback;
  if (/^\d{1,5}$/.test(text)) {
    const n = Number(text);
    return n >= 1 && n <= 65535 ? text : fallback;
  }
  return fallback;
}

function normalizeApiPath(value: unknown, fallback: string) {
  const text = typeof value === "string" ? value.trim() : normalizeTextSetting(value) || fallback;
  if (!text) return "";
  const path = text.startsWith("/") ? text : `/${text}`;
  return path.length > 1 ? path.replace(/\/+$/g, "") : path;
}

function normalizeTextSetting(value: unknown) {
  return typeof value === "string" ? value.trim() : "";
}

function normalizeDisplayText(value: unknown, fallback: string) {
  const text = normalizeTextSetting(value);
  return text || fallback;
}

// Normalizes a user-editable text field. During live editing (preserveText) the raw string is
// kept verbatim — including empty — so clearing a field does not snap back to a fallback value.
// On load (preserveText=false) the fallback is applied so persisted/imported data stays valid.
function normalizeEditableText(value: unknown, fallback: string, preserveText: boolean) {
  if (preserveText) return typeof value === "string" ? value : fallback;
  return normalizeDisplayText(value, fallback);
}

function normalizeNullableDisplayText(value: unknown, fallback: string) {
  if (typeof value === "string") return value.trim();
  const text = normalizeTextSetting(value);
  return text || fallback;
}

function normalizeIdentifier(value: unknown, fallbackPrefix: string) {
  const text = normalizeTextSetting(value)
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return text || `${fallbackPrefix}-${Date.now().toString(36)}-${crypto.randomUUID().slice(0, 8)}`;
}

function normalizeAgentMode(value: unknown): AiAgentMode {
  if (value === "agent" || value === "automatic" || value === "plan" || value === "ask") return value;
  if (value === "edit") return "plan";
  return defaultAiPreferences.agentMode;
}

function normalizeToolApprovalMode(value: unknown): AiToolApprovalMode {
  if (value === "full-access" || value === "fullAccess" || value === "auto") return "full-access";
  return defaultAiPreferences.toolApprovalMode;
}

function normalizeVoiceInputProvider(value: unknown): AiVoiceInputProvider {
  return value === "native-webview" || value === "local" ? value : defaultAiPreferences.voiceInputProvider;
}

function normalizeVoiceInputLanguage(value: unknown): AiVoiceInputLanguage {
  return value === "ru-RU" || value === "en-US" || value === "auto" ? value : defaultAiPreferences.voiceInputLanguage;
}

function normalizeVisionImageFormat(value: unknown): AiVisionImageFormatPreference {
  return value === "webp" || value === "png" || value === "auto" ? value : defaultAiPreferences.visionImageFormat;
}

function normalizeScanConcurrency(value: unknown): AiScanConcurrency {
  return value === "all" || value === "half" || value === "auto" ? value : defaultAiPreferences.scanConcurrency;
}

function resolveToolRoundLimitSource(source: Record<string, unknown>) {
  if (Object.hasOwn(source, "toolRoundLimit")) return source.toolRoundLimit;
  if (Object.hasOwn(source, "maxToolRounds")) return source.maxToolRounds;
  if (Object.hasOwn(source, "toolRounds")) return source.toolRounds;
  return undefined;
}

function normalizeToolRoundLimit(value: unknown): AiToolRoundLimit {
  if (value === undefined || value === null || value === "" || value === "unlimited" || value === "none" || value === 0 || String(value).trim() === "0") return defaultAiToolRoundLimit;
  return clampInteger(value, aiToolRoundLimitMin, aiToolRoundLimitMax, defaultLimitedAiToolRoundLimit);
}

function clampInteger(value: unknown, min: number, max: number, fallback: number) {
  const numberValue = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numberValue)) return fallback;
  return Math.min(max, Math.max(min, Math.round(numberValue)));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function dedupeById<T extends { id: string }>(items: T[]) {
  const seen = new Set<string>();
  return items.filter((item) => {
    if (seen.has(item.id)) return false;
    seen.add(item.id);
    return true;
  });
}

function uniqueConfigId(preferredId: string, existingIds: string[]) {
  const existing = new Set(existingIds);
  const normalizedPreferredId = normalizeIdentifier(preferredId, "config");
  if (!existing.has(normalizedPreferredId)) return normalizedPreferredId;
  for (let index = 2; index < 1000; index += 1) {
    const id = `${normalizedPreferredId}-${index}`;
    if (!existing.has(id)) return id;
  }
  return `${normalizedPreferredId}-${crypto.randomUUID().slice(0, 8)}`;
}

function cloneDefaultProviders() {
  return defaultAiProviders.map(cloneProvider);
}

function cloneProvider(provider: AiProviderConfig): AiProviderConfig {
  return {
    ...provider,
    models: provider.models.map((model) => ({
      ...model,
      effortLevels: model.effortLevels.map((effort) => ({ ...effort })),
    })),
  };
}

function cloneModelTemplates(models: readonly AiModelTemplate[]): AiModelConfig[] {
  return models.map((model) => ({
    id: model.id,
    name: model.name,
    alias: model.alias,
    contextTokens: model.contextTokens ?? inferContextTokensFromModelRef(model.alias || model.id),
    inputPricePerMillion: null,
    outputPricePerMillion: null,
    effortLevels: cloneEfforts(model.effortLevels ?? []),
  }));
}

function cloneEfforts(efforts: readonly AiEffortConfig[]) {
  return efforts.map((effort) => ({ ...effort }));
}

function cloneDefaultAgentProfiles() {
  return defaultAiAgentProfiles.map((profile) => ({ ...profile }));
}
