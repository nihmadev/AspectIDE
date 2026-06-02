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
import { firstChoice, readReasoningDelta, requestChatCompletion, type ChatCompletionMessage, type ChatCompletionResult, type OpenAiToolCall } from "./aiChatTransport";
import { buildAiProjectIndexSnapshot } from "./aiProjectIndex";
import { createDeleteApproval, createPatchApproval, createShellApproval, createStrReplaceApproval, createTerminalWriteApproval, createWriteApproval } from "./aiRuntimeApprovals";
import { checkpointTool } from "./aiRuntimeCheckpoints";
import { addDirectContextBudgetItems, addToolContextBudgetItems, buildContextBudgeterNextActions, contextBudgeterUnavailable, parseToolContent, rankContextBudgetItems, selectContextBudgetItems, type ContextBudgetItem } from "./aiRuntimeContextBudget";
import { docsContext, memoryContext, rulesContext } from "./aiRuntimeContextSources";
import { diagnosticsContext, failureAnalyzer, gitContext, readLints, reviewDiff, testHealth } from "./aiRuntimeDiagnostics";
import { countLines, patchOperationsArg } from "./aiRuntimePatch";
import { activeDocumentContextMaxChars, buildInitialMessages } from "./aiRuntimePrompt";
import { publicSecretFinding, scanSecrets, secretGuard as runSecretGuard } from "./aiRuntimeSecretGuard";
import { booleanArg, clamp, isRecord, maxToolOutputChars, normalizePathSlashes, numberArg, optionalPositiveNumberArg, readErrorMessage, stringArg, stringArrayArg, toolJson, topCounts, truncateText, type ToolResult, type UnknownRecord } from "./aiRuntimeShared";
import { compactTerminalContext, compactTerminalSession, selectTerminalSession, terminalSessionsForContext, terminalWritePreview } from "./aiRuntimeTerminal";
import {
  compactIndexedFile,
  compareRelatedDescriptors,
  createRelatedFileDescriptor,
  isEntrypointFile,
  isImportantProjectFile,
  isLowSignalRelatedPath,
  isSourcePath,
  isTestFile,
  languageForPath,
  passesSemanticPathFilter,
  resolveWorkspacePath,
  scorePath,
  scoreRelatedFile,
  scoreSemanticFile,
  scoreSemanticSymbol,
  scoreSemanticTextHit,
  tokenizeRelatedQuery,
  topDirectory,
  upsertSemanticResult,
  type RelatedFileMatch,
  type SemanticSearchResult,
} from "./aiRuntimeFileContext";
import { runtimeTools, type RuntimeToolName } from "./aiRuntimeTools";
import { isTauriRuntime, luxCommands } from "./tauri";
import type { FileInspection, LspDocumentSymbol, LspLocation, WorkspaceInfo } from "./types";

type FileToolResult = Awaited<ReturnType<typeof luxCommands.aiFileWrite>>;

type SessionTodoStatus = "pending" | "in_progress" | "completed" | "blocked" | "cancelled";

type SessionTodoPriority = "low" | "medium" | "high";

type SessionTodo = {
  id: string;
  content: string;
  status: SessionTodoStatus;
  priority: SessionTodoPriority;
  notes?: string;
};

type RuntimeToolSession = {
  todos: SessionTodo[];
};

type ResponseTimingAccumulator = Omit<AiChatResponseTiming, "overheadMs" | "totalMs"> & {
  startedAtMs: number;
};

type ToolExecutionUi = {
  setApproval: (approval: AiToolApprovalState) => void;
  setRunning: (approval?: AiToolApprovalState) => void;
};

