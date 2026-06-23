import type { AiChatMessage, AiChatSendInput, AiToolApprovalRequest } from "./aiChatTypes";
import { createTurnTimeline } from "./aiChatTimeline";
import { isAnthropicCacheModel, reasoningPayload } from "./aiChatTransport";
import { attachTurnCostEstimate } from "./aiTurnUsage";
import { buildUserContent } from "./aiRuntimePrompt";
import { bridgeNativeToolCompleted, bridgeNativeToolStarted } from "./aiNativeOrchestrationBridge";
import { captureNativeEditBefore, NATIVE_FILE_EDIT_TOOLS, registerNativeEditReview } from "./aiNativeFileReview";
import { clearPendingQuestionsForSession, registerPendingQuestion } from "./aiPendingQuestion";
import { clearPendingPlansForSession, getPendingPlanForSession, registerPendingPlan } from "./aiPendingPlan";
import { browserSessionName, ensureBrowserStream } from "./agentBrowser";
import { bumpBrowserStreamRefresh } from "./aiChatTurnRuntime";
import { isTauriRuntime, luxCommands, subscribeAiTurn, type AiRunTurnInput, type AiTurnEvent } from "./tauri";

/**
 * Browser tools that change what's on screen. When one of these completes on the
 * native turn loop, we enable the viewport stream and nudge the live preview to
 * attach — so the user sees (with the blue activity dot) the browser the agent is
 * driving, without having opened it manually.
 */
const NAVIGATIONAL_BROWSER_TOOLS = new Set([
  "BrowserOpen",
  "BrowserAct",
  "BrowserChat",
  "BrowserScreenshot",
  "BrowserClose",
]);

