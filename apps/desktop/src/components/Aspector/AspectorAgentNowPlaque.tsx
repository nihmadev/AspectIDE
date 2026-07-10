import { FileCode2, Network, Wrench } from "lucide-react";
import { useSyncExternalStore } from "react";
import { getAiTurnActivity, getAiTurnActivitySnapshot, subscribeAiTurnActivity } from '../../lib/aspector/utils/turn-activity';
import { listSubagentRunsForSession, subscribeSubagentRuns } from '../../lib/aspector/subagents/runs';
import { aiChatStatusLabel } from '../../lib/aspector/chat/presentation';
import type { TranslateFn } from '../../lib/i18n/useTranslation';
import type { AiChatSessionStatus } from '../../lib/store/index';

type AspectorAgentNowPlaqueProps = {
  sessionId: string;
  status: AiChatSessionStatus;
  /** Live single-line tail of the agent's raw output (streaming reasoning/text),
   *  shown flowing to the right while it works. Empty = ticker off/nothing yet. */
  workTail?: string;
  t: TranslateFn;
};

/**
 * Bottom-of-thread live status plaque: the classic pulsing-dots "Thinking…/
 * Writing…" indicator, upgraded to narrate the WHOLE turn — the current verb
 * plus what the agent is actually touching (tool, file, or running subagent).
 * Stays up for the full busy period (the caller gates on session busy);
 * duplicates the island's now-bar on purpose — one lives with the conversation,
 * one with the Goal/Tasks overview.
 */
export function AspectorAgentNowPlaque({ sessionId, status, workTail, t }: AspectorAgentNowPlaqueProps) {
  useSyncExternalStore(subscribeAiTurnActivity, getAiTurnActivitySnapshot, getAiTurnActivitySnapshot);
  useSyncExternalStore(subscribeSubagentRuns, () => 0, () => 0);

  const activity = getAiTurnActivity(sessionId);
  const runningSubagent = listSubagentRunsForSession(sessionId).find((run) => run.status === "running");

  const subagentText = runningSubagent
    ? `${runningSubagent.subagentType}: ${runningSubagent.description}`
    : activity.subagentLabel;
  // The raw-work ticker replaces the tool/file chips while there's live output to
  // show — one calm line beats a chip row plus a duplicate word. Chips remain the
  // fallback when the ticker is off or before any tokens arrive.
  const tail = workTail?.trim();

  return (
    <div className="ai-thinking-indicator ai-agent-now-plaque" data-status={status} data-ticker={tail ? "true" : undefined} role="status" aria-live="polite">
      <span />
      <span />
      <span />
      <strong>{aiChatStatusLabel(status, true, t)}</strong>
      {tail ? (
        <span className="ai-agent-now-plaque-ticker" title={tail}>{tail}</span>
      ) : subagentText ? (
        <span className="ai-agent-now-plaque-chip" title={subagentText}>
          <Network size={11} aria-hidden="true" />
          <span>{subagentText}</span>
        </span>
      ) : activity.toolName ? (
        <span className="ai-agent-now-plaque-chip" title={activity.toolName}>
          <Wrench size={11} aria-hidden="true" />
          <span>{activity.toolName}</span>
        </span>
      ) : null}
      {!tail && !subagentText && activity.filePath && (
        <span className="ai-agent-now-plaque-chip" title={activity.filePath}>
          <FileCode2 size={11} aria-hidden="true" />
          <span>{basename(activity.filePath)}</span>
        </span>
      )}
    </div>
  );
}

function basename(path: string) {
  const normalized = path.replace(/\\/g, "/");
  const parts = normalized.split("/");
  return parts[parts.length - 1] || path;
}