const maxAttachmentChars = 18_000;
export async function readChatAttachment(file: File): Promise<AiChatAttachmentInput> {
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

  const messages = buildInitialMessages(input);
  const toolSession: RuntimeToolSession = { todos: [] };
  const timeline = createTurnTimeline((patch) => input.onAssistantMessageUpdate(assistantMessage.id, patch));
  const toolRoundLimit = input.preferences.toolRoundLimit;

  for (let round = 0; toolRoundLimit === null || round < toolRoundLimit; round += 1) {
    throwIfAborted(input.abortSignal);
    input.onStatusChange?.(round === 0 ? "thinking" : "running-tools");
    timeline.beginRound();
    const modelCallStartedAtMs = performance.now();
    const response = await requestRuntimeChatCompletion(input, messages, (progress) => {
      input.onStatusChange?.(progress.content ? "streaming" : "thinking");
      timeline.setStreaming(progress);
    });
    recordModelTiming(timing, response, modelCallStartedAtMs);
    const choice = firstChoice(response.body);
    const assistant = normalizeAssistantMessage(choice?.message);
    timeline.commitRound(assistant.content ?? "", assistant.reasoning ?? "");

    const requestedToolCalls = normalizeToolCalls(assistant.tool_calls);
    if (requestedToolCalls.length === 0) {
      if (deriveSegmentContent(timeline.snapshot().segments ?? []).trim().length === 0) {
        timeline.appendText("Done.");
      }
      const finalMessage = assistantMessageWithTiming(assistantMessage, timeline.snapshot(), timing);
      input.onAssistantMessageUpdate(assistantMessage.id, finalMessage);
      return finalMessage;
    }

    messages.push({
      role: "assistant",
      content: assistant.content || null,
      tool_calls: requestedToolCalls,
    });

    input.onStatusChange?.("running-tools");

    const toolsStartedAtMs = performance.now();
    const toolResults: ChatCompletionMessage[] = [];
    for (const requestedCall of requestedToolCalls) {
      throwIfAborted(input.abortSignal);
      const uiCall = createRunningToolCall(requestedCall);
      timeline.addToolCalls([uiCall]);
      try {
        const result = await runRuntimeTool(requestedCall, input, toolSession, {
          setApproval: (approval) => {
            input.onStatusChange?.("waiting-approval");
            timeline.updateToolCall(uiCall.id, { status: "approval", approval });
          },
          setRunning: (approval) => {
            input.onStatusChange?.("running-tools");
            timeline.updateToolCall(uiCall.id, { status: "running", approval });
          },
        });
        timeline.updateToolCall(uiCall.id, { status: "success", output: formatToolOutput(result), endTime: Date.now(), stats: result.stats });
        toolResults.push({
          role: "tool" as const,
          tool_call_id: requestedCall.id ?? uiCall.id,
          content: result.content,
        });
      } catch (error) {
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

    messages.push(...toolResults);
  }

  if (toolRoundLimit !== null) await requestToolLimitFinalAnswer(input, messages, timeline, toolRoundLimit, timing);
  const limitedMessage = assistantMessageWithTiming(assistantMessage, timeline.snapshot(), timing);
  input.onAssistantMessageUpdate(assistantMessage.id, limitedMessage);
  return limitedMessage;
}

async function requestToolLimitFinalAnswer(input: AiChatSendInput, messages: ChatCompletionMessage[], timeline: TurnTimeline, toolRoundLimit: number, timing: ResponseTimingAccumulator) {
  throwIfAborted(input.abortSignal);
  const toolCalls = timeline.toolCalls();
  const successfulTools = toolCalls.filter((toolCall) => toolCall.status === "success").length;
  const failedTools = toolCalls.filter((toolCall) => toolCall.status === "error").length;
  messages.push({
    role: "user",
    content: [
      `Lux reached the configured tool round limit (${toolRoundLimit}).`,
      "Do not call more tools in this turn.",
      "Write the best final answer from the evidence already gathered.",
      "If the work is incomplete, be specific about what is done, what remains, and which setting controls the limit.",
      `Tool summary: ${successfulTools} succeeded, ${failedTools} failed, ${toolCalls.length} total.`,
    ].join("\n"),
  });

  // The post-limit answer is its own trailing segment so the prior text/tool
  // timeline stays intact and is never overwritten.
  timeline.beginRound();
  let streamedAnswer = "";
  try {
    input.onStatusChange?.("thinking");
    const modelCallStartedAtMs = performance.now();
    const response = await requestRuntimeChatCompletion(input, messages, (progress) => {
      streamedAnswer = progress.content || streamedAnswer;
      input.onStatusChange?.(progress.content ? "streaming" : "thinking");
      timeline.setStreaming(progress);
    }, { toolsEnabled: false });
    recordModelTiming(timing, response, modelCallStartedAtMs);
    const assistant = normalizeAssistantMessage(firstChoice(response.body)?.message);
    timeline.commitRound(assistant.content ?? "", assistant.reasoning ?? "");
    if ((assistant.content?.trim() || streamedAnswer.trim())) return;
  } catch (error) {
    throwIfAborted(input.abortSignal);
    if (streamedAnswer.trim()) return;
  }

  if (deriveSegmentContent(timeline.snapshot().segments ?? []).trim().length === 0) {
    timeline.appendText([
      `Tool round limit reached (${toolRoundLimit}).`,
      "Lux executed the available tool calls but the model did not produce a final answer after the limit.",
      "Increase Settings -> AI -> Tool rounds for longer autonomous tasks, then send a follow-up if more work is needed.",
    ].join("\n\n"));
  }
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

function assistantMessageWithTiming(assistantMessage: AiChatMessage, patch: Partial<AiChatMessage>, timing: ResponseTimingAccumulator): AiChatMessage {
  const responseTiming = finalizeResponseTiming(timing);
  return {
    ...assistantMessage,
    ...patch,
    responseDurationMs: responseTiming.totalMs,
    responseTiming,
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

async function runRuntimeTool(call: OpenAiToolCall, input: AiChatSendInput, session: RuntimeToolSession, ui: ToolExecutionUi): Promise<ToolResult> {
  const name = call.function?.name as RuntimeToolName | undefined;
  const args = parseToolArguments(call.function?.arguments);
  switch (name) {
    case "FastContext":
      return fastContext(input, stringArg(args, "query", input.message));
    case "RepoMap":
      return repoMap(numberArg(args, "maxFiles", 80));
    case "WorkspaceIndex":
      return workspaceIndex(args, input);
    case "ActiveContext":
      return activeContext(args, input);
    case "RulesContext":
      return rulesContext(args, input);
    case "DocsContext":
      return docsContext(args, input);
    case "MemoryContext":
      return memoryContext(args, input);
    case "ContextBudgeter":
      return contextBudgeter(args, input);
    case "SemanticSearch":
      return semanticSearch(args, input);
    case "Glob":
      return globFiles(stringArg(args, "pattern"), numberArg(args, "maxResults", 80));
    case "Read":
      return readFileTool(stringArg(args, "path"), numberArg(args, "maxBytes", 120_000));
    case "InspectFile":
      return inspectFileTool(args);
    case "Write":
      return writeFileTool(args, input, ui);
    case "StrReplace":
      return strReplaceTool(args, input, ui);
    case "PatchEngine":
      return patchEngineTool(args, input, ui);
    case "Checkpoint":
      return checkpointTool(args, input, {
        requireApproval: (approval) => requireToolApproval(input, ui, approval),
        applyPatch: async (operations, saveToDisk, dryRun) => {
          const result = await luxCommands.aiFilePatch(operations, saveToDisk, dryRun);
          return toolResultFromFileOperation("Checkpoint", result);
        },
      });
    case "Delete":
      return deleteFileTool(stringArg(args, "path"), input, ui);
    case "Shell":
      return shellTool(args, input, ui);
    case "TerminalContext":
      return terminalContextTool(args, input);
    case "TerminalWrite":
      return terminalWriteTool(args, input, ui);
    case "Grep":
      return grepTool(args);
    case "ReadLints":
      return readLints(args, input);
    case "TodoWrite":
      return todoWrite(args, session);
    case "WebFetch":
      return webFetchTool(args);
    case "SymbolContext":
      return symbolContext(args, input);
    case "RelatedFiles":
      return relatedFiles(args, input);
    case "DiagnosticsContext":
      return diagnosticsContext(numberArg(args, "maxResults", 80));
    case "GitContext":
      return gitContext();
    case "TestHealth":
      return testHealth();
    case "FailureAnalyzer":
      return failureAnalyzer(args);
    case "ImpactAnalysis":
      return impactAnalysis(args, input);
    case "ReviewDiff":
      return reviewDiff(args);
    case "SecretGuard":
      return runSecretGuard(args);
    default:
      throw new Error(`Unknown tool: ${name ?? "missing"}`);
  }
}

async function fastContext(input: AiChatSendInput, query: string): Promise<ToolResult> {
  const [active, index, repo, rules, memory, diagnostics, git, related, impact, search] = await Promise.allSettled([
    activeContext({ maxOpenDocuments: 16 }, input),
    workspaceIndex({ maxFiles: 24, maxScan: 2_500 }, input),
    repoMap(48),
    rulesContext({ query, maxFiles: 8 }, input),
    memoryContext({ query, maxFiles: 8, maxSignals: 24, includeRecentChat: true }, input),
    diagnosticsContext(40),
    gitContext(),
    relatedFiles({ query, maxResults: 24 }, input),
    impactAnalysis({ query, maxResults: 18 }, input),
    query.trim() ? grepTool({ query, maxResults: 20, useRegex: false, caseSensitive: false }) : globFiles("", 40),
  ]);
  const parts = [
    `Active document: ${input.activeDocument?.path ?? input.activeDocument?.title ?? "none"}`,
    settledContent("ActiveContext", active),
    settledContent("WorkspaceIndex", index),
    settledContent("RepoMap", repo),
    settledContent("RulesContext", rules),
    settledContent("MemoryContext", memory),
    settledContent("DiagnosticsContext", diagnostics),
    settledContent("GitContext", git),
    settledContent("RelatedFiles", related),
    settledContent("ImpactAnalysis", impact),
    settledContent("Search", search),
  ];
  return toolJson("FastContext", { query, context: parts.join("\n\n") });
}

async function repoMap(maxFiles: number): Promise<ToolResult> {
  const files = await luxCommands.fsListFiles(clamp(maxFiles, 1, 500));
  const important = files
    .filter((entry) => entry.kind === "file")
    .sort((left, right) => scorePath(right.path) - scorePath(left.path) || left.path.localeCompare(right.path))
    .slice(0, clamp(maxFiles, 1, 500));
  return toolJson("RepoMap", {
    totalListed: files.length,
    files: important.map((entry) => ({ path: entry.path, size: entry.size, modifiedAt: entry.modified_at })),
  });
}

async function workspaceIndex(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const startedAtMs = performance.now();
  const maxFiles = clamp(numberArg(args, "maxFiles", 60), 1, 180);
  const maxScan = clamp(numberArg(args, "maxScan", input.preferences.maxIndexedFiles), 500, 20_000);
  const entries = await luxCommands.fsListFiles(maxScan);
  const files = entries.filter((entry) => entry.kind === "file" && !isLowSignalRelatedPath(entry.path));
  const descriptors = files.map((entry) => createRelatedFileDescriptor(entry, input.workspace?.root ?? ""));
  const projectSnapshot = input.workspace
    ? buildAiProjectIndexSnapshot(entries, {
        finishedAtMs: performance.now(),
        includeImages: input.preferences.includeImages,
        maxIndexedFiles: input.preferences.maxIndexedFiles,
        scanLimit: maxScan,
        source: "workspace-scan",
        startedAtMs,
        workspaceRoot: input.workspace.root,
      })
    : null;
  const byLanguage = topCounts(descriptors.map((file) => languageForPath(file.basenameLower)), 20);
  const byDirectory = topCounts(descriptors.map((file) => topDirectory(file.relativePath)), 24);
  const important = descriptors
    .filter(isImportantProjectFile)
    .sort((left, right) => scorePath(right.relativePath) - scorePath(left.relativePath) || left.relativeLower.localeCompare(right.relativeLower))
    .slice(0, maxFiles);
  const tests = descriptors.filter(isTestFile).sort(compareRelatedDescriptors).slice(0, maxFiles);
  const source = descriptors.filter((file) => isSourcePath(file) && !isTestFile(file)).sort(compareRelatedDescriptors).slice(0, maxFiles);
  const entrypoints = descriptors.filter(isEntrypointFile).sort(compareRelatedDescriptors).slice(0, maxFiles);
  const largest = [...descriptors]
    .sort((left, right) => (right.entry?.size ?? 0) - (left.entry?.size ?? 0) || left.relativeLower.localeCompare(right.relativeLower))
    .slice(0, Math.min(20, maxFiles));

  return toolJson("WorkspaceIndex", {
    workspaceRoot: input.workspace?.root ?? null,
    scanned: entries.length,
    indexedFiles: descriptors.length,
    truncated: entries.length >= maxScan,
    indexSettings: {
      enabled: input.preferences.projectIndexingEnabled,
      realtime: input.preferences.realtimeIndexing,
      maxIndexedFiles: input.preferences.maxIndexedFiles,
      scanLimit: maxScan,
      includeImages: input.preferences.includeImages,
    },
    indexHealth: projectSnapshot ? {
      quality: projectSnapshot.quality,
      source: projectSnapshot.source,
      scanLimit: projectSnapshot.scanLimit,
      scanTruncated: projectSnapshot.scanTruncated,
      ignoredFiles: projectSnapshot.ignoredFiles,
      truncatedFiles: projectSnapshot.truncatedFiles,
      sourceFiles: projectSnapshot.sourceFiles,
      testFiles: projectSnapshot.testFiles,
      rulesFiles: projectSnapshot.rulesFiles,
      docsFiles: projectSnapshot.docsFiles,
      memoryFiles: projectSnapshot.memoryFiles,
      durationMs: projectSnapshot.durationMs,
      contextAnchors: projectSnapshot.importantFiles,
    } : null,
    languageMix: byLanguage,
    topDirectories: byDirectory,
    importantFiles: important.map(compactIndexedFile),
    entrypoints: entrypoints.map(compactIndexedFile),
    sourceFiles: source.map(compactIndexedFile),
    testFiles: tests.map(compactIndexedFile),
    largestFiles: largest.map(compactIndexedFile),
  });
}

function activeContext(args: UnknownRecord, input: AiChatSendInput): ToolResult {
  const includeActiveText = booleanArg(args, "includeActiveText", false);
  const maxOpenDocuments = clamp(numberArg(args, "maxOpenDocuments", 24), 1, 80);
  const activePath = input.activeDocument?.path ?? input.activeDocument?.title ?? null;
  const openDocuments = input.openDocuments.slice(0, maxOpenDocuments).map((document) => ({
    id: document.id,
    path: document.path,
    title: document.title,
    language: document.language_id,
    dirty: document.is_dirty,
    untitled: document.is_untitled,
    active: document.id === input.activeDocument?.id,
    size: document.text.length,
    lines: countLines(document.text),
  }));
  return toolJson("ActiveContext", {
    workspace: input.workspace ? { name: input.workspace.name, root: input.workspace.root } : null,
    activeDocument: input.activeDocument ? {
      id: input.activeDocument.id,
      path: input.activeDocument.path,
      title: input.activeDocument.title,
      language: input.activeDocument.language_id,
      dirty: input.activeDocument.is_dirty,
      untitled: input.activeDocument.is_untitled,
      size: input.activeDocument.text.length,
      lines: countLines(input.activeDocument.text),
      text: includeActiveText ? truncateText(input.activeDocument.text, activeDocumentContextMaxChars) : undefined,
    } : null,
    openDocuments,
    openDocumentCount: input.openDocuments.length,
    dirtyDocuments: input.openDocuments
      .filter((document) => document.is_dirty)
      .map((document) => document.path ?? document.title),
    attachments: input.attachments.map((attachment) => ({ name: attachment.name, size: attachment.size, textLength: attachment.text.length })),
    terminal: compactTerminalContext(input, 4_000),
    chat: {
      currentMessage: input.message,
      historyMessages: input.history.length,
      lastUserMessage: [...input.history].reverse().find((message) => message.role === "user")?.content ?? null,
    },
    aiRuntime: {
      provider: input.provider.name,
      protocol: input.provider.protocol,
      baseUrl: input.provider.baseUrl,
      model: input.selectedModel.alias || input.selectedModel.id,
      reasoningEffort: input.preferences.selectedEffortId,
      agent: input.selectedAgentName || input.preferences.agentMode,
      toolApprovalMode: input.preferences.toolApprovalMode,
    },
    notes: [
      activePath ? `Active document is ${activePath}.` : "No active document is open.",
      input.preferences.toolApprovalMode === "full-access" ? "Dangerous tools auto-run inside workspace guards." : "Dangerous tools require explicit approval.",
    ],
  });
}

async function contextBudgeter(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message).trim();
  if (!query) throw new Error("ContextBudgeter requires a non-empty query.");
  const requestedTargetChars = clamp(numberArg(args, "targetChars", 16_000), 2_000, 22_000);
  const targetChars = Math.min(requestedTargetChars, maxToolOutputChars - 8_000);
  const maxItems = clamp(numberArg(args, "maxItems", 28), 4, 80);
  const includeActiveText = booleanArg(args, "includeActiveText", true);
  const includeOpenDocuments = booleanArg(args, "includeOpenDocuments", true);
  const includeToolContext = booleanArg(args, "includeToolContext", true);
  const queryTokens = tokenizeRelatedQuery(query);
  const items: ContextBudgetItem[] = [];

  addDirectContextBudgetItems(items, input, query, queryTokens, includeActiveText, includeOpenDocuments);

  const toolResults: PromiseSettledResult<ToolResult>[] = includeToolContext ? await Promise.allSettled([
    memoryContext({ query, maxFiles: 8, maxSignals: 24, includeRecentChat: true }, input),
    rulesContext({ query, maxFiles: 6 }, input),
    docsContext({ query, maxFiles: 6 }, input),
    relatedFiles({ query, path: input.activeDocument?.path ?? "", maxResults: 24 }, input),
    semanticSearch({ query, maxResults: 18 }, input),
    diagnosticsContext(40),
    gitContext(),
  ]) : [];
  if (includeToolContext) addToolContextBudgetItems(items, toolResults, queryTokens);

  const rankedItems = rankContextBudgetItems(items, queryTokens);
  const selected = selectContextBudgetItems(rankedItems, targetChars, maxItems);
  const selectedChars = selected.reduce((sum, item) => sum + item.content.length, 0);
  const dropped = rankedItems.length - selected.length;
  const byKind = topCounts(selected.map((item) => item.kind), 16);
  const packet = selected.map((item, index) => ({
    index: index + 1,
    id: item.id,
    kind: item.kind,
    source: item.source,
    path: item.path,
    line: item.line,
    reason: item.reason,
    chars: item.content.length,
    content: item.content,
  }));

  return toolJson("ContextBudgeter", {
    workspaceRoot: input.workspace?.root ?? null,
    query,
    budget: {
      requestedTargetChars,
      targetChars,
      selectedChars,
      utilization: targetChars > 0 ? Number((selectedChars / targetChars).toFixed(3)) : 0,
      candidateItems: rankedItems.length,
      selectedItems: selected.length,
      droppedItems: dropped,
      truncatedItems: selected.filter((item) => item.content.includes("...[truncated ")).length,
      maxItems,
    },
    byKind,
    packet,
    nextActions: buildContextBudgeterNextActions(selected),
    unavailable: contextBudgeterUnavailable(toolResults),
    notes: [
      "ContextBudgeter is read-only and returns a compact packet for the next reasoning step.",
      "Scores combine source priority, query-token hits, active editor state, diagnostics, git status, project rules, docs, memory, and related files.",
    ],
  });
}

async function globFiles(pattern: string, maxResults: number): Promise<ToolResult> {
  const files = await luxCommands.fsListFiles(Math.max(clamp(maxResults, 1, 500) * 4, 200));
  const needle = pattern.trim().toLowerCase();
  const matched = files
    .filter((entry) => entry.kind === "file")
    .filter((entry) => !needle || entry.path.toLowerCase().includes(needle))
    .slice(0, clamp(maxResults, 1, 500));
  return toolJson("Glob", {
    pattern,
    count: matched.length,
    files: matched.map((entry) => ({ path: entry.path, size: entry.size })),
  });
}

async function readFileTool(path: string, maxBytes: number): Promise<ToolResult> {
  const response = await luxCommands.fsReadText(path, clamp(maxBytes, 1_000, 1_000_000));
  return toolJson("Read", {
    path: response.path,
    size: response.size,
    truncated: response.truncated,
    text: truncateText(response.text, maxToolOutputChars),
  });
}

async function inspectFileTool(args: UnknownRecord): Promise<ToolResult> {
  const path = stringArg(args, "path").trim();
  if (!path) throw new Error("InspectFile requires a path.");
  const maxRows = clamp(numberArg(args, "maxRows", 80), 1, 500);
  const maxColumns = clamp(numberArg(args, "maxColumns", 24), 1, 200);
  const maxBytes = clamp(numberArg(args, "maxBytes", 120_000), 1_000, 1_000_000);
  const inspection = await luxCommands.fileInspect(path, {
    maxTextBytes: BigInt(maxBytes),
    maxRows,
    maxColumns,
    maxArchiveEntries: maxRows,
  });
  return toolJson("InspectFile", compactFileInspection(inspection, { maxBytes, maxColumns, maxRows }));
}

type InspectFileLimits = {
  maxBytes: number;
  maxColumns: number;
  maxRows: number;
};

function compactFileInspection(inspection: FileInspection, limits: InspectFileLimits) {
  const maxStringChars = clamp(Math.min(limits.maxBytes, 12_000), 1_000, 12_000);
  const maxAiContextChars = clamp(Math.min(Math.max(limits.maxBytes, 4_000), 10_000), 4_000, 10_000);
  return {
    path: inspection.path,
    title: inspection.title,
    descriptor: {
      ...inspection.descriptor,
      maxInlineBytes: jsonNumber(inspection.descriptor.maxInlineBytes),
    },
    metadata: inspection.metadata,
    truncated: inspection.truncated,
    warnings: inspection.warnings,
    preview: compactFilePreview(inspection.preview, limits, maxStringChars),
    aiContext: truncateText(inspection.aiContext, maxAiContextChars),
  };
}

function compactFilePreview(preview: FileInspection["preview"], limits: InspectFileLimits, maxStringChars: number): unknown {
  const maxCellChars = 800;
  switch (preview.kind) {
    case "text":
      return { ...preview, text: truncateText(preview.text, maxStringChars) };
    case "table":
      return {
        ...preview,
        headers: compactStringRow(preview.headers, limits.maxColumns, maxCellChars),
        rows: compactStringRows(preview.rows, limits, maxCellChars),
      };
    case "spreadsheet":
      return {
        ...preview,
        sheets: preview.sheets.map((sheet) => ({
          ...sheet,
          headers: compactStringRow(sheet.headers, limits.maxColumns, maxCellChars),
          rows: compactStringRows(sheet.rows, limits, maxCellChars),
        })),
      };
    case "database":
      return {
        ...preview,
        tables: preview.tables.map((table) => ({
          ...table,
          columns: table.columns.slice(0, limits.maxColumns),
          rows: compactStringRows(table.rows, limits, maxCellChars),
        })),
      };
    case "pdf":
    case "office":
      return { ...preview, text: truncateText(preview.text, maxStringChars) };
    case "notebook":
      return {
        ...preview,
        cells: preview.cells.slice(0, limits.maxRows).map((cell) => ({
          ...cell,
          text: truncateText(cell.text, maxStringChars),
          outputText: truncateText(cell.outputText, maxStringChars),
        })),
      };
    case "binary":
      return {
        ...preview,
        hex: truncateText(preview.hex, maxStringChars),
        ascii: truncateText(preview.ascii, maxStringChars),
      };
    default:
      return preview;
  }
}

function compactStringRows(rows: string[][], limits: InspectFileLimits, maxStringChars: number) {
  return rows.slice(0, limits.maxRows).map((row) => compactStringRow(row, limits.maxColumns, maxStringChars));
}

function compactStringRow(row: string[], maxColumns: number, maxStringChars: number) {
  return row.slice(0, maxColumns).map((cell) => truncateText(cell, maxStringChars));
}

function jsonNumber(value: bigint | number | null) {
  if (value === null) return null;
  if (typeof value === "number") return value;
  const numeric = Number(value);
  return Number.isSafeInteger(numeric) ? numeric : value.toString();
}

async function semanticSearch(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message).trim();
  if (!query) throw new Error("SemanticSearch requires a non-empty query.");
  const maxResults = clamp(numberArg(args, "maxResults", 24), 1, 80);
  const pathFilter = normalizePathSlashes(stringArg(args, "path", "")).toLowerCase();
  const queryTokens = tokenizeRelatedQuery(query);
  const workspaceRoot = input.workspace?.root ?? "";

  const [symbolsResult, searchResult, filesResult] = await Promise.allSettled([
    luxCommands.lspWorkspaceSymbols(query),
    luxCommands.searchQuery(query, {
      case_sensitive: false,
      whole_word: false,
      use_regex: false,
      include_hidden: false,
      include_globs: [],
      exclude_globs: [],
      max_results: Math.min(120, Math.max(maxResults * 4, 40)),
    }),
    luxCommands.fsListFiles(clamp(input.preferences.maxIndexedFiles, 500, 20_000)),
  ]);

  const results = new Map<string, SemanticSearchResult>();
  const symbols = symbolsResult.status === "fulfilled" ? symbolsResult.value : [];
  for (const symbol of symbols) {
    const path = normalizePathSlashes(symbol.location.path);
    if (!passesSemanticPathFilter(path, pathFilter)) continue;
    const score = scoreSemanticSymbol(symbol, query, queryTokens, path, workspaceRoot);
    upsertSemanticResult(results, {
      type: "symbol",
      source: "lsp-symbols",
      score,
      path,
      relativePath: createRelatedFileDescriptor({ path }, workspaceRoot).relativePath,
      line: symbol.location.range.start_line + 1,
      column: symbol.location.range.start_column + 1,
      name: symbol.name,
      kind: String(symbol.kind),
      containerName: symbol.container_name,
      preview: [symbol.container_name, symbol.name].filter(Boolean).join("."),
    });
  }

  const search = searchResult.status === "fulfilled" ? searchResult.value : null;
  for (const hit of search?.hits ?? []) {
    const path = normalizePathSlashes(hit.path);
    if (!passesSemanticPathFilter(path, pathFilter)) continue;
    const score = scoreSemanticTextHit(path, hit.preview, hit.match_text, queryTokens, workspaceRoot);
    upsertSemanticResult(results, {
      type: "text",
      source: "indexed-search",
      score,
      path,
      relativePath: createRelatedFileDescriptor({ path }, workspaceRoot).relativePath,
      line: hit.line,
      column: hit.column,
      matchText: hit.match_text,
      preview: hit.preview,
    });
  }

  const entries = filesResult.status === "fulfilled" ? filesResult.value : [];
  const fileCandidates = entries
    .filter((entry) => entry.kind === "file" && !isLowSignalRelatedPath(entry.path))
    .map((entry) => createRelatedFileDescriptor(entry, workspaceRoot))
    .filter((file) => passesSemanticPathFilter(file.path, pathFilter))
    .map((file) => ({ file, score: scoreSemanticFile(file, queryTokens) }))
    .filter((item) => item.score > 0)
    .sort((left, right) => right.score - left.score || left.file.relativeLower.localeCompare(right.file.relativeLower))
    .slice(0, Math.min(maxResults * 2, 80));
  for (const { file, score } of fileCandidates) {
    upsertSemanticResult(results, {
      type: "file",
      source: "workspace-index",
      score,
      path: file.path,
      relativePath: file.relativePath,
      name: file.basename,
      kind: languageForPath(file.basenameLower),
      preview: file.relativePath,
    });
  }

  const ranked = Array.from(results.values())
    .sort((left, right) => right.score - left.score || left.path.localeCompare(right.path) || (left.line ?? 0) - (right.line ?? 0))
    .slice(0, maxResults);

  return toolJson("SemanticSearch", {
    workspaceRoot: input.workspace?.root ?? null,
    query,
    pathFilter: pathFilter || null,
    count: ranked.length,
    results: ranked,
    unavailable: {
      symbols: symbolsResult.status === "rejected" ? readErrorMessage(symbolsResult.reason) : null,
      textSearch: searchResult.status === "rejected" ? readErrorMessage(searchResult.reason) : null,
      workspaceIndex: filesResult.status === "rejected" ? readErrorMessage(filesResult.reason) : null,
    },
  });
}

function todoWrite(args: UnknownRecord, session: RuntimeToolSession): ToolResult {
  const rawTodos = args.todos;
  if (!Array.isArray(rawTodos)) throw new Error("TodoWrite requires a todos array.");
  const todos = rawTodos.map(normalizeSessionTodo).filter((todo): todo is SessionTodo => Boolean(todo));
  if (todos.length === 0) throw new Error("TodoWrite requires at least one valid todo item.");
  session.todos = todos;
  const statusCounts = topCounts(todos.map((todo) => todo.status), 8);
  return toolJson("TodoWrite", {
    count: todos.length,
    statusCounts,
    todos,
    notes: ["This task list is scoped to the current AI response and does not modify workspace files."],
  });
}

async function webFetchTool(args: UnknownRecord): Promise<ToolResult> {
  const url = stringArg(args, "url", "").trim();
  if (!url) throw new Error("WebFetch requires a URL.");
  const maxBytes = clamp(numberArg(args, "maxBytes", 250_000), 1_024, 1_000_000);
  const timeoutSecs = clamp(numberArg(args, "timeoutSecs", 20), 1, 60);
  const allowPrivateHosts = booleanArg(args, "allowPrivateHosts", false);
  const response = await luxCommands.webFetch(url, maxBytes, timeoutSecs, allowPrivateHosts);
  const scan = scanSecrets(response.text, response.finalUrl || response.url);
  return toolJson("WebFetch", {
    url: response.url,
    finalUrl: response.finalUrl,
    status: response.status,
    contentType: response.contentType,
    title: response.title,
    bytesRead: response.bytesRead,
    truncated: response.truncated,
    elapsedMs: response.elapsedMs,
    text: scan.redactedText,
    secretGuard: {
      redacted: scan.findings.length > 0,
      findingCount: scan.findings.length,
      findings: scan.findings.slice(0, 20).map(publicSecretFinding),
    },
  });
}

async function writeFileTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const path = stringArg(args, "path");
  const text = stringArg(args, "text");
  const overwrite = booleanArg(args, "overwrite", false);
  const saveToDisk = booleanArg(args, "saveToDisk", true);
  const approval = createWriteApproval(input.locale, path, text, overwrite, saveToDisk);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiFileWrite(
    path,
    text,
    overwrite,
    saveToDisk,
  );
  return toolResultFromFileOperation("Write", result);
}

async function strReplaceTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const path = stringArg(args, "path");
  const oldText = stringArg(args, "oldText");
  const newText = stringArg(args, "newText");
  const expectedReplacements = clamp(numberArg(args, "expectedReplacements", 1), 1, 1000);
  const saveToDisk = booleanArg(args, "saveToDisk", true);
  const approval = createStrReplaceApproval(input.locale, path, oldText, newText, expectedReplacements, saveToDisk);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiFileStrReplace(
    path,
    oldText,
    newText,
    expectedReplacements,
    saveToDisk,
  );
  return toolResultFromFileOperation("StrReplace", result);
}

async function patchEngineTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const operations = patchOperationsArg(args);
  const saveToDisk = booleanArg(args, "saveToDisk", true);
  const dryRun = booleanArg(args, "dryRun", false);
  const approval = createPatchApproval(input.locale, operations, saveToDisk, dryRun);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiFilePatch(operations, saveToDisk, dryRun);
  return toolResultFromFileOperation("PatchEngine", result);
}

