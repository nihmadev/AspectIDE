import type { ChangeEvent, ClipboardEvent, DragEvent, KeyboardEvent, ReactNode, RefObject } from "react";
import type { AiMentionCandidate } from '../../lib/aspector/chat/mentions';
import { AspectorComposerAttachments, type AspectorComposerAttachmentView } from "./AspectorComposerAttachments";
import { AspectorComposerCommandMenus } from "./AspectorComposerCommandMenus";
import { AspectorComposerInputArea } from "./AspectorComposerInputArea";
import { AspectorComposerModelControls } from "./AspectorComposerModelControls";
import { AspectorComposerSendControls } from "./AspectorComposerSendControls";
import { formatAspectUsageLabel, useAspectSelectedModelUsage } from '../../lib/aspect/model-sync';
import type { AspectorComposerSelectOption, AspectorComposerVoiceState } from "./AspectorComposerTypes";
import type { SlashCommandMatch } from '../../lib/aspector/chat/slash-commands';
import type { AiChatContextUsageMeta, AiChatContextUsageSummary } from '../../lib/aspector/chat/context-usage';
import type { AiChatContextDropSummary } from '../../lib/aspector/chat/context-report';
import type { AiPreferences } from '../../lib/aspector/utils/preferences';
import type { TranslateFn } from '../../lib/i18n/useTranslation';

// Re-export shared composer types so existing import sites stay valid after the
// composer was decomposed into focused sections.
export type { AspectorComposerVoiceState } from "./AspectorComposerTypes";

type AspectorChatComposerProps = {
  activeSessionSending: boolean;
  compacting?: boolean;
  /** Live tok/s of the running turn, or null to hide the readout. */
  tokenSpeed?: number | null;
  agentOptions: AspectorComposerSelectOption[];
  attachments: AspectorComposerAttachmentView[];
  attachFiles: (files: FileList | File[] | null) => void;
  canSend: boolean;
  contextOpen: boolean;
  contextTitle: string;
  contextUsage: AiChatContextUsageSummary & AiChatContextUsageMeta;
  contextDrops?: AiChatContextDropSummary | null;
  disabled: boolean;
  draggingFiles: boolean;
  effortOptions: AspectorComposerSelectOption[];
  fileInputRef: RefObject<HTMLInputElement | null>;
  handleCancelSend: () => void;
  handleComposerDragOver: (event: DragEvent<HTMLDivElement>) => void;
  handleComposerDrop: (event: DragEvent<HTMLDivElement>) => void;
  handleComposerKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  handleComposerPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void;
  handleMessageChange: (event: ChangeEvent<HTMLTextAreaElement>) => void;
  handleSend: () => void;
  mentionActiveIndex: number;
  mentionCandidates: AiMentionCandidate[];
  mentionMenuOpen: boolean;
  mentionMenuRef: RefObject<HTMLDivElement | null>;
  onMentionHighlight: (index: number) => void;
  onMentionSelect: (candidate: AiMentionCandidate) => void;
  onContextCompact?: () => void;
  onOpenSettings?: () => void;
  isAgentHome: boolean;
  message: string;
  onSlashHighlight: (index: number) => void;
  onSlashSelect: (command: SlashCommandMatch) => void;
  slashActiveIndex: number;
  slashCommands: SlashCommandMatch[];
  slashMenuOpen: boolean;
  slashMenuRef: RefObject<HTMLDivElement | null>;
  modelOptions: AspectorComposerSelectOption[];
  modelSupportsEffort: boolean;
  modelSearchPlaceholder?: string;
  modelSearchEmptyHint?: string;
  onHideModel?: (value: string) => void;
  hideModelLabel?: string;
  modelFooter?: ReactNode;
  preferences: AiPreferences;
  removeAttachment: (id: string) => void;
  selectedModelId: string;
  selectedProviderReady: boolean;
  setContextOpen: (open: boolean | ((open: boolean) => boolean)) => void;
  setDraggingFiles: (dragging: boolean) => void;
  t: TranslateFn;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  updateAiPreference: (patch: Partial<AiPreferences>) => void;
  updateModel: (selectedModelId: string) => void;
  voiceInput: AspectorComposerVoiceState;
};

/**
 * Composer shell. Orchestration state lives in the parent; this component lays
 * out the drag/drop frame and delegates each concern to a focused, memoized
 * section (command menus, input area, model controls, send/voice controls) so an
 * unrelated state change re-renders only the section whose narrow props changed.
 */
