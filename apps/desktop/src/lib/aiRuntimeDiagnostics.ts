import type { AiChatSendInput } from "./aiChatTypes";
import { publicSecretFinding, scanSecrets, type SecretFindingInternal } from "./aiRuntimeSecretGuard";
import { booleanArg, clamp, normalizePathSlashes, numberArg, readErrorMessage, stringArg, toolJson, topCounts, truncateText, type ToolResult, type UnknownRecord } from "./aiRuntimeShared";
import { luxCommands } from "./tauri";
import type { WorkspaceDiagnostic } from "./types";

type FailureFinding = {
  source: string;
  kind: string;
  message: string;
  path?: string;
  line?: number;
  column?: number;
  evidence: string;
};

/** Upper bound on diagnostics folded into FailureAnalyzer ranking (after severity sort). */
const maxFailureAnalyzerDiagnostics = 80;

export async function readLints(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const pathFilter = normalizePathSlashes(stringArg(args, "path", input.activeDocument?.path ?? "")).toLowerCase();
  const severity = stringArg(args, "severity", "").trim().toLowerCase();
  const source = stringArg(args, "source", "").trim().toLowerCase();
  const maxResults = clamp(numberArg(args, "maxResults", 80), 1, 500);
  const diagnostics = await luxCommands.diagnosticsSnapshot();
  const filtered = diagnostics
    .filter((diagnostic) => !pathFilter || normalizePathSlashes(diagnostic.path).toLowerCase().includes(pathFilter))
    .filter((diagnostic) => !severity || diagnostic.severity.toLowerCase() === severity)
    .filter((diagnostic) => !source || diagnostic.source.toLowerCase().includes(source))
    .sort(compareDiagnostics)
    .slice(0, maxResults);
  return toolJson("ReadLints", {
    workspaceRoot: input.workspace?.root ?? null,
    filters: {
      path: pathFilter || null,
      severity: severity || null,
      source: source || null,
    },
    totalDiagnostics: diagnostics.length,
    count: filtered.length,
    bySeverity: topCounts(diagnostics.map((diagnostic) => diagnostic.severity), 8),
    bySource: topCounts(diagnostics.map((diagnostic) => diagnostic.source || "unknown"), 16),
    diagnostics: filtered.map((diagnostic) => ({
      path: diagnostic.path,
      line: diagnostic.line,
      column: diagnostic.column,
      severity: diagnostic.severity,
      source: diagnostic.source,
      message: diagnostic.message,
    })),
  });
}

export async function diagnosticsContext(maxResults: number): Promise<ToolResult> {
  const diagnostics = await luxCommands.diagnosticsSnapshot();
  // Sort by severity (errors → warnings → info → hint) BEFORE capping so a snapshot
  // that happens to start with hints can't push real compiler/type errors out of the
  // returned window — the agent must see blockers first. Report what was dropped.
  const sorted = [...diagnostics].sort(compareDiagnostics);
  const cap = clamp(maxResults, 1, 500);
  const selected = sorted.slice(0, cap);
  const omitted = sorted.slice(cap);
  return toolJson("DiagnosticsContext", {
    count: diagnostics.length,
    returned: selected.length,
    omitted: omitted.length,
    omittedBySeverity: omitted.length > 0 ? topCounts(omitted.map((diagnostic) => diagnostic.severity), 8) : undefined,
    diagnostics: selected.map((diagnostic) => ({
      path: diagnostic.path,
      line: diagnostic.line,
      column: diagnostic.column,
      severity: diagnostic.severity,
      source: diagnostic.source,
      message: diagnostic.message,
    })),
  });
}

export async function gitContext(): Promise<ToolResult> {
  const status = await luxCommands.gitStatus();
  return toolJson("GitContext", {
    branch: status.branch,
    ahead: status.ahead,
    behind: status.behind,
    changedFiles: status.files.map((file) => ({
      path: file.path,
      indexStatus: file.index_status,
      worktreeStatus: file.worktree_status,
    })),
  });
}

export async function testHealth(): Promise<ToolResult> {
  const health = await luxCommands.testHealth();
  return toolJson("TestHealth", {
    workspaceRoot: health.workspaceRoot,
    status: health.status,
    summary: health.summary,
    runners: health.runners.map((runner) => ({
      path: runner.workspaceRelativePath,
      status: runner.status,
      kind: runner.kind,
      language: runner.language,
      framework: runner.framework,
      command: runner.command,
      exitCode: runner.exitCode,
      durationMs: runner.durationMs,
      timedOut: runner.timedOut,
      stdout: runner.stdout,
      stderr: runner.stderr,
    })),
    language: health.language,
    framework: health.framework,
    command: health.command,
    exitCode: health.exitCode,
    durationMs: health.durationMs,
    timedOut: health.timedOut,
    stdout: health.stdout,
    stderr: health.stderr,
  });
}

