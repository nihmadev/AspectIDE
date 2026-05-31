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
  LayoutGrid,
  Network,
  Pencil,
  Search,
  Shield,
  Terminal,
  TestTube,
  Trash2,
  Wrench,
  Zap,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";

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
  subtitle: string;
  accent: string;
  tools: ToolDef[];
};

const categories: ToolCategory[] = [
  {
    id: "builtin",
    title: "Built-in Tools",
    subtitle: "Core IDE operations",
    accent: "#3b9eff",
    tools: [
      { id: "semantic-search", name: "SemanticSearch", description: "Ranked code search across symbols, text, and files", status: "ready", icon: Search },
      { id: "grep", name: "Grep", description: "Fast text/regex search via ripgrep", status: "ready", icon: FileSearch },
      { id: "glob", name: "Glob", description: "File pattern matching", status: "ready", icon: FolderTree },
      { id: "read", name: "Read", description: "Read files and images", status: "ready", icon: Eye },
      { id: "write", name: "Write", description: "Create or overwrite files", status: "ready", icon: FileText },
      { id: "str-replace", name: "StrReplace", description: "Precise text replacement", status: "ready", icon: Pencil },
      { id: "delete", name: "Delete", description: "Remove files safely", status: "ready", icon: Trash2 },
      { id: "shell", name: "Shell", description: "Terminal commands", status: "ready", icon: Terminal },
      { id: "read-lints", name: "ReadLints", description: "Filtered linter and language diagnostics", status: "ready", icon: AlertTriangle },
      { id: "todo-write", name: "TodoWrite", description: "Visible session task list", status: "ready", icon: LayoutGrid },
      { id: "web-fetch", name: "WebFetch", description: "Fetch and parse HTTP/HTTPS URLs", status: "ready", icon: Network },
    ],
  },
  {
    id: "context",
    title: "Context & Analysis",
    subtitle: "Acceleration layer for quality and speed",
    accent: "#4ec98a",
    tools: [
      { id: "fast-context", name: "FastContext", description: "Orchestrated context package for any task", status: "ready", icon: Zap },
      { id: "repo-map", name: "RepoMap", description: "Compressed project structure map", status: "ready", icon: FolderTree },
      { id: "symbol-context", name: "SymbolContext", description: "Definitions, usages, signatures, call sites", status: "ready", icon: Code2 },
      { id: "related-files", name: "RelatedFiles", description: "Tests, styles, types, routes, schemas", status: "ready", icon: Layers },
      { id: "git-context", name: "GitContext", description: "Structured git state model", status: "ready", icon: GitBranch },
      { id: "diagnostics-context", name: "DiagnosticsContext", description: "All current errors in one list", status: "ready", icon: AlertTriangle },
      { id: "test-context", name: "TestHealth", description: "Universal tests, checks, and logs", status: "ready", icon: TestTube },
      { id: "failure-analyzer", name: "FailureAnalyzer", description: "Root cause from logs and CI", status: "ready", icon: Activity },
      { id: "docs-context", name: "DocsContext", description: "Local docs and versioned manifests", status: "ready", icon: BookOpen },
      { id: "memory-context", name: "MemoryContext", description: "Project decisions and preferences", status: "ready", icon: Brain },
      { id: "impact-analysis", name: "ImpactAnalysis", description: "Blast radius before edits", status: "ready", icon: Shield },
      { id: "review-diff", name: "ReviewDiff", description: "Quality gate on current diff", status: "ready", icon: Eye },
    ],
  },
  {
    id: "platform",
    title: "Platform",
    subtitle: "IDE runtime state and safety capabilities",
    accent: "#8a8a8a",
    tools: [
      { id: "workspace-index", name: "WorkspaceIndex", description: "Incremental file and symbol index", status: "ready", icon: Database },
      { id: "active-context", name: "ActiveContext", description: "Tabs, cursor, terminal, recent edits", status: "ready", icon: Eye },
      { id: "rules-context", name: "RulesContext", description: "Auto-pickup of project rules", status: "ready", icon: FileText },
      { id: "checkpoint", name: "Checkpoint", description: "In-session snapshots, diffs, and guarded rollback", status: "ready", icon: Shield },
      { id: "secret-guard", name: "SecretGuard", description: "Secret detection and redaction in outputs", status: "ready", icon: Shield },
      { id: "patch-engine", name: "PatchEngine", description: "Preflighted multi-file patch with rollback", status: "ready", icon: Wrench },
      { id: "context-budgeter", name: "ContextBudgeter", description: "Context prioritization under limits", status: "ready", icon: Brain },
    ],
  },
];

