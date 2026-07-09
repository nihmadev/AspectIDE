import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  Activity,
  AlertTriangle,
  BookOpen,
  Check,
  ChevronRight,
  Code2,
  Eye,
  FileSearch,
  FileText,
  FolderTree,
  GitBranch,
  Globe,
  Layers,
  Loader2,
  Minus,
  MapPin,
  MessageCircleQuestion,
  Network,
  Pencil,
  Search,
  Server,
  Shield,
  Sparkles,
  Target,
  Terminal,
  Trash2,
  Wrench,
  Zap,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { AiToolApprovalDecision, AiToolApprovalState } from "../../lib/aspector/chat/types";
import { extractReviewPathFromToolInput, requestFileReviewFocus } from "../../lib/aspector/utils/file-review/bridge";
import { getAiShellLiveOutput, getAiShellLiveOutputVersion, subscribeAiShellLiveOutput } from "../../lib/aspector/utils/shell-live-output";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

/** How long a manual expand/collapse of a tool-call group survives before the
 *  panel reverts to auto-follow (open while active, closed when idle). Long enough
 *  to read tool output without the next streaming status flip snapping it shut. */
const MANUAL_DISCLOSURE_HOLD_MS = 20_000;

/**
 * Disclosure state for streaming tool-call groups. By default `open` tracks
 * `active` (auto-expand while work runs, auto-collapse when done). A manual
 * toggle PINS the user's choice and holds it for ~20s of no further interaction,
 * then releases back to auto-follow — so a status flip mid-read never collapses
 * what the user just opened. Each toggle re-arms the hold timer.
 */
function useStickyDisclosure(active: boolean) {
  const [override, setOverride] = useState<boolean | null>(null);
  const timerRef = useRef<number | null>(null);

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const toggle = useCallback(() => {
    setOverride((current) => {
      const base = current ?? active;
      return !base;
    });
    clearTimer();
    timerRef.current = window.setTimeout(() => {
      setOverride(null);
      timerRef.current = null;
    }, MANUAL_DISCLOSURE_HOLD_MS);
  }, [active, clearTimer]);

  useEffect(() => clearTimer, [clearTimer]);

  return { open: override ?? active, toggle };
}

export type ToolCallStatus = "approval" | "running" | "success" | "skipped" | "error";

export type ToolCall = {
  id: string;
  tool: string;
  status: ToolCallStatus;
  input?: string;
  output?: string;
  error?: string;
  approval?: AiToolApprovalState;
  startTime: number;
  endTime?: number;
  stats?: {
    linesAdded?: number;
    linesRemoved?: number;
    filesChanged?: number;
    filesCreated?: number;
    filesDeleted?: number;
  };
};

const toolIcons: Record<string, LucideIcon> = {
  SemanticSearch: Search,
  Grep: FileSearch,
  Glob: FolderTree,
  Read: Eye,
  InspectFile: FileSearch,
  Write: FileText,
  StrReplace: Pencil,
  PatchEngine: Wrench,
  Checkpoint: Shield,
  Delete: Trash2,
  Shell: Terminal,
  TerminalContext: Terminal,
  TerminalWrite: Terminal,
  SshConnect: Server,
  SshExec: Server,
  SshTransfer: Server,
  SshList: Server,
  SshDisconnect: Server,
  ReadLints: AlertTriangle,
  TodoWrite: Layers,
  Goal: Target,
  Task: Network,
  AgentMessage: Network,
  AskUser: MessageCircleQuestion,
  PresentPlan: Sparkles,
  McpManage: Server,
  WebFetch: Network,
  BrowserStatus: Globe,
  BrowserOpen: Globe,
  BrowserSnapshot: Eye,
  BrowserAct: Globe,
  BrowserInvoke: Terminal,
  BrowserScreenshot: Eye,
  BrowserClose: Globe,
  BrowserChat: Globe,
  BrowserDashboard: Network,
  BrowserInstall: Wrench,
  BrowserHelp: BookOpen,
  BrowserDoctor: AlertTriangle,
  FastContext: Zap,
  RepoMap: FolderTree,
  WorkspaceIndex: FolderTree,
  ActiveContext: Eye,
  RulesContext: FileText,
  DocsContext: BookOpen,
  ImpactAnalysis: AlertTriangle,
  ReviewDiff: Eye,
  SecretGuard: AlertTriangle,
  SymbolContext: Code2,
  RelatedFiles: Layers,
  FailureAnalyzer: Activity,
  GitContext: GitBranch,
  DiagnosticsContext: AlertTriangle,
  TestHealth: Check,
  MemoryContext: BookOpen,
  ContextBudgeter: BookOpen,
  default: Wrench,
};