export async function failureAnalyzer(args: UnknownRecord): Promise<ToolResult> {
  const rawLog = stringArg(args, "log");
  const includeTestHealth = booleanArg(args, "includeTestHealth", true);
  const includeDiagnostics = booleanArg(args, "includeDiagnostics", true);
  const maxFindings = clamp(numberArg(args, "maxFindings", 12), 1, 40);
  const [healthResult, diagnosticsResult] = await Promise.allSettled([
    includeTestHealth ? luxCommands.testHealth() : Promise.resolve(null),
    includeDiagnostics ? luxCommands.diagnosticsSnapshot() : Promise.resolve([]),
  ]);
  const health = healthResult.status === "fulfilled" ? healthResult.value : null;
  const diagnostics = diagnosticsResult.status === "fulfilled" ? diagnosticsResult.value : [];
  const sections = collectFailureSections(rawLog, health);
  // Severity-sort before the cap: feeding an unsorted snapshot into ranking could
  // drop the actual errors (kept behind leading hints) below the slice window.
  const rankedDiagnostics = [...diagnostics].sort(compareDiagnostics).slice(0, maxFailureAnalyzerDiagnostics);
  const findings = rankFailureFindings([
    ...rankedDiagnostics.map((diagnostic): FailureFinding => ({
      source: "diagnostics",
      kind: diagnostic.severity || "diagnostic",
      message: diagnostic.message,
      path: diagnostic.path,
      line: diagnostic.line,
      column: diagnostic.column,
      evidence: `${diagnostic.path}:${diagnostic.line}:${diagnostic.column} ${diagnostic.message}`,
    })),
    ...sections.flatMap((section) => extractFailureFindings(section.source, section.text)),
  ]).slice(0, maxFindings);
  const affectedFiles = Array.from(new Set(findings.flatMap((finding) => finding.path ? [finding.path] : extractPathsFromText(finding.evidence)))).slice(0, 24);

  return toolJson("FailureAnalyzer", {
    status: health?.status ?? (findings.length > 0 ? "failed" : "unknown"),
    summary: buildFailureSummary(findings, health),
    testHealth: health ? {
      status: health.status,
      summary: health.summary,
      runners: health.runners.map((runner) => ({
        path: runner.workspaceRelativePath,
        status: runner.status,
        kind: runner.kind,
        language: runner.language,
        framework: runner.framework,
        command: runner.command,
        exitCode: runner.exitCode,
        timedOut: runner.timedOut,
      })),
    } : null,
    diagnosticsUnavailable: diagnosticsResult.status === "rejected" ? readErrorMessage(diagnosticsResult.reason) : null,
    testHealthUnavailable: healthResult.status === "rejected" ? readErrorMessage(healthResult.reason) : null,
    affectedFiles,
    findings,
    nextActions: buildFailureNextActions(findings, health),
  });
}

export async function reviewDiff(args: UnknownRecord): Promise<ToolResult> {
  const includePatch = booleanArg(args, "includePatch", true);
  const maxFindings = clamp(numberArg(args, "maxFindings", 12), 1, 40);
  const [statusResult, diffResult, diagnosticsResult] = await Promise.allSettled([
    luxCommands.gitStatus(),
    luxCommands.gitDiff(),
    luxCommands.diagnosticsSnapshot(),
  ]);
  const status = statusResult.status === "fulfilled" ? statusResult.value : null;
  const diff = diffResult.status === "fulfilled" ? diffResult.value : null;
  const diagnostics = diagnosticsResult.status === "fulfilled" ? diagnosticsResult.value : [];
  const changedFiles = mergeDiffAndStatusFiles(diff?.files ?? [], status?.files ?? []);
  const secretScan = scanSecrets(diff?.patch ?? "", "git.diff");
  const findings = buildDiffReviewFindings(changedFiles, diagnostics, secretScan.findings).slice(0, maxFindings);

  return toolJson("ReviewDiff", {
    branch: status?.branch ?? null,
    ahead: status?.ahead ?? 0,
    behind: status?.behind ?? 0,
    changedFiles: changedFiles.map((file) => ({
      path: normalizePathSlashes(file.path),
      oldPath: file.old_path ? normalizePathSlashes(file.old_path) : null,
      status: file.status,
      additions: file.additions,
      deletions: file.deletions,
      binary: file.binary,
    })),
    totals: {
      files: changedFiles.length,
      additions: diff?.additions ?? 0,
      deletions: diff?.deletions ?? 0,
      diagnostics: diagnostics.length,
    },
    findings,
    secretGuard: {
      redacted: secretScan.findings.length > 0,
      findingCount: secretScan.findings.length,
      findings: secretScan.findings.slice(0, 20).map(publicSecretFinding),
    },
    recommendedChecks: buildReviewDiffChecks(changedFiles),
    patch: includePatch ? secretScan.redactedText : undefined,
    truncated: diff?.truncated ?? false,
    unavailable: {
      status: statusResult.status === "rejected" ? readErrorMessage(statusResult.reason) : null,
      diff: diffResult.status === "rejected" ? readErrorMessage(diffResult.reason) : null,
      diagnostics: diagnosticsResult.status === "rejected" ? readErrorMessage(diagnosticsResult.reason) : null,
    },
  });
}

