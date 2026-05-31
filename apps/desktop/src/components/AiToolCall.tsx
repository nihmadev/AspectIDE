import { motion } from "motion/react";
import {
  Activity,
  AlertTriangle,
  BookOpen,
  CheckCircle2,
  Code2,
  Eye,
  FileSearch,
  FileText,
  FolderTree,
  GitBranch,
  Layers,
  Loader2,
  Network,
  Pencil,
  Search,
  Shield,
  Terminal,
  Trash2,
  Wrench,
  Zap,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { AiToolApprovalDecision, AiToolApprovalState } from "../lib/aiChatRuntime";

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
  Write: FileText,
  StrReplace: Pencil,
  PatchEngine: Wrench,
  Checkpoint: Shield,
  Delete: Trash2,
  Shell: Terminal,
  ReadLints: AlertTriangle,
  TodoWrite: Layers,
  WebFetch: Network,
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
  TestHealth: CheckCircle2,
  MemoryContext: BookOpen,
  ContextBudgeter: BookOpen,
  default: Wrench,
};

const toolColors: Record<string, string> = {
  SemanticSearch: "#3b9eff",
  Grep: "#3b9eff",
  Glob: "#3b9eff",
  Read: "#3b9eff",
  Write: "#3b9eff",
  StrReplace: "#3b9eff",
  PatchEngine: "#4ec98a",
  Checkpoint: "#4ec98a",
  Delete: "#f14c4c",
  Shell: "#e2b341",
  ReadLints: "#3b9eff",
  TodoWrite: "#3b9eff",
  WebFetch: "#3b9eff",
  FastContext: "#4ec98a",
  RepoMap: "#4ec98a",
  WorkspaceIndex: "#4ec98a",
  ActiveContext: "#4ec98a",
  RulesContext: "#4ec98a",
  DocsContext: "#4ec98a",
  ImpactAnalysis: "#4ec98a",
  ReviewDiff: "#4ec98a",
  SecretGuard: "#4ec98a",
  SymbolContext: "#4ec98a",
  RelatedFiles: "#4ec98a",
  FailureAnalyzer: "#4ec98a",
  GitContext: "#4ec98a",
  DiagnosticsContext: "#4ec98a",
  TestHealth: "#4ec98a",
  MemoryContext: "#4ec98a",
  ContextBudgeter: "#4ec98a",
  default: "#8a8a8a",
};

type AiToolCallProps = {
  onApprovalDecision?: (approvalId: string, decision: AiToolApprovalDecision) => void;
  toolCall: ToolCall;
};