async function deleteFileTool(path: string, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const approval = createDeleteApproval(input.locale, path);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiFileDelete(path);
  return toolResultFromFileOperation("Delete", result);
}

async function shellTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const command = stringArg(args, "command");
  const cwd = stringArg(args, "cwd", input.workspace?.root ?? "");
  const timeoutSecs = clamp(numberArg(args, "timeoutSecs", 120), 1, 600);
  const approval = createShellApproval(input.locale, command, cwd, timeoutSecs);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiShell(command, cwd || null, timeoutSecs);
  const stdoutScan = scanSecrets(result.stdout, "shell.stdout");
  const stderrScan = scanSecrets(result.stderr, "shell.stderr");
  const secretFindings = [...stdoutScan.findings, ...stderrScan.findings];
  return toolJson("Shell", {
    workspaceRoot: result.workspaceRoot,
    cwd: result.cwd,
    command: result.command,
    exitCode: result.exitCode,
    durationMs: result.durationMs,
    timedOut: result.timedOut,
    stdout: stdoutScan.redactedText,
    stderr: stderrScan.redactedText,
    secretGuard: {
      redacted: secretFindings.length > 0,
      findingCount: secretFindings.length,
      findings: secretFindings.slice(0, 20).map(publicSecretFinding),
    },
  });
}

