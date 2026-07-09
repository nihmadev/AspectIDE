import type { AiChatSendInput, AiChatMessage } from "./../chat/types";
import { sendAiChatMessage } from "./../chat/runtime";
import { truncateText } from "./../runtime/shared";
import { appendSubagentTranscript, completeSubagentRun, registerSubagentRun, updateLastSubagentTranscript } from "./runs";
import { countRunningSubagents } from "./runs";
import { setAiTurnActivity } from "./../utils/turn-activity";

export const MAX_SUBAGENT_DEPTH = 4;

export type SubagentKind =
  | "generalPurpose"
  | "codeReviewer"
  | "testRunner"
  | "explorer";

export type SubagentCatalogEntry = {
  id: SubagentKind;
  label: string;
  description: string;
  readonlyTools?: boolean;
};

export const subagentCatalog: SubagentCatalogEntry[] = [
  {
    id: "generalPurpose",
    label: "General",
    description: "Research, search, and multi-step tasks in an isolated context.",
  },
  {
    id: "codeReviewer",
    label: "Code review",
    description: "Review diffs and implementation against requirements. Read-only tools.",
    readonlyTools: true,
  },
  {
    id: "testRunner",
    label: "Test runner",
    description: "Run tests and report failures with evidence.",
  },
  {
    id: "explorer",
    label: "Explorer",
    description: "Fast codebase exploration without file writes.",
    readonlyTools: true,
  },
];

export type SubagentSpawnInput = {
  parentInput: AiChatSendInput;
  description: string;
  prompt: string;
  subagentType: SubagentKind;
  model?: string;
  depth: number;
  parentAgentId?: string;
};

export type SubagentSpawnResult = {
  agentId: string;
  subagentType: SubagentKind;
  depth: number;
  parentAgentId: string | null;
  childAgentIds: string[];
  summary: string;
  message: AiChatMessage;
};

export async function spawnSubagent(input: SubagentSpawnInput): Promise<SubagentSpawnResult> {
  if (input.depth >= MAX_SUBAGENT_DEPTH) {
    throw new Error(`Subagent depth limit reached (${MAX_SUBAGENT_DEPTH}).`);
  }

  const profile = subagentCatalog.find((entry) => entry.id === input.subagentType) ?? subagentCatalog[0];
  const agentId = `subagent-${Date.now().toString(36)}-${crypto.randomUUID().slice(0, 8)}`;
  // Resolve the optional model override: if the caller specified a model string that matches
  // a model id in the active provider, use it for the child turn. Otherwise fall back to the
  // parent's selection so the no-override case is silent and never produces a wrong model.
  const resolveChildModelId = (): string => {
    const requested = input.model?.trim();
    if (!requested) return input.parentInput.preferences.selectedModelId;
    const activeProvider = input.parentInput.preferences.providers.find(
      (provider) => provider.id === input.parentInput.preferences.selectedProviderId,
    );
    if (activeProvider?.models.some((model) => model.id === requested)) return requested;
    // Model id not found — fall back silently (don't throw; a wrong id breaks the child turn).
    return input.parentInput.preferences.selectedModelId;
  };

  const childPreferences = {
    ...input.parentInput.preferences,
    agentMode: profile.readonlyTools ? "ask" as const : input.parentInput.preferences.agentMode,
    toolRoundLimit: input.parentInput.preferences.toolRoundLimit === null
      ? null
      : Math.min(input.parentInput.preferences.toolRoundLimit, 24),
    // Apply explicit model override so subagents run on the requested model (F3 fix).
    selectedModelId: resolveChildModelId(),
  };

  const instructions = [
    `You are a Lux subagent (${profile.label}). Parent depth: ${input.depth}.`,
    `Task title: ${input.description}`,
    profile.description,
    "Return a concise final summary for the parent agent. Do not mention internal tool names unless relevant.",
    "Coordination: this chat session has a shared agent board. Before deep work, AgentMessage action=read to see what sibling/parent agents already found. When you discover something other agents need (file locations, decisions, contracts, blockers), AgentMessage action=post it with a clear topic so the work is not repeated.",
    input.depth + 1 < MAX_SUBAGENT_DEPTH
      ? "You may spawn nested subagents via Task when work is independent."
      : "Do not spawn further subagents — depth limit reached.",
  ].join("\n");

  const childAbort = new AbortController();
  const parentAbort = input.parentInput.abortSignal;
  if (parentAbort.aborted) childAbort.abort();
  else parentAbort.addEventListener("abort", () => childAbort.abort(), { once: true });

  registerSubagentRun({
    id: agentId,
    sessionId: input.parentInput.chatSessionId,
    description: input.description,
    subagentType: input.subagentType,
    depth: input.depth,
    parentAgentId: input.parentAgentId ?? null,
    abortController: childAbort,
  });
  setAiTurnActivity(input.parentInput.chatSessionId, {
    phase: "subagent",
    subagentLabel: `${profile.label}: ${input.description}`,
    toolName: null,
    filePath: null,
  });
  appendSubagentTranscript(agentId, input.prompt, "system");

  let resultMessage: AiChatMessage | null = null;
  try {
    await sendAiChatMessage({
      ...input.parentInput,
      abortSignal: childAbort.signal,
      preferences: childPreferences,
      message: input.prompt,
      history: [],
      subagentContext: {
        depth: input.depth + 1,
        parentAgentId: agentId,
      },
      selectedAgentInstructions: instructions,
      selectedAgentName: profile.label,
      globalInstructions: input.parentInput.globalInstructions,
      projectInstructions: input.parentInput.projectInstructions,
      onAssistantMessage: (message) => {
        resultMessage = message;
      },
      onAssistantMessageUpdate: (_id, patch) => {
        if (resultMessage) resultMessage = { ...resultMessage, ...patch };
        // Update the last transcript entry in-place rather than appending the full
        // accumulated content as a new entry on every streaming chunk — prevents
        // O(n²) allocation and duplicate/growing-prefix entries in the panel (F2 fix).
        if (typeof patch.content === "string" && patch.content.trim()) {
          updateLastSubagentTranscript(agentId, patch.content);
        }
      },
      onToolApproval: input.parentInput.onToolApproval,
      onStatusChange: input.parentInput.onStatusChange,
      onContextBudgetReport: input.parentInput.onContextBudgetReport,
      onFilePathsEdited: input.parentInput.onFilePathsEdited,
    });
    const summary = truncateText((resultMessage as AiChatMessage | null)?.content ?? "", 4_000);
    completeSubagentRun(agentId, summary, childAbort.signal.aborted ? "cancelled" : "completed");
  } catch (error) {
    const summary = error instanceof Error ? error.message : String(error);
    completeSubagentRun(agentId, summary, childAbort.signal.aborted ? "cancelled" : "failed");
    throw error;
  } finally {
    if (countRunningSubagents(input.parentInput.chatSessionId) === 0) {
      setAiTurnActivity(input.parentInput.chatSessionId, { subagentLabel: null });
    }
  }

  const message = resultMessage ?? {
    id: agentId,
    role: "assistant" as const,
    content: "Subagent finished without a response.",
    timestamp: Date.now(),
  };

  return {
    agentId,
    subagentType: input.subagentType,
    depth: input.depth + 1,
    parentAgentId: input.parentAgentId ?? null,
    childAgentIds: [],
    summary: truncateText(message.content, 8_000),
    message,
  };
}

export function resolveSubagentType(value: string): SubagentKind {
  const normalized = value.trim();
  if (subagentCatalog.some((entry) => entry.id === normalized)) return normalized as SubagentKind;
  return "generalPurpose";
}