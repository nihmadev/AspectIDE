import { Archive, ArchiveRestore, Download, History, Pin, PinOff, Pencil, Trash2 } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { exportChatSessionMarkdown } from '../../lib/aspector/chat/export';
import { aiChatSessionTitle, aiChatStatusLabel } from '../../lib/aspector/chat/presentation';
import { sameWorkspaceRoot } from '../../lib/explorer/file-tree';
import { useTranslation } from '../../lib/i18n/useTranslation';
import { useLuxStore, type AiChatSession } from '../../lib/store/index';

type AspectorChatHistoryPopoverProps = {
  workspaceRoot: string | null;
};

type PopoverPosition = {
  left: number;
  maxHeight: number;
  top: number;
  width: number;
};

export function AspectorChatHistoryPopover({ workspaceRoot }: AspectorChatHistoryPopoverProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [position, setPosition] = useState<PopoverPosition | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const activeSessionId = useLuxStore((state) => state.activeAiChatSessionId);
  const chatSessions = useLuxStore((state) => state.aiChatSessions);
  const setActiveChatSession = useLuxStore((state) => state.setActiveAiChatSession);
  const restoreChatSession = useLuxStore((state) => state.restoreAiChatSession);
  const renameChatSession = useLuxStore((state) => state.renameAiChatSession);
  const pinChatSession = useLuxStore((state) => state.pinAiChatSession);
  const closeChatSession = useLuxStore((state) => state.closeAiChatSession);
  const deleteChatSession = useLuxStore((state) => state.deleteAiChatSession);

  const scopedSessions = useMemo(
    () => chatSessions.filter((session) => sameWorkspaceRoot(session.workspaceRoot, workspaceRoot)),
    [chatSessions, workspaceRoot],
  );
  const filteredSessions = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return scopedSessions;
    return scopedSessions.filter((session) => aiChatSessionTitle(session.title, t).toLowerCase().includes(needle));
  }, [query, scopedSessions, t]);
  const openSessions = useMemo(
    () => [...filteredSessions].filter((session) => !session.closedAt).sort(compareChatSessions),
    [filteredSessions],
  );
  const archivedSessions = useMemo(
    () => [...filteredSessions].filter((session) => session.closedAt).sort(compareChatSessions),
    [filteredSessions],
  );

  const updatePosition = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const viewportGap = 8;
    const menuGap = 5;
    const width = Math.min(360, Math.max(260, rect.width + 200));
    const naturalHeight = Math.min(420, 96 + (openSessions.length + archivedSessions.length) * 36);
    const spaceBelow = window.innerHeight - rect.bottom - viewportGap;
    const spaceAbove = rect.top - viewportGap;
    const openBelow = spaceBelow >= Math.min(naturalHeight, 120) || spaceBelow >= spaceAbove;
    const maxHeight = Math.max(160, Math.min(naturalHeight, openBelow ? spaceBelow : spaceAbove));
    const top = openBelow ? rect.bottom + menuGap : rect.top - maxHeight - menuGap;
    const left = Math.min(Math.max(viewportGap, rect.right - width), window.innerWidth - width - viewportGap);
    setPosition({ left, maxHeight, top: Math.max(viewportGap, top), width });
  }, [archivedSessions.length, openSessions.length]);

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
  }, [open, updatePosition]);

  useEffect(() => {
    if (!open) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    window.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
      window.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [open, updatePosition]);

  const selectSession = (sessionId: string) => {
    setActiveChatSession(sessionId);
    setOpen(false);
  };

  const deleteSession = (session: AiChatSession) => {
    if (window.confirm(t("agent.chat.deleteConfirm", { title: aiChatSessionTitle(session.title, t) }))) {
      deleteChatSession(session.id);
    }
  };

  const exportSession = (session: AiChatSession) => {
    const markdown = exportChatSessionMarkdown(session, workspaceRoot);
    const blob = new Blob([markdown], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `${session.title.trim() || "aspect-chat"}.md`;
    anchor.click();
    URL.revokeObjectURL(url);
  };

  return (
    <>
      <button
        ref={triggerRef}
        className="icon-button compact"
        type="button"
        data-active={open || undefined}
        aria-expanded={open}
        aria-haspopup="menu"
        aria-label={t("aiChat.history.aria")}
        title={t("aiChat.history.aria")}
        onClick={() => setOpen((value) => !value)}
      >
        <History size={15} />
      </button>
      {open && position && createPortal(
        <div
          ref={menuRef}
          className="ai-chat-history-popover"
          role="menu"
          style={{ left: position.left, maxHeight: position.maxHeight, top: position.top, width: position.width }}
        >
          <div className="ai-chat-history-popover-toolbar">
            <input
              type="search"
              value={query}
              placeholder={t("aiChat.history.search")}
              onChange={(event) => setQuery(event.target.value)}
            />
          </div>
          <div className="ai-chat-history-popover-scroll">
            <AspectorChatHistorySection
              emptyLabel={t("aiChat.history.emptyOpen")}
              sessions={openSessions}
              activeSessionId={activeSessionId}
              renamingId={renamingId}
              renameDraft={renameDraft}
              onRenameDraft={setRenameDraft}
              onStartRename={(session) => {
                setRenamingId(session.id);
                setRenameDraft(aiChatSessionTitle(session.title, t));
              }}
              onCommitRename={(sessionId) => {
                if (renameDraft.trim()) renameChatSession(sessionId, renameDraft);
                setRenamingId(null);
              }}
              onPin={(sessionId, pinned) => pinChatSession(sessionId, pinned)}
              onExport={exportSession}
              onArchive={(sessionId) => closeChatSession(sessionId)}
              onDelete={deleteSession}
              onSelect={selectSession}
              title={t("agent.sidebar.chats")}
            />
            <AspectorChatHistorySection
              emptyLabel={t("agent.chat.historyEmpty")}
              sessions={archivedSessions}
              activeSessionId={activeSessionId}
              archived
              onRestore={(sessionId) => {
                restoreChatSession(sessionId);
                setOpen(false);
              }}
              onPin={(sessionId, pinned) => pinChatSession(sessionId, pinned)}
              onExport={exportSession}
              onDelete={deleteSession}
              onSelect={selectSession}
              title={t("agent.sidebar.history")}
            />
          </div>
        </div>,
        document.body,
      )}
    </>
  );
}

function AspectorChatHistorySection({
  activeSessionId,
  archived = false,
  emptyLabel,
  onRestore,
  onSelect,
  onPin,
  onExport,
  onArchive,
  onDelete,
  renamingId,
  renameDraft,
  onRenameDraft,
  onStartRename,
  onCommitRename,
  sessions,
  title,
}: {
  activeSessionId: string;
  archived?: boolean;
  emptyLabel: string;
  onRestore?: (sessionId: string) => void;
  onSelect: (sessionId: string) => void;
  onPin?: (sessionId: string, pinned: boolean) => void;
  onExport?: (session: AiChatSession) => void;
  onArchive?: (sessionId: string) => void;
  onDelete?: (session: AiChatSession) => void;
  renamingId?: string | null;
  renameDraft?: string;
  onRenameDraft?: (value: string) => void;
  onStartRename?: (session: AiChatSession) => void;
  onCommitRename?: (sessionId: string) => void;
  sessions: AiChatSession[];
  title: string;
}) {
  const { t } = useTranslation();

  return (
    <section className="ai-chat-history-section">
      <h3>{title}</h3>
      {sessions.length === 0 ? (
        <p className="ai-chat-history-empty">{emptyLabel}</p>
      ) : sessions.map((session) => {
        const label = aiChatSessionTitle(session.title, t);
        const renaming = renamingId === session.id;
        return (
          <div key={session.id} className="ai-chat-history-item-row" data-active={session.id === activeSessionId || undefined}>
            {renaming ? (
              <input
                className="ai-chat-history-rename"
                value={renameDraft ?? ""}
                autoFocus
                onChange={(event) => onRenameDraft?.(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") onCommitRename?.(session.id);
                  if (event.key === "Escape") onCommitRename?.(session.id);
                }}
                onBlur={() => onCommitRename?.(session.id)}
              />
            ) : (
              <button
                className="ai-chat-history-item"
                type="button"
                role="menuitem"
                data-archived={archived || undefined}
                title={label}
                onClick={() => {
                  if (archived && onRestore) onRestore(session.id);
                  else onSelect(session.id);
                }}
              >
                <span className="ai-chat-history-item-label">
                  {session.pinned && <Pin size={11} aria-hidden="true" />}
                  {label}
                </span>
                {session.status !== "idle" && <small>{aiChatStatusLabel(session.status, true, t)}</small>}
                {archived && <Archive size={12} />}
              </button>
            )}
            <div className="ai-chat-history-item-actions">
              {onPin && (
                <button type="button" title={session.pinned ? t("aiChat.history.unpin") : t("aiChat.history.pin")} onClick={() => onPin(session.id, !session.pinned)}>
                  {session.pinned ? <PinOff size={12} /> : <Pin size={12} />}
                </button>
              )}
              {onStartRename && !archived && (
                <button type="button" title={t("aiChat.history.rename")} onClick={() => onStartRename(session)}>
                  <Pencil size={12} />
                </button>
              )}
              {onExport && (
                <button type="button" title={t("aiChat.history.export")} onClick={() => onExport(session)}>
                  <Download size={12} />
                </button>
              )}
              {archived && onRestore && (
                <button type="button" title={t("aiChat.restoreChat")} onClick={() => onRestore(session.id)}>
                  <ArchiveRestore size={12} />
                </button>
              )}
              {!archived && onArchive && (
                <button type="button" title={t("agent.chat.contextMenu.archive")} onClick={() => onArchive(session.id)}>
                  <Archive size={12} />
                </button>
              )}
              {onDelete && (
                <button type="button" className="ai-chat-history-item-danger" title={t("agent.chat.contextMenu.delete")} onClick={() => onDelete(session)}>
                  <Trash2 size={12} />
                </button>
              )}
            </div>
          </div>
        );
      })}
    </section>
  );
}

function compareChatSessions(left: AiChatSession, right: AiChatSession) {
  if (Boolean(left.pinned) !== Boolean(right.pinned)) return left.pinned ? -1 : 1;
  return right.updatedAt - left.updatedAt;
}