export function mergeDiffAndStatusFiles(
  diffFiles: Array<{ path: string; old_path: string | null; status: string; additions: number; deletions: number; binary: boolean }>,
  statusFiles: Array<{ path: string; index_status: string; worktree_status: string }>,
) {
  const byPath = new Map<string, { path: string; old_path: string | null; status: string; additions: number; deletions: number; binary: boolean }>();
  for (const file of diffFiles) byPath.set(normalizePathSlashes(file.path).toLowerCase(), file);
  for (const file of statusFiles) {
    const key = normalizePathSlashes(file.path).toLowerCase();
    if (byPath.has(key)) continue;
    const status = file.index_status !== " " && file.index_status !== "?" ? file.index_status : file.worktree_status;
    byPath.set(key, {
      path: file.path,
      old_path: null,
      status: status === "?" ? "A" : status || "M",
      additions: 0,
      deletions: 0,
      binary: false,
    });
  }
  return Array.from(byPath.values()).sort((left, right) => normalizePathSlashes(left.path).localeCompare(normalizePathSlashes(right.path)));
}

function compareDiagnostics(left: WorkspaceDiagnostic, right: WorkspaceDiagnostic) {
  return diagnosticSeverityRank(right.severity) - diagnosticSeverityRank(left.severity) ||
    normalizePathSlashes(left.path).localeCompare(normalizePathSlashes(right.path)) ||
    left.line - right.line ||
    left.column - right.column;
}

function diagnosticSeverityRank(severity: string) {
  switch (severity) {
    case "error":
      return 4;
    case "warning":
      return 3;
    case "information":
      return 2;
    case "hint":
      return 1;
    default:
      return 0;
  }
}

function buildDiffReviewFindings(changedFiles: Array<{ path: string; status: string; additions: number; deletions: number; binary: boolean }>, diagnostics: Awaited<ReturnType<typeof luxCommands.diagnosticsSnapshot>>, secrets: SecretFindingInternal[] = []) {
  const findings: Array<{ severity: "low" | "medium" | "high"; path?: string; message: string; evidence: string }> = [];
  const lowerPaths = changedFiles.map((file) => normalizePathSlashes(file.path).toLowerCase());
  if (diagnostics.length > 0) {
    findings.push({ severity: "high", message: "Workspace diagnostics are present while reviewing the diff.", evidence: `${diagnostics.length} diagnostic(s) reported.` });
  }
  if (secrets.length > 0) {
    findings.push({ severity: "high", message: "Potential secrets are present in the current diff.", evidence: `${secrets.length} secret-like value(s) detected and redacted by SecretGuard.` });
  }
  for (const file of changedFiles) {
    const path = normalizePathSlashes(file.path);
    const lower = path.toLowerCase();
    const churn = file.additions + file.deletions;
    if (file.binary) findings.push({ severity: "medium", path, message: "Binary file changed; automated text review cannot inspect content.", evidence: `${file.status} ${path}` });
    if (churn > 500) findings.push({ severity: "medium", path, message: "Large file churn; review for generated output or unrelated changes.", evidence: `+${file.additions} -${file.deletions}` });
    if (/package\.json|cargo\.toml|lock|pnpm-lock|yarn\.lock|package-lock/.test(lower)) findings.push({ severity: "medium", path, message: "Dependency or script metadata changed; verify install/build/test behavior.", evidence: `${file.status} ${path}` });
    if (/migration|schema|model|entity|\.sql$|\.graphql$|\.proto$/.test(lower)) findings.push({ severity: "high", path, message: "Schema or persistence contract changed; verify compatibility and migrations.", evidence: `${file.status} ${path}` });
    if (file.status === "D") findings.push({ severity: "high", path, message: "File deletion needs explicit justification and related references checked.", evidence: `${file.status} ${path}` });
  }
  const hasSource = lowerPaths.some((path) => /\.(ts|tsx|js|jsx|rs|py|go|java|kt|cs)$/.test(path));
  const hasTests = lowerPaths.some((path) => /(^|\/)(__tests__|tests?|specs?)(\/|$)|[._-](test|spec)\./.test(path));
  if (hasSource && !hasTests) findings.push({ severity: "medium", message: "Source files changed without nearby test changes in the current diff.", evidence: "No test/spec paths found among changed files." });
  if (findings.length === 0) findings.push({ severity: "low", message: "No obvious diff risks found from metadata, diagnostics, or file mix.", evidence: `${changedFiles.length} changed file(s).` });
  return findings;
}