export function AiToolCall({ onApprovalDecision, toolCall }: AiToolCallProps) {
  const Icon = toolIcons[toolCall.tool] || toolIcons.default;
  const color = toolColors[toolCall.tool] || toolColors.default;
  const duration = toolCall.endTime ? toolCall.endTime - toolCall.startTime : Date.now() - toolCall.startTime;
  const durationText = duration < 1000 ? `${duration}ms` : `${(duration / 1000).toFixed(1)}s`;

  const hasStats = toolCall.stats && (
    toolCall.stats.linesAdded ||
    toolCall.stats.linesRemoved ||
    toolCall.stats.filesChanged ||
    toolCall.stats.filesCreated ||
    toolCall.stats.filesDeleted
  );

  return (
    <motion.div
      className="ai-tool-call"
      data-status={toolCall.status}
      initial={{ opacity: 0, y: 4, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={{ duration: 0.2 }}
    >
      <div className="ai-tool-call-header">
        <div className="ai-tool-call-icon" style={{ color }}>
          {toolCall.status === "running" || toolCall.status === "approval" ? (
            <Loader2 size={14} className="spin-icon" />
          ) : (
            <Icon size={14} />
          )}
        </div>
        <div className="ai-tool-call-main">
          <div className="ai-tool-call-title">
            <span>{toolCall.tool}</span>
            {toolCall.status !== "running" && toolCall.status !== "approval" && (
              <span className="ai-tool-call-duration">{durationText}</span>
            )}
            {toolCall.status === "approval" && (
              <span className="ai-tool-call-duration">approval</span>
            )}
          </div>
          {toolCall.input && (
            <div className="ai-tool-call-input">{toolCall.input}</div>
          )}
          {hasStats && toolCall.stats && (
            <div className="ai-tool-call-stats">
              {toolCall.stats.linesAdded !== undefined && toolCall.stats.linesAdded > 0 && (
                <span className="ai-tool-stat" data-type="added">+{toolCall.stats.linesAdded}</span>
              )}
              {toolCall.stats.linesRemoved !== undefined && toolCall.stats.linesRemoved > 0 && (
                <span className="ai-tool-stat" data-type="removed">-{toolCall.stats.linesRemoved}</span>
              )}
              {toolCall.stats.filesCreated !== undefined && toolCall.stats.filesCreated > 0 && (
                <span className="ai-tool-stat" data-type="created">{toolCall.stats.filesCreated} created</span>
              )}
              {toolCall.stats.filesChanged !== undefined && toolCall.stats.filesChanged > 0 && (
                <span className="ai-tool-stat" data-type="changed">{toolCall.stats.filesChanged} changed</span>
              )}
              {toolCall.stats.filesDeleted !== undefined && toolCall.stats.filesDeleted > 0 && (
                <span className="ai-tool-stat" data-type="deleted">{toolCall.stats.filesDeleted} deleted</span>
              )}
            </div>
          )}
        </div>
      </div>

      {toolCall.status === "approval" && toolCall.approval && (
        <motion.div
          className="ai-tool-approval"
          data-risk={toolCall.approval.risk}
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.2 }}
        >
          <div className="ai-tool-approval-head">
            <div>
              <strong>{toolCall.approval.title}</strong>
              <span>{toolCall.approval.path}</span>
            </div>
          </div>
          <p>{toolCall.approval.summary}</p>
          <pre>{toolCall.approval.preview}</pre>
          <div className="ai-tool-approval-actions">
            <button type="button" className="ai-tool-approval-reject" onClick={() => onApprovalDecision?.(toolCall.approval!.id, "rejected")}>{toolCall.approval.rejectLabel}</button>
            <button type="button" className="ai-tool-approval-approve" data-risk={toolCall.approval.risk} onClick={() => onApprovalDecision?.(toolCall.approval!.id, "approved")}>{toolCall.approval.approveLabel}</button>
          </div>
        </motion.div>
      )}

      {toolCall.status === "success" && toolCall.output && (
        <motion.div
          className="ai-tool-call-output"
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.2 }}
        >
          <pre>{toolCall.output}</pre>
        </motion.div>
      )}

      {toolCall.status === "error" && toolCall.error && (
        <motion.div
          className="ai-tool-call-error"
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.2 }}
        >
          {toolCall.error}
        </motion.div>
      )}

      {toolCall.status === "skipped" && toolCall.error && (
        <motion.div
          className="ai-tool-call-skipped"
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.2 }}
        >
          {toolCall.error}
        </motion.div>
      )}
    </motion.div>
  );
}

type AiToolCallsGroupProps = {
  onApprovalDecision?: (approvalId: string, decision: AiToolApprovalDecision) => void;
  toolCalls: ToolCall[];
};

export function AiToolCallsGroup({ onApprovalDecision, toolCalls }: AiToolCallsGroupProps) {
  if (toolCalls.length === 0) return null;

  const approvalCount = toolCalls.filter((t) => t.status === "approval").length;
  const runningCount = toolCalls.filter((t) => t.status === "running").length;
  const successCount = toolCalls.filter((t) => t.status === "success").length;
  const skippedCount = toolCalls.filter((t) => t.status === "skipped").length;
  const errorCount = toolCalls.filter((t) => t.status === "error").length;

  return (
    <div className="ai-tool-calls-group">
      <div className="ai-tool-calls-summary">
        <Wrench size={13} />
        <span>
          {approvalCount > 0 && `Waiting for ${approvalCount} approval${approvalCount > 1 ? "s" : ""}...`}
          {runningCount > 0 && `Running ${runningCount} tool${runningCount > 1 ? "s" : ""}...`}
          {approvalCount === 0 && runningCount === 0 && `Used ${toolCalls.length} tool${toolCalls.length > 1 ? "s" : ""}`}
        </span>
        {successCount > 0 && <span className="ai-tool-calls-badge" data-status="success">{successCount}</span>}
        {skippedCount > 0 && <span className="ai-tool-calls-badge" data-status="skipped">{skippedCount}</span>}
        {errorCount > 0 && <span className="ai-tool-calls-badge" data-status="error">{errorCount}</span>}
      </div>
      <div className="ai-tool-calls-list">
        {toolCalls.map((toolCall) => (
          <AiToolCall key={toolCall.id} onApprovalDecision={onApprovalDecision} toolCall={toolCall} />
        ))}
      </div>
    </div>
  );
}

// Example usage in a message:
export type AiMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  toolCalls?: ToolCall[];
  timestamp: number;
};
