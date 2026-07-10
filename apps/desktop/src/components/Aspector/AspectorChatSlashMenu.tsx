import { Bot, Command, Database, ScrollText, Settings, Sparkles, Target, Trash2, Undo2, Wand2 } from "lucide-react";
import type { CSSProperties, RefObject } from "react";
import { slashCommandDescription, slashCommandLabel, type SlashCommandMatch } from '../../lib/aspector/chat/slash-commands';
import type { TranslateFn } from '../../lib/i18n/useTranslation';

type AspectorChatSlashMenuProps = {
  activeIndex: number;
  commands: SlashCommandMatch[];
  menuRef: RefObject<HTMLDivElement | null>;
  onHighlight: (index: number) => void;
  onSelect: (command: SlashCommandMatch) => void;
  t: TranslateFn;
};

const commandIcons = {
  compact: Sparkles,
  clear: Trash2,
  undo: Undo2,
  help: Command,
  goal: Target,
  model: Wand2,
  agent: Bot,
  settings: Settings,
  index: Database,
} as const;

export function AspectorChatSlashMenu({ activeIndex, commands, menuRef, onHighlight, onSelect, t }: AspectorChatSlashMenuProps) {
  if (commands.length === 0) return null;

  return (
    <div className="ai-slash-menu" ref={menuRef} role="listbox" aria-label={t("aiChat.slash.menuAria")}>
      <div className="ai-slash-menu-head">
        <span>{t("aiChat.slash.menuTitle")}</span>
        <small>{t("aiChat.slash.menuHint")}</small>
      </div>
      <ul>
        {commands.map((command, index) => {
          const Icon = command.kind === "project" ? ScrollText : commandIcons[command.id];
          const active = index === activeIndex;
          return (
            <li key={command.id}>
              <button
                type="button"
                role="option"
                aria-selected={active}
                data-active={active}
                style={{ "--slash-index": index } as CSSProperties}
                onMouseEnter={() => onHighlight(index)}
                onMouseDown={(event) => {
                  event.preventDefault();
                  onSelect(command);
                }}
              >
                <span className="ai-slash-menu-icon" aria-hidden="true">
                  <Icon size={14} />
                </span>
                <span className="ai-slash-menu-copy">
                  <strong>{slashCommandLabel(command, t)}</strong>
                  <small>{slashCommandDescription(command, t)}</small>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}