import { FileCode2, Network, Wrench } from "lucide-react";
import { useSyncExternalStore } from "react";
import { getAiTurnActivity, getAiTurnActivitySnapshot, subscribeAiTurnActivity } from "../../lib/aiTurnActivity";
import { listSubagentRunsForSession, subscribeSubagentRuns } from "../../lib/aiSubagentRuns";
import { aiChatStatusLabel } from "../../lib/aiChatPresentation";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { AiChatSessionStatus } from "../../lib/store";

type AiAgentNowPlaqueProps = {
  sessionId: string;
  status: AiChatSessionStatus;
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
export function AiAgentNowPlaque({ sessionId, status, t }: AiAgentNowPlaqueProps) {
  useSyncExternalStore(subscribeAiTurnActivity, getAiTurnActivitySnapshot, getAiTurnActivitySnapshot);
  useSyncExternalStore(subscribeSubagentRuns, () => 0, () => 0);

  const activity = getAiTurnActivity(sessionId);
  const runningSubagent = listSubagentRunsForSession(sessionId).find((run) => run.status === "running");

  const subagentText = runningSubagent
    ? `${runningSubagent.subagentType}: ${runningSubagent.description}`
    : activity.subagentLabel;

  return (
    <div className="ai-thinking-indicator ai-agent-now-plaque" data-status={status} role="status" aria-live="polite">
      <span />
      <span />
      <span />
      <strong>{aiChatStatusLabel(status, true, t)}</strong>
      {subagentText ? (
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
      {!subagentText && activity.filePath && (
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
