import type { AiChatSendInput } from "./aiChatTypes";
import { docsContext, rulesContext } from "./aiRuntimeContextSources";
import { diagnosticsContext, gitContext } from "./aiRuntimeDiagnostics";
import { parseToolContent } from "./aiRuntimeContextBudget";
import { publicSecretFinding, scanSecrets } from "./aiRuntimeSecretGuard";
import {
  compareRelatedDescriptors,
  createRelatedFileDescriptor,
  isLowSignalRelatedPath,
  resolveWorkspacePath,
  scoreRelatedFile,
  tokenizeRelatedQuery,
  type RelatedFileMatch,
} from "./aiRuntimeFileContext";
import {
  booleanArg,
  clamp,
  isRecord,
  maxToolOutputChars,
  numberArg,
  optionalPositiveNumberArg,
  stringArg,
  stringArrayArg,
  toolJson,
  truncateText,
  type ToolResult,
  type UnknownRecord,
} from "./aiRuntimeShared";
import { isTauriRuntime, luxCommands } from "./tauri";
import type { FileInspection, LspDocumentSymbol, LspLocation } from "./types";

export async function globFiles(pattern: string, maxResults: number): Promise<ToolResult> {
  const cap = clamp(maxResults, 1, 500);
  const limit = Math.max(cap * 4, 200);
  const files = await luxCommands.fsListFiles(limit);
  const needle = pattern.trim().toLowerCase();
  const allMatched = files
    .filter((entry) => entry.kind === "file")
    .filter((entry) => !needle || entry.path.toLowerCase().includes(needle));
  const matched = allMatched.slice(0, cap);
  return toolJson("Glob", {
    pattern,
    count: matched.length,
    truncated: allMatched.length > cap || files.length >= limit,
    files: matched.map((entry) => ({ path: entry.path, size: entry.size })),
  });
}

export async function readFileTool(path: string, maxBytes: number): Promise<ToolResult> {
  const response = await luxCommands.fsReadText(sanitizeWorkspaceToolPath(path), clamp(maxBytes, 1_000, 1_000_000));
  return toolJson("Read", {
    path: response.path,
    size: response.size,
    truncated: response.truncated,
    text: truncateText(response.text, maxToolOutputChars),
  });
}

export async function inspectFileTool(args: UnknownRecord): Promise<ToolResult> {
  const path = stringArg(args, "path").trim();
  if (!path) throw new Error("InspectFile requires a path.");
  const maxRows = clamp(numberArg(args, "maxRows", 80), 1, 500);
  const maxColumns = clamp(numberArg(args, "maxColumns", 24), 1, 200);
  const maxBytes = clamp(numberArg(args, "maxBytes", 120_000), 1_000, 1_000_000);
  const inspection = await luxCommands.fileInspect(path, {
    maxTextBytes: Number(maxBytes),
    maxRows,
    maxColumns,
    maxArchiveEntries: maxRows,
  });
  return toolJson("InspectFile", compactFileInspection(inspection, { maxBytes, maxColumns, maxRows }));
}

export async function grepTool(args: UnknownRecord): Promise<ToolResult> {
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

export async function symbolContext(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
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

export async function relatedFiles(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const path = stringArg(args, "path", input.activeDocument?.path ?? "");
  const query = stringArg(args, "query", input.message);
  const maxResults = clamp(numberArg(args, "maxResults", 40), 1, 120);
  const scanLimit = clamp(input.preferences.maxIndexedFiles, 500, 20_000);

  if (isTauriRuntime()) {
    const native = await luxCommands.aiRelatedFiles(path || null, query || null, maxResults, scanLimit);
    return toolJson("RelatedFiles", native);
  }

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

export async function impactAnalysis(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
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

export async function webFetchTool(args: UnknownRecord): Promise<ToolResult> {
  const url = stringArg(args, "url", "").trim();
  if (!url) throw new Error("WebFetch requires a URL.");
  const maxBytes = clamp(numberArg(args, "maxBytes", 250_000), 1_024, 1_000_000);
  const timeoutSecs = clamp(numberArg(args, "timeoutSecs", 20), 1, 60);
  // SSRF guard is always on; no caller/model-controlled private-host bypass (H1).
  const response = await luxCommands.webFetch(url, maxBytes, timeoutSecs);
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

/** Strip model-hallucinated prefixes (e.g. ////?//E://...) before FS calls. */
export function sanitizeWorkspaceToolPath(path: string) {
  let normalized = path.trim().replaceAll("\\", "/");
  normalized = normalized.replace(/^\/+/, "");
  normalized = normalized.replace(/^\?\/+/, "");
  normalized = normalized.replace(/^([a-zA-Z]):\/{2,}/, "$1:/");
  normalized = normalized.replace(/\/{2,}/g, "/");
  return normalized;
}