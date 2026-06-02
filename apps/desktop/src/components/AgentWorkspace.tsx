import { Edit3, FolderOpen, MessageSquare, MessageSquarePlus, Plus, Search, Settings, Trash2 } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { LazyAiChatPanel } from "./LazyAiChatPanel";
import { aiChatSessionTitle, aiChatStatusLabel } from "../lib/aiChatPresentation";
import { useTranslation } from "../lib/i18n/useTranslation";
import { useLuxStore, type AiChatSession } from "../lib/store";
import { luxCommands } from "../lib/tauri";

type AgentWorkspaceProps = {
  onOpenProject: () => void;
};

export function AgentWorkspace({ onOpenProject }: AgentWorkspaceProps) {
  const { t } = useTranslation();
  const activeSessionId = useLuxStore((state) => state.activeAiChatSessionId);
  const chatSessions = useLuxStore((state) => state.aiChatSessions);
  const closeChatSession = useLuxStore((state) => state.closeAiChatSession);
  const createChatSession = useLuxStore((state) => state.createAiChatSession);
  const deleteChatSession = useLuxStore((state) => state.deleteAiChatSession);
  const renameChatSession = useLuxStore((state) => state.renameAiChatSession);
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
          <AgentSidebarSection title={t("agent.sidebar.currentProject")}>
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
                onCreateChat={() => createChatSession(session.workspaceRoot ?? workspace?.root ?? null)}
                onDelete={() => deleteChatSession(session.id)}
                onRename={(title) => renameChatSession(session.id, title)}
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

type AgentChatMenuAction = {
  danger?: boolean;
  disabled?: boolean;
  label: string;
  onClick: () => void;
  shortcut?: string;
};

function AgentChatRow({ active, onClose, onCreateChat, onDelete, onRename, onRestore, onSelect, session }: {
  active: boolean;
  onClose: () => void;
  onCreateChat: () => void;
  onDelete: () => void;
  onRename: (title: string) => void;
  onRestore: () => void;
  onSelect: () => void;
  session: AiChatSession;
}) {
  const { t } = useTranslation();
  const closed = Boolean(session.closedAt);
  const title = aiChatSessionTitle(session.title, t);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState(false);

  const rename = (nextTitle: string) => {
    const trimmed = nextTitle.trim();
    if (trimmed && trimmed !== session.title) onRename(trimmed);
    setRenaming(false);
  };

  const menuGroups: AgentChatMenuAction[][] = [
    [
      { label: t("agent.chat.contextMenu.open"), onClick: onSelect },
      { label: t("agent.chat.contextMenu.newChat"), onClick: onCreateChat },
    ],
    [
      { label: t("agent.chat.contextMenu.rename"), onClick: () => setRenaming(true), shortcut: "F2" },
      { label: t("agent.chat.contextMenu.copyTitle"), onClick: () => void luxCommands.clipboardWriteText(title) },
      { label: t("agent.chat.contextMenu.copyTranscript"), onClick: () => void luxCommands.clipboardWriteText(formatChatTranscript(session, title)) },
    ],
    [
      closed
        ? { label: t("agent.chat.contextMenu.continue"), onClick: onRestore }
        : { label: t("agent.chat.contextMenu.archive"), onClick: onClose, shortcut: "Del" },
      { danger: true, label: t("agent.chat.contextMenu.delete"), onClick: () => {
        if (window.confirm(t("agent.chat.deleteConfirm", { title }))) onDelete();
      } },
    ],
  ];

  return (
    <div className="agent-chat-row" data-active={active} data-closed={closed} onContextMenu={(event) => {
      event.preventDefault();
      event.stopPropagation();
      onSelect();
      setContextMenu({ x: event.clientX, y: event.clientY });
    }}>
      {renaming ? (
        <form className="agent-chat-row-edit" onSubmit={(event) => {
          event.preventDefault();
          const input = event.currentTarget.elements.namedItem("chat-title");
          rename(input instanceof HTMLInputElement ? input.value : title);
        }}>
          <Edit3 size={13} />
          <input
            autoFocus
            defaultValue={title}
            name="chat-title"
            aria-label={t("agent.chat.renameAria")}
            onBlur={(event) => rename(event.currentTarget.value)}
            onFocus={(event) => event.currentTarget.select()}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.currentTarget.value = title;
                setRenaming(false);
              }
            }}
          />
        </form>
      ) : (
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
      )}
      {closed ? (
        <button className="agent-chat-row-close" type="button" aria-label={t("aiChat.restoreChat")} title={t("aiChat.restoreChat")} onClick={onRestore}>
          <MessageSquarePlus size={12} />
        </button>
      ) : (
        <button className="agent-chat-row-close" type="button" aria-label={t("aiChat.closeChat")} title={t("aiChat.closeChat")} onClick={onClose}>
          <Trash2 size={12} />
        </button>
      )}
      {contextMenu && <AgentChatContextMenu groups={menuGroups} x={contextMenu.x} y={contextMenu.y} onClose={() => setContextMenu(null)} />}
    </div>
  );
}

function AgentChatContextMenu({ groups, onClose, x, y }: { groups: AgentChatMenuAction[][]; onClose: () => void; x: number; y: number }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [position, setPosition] = useState({ x, y });

  useEffect(() => {
    const close = () => onClose();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  useEffect(() => {
    const menu = ref.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    setPosition({
      x: Math.max(6, Math.min(x, window.innerWidth - rect.width - 8)),
      y: Math.max(6, Math.min(y, window.innerHeight - rect.height - 8)),
    });
  }, [x, y]);

  return (
    <div className="agent-chat-context-menu" ref={ref} style={{ left: position.x, top: position.y }} onPointerDown={(event) => event.stopPropagation()}>
      {groups.map((group, groupIndex) => (
        <div className="agent-chat-context-menu-group" key={groupIndex}>
          {group.map((action) => (
            <button
              className="agent-chat-context-menu-item"
              data-danger={action.danger}
              type="button"
              disabled={action.disabled}
              key={action.label}
              onClick={() => {
                if (action.disabled) return;
                action.onClick();
                onClose();
              }}
            >
              <span>{action.label}</span>
              {action.shortcut ? <kbd>{action.shortcut}</kbd> : <span />}
            </button>
          ))}
        </div>
      ))}
    </div>
  );
}

function formatChatTranscript(session: AiChatSession, title: string) {
  const lines = [`# ${title}`, ""];
  for (const message of session.messages) {
    lines.push(`## ${message.role} - ${new Date(message.timestamp).toLocaleString()}`);
    if (message.reasoning?.trim()) lines.push("", "### Reasoning", message.reasoning.trim());
    if (message.content.trim()) lines.push("", message.content.trim());
    if (message.toolCalls?.length) {
      lines.push("", "### Tools");
      for (const toolCall of message.toolCalls) {
        lines.push(`- ${toolCall.tool}: ${toolCall.status}`);
      }
    }
    lines.push("");
  }
  return lines.join("\n").trim();
}

function compareAgentChatSessions(left: AiChatSession, right: AiChatSession) {
  if (!left.closedAt && right.closedAt) return -1;
  if (left.closedAt && !right.closedAt) return 1;
  return right.updatedAt - left.updatedAt;
}
