import { Check, FolderOpen, MessageSquare, Plus, RotateCcw, Search, Settings } from "lucide-react";
import type { ReactNode } from "react";
import { LazyAiChatPanel } from "./LazyAiChatPanel";
import { aiChatSessionTitle, aiChatStatusLabel } from "../lib/aiChatPresentation";
import { useTranslation } from "../lib/i18n/useTranslation";
import { useLuxStore, type AiChatSession } from "../lib/store";

type AgentWorkspaceProps = {
  onOpenProject: () => void;
};

export function AgentWorkspace({ onOpenProject }: AgentWorkspaceProps) {
  const { t } = useTranslation();
  const activeSessionId = useLuxStore((state) => state.activeAiChatSessionId);
  const chatSessions = useLuxStore((state) => state.aiChatSessions);
  const closeChatSession = useLuxStore((state) => state.closeAiChatSession);
  const createChatSession = useLuxStore((state) => state.createAiChatSession);
  const restoreChatSession = useLuxStore((state) => state.restoreAiChatSession);
  const setActiveChatSession = useLuxStore((state) => state.setActiveAiChatSession);
  const setSettingsOpen = useLuxStore((state) => state.setSettingsOpen);
  const workspace = useLuxStore((state) => state.workspace);
  const sortedChatSessions = [...chatSessions].sort(compareAgentChatSessions);

  return (
    <main className="agent-workspace" aria-label={t("agent.workspace.label")}>
      <aside className="agent-rail">
        <nav className="agent-nav" aria-label={t("agent.navigation.label")}>
          <AgentNavButton icon={<Plus size={15} />} label={t("agent.newChat")} onClick={() => createChatSession(workspace?.root ?? null)} />
          <AgentNavButton icon={<Search size={15} />} label={t("agent.search")} />
        </nav>

        <div className="agent-scroll-list">
          <AgentSidebarSection title={t("agent.sidebar.pinned")}>
            {workspace ? (
              <ProjectHeaderButton icon={<FolderOpen size={14} />} label={workspace.name} onClick={onOpenProject} />
            ) : (
              <ProjectHeaderButton icon={<FolderOpen size={14} />} label={t("agent.openProject")} onClick={onOpenProject} />
            )}
          </AgentSidebarSection>

          <AgentSidebarSection title={t("agent.sidebar.chats")}>
            {sortedChatSessions.map((session) => (
              <AgentChatRow
                key={session.id}
                active={session.id === activeSessionId}
                session={session}
                onClose={() => closeChatSession(session.id)}
                onRestore={() => restoreChatSession(session.id)}
                onSelect={() => setActiveChatSession(session.id)}
              />
            ))}
          </AgentSidebarSection>

        </div>

        <button className="agent-settings-link" type="button" onClick={() => setSettingsOpen(true)}>
          <Settings size={15} />
          <span>{t("agent.settings")}</span>
        </button>
      </aside>

      <section className="agent-chat-home">
        <LazyAiChatPanel embedded presentation="agent" showCloseButton={false} />
      </section>
    </main>
  );
}

function AgentNavButton({ icon, label, onClick }: { icon: ReactNode; label: string; onClick?: () => void }) {
  return (
    <button className="agent-nav-button" type="button" onClick={onClick}>
      {icon}
      <span>{label}</span>
    </button>
  );
}

function AgentSidebarSection({ children, title }: { children: ReactNode; title: string }) {
  return (
    <section className="agent-sidebar-section">
      <h2>{title}</h2>
      {children}
    </section>
  );
}

function ProjectHeaderButton({ icon, label, onClick }: { icon: ReactNode; label: string; onClick: () => void }) {
  return (
    <button className="agent-project-row" type="button" onClick={onClick}>
      {icon}
      <span>{label}</span>
    </button>
  );
}

function AgentChatRow({ active, onClose, onRestore, onSelect, session }: {
  active: boolean;
  onClose: () => void;
  onRestore: () => void;
  onSelect: () => void;
  session: AiChatSession;
}) {
  const { t } = useTranslation();
  const closed = Boolean(session.closedAt);
  const title = aiChatSessionTitle(session.title, t);
  return (
    <div className="agent-chat-row" data-active={active} data-closed={closed}>
      <button
        type="button"
        title={closed ? `${title} (${t("agent.chat.closed")})` : title}
        onClick={onSelect}
        onMouseDown={(event) => {
          if (event.button !== 1) return;
          event.preventDefault();
          if (!closed) onClose();
        }}
      >
        <MessageSquare size={14} />
        <span>{title}</span>
        {session.status !== "idle" && <small>{aiChatStatusLabel(session.status, true, t)}</small>}
        {closed && <small>{t("agent.chat.closed")}</small>}
      </button>
      {closed ? (
        <button className="agent-chat-row-close" type="button" aria-label={t("aiChat.restoreChat")} title={t("aiChat.restoreChat")} onClick={onRestore}>
          <RotateCcw size={12} />
        </button>
      ) : (
        <button className="agent-chat-row-close" type="button" aria-label={t("aiChat.closeChat")} title={t("aiChat.closeChat")} onClick={onClose}>
          <Check size={12} />
        </button>
      )}
    </div>
  );
}

function compareAgentChatSessions(left: AiChatSession, right: AiChatSession) {
  if (!left.closedAt && right.closedAt) return -1;
  if (left.closedAt && !right.closedAt) return 1;
  return right.updatedAt - left.updatedAt;
}