type AspectorToolCallProps = {
  onApprovalDecision?: (approvalId: string, decision: AiToolApprovalDecision) => void;
  t: TranslateFn;
  toolCall: ToolCall;
};

function isFileEditTool(tool: string) {
  return tool === "Write" || tool === "StrReplace" || tool === "PatchEngine";
}

function StatusGlyph({ status, Icon }: { status: ToolCallStatus; Icon: LucideIcon }) {
  if (status === "running") return <Loader2 size={13} className="spin-icon" />;
  if (status === "approval") return <Shield size={13} />;
  if (status === "error") return <AlertTriangle size={13} />;
  if (status === "skipped") return <Minus size={13} />;
  return <Icon size={13} />;
}

export function AspectorToolCall({ onApprovalDecision, t, toolCall }: AspectorToolCallProps) {
  const Icon = toolIcons[toolCall.tool] || toolIcons.default;
  const duration = toolCall.endTime ? toolCall.endTime - toolCall.startTime : Date.now() - toolCall.startTime;
  const durationText = duration < 1000 ? t("aiTools.duration.ms", { duration }) : t("aiTools.duration.s", { duration: (duration / 1000).toFixed(1) });
  const isApproval = toolCall.status === "approval";
  // Live tail for a RUNNING Shell: the Rust mirror streams output keyed by tool-call
  // id, so the row is expandable while the command runs — not only after completion.
  const isRunning = toolCall.status === "running";
  useSyncExternalStore(subscribeAiShellLiveOutput, getAiShellLiveOutputVersion, getAiShellLiveOutputVersion);
  const liveOutput = isRunning ? getAiShellLiveOutput(toolCall.id) : "";
  const finalDetail = toolCall.status === "error" ? toolCall.error : toolCall.status === "skipped" ? toolCall.error : toolCall.output;
  const detail = finalDetail?.trim() ? finalDetail : liveOutput;
  const isLiveDetail = Boolean(isRunning && !finalDetail?.trim() && liveOutput);
  const hasDetail = Boolean(detail && detail.trim());
  const collapsible = !isApproval && hasDetail;
  const [expanded, setExpanded] = useState(false);
  // Auto-follow: keep the live tail pinned to the newest output while expanded.
  const liveTailRef = useRef<HTMLPreElement | null>(null);
  useEffect(() => {
    if (!expanded || !isLiveDetail) return;
    const node = liveTailRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [expanded, isLiveDetail, liveOutput]);

  const stats = toolCall.stats;
  const hasStats = Boolean(stats && (stats.linesAdded || stats.linesRemoved || stats.filesChanged || stats.filesCreated || stats.filesDeleted));
  const reviewPath = isFileEditTool(toolCall.tool) ? extractReviewPathFromToolInput(toolCall.tool, toolCall.input) : null;
  const canJumpToReview = Boolean(reviewPath && (toolCall.status === "success" || toolCall.status === "running"));

  return (
    <motion.div
      className="ai-tool-call"
      data-status={toolCall.status}
      data-open={collapsible && expanded ? true : undefined}
      initial={{ opacity: 0, y: 2 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.16 }}
    >
      <button
        type="button"
        className="ai-tool-call-row"
        data-interactive={collapsible || undefined}
        onClick={collapsible ? () => setExpanded((value) => !value) : undefined}
        aria-expanded={collapsible ? expanded : undefined}
      >
        <span className="ai-tool-call-glyph">
          <StatusGlyph status={toolCall.status} Icon={Icon} />
        </span>
        <span className="ai-tool-call-name">{toolCall.tool}</span>
        {toolCall.input && <span className="ai-tool-call-target">{toolCall.input}</span>}
        <span className="ai-tool-call-flex" />
        {hasStats && stats && (
          <span className="ai-tool-call-stats">
            {stats.linesAdded ? <span className="ai-tool-stat" data-type="added">+{stats.linesAdded}</span> : null}
            {stats.linesRemoved ? <span className="ai-tool-stat" data-type="removed">-{stats.linesRemoved}</span> : null}
            {stats.filesCreated ? <span className="ai-tool-stat" data-type="created">{stats.filesCreated} new</span> : null}
            {stats.filesChanged ? <span className="ai-tool-stat" data-type="changed">{stats.filesChanged} changed</span> : null}
            {stats.filesDeleted ? <span className="ai-tool-stat" data-type="deleted">{stats.filesDeleted} deleted</span> : null}
          </span>
        )}
        {canJumpToReview && reviewPath && (
          <button
            type="button"
            className="ai-tool-call-jump"
            title={t("aiChat.review.jumpToChange")}
            onClick={(event) => {
              event.stopPropagation();
              requestFileReviewFocus({ path: reviewPath, toolCallId: toolCall.id });
            }}
          >
            <MapPin size={12} />
          </button>
        )}
        {!isApproval && toolCall.status !== "running" && <span className="ai-tool-call-duration">{durationText}</span>}
        {collapsible && <ChevronRight className="ai-tool-call-caret" data-expanded={expanded} size={13} />}
      </button>

      {isApproval && toolCall.approval && (
        <div className="ai-tool-approval" data-risk={toolCall.approval.risk}>
          <div className="ai-tool-approval-head">
            <div>
              <strong>{toolCall.approval.title}</strong>
              <span>{toolCall.approval.path}</span>
            </div>
          </div>
          <p>{toolCall.approval.summary}</p>
          {toolCall.approval.preview && <pre>{toolCall.approval.preview}</pre>}
          <div className="ai-tool-approval-actions">
            <button type="button" className="ai-tool-approval-reject" onClick={() => onApprovalDecision?.(toolCall.approval!.id, "rejected")}>{toolCall.approval.rejectLabel}</button>
            <button type="button" className="ai-tool-approval-approve" data-risk={toolCall.approval.risk} onClick={() => onApprovalDecision?.(toolCall.approval!.id, "approved")}>{toolCall.approval.approveLabel}</button>
          </div>
        </div>
      )}

      <AnimatePresence initial={false}>
        {collapsible && expanded && (
          <motion.div
            className="ai-tool-call-body"
            data-kind={toolCall.status}
            data-live={isLiveDetail || undefined}
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
          >
            <pre ref={liveTailRef}>{detail}</pre>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}

type AspectorToolCallsGroupProps = {
  onApprovalDecision?: (approvalId: string, decision: AiToolApprovalDecision) => void;
  t: TranslateFn;
  toolCalls: ToolCall[];
};

export function AspectorToolCallsGroup({ onApprovalDecision, t, toolCalls }: AspectorToolCallsGroupProps) {
  const approvalCount = toolCalls.filter((call) => call.status === "approval").length;
  const runningCount = toolCalls.filter((call) => call.status === "running").length;
  const errorCount = toolCalls.filter((call) => call.status === "error").length;
  const active = approvalCount > 0 || runningCount > 0;
  const { open, toggle } = useStickyDisclosure(active);
  const groupedBatches = useMemo(() => groupToolCalls(toolCalls), [toolCalls]);

  if (toolCalls.length === 0) return null;

  const summary = approvalCount > 0
    ? t("aiTools.summary.waitingApproval", { count: approvalCount })
    : runningCount > 0
      ? t("aiTools.summary.running", { count: runningCount })
      : t("aiTools.summary.ran", { count: toolCalls.length });

  return (
    <div className="ai-tool-calls-group" data-active={active || undefined} data-open={open || undefined}>
      <button type="button" className="ai-tool-calls-summary" onClick={toggle} aria-expanded={open}>
        <span className="ai-tool-calls-rail" aria-hidden="true" />
        <span className="ai-tool-calls-summary-label">{summary}</span>
        {groupedBatches.length > 1 && <span className="ai-tool-calls-badge" data-status="neutral">{t("aiTools.summary.groups", { count: groupedBatches.length })}</span>}
        {errorCount > 0 && <span className="ai-tool-calls-badge" data-status="error">{t("aiTools.summary.failed", { count: errorCount })}</span>}
        <ChevronRight className="ai-tool-calls-caret" data-expanded={open} size={13} />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            className="ai-tool-calls-list"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
          >
            {groupedBatches.map((batch) => (
              <ToolCallBatch key={batch.toolCalls[0]?.id ?? batch.id} batch={batch} onApprovalDecision={onApprovalDecision} t={t} />
            ))}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

type ToolCallBatchModel = {
  id: string;
  tool: string | null;
  toolCalls: ToolCall[];
};

function ToolCallBatch({ batch, onApprovalDecision, t }: { batch: ToolCallBatchModel; onApprovalDecision?: (approvalId: string, decision: AiToolApprovalDecision) => void; t: TranslateFn }) {
  const active = batch.toolCalls.some((call) => call.status === "approval" || call.status === "running");
  const collapsible = batch.toolCalls.length > 2;
  const { open: stickyOpen, toggle } = useStickyDisclosure(active);
  const open = collapsible ? stickyOpen : true;

  return (
    <div className="ai-tool-call-batch" data-open={open || undefined} data-active={active || undefined}>
      {collapsible && (
        <button type="button" className="ai-tool-call-batch-head" onClick={toggle} aria-expanded={open}>
          <ChevronRight className="ai-tool-call-batch-caret" data-expanded={open} size={13} />
          <span>{batch.tool ?? t("aiTools.summary.mixedGroup")}</span>
          <strong>{batch.toolCalls.length}</strong>
        </button>
      )}
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            className="ai-tool-call-batch-list"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
          >
            {batch.toolCalls.map((toolCall) => (
              <AspectorToolCall key={toolCall.id} onApprovalDecision={onApprovalDecision} t={t} toolCall={toolCall} />
            ))}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function groupToolCalls(toolCalls: ToolCall[]): ToolCallBatchModel[] {
  const batches: ToolCallBatchModel[] = [];
  let mixed: ToolCall[] = [];
  const flush = () => {
    if (mixed.length === 0) return;
    const tool = mixed[0]?.tool ?? null;
    batches.push({
      id: mixed.map((call) => call.id).join("-"),
      tool: mixed.every((call) => call.tool === tool) ? tool : null,
      toolCalls: mixed,
    });
    mixed = [];
  };

  for (let index = 0; index < toolCalls.length;) {
    const tool = toolCalls[index]?.tool;
    const run: ToolCall[] = [];
    while (index < toolCalls.length && toolCalls[index]?.tool === tool) {
      run.push(toolCalls[index]);
      index += 1;
    }

    if (run.length >= 3) {
      flush();
      mixed = run;
      flush();
      continue;
    }

    mixed.push(...run);
    if (mixed.length >= 8) flush();
  }
  flush();
  return batches;
}

// Example usage in a message:
export type AspectorMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  toolCalls?: ToolCall[];
  timestamp: number;
};
