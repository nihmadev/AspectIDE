import { memo } from "react";
import type { RefObject } from "react";
import { AspectorChatSlashMenu } from "./AspectorChatSlashMenu";
import { AspectorChatMentionMenu } from "./AspectorChatMentionMenu";
import type { AiMentionCandidate } from '../../lib/aspector/chat/mentions';
import type { SlashCommandMatch } from '../../lib/aspector/chat/slash-commands';
import type { TranslateFn } from '../../lib/i18n/useTranslation';

type AspectorComposerCommandMenusProps = {
  mentionMenuOpen: boolean;
  mentionActiveIndex: number;
  mentionCandidates: AiMentionCandidate[];
  mentionMenuRef: RefObject<HTMLDivElement | null>;
  onMentionHighlight: (index: number) => void;
  onMentionSelect: (candidate: AiMentionCandidate) => void;
  slashMenuOpen: boolean;
  slashActiveIndex: number;
  slashCommands: SlashCommandMatch[];
  slashMenuRef: RefObject<HTMLDivElement | null>;
  onSlashHighlight: (index: number) => void;
  onSlashSelect: (command: SlashCommandMatch) => void;
  t: TranslateFn;
};

/** The two floating composer menus (mentions + slash commands). */
export const AspectorComposerCommandMenus = memo(function AspectorComposerCommandMenus({
  mentionMenuOpen,
  mentionActiveIndex,
  mentionCandidates,
  mentionMenuRef,
  onMentionHighlight,
  onMentionSelect,
  slashMenuOpen,
  slashActiveIndex,
  slashCommands,
  slashMenuRef,
  onSlashHighlight,
  onSlashSelect,
  t,
}: AspectorComposerCommandMenusProps) {
  return (
    <>
      {mentionMenuOpen && (
        <AspectorChatMentionMenu
          activeIndex={mentionActiveIndex}
          candidates={mentionCandidates}
          menuRef={mentionMenuRef}
          onHighlight={onMentionHighlight}
          onSelect={onMentionSelect}
          t={t}
        />
      )}
      {slashMenuOpen && (
        <AspectorChatSlashMenu
          activeIndex={slashActiveIndex}
          commands={slashCommands}
          menuRef={slashMenuRef}
          onHighlight={onSlashHighlight}
          onSelect={onSlashSelect}
          t={t}
        />
      )}
    </>
  );
});