function buildReviewDiffChecks(changedFiles: Array<{ path: string }>) {
  const checks = new Set<string>();
  const lowerPaths = changedFiles.map((file) => normalizePathSlashes(file.path).toLowerCase());
  if (lowerPaths.some((path) => /\.(ts|tsx|js|jsx)$/.test(path) || path.endsWith("package.json"))) checks.add("pnpm --filter @lux/desktop typecheck");
  if (lowerPaths.some((path) => /\.(ts|tsx|js|jsx|css|scss)$/.test(path) || path.endsWith("package.json"))) checks.add("pnpm --filter @lux/desktop build");
  if (lowerPaths.some((path) => path.endsWith(".rs") || path.endsWith("cargo.toml"))) checks.add("cargo test --workspace");
  if (lowerPaths.some((path) => /components|\.css$|\.tsx$/.test(path))) checks.add("Browser smoke test for the changed UI flow");
  if (checks.size === 0) checks.add("Run the smallest project-specific test/build command covering the changed files");
  return Array.from(checks);
}

function collectFailureSections(rawLog: string, health: Awaited<ReturnType<typeof luxCommands.testHealth>> | null) {
  const sections: Array<{ source: string; text: string }> = [];
  if (rawLog.trim()) sections.push({ source: "provided-log", text: rawLog });
  if (!health) return sections;
  if (health.stderr || health.stdout) sections.push({ source: "test-health", text: [health.stderr, health.stdout].filter(Boolean).join("\n") });
  for (const runner of health.runners) {
    const text = [runner.stderr, runner.stdout].filter(Boolean).join("\n");
    if (text.trim()) sections.push({ source: `runner:${runner.workspaceRelativePath || runner.framework}`, text });
  }
  return sections;
}

function extractFailureFindings(source: string, text: string): FailureFinding[] {
  const lines = text.split(/\r?\n/);
  const findings: FailureFinding[] = [];
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index].trimEnd();
    if (!isFailureSignalLine(line)) continue;
    const context = compactFailureContext(lines, index);
    const location = firstFailureLocation(lines.slice(index, index + 5).join("\n"));
    findings.push({
      source,
      kind: classifyFailureLine(line),
      message: compactFailureMessage(line),
      path: location?.path,
      line: location?.line,
      column: location?.column,
      evidence: context,
    });
  }
  return findings;
}

function isFailureSignalLine(line: string) {
  return /\b(error|failed|failure|panic|exception|traceback|assertion|expected|received|mismatch|cannot find|not found|undefined|timed out|timeout|exit code|compilation failed|typeerror|referenceerror|syntaxerror)\b/i.test(line) ||
    /^\s*(E\s+|FAIL\b|FAILED\b|error\[E\d+\]|\[ERROR\]|Caused by:)/.test(line) ||
    /\s+at\s+[^\s]+\(.+?:\d+:\d+\)/.test(line);
}

function classifyFailureLine(line: string) {
  if (/traceback|exception|typeerror|referenceerror|syntaxerror/i.test(line)) return "runtime-exception";
  if (/assert|expected|received|mismatch|should/i.test(line) || (/failed/i.test(line) && !/compilation failed/i.test(line))) return "test-assertion";
  if (/cannot find|not found|undefined|missing module|module not found/i.test(line)) return "missing-reference";
  if (/timed out|timeout/i.test(line)) return "timeout";
  if (/panic|error\[E\d+\]|compilation failed|ts\d{4}/i.test(line)) return "compiler-error";
  return "failure";
}