function terminalContextTool(args: UnknownRecord, input: AiChatSendInput): ToolResult {
  const sessionId = stringArg(args, "sessionId", "").trim();
  const maxChars = clamp(numberArg(args, "maxChars", 12_000), 500, 24_000);
  return toolJson("TerminalContext", compactTerminalContext(input, maxChars, sessionId || undefined));
}

async function terminalWriteTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const data = stringArg(args, "data");
  if (!data) throw new Error("TerminalWrite requires non-empty data.");
  const session = selectTerminalSession(input, stringArg(args, "sessionId", ""));
  if (!session) throw new Error("TerminalWrite requires an active terminal session.");
  const approval = createTerminalWriteApproval(input.locale, session, data);
  await requireToolApproval(input, ui, approval);
  await luxCommands.terminalWrite(session.id, data);
  return toolJson("TerminalWrite", {
    session: compactTerminalSession(session, input, 1_200),
    bytesWritten: data.length,
    preview: terminalWritePreview(data),
  });
}

async function requireToolApproval(input: AiChatSendInput, ui: ToolExecutionUi, approval: AiToolApprovalRequest) {
  throwIfAborted(input.abortSignal);
  if (input.preferences.toolApprovalMode === "full-access") {
    ui.setRunning({ ...approval, decision: "approved" });
    return;
  }
  ui.setApproval(approval);
  const decision = await input.onToolApproval(approval);
  throwIfAborted(input.abortSignal);
  const approvalState = { ...approval, decision };
  if (decision !== "approved") {
    ui.setApproval(approvalState);
    throw new ToolApprovalRejectedError(`${approval.tool} was rejected by the user.`);
  }
  ui.setRunning(approvalState);
}

class ToolApprovalRejectedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ToolApprovalRejectedError";
  }
}

async function grepTool(args: UnknownRecord): Promise<ToolResult> {
  const query = stringArg(args, "query");
  const maxResults = clamp(numberArg(args, "maxResults", 50), 1, 200);
  const response = await luxCommands.searchQuery(query, {
    case_sensitive: booleanArg(args, "caseSensitive", false),
    whole_word: false,
    use_regex: booleanArg(args, "useRegex", false),
    include_hidden: false,
    include_globs: stringArrayArg(args, "includeGlobs"),
    exclude_globs: [],
    max_results: maxResults,
  });
  return toolJson("Grep", {
    query: response.query,
    truncated: response.truncated,
    elapsedMs: response.elapsed_ms,
    hits: response.hits.map((hit) => ({
      path: hit.path,
      line: hit.line,
      column: hit.column,
      preview: hit.preview,
    })),
  });
}

async function symbolContext(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message);
  const path = stringArg(args, "path", input.activeDocument?.path ?? "");
  const line = optionalPositiveNumberArg(args, "line");
  const column = optionalPositiveNumberArg(args, "column");
  const maxResults = clamp(numberArg(args, "maxResults", 80), 1, 300);
  const response = await luxCommands.aiSymbolContext(
    query.trim() || null,
    path.trim() || null,
    line,
    column,
    maxResults,
  );
  return toolJson("SymbolContext", {
    workspaceRoot: response.workspaceRoot,
    query: response.query,
    path: response.path,
    position: response.position,
    workspaceSymbols: response.workspaceSymbols.map((symbol) => ({
      name: symbol.name,
      kind: symbol.kind,
      containerName: symbol.container_name,
      location: compactLocation(symbol.location),
    })),
    documentSymbols: response.documentSymbols.map(compactDocumentSymbol),
    hover: response.hover ? {
      contents: response.hover.contents,
      range: response.hover.range,
    } : null,
    definitions: response.definitions.map(compactLocation),
    references: response.references.map(compactLocation),
    signatureHelp: response.signatureHelp ? {
      activeSignature: response.signatureHelp.active_signature,
      activeParameter: response.signatureHelp.active_parameter,
      signatures: response.signatureHelp.signatures.slice(0, 12).map((signature) => ({
        label: signature.label,
        documentation: signature.documentation,
        parameters: signature.parameters.map((parameter) => ({
          label: parameter.label,
          documentation: parameter.documentation,
        })),
      })),
    } : null,
    diagnostics: response.diagnostics
      .filter((diagnostic) => !response.path || normalizePathForCompare(diagnostic.path) === normalizePathForCompare(response.path))
      .slice(0, 80)
      .map((diagnostic) => ({
        path: diagnostic.path,
        line: diagnostic.line,
        column: diagnostic.column,
        severity: diagnostic.severity,
        source: diagnostic.source,
        message: diagnostic.message,
      })),
    notes: response.notes,
  });
}

