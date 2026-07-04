import {
  deriveSegmentContent,
  type AiChatAttachmentInput,
  type AiChatMessage,
  type AiChatResponseTiming,
  type AiChatSendInput,
  type AiChatToolCall,
  type AiToolApprovalDecision,
  type AiToolApprovalRequest,
  type AiToolApprovalState,
} from "./aiChatTypes";
import { createTurnTimeline, type TurnTimeline } from "./aiChatTimeline";
import { runAutomaticPostEditVerification, isFileEditToolName } from "./aiAutomaticVerification";
import { attachTurnCostEstimate, estimateTurnUsageFromAssistant, extractTurnTokenUsage, mergeTurnTokenUsage } from "./aiTurnUsage";
import type { AiChatTurnTokenUsage } from "./aiChatTypes";
import { consumeStopAfterToolRound } from "./aiChatTurnRuntime";
import { clearAiTurnActivity, extractToolPath, setAiTurnActivity } from "./aiTurnActivity";
import { firstChoice, readReasoningDelta, requestChatCompletion, type ChatCompletionMessage, type ChatCompletionResult, type OpenAiToolCall } from "./aiChatTransport";
import { ToolApprovalRejectedError } from "./aiRuntimeToolApproval";
import { collectChangedPathsFromToolResult, createRunningToolCall, formatToolOutput } from "./aiRuntimeToolBridge";
import { runRuntimeTool } from "./aiRuntimeToolDispatch";
import type { RuntimeToolSession } from "./aiRuntimeToolSession";
import type { AiModelConfig } from "./aiPreferences";
import {
  buildPathAttachmentContext,
  encodeVisionImageFromDataUrl,
  imageAttachmentText,
  isVisionImageFile,
  isVisionImagePath,
  maxVisionImageBytes,
  maxVisionSourceBytes,
  readFileAsDataUrl,
} from "./aiFileContext";
import type { VisionImageFormat } from "./aiVisionFormat";
import { buildToolStepsExhaustedBlock } from "./aiSystemPrompt";
import { activeDocumentContextMaxChars, buildInitialMessages } from "./aiRuntimePrompt";
import { isRecord, readErrorMessage, truncateText } from "./aiRuntimeShared";
import { runtimeTools } from "./aiRuntimeTools";

type ResponseTimingAccumulator = Omit<AiChatResponseTiming, "overheadMs" | "totalMs"> & {
  startedAtMs: number;
};

const maxAttachmentChars = 18_000;

function filePathFromDomFile(file: File) {
  const candidate = file as File & { path?: string };
  return candidate.path?.trim() || null;
}

export async function readChatAttachment(
  file: File,
  options: {
    includeVisionImage?: boolean;
    visionImageFormat?: VisionImageFormat;
    includeMediaContext?: boolean;
    localSttCommand?: string;
    localSttModelPath?: string;
    voiceInputLanguage?: string;
  } = {},
): Promise<AiChatAttachmentInput> {
  const path = filePathFromDomFile(file);
  const image = isVisionImageFile(file) || Boolean(path && isVisionImagePath(path));

  if (path) {
    const context = await buildPathAttachmentContext(path, `Attached file: ${file.name}`, options);
    if (image) {
      const visionAttached = Boolean(options.includeVisionImage && context.visionImageUrl);
      return {
        name: file.name,
        size: context.size || file.size,
        text: imageAttachmentText(file.name, context.size || file.size, {
          visionAttached,
          note: visionAttached
            ? undefined
            : file.size > maxVisionImageBytes
              ? `Image exceeds ${maxVisionImageBytes} byte vision limit.`
              : !options.includeVisionImage
                ? "Enable image metadata in AI settings to send vision input."
                : undefined,
        }),
        visionImageUrl: context.visionImageUrl,
        visionFrameUrls: context.visionFrameUrls,
      };
    }
    return {
      name: file.name,
      size: context.size || file.size,
      text: context.text,
      visionImageUrl: context.visionImageUrl,
      visionFrameUrls: context.visionFrameUrls,
    };
  }

  if (image) {
    // No disk path (clipboard paste / drag-drop blob): read the raw bytes up to
    // the larger *source* budget, then let the native encoder downscale + encode
    // to WebP/PNG. The encoder enforces the inline budget and falls back safely.
    let visionImageUrl: string | undefined;
    if (options.includeVisionImage) {
      const rawDataUrl = await readFileAsDataUrl(file, maxVisionSourceBytes);
      if (rawDataUrl) {
        visionImageUrl = await encodeVisionImageFromDataUrl(rawDataUrl, options.visionImageFormat ?? "png");
      }
    }
    return {
      name: file.name,
      size: file.size,
      text: imageAttachmentText(file.name, file.size, {
        visionAttached: Boolean(visionImageUrl),
        note: !options.includeVisionImage
          ? "Enable image metadata in AI settings to send vision input."
          : !visionImageUrl && file.size > maxVisionSourceBytes
            ? `Image exceeds ${maxVisionSourceBytes} byte vision limit.`
            : undefined,
      }),
      visionImageUrl,
    };
  }

  const text = await file.text();
  return {
    name: file.name,
    size: file.size,
    text: truncateText(text, maxAttachmentChars),
  };
}

