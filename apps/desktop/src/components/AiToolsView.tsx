import { useCallback, useState, type CSSProperties } from "react";
import { motion, AnimatePresence } from "motion/react";
import {
  Activity,
  AlertTriangle,
  BookOpen,
  Brain,
  Code2,
  Database,
  Eye,
  FileSearch,
  FileText,
  FolderTree,
  GitBranch,
  Layers,
  Loader2,
  Network,
  Pencil,
  Play,
  Search,
  Shield,
  Terminal,
  TestTube,
  Trash2,
  Wrench,
  Zap,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { luxCommands, type TestHealthResponse } from "../lib/tauri";

type ToolStatus = "ready";

type ToolDef = {
  id: string;
  name: string;
  description: string;
  status: ToolStatus;
  icon: LucideIcon;
};

type ToolCategory = {
  id: string;
  title: string;
  accent: string;
  tools: ToolDef[];
};

const categories: ToolCategory[] = [
  {
    id: "builtin",
    title: "Built-in",
    accent: "#3b9eff",
    tools: [
      { id: "semantic-search", name: "SemanticSearch", description: "Ranked code search", status: "ready", icon: Search },
      { id: "grep", name: "Grep", description: "Fast text/regex search", status: "ready", icon: FileSearch },
      { id: "glob", name: "Glob", description: "File pattern matching", status: "ready", icon: FolderTree },
      { id: "read", name: "Read", description: "Read files", status: "ready", icon: Eye },
      { id: "write", name: "Write", description: "Guarded file writes", status: "ready", icon: FileText },
      { id: "str-replace", name: "StrReplace", description: "Exact guarded edits", status: "ready", icon: Pencil },
      { id: "delete", name: "Delete", description: "Guarded file removal", status: "ready", icon: Trash2 },
      { id: "shell", name: "Shell", description: "Approved shell commands", status: "ready", icon: Terminal },
      { id: "read-lints", name: "ReadLints", description: "Filtered diagnostics", status: "ready", icon: AlertTriangle },
      { id: "todo-write", name: "TodoWrite", description: "Session task list", status: "ready", icon: Layers },
      { id: "web-fetch", name: "WebFetch", description: "Fetch URLs", status: "ready", icon: Network },
    ],
  },
  {
    id: "context",
    title: "Context",
    accent: "#4ec98a",
    tools: [
      { id: "fast-context", name: "FastContext", description: "Context package", status: "ready", icon: Zap },
      { id: "repo-map", name: "RepoMap", description: "Project structure", status: "ready", icon: FolderTree },
      { id: "symbol-context", name: "SymbolContext", description: "Semantic symbols", status: "ready", icon: Code2 },
      { id: "related-files", name: "RelatedFiles", description: "Tests/styles/types", status: "ready", icon: Layers },
      { id: "git-context", name: "GitContext", description: "Git state", status: "ready", icon: GitBranch },
      { id: "diagnostics", name: "DiagnosticsContext", description: "All errors", status: "ready", icon: AlertTriangle },
      { id: "test-health", name: "TestHealth", description: "Universal tests and validation", status: "ready", icon: TestTube },
      { id: "failure-analyzer", name: "FailureAnalyzer", description: "Root cause from logs", status: "ready", icon: Activity },
      { id: "docs-context", name: "DocsContext", description: "Local docs and versions", status: "ready", icon: BookOpen },
      { id: "memory-context", name: "MemoryContext", description: "Project memory", status: "ready", icon: Brain },
      { id: "context-budgeter", name: "ContextBudgeter", description: "Context under limits", status: "ready", icon: Brain },
      { id: "impact-analysis", name: "ImpactAnalysis", description: "Blast radius", status: "ready", icon: Shield },
      { id: "review-diff", name: "ReviewDiff", description: "Diff quality gate", status: "ready", icon: Eye },
    ],
  },
  {
    id: "platform",
    title: "Platform",
    accent: "#8a8a8a",
    tools: [
      { id: "workspace-index", name: "WorkspaceIndex", description: "File index", status: "ready", icon: Database },
      { id: "active-context", name: "ActiveContext", description: "Current state", status: "ready", icon: Eye },
      { id: "rules-context", name: "RulesContext", description: "Project rules", status: "ready", icon: FileText },
      { id: "secret-guard", name: "SecretGuard", description: "Secret redaction", status: "ready", icon: Shield },
      { id: "patch-engine", name: "PatchEngine", description: "Multi-file patch", status: "ready", icon: Wrench },
      { id: "checkpoint", name: "Checkpoint", description: "Snapshot, diff, rollback", status: "ready", icon: Shield },
    ],
  },
];

const statusConfig: Record<ToolStatus, { label: string; color: string }> = {
  ready: { label: "Ready", color: "#4ec98a" },
};

export function AiToolsView() {
  const [activeCategory, setActiveCategory] = useState<string | null>(null);
  const [testHealth, setTestHealth] = useState<TestHealthResponse | null>(null);
  const [testHealthRunning, setTestHealthRunning] = useState(false);
  const [testHealthError, setTestHealthError] = useState<string | null>(null);

  const filteredCategories = activeCategory
    ? categories.filter((cat) => cat.id === activeCategory)
    : categories;

  const totalTools = categories.reduce((sum, cat) => sum + cat.tools.length, 0);
  const readyTools = categories.reduce((sum, cat) => sum + cat.tools.filter((t) => t.status === "ready").length, 0);
  const testHealthStatus = testHealthRunning ? "running" : testHealth?.status ?? (testHealthError ? "error" : "idle");
  const testHealthOutput = compactTestOutput(testHealth, testHealthError);

  const runTestHealth = useCallback(() => {
    setTestHealthRunning(true);
    setTestHealthError(null);
    void luxCommands.testHealth()
      .then(setTestHealth)
      .catch((error) => setTestHealthError(error instanceof Error ? error.message : String(error)))
      .finally(() => setTestHealthRunning(false));
  }, []);

  return (
    <div className="ai-tools-view">
      <div className="ai-tools-view-header">
        <div className="ai-tools-view-stats">
          <span className="ai-tools-stat" data-status="ready">{readyTools} ready</span>
          <span className="ai-tools-stat" data-status="total">{totalTools} tools</span>
        </div>
        <div className="ai-tools-view-filters">
          <button
            type="button"
            className="ai-tools-filter-chip"
            data-active={activeCategory === null}
            onClick={() => setActiveCategory(null)}
          >
            All
          </button>
          {categories.map((cat) => (
            <button
              key={cat.id}
              type="button"
              className="ai-tools-filter-chip"
              data-active={activeCategory === cat.id}
              style={{ "--chip-accent": cat.accent } as CSSProperties}
              onClick={() => setActiveCategory(activeCategory === cat.id ? null : cat.id)}
            >
              <span className="ai-tools-chip-dot" style={{ background: cat.accent }} />
              {cat.title}
            </button>
          ))}
        </div>
        <div className="ai-test-health-card" data-status={testHealthStatus}>
          <div className="ai-test-health-icon">
            {testHealthRunning ? <Loader2 size={15} className="spin-icon" /> : <TestTube size={15} />}
          </div>
          <div className="ai-test-health-main">
            <div className="ai-test-health-title">
              <span>TestHealth</span>
              <strong>{testHealthStatus === "idle" ? "ready" : testHealthStatus}</strong>
            </div>
            <div className="ai-test-health-meta">
              {testHealth ? formatTestHealthMeta(testHealth) : "Universal test and validation health across languages."}
            </div>
            {testHealth && (
              <>
                <div className="ai-test-health-pills">
                  {summaryPills(testHealth).map((pill) => (
                    <span key={pill.key} className="ai-test-health-pill" data-status={pill.status}>{pill.label}</span>
                  ))}
                </div>
                {testHealth.runners.length > 0 && (
                  <div className="ai-test-runner-list">
                    {testHealth.runners.slice(0, 4).map((runner) => (
                      <div key={runner.id} className="ai-test-runner-row" data-status={runner.status}>
                        <span className="ai-test-runner-dot" />
                        <div className="ai-test-runner-main">
                          <div className="ai-test-runner-title">
                            <span>{runner.workspaceRelativePath}</span>
                            <small>{runner.kind}</small>
                            <strong>{runner.framework}</strong>
                            <em>{formatDuration(runner.durationMs)}</em>
                          </div>
                          <code>{runner.command}</code>
                        </div>
                      </div>
                    ))}
                    {testHealth.runners.length > 4 && (
                      <span className="ai-test-runner-more">+{testHealth.runners.length - 4} more runner{testHealth.runners.length - 4 === 1 ? "" : "s"}</span>
                    )}
                  </div>
                )}
              </>
            )}
            {(testHealthOutput || testHealthError) && (
              <pre className="ai-test-health-output">{testHealthError ?? testHealthOutput}</pre>
            )}
          </div>
          <button className="ai-test-health-run" type="button" disabled={testHealthRunning} onClick={runTestHealth}>
            {testHealthRunning ? <Loader2 size={12} className="spin-icon" /> : <Play size={12} />}
            <span>{testHealthRunning ? "Running" : "Run"}</span>
          </button>
        </div>
      </div>

      <div className="ai-tools-view-body">
        <AnimatePresence mode="popLayout">
          {filteredCategories.map((category) => (
            <motion.div
              key={category.id}
              className="ai-tools-category"
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.15 }}
            >
              <div className="ai-tools-category-header">
                <span className="ai-tools-category-dot" style={{ background: category.accent }} />
                <h3>{category.title}</h3>
                <span className="ai-tools-category-count">{category.tools.length}</span>
              </div>
              <div className="ai-tools-grid-compact">
                {category.tools.map((tool) => {
                  const Icon = tool.icon;
                  const status = statusConfig[tool.status];
                  return (
                    <motion.button
                      key={tool.id}
                      type="button"
                      className="ai-tool-compact-card"
                      data-status={tool.status}
                      style={{ "--card-accent": category.accent } as CSSProperties}
                      layout
                      initial={{ opacity: 0, scale: 0.95 }}
                      animate={{ opacity: 1, scale: 1 }}
                      transition={{ duration: 0.12 }}
                    >
                      <div className="ai-tool-compact-icon" style={{ color: category.accent }}>
                        <Icon size={14} strokeWidth={2} />
                      </div>
                      <div className="ai-tool-compact-content">
                        <span className="ai-tool-compact-name">{tool.name}</span>
                        <span className="ai-tool-compact-desc">{tool.description}</span>
                      </div>
                      <span className="ai-tool-compact-status" style={{ background: status.color }} />
                    </motion.button>
                  );
                })}
              </div>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>
    </div>
  );
}

function formatDuration(ms: number) {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(ms >= 10_000 ? 0 : 1)}s`;
}

function formatTestHealthMeta(result: TestHealthResponse) {
  if (result.runners.length === 0) return result.stderr || "No supported test runner detected.";
  const summary = result.summary;
  const parts = [
    `${summary.total} check${summary.total === 1 ? "" : "s"}`,
    `${summary.passed} passed`,
    summary.failed ? `${summary.failed} failed` : "",
    summary.timedOut ? `${summary.timedOut} timeout` : "",
    summary.errored ? `${summary.errored} error` : "",
    summary.skipped ? `${summary.skipped} skipped` : "",
    formatDuration(summary.durationMs || result.durationMs),
  ].filter(Boolean);
  return parts.join(" / ");
}

function summaryPills(result: TestHealthResponse) {
  return [
    { key: "total", label: `${result.summary.total} total`, status: "total" },
    { key: "passed", label: `${result.summary.passed} passed`, status: "passed" },
    ...kindPills(result),
    result.summary.failed ? { key: "failed", label: `${result.summary.failed} failed`, status: "failed" } : null,
    result.summary.timedOut ? { key: "timeout", label: `${result.summary.timedOut} timeout`, status: "timeout" } : null,
    result.summary.errored ? { key: "error", label: `${result.summary.errored} error`, status: "error" } : null,
    result.summary.skipped ? { key: "skipped", label: `${result.summary.skipped} skipped`, status: "skipped" } : null,
  ].filter((pill): pill is { key: string; label: string; status: string } => Boolean(pill));
}

function kindPills(result: TestHealthResponse) {
  const counts = result.runners.reduce<Record<string, number>>((acc, runner) => {
    acc[runner.kind] = (acc[runner.kind] ?? 0) + 1;
    return acc;
  }, {});
  return Object.entries(counts).slice(0, 4).map(([kind, count]) => ({
    key: `kind-${kind}`,
    label: `${count} ${kind}`,
    status: "total",
  }));
}

function compactTestOutput(result: TestHealthResponse | null, error: string | null) {
  if (error) return error;
  if (!result) return "";
  if (result.status === "passed" && result.runners.length > 0) return "";

  const failedRunners = result.runners.filter((runner) => runner.status !== "passed");
  const sections = (failedRunners.length ? failedRunners : result.runners).slice(0, 3).map((runner) => {
    const output = [runner.stderr, runner.stdout].filter(Boolean).join("\n").trim() || `exit ${runner.exitCode ?? "n/a"}`;
    return `${runner.workspaceRelativePath} / ${runner.kind} / ${runner.framework}\n${runner.command}\n${output}`;
  });
  const output = sections.join("\n\n").trim() || result.stderr || result.stdout;
  return output.length > 1100 ? `${output.slice(0, 1100)}\n...[truncated]` : output;
}
