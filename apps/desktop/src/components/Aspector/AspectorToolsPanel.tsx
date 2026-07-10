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
import { useMemo, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useTranslation, type TranslateFn } from '../../lib/i18n/useTranslation';
import type { MessageKey } from '../../lib/i18n';
import { isReadOnlyAgentMode } from '../../lib/aspector/utils/preferences';
import { useLuxStore } from '../../lib/store/index';

/**
 * "ready"       — registered and invokable in the current runtime configuration.
 * "needs-setup" — registered but requires external config (browser install, SSH
 *                 host, etc.) before the agent can invoke it successfully.
 * "unavailable" — registered but NOT invokable in the current configuration: the
 *                 active agent mode or a disabled feature has removed it from the
 *                 callable toolset (e.g. write/shell/SSH in read-only Plan/Ask
 *                 modes, or Browser tools when browser automation is off). The
 *                 panel surfaces this so users don't trust a tool that the model
 *                 cannot actually call in the current mode.
 */
type ToolStatus = "ready" | "needs-setup" | "unavailable";

/**
 * Runtime capability a tool depends on. Static metadata stays in the catalog while
 * the *effective* status is derived per render from live preferences, so the panel
 * never claims a tool is Ready when the current mode/settings make it uncallable.
 *  - "execution": needs a full-execution mode (Agent/Automatic); gone in Plan/Ask.
 *  - "browser":   needs browser automation enabled in settings.
 */
type ToolRequirement = "execution" | "browser";

type ToolDef = {
  id: string;
  name: string;
  /** Status when all runtime requirements are satisfied. */
  status: ToolStatus;
  icon: LucideIcon;
  /** Live gates that can downgrade the tool to "unavailable" (see resolveToolStatus). */
  requires?: ToolRequirement[];
};

type ToolCategory = {
  id: string;
  accent: string;
  tools: ToolDef[];
};

/** Live runtime signals the frontend can derive that gate tool availability. */
type ToolRuntime = {
  readOnlyMode: boolean;
  browserEnabled: boolean;
};

/**
 * Collapse a tool's static base status with the live runtime gates into the status
 * actually shown. A failing gate always wins (a tool that cannot be called in this
 * mode is "unavailable" regardless of its setup state).
 */
function resolveToolStatus(tool: ToolDef, runtime: ToolRuntime): ToolStatus {
  if (tool.requires?.includes("execution") && runtime.readOnlyMode) return "unavailable";
  if (tool.requires?.includes("browser") && !runtime.browserEnabled) return "unavailable";
  return tool.status;
}

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
     { id: "write", name: "Write", status: "ready", icon: FileText, requires: ["execution"] },
      { id: "str-replace", name: "StrReplace", status: "ready", icon: Pencil, requires: ["execution"] },
      { id: "delete", name: "Delete", status: "ready", icon: Trash2, requires: ["execution"] },
      { id: "shell", name: "Shell", status: "ready", icon: Terminal, requires: ["execution"] },
      { id: "terminal-write", name: "TerminalWrite", status: "ready", icon: Terminal, requires: ["execution"] },
      { id: "read-lints", name: "ReadLints", status: "ready", icon: AlertTriangle },
      { id: "todo-write", name: "TodoWrite", status: "ready", icon: LayoutGrid, requires: ["execution"] },
      { id: "goal", name: "Goal", status: "ready", icon: Target, requires: ["execution"] },
      { id: "task", name: "Task", status: "ready", icon: Network, requires: ["execution"] },
      { id: "agent-message", name: "AgentMessage", status: "ready", icon: Network, requires: ["execution"] },
      { id: "ask-user", name: "AskUser", status: "ready", icon: MessageCircleQuestion },
      { id: "present-plan", name: "PresentPlan", status: "ready", icon: Sparkles },
      { id: "mcp-manage", name: "McpManage", status: "ready", icon: Server },
      { id: "web-fetch", name: "WebFetch", status: "ready", icon: Network },
    ],
  },
  {
    id: "ssh",
    accent: "#f4a259",
    // SSH tools require an active SSH connection profile to be configured. They are
    // registered in the runtime but not usable without setup, and not callable at all
    // in read-only Plan/Ask modes (they execute remote commands/transfers).
    tools: [
      { id: "ssh-connect", name: "SshConnect", status: "needs-setup", icon: Server, requires: ["execution"] },
      { id: "ssh-exec", name: "SshExec", status: "needs-setup", icon: Server, requires: ["execution"] },
      { id: "ssh-transfer", name: "SshTransfer", status: "needs-setup", icon: Server, requires: ["execution"] },
      { id: "ssh-list", name: "SshList", status: "needs-setup", icon: Server, requires: ["execution"] },
      { id: "ssh-disconnect", name: "SshDisconnect", status: "needs-setup", icon: Server, requires: ["execution"] },
    ],
  },
  {
    id: "browser",
    accent: "#c77dff",
    // Browser tools require a supported browser to be installed and browser
    // automation enabled in settings. When automation is disabled they are not part
    // of the callable toolset (gated by "browser"); when enabled they still report
    // "needs-setup" until the driver/browser is provisioned.
    tools: [
      { id: "browser-status", name: "BrowserStatus", status: "needs-setup", icon: Globe, requires: ["browser"] },
      { id: "browser-open", name: "BrowserOpen", status: "needs-setup", icon: Globe, requires: ["browser"] },
      { id: "browser-snapshot", name: "BrowserSnapshot", status: "needs-setup", icon: Eye, requires: ["browser"] },
      { id: "browser-act", name: "BrowserAct", status: "needs-setup", icon: Globe, requires: ["browser"] },
      { id: "browser-invoke", name: "BrowserInvoke", status: "needs-setup", icon: Terminal, requires: ["browser"] },
      { id: "browser-screenshot", name: "BrowserScreenshot", status: "needs-setup", icon: Eye, requires: ["browser"] },
      { id: "browser-close", name: "BrowserClose", status: "needs-setup", icon: Globe, requires: ["browser"] },
      { id: "browser-chat", name: "BrowserChat", status: "needs-setup", icon: Globe, requires: ["browser"] },
      { id: "browser-dashboard", name: "BrowserDashboard", status: "needs-setup", icon: Network, requires: ["browser"] },
      { id: "browser-install", name: "BrowserInstall", status: "needs-setup", icon: Wrench, requires: ["browser"] },
      { id: "browser-help", name: "BrowserHelp", status: "needs-setup", icon: BookOpen, requires: ["browser"] },
      { id: "browser-doctor", name: "BrowserDoctor", status: "needs-setup", icon: AlertTriangle, requires: ["browser"] },
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
      { id: "checkpoint", name: "Checkpoint", status: "ready", icon: Shield, requires: ["execution"] },
      { id: "secret-guard", name: "SecretGuard", status: "ready", icon: Shield },
      { id: "patch-engine", name: "PatchEngine", status: "ready", icon: Wrench, requires: ["execution"] },
      { id: "context-budgeter", name: "ContextBudgeter", status: "ready", icon: Brain },
    ],
  },
];