export function AspectorChatComposer({
  activeSessionSending,
  compacting = false,
  tokenSpeed = null,
  agentOptions,
  attachments,
  attachFiles,
  canSend,
  contextOpen,
  contextTitle,
  contextUsage,
  contextDrops,
  disabled,
  draggingFiles,
  effortOptions,
  fileInputRef,
  handleCancelSend,
  handleComposerDragOver,
  handleComposerDrop,
  handleComposerKeyDown,
  handleComposerPaste,
  handleMessageChange,
  handleSend,
  isAgentHome,
  mentionActiveIndex,
  mentionCandidates,
  mentionMenuOpen,
  mentionMenuRef,
  message,
  onMentionHighlight,
  onMentionSelect,
  onContextCompact,
  onOpenSettings,
  onSlashHighlight,
  onSlashSelect,
  slashActiveIndex,
  slashCommands,
  slashMenuOpen,
  slashMenuRef,
  modelOptions,
  modelSupportsEffort,
  modelSearchPlaceholder,
  modelSearchEmptyHint,
  onHideModel,
  hideModelLabel,
  modelFooter,
  preferences,
  removeAttachment,
  selectedModelId,
  selectedProviderReady,
  setContextOpen,
  setDraggingFiles,
  t,
  textareaRef,
  updateAiPreference,
  updateModel,
  voiceInput,
}: AspectorChatComposerProps) {
  // Compact "used / cap" daily-budget readout for the bundled aspect models; null
  // for any other provider or when the selected model has neither a cap nor usage.
  const aspectUsageLabel = formatAspectUsageLabel(useAspectSelectedModelUsage());
  return (
    <div
      className="ai-chat-composer"
      data-agent-home={isAgentHome}
      data-dragging-files={draggingFiles}
      onDragLeave={(event) => {
        if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
        setDraggingFiles(false);
      }}
      onDragOver={handleComposerDragOver}
      onDrop={handleComposerDrop}
    >
      <AspectorComposerCommandMenus
        mentionMenuOpen={mentionMenuOpen}
        mentionActiveIndex={mentionActiveIndex}
        mentionCandidates={mentionCandidates}
        mentionMenuRef={mentionMenuRef}
        onMentionHighlight={onMentionHighlight}
        onMentionSelect={onMentionSelect}
        slashMenuOpen={slashMenuOpen}
        slashActiveIndex={slashActiveIndex}
        slashCommands={slashCommands}
        slashMenuRef={slashMenuRef}
        onSlashHighlight={onSlashHighlight}
        onSlashSelect={onSlashSelect}
        t={t}
      />
      {aspectUsageLabel && (
        <div className="ai-composer-usage" title="Your usage / your cap per window В· ОЈ = your all-time total for this model">
          {aspectUsageLabel}
        </div>
      )}
      <AspectorComposerInputArea
        message={message}
        disabled={disabled}
        compacting={compacting}
        textareaRef={textareaRef}
        handleMessageChange={handleMessageChange}
        handleComposerKeyDown={handleComposerKeyDown}
        handleComposerPaste={handleComposerPaste}
        voiceInput={voiceInput}
        t={t}
      />
      <AspectorComposerAttachments
        attachments={attachments}
        draggingFiles={draggingFiles}
        removeAttachment={removeAttachment}
        t={t}
      />
      <div className="ai-composer-actions">
        <AspectorComposerModelControls
          disabled={disabled}
          fileInputRef={fileInputRef}
          attachFiles={attachFiles}
          attachmentCount={attachments.length}
          agentOptions={agentOptions}
          modelOptions={modelOptions}
          selectedModelId={selectedModelId}
          updateModel={updateModel}
          modelSupportsEffort={modelSupportsEffort}
          effortOptions={effortOptions}
          modelSearchPlaceholder={modelSearchPlaceholder}
          modelSearchEmptyHint={modelSearchEmptyHint}
          onHideModel={onHideModel}
          hideModelLabel={hideModelLabel}
          modelFooter={modelFooter}
          preferences={preferences}
          updateAiPreference={updateAiPreference}
          t={t}
        />
        <AspectorComposerSendControls
          tokenSpeed={tokenSpeed}
          contextOpen={contextOpen}
          contextTitle={contextTitle}
          contextUsage={contextUsage}
          contextDrops={contextDrops}
          onContextCompact={onContextCompact}
          onOpenSettings={onOpenSettings}
          setContextOpen={setContextOpen}
          voiceInput={voiceInput}
          disabled={disabled}
          message={message}
          activeSessionSending={activeSessionSending}
          canSend={canSend}
          selectedProviderReady={selectedProviderReady}
          handleSend={handleSend}
          handleCancelSend={handleCancelSend}
          t={t}
        />
      </div>
    </div>
  );
}
