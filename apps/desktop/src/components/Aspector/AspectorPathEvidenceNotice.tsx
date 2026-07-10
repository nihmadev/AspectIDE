import { AlertTriangle } from "lucide-react";
import { useMemo } from "react";
import { listUnverifiedPathsInAssistantMessage, shouldShowPathEvidenceNotice } from '../../lib/aspector/chat/path-evidence';
import type { AiChatMessage } from '../../lib/aspector/chat/types';
import { normalizePath } from '../../lib/explorer/file-tree';
import type { TranslateFn } from '../../lib/i18n/useTranslation';
import { useLuxStore } from '../../lib/store/index';

type AspectorPathEvidenceNoticeProps = {
  message: AiChatMessage;
  streaming: boolean;
  t: TranslateFn;
};

export function AspectorPathEvidenceNotice({ message, streaming, t }: AspectorPathEvidenceNoticeProps) {
  const workspaceRoot = useLuxStore((state) => state.workspace?.root ?? null);
  const fileTreeDirectories = useLuxStore((state) => state.fileTreeDirectories);
  // The workspace's real top-level directories gate which slash-separated
  // citations count as directory paths — prose lists ("web/browser/MCP/SSH/")
  // share the same shape and must not trigger the notice.
  const knownRoots = useMemo(() => {
    if (!workspaceRoot) return new Set<string>();
    const entries = fileTreeDirectories[normalizePath(workspaceRoot)] ?? [];
    return new Set(
      entries
        .filter((entry) => entry.kind === "directory")
        .map((entry) => entry.name.toLowerCase()),
    );
  }, [workspaceRoot, fileTreeDirectories]);

  const unverified = useMemo(
    () => listUnverifiedPathsInAssistantMessage(message, knownRoots),
    [message, knownRoots],
  );
  const show = useMemo(
    () => shouldShowPathEvidenceNotice(message, streaming, knownRoots),
    [message, streaming, knownRoots],
  );

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