function compactFailureMessage(line: string) {
  return truncateText(line.trim().replace(/^\s*(?:(?:FAILED|FAIL|ERROR)\s*:?\s*|E\s+)/i, ""), 260);
}

function compactFailureContext(lines: string[], index: number) {
  const start = Math.max(0, index - 2);
  const end = Math.min(lines.length, index + 5);
  return truncateText(lines.slice(start, end).join("\n").trim(), 1600);
}

function firstFailureLocation(text: string) {
  const patterns = [
    /([A-Za-z]:[\\/][^\s:()<>"']+):(\d+):(\d+)/,
    /([A-Za-z]:[\\/][^\s:()<>"']+):(\d+)/,
    /([./]?[\w@~-][\w@~./\\-]+\.[A-Za-z][\w]+):(\d+):(\d+)/,
    /([./]?[\w@~-][\w@~./\\-]+\.[A-Za-z][\w]+):(\d+)/,
  ];
  for (const pattern of patterns) {
    const match = text.match(pattern);
    if (!match) continue;
    return {
      path: normalizePathSlashes(match[1]),
      line: Number(match[2]),
      column: match[3] ? Number(match[3]) : undefined,
    };
  }
  return null;
}

function extractPathsFromText(text: string) {
  const paths = new Set<string>();
  const pattern = /(?:[A-Za-z]:[\\/]|\.\.?[\\/]|[\w@~-]+[\\/])[\w@~./\\ -]+\.[A-Za-z][\w]+/g;
  for (const match of text.matchAll(pattern)) paths.add(normalizePathSlashes(match[0]));
  return Array.from(paths);
}

function rankFailureFindings(findings: FailureFinding[]) {
  const seen = new Set<string>();
  return findings
    .filter((finding) => {
      const key = `${finding.kind}:${finding.path ?? ""}:${finding.line ?? ""}:${finding.message}`.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .sort((left, right) => failureFindingScore(right) - failureFindingScore(left));
}

function failureFindingScore(finding: FailureFinding) {
  let score = 0;
  if (finding.path) score += 30;
  if (finding.line) score += 12;
  if (finding.source === "diagnostics") score += 18;
  if (finding.kind === "compiler-error" || finding.kind === "runtime-exception") score += 16;
  if (finding.kind === "test-assertion") score += 12;
  if (finding.kind === "timeout") score += 8;
  if (/node_modules|target|dist|build/.test(finding.evidence.toLowerCase())) score -= 18;
  return score;
}

function buildFailureSummary(findings: FailureFinding[], health: Awaited<ReturnType<typeof luxCommands.testHealth>> | null) {
  if (findings.length === 0) return health?.status === "passed" ? "No failures detected in diagnostics or test output." : "No specific failure signal was extracted.";
  const top = findings[0];
  const location = top.path ? ` at ${top.path}${top.line ? `:${top.line}` : ""}` : "";
  return `${findings.length} failure signal${findings.length === 1 ? "" : "s"}; top candidate: ${top.kind}${location}.`;
}

function buildFailureNextActions(findings: FailureFinding[], health: Awaited<ReturnType<typeof luxCommands.testHealth>> | null) {
  const actions = new Set<string>();
  const top = findings[0];
  if (top?.path) actions.add(`Open ${top.path}${top.line ? `:${top.line}` : ""} and inspect the reported code path.`);
  if (findings.some((finding) => finding.kind === "compiler-error" || finding.source === "diagnostics")) actions.add("Fix compiler/language diagnostics before rerunning tests.");
  if (findings.some((finding) => finding.kind === "missing-reference")) actions.add("Check imports, generated files, package names, and workspace-relative paths for missing references.");
  if (findings.some((finding) => finding.kind === "test-assertion")) actions.add("Compare the failing assertion's expected and received values, then inspect the nearest related source and test files.");
  if (findings.some((finding) => finding.kind === "timeout") || health?.timedOut) actions.add("Look for async waits, deadlocks, hanging dev servers, or tests that need tighter timeouts/mocks.");
  if (health && health.status !== "passed" && health.command) actions.add(`Rerun the focused failing command after changes: ${health.command}`);
  return Array.from(actions).slice(0, 6);
}