const statusColor: Record<ToolStatus, string> = {
  ready: "#4ec98a",
  // Amber — tool exists in the registry but requires additional setup by the user.
  "needs-setup": "#f4a259",
  // Muted grey — tool is registered but not callable in the current mode/settings.
  unavailable: "#8a8a8a",
};

function statusLabel(status: ToolStatus, t: TranslateFn): string {
  if (status === "ready") return t("aiTools.status.ready");
  if (status === "needs-setup") return t("aiTools.status.needsSetup");
  // Reuse the existing generic "Unavailable" string (present in every locale) so
  // the panel stays fully localized without a dedicated aiTools key.
  return t("voice.status.unavailable");
}

export function AspectorToolsPanel() {
  const { t } = useTranslation();
  const [activeCategory, setActiveCategory] = useState<string | null>(null);
  const [hoveredTool, setHoveredTool] = useState<string | null>(null);
  // Live runtime gates: the active agent mode and the browser-automation toggle
  // decide which tools the agent can actually call right now. Subscribing to these
  // keeps the panel honest instead of always reporting Ready.
  const agentMode = useLuxStore((state) => state.aiPreferences.agentMode);
  const browserEnabled = useLuxStore((state) => state.aiPreferences.agentBrowserEnabled);

  // Effective per-tool status keyed by tool id, recomputed when the gates change.
  const effectiveStatus = useMemo(() => {
    const runtime: ToolRuntime = { readOnlyMode: isReadOnlyAgentMode(agentMode), browserEnabled };
    const map = new Map<string, ToolStatus>();
    for (const category of categories) {
      for (const tool of category.tools) map.set(tool.id, resolveToolStatus(tool, runtime));
    }
    return map;
  }, [agentMode, browserEnabled]);

  const totalTools = categories.reduce((sum, cat) => sum + cat.tools.length, 0);
  const readyTools = categories.reduce(
    (sum, cat) => sum + cat.tools.filter((tool) => effectiveStatus.get(tool.id) === "ready").length,
    0,
  );
  const needsSetupTools = categories.reduce(
    (sum, cat) => sum + cat.tools.filter((tool) => effectiveStatus.get(tool.id) === "needs-setup").length,
    0,
  );
  const unavailableTools = categories.reduce(
    (sum, cat) => sum + cat.tools.filter((tool) => effectiveStatus.get(tool.id) === "unavailable").length,
    0,
  );

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
            {unavailableTools > 0 && (
              <span className="ai-tools-stat" data-status="unavailable">{statusLabel("unavailable", t)}: {unavailableTools}</span>
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
                  const status = effectiveStatus.get(tool.id) ?? tool.status;
                  const isHovered = hoveredTool === tool.id;
                  return (
                    <motion.div
                      key={tool.id}
                      className="ai-tool-card"
                      data-status={status}
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
                          <span className="ai-tool-status-dot" style={{ background: statusColor[status] }} title={statusLabel(status, t)} />
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
            <span className="ai-tools-legend-item"><span style={{ background: statusColor.ready }} />{t("aiTools.status.ready")}</span>
            <span className="ai-tools-legend-item"><span style={{ background: statusColor["needs-setup"] }} />{t("aiTools.status.needsSetup")}</span>
            {unavailableTools > 0 && (
              <span className="ai-tools-legend-item"><span style={{ background: statusColor.unavailable }} />{statusLabel("unavailable", t)}</span>
            )}
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
