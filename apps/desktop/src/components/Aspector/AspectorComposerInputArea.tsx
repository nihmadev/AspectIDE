import { memo } from "react";
import type { ChangeEvent, ClipboardEvent, KeyboardEvent, RefObject } from "react";
import { AspectorComposerInlineMentions } from "./AspectorComposerInlineMentions";
import { VOICE_MODE_RECORDING, VOICE_MODE_TRANSCRIBING, type AspectorComposerVoiceState } from "./AspectorComposerTypes";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AspectorComposerInputAreaProps = {
  message: string;
  disabled: boolean;
  compacting: boolean;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  handleMessageChange: (event: ChangeEvent<HTMLTextAreaElement>) => void;
  handleComposerKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  handleComposerPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void;
  voiceInput: AspectorComposerVoiceState;
  t: TranslateFn;
};

/** The text input shell: inline mentions overlay, voice/compaction status, textarea. */
export const AspectorComposerInputArea = memo(function AspectorComposerInputArea({
  message,
  disabled,
  compacting,
  textareaRef,
  handleMessageChange,
  handleComposerKeyDown,
  handleComposerPaste,
  voiceInput,
  t,
}: AspectorComposerInputAreaProps) {
  return (
    <div
      className="ai-composer-input-wrap"
      data-voice-recording={voiceInput.voiceMode === VOICE_MODE_RECORDING || undefined}
      data-voice-transcribing={voiceInput.voiceMode === VOICE_MODE_TRANSCRIBING || undefined}
    >
      <AspectorComposerInlineMentions message={message} />
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
          <span className="ai-composer-compacting-label">{t("aiChat.composer.compacting")}</span>
          {/* Indeterminate Codex-style sweep: summarization duration is unknown,
              so a travelling highlight beats fake percentages. */}
          <span className="ai-composer-compacting-track" aria-hidden="true">
            <span className="ai-composer-compacting-sweep" />
          </span>
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
