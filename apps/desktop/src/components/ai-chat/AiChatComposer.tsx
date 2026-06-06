import { Bot, Brain, CornerDownLeft, Mic, Paperclip, SendHorizontal, Server, Sparkles, Square, X } from "lucide-react";
import type { ChangeEvent, ClipboardEvent, DragEvent, KeyboardEvent, RefObject } from "react";
import { CompactDropdown } from "../CompactDropdown";
import { AiChatSlashMenu } from "./AiChatSlashMenu";
import { AiChatMentionMenu } from "./AiChatMentionMenu";
import type { AiMentionCandidate } from "../../lib/aiChatMentions";
import { AiComposerAttachments, type AiComposerAttachmentView } from "./AiComposerAttachments";
import { AiComposerInlineMentions } from "./AiComposerInlineMentions";
import type { SlashCommandMatch } from "../../lib/aiChatSlashCommands";
import type { AiChatContextUsageMeta, AiChatContextUsageSummary } from "../../lib/aiChatContextUsage";
import type { AiChatContextDropSummary } from "../../lib/aiChatContextReport";
import { AiContextIndicator } from "./AiContextIndicator";
import type { AiPreferences } from "../../lib/aiPreferences";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

export type AiComposerVoiceState = {
  canUseVoice: boolean;
  listening: boolean;
  toggleVoiceInput: () => void;
  voiceError: string | null;
  voiceMode: string;
  voiceTitle: string;
};

type SelectOption = {
  label: string;
  value: string;
};

type AiChatComposerProps = {
  activeSessionSending: boolean;
  compacting?: boolean;
  agentOptions: SelectOption[];
  attachments: AiComposerAttachmentView[];
  attachFiles: (files: FileList | File[] | null) => void;
  canSend: boolean;
  contextOpen: boolean;
  contextTitle: string;
  contextUsage: AiChatContextUsageSummary & AiChatContextUsageMeta;
  contextDrops?: AiChatContextDropSummary | null;
  disabled: boolean;
  draggingFiles: boolean;
  effortOptions: SelectOption[];
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
  modelOptions: SelectOption[];
  modelSupportsEffort: boolean;
  providerOptions: SelectOption[];
  selectedProviderId: string;
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
  updateProvider: (selectedProviderId: string) => void;
  voiceInput: AiComposerVoiceState;
};

export function AiChatComposer({
  activeSessionSending,
  compacting = false,
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
  providerOptions,
  selectedProviderId,
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
  updateProvider,
  voiceInput,
}: AiChatComposerProps) {
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
      <div className="ai-composer-input-wrap" data-voice-recording={voiceInput.voiceMode === "recording" || undefined} data-voice-transcribing={voiceInput.voiceMode === "transcribing" || undefined}>
        <AiComposerInlineMentions message={message} />
        {voiceInput.voiceMode === "recording" && (
          <div className="ai-composer-voice-indicator">
            <span className="ai-voice-live-dot" />
            <span className="ai-composer-voice-label">{t("aiChat.voiceInput.recording")}</span>
            <div className="ai-voice-live-meter" aria-hidden="true">
              <span /><span /><span /><span />
            </div>
          </div>
        )}
        {voiceInput.voiceMode === "transcribing" && (
          <div className="ai-composer-voice-indicator">
            <span className="ai-voice-transcribing-icon" />
            <span className="ai-composer-voice-label">{t("aiChat.voiceInput.transcribing")}</span>
          </div>
        )}
        {voiceInput.voiceError && (
          <div className="ai-composer-voice-error">{voiceInput.voiceError}</div>
        )}
        <textarea
          ref={textareaRef}
          value={message}
          onChange={handleMessageChange}
          onKeyDown={handleComposerKeyDown}
          onPaste={handleComposerPaste}
          placeholder={compacting ? t("aiChat.composer.compacting") : t("aiChat.composer.placeholder")}
          disabled={disabled || compacting}
          rows={1}
        />
      </div>
      <AiComposerAttachments
        attachments={attachments}
        draggingFiles={draggingFiles}
        removeAttachment={removeAttachment}
        t={t}
      />
      <div className="ai-composer-actions">
        <div className="ai-composer-left-actions">
          <input
            ref={fileInputRef}
            className="sr-only"
            type="file"
            multiple
            disabled={disabled}
            onChange={(event) => {
              attachFiles(event.currentTarget.files);
              event.currentTarget.value = "";
            }}
          />
          <button className="icon-button compact" type="button" aria-label={t("aiChat.attachFiles")} title={t("aiChat.attachFiles")} disabled={disabled} onClick={() => fileInputRef.current?.click()}>
            <Paperclip size={15} />
          </button>
          {providerOptions.length > 1 && (
            <CompactDropdown
              className="ai-composer-select ai-composer-select-provider"
              icon={<Server size={13} />}
              label={t("aiChat.provider.label")}
              value={selectedProviderId}
              options={providerOptions}
              onChange={updateProvider}
            />
          )}
          <CompactDropdown
            className="ai-composer-select"
            icon={<Bot size={13} />}
            label={t("aiChat.mode.agent")}
            value={preferences.selectedAgentId}
            options={agentOptions}
            onChange={(selectedAgentId) => updateAiPreference({ selectedAgentId })}
          />
          <CompactDropdown
            className="ai-composer-select"
            icon={<Sparkles size={13} />}
            label={t("aiChat.model.label")}
            value={selectedModelId}
            options={modelOptions}
            onChange={updateModel}
          />
          {modelSupportsEffort && (
            <CompactDropdown
              className="ai-composer-select"
              icon={<Brain size={13} />}
              label={t("aiChat.reasoningEffort.label")}
              value={preferences.selectedEffortId}
              options={effortOptions}
              onChange={(selectedEffortId) => updateAiPreference({ selectedEffortId })}
            />
          )}
          {attachments.length > 0 && <span className="ai-attachment-count">{attachments.length}</span>}
        </div>
        <div className="ai-composer-right-actions">
          <AiContextIndicator
            contextOpen={contextOpen}
            contextTitle={contextTitle}
            contextUsage={contextUsage}
            contextDrops={contextDrops}
            onCompact={onContextCompact}
            onOpenSettings={onOpenSettings}
            setContextOpen={setContextOpen}
            t={t}
          />
          <button
            className="ai-voice-button"
            type="button"
            aria-label={t("aiChat.voiceInput.aria")}
            title={voiceInput.voiceTitle}
            data-recording={voiceInput.voiceMode === "recording" || voiceInput.listening}
            data-transcribing={voiceInput.voiceMode === "transcribing"}
            disabled={disabled || !voiceInput.canUseVoice || voiceInput.voiceMode === "transcribing"}
            onClick={voiceInput.toggleVoiceInput}
          >
            <Mic size={14} />
          </button>
          <button
            className="ai-send-button"
            type="button"
            aria-label={t("aiChat.send.aria")}
            title={activeSessionSending ? t("aiChat.stop.aria") : selectedProviderReady ? t("aiChat.send.aria") : t("aiChat.send.disabledTooltip")}
            disabled={disabled || (!activeSessionSending && !canSend)}
            onClick={activeSessionSending ? handleCancelSend : () => handleSend()}
          >
            {activeSessionSending ? <Square size={13} /> : message.trim() ? <SendHorizontal size={15} /> : <CornerDownLeft size={15} />}
          </button>
        </div>
      </div>
    </div>
  );
}