export async function sendAiChatMessage(input: AiChatSendInput): Promise<AiChatMessage> {
  const timing = createResponseTimingAccumulator();
  const assistantMessage: AiChatMessage = {
    id: crypto.randomUUID(),
    role: "assistant",
    content: "",
    toolCalls: [],
    segments: [],
    timestamp: Date.now(),
  };
  input.onAssistantMessage(assistantMessage);

  const messages = await buildInitialMessages(input);
  const toolSession: RuntimeToolSession = {
    todos: [],
    subagentDepth: input.subagentContext?.depth ?? 0,
    parentAgentId: input.subagentContext?.parentAgentId ?? null,
  };
  const timeline = createTurnTimeline((patch) => input.onAssistantMessageUpdate(assistantMessage.id, patch));
  const toolRoundLimit = input.preferences.toolRoundLimit;
  let turnUsage: AiChatTurnTokenUsage | null = null;

  for (let round = 0; toolRoundLimit === null || round < toolRoundLimit; round += 1) {
    throwIfAborted(input.abortSignal);
    const phase = round === 0 ? "thinking" : "running-tools";
    input.onStatusChange?.(phase);
    setAiTurnActivity(input.chatSessionId, { phase, toolName: null, filePath: null, subagentLabel: null });
    timeline.beginRound();
    const modelCallStartedAtMs = performance.now();
    const response = await requestRuntimeChatCompletion(input, messages, (progress) => {
      const streamPhase = progress.content ? "streaming" : "thinking";
      input.onStatusChange?.(streamPhase);
      setAiTurnActivity(input.chatSessionId, { phase: streamPhase, toolName: null, filePath: null, subagentLabel: null });
      timeline.setStreaming(progress);
    });
    recordModelTiming(timing, response, modelCallStartedAtMs);
    const usage = extractTurnTokenUsage(response.body);
    if (usage) {
      turnUsage = mergeTurnTokenUsage(turnUsage, usage);
    }
    const choice = firstChoice(response.body);
    const assistant = normalizeAssistantMessage(choice?.message);
    timeline.commitRound(assistant.content ?? "", assistant.reasoning ?? "");

    const requestedToolCalls = normalizeToolCalls(assistant.tool_calls);
    if (requestedToolCalls.length === 0) {
      if (deriveSegmentContent(timeline.snapshot().segments ?? []).trim().length === 0) {
        timeline.appendText("The turn produced no answer. Press **Retry** or rephrase your request.");
      }
      const finalMessage = assistantMessageWithTiming(assistantMessage, timeline.snapshot(), timing, turnUsage, input.selectedModel);
      input.onAssistantMessageUpdate(assistantMessage.id, finalMessage);
      return finalMessage;
    }

    messages.push({
      role: "assistant",
      content: assistant.content || null,
      tool_calls: requestedToolCalls,
    });

    input.onStatusChange?.("running-tools");
    setAiTurnActivity(input.chatSessionId, { phase: "running-tools", toolName: null, filePath: null, subagentLabel: null });

    const toolsStartedAtMs = performance.now();
    const toolResults: ChatCompletionMessage[] = [];
    const editedPathsThisRound: string[] = [];
    for (const requestedCall of requestedToolCalls) {
      throwIfAborted(input.abortSignal);
      const uiCall = createRunningToolCall(requestedCall);
      const toolName = requestedCall.function?.name ?? uiCall.tool;
      const toolPath = extractToolPath(toolName, uiCall.input);
      setAiTurnActivity(input.chatSessionId, { phase: "running-tools", toolName, filePath: toolPath, subagentLabel: null });
      timeline.addToolCalls([uiCall]);
      try {
        const result = await runRuntimeTool(requestedCall, input, toolSession, {
          toolCallId: uiCall.id,
          setApproval: (approval) => {
            input.onStatusChange?.("waiting-approval");
            setAiTurnActivity(input.chatSessionId, { phase: "waiting-approval", toolName, filePath: toolPath, subagentLabel: null });
            timeline.updateToolCall(uiCall.id, { status: "approval", approval });
          },
          setRunning: (approval) => {
            input.onStatusChange?.("running-tools");
            setAiTurnActivity(input.chatSessionId, { phase: "running-tools", toolName, filePath: toolPath, subagentLabel: null });
            timeline.updateToolCall(uiCall.id, { status: "running", approval });
          },
        });
        timeline.updateToolCall(uiCall.id, { status: "success", output: formatToolOutput(result), endTime: Date.now(), stats: result.stats });
        if (isFileEditToolName(requestedCall.function?.name)) {
          collectChangedPathsFromToolResult(result.content, editedPathsThisRound);
        }
        toolResults.push({
          role: "tool" as const,
          tool_call_id: requestedCall.id ?? uiCall.id,
          content: result.content,
        });
      } catch (error) {
        if (isAbortErrorLike(error)) throw error;
        const message = readErrorMessage(error);
        const skipped = error instanceof ToolApprovalRejectedError;
        timeline.updateToolCall(uiCall.id, { status: skipped ? "skipped" : "error", error: message, endTime: Date.now() });
        toolResults.push({
          role: "tool" as const,
          tool_call_id: requestedCall.id ?? uiCall.id,
          content: JSON.stringify({ error: message }),
        });
      }
    }
    recordToolTiming(timing, toolsStartedAtMs, requestedToolCalls.length);
    throwIfAborted(input.abortSignal);

    messages.push(...toolResults);

    if (editedPathsThisRound.length > 0) {
      const verification = await runAutomaticPostEditVerification(input, editedPathsThisRound);
      if (verification) {
        messages.push({ role: "system", content: verification.content });
      }
    }

    if (consumeStopAfterToolRound()) {
      const finalUsage = await requestToolLimitFinalAnswer(input, messages, timeline, round + 1, timing, true);
      if (finalUsage) turnUsage = mergeTurnTokenUsage(turnUsage, finalUsage);
      const stoppedMessage = assistantMessageWithTiming(assistantMessage, timeline.snapshot(), timing, turnUsage, input.selectedModel);
      input.onAssistantMessageUpdate(assistantMessage.id, stoppedMessage);
      return stoppedMessage;
    }
  }

  if (toolRoundLimit !== null) {
    const finalUsage = await requestToolLimitFinalAnswer(input, messages, timeline, toolRoundLimit, timing);
    if (finalUsage) turnUsage = mergeTurnTokenUsage(turnUsage, finalUsage);
  }
  const limitedMessage = assistantMessageWithTiming(assistantMessage, timeline.snapshot(), timing, turnUsage, input.selectedModel);
  input.onAssistantMessageUpdate(assistantMessage.id, limitedMessage);
  return limitedMessage;
}

