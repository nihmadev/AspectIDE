import type { TranslateFn } from "../../i18n/useTranslation";
import type { AiChatSessionStatus } from "./../../store/index";

export function aiChatStatusLabel(status: AiChatSessionStatus, active: boolean, t: TranslateFn) {
  if (!active && status === "idle") return t("aiChat.status.idle");
  switch (status) {
    case "thinking": return t("aiChat.status.thinking");
    case "streaming": return t("aiChat.status.streaming");
    case "preparing": return t("aiChat.status.preparing");
    case "running-tools": return t("aiChat.status.tools");
    case "waiting-approval": return t("aiChat.status.approval");
    case "error": return t("aiChat.status.error");
    default: return active ? t("aiChat.status.ready") : t("aiChat.status.idle");
  }
}

export function aiChatSessionTitle(title: string, t: TranslateFn) {
  return title === "New chat" ? t("agent.newChat") : title;
}