const statusConfig: Record<ToolStatus, { label: string; color: string }> = {
  ready: { label: "Ready", color: "#4ec98a" },
};

export function AiToolsPanel() {
  const [activeCategory, setActiveCategory] = useState<string | null>(null);
  const [hoveredTool, setHoveredTool] = useState<string | null>(null);

  const totalTools = categories.reduce((sum, cat) => sum + cat.tools.length, 0);
  const readyTools = categories.reduce((sum, cat) => sum + cat.tools.filter((t) => t.status === "ready").length, 0);

  const filteredCategories = activeCategory
    ? categories.filter((cat) => cat.id === activeCategory)
    : categories;

  return (
    <div className="ai-tools-panel">
      <header className="ai-tools-header">
        <div className="ai-tools-title-row">
          <div className="ai-tools-title">
            <Wrench size={18} strokeWidth={1.8} />
            <h1>AI Tools</h1>
          </div>
          <div className="ai-tools-stats">
            <span className="ai-tools-stat" data-status="ready">{readyTools} ready</span>
            <span className="ai-tools-stat" data-status="total">{totalTools} total</span>
          </div>
        </div>
        <nav className="ai-tools-category-nav">
          <button
            type="button"
            className="ai-tools-category-chip"
            data-active={activeCategory === null}
            onClick={() => setActiveCategory(null)}
          >
            All
          </button>
          {categories.map((cat) => (
            <button
              key={cat.id}
              type="button"
              className="ai-tools-category-chip"
              data-active={activeCategory === cat.id}
              style={{ "--chip-accent": cat.accent } as React.CSSProperties}
              onClick={() => setActiveCategory(activeCategory === cat.id ? null : cat.id)}
            >
              <span className="ai-tools-chip-dot" style={{ background: cat.accent }} />
              {cat.title}
            </button>
          ))}
        </nav>
      </header>

      <div className="ai-tools-body">
        <AnimatePresence mode="popLayout">
          {filteredCategories.map((category) => (
            <motion.section
              key={category.id}
              className="ai-tools-section"
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.2 }}
            >
              <div className="ai-tools-section-header">
                <div className="ai-tools-section-title">
                  <span className="ai-tools-section-dot" style={{ background: category.accent }} />
                  <h2>{category.title}</h2>
                  <span className="ai-tools-section-count">{category.tools.length}</span>
                </div>
                <p className="ai-tools-section-subtitle">{category.subtitle}</p>
              </div>
              <div className="ai-tools-grid">
                {category.tools.map((tool) => {
                  const Icon = tool.icon;
                  const status = statusConfig[tool.status];
                  const isHovered = hoveredTool === tool.id;
                  return (
                    <motion.div
                      key={tool.id}
                      className="ai-tool-card"
                      data-status={tool.status}
                      style={{ "--card-accent": category.accent } as React.CSSProperties}
                      onMouseEnter={() => setHoveredTool(tool.id)}
                      onMouseLeave={() => setHoveredTool(null)}
                      layout
                      initial={{ opacity: 0, scale: 0.96 }}
                      animate={{ opacity: 1, scale: 1 }}
                      transition={{ duration: 0.15 }}
                    >
                      <div className="ai-tool-card-icon" style={{ color: category.accent }}>
                        <Icon size={16} strokeWidth={1.8} />
                      </div>
                      <div className="ai-tool-card-content">
                        <div className="ai-tool-card-name">
                          <span>{tool.name}</span>
                          <span className="ai-tool-status-dot" style={{ background: status.color }} title={status.label} />
                        </div>
                        <p className="ai-tool-card-desc">{tool.description}</p>
                      </div>
                      {isHovered && (
                        <motion.div
                          className="ai-tool-card-glow"
                          style={{ background: category.accent }}
                          layoutId="tool-glow"
                          initial={{ opacity: 0 }}
                          animate={{ opacity: 0.06 }}
                          exit={{ opacity: 0 }}
                          transition={{ duration: 0.15 }}
                        />
                      )}
                    </motion.div>
                  );
                })}
              </div>
            </motion.section>
          ))}
        </AnimatePresence>
      </div>

      <footer className="ai-tools-footer">
        <div className="ai-tools-footer-bar">
          <span className="ai-tools-footer-legend">
            <span className="ai-tools-legend-item"><span style={{ background: statusConfig.ready.color }} />{statusConfig.ready.label}</span>
          </span>
          <span className="ai-tools-footer-note">Ready tools are callable by the AI runtime</span>
        </div>
      </footer>
    </div>
  );
}
