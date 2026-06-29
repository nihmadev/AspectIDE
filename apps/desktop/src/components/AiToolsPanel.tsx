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
  Globe,
  MessageCircleQuestion,
  Network,
  Pencil,
  Search,
  Server,
  Shield,
  Sparkles,
  Target,
  Terminal,
  TestTube,
  Trash2,
  Wrench,
  Zap,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
import type { MessageKey } from "../lib/i18n";

/**
 * "ready"       — always available in this runtime.
 * "needs-setup" — tool is registered but requires external config (SSH host,
 *                 browser install, etc.) before the agent can invoke it
 *                 successfully. The UI surfaces this so users know why a
 *                 workflow failed, rather than discovering it mid-run.
 */
type ToolStatus = "ready" | "needs-setup";

type ToolDef = {
  id: string;
  name: string;
  status: ToolStatus;
  icon: LucideIcon;
};

type ToolCategory = {
  id: string;
  accent: string;
  tools: ToolDef[];
};

const categories: ToolCategory[] = [
  {
    id: "builtin",
    accent: "#3b9eff",
    tools: [
      { id: "semantic-search", name: "SemanticSearch", status: "ready", icon: Search },
      { id: "grep", name: "Grep", status: "ready", icon: FileSearch },
      { id: "glob", name: "Glob", status: "ready", icon: FolderTree },
     { id: "read", name: "Read", status: "ready", icon: Eye },
      { id: "inspect-file", name: "InspectFile", status: "ready", icon: Search },
     { id: "write", name: "Write", status: "ready", icon: FileText },
      { id: "str-replace", name: "StrReplace", status: "ready", icon: Pencil },
      { id: "delete", name: "Delete", status: "ready", icon: Trash2 },
      { id: "shell", name: "Shell", status: "ready", icon: Terminal },
      { id: "terminal-write", name: "TerminalWrite", status: "ready", icon: Terminal },
      { id: "read-lints", name: "ReadLints", status: "ready", icon: AlertTriangle },
      { id: "todo-write", name: "TodoWrite", status: "ready", icon: LayoutGrid },
      { id: "goal", name: "Goal", status: "ready", icon: Target },
      { id: "task", name: "Task", status: "ready", icon: Network },
      { id: "agent-message", name: "AgentMessage", status: "ready", icon: Network },
      { id: "ask-user", name: "AskUser", status: "ready", icon: MessageCircleQuestion },
      { id: "present-plan", name: "PresentPlan", status: "ready", icon: Sparkles },
      { id: "mcp-manage", name: "McpManage", status: "ready", icon: Server },
      { id: "web-fetch", name: "WebFetch", status: "ready", icon: Network },
    ],
  },
  {
    id: "ssh",
    accent: "#f4a259",
    // SSH tools require an active SSH connection profile to be configured.
    // They are registered in the runtime but not usable without setup.
    tools: [
      { id: "ssh-connect", name: "SshConnect", status: "needs-setup", icon: Server },
      { id: "ssh-exec", name: "SshExec", status: "needs-setup", icon: Server },
      { id: "ssh-transfer", name: "SshTransfer", status: "needs-setup", icon: Server },
      { id: "ssh-list", name: "SshList", status: "needs-setup", icon: Server },
      { id: "ssh-disconnect", name: "SshDisconnect", status: "needs-setup", icon: Server },
    ],
  },
  {
    id: "browser",
    accent: "#c77dff",
    // Browser tools require a supported browser to be installed and the
    // Lux browser extension or headless driver to be available.
    tools: [
      { id: "browser-status", name: "BrowserStatus", status: "needs-setup", icon: Globe },
      { id: "browser-open", name: "BrowserOpen", status: "needs-setup", icon: Globe },
      { id: "browser-snapshot", name: "BrowserSnapshot", status: "needs-setup", icon: Eye },
      { id: "browser-act", name: "BrowserAct", status: "needs-setup", icon: Globe },
      { id: "browser-invoke", name: "BrowserInvoke", status: "needs-setup", icon: Terminal },
      { id: "browser-screenshot", name: "BrowserScreenshot", status: "needs-setup", icon: Eye },
      { id: "browser-close", name: "BrowserClose", status: "needs-setup", icon: Globe },
      { id: "browser-chat", name: "BrowserChat", status: "needs-setup", icon: Globe },
      { id: "browser-dashboard", name: "BrowserDashboard", status: "needs-setup", icon: Network },
      { id: "browser-install", name: "BrowserInstall", status: "needs-setup", icon: Wrench },
      { id: "browser-help", name: "BrowserHelp", status: "needs-setup", icon: BookOpen },
      { id: "browser-doctor", name: "BrowserDoctor", status: "needs-setup", icon: AlertTriangle },
    ],
  },
  {
    id: "context",
    accent: "#4ec98a",
    tools: [
      { id: "fast-context", name: "FastContext", status: "ready", icon: Zap },
      { id: "repo-map", name: "RepoMap", status: "ready", icon: FolderTree },
      { id: "symbol-context", name: "SymbolContext", status: "ready", icon: Code2 },
      { id: "related-files", name: "RelatedFiles", status: "ready", icon: Layers },
      { id: "git-context", name: "GitContext", status: "ready", icon: GitBranch },
      { id: "diagnostics-context", name: "DiagnosticsContext", status: "ready", icon: AlertTriangle },
      { id: "test-context", name: "TestHealth", status: "ready", icon: TestTube },
      { id: "failure-analyzer", name: "FailureAnalyzer", status: "ready", icon: Activity },
      { id: "docs-context", name: "DocsContext", status: "ready", icon: BookOpen },
      { id: "memory-context", name: "MemoryContext", status: "ready", icon: Brain },
      { id: "terminal-context", name: "TerminalContext", status: "ready", icon: Terminal },
      { id: "impact-analysis", name: "ImpactAnalysis", status: "ready", icon: Shield },
      { id: "review-diff", name: "ReviewDiff", status: "ready", icon: Eye },
    ],
  },
  {
    id: "platform",
    accent: "#8a8a8a",
    tools: [
      { id: "workspace-index", name: "WorkspaceIndex", status: "ready", icon: Database },
      { id: "active-context", name: "ActiveContext", status: "ready", icon: Eye },
      { id: "rules-context", name: "RulesContext", status: "ready", icon: FileText },
      { id: "checkpoint", name: "Checkpoint", status: "ready", icon: Shield },
      { id: "secret-guard", name: "SecretGuard", status: "ready", icon: Shield },
      { id: "patch-engine", name: "PatchEngine", status: "ready", icon: Wrench },
      { id: "context-budgeter", name: "ContextBudgeter", status: "ready", icon: Brain },
    ],
  },
];