async function relatedFiles(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const path = stringArg(args, "path", input.activeDocument?.path ?? "");
  const query = stringArg(args, "query", input.message);
  const maxResults = clamp(numberArg(args, "maxResults", 40), 1, 120);
  const scanLimit = clamp(input.preferences.maxIndexedFiles, 500, 20_000);
  const entries = await luxCommands.fsListFiles(scanLimit);
  const workspaceRoot = input.workspace?.root ?? "";
  const targetPath = path.trim() ? resolveWorkspacePath(path, workspaceRoot) : "";
  const target = targetPath ? createRelatedFileDescriptor({ path: targetPath }, workspaceRoot) : null;
  const queryTokens = tokenizeRelatedQuery(query);
  const matches = new Map<string, RelatedFileMatch>();

  for (const entry of entries) {
    if (entry.kind !== "file" || isLowSignalRelatedPath(entry.path)) continue;
    const descriptor = createRelatedFileDescriptor(entry, workspaceRoot);
    if (target && descriptor.lower === target.lower) continue;

    const match = scoreRelatedFile(descriptor, target, queryTokens);
    if (match.score <= 0) continue;
    matches.set(descriptor.lower, match);
  }

  const related = Array.from(matches.values())
    .sort((left, right) => right.score - left.score || left.descriptor.relativeLower.localeCompare(right.descriptor.relativeLower))
    .slice(0, maxResults);

  return toolJson("RelatedFiles", {
    workspaceRoot: input.workspace?.root ?? null,
    target: target ? {
      path: target.path,
      relativePath: target.relativePath,
      basename: target.basename,
      familyStem: target.familyStem,
    } : null,
    query,
    scanned: entries.filter((entry) => entry.kind === "file").length,
    count: related.length,
    files: related.map((match) => ({
      path: match.descriptor.path,
      relativePath: match.descriptor.relativePath,
      relations: Array.from(match.relations).sort(),
      score: match.score,
      queryHits: match.queryHits,
      size: match.descriptor.entry?.size ?? null,
      modifiedAt: match.descriptor.entry?.modified_at ?? null,
    })),
  });
}