async function requestToolLimitFinalAnswer(input: AiChatSendInput, messages: ChatCompletionMessage[], timeline: TurnTimeline, toolRoundLimit: number, timing: ResponseTimingAccumulator, manualStop = false): Promise<AiChatTurnTokenUsage | null> {
  throwIfAborted(input.abortSignal);
  const toolCalls = timeline.toolCalls();
  const successfulTools = toolCalls.filter((toolCall) => toolCall.status === "success").length;
  const failedTools = toolCalls.filter((toolCall) => toolCall.status === "error").length;
  const exhaustedSummary = { succeeded: successfulTools, failed: failedTools, total: toolCalls.length };
  messages.push({
    role: "system",
    content: buildToolStepsExhaustedBlock(toolRoundLimit, exhaustedSummary),
  });
  messages.push({
    role: "user",
    content: [
      "Finish this turn now without calling more tools.",
      "Use only the evidence already in the conversation.",
      "If work is incomplete, list done / remaining / blockers and mention Settings → AI → Tool rounds.",
    ].join("\n"),
  });

  // The post-limit answer is its own trailing segment so the prior text/tool
  // timeline stays intact and is never overwritten.
  timeline.beginRound();
  let streamedAnswer = "";
  let finalUsage: AiChatTurnTokenUsage | null = null;
  try {
    input.onStatusChange?.("thinking");
    const modelCallStartedAtMs = performance.now();
    const response = await requestRuntimeChatCompletion(input, messages, (progress) => {
      streamedAnswer = progress.content || streamedAnswer;
      input.onStatusChange?.(progress.content ? "streaming" : "thinking");
      timeline.setStreaming(progress);
    }, { toolsEnabled: false });
    recordModelTiming(timing, response, modelCallStartedAtMs);
    finalUsage = extractTurnTokenUsage(response.body);
    const assistant = normalizeAssistantMessage(firstChoice(response.body)?.message);
    timeline.commitRound(assistant.content ?? "", assistant.reasoning ?? "");
    if ((assistant.content?.trim() || streamedAnswer.trim())) return finalUsage;
  } catch (error) {
    throwIfAborted(input.abortSignal);
    if (streamedAnswer.trim()) return finalUsage;
  }

  if (deriveSegmentContent(timeline.snapshot().segments ?? []).trim().length === 0) {
    timeline.appendText(
      manualStop
        ? [
            "Stopped after the current tool round at your request.",
            "Lux executed the tool calls already in flight but the model did not produce a final answer. Send a follow-up to continue.",
          ].join("\n\n")
        : [
            `Tool round limit reached (${toolRoundLimit}).`,
            "Lux executed the available tool calls but the model did not produce a final answer after the limit.",
            "Increase Settings -> AI -> Tool rounds for longer autonomous tasks, then send a follow-up if more work is needed.",
          ].join("\n\n"),
    );
  }

  return finalUsage;
}