const statusConfig: Record<ToolStatus, { label: string; color: string }> = {
  ready: { label: "Ready", color: "#4ec98a" },
  // Amber — tool exists in the registry but requires additional setup by the user.
  "needs-setup": { label: "Needs setup", color: "#f4a259" },
};

export function AiToolsPanel() {
  const { t } = useTranslation();
  const [activeCategory, setActiveCategory] = useState<string | null>(null);
  const [hoveredTool, setHoveredTool] = useState<string | null>(null);

  const totalTools = categories.reduce((sum, cat) => sum + cat.tools.length, 0);
  const readyTools = categories.reduce((sum, cat) => sum + cat.tools.filter((tool) => tool.status === "ready").length, 0);
  const needsSetupTools = categories.reduce((sum, cat) => sum + cat.tools.filter((tool) => tool.status === "needs-setup").length, 0);

  const filteredCategories = activeCategory
    ? categories.filter((cat) => cat.id === activeCategory)
    : categories;

  return (
    <div className="ai-tools-panel">
      <header className="ai-tools-header">
        <div className="ai-tools-title-row">
          <div className="ai-tools-title">
            <Wrench size={18} strokeWidth={1.8} />
            <h1>{t("aiTools.title")}</h1>
          </div>
          <div className="ai-tools-stats">
            <span className="ai-tools-stat" data-status="ready">{t("aiTools.readyCount", { count: readyTools })}</span>
            {needsSetupTools > 0 && (
              <span className="ai-tools-stat" data-status="needs-setup">{t("aiTools.needsSetupCount", { count: needsSetupTools })}</span>
            )}
            <span className="ai-tools-stat" data-status="total">{t("aiTools.totalCount", { count: totalTools })}</span>
          </div>
        </div>
        <nav className="ai-tools-category-nav">
          <button
            type="button"
            className="ai-tools-category-chip"
            data-active={activeCategory === null}
            onClick={() => setActiveCategory(null)}
          >
            {t("aiTools.all")}
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
              {toolCategoryTitle(cat, t)}
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
                  <h2>{toolCategoryTitle(category, t)}</h2>
                  <span className="ai-tools-section-count">{category.tools.length}</span>
                </div>
                <p className="ai-tools-section-subtitle">{toolCategorySubtitle(category, t)}</p>
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
                        <p className="ai-tool-card-desc">{toolDescription(tool, t)}</p>
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
            <span className="ai-tools-legend-item"><span style={{ background: statusConfig.ready.color }} />{t("aiTools.status.ready")}</span>
            <span className="ai-tools-legend-item"><span style={{ background: statusConfig["needs-setup"].color }} />{t("aiTools.status.needsSetup")}</span>
          </span>
          <span className="ai-tools-footer-note">{t("aiTools.footerNote")}</span>
        </div>
      </footer>
    </div>
  );
}

function toolCategoryTitle(category: ToolCategory, t: TranslateFn) {
  return t(`aiTools.category.${category.id}.title` as MessageKey);
}

function toolCategorySubtitle(category: ToolCategory, t: TranslateFn) {
  return t(`aiTools.category.${category.id}.subtitle` as MessageKey);
}

function toolDescription(tool: ToolDef, t: TranslateFn) {
  return t(`aiTools.tool.${tool.id}.description` as MessageKey);
}
