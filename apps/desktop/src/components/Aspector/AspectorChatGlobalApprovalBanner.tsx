import { Shield } from "lucide-react";
import type { PendingToolApprovalRef } from "../../lib/aspector/chat/pending-approval";
import type { AiToolApprovalDecision } from "../../lib/aspector/chat/types";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AspectorChatGlobalApprovalBannerProps = {
  pending: PendingToolApprovalRef;
  hiddenOnActiveSession: boolean;
  onFocusSession: () => void;
  onDecision: (approvalId: string, decision: AiToolApprovalDecision) => void;
  t: TranslateFn;
};

export function AspectorChatGlobalApprovalBanner({
  pending,
  hiddenOnActiveSession,
  onFocusSession,
  onDecision,
  t,
}: AspectorChatGlobalApprovalBannerProps) {
  if (hiddenOnActiveSession || !pending.toolCall.approval) return null;
  const approval = pending.toolCall.approval;

  return (
    <div className="ai-chat-global-approval" role="alertdialog" aria-label={t("aiChat.approval.globalAria")}>
      <div className="ai-chat-global-approval-head">
        <Shield size={14} />
        <div>
          <strong>{t("aiChat.approval.globalTitle")}</strong>
          <span>{approval.title}</span>
        </div>
        <button type="button" className="ai-chat-global-approval-focus" onClick={onFocusSession}>
          {t("aiChat.approval.openSession")}
        </button>
      </div>
      <p>{approval.summary}</p>
      <div className="ai-tool-approval-actions">
        <button type="button" className="ai-tool-approval-reject" onClick={() => onDecision(approval.id, "rejected")}>
          {approval.rejectLabel}
        </button>
        <button type="button" className="ai-tool-approval-approve" data-risk={approval.risk} onClick={() => onDecision(approval.id, "approved")}>
          {approval.approveLabel}
        </button>
      </div>
    </div>
  );
}