function createResponseTimingAccumulator(): ResponseTimingAccumulator {
  return {
    startedAtMs: performance.now(),
    modelMs: 0,
    toolMs: 0,
    firstTokenMs: null,
    streamMs: null,
    modelCalls: 0,
    toolCalls: 0,
    rounds: 0,
    streamed: false,
  };
}

function recordModelTiming(timing: ResponseTimingAccumulator, response: ChatCompletionResult, modelCallStartedAtMs: number) {
  timing.modelCalls += 1;
  timing.rounds += 1;
  timing.modelMs += response.timing.durationMs;
  timing.streamed ||= response.streamed;

  if (response.timing.firstTokenMs !== null && timing.firstTokenMs === null) {
    timing.firstTokenMs = Math.max(0, Math.round(modelCallStartedAtMs + response.timing.firstTokenMs - timing.startedAtMs));
  }

  if (response.timing.streamMs !== null) {
    timing.streamMs = (timing.streamMs ?? 0) + response.timing.streamMs;
  }
}

function recordToolTiming(timing: ResponseTimingAccumulator, startedAtMs: number, toolCalls: number) {
  timing.toolMs += Math.max(0, Math.round(performance.now() - startedAtMs));
  timing.toolCalls += toolCalls;
}

function assistantMessageWithTiming(
  assistantMessage: AiChatMessage,
  patch: Partial<AiChatMessage>,
  timing: ResponseTimingAccumulator,
  turnUsage: AiChatTurnTokenUsage | null,
  model: AiModelConfig,
): AiChatMessage {
  const responseTiming = finalizeResponseTiming(timing);
  const merged: AiChatMessage = {
    ...assistantMessage,
    ...patch,
    responseDurationMs: responseTiming.totalMs,
    responseTiming,
  };
  const baseUsage = turnUsage ?? estimateTurnUsageFromAssistant(merged) ?? undefined;
  // Populate the cost estimate (manual model price first, else alias-based rates).
  const resolvedUsage = baseUsage
    ? attachTurnCostEstimate(
      responseTiming.modelCalls > 0 && baseUsage.requestCount === undefined
        ? { ...baseUsage, requestCount: responseTiming.modelCalls }
        : baseUsage,
      model,
    )
    : undefined;
  return {
    ...merged,
    turnUsage: resolvedUsage,
  };
}

