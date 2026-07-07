import type { AiChatSendInput } from "./aiChatTypes";
import { recordContextBudgetReport } from "./aiChatContextReport";
import { buildAiProjectIndexSnapshot } from "./aiProjectIndex";
import {
  addDirectContextBudgetItems,
  addToolContextBudgetItems,
  buildContextBudgeterNextActions,
  contextBudgeterUnavailable,
  rankContextBudgetItems,
  selectContextBudgetItems,
  type ContextBudgetItem,
} from "./aiRuntimeContextBudget";
import { docsContext, memoryContext, rulesContext } from "./aiRuntimeContextSources";
import { diagnosticsContext, gitContext } from "./aiRuntimeDiagnostics";
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
  scorePath,
  tokenizeRelatedQuery,
  topDirectory,
} from "./aiRuntimeFileContext";
import { normalizePathSlashes } from "./aiRuntimeShared";
import { countLines } from "./aiRuntimePatch";
import { activeDocumentContextMaxChars } from "./aiRuntimePrompt";
import { globFiles, grepTool, impactAnalysis, relatedFiles } from "./aiRuntimeExploreTools";
import { semanticSearch } from "./aiRuntimeSemanticSearch";
import {
  booleanArg,
  clamp,
  maxToolOutputChars,
  numberArg,
  readErrorMessage,
  stringArg,
  toolJson,
  topCounts,
  truncateText,
  type ToolResult,
  type UnknownRecord,
} from "./aiRuntimeShared";
import { compactTerminalContext } from "./aiRuntimeTerminal";
import { isTauriRuntime, luxCommands } from "./tauri";

export async function fastContext(input: AiChatSendInput, query: string): Promise<ToolResult> {
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

export async function repoMap(maxFiles: number): Promise<ToolResult> {
  if (isTauriRuntime()) {
    const native = await luxCommands.aiRepoMap(clamp(maxFiles, 1, 500));
    return toolJson("RepoMap", native);
  }
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

async function resolveIndexLanguages(entries: import("./types").FsEntry[], workspaceRoot: string, options: import("./aiProjectIndex").BuildAiProjectIndexOptions): Promise<import("./aiProjectIndex").AiProjectIndexSnapshot> {
  if (isTauriRuntime()) {
    const root = normalizePathSlashes(workspaceRoot).replace(/\/+$/, "");
    const fileEntries = entries.filter((e) => e.kind === "file");
    const filePaths = fileEntries.map((e) => normalizePathSlashes(e.path));
    if (filePaths.length > 0) {
      const langs = await luxCommands.resolveFileLanguages(filePaths);
      const langMap = new Map<string, string>();
      for (let i = 0; i < filePaths.length; i++) {
        const rp = root && filePaths[i].toLowerCase().startsWith(`${root.toLowerCase()}/`)
          ? filePaths[i].slice(root.length + 1)
          : filePaths[i];
        langMap.set(rp, langs[i]);
      }
      return buildAiProjectIndexSnapshot(entries, options, (relativePath) => langMap.get(relativePath) ?? null);
    }
  }
  return buildAiProjectIndexSnapshot(entries, options);
}

export async function workspaceIndex(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const startedAtMs = performance.now();
  const maxFiles = clamp(numberArg(args, "maxFiles", 60), 1, 180);
  const maxScan = clamp(numberArg(args, "maxScan", input.preferences.maxIndexedFiles), 500, 20_000);

  // Native Rust path for the heavy file-scan + categorize; index-health/settings
  // are still computed in TS from preferences (thin overlay on top of the Rust base).
  if (isTauriRuntime()) {
    const native = await luxCommands.aiWorkspaceIndex(maxFiles, maxScan);
    const entries = await luxCommands.fsListFiles(maxScan);
    const projectSnapshot = input.workspace
      ? await resolveIndexLanguages(entries, input.workspace.root, {
          finishedAtMs: performance.now(),
          includeImages: input.preferences.includeImages,
          maxIndexedFiles: input.preferences.maxIndexedFiles,
          scanLimit: maxScan,
          source: "workspace-scan",
          startedAtMs,
          workspaceRoot: input.workspace.root,
        })
      : null;
    return toolJson("WorkspaceIndex", {
      ...native,
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
      } : null,
    });
  }

  const entries = await luxCommands.fsListFiles(maxScan);
  const files = entries.filter((entry) => entry.kind === "file" && !isLowSignalRelatedPath(entry.path));
  const descriptors = files.map((entry) => createRelatedFileDescriptor(entry, input.workspace?.root ?? ""));
  const projectSnapshot = input.workspace
    ? await resolveIndexLanguages(entries, input.workspace.root, {
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

export function activeContext(args: UnknownRecord, input: AiChatSendInput): ToolResult {
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

export async function contextBudgeter(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message).trim();
  if (!query) throw new Error("ContextBudgeter requires a non-empty query.");
  const requestedTargetChars = clamp(numberArg(args, "targetChars", 16_000), 2_000, 22_000);
  const targetChars = Math.min(requestedTargetChars, maxToolOutputChars - 8_000);
  const maxItems = clamp(numberArg(args, "maxItems", 28), 4, 80);
  // Default OFF to match the tool schema and avoid silently sending active-file
  // text / unsaved open-tab excerpts into the packet (token cost + privacy). The
  // model must opt in explicitly when it wants editor contents.
  const includeActiveText = booleanArg(args, "includeActiveText", false);
  const includeOpenDocuments = booleanArg(args, "includeOpenDocuments", false);
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
  recordContextBudgetReport(
    input.chatSessionId,
    query,
    rankedItems,
    selected,
    targetChars,
    maxItems,
    (sessionId, report) => input.onContextBudgetReport?.(report),
  );
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

function settledContent(name: string, result: PromiseSettledResult<ToolResult>) {
  if (result.status === "fulfilled") return `## ${name}\n${result.value.content}`;
  return `## ${name}\n${JSON.stringify({ error: readErrorMessage(result.reason) })}`;
}