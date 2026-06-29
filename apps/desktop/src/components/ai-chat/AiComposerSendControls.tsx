import { CornerDownLeft, Mic, SendHorizontal, Square } from "lucide-react";
import { memo } from "react";
import { AiContextIndicator } from "./AiContextIndicator";
import { VOICE_MODE_RECORDING, VOICE_MODE_TRANSCRIBING, type AiComposerVoiceState } from "./aiComposerTypes";
import type { AiChatContextUsageMeta, AiChatContextUsageSummary } from "../../lib/aiChatContextUsage";
import type { AiChatContextDropSummary } from "../../lib/aiChatContextReport";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiComposerSendControlsProps = {
  contextOpen: boolean;
  contextTitle: string;
  contextUsage: AiChatContextUsageSummary & AiChatContextUsageMeta;
  contextDrops?: AiChatContextDropSummary | null;
  onContextCompact?: () => void;
  onOpenSettings?: () => void;
  setContextOpen: (open: boolean | ((open: boolean) => boolean)) => void;
  voiceInput: AiComposerVoiceState;
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
export const AiComposerSendControls = memo(function AiComposerSendControls({
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
}: AiComposerSendControlsProps) {
  const sendTitle = activeSessionSending
    ? t("aiChat.stop.aria")
    : selectedProviderReady
      ? t("aiChat.send.aria")
      : t("aiChat.send.disabledTooltip");
  return (
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
        data-recording={voiceInput.voiceMode === VOICE_MODE_RECORDING || voiceInput.listening}
        data-transcribing={voiceInput.voiceMode === VOICE_MODE_TRANSCRIBING}
        disabled={disabled || !voiceInput.canUseVoice || voiceInput.voiceMode === VOICE_MODE_TRANSCRIBING}
        onClick={voiceInput.toggleVoiceInput}
      >
        <Mic size={14} />
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
