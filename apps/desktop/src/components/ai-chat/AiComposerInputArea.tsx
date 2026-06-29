import { memo } from "react";
import type { ChangeEvent, ClipboardEvent, KeyboardEvent, RefObject } from "react";
import { AiComposerInlineMentions } from "./AiComposerInlineMentions";
import { VOICE_MODE_RECORDING, VOICE_MODE_TRANSCRIBING, type AiComposerVoiceState } from "./aiComposerTypes";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiComposerInputAreaProps = {
  message: string;
  disabled: boolean;
  compacting: boolean;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  handleMessageChange: (event: ChangeEvent<HTMLTextAreaElement>) => void;
  handleComposerKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  handleComposerPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void;
  voiceInput: AiComposerVoiceState;
  t: TranslateFn;
};

/** The text input shell: inline mentions overlay, voice/compaction status, textarea. */
export const AiComposerInputArea = memo(function AiComposerInputArea({
  message,
  disabled,
  compacting,
  textareaRef,
  handleMessageChange,
  handleComposerKeyDown,
  handleComposerPaste,
  voiceInput,
  t,
}: AiComposerInputAreaProps) {
  return (
    <div
      className="ai-composer-input-wrap"
      data-voice-recording={voiceInput.voiceMode === VOICE_MODE_RECORDING || undefined}
      data-voice-transcribing={voiceInput.voiceMode === VOICE_MODE_TRANSCRIBING || undefined}
    >
      <AiComposerInlineMentions message={message} />
      {voiceInput.voiceMode === VOICE_MODE_RECORDING && (
        <div className="ai-composer-voice-indicator">
          <span className="ai-voice-live-dot" />
          <span className="ai-composer-voice-label">{t("aiChat.voiceInput.recording")}</span>
          <div className="ai-voice-live-meter" aria-hidden="true">
            <span /><span /><span /><span />
          </div>
        </div>
      )}
      {voiceInput.voiceMode === VOICE_MODE_TRANSCRIBING && (
        <div className="ai-composer-voice-indicator">
          <span className="ai-voice-transcribing-icon" />
          <span className="ai-composer-voice-label">{t("aiChat.voiceInput.transcribing")}</span>
        </div>
      )}
      {voiceInput.voiceError && (
        <div className="ai-composer-voice-error">{voiceInput.voiceError}</div>
      )}
      {compacting && (
        <div className="ai-composer-compacting" role="status" aria-live="polite">
          <span className="ai-composer-compacting-bar" aria-hidden="true">
            <span /><span /><span />
          </span>
          <span className="ai-composer-compacting-label">{t("aiChat.composer.compacting")}</span>
        </div>
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
  );
});