async function impactAnalysis(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message);
  const path = stringArg(args, "path", input.activeDocument?.path ?? "");
  const maxResults = clamp(numberArg(args, "maxResults", 32), 1, 100);
  const [relatedResult, diagnosticsResult, symbolsResult, rulesResult, docsResult] = await Promise.allSettled([
    relatedFiles({ path, query, maxResults }, input),
    diagnosticsContext(80),
    symbolContext({ query, path, maxResults: 80 }, input),
    rulesContext({ query, maxFiles: 6 }, input),
    docsContext({ query, maxFiles: 6 }, input),
  ]);
  const related = parseToolContent(relatedResult);
  const diagnostics = parseToolContent(diagnosticsResult);
  const symbols = parseToolContent(symbolsResult);
  const relatedFilesList = Array.isArray(related?.files) ? related.files.filter(isRecord) : [];
  const diagnosticsList = Array.isArray(diagnostics?.diagnostics) ? diagnostics.diagnostics.filter(isRecord) : [];
  const symbolFiles = collectSymbolFiles(symbols).slice(0, maxResults);
  const riskSignals = buildImpactRiskSignals(relatedFilesList, diagnosticsList, symbolFiles);
  const validation = buildImpactValidation(relatedFilesList, query);

  return toolJson("ImpactAnalysis", {
    workspaceRoot: input.workspace?.root ?? null,
    target: path || input.activeDocument?.path || input.activeDocument?.title || null,
    query,
    riskLevel: riskSignals.some((signal) => signal.level === "high") ? "high" : riskSignals.some((signal) => signal.level === "medium") ? "medium" : "low",
    affectedFiles: relatedFilesList.slice(0, maxResults).map((file) => ({
      path: file.path,
      relativePath: file.relativePath,
      relations: file.relations,
      score: file.score,
    })),
    symbolFiles,
    diagnostics: diagnosticsList.slice(0, 24),
    riskSignals,
    validation,
    rules: parseToolContent(rulesResult),
    docs: parseToolContent(docsResult),
  });
}

function normalizeSessionTodo(value: unknown, index: number): SessionTodo | null {
  if (!isRecord(value)) return null;
  const content = typeof value.content === "string" ? value.content.trim() : "";
  if (!content) return null;
  const id = typeof value.id === "string" && value.id.trim() ? value.id.trim() : `todo-${index + 1}`;
  const status = normalizeSessionTodoStatus(value.status);
  const priority = normalizeSessionTodoPriority(value.priority);
  const notes = typeof value.notes === "string" && value.notes.trim() ? truncateText(value.notes.trim(), 500) : undefined;
  return { id, content: truncateText(content, 500), status, priority, notes };
}

function normalizeSessionTodoStatus(value: unknown): SessionTodoStatus {
  const normalized = typeof value === "string" ? value.toLowerCase().replace(/[-\s]+/g, "_") : "";
  switch (normalized) {
    case "in_progress":
    case "completed":
    case "blocked":
    case "cancelled":
      return normalized;
    default:
      return "pending";
  }
}

