import { Bot, Brain, CornerDownLeft, Mic, Paperclip, SendHorizontal, Sparkles, Square, X } from "lucide-react";
import type { ChangeEvent, ClipboardEvent, CSSProperties, DragEvent, KeyboardEvent, RefObject } from "react";
import { CompactDropdown } from "../CompactDropdown";
import { formatAiChatContextValue, formatCompactTokens, type AiChatContextUsageSummary } from "../../lib/aiChatContextUsage";
import type { AiPreferences } from "../../lib/aiPreferences";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

export type AiComposerAttachment = {
  id: string;
  name: string;
  size: number;
};

export type AiComposerVoiceState = {
  canUseVoice: boolean;
  listening: boolean;
  toggleVoiceInput: () => void;
  voiceMode: string;
  voiceTitle: string;
};

type SelectOption = {
  label: string;
  value: string;
};

type AiChatComposerProps = {
  activeSessionSending: boolean;
  agentOptions: SelectOption[];
  attachments: AiComposerAttachment[];
  attachFiles: (files: FileList | File[] | null) => void;
  canSend: boolean;
  contextOpen: boolean;
  contextTitle: string;
  contextUsage: AiChatContextUsageSummary;
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
  isAgentHome: boolean;
  message: string;
  modelOptions: SelectOption[];
  modelSupportsEffort: boolean;
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
  voiceInput: AiComposerVoiceState;
};

export function AiChatComposer({
  activeSessionSending,
  agentOptions,
  attachments,
  attachFiles,
  canSend,
  contextOpen,
  contextTitle,
  contextUsage,
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
  message,
  modelOptions,
  modelSupportsEffort,
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
      {contextOpen && <AiContextPopover contextUsage={contextUsage} setContextOpen={setContextOpen} t={t} />}
      <textarea
        ref={textareaRef}
        value={message}
        onChange={handleMessageChange}
        onKeyDown={handleComposerKeyDown}
        onPaste={handleComposerPaste}
        placeholder={t("aiChat.composer.placeholder")}
        disabled={disabled}
        rows={1}
      />
      {attachments.length > 0 && <AiAttachmentList attachments={attachments} removeAttachment={removeAttachment} t={t} />}
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
          <button className="ai-context-square" type="button" aria-label={t("aiChat.context.label")} title={contextTitle} data-active={contextOpen} style={{ "--context-percent": contextUsage.percent } as CSSProperties} onClick={() => setContextOpen((open) => !open)}>
            <span className="ai-context-square-fill" aria-hidden="true" />
            <span className="ai-context-square-value">{contextUsage.percent}</span>
            <span className="ai-context-square-unit">%</span>
          </button>
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

function AiContextPopover({ contextUsage, setContextOpen, t }: {
  contextUsage: AiChatContextUsageSummary;
  setContextOpen: (open: boolean | ((open: boolean) => boolean)) => void;
  t: TranslateFn;
}) {
  return (
    <div className="ai-context-popover">
      <div className="ai-context-popover-head">
        <div>
          <span>{t("aiChat.context.label")}</span>
          <strong>{t("aiChat.context.full", { percent: contextUsage.percent })}</strong>
          <small>{t("aiChat.context.distribution")}</small>
        </div>
        <button type="button" aria-label={t("aiChat.context.closeAria")} title={t("common.close")} onClick={() => setContextOpen(false)}>
          <X size={13} />
        </button>
      </div>
      <div className="ai-context-token-row">
        <span>{t("aiChat.context.estimatedUsage")}</span>
        <strong>{t("aiChat.context.tokenUsage", { totalTokens: formatCompactTokens(contextUsage.totalTokens), tokenBudget: formatCompactTokens(contextUsage.tokenBudget) })}</strong>
      </div>
      <div className="ai-context-meter" aria-hidden="true">
        {contextUsage.rows.map((row) => (
          <span key={row.id} style={{ width: `${row.percent}%`, background: row.color }} />
        ))}
      </div>
      <dl className="ai-context-breakdown">
        {contextUsage.rows.map((row) => (
          <div key={row.id}>
            <dt><span style={{ background: row.color }} />{row.label}</dt>
            <dd title={row.detail || undefined}>{formatAiChatContextValue(row)}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

function AiAttachmentList({ attachments, removeAttachment, t }: {
  attachments: AiComposerAttachment[];
  removeAttachment: (id: string) => void;
  t: TranslateFn;
}) {
  return (
    <div className="ai-attachment-list" aria-label={t("aiChat.attachments.aria")}>
      {attachments.map((attachment) => {
        const size = formatBytes(attachment.size, t);
        return (
          <span className="ai-attachment-chip" key={attachment.id} title={t("aiChat.attachment.tooltip", { name: attachment.name, size })}>
            <span>{attachment.name}</span>
            <small>{size}</small>
            <button type="button" aria-label={t("aiChat.attachment.removeAria", { name: attachment.name })} title={t("common.remove")} onClick={() => removeAttachment(attachment.id)}>
              <X size={12} />
            </button>
          </span>
        );
      })}
    </div>
  );
}

function formatBytes(bytes: number, t: TranslateFn) {
  if (bytes < 1024) return t("common.fileSize.bytes", { bytes });
  const kilobytes = bytes / 1024;
  if (kilobytes < 1024) return t("common.fileSize.kilobytes", { kilobytes: kilobytes.toFixed(kilobytes >= 10 ? 0 : 1) });
  const megabytes = kilobytes / 1024;
  return t("common.fileSize.megabytes", { megabytes: megabytes.toFixed(megabytes >= 10 ? 0 : 1) });
}
