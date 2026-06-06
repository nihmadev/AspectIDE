import type { AiChatSendInput } from "./aiChatTypes";
import {
  createRelatedFileDescriptor,
  isLowSignalRelatedPath,
  languageForPath,
  passesSemanticPathFilter,
  scoreSemanticFile,
  scoreSemanticSymbol,
  scoreSemanticTextHit,
  tokenizeRelatedQuery,
  upsertSemanticResult,
  type SemanticSearchResult,
} from "./aiRuntimeFileContext";
import { clamp, normalizePathSlashes, numberArg, readErrorMessage, stringArg, toolJson, type ToolResult, type UnknownRecord } from "./aiRuntimeShared";
import { isTauriRuntime, luxCommands } from "./tauri";

export async function semanticSearch(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message).trim();
  if (!query) throw new Error("SemanticSearch requires a non-empty query.");
  const maxResults = clamp(numberArg(args, "maxResults", 24), 1, 80);
  const pathArg = stringArg(args, "path", "").trim();

  // Native path: one Rust command composes LSP + text search + file index and ranks
  // natively. The TS pipeline below is the browser/dev-runtime fallback only.
  if (isTauriRuntime()) {
    const native = await luxCommands.aiSemanticSearch(
      query,
      pathArg || null,
      maxResults,
      clamp(input.preferences.maxIndexedFiles, 500, 20_000),
    );
    return toolJson("SemanticSearch", {
      workspaceRoot: native.workspaceRoot,
      query: native.query,
      pathFilter: native.pathFilter,
      count: native.count,
      results: native.results,
    });
  }

  const pathFilter = normalizePathSlashes(pathArg).toLowerCase();
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