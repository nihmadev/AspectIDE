import {
  estimateHistoryTokens,
  estimateTokens,
  isCompactionCheckpointMessage,
  resolveAutoCompactThreshold,
  resolveContextUsageBudget,
} from "./aiChatContextCompaction";
import type { AiModelConfig, AiPreferences } from "./aiPreferences";
import type { AiChatAttachmentInput, AiChatMessage } from "./aiChatTypes";
import { normalizeVisibleReasoning } from "./aiChatReasoning";
import { resolveContextCompactTriggerTokens } from "./aiModelContext";
import { automaticModeEnforcementPrompt } from "./aiAutomaticModeEnforcement";
import { luxSystemPromptBaseText } from "./aiSystemPrompt";
import type { TranslateFn } from "./i18n/useTranslation";

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

export function buildAiChatContextUsageSummary({
  pinnedEditorPaths,
  aiIndexStatus,
  agentInstruction,
  agentName,
  attachments,
  conversation,
  message,
  preferences,
  selectedModel,
  selectedModelAlias,
  t,
hasGlobalInstructions,
    hasProjectInstructions,
  }: BuildContextUsageInput): AiChatContextUsageSummary & AiChatContextUsageMeta {
  const contextTokenBudget = resolveContextUsageBudget(selectedModel);
  const autoCompactThresholdPercent = Math.round(resolveAutoCompactThreshold(preferences) * 100);
  const compactTriggerTokens = resolveContextCompactTriggerTokens(selectedModel, preferences.contextAutoCompactThreshold);
  const hasInstructions = (hasGlobalInstructions === true || hasProjectInstructions === true);
  const systemDetail = [
    agentName || preferences.agentMode,
    preferences.toolApprovalMode === "full-access" ? "full-access" : "default",
    hasInstructions ? t("aiChat.context.withInstructions") : "",
  ].filter(Boolean).join(" · ");
  // The real system message is dominated by the Lux core prompt; count it so the
  // meter reflects the true system-prompt footprint instead of only agent metadata.
  const basePromptTokens = estimateTokens(luxSystemPromptBaseText(preferences.agentMode))
    + (preferences.agentMode === "automatic" ? estimateTokens(automaticModeEnforcementPrompt) : 0);
  const systemTokens = basePromptTokens
    + estimateTokens([agentName, preferences.agentMode, agentInstruction, preferences.toolApprovalMode].join(" "));
  const modelTokens = estimateTokens(selectedModelAlias);
  const indexTokens = preferences.projectIndexingEnabled && aiIndexStatus !== "disabled" ? estimateTokens([aiIndexStatus, String(preferences.maxIndexedFiles)].join(" ")) : 0;
  const pinnedEditorTokens = pinnedEditorPaths.reduce((sum, path) => sum + estimateTokens(path), 0);
  const attachmentTokens = attachments.reduce((sum, attachment) => sum + estimateAttachmentTokens(attachment), 0);
  const compactionTokens = conversation.filter(isCompactionCheckpointMessage).reduce((sum, entry) => sum + estimateTokens(entry.content), 0);
  const historyTokens = conversation.reduce((sum, entry) => {
    if (isCompactionCheckpointMessage(entry)) return sum;
    return sum + estimateTokens(entry.content) + estimateTokens(normalizeVisibleReasoning(entry.reasoning) ?? "");
  }, 0);
  const toolTokens = conversation.reduce((sum, entry) => sum + (entry.toolCalls?.reduce((toolSum, call) => toolSum
    + estimateTokens(call.tool) + estimateTokens(call.input ?? "") + estimateTokens(call.output ?? "") + estimateTokens(call.error ?? ""), 0) ?? 0), 0);
  const conversationTokens = estimateHistoryTokens(conversation);
  const inputTokens = Math.max(estimateTokens(message), message.trim() ? 80 : 0);
  const messageCount = conversation.length;
  const toolCount = conversation.reduce((sum, entry) => sum + (entry.toolCalls?.length ?? 0), 0);
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

export { estimateTokens } from "./aiChatContextCompaction";

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


