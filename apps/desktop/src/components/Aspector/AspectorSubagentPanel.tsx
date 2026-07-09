import { Loader2, Square, Network } from "lucide-react";
import { useEffect, useMemo, useState, type CSSProperties } from "react";
import {
  cancelSubagentRun,
  listSubagentRunsForSession,
  subscribeSubagentRuns,
  type SubagentRun,
} from "../../lib/aspector/subagents/runs";
import { resolveMaxParallelSubagents } from "../../lib/aspector/subagents/policy";
import { useLuxStore } from "../../lib/store/index";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AspectorSubagentPanelProps = {
  sessionId: string;
  t: TranslateFn;
};

type SubagentTreeNode = {
  run: SubagentRun;
  children: SubagentTreeNode[];
};

export function AspectorSubagentPanel({ sessionId, t }: AspectorSubagentPanelProps) {
  const maxParallel = resolveMaxParallelSubagents(useLuxStore((state) => state.aiPreferences));
  const [, setTick] = useState(0);
  useEffect(() => subscribeSubagentRuns(() => { setTick((value) => value + 1); }), []);

  const runs = listSubagentRunsForSession(sessionId).filter(
    (run) => run.status === "running" || Date.now() - (run.endedAt ?? run.startedAt) < 120_000,
  );
  const tree = useMemo(() => buildSubagentTree(runs), [runs]);
  const runningCount = runs.filter((run) => run.status === "running").length;

  if (runs.length === 0) return null;

  return (
    <div className="ai-subagent-panel" aria-label={t("aiChat.subagents.aria")}>
      <header>
        <Network size={13} />
        <strong>{t("aiChat.subagents.title")}</strong>
        <span>{t("aiChat.subagents.running", { count: runningCount })}</span>
        <span className="ai-subagent-panel-limit">{runningCount}/{maxParallel}</span>
      </header>
      <ul className="ai-subagent-tree">
        {tree.map((node) => (
          <SubagentTreeRow key={node.run.id} node={node} depth={0} t={t} />
        ))}
      </ul>
    </div>
  );
}

function SubagentTreeRow({ node, depth, t }: { node: SubagentTreeNode; depth: number; t: TranslateFn }) {
  const { run } = node;
  return (
    <li data-status={run.status} style={{ "--subagent-depth": depth } as CSSProperties}>
      <div className="ai-subagent-panel-row">
        <span className="ai-subagent-panel-type">{run.subagentType}</span>
        <span className="ai-subagent-panel-desc" title={run.description}>{run.description}</span>
        <span className="ai-subagent-panel-depth">{t("aiChat.subagents.depth", { depth: run.depth })}</span>
        {run.status === "running" ? (
          <button type="button" className="ai-subagent-cancel" title={t("aiChat.subagents.cancel")} onClick={() => cancelSubagentRun(run.id)}>
            <Square size={11} />
            <span>{t("aiChat.subagents.cancel")}</span>
          </button>
        ) : (
          <span className="ai-subagent-panel-status">{t(`aiChat.subagents.status.${run.status}` as "aiChat.subagents.status.completed")}</span>
        )}
      </div>
      {run.status === "running" && <Loader2 size={12} className="spin-icon" aria-hidden="true" />}
      {node.children.length > 0 && (
        <ul>
          {node.children.map((child) => (
            <SubagentTreeRow key={child.run.id} node={child} depth={depth + 1} t={t} />
          ))}
        </ul>
      )}
    </li>
  );
}

function buildSubagentTree(runs: SubagentRun[]): SubagentTreeNode[] {
  const byId = new Map(runs.map((run) => [run.id, run]));
  const childrenByParent = new Map<string | null, SubagentRun[]>();
  for (const run of runs) {
    const parentKey = run.parentAgentId && byId.has(run.parentAgentId) ? run.parentAgentId : null;
    const bucket = childrenByParent.get(parentKey) ?? [];
    bucket.push(run);
    childrenByParent.set(parentKey, bucket);
  }
  const sortRuns = (left: SubagentRun, right: SubagentRun) => right.startedAt - left.startedAt;
  const toNode = (run: SubagentRun): SubagentTreeNode => ({
    run,
    children: (childrenByParent.get(run.id) ?? []).sort(sortRuns).map(toNode),
  });
  return (childrenByParent.get(null) ?? []).sort(sortRuns).map(toNode);
}