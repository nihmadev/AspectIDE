import { FileCode2, Loader2, Network, Wrench } from "lucide-react";
import { useSyncExternalStore } from "react";
import { getAiTurnActivity, getAiTurnActivitySnapshot, subscribeAiTurnActivity } from "../../lib/aiTurnActivity";
import { listSubagentRunsForSession, subscribeSubagentRuns } from "../../lib/aiSubagentRuns";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { AiChatSessionStatus } from "../../lib/store";

type AiAgentNowBarProps = {
  sessionId: string;
  sessionStatus: AiChatSessionStatus;
  t: TranslateFn;
};

export function AiAgentNowBar({ sessionId, sessionStatus, t }: AiAgentNowBarProps) {
  useSyncExternalStore(subscribeAiTurnActivity, getAiTurnActivitySnapshot, getAiTurnActivitySnapshot);
  useSyncExternalStore(subscribeSubagentRuns, () => 0, () => 0);

  const activity = getAiTurnActivity(sessionId);
  const runningSubagent = listSubagentRunsForSession(sessionId).find((run) => run.status === "running");
  const busy = sessionStatus !== "idle" && sessionStatus !== "error";
  const phase = runningSubagent ? "subagent" : activity.phase !== "idle" ? activity.phase : busy ? mapSessionStatus(sessionStatus) : "idle";

  if (phase === "idle") return null;

  const label = runningSubagent
    ? `${runningSubagent.subagentType}: ${runningSubagent.description}`
    : activity.subagentLabel
      ?? activity.toolName
      ?? t(`aiChat.orchestration.nowPhase.${phase}` as "aiChat.orchestration.nowPhase.thinking");

  const detail = runningSubagent
    ? null
    : activity.filePath ?? (activity.toolName ? t("aiChat.orchestration.nowTool", { tool: activity.toolName }) : null);

  return (
    <div className="ai-agent-now-bar" data-phase={phase}>
      <Loader2 size={12} className="spin-icon" aria-hidden="true" />
      {runningSubagent || activity.subagentLabel ? <Network size={12} aria-hidden="true" /> : activity.toolName ? <Wrench size={12} aria-hidden="true" /> : null}
      <span className="ai-agent-now-label" title={label}>{label}</span>
      {detail && (
        <span className="ai-agent-now-detail" title={detail}>
          <FileCode2 size={11} aria-hidden="true" />
          {basename(detail)}
        </span>
      )}
    </div>
  );
}

function mapSessionStatus(status: AiChatSessionStatus) {
  if (status === "waiting-approval") return "waiting-approval";
  if (status === "running-tools") return "running-tools";
  if (status === "preparing") return "preparing";
  if (status === "streaming") return "streaming";
  return "thinking";
}

function basename(path: string) {
  const normalized = path.replace(/\\/g, "/");
  const parts = normalized.split("/");
  return parts[parts.length - 1] || path;
}