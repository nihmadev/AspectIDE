import { AtSign, BookOpen, Database, FileCode2, Folder, Search } from "lucide-react";
import type { CSSProperties, RefObject } from "react";
import type { AiMentionCandidate } from "../../lib/aiChatMentions";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiChatMentionMenuProps = {
  activeIndex: number;
  candidates: AiMentionCandidate[];
  menuRef: RefObject<HTMLDivElement | null>;
  onHighlight: (index: number) => void;
  onSelect: (candidate: AiMentionCandidate) => void;
  t: TranslateFn;
};

const mentionIcons = {
  file: FileCode2,
  folder: Folder,
  symbol: Search,
  codebase: Database,
  docs: BookOpen,
} as const;

export function AiChatMentionMenu({ activeIndex, candidates, menuRef, onHighlight, onSelect, t }: AiChatMentionMenuProps) {
  if (candidates.length === 0) return null;

  return (
    <div className="ai-mention-menu" ref={menuRef} role="listbox" aria-label={t("aiChat.mention.menuAria")}>
      <div className="ai-mention-menu-head">
        <AtSign size={14} aria-hidden="true" />
        <span>{t("aiChat.mention.menuTitle")}</span>
        <small>{t("aiChat.mention.menuHint")}</small>
      </div>
      <ul>
        {candidates.map((candidate, index) => {
          const Icon = mentionIcons[candidate.kind];
          const active = index === activeIndex;
          return (
            <li key={candidate.id}>
              <button
                type="button"
                role="option"
                aria-selected={active}
                data-active={active}
                style={{ "--mention-index": index } as CSSProperties}
                onMouseEnter={() => onHighlight(index)}
                onMouseDown={(event) => {
                  event.preventDefault();
                  onSelect(candidate);
                }}
              >
                <span className="ai-mention-menu-icon" aria-hidden="true">
                  <Icon size={14} />
                </span>
                <span className="ai-mention-menu-copy">
                  <strong>{candidate.label}</strong>
                  <small>{candidate.detail}</small>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}