import type { AiPreferences } from "./aiPreferences";
import type { AiChatAttachmentInput, AiChatMessage } from "./aiChatTypes";
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

type BuildContextUsageInput = {
  activeDocumentPath: string | null;
  aiIndexStatus: string;
  agentInstruction: string;
  agentName: string;
  attachments: AiChatContextAttachment[];
  conversation: AiChatMessage[];
  message: string;
  preferences: AiPreferences;
  selectedModelAlias: string;
  t: TranslateFn;
};

const contextTokenBudget = 200_000;

const contextRowColors = {
  agent: "#9aa0a6",
  model: "#8fb5d9",
  index: "#57c178",
  files: "#ffc46b",
  conversation: "#6d8589",
} as const;

export function buildAiChatContextUsageSummary({
  activeDocumentPath,
  aiIndexStatus,
  agentInstruction,
  agentName,
  attachments,
  conversation,
  message,
  preferences,
  selectedModelAlias,
  t,
}: BuildContextUsageInput): AiChatContextUsageSummary {
  const agentTokens = estimateTokens([agentName, preferences.agentMode, agentInstruction].join(" "));
  const modelTokens = estimateTokens(selectedModelAlias);
  const indexTokens = preferences.projectIndexingEnabled && aiIndexStatus !== "disabled" ? estimateTokens(aiIndexStatus) : 0;
  const filesTokens = activeDocumentPath ? estimateTokens(activeDocumentPath) : 0;
  const historyTokens = conversation.reduce((sum, entry) => {
    const toolTokens = entry.toolCalls?.reduce((toolSum, call) => toolSum
      + estimateTokens(call.input ?? "") + estimateTokens(call.output ?? "") + estimateTokens(call.error ?? ""), 0) ?? 0;
    return sum + estimateTokens(entry.content) + toolTokens;
  }, 0);
  const conversationTokens = historyTokens
    + Math.max(estimateTokens(message), message.trim() ? 80 : 0)
    + attachments.reduce((sum, attachment) => sum + estimateAttachmentTokens(attachment), 0);
  const messageCount = conversation.length;
  const conversationDetail = messageCount > 0
    ? t("aiChat.context.messageCount", { count: messageCount })
    : attachments.length > 0 ? t("aiChat.attachment.count", { count: attachments.length }) : "";
  const rawRows: Omit<AiChatContextUsageRow, "percent">[] = [
    { color: contextRowColors.agent, detail: agentName || preferences.agentMode, id: "agent", label: t("aiChat.context.agent"), tokens: agentTokens },
    { color: contextRowColors.model, detail: selectedModelAlias, id: "model", label: t("aiChat.model.label"), tokens: modelTokens },
    { color: contextRowColors.index, detail: preferences.projectIndexingEnabled ? aiIndexStatus : t("common.off"), id: "index", label: t("aiChat.context.index"), tokens: indexTokens },
    { color: contextRowColors.files, detail: activeDocumentPath ?? "", id: "files", label: t("aiChat.context.file"), tokens: filesTokens },
    { color: contextRowColors.conversation, detail: conversationDetail, id: "conversation", label: t("aiChat.context.message"), tokens: conversationTokens },
  ].filter((row) => row.tokens > 0 || row.detail);
  const totalTokens = rawRows.reduce((sum, row) => sum + row.tokens, 0);
  const rows = rawRows.map((row) => ({
    ...row,
    percent: totalTokens > 0 ? Math.max(0.8, (row.tokens / totalTokens) * 100) : 0,
  }));

  return {
    percent: Math.min(100, Math.round((totalTokens / contextTokenBudget) * 100)),
    rows,
    tokenBudget: contextTokenBudget,
    totalTokens,
  };
}

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

function estimateTokens(value: string) {
  const trimmed = value.trim();
  if (!trimmed) return 0;
  return Math.ceil(trimmed.length / 4);
}
