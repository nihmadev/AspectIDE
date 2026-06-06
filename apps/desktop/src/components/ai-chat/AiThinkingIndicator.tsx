import { aiChatStatusLabel } from "../../lib/aiChatPresentation";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { AiChatSessionStatus } from "../../lib/store";

type AiThinkingIndicatorProps = {
  status: AiChatSessionStatus;
  t: TranslateFn;
  compact?: boolean;
};

export function AiThinkingIndicator({ status, t, compact = false }: AiThinkingIndicatorProps) {
  return (
    <div className="ai-thinking-indicator" data-status={status} data-compact={compact || undefined}>
      <span />
      <span />
      <span />
      <strong>{aiChatStatusLabel(status, true, t)}</strong>
    </div>
  );
}

export function isPendingAssistantShell(message: { role: string; content: string; reasoning?: string; toolCalls?: unknown[]; segments?: unknown[] }, streaming: boolean) {
  if (!streaming || message.role !== "assistant") return false;
  const content = message.content.trim();
  const reasoning = message.reasoning?.trim() ?? "";
  const toolCount = message.toolCalls?.length ?? 0;
  const segmentCount = message.segments?.length ?? 0;
  return !content && !reasoning && toolCount === 0 && segmentCount === 0;
}