function finalizeResponseTiming(timing: ResponseTimingAccumulator): AiChatResponseTiming {
  const totalMs = Math.max(0, Math.round(performance.now() - timing.startedAtMs));
  return {
    totalMs,
    modelMs: timing.modelMs,
    toolMs: timing.toolMs,
    overheadMs: Math.max(0, totalMs - timing.modelMs - timing.toolMs),
    firstTokenMs: timing.firstTokenMs,
    streamMs: timing.streamMs,
    modelCalls: timing.modelCalls,
    toolCalls: timing.toolCalls,
    rounds: timing.rounds,
    streamed: timing.streamed,
  };
}

function requestRuntimeChatCompletion(
  input: AiChatSendInput,
  messages: ChatCompletionMessage[],
  onStreamProgress: Parameters<typeof requestChatCompletion>[2],
  options: { toolsEnabled?: boolean } = {},
) {
  return requestChatCompletion({
    abortSignal: input.abortSignal,
    provider: input.provider,
    selectedEffortId: input.preferences.selectedEffortId,
    selectedModel: input.selectedModel,
  }, messages, onStreamProgress, {
    tools: runtimeTools,
    toolsEnabled: options.toolsEnabled,
  });
}

function normalizeAssistantMessage(value: unknown) {
  if (!isRecord(value)) return { role: "assistant" as const, content: "", reasoning: "", tool_calls: [] as OpenAiToolCall[] };
  return {
    role: "assistant" as const,
    content: typeof value.content === "string" ? value.content : "",
    reasoning: readReasoningDelta(value),
    tool_calls: normalizeToolCalls(value.tool_calls),
  };
}

function normalizeToolCalls(value: unknown): OpenAiToolCall[] {
  if (!Array.isArray(value)) return [];
  return value.filter(isRecord).map((call, index) => ({
    id: typeof call.id === "string" ? call.id : `tool-${Date.now()}-${index}`,
    type: call.type === "function" ? "function" : "function",
    function: isRecord(call.function) ? {
      name: typeof call.function.name === "string" ? call.function.name : "",
      arguments: typeof call.function.arguments === "string" ? call.function.arguments : "{}",
    } : { name: "", arguments: "{}" },
  }));
}

function throwIfAborted(signal: AbortSignal) {
  if (signal.aborted) throw new DOMException("AI request was cancelled", "AbortError");
}

function isAbortErrorLike(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

