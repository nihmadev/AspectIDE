import type { AiChatMessage, AiChatSendInput, AiToolApprovalRequest } from "./aiChatTypes";
import { createTurnTimeline } from "./aiChatTimeline";
import { buildUserContent } from "./aiRuntimePrompt";
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
  const pendingApprovals = new Map<string, (decision: "approved" | "rejected") => void>();
  let resolved = false;
  const startedAt = Date.now();

  // Build an ORDERED segment timeline (reasoning → text → tools → reasoning →
  // …), the same structure the TS turn-loop produces, so the UI renders the
  // model's thinking/tools/answer in the real order of work instead of editing
  // flat content/reasoning fields in place.
  const timeline = createTurnTimeline((patch) => input.onAssistantMessageUpdate(messageId, patch));
  // Per-round accumulators: the timeline wants the full text of the *current*
  // active segment, while the backend streams incremental deltas. beginRound()
  // (on each new model round) resets these so a fresh reasoning/text block opens
  // after tools run.
  let roundContent = "";
  let roundReasoning = "";

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

    const handleEvent = (event: AiTurnEvent) => {
      if (!("turnId" in event) || event.turnId !== turnId) return;
      switch (event.kind) {
        case "assistantCreated":
          input.onAssistantMessage(assistantMessage);
          break;
        case "statusChange":
          // "thinking" marks the start of a fresh model round (round 2+ after
          // tools). Open new ordered reasoning/text segments so they append after
          // the previous tools instead of overwriting earlier blocks. NOT done on
          // "running-tools" — that fires before toolCallStarted, which is what
          // commits the just-streamed text.
          if (event.phase === "thinking") {
            timeline.beginRound();
            roundContent = "";
            roundReasoning = "";
          }
          input.onStatusChange?.(mapPhase(event.phase));
          break;
        case "streamDelta":
          // Accumulate this round's text/reasoning and hand the full current
          // segment text to the timeline, which extends the right ordered block.
          roundContent += event.content;
          roundReasoning += event.reasoning;
          timeline.setStreaming({ content: roundContent, reasoning: roundReasoning });
          break;
        case "toolCallStarted":
          // Any streamed text/reasoning for this round is final once tools start.
          timeline.commitRound(roundContent, roundReasoning);
          roundContent = "";
          roundReasoning = "";
          timeline.addToolCalls([{
            id: event.callId,
            tool: event.tool,
            input: event.input,
            status: "running",
            startTime: Date.now(),
          }]);
          break;
        case "toolCallCompleted":
          timeline.updateToolCall(event.callId, {
            status: event.status === "error" ? "error" : "success",
            output: event.output,
            ...(event.error ? { error: event.error } : {}),
            endTime: Date.now(),
          });
          break;
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
          // Finalize the last streamed round, then snapshot the ordered segments.
          timeline.commitRound(roundContent, roundReasoning);
          roundContent = "";
          roundReasoning = "";
          const snapshot = timeline.snapshot();
          const finalMessage: AiChatMessage = {
            ...assistantMessage,
            ...snapshot,
            // Prefer the timeline's derived content; fall back to the final event
            // content only if streaming produced no text segment.
            content: snapshot.content?.trim() ? snapshot.content : (event.content || ""),
            turnUsage: assistantMessage.turnUsage,
            responseDurationMs: event.durationMs || (Date.now() - startedAt),
          };
          input.onAssistantMessageUpdate(messageId, {
            segments: finalMessage.segments,
            content: finalMessage.content,
            reasoning: finalMessage.reasoning,
            toolCalls: finalMessage.toolCalls,
            turnUsage: finalMessage.turnUsage,
            responseDurationMs: finalMessage.responseDurationMs,
          });
          settleResolve(finalMessage);
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
    // Assemble the full user content (pinned attachments, goal/todo blocks,
    // terminal snapshot, and vision `image_url` parts) so the native turn-loop
    // delivers everything the dev/browser TS path does — including images.
    userContent: buildUserContent(input),
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
