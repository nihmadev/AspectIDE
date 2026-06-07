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
  showResponseDuration: boolean;
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
  | "google"
  | "mistral"
  | "groq"
  | "cohere"
  | "deepseek"
  | "xai"
  | "azure-openai"
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
};

export type AiModelConfig = {
  id: string;
  name: string;
  alias: string;
  /** Max context tokens for this model. Omit or 0 to auto-detect from alias/id. */
  contextTokens?: number | null;
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
    "When changing code, keep behavior production-ready: handle errors explicitly, avoid silent fallbacks, avoid placeholder implementations, and surface real residual risk if verification cannot cover it.",
    "Final reports should be concise: what changed, what was verified, and what remains only if something genuinely remains.",
  ].join("\n") },
  { id: "plan", name: "Plan", mode: "plan", instructions: [
    "Stay read-only unless the user explicitly approves implementation. Gather only enough context to understand the task, constraints, affected files, and verification surface.",
    "Return a concrete execution plan with assumptions, edit targets, ordering, risk points, and validation commands. Do not bury uncertainty; name the decision that needs confirmation.",
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
  showResponseDuration: true,
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
    showResponseDuration: typeof source.showResponseDuration === "boolean" ? source.showResponseDuration : defaultAiPreferences.showResponseDuration,
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
    effortLevels: normalizeEffortLevels(value.effortLevels, preserveText),
  };
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
  if (/^\d{1,5}$/.test(text)) return text;
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
  return text || `${fallbackPrefix}-${Date.now().toString(36)}`;
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
  if (value === undefined || value === null || value === "" || value === "unlimited" || value === "none" || value === 0) return defaultAiToolRoundLimit;
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
    effortLevels: cloneEfforts(model.effortLevels ?? []),
  }));
}

function cloneEfforts(efforts: readonly AiEffortConfig[]) {
  return efforts.map((effort) => ({ ...effort }));
}

function cloneDefaultAgentProfiles() {
  return defaultAiAgentProfiles.map((profile) => ({ ...profile }));
}