async function reflectBrowserActivityInPreview(input: AiChatSendInput) {
  if (!input.preferences.agentBrowserAutoStreamPreview) return;
  try {
    await ensureBrowserStream(
      browserSessionName(input.chatSessionId),
      input.preferences.agentBrowserCommand.trim() || undefined,
    );
  } catch {
    // Stream enable is best-effort: the preview also polls, and a failure here
    // must never break the turn. The bump still refreshes any open preview.
  }
  bumpBrowserStreamRefresh();
}

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
  // toolCallCompleted carries no tool name, so remember it from toolCallStarted to
  // route the orchestration bridge (TodoWrite/Goal/Task) on completion.
  const toolNameByCallId = new Map<string, string>();
  // Pre-edit file snapshots captured at toolCallStarted, consumed at completion to
  // register a Cursor-style pending review (green/red diff + Accept/Reject).
  const editBeforeByCallId = new Map<string, Promise<Awaited<ReturnType<typeof captureNativeEditBefore>>>>();

  return await new Promise<AiChatMessage>((resolve, reject) => {
    let unlisten: (() => void) | undefined;
    let abortListener: (() => void) | undefined;

    const cleanup = () => {
      unlisten?.();
      if (abortListener) input.abortSignal.removeEventListener("abort", abortListener);
      // Cancel any coalesced streaming frame so no flush lands after settle.
      timeline.dispose();
    };

    const settleResolve = (message: AiChatMessage) => {
      if (resolved) return;
      resolved = true;
      cleanup();
      input.onRetryNotice?.(null);
      // A question tied to this (now finished) turn can never be answered back into
      // the loop, so drop it. An auto-started plan (Automatic mode) was a record of
      // work already executed — clear it; a manual plan persists so its Start button
      // survives the turn that proposed it.
      clearPendingQuestionsForSession(input.chatSessionId);
      const plan = getPendingPlanForSession(input.chatSessionId);
      if (plan?.autoStart) clearPendingPlansForSession(input.chatSessionId);
      resolve(message);
    };
    const settleReject = (error: unknown) => {
      if (resolved) return;
      resolved = true;
      cleanup();
      input.onRetryNotice?.(null);
      // The turn errored/aborted: both prompts are bound to a dead loop now.
      clearPendingQuestionsForSession(input.chatSessionId);
      clearPendingPlansForSession(input.chatSessionId);
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
          // The call recovered (tokens flowing / tools running) — drop any retry notice.
          if (event.phase === "streaming" || event.phase === "running-tools") {
            input.onRetryNotice?.(null);
          }
          input.onStatusChange?.(mapPhase(event.phase));
          break;
        case "userMessageInjected":
          // A message the user staged mid-work was folded into the running turn at a
          // round boundary. Render it as a user bubble (in order) so the transcript
          // shows the steer that the next answer responds to.
          input.onUserMessageInjected?.(event.text);
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
          // Commit only on the FIRST tool of the round (when there is buffered
          // text/reasoning); later parallel tool calls must not re-commit an empty
          // round, which used to splice out the just-shown narration line.
          if (roundContent || roundReasoning) {
            timeline.commitRound(roundContent, roundReasoning);
            roundContent = "";
            roundReasoning = "";
          }
          timeline.addToolCalls([{
            id: event.callId,
            tool: event.tool,
            input: event.input,
            status: "running",
            startTime: Date.now(),
          }]);
          // Mirror native orchestration tools into the Agent-rail stores (the Rust
          // loop executes them but can't write the TS stores the panel reads).
          toolNameByCallId.set(event.callId, event.tool);
          bridgeNativeToolStarted(input.chatSessionId, event.callId, event.tool, event.input);
          // Capture each touched file's pre-edit text now, before the write lands
          // (Default approval mode suspends here, so there is no read/write race),
          // so completion can register an Accept/Reject review against it.
          if (NATIVE_FILE_EDIT_TOOLS.has(event.tool)) {
            editBeforeByCallId.set(event.callId, captureNativeEditBefore(event.tool, event.input, input));
          }
          break;
        case "toolCallCompleted": {
          const completed = timeline.updateToolCall(event.callId, {
            status: event.status === "error" ? "error" : "success",
            output: event.output,
            ...(event.error ? { error: event.error } : {}),
            endTime: Date.now(),
          });
          bridgeNativeToolCompleted(input.chatSessionId, event.callId, toolNameByCallId.get(event.callId) ?? "", event.status, event.output);
          // A successful navigational browser tool means the agent is driving a live
          // page — reflect it in the preview (stream + blue activity dot) automatically.
          if (event.status !== "error" && completed && NAVIGATIONAL_BROWSER_TOOLS.has(completed.tool)) {
            void reflectBrowserActivityInPreview(input);
          }
          // Register the Cursor-style file review once the edit landed successfully.
          {
            const beforeCapture = editBeforeByCallId.get(event.callId);
            if (beforeCapture) {
              editBeforeByCallId.delete(event.callId);
              if (event.status !== "error") {
                const toolName = toolNameByCallId.get(event.callId) ?? "Edit";
                void beforeCapture
                  .then((before) => registerNativeEditReview(toolName, event.callId, event.output, before, input))
                  .catch(() => undefined);
              }
            }
          }
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
        case "questionRequired":
          // Park the question in the ephemeral store; the AiQuestionCard renders it
          // and replies via aiResolveTurnQuestion (which unblocks the Rust loop).
          registerPendingQuestion({
            requestId: event.requestId,
            turnId,
            sessionId: input.chatSessionId,
            question: event.question,
            detail: event.detail,
            options: event.options,
            multiSelect: event.multiSelect,
            allowCustom: event.allowCustom,
            htmlPreview: event.htmlPreview,
          });
          break;
        case "planProposed":
          // Surface the plan card. Goal + task list were already pinned by Rust, so
          // the rail reflects it instantly; the card adds the readable detail + Start.
          registerPendingPlan({
            planId: event.planId,
            turnId,
            sessionId: input.chatSessionId,
            title: event.title,
            summary: event.summary,
            steps: event.steps,
            alternatives: event.alternatives ?? [],
            risks: event.risks ?? [],
            verification: event.verification ?? [],
            quality: event.quality ?? 1,
            coaching: event.coaching ?? [],
            autoStart: event.autoStart,
          });
          break;
        case "turnUsage":
          // Attach the cost estimate immediately so the turn summary shows price
          // without waiting for the post-turn pass. Uses the model's manual price
          // (Settings → Providers) when set, else alias-based defaults.
          assistantMessage.turnUsage = attachTurnCostEstimate(
            {
              promptTokens: event.promptTokens,
              completionTokens: event.completionTokens,
              totalTokens: event.totalTokens,
              estimatedCostUsd: null,
              ...(event.cachedPromptTokens && event.cachedPromptTokens > 0
                ? { cachedPromptTokens: event.cachedPromptTokens }
                : {}),
            },
            input.selectedModel,
          );
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
        case "turnRetry":
          // A transient provider failure is being auto-retried (connection phase,
          // no tokens streamed yet). Surface a live notice; the next streaming /
          // tool / done event clears it once the call recovers or the turn ends.
          input.onRetryNotice?.({
            attempt: event.attempt,
            maxAttempts: event.maxAttempts,
            reason: event.reason,
            detail: event.detail,
            delayMs: event.delayMs,
          });
          input.onStatusChange?.("thinking");
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
    // User-configured deny/ask/allow permission rules must reach the native loop:
    // they are the authoritative gate (deny is a hard block even in full-access /
    // automatic). Without this the rules only ran on the dev-only TS path.
    toolPermissionRules: input.preferences.toolPermissionRules,
    // Reasoning effort the TS path attaches via reasoningPayload() must also reach
    // the native Rust turn-loop, otherwise desktop requests silently drop the
    // effort on reasoning models. Empty object when the model has no effort levels.
    reasoning: reasoningPayload(input.preferences.selectedEffortId, input.provider),
    // Anthropic prompt caching: tag the (stable) system prompt with a cache_control
    // breakpoint so Claude-family models cache it and re-read it cheaply each turn.
    // The TS path does this via applyPromptCacheBreakpoints; the native path needs
    // the flag so Rust applies the same breakpoint (otherwise desktop never caches).
    anthropicCache: isAnthropicCacheModel(input.selectedModel),
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
      // Token-economy + custom-prompt overrides must reach the native Rust prompt
      // builder, otherwise desktop turns silently ignore both settings.
      tokenEconomy: input.preferences.tokenEconomyEnabled,
      customPromptEnabled: input.preferences.customSystemPromptEnabled,
      customPrompt: input.preferences.customSystemPrompt,
    },
    agentBrowserEnabled: input.preferences.agentBrowserEnabled,
    activeDocumentPath: input.activeDocument?.path ?? null,
    openDocumentPaths: input.openDocuments.map((doc) => doc.path ?? "").filter(Boolean),
    terminalContext: input.terminalContext,
  };
}
