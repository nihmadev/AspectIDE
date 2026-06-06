import { AlertTriangle } from "lucide-react";
import { useMemo } from "react";
import { listUnverifiedPathsInAssistantMessage, shouldShowPathEvidenceNotice } from "../../lib/aiChatPathEvidence";
import type { AiChatMessage } from "../../lib/aiChatTypes";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiPathEvidenceNoticeProps = {
  message: AiChatMessage;
  streaming: boolean;
  t: TranslateFn;
};

export function AiPathEvidenceNotice({ message, streaming, t }: AiPathEvidenceNoticeProps) {
  const unverified = useMemo(() => listUnverifiedPathsInAssistantMessage(message), [message]);
  const show = useMemo(() => shouldShowPathEvidenceNotice(message, streaming), [message, streaming]);

  if (!show) return null;

  const preview = unverified.slice(0, 4).join(", ");
  const extra = unverified.length > 4 ? ` +${unverified.length - 4}` : "";

  return (
    <p className="ai-chat-path-evidence-notice" role="note">
      <AlertTriangle size={12} aria-hidden />
      <span>{t("aiChat.pathEvidence.summary", { paths: `${preview}${extra}` })}</span>
    </p>
  );
}