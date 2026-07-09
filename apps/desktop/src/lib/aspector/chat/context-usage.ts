import {
  estimateHistoryTokens,
  estimateTokens,
  isCompactionCheckpointMessage,
  pruneStaleToolOutputs,
  resolveContextUsageBudget,
} from "./context-compaction";
import { resolveEffectiveAutoCompactThreshold, type AiAgentMode, type AiModelConfig, type AiPreferences, type AiProviderConfig } from "./../utils/preferences";
import type { AiChatAttachmentInput, AiChatMessage } from "./types";
import { normalizeVisibleReasoning } from "./reasoning";
import { clampContextAutoCompactThreshold, resolveContextCompactTriggerTokens } from "./../utils/model-context";
import { automaticModeEnforcementPrompt } from "./../automatic/mode-enforcement";
import { luxSystemPromptBaseText } from "./../utils/system-prompt";
import type { TranslateFn } from "../../i18n/useTranslation";

export type AiChatContextUsageRow = {
  color: string;
  detail: string;
  id: string;
  label: string;
  percent: number;
  tokens: number;
};

export type AiChatContextUsageSummary = {
  percent: number;
  rows: AiChatContextUsageRow[];
  tokenBudget: number;
  totalTokens: number;
};

export type AiChatContextAttachment = Pick<AiChatAttachmentInput, "name" | "size">;

export type BuildContextUsageInput = {
  pinnedEditorPaths: string[];
  aiIndexStatus: string;
  agentInstruction: string;
  agentName: string;
  attachments: AiChatContextAttachment[];
  conversation: AiChatMessage[];
  message: string;
  preferences: AiPreferences;
  /** Active provider — carries an optional per-provider auto-compact override. */
  selectedProvider?: AiProviderConfig | null;
  selectedModel: AiModelConfig | null;
  selectedModelAlias: string;
  t: TranslateFn;
  hasProjectInstructions?: boolean;
  hasGlobalInstructions?: boolean;
};

export type AiChatContextUsageMeta = {
  autoCompactEnabled: boolean;
  autoCompactThresholdPercent: number;
  compactTriggerTokens: number;
};

const contextRowColors = {
  attachments: "#d7a85d",
  compaction: "#9b7ed8",
  history: "#6d8589",
  index: "#57c178",
  input: "#8fb5d9",
  model: "#9aa0a6",
  openFile: "#c990d8",
  system: "#b8b8b8",
  tools: "#7aa6a1",
} as const;

// The two system-prompt base texts vary only by agent mode (constant per mode,
// never changes at runtime), so caching their token estimates avoids re-tokenizing
// the same multi-KB string every time the context-usage memo fires.
const basePromptTokenCache = new Map<AiAgentMode, number>();

function cachedBasePromptTokens(mode: AiAgentMode): number {
  let tokens = basePromptTokenCache.get(mode);
  if (tokens === undefined) {
    tokens = estimateTokens(luxSystemPromptBaseText(mode))
      + (mode === "automatic" ? estimateTokens(automaticModeEnforcementPrompt) : 0);
    basePromptTokenCache.set(mode, tokens);
  }
  return tokens;
}

