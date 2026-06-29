import { memo } from "react";
import type { RefObject } from "react";
import { AiChatSlashMenu } from "./AiChatSlashMenu";
import { AiChatMentionMenu } from "./AiChatMentionMenu";
import type { AiMentionCandidate } from "../../lib/aiChatMentions";
import type { SlashCommandMatch } from "../../lib/aiChatSlashCommands";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiComposerCommandMenusProps = {
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
export const AiComposerCommandMenus = memo(function AiComposerCommandMenus({
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
}: AiComposerCommandMenusProps) {
  return (
    <>
      {mentionMenuOpen && (
        <AiChatMentionMenu
          activeIndex={mentionActiveIndex}
          candidates={mentionCandidates}
          menuRef={mentionMenuRef}
          onHighlight={onMentionHighlight}
          onSelect={onMentionSelect}
          t={t}
        />
      )}
      {slashMenuOpen && (
        <AiChatSlashMenu
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
