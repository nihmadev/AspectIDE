import { CornerDownLeft, Gauge, Mic, SendHorizontal, Square } from "lucide-react";
import { memo } from "react";
import { AspectorContextIndicator } from "./AspectorContextIndicator";
import { VOICE_MODE_RECORDING, VOICE_MODE_TRANSCRIBING, type AspectorComposerVoiceState } from "./AspectorComposerTypes";
import type { AiChatContextUsageMeta, AiChatContextUsageSummary } from "../../lib/aspector/chat/context-usage";
import type { AiChatContextDropSummary } from "../../lib/aspector/chat/context-report";
import { formatTokenSpeed } from "../../lib/hooks/use-live-token-speed";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AspectorComposerSendControlsProps = {
  /** Live tok/s of the running turn, or null to hide the readout. */
  tokenSpeed?: number | null;
  contextOpen: boolean;
  contextTitle: string;
  contextUsage: AiChatContextUsageSummary & AiChatContextUsageMeta;
  contextDrops?: AiChatContextDropSummary | null;
  onContextCompact?: () => void;
  onOpenSettings?: () => void;
  setContextOpen: (open: boolean | ((open: boolean) => boolean)) => void;
  voiceInput: AspectorComposerVoiceState;
  disabled: boolean;
  message: string;
  activeSessionSending: boolean;
  canSend: boolean;
  selectedProviderReady: boolean;
  handleSend: () => void;
  handleCancelSend: () => void;
  t: TranslateFn;
};

/** Right composer actions: context budget indicator + voice + send/stop. */
export const AspectorComposerSendControls = memo(function AspectorComposerSendControls({
  tokenSpeed = null,
  contextOpen,
  contextTitle,
  contextUsage,
  contextDrops,
  onContextCompact,
  onOpenSettings,
  setContextOpen,
  voiceInput,
  disabled,
  message,
  activeSessionSending,
  canSend,
  selectedProviderReady,
  handleSend,
  handleCancelSend,
  t,
}: AspectorComposerSendControlsProps) {
  const sendTitle = activeSessionSending
    ? t("aiChat.stop.aria")
    : selectedProviderReady
      ? t("aiChat.send.aria")
      : t("aiChat.send.disabledTooltip");
  return (
    <div className="ai-composer-right-actions">
      {tokenSpeed !== null && (
        <span
          className="ai-token-speed"
          role="status"
          aria-live="off"
          title={t("aiChat.tokenSpeed.title")}
        >
          <Gauge size={11} aria-hidden="true" />
          {formatTokenSpeed(tokenSpeed)}
          <em>tok/s</em>
        </span>
      )}
      <AspectorContextIndicator
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
        data-recording={voiceInput.voiceMode === VOICE_MODE_RECORDING || voiceInput.listening}
        data-transcribing={voiceInput.voiceMode === VOICE_MODE_TRANSCRIBING}
        disabled={disabled || !voiceInput.canUseVoice || voiceInput.voiceMode === VOICE_MODE_TRANSCRIBING}
        onClick={voiceInput.toggleVoiceInput}
      >
        <Mic size={14} className="ai-voice-icon" />
        {/* Real mic-level bars, swapped in for the icon while capturing — driven
            frame-by-frame from useVoiceInput's analyser via direct style writes,
            never React state (see voiceBarsRef). */}
        <span className="ai-voice-wave" ref={voiceInput.voiceBarsRef} aria-hidden="true">
          <span /><span /><span /><span /><span />
        </span>
      </button>
      <button
        className="ai-send-button"
        type="button"
        aria-label={t("aiChat.send.aria")}
        title={sendTitle}
        disabled={disabled || (!activeSessionSending && !canSend)}
        onClick={activeSessionSending ? handleCancelSend : () => handleSend()}
      >
        {activeSessionSending ? <Square size={13} /> : message.trim() ? <SendHorizontal size={15} /> : <CornerDownLeft size={15} />}
      </button>
    </div>
  );
});
