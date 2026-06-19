import type { AiChatContextBudgetReport } from "./aiChatContextReport";
import type { AiModelConfig, AiPreferences, AiProviderConfig } from "./aiPreferences";
import type { Locale } from "./i18n";
import type { TerminalOutputBuffer } from "./terminalTypes";
import type { DocumentSnapshot, TerminalSessionInfo, WorkspaceInfo } from "./types";

export type AiChatRole = "system" | "user" | "assistant" | "tool";

export type AiChatToolStatus = "approval" | "running" | "success" | "skipped" | "error";
export type AiChatRuntimeStatus = "thinking" | "streaming" | "running-tools" | "waiting-approval";

export type AiToolApprovalDecision = "approved" | "rejected";

export type AiToolApprovalRequest = {
  id: string;
  tool: "Write" | "StrReplace" | "Delete" | "Shell" | "TerminalWrite" | "PatchEngine" | "Checkpoint" | "BrowserOpen" | "BrowserAct" | "BrowserChat" | "BrowserInstall";
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
};

// Assistant turns are rendered as an ordered timeline, so reasoning, text and
// tool calls keep their exact model order instead of being flattened.
export type AiMessageSegment =
  | { kind: "reasoning"; id: string; text: string }
  | { kind: "text"; id: string; text: string }
  | { kind: "tool"; id: string; toolCall: AiChatToolCall };

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
