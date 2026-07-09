import { aiChatStatusLabel } from "../../lib/aspector/chat/presentation";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { AiChatSessionStatus } from "../../lib/store/index";

type AspectorThinkingIndicatorProps = {
  status: AiChatSessionStatus;
  t: TranslateFn;
  compact?: boolean;
};

export function AspectorThinkingIndicator({ status, t, compact = false }: AspectorThinkingIndicatorProps) {
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