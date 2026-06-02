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
  tool: "Write" | "StrReplace" | "Delete" | "Shell" | "TerminalWrite" | "PatchEngine" | "Checkpoint";
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

// Assistant turns are rendered as an ordered timeline, so reasoning, text and
// tool calls keep their exact model order instead of being flattened.
export type AiMessageSegment =
  | { kind: "reasoning"; id: string; text: string }
  | { kind: "text"; id: string; text: string }
  | { kind: "tool"; id: string; toolCall: AiChatToolCall };

export type AiChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  reasoning?: string;
  toolCalls?: AiChatToolCall[];
  segments?: AiMessageSegment[];
  responseDurationMs?: number;
  responseTiming?: AiChatResponseTiming;
  timestamp: number;
};

export type AiChatAttachmentInput = {
  name: string;
  size: number;
  text: string;
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
  onAssistantMessage: (message: AiChatMessage) => void;
  onAssistantMessageUpdate: (messageId: string, patch: Partial<AiChatMessage>) => void;
  onStatusChange?: (status: AiChatRuntimeStatus) => void;
  onToolApproval: (request: AiToolApprovalRequest) => Promise<AiToolApprovalDecision>;
};

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