export function buildAiChatContextUsageSummary({
  pinnedEditorPaths,
  aiIndexStatus,
  agentInstruction,
  agentName,
  attachments,
  conversation,
  message,
  preferences,
  selectedProvider,
  selectedModel,
  selectedModelAlias,
  t,
  hasGlobalInstructions,
  hasProjectInstructions,
}: BuildContextUsageInput): AiChatContextUsageSummary & AiChatContextUsageMeta {
  const contextTokenBudget = resolveContextUsageBudget(selectedModel);
  // Effective threshold: model override → provider override → global preference.
  const effectiveThreshold = clampContextAutoCompactThreshold(
    resolveEffectiveAutoCompactThreshold(preferences.contextAutoCompactThreshold, selectedProvider, selectedModel),
  );
  const autoCompactThresholdPercent = Math.round(effectiveThreshold * 100);
  const compactTriggerTokens = resolveContextCompactTriggerTokens(selectedModel, effectiveThreshold);
  const hasInstructions = hasGlobalInstructions === true || hasProjectInstructions === true;
  const systemDetail = [
    agentName || preferences.agentMode,
    preferences.toolApprovalMode === "full-access" ? "full-access" : "default",
    hasInstructions ? t("aiChat.context.withInstructions") : "",
  ].filter(Boolean).join(" · ");
  // The context meter must estimate what is actually sent next, not the raw stored
  // transcript. Older bulky tool outputs are replaced with small reload markers by
  // the turn/compaction pipeline, so counting raw history can show fake values like
  // "320K / 239K" while the next request is far smaller.
  const promptConversation = pruneStaleToolOutputs(conversation);
  const basePromptTokens = cachedBasePromptTokens(preferences.agentMode);
  const systemTokens = basePromptTokens
    + estimateTokens([agentName, preferences.agentMode, agentInstruction, preferences.toolApprovalMode].join(" "));
  const modelTokens = estimateTokens(selectedModelAlias);
  const indexTokens = preferences.projectIndexingEnabled && aiIndexStatus !== "disabled" ? estimateTokens([aiIndexStatus, String(preferences.maxIndexedFiles)].join(" ")) : 0;
  const pinnedEditorTokens = pinnedEditorPaths.reduce((sum, path) => sum + estimateTokens(path), 0);
  const attachmentTokens = attachments.reduce((sum, attachment) => sum + estimateAttachmentTokens(attachment), 0);
  const compactionTokens = promptConversation.filter(isCompactionCheckpointMessage).reduce((sum, entry) => sum + estimateTokens(entry.content), 0);
  const historyTokens = promptConversation.reduce((sum, entry) => {
    if (isCompactionCheckpointMessage(entry)) return sum;
    return sum + estimateTokens(entry.content) + estimateTokens(normalizeVisibleReasoning(entry.reasoning) ?? "");
  }, 0);
  const toolTokens = promptConversation.reduce((sum, entry) => sum + (entry.toolCalls?.reduce((toolSum, call) => toolSum
    + estimateTokens(call.tool) + estimateTokens(call.input ?? "") + estimateTokens(call.output ?? "") + estimateTokens(call.error ?? ""), 0) ?? 0), 0);
  const conversationTokens = estimateHistoryTokens(promptConversation);
  const inputTokens = Math.max(estimateTokens(message), message.trim() ? 80 : 0);
  const messageCount = promptConversation.length;
  const toolCount = promptConversation.reduce((sum, entry) => sum + (entry.toolCalls?.length ?? 0), 0);
  const rawRows: Omit<AiChatContextUsageRow, "percent">[] = [
    { color: contextRowColors.system, detail: systemDetail, id: "system", label: t("aiChat.context.system"), tokens: systemTokens },
    { color: contextRowColors.model, detail: selectedModelAlias, id: "model", label: t("aiChat.model.label"), tokens: modelTokens },
    { color: contextRowColors.index, detail: preferences.projectIndexingEnabled ? aiIndexStatus : t("common.off"), id: "index", label: t("aiChat.context.index"), tokens: indexTokens },
    { color: contextRowColors.openFile, detail: pinnedEditorPaths.length > 0 ? pinnedEditorPaths.join(", ") : "", id: "pinned-editor", label: t("aiChat.context.pinnedEditor"), tokens: pinnedEditorTokens },
    { color: contextRowColors.attachments, detail: attachments.length > 0 ? t("aiChat.attachment.count", { count: attachments.length }) : "", id: "attachments", label: t("aiChat.context.attachments"), tokens: attachmentTokens },
    { color: contextRowColors.compaction, detail: compactionTokens > 0 ? t("aiChat.context.compactionActive") : "", id: "compaction", label: t("aiChat.context.compaction"), tokens: compactionTokens },
    { color: contextRowColors.history, detail: messageCount > 0 ? t("aiChat.context.messageCount", { count: messageCount }) : "", id: "history", label: t("aiChat.context.history"), tokens: historyTokens },
    { color: contextRowColors.tools, detail: toolCount > 0 ? t("aiTools.summary.ran", { count: toolCount }) : "", id: "tools", label: t("aiChat.context.tools"), tokens: toolTokens },
    { color: contextRowColors.input, detail: message.trim() ? t("aiChat.context.currentRequest") : "", id: "input", label: t("aiChat.context.input"), tokens: inputTokens },
  ].filter((row) => row.tokens > 0 || row.detail);
  const totalTokens = systemTokens + modelTokens + indexTokens + pinnedEditorTokens + attachmentTokens + conversationTokens + inputTokens;
  const rows = rawRows.map((row) => ({
    ...row,
    percent: totalTokens > 0 ? Math.max(0.8, (row.tokens / totalTokens) * 100) : 0,
  }));

  return {
    percent: Math.min(100, Math.round((totalTokens / contextTokenBudget) * 100)),
    rows,
    tokenBudget: contextTokenBudget,
    totalTokens,
    autoCompactEnabled: preferences.contextAutoCompactEnabled,
    autoCompactThresholdPercent,
    compactTriggerTokens,
  };
}

export { estimateTokens } from "./context-compaction";

export function formatCompactTokens(tokens: number) {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens >= 10_000_000 ? 0 : 1)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(tokens >= 10_000 ? 0 : 1)}K`;
  return String(tokens);
}

export function formatAiChatContextValue(row: AiChatContextUsageRow) {
  const tokens = formatCompactTokens(row.tokens);
  return row.detail ? `${tokens} - ${row.detail}` : tokens;
}

function estimateAttachmentTokens(attachment: AiChatContextAttachment) {
  return estimateTokens(attachment.name) + Math.max(1, Math.ceil(attachment.size / 1024));
}

