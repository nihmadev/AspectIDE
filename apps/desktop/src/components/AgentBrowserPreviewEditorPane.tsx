import { AgentBrowserPreview } from "./ai-chat/AgentBrowserPreview";
import { chatSessionIdFromBrowserPreviewPath } from "../lib/agentBrowserPreviewDocument";
import { useTranslation } from "../lib/i18n/useTranslation";
import { luxCommands } from "../lib/tauri";
import type { AiPreferences } from "../lib/aiPreferences";
import type { DocumentSnapshot } from "../lib/types";

type AgentBrowserPreviewEditorPaneProps = {
  document: DocumentSnapshot;
  preferences: AiPreferences;
};

export function AgentBrowserPreviewEditorPane({ document, preferences }: AgentBrowserPreviewEditorPaneProps) {
  const { t } = useTranslation();
  const chatSessionId = chatSessionIdFromBrowserPreviewPath(document.path);
  if (!chatSessionId) {
    return <div className="agent-browser-preview-editor-empty">{t("aiChat.browserPreview.disabled")}</div>;
  }

  return (
    <div className="agent-browser-preview-editor">
      <AgentBrowserPreview
        variant="editor"
        chatSessionId={chatSessionId}
        preferences={preferences}
        onOpenDashboard={() => {
          void luxCommands.agentBrowserDashboard({
            action: "start",
            port: preferences.agentBrowserDashboardPort,
            commandPath: preferences.agentBrowserCommand.trim() || null,
          })
            .then((response) => {
              if (response.url) return luxCommands.fileOpenExternal(response.url);
            });
        }}
      />
    </div>
  );
}