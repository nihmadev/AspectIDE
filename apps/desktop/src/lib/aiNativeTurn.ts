import type { AiChatMessage, AiChatSendInput, AiToolApprovalRequest } from "./aiChatTypes";
import type { AiChatToolCall } from "./aiChatTypes";
import { isTauriRuntime, luxCommands, subscribeAiTurn, type AiRunTurnInput, type AiTurnEvent } from "./tauri";

/**
 * Native turn-loop bridge (Stage 5).
 *
 * Drives a chat turn through the Rust `ai_run_turn` command and maps the
 * `lux://ai-turn` events back onto the same callback contract the React side
 * already uses with `sendAiChatMessage`. The orchestration (model↔tool loop,
 * dispatch, approvals) runs entirely in Rust; this file is the thin visual
 * bridge that renders events into React state.
 *
 * Returns the completed assistant message, matching `sendAiChatMessage`.
 */
export async function runNativeChatTurn(input: AiChatSendInput): Promise<AiChatMessage> {
  if (!isTauriRuntime()) {
    throw new Error("Native turn loop requires the desktop runtime.");
  }

  const turnId = crypto.randomUUID();
  const messageId = crypto.randomUUID();

  const assistantMessage: AiChatMessage = {
    id: messageId,
    role: "assistant",
    content: "",
    toolCalls: [],
    segments: [],
    timestamp: Date.now(),
  };
  // Track tool calls by id so completion events can update the right card.
  const toolCallsById = new Map<string, AiChatToolCall>();
  const pendingApprovals = new Map<string, (decision: "approved" | "rejected") => void>();
  let streamedContent = "";
  let streamedReasoning = "";
  let resolved = false;
  const startedAt = Date.now();

  return await new Promise<AiChatMessage>((resolve, reject) => {
    let unlisten: (() => void) | undefined;
    let abortListener: (() => void) | undefined;

    const cleanup = () => {
      unlisten?.();
      if (abortListener) input.abortSignal.removeEventListener("abort", abortListener);
    };

    const settleResolve = (message: AiChatMessage) => {
      if (resolved) return;
      resolved = true;
      cleanup();
      resolve(message);
    };
    const settleReject = (error: unknown) => {
      if (resolved) return;
      resolved = true;
      cleanup();
      reject(error);
    };

    const flushAssistant = () => {
      assistantMessage.content = streamedContent;
      if (streamedReasoning) assistantMessage.reasoning = streamedReasoning;
      assistantMessage.toolCalls = [...toolCallsById.values()];
      input.onAssistantMessageUpdate(messageId, {
        content: assistantMessage.content,
        reasoning: assistantMessage.reasoning,
        toolCalls: assistantMessage.toolCalls,
      });
    };

    const handleEvent = (event: AiTurnEvent) => {
      if (!("turnId" in event) || event.turnId !== turnId) return;
      switch (event.kind) {
        case "assistantCreated":
          input.onAssistantMessage(assistantMessage);
          break;
        case "statusChange":
          input.onStatusChange?.(mapPhase(event.phase));
          break;
        case "streamDelta":
          streamedContent += event.content;
          streamedReasoning += event.reasoning;
          flushAssistant();
          break;
        case "toolCallStarted":
          toolCallsById.set(event.callId, {
            id: event.callId,
            tool: event.tool,
            input: event.input,
            status: "running",
            startTime: Date.now(),
          });
          flushAssistant();
          break;
        case "toolCallCompleted": {
          const existing = toolCallsById.get(event.callId);
          if (existing) {
            existing.status = event.status === "error" ? "error" : "success";
            existing.output = event.output;
            if (event.error) existing.error = event.error;
            existing.endTime = Date.now();
          }
          flushAssistant();
          break;
        }
        case "approvalRequired": {
          const request: AiToolApprovalRequest = {
            id: event.requestId,
            tool: event.tool as AiToolApprovalRequest["tool"],
            title: event.title,
            path: "",
            summary: event.summary,
            preview: event.preview,
            risk: event.risk as AiToolApprovalRequest["risk"],
            approveLabel: "Approve",
            rejectLabel: "Reject",
          };
          // Bridge UI approval back to Rust via the resolve command.
          void input.onToolApproval(request).then((decision) => {
            const normalized = decision === "approved" ? "approved" : "rejected";
            void luxCommands.aiResolveTurnApproval(turnId, event.requestId, normalized).catch(() => undefined);
          });
          break;
        }
        case "turnUsage":
          assistantMessage.turnUsage = {
            promptTokens: event.promptTokens,
            completionTokens: event.completionTokens,
            totalTokens: event.totalTokens,
            estimatedCostUsd: null,
          };
          break;
        case "turnDone": {
          streamedContent = event.content || streamedContent;
          assistantMessage.content = streamedContent;
          assistantMessage.toolCalls = [...toolCallsById.values()];
          assistantMessage.responseDurationMs = event.durationMs || (Date.now() - startedAt);
          input.onAssistantMessageUpdate(messageId, {
            content: assistantMessage.content,
            reasoning: assistantMessage.reasoning,
            toolCalls: assistantMessage.toolCalls,
            turnUsage: assistantMessage.turnUsage,
            responseDurationMs: assistantMessage.responseDurationMs,
          });
          settleResolve(assistantMessage);
          break;
        }
        case "turnError":
          settleReject(new Error(event.error));
          break;
        case "turnCancelled":
          settleReject(new DOMException("AI request was cancelled", "AbortError"));
          break;
      }
    };

    if (input.abortSignal.aborted) {
      reject(new DOMException("AI request was cancelled", "AbortError"));
      return;
    }
    abortListener = () => {
      void luxCommands.aiCancelTurn(turnId).catch(() => undefined);
      // Reject pending approvals so the Rust loop unblocks.
      pendingApprovals.forEach((fn) => fn("rejected"));
      settleReject(new DOMException("AI request was cancelled", "AbortError"));
    };
    input.abortSignal.addEventListener("abort", abortListener, { once: true });

    void subscribeAiTurn(handleEvent)
      .then((stop) => {
        unlisten = stop;
        if (resolved) { stop(); return; }
        // Launch the native turn after the subscription is live.
        void luxCommands.aiRunTurn(buildRunTurnInput(input, turnId, messageId)).catch((error) => {
          settleReject(error);
        });
      })
      .catch((error) => settleReject(error));
  });
}

function mapPhase(phase: string): "thinking" | "streaming" | "running-tools" | "waiting-approval" {
  switch (phase) {
    case "streaming": return "streaming";
    case "running-tools": return "running-tools";
    case "waiting-approval": return "waiting-approval";
    default: return "thinking";
  }
}

function buildRunTurnInput(input: AiChatSendInput, turnId: string, messageId: string): AiRunTurnInput {
  const selectedModelAlias = input.selectedModel.alias || input.selectedModel.id;
  return {
    turnId,
    messageId,
    sessionId: input.chatSessionId,
    message: input.message,
    history: input.history.map((message) => ({ role: message.role, content: message.content })),
    baseUrl: input.provider.baseUrl,
    apiKey: input.provider.apiKey || null,
    model: selectedModelAlias,
    agentMode: input.preferences.agentMode,
    toolRoundLimit: input.preferences.toolRoundLimit,
    toolApprovalMode: input.preferences.toolApprovalMode,
    promptInput: {
      agentMode: input.preferences.agentMode,
      agentName: input.selectedAgentName,
      agentInstructions: input.selectedAgentInstructions,
      globalInstructions: input.globalInstructions,
      projectInstructions: input.projectInstructions,
      projectAgentsSnip: "",
      toolApprovalMode: input.preferences.toolApprovalMode,
      toolRoundLimit: input.preferences.toolRoundLimit,
      selectedEffortId: input.preferences.selectedEffortId,
      selectedModelAlias,
      providerName: input.provider.name,
      providerProtocol: input.provider.protocol,
      workspaceRoot: input.workspace?.root ?? "",
      runtimeToolsAvailable: true,
      agentBrowserEnabled: input.preferences.agentBrowserEnabled,
    },
    agentBrowserEnabled: input.preferences.agentBrowserEnabled,
    activeDocumentPath: input.activeDocument?.path ?? null,
    openDocumentPaths: input.openDocuments.map((doc) => doc.path ?? "").filter(Boolean),
    terminalContext: input.terminalContext,
  };
}
