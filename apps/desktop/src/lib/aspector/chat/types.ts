import type { AiChatContextBudgetReport } from "./context-report";
import type { AiModelConfig, AiPreferences, AiProviderConfig } from "./../utils/preferences";
import type { Locale } from "../../i18n";
import type { TerminalOutputBuffer } from "./../../terminal/types";
import type { DocumentSnapshot, TerminalSessionInfo, WorkspaceInfo } from "./../../types/index";

export type AiChatRole = "system" | "user" | "assistant" | "tool";

export type AiChatToolStatus = "approval" | "running" | "success" | "skipped" | "error";
/** "preparing" = the model has finished thinking and is constructing a tool call
 *  (Rust's "building-tools" phase, before toolCallStarted names the tool). Kept
 *  distinct from "thinking"/"streaming" so the status chip and reasoning shimmer
 *  never lie about there being live tokens once the model has moved on. */
export type AiChatRuntimeStatus = "thinking" | "streaming" | "running-tools" | "waiting-approval" | "preparing";

export type AiToolApprovalDecision = "approved" | "rejected";

export type AiToolApprovalRequest = {
  id: string;
  tool: "Write" | "StrReplace" | "Delete" | "Shell" | "TerminalWrite" | "PatchEngine" | "Checkpoint" | "BrowserOpen" | "BrowserAct" | "BrowserChat" | "BrowserDashboard" | "BrowserInstall" | "SshConnect" | "SshExec" | "SshTransfer";
  title: string;
  path: string;
  summary: string;
  preview: string;
  risk: "create" | "modify" | "delete" | "execute";
  approveLabel: string;
  rejectLabel: string;
};

export type AiToolApprovalState = AiToolApprovalRequest & {
  decision?: AiToolApprovalDecision;
};

export type AiChatToolCall = {
  id: string;
  tool: string;
  status: AiChatToolStatus;
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

export type AiChatResponseTiming = {
  totalMs: number;
  modelMs: number;
  toolMs: number;
  overheadMs: number;
  firstTokenMs: number | null;
  streamMs: number | null;
  modelCalls: number;
  toolCalls: number;
  rounds: number;
  streamed: boolean;
};

export type AiChatTurnTokenUsage = {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  estimatedCostUsd: number | null;
  /** Prompt tokens served from the provider's prompt cache (cache read), when reported. */
  cachedPromptTokens?: number;
  /** Model API requests the turn issued (loop rounds + recovery synthesis). */
  requestCount?: number;
};

// Assistant turns are rendered as an ordered timeline, so reasoning, text and
/** Structured payload of an inline turn notice (rendered as a plaque in the
 *  assistant timeline at the exact position the event happened). */
export type AiInlineNotice = {
  type: "reasoning-fallback";
  /** The reasoning effort the user configured (rejected by the provider). */
  requested: string;
  /** The strongest provider-accepted effort applied instead. */
  applied: string;
};

// tool calls keep their exact model order instead of being flattened.
export type AiMessageSegment =
  | { kind: "reasoning"; id: string; text: string }
  | { kind: "text"; id: string; text: string }
  | { kind: "tool"; id: string; toolCall: AiChatToolCall }
  /** Inline event plaque (e.g. reasoning-effort fallback). `text` is a plain
   *  one-line fallback for persistence/export; the UI renders `notice`. Never
   *  part of model-visible content (deriveSegmentContent filters by kind). */
  | { kind: "notice"; id: string; text: string; notice: AiInlineNotice };

export type AiChatMessageKind = "default" | "compaction-checkpoint" | "goal-orchestration" | "review-request";

export type AiChatMessageVisibility = "visible" | "internal";

export type AiChatMessageAttachment = {
  id: string;
  kind: "image" | "file";
  name: string;
  size: number;
  /** Data URL for inline chat preview (images only). */
  previewUrl?: string;
};

export type AiChatMessage = {
  id: string;
  role: "user" | "assistant";
  kind?: AiChatMessageKind;
  /** Internal orchestration turns (goal kickoff/continuation) stay in history but not in the chat UI. */
  visibility?: AiChatMessageVisibility;
  content: string;
  /** True for a user message staged as a "recommendation" while the agent was
   *  working (folded into the running turn, not a fresh turn). The UI renders a
   *  small "sent as recommendation" caption under it. */
  recommendation?: boolean;
  /** Snapshot taken before this user turn (files + prior messages). */
  turnCheckpointId?: string;
  attachments?: AiChatMessageAttachment[];
  reasoning?: string;
  toolCalls?: AiChatToolCall[];
  segments?: AiMessageSegment[];
  responseDurationMs?: number;
  responseTiming?: AiChatResponseTiming;
  turnUsage?: AiChatTurnTokenUsage;
  timestamp: number;
};

export type AiChatMentionHints = {
  codebase?: boolean;
  docs?: boolean;
};

/** Payload delivered to onRetryNotice when the turn auto-retries a transient failure. */
export type AiChatRetryNotice = {
  attempt: number;
  maxAttempts: number;
  reason: string;
  detail: string;
  delayMs: number;
};

export type AiChatAttachmentInput = {
  name: string;
  size: number;
  text: string;
  /** Data URL for vision-capable models when an image tab/file is pinned into chat. */
  visionImageUrl?: string;
  /** Video frame snapshots for vision-capable models. */
  visionFrameUrls?: string[];
};

export type AiChatTerminalContext = {
  activeTerminalId: string | null;
  outputBuffers: Record<string, TerminalOutputBuffer>;
  sessions: TerminalSessionInfo[];
};

export type AiChatSendInput = {
  abortSignal: AbortSignal;
  activeDocument: DocumentSnapshot | null;
  attachments: AiChatAttachmentInput[];
  mentionHints?: AiChatMentionHints;
  history: AiChatMessage[];
  locale: Locale;
  message: string;
  openDocuments: DocumentSnapshot[];
  preferences: AiPreferences;
  provider: AiProviderConfig;
  globalInstructions: string;
  projectInstructions: string;
  selectedAgentInstructions: string;
  selectedAgentName: string;
  selectedModel: AiModelConfig;
  terminal: TerminalSessionInfo | null;
  terminalContext: AiChatTerminalContext;
  workspace: WorkspaceInfo | null;
  chatSessionId: string;
  /** Nested Task/subagent execution context. */
  subagentContext?: {
    depth: number;
    parentAgentId: string | null;
  };
  /** Active user-turn snapshot used to extend file rollback coverage during tool writes. */
  turnCheckpoint?: {
    turnCheckpointId: string;
    fileCheckpointId: string;
  };
  onAssistantMessage: (message: AiChatMessage) => void;
  onAssistantMessageUpdate: (messageId: string, patch: Partial<AiChatMessage>) => void;
  onStatusChange?: (status: AiChatRuntimeStatus) => void;
  /** A user message was folded into the running turn mid-work (see ai_inject_message);
   *  render it as a user bubble in order, before the answer that follows. */
  onUserMessageInjected?: (text: string) => void;
  /** Live notice that a transient provider failure is being auto-retried. `null` clears it. */
  onRetryNotice?: (notice: AiChatRetryNotice | null) => void;
  onToolApproval: (request: AiToolApprovalRequest) => Promise<AiToolApprovalDecision>;
  onContextBudgetReport?: (report: AiChatContextBudgetReport) => void;
  onFilePathsEdited?: (paths: string[]) => void;
};

/**
 * A review-request turn. Its `content` carries the full review instruction sent to the
 * model, but the chat UI renders a compact badge instead of the raw prompt text (so the
 * long instruction never clutters the transcript). Mirrors the compaction-checkpoint
 * pattern: real content for the model, custom card in the view.
 */
export function isReviewRequestMessage(message: AiChatMessage): boolean {
  return message.kind === "review-request";
}

export function deriveSegmentContent(segments: AiMessageSegment[]): string {
  return segments
    .filter((segment): segment is Extract<AiMessageSegment, { kind: "text" }> => segment.kind === "text")
    .map((segment) => segment.text)
    .filter((text) => text.trim().length > 0)
    .join("\n\n");
}

export function deriveSegmentReasoning(segments: AiMessageSegment[]): string {
  return segments
    .filter((segment): segment is Extract<AiMessageSegment, { kind: "reasoning" }> => segment.kind === "reasoning")
    .map((segment) => segment.text)
    .filter((text) => text.trim().length > 0)
    .join("\n\n");
}

export function deriveSegmentToolCalls(segments: AiMessageSegment[]): AiChatToolCall[] {
  return segments
    .filter((segment): segment is Extract<AiMessageSegment, { kind: "tool" }> => segment.kind === "tool")
    .map((segment) => segment.toolCall);
}