function normalizeSessionTodoPriority(value: unknown): SessionTodoPriority {
  const normalized = typeof value === "string" ? value.toLowerCase() : "";
  switch (normalized) {
    case "low":
    case "high":
      return normalized;
    default:
      return "medium";
  }
}

function collectSymbolFiles(symbols: UnknownRecord) {
  const paths = new Set<string>();
  const collectLocation = (value: unknown) => {
    if (!isRecord(value)) return;
    if (typeof value.path === "string") paths.add(value.path);
    if (isRecord(value.location)) collectLocation(value.location);
  };
  for (const key of ["workspaceSymbols", "definitions", "references"]) {
    const values = symbols[key];
    if (Array.isArray(values)) values.forEach(collectLocation);
  }
  return Array.from(paths);
}

function buildImpactRiskSignals(relatedFiles: UnknownRecord[], diagnostics: UnknownRecord[], symbolFiles: string[]) {
  const signals: Array<{ level: "low" | "medium" | "high"; message: string }> = [];
  if (diagnostics.length > 0) signals.push({ level: "high", message: `${diagnostics.length} existing diagnostic(s) may mask or compound this change.` });
  if (relatedFiles.some((file) => Array.isArray(file.relations) && file.relations.includes("schema"))) signals.push({ level: "high", message: "Schema/model/migration files are in scope; check persistence and API contracts." });
  if (relatedFiles.some((file) => Array.isArray(file.relations) && file.relations.includes("entrypoint"))) signals.push({ level: "medium", message: "Entrypoints are related; test startup and core flows." });
  if (relatedFiles.some((file) => Array.isArray(file.relations) && file.relations.includes("test"))) signals.push({ level: "low", message: "Related tests were found and should be run after edits." });
  if (symbolFiles.length > 12) signals.push({ level: "medium", message: `${symbolFiles.length} symbol-linked file(s) suggest a broader API surface.` });
  if (signals.length === 0) signals.push({ level: "low", message: "No broad blast-radius signals found in the current indexed context." });
  return signals;
}

function buildImpactValidation(relatedFiles: UnknownRecord[], query: string) {
  const checks = new Set<string>();
  const paths = relatedFiles.map((file) => typeof file.relativePath === "string" ? file.relativePath.toLowerCase() : "");
  if (paths.some((path) => /package\.json|pnpm-lock|yarn\.lock|package-lock/.test(path))) checks.add("Run the package manager test/build commands affected by dependency or script changes.");
  if (paths.some((path) => path.endsWith("cargo.toml") || path.endsWith(".rs"))) checks.add("Run the relevant Cargo tests or cargo check for Rust changes.");
  if (paths.some((path) => /\.(ts|tsx|js|jsx)$/.test(path))) checks.add("Run TypeScript typecheck and the nearest JS/TS test suite.");
  if (paths.some((path) => /\.(css|scss|sass|less)$/.test(path))) checks.add("Verify the affected UI in browser at desktop and mobile widths.");
  if (/test|spec|coverage/i.test(query)) checks.add("Run focused tests first, then the broader suite if shared code changed.");
  if (checks.size === 0) checks.add("Run the smallest relevant build/test command, then broaden if shared files changed.");
  return Array.from(checks).slice(0, 8);
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

function createRunningToolCall(call: OpenAiToolCall): AiChatToolCall {
  const name = call.function?.name || "Tool";
  return {
    id: call.id ?? crypto.randomUUID(),
    tool: name,
    status: "running",
    input: summarizeToolInput(call.function?.arguments),
    startTime: Date.now(),
  };
}

function summarizeToolInput(value: string | undefined) {
  if (!value) return "";
  try {
    const parsed = JSON.parse(value) as unknown;
    if (isRecord(parsed)) {
      const primaryKey = ["path", "query", "command", "pattern", "url", "cwd"].find((key) => typeof parsed[key] === "string" && String(parsed[key]).trim());
      if (primaryKey) return sanitizeToolSummary(String(parsed[primaryKey]));
      return truncateText(Object.entries(parsed).map(([key, entry]) => `${key}: ${formatToolInputValue(entry)}`).join(", "), 180);
    }
  } catch {
    return sanitizeToolSummary(truncateText(value, 180));
  }
  return sanitizeToolSummary(truncateText(value, 180));
}

function formatToolInputValue(value: unknown) {
  if (typeof value === "string") return sanitizeToolSummary(value);
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) return `${value.length} item${value.length === 1 ? "" : "s"}`;
  if (isRecord(value)) return "object";
  return String(value ?? "");
}

function sanitizeToolSummary(value: string) {
  return truncateText(normalizePathSlashes(value.trim()).replace(/^\/\/\?\//, "").replace(/^\/\?\//, ""), 180);
}

function parseToolArguments(value: string | undefined): UnknownRecord {
  if (!value) return {};
  try {
    const parsed = JSON.parse(value) as unknown;
    return isRecord(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

// Output shown in the expandable tool card: the human title plus the actual
// tool result body, so the user sees what the tool really did (not a stub label).
function formatToolOutput(result: ToolResult): string {
  const title = result.title?.trim() ?? "";
  const body = result.content?.trim() ?? "";
  if (!body) return title;
  if (!title || body.startsWith(title)) return body;
  return `${title}\n\n${body}`;
}

function toolResultFromFileOperation(title: string, result: FileToolResult): ToolResult {
  return {
    title: result.message,
    content: truncateText(JSON.stringify({
      operation: result.operation,
      path: result.path,
      savedToDisk: result.savedToDisk,
      changedPaths: result.changedPaths,
      stats: result.stats,
      message: result.message,
    }, null, 2), maxToolOutputChars),
    stats: result.stats,
  };
}

function settledContent(name: string, result: PromiseSettledResult<ToolResult>) {
  if (result.status === "fulfilled") return `## ${name}\n${result.value.content}`;
  return `## ${name}\n${JSON.stringify({ error: readErrorMessage(result.reason) })}`;
}

function compactLocation(location: LspLocation) {
  return {
    path: location.path,
    range: location.range,
  };
}

function compactDocumentSymbol(symbol: LspDocumentSymbol): unknown {
  return {
    name: symbol.name,
    detail: symbol.detail,
    kind: symbol.kind,
    range: symbol.range,
    selectionRange: symbol.selection_range,
    children: symbol.children.map(compactDocumentSymbol),
  };
}

function normalizePathForCompare(path: string) {
  return path.replaceAll("\\", "/").toLowerCase();
}

function throwIfAborted(signal: AbortSignal) {
  if (signal.aborted) throw new DOMException("AI request was cancelled", "AbortError");
}

function isAbortErrorLike(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}
