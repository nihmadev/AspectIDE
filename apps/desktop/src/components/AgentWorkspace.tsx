import { Bot, Brain, FolderOpen, Mic, Paperclip, Plus, Search, SendHorizontal, Settings, ShieldCheck } from "lucide-react";
import type { ChangeEvent, FormEvent, ReactNode } from "react";
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { CompactDropdown } from "./CompactDropdown";
import { AI_PREFERENCES_KEY, getAiModel, getAiProvider, mergeAiPreferences, type AiPreferences, type AiToolApprovalMode } from "../lib/aiPreferences";
import { useTranslation } from "../lib/i18n/useTranslation";
import { useLuxStore } from "../lib/store";
import { luxCommands } from "../lib/tauri";
import { useVoiceInput } from "../lib/useVoiceInput";

type AgentWorkspaceProps = {
  onOpenProject: () => void;
};

export function AgentWorkspace({ onOpenProject }: AgentWorkspaceProps) {
  const { t } = useTranslation();
  const [composerValue, setComposerValue] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const workspace = useLuxStore((state) => state.workspace);
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const setAiPreferences = useLuxStore((state) => state.setAiPreferences);
  const setSettingsOpen = useLuxStore((state) => state.setSettingsOpen);

  const selectedProvider = getAiProvider(aiPreferences.providers, aiPreferences.selectedProviderId) ?? aiPreferences.providers[0] ?? null;
  const selectedModel = getAiModel(selectedProvider, aiPreferences.selectedModelId) ?? selectedProvider?.models[0] ?? null;
  const selectedEffort = selectedModel?.effortLevels.find((effort) => effort.id === aiPreferences.selectedEffortId) ?? selectedModel?.effortLevels[0] ?? null;
  const titleText = workspace ? t("agent.welcome.titleWithWorkspace", { workspaceName: workspace.name }) : t("agent.welcome.title");
  const toolApprovalOptions: Array<{ label: string; value: AiToolApprovalMode }> = [
    { label: t("aiChat.toolApproval.default"), value: "default" },
    { label: t("aiChat.toolApproval.fullAccess"), value: "full-access" },
  ];

  const resizeComposerTextarea = useCallback((target?: HTMLTextAreaElement | null) => {
    const textarea = target ?? textareaRef.current;
    if (!textarea) return;
    const maxHeight = 150;
    textarea.style.height = "auto";
    const nextHeight = Math.min(maxHeight, Math.max(58, textarea.scrollHeight));
    textarea.style.height = `${nextHeight}px`;
    textarea.style.overflowY = textarea.scrollHeight > maxHeight ? "auto" : "hidden";
  }, []);

  useLayoutEffect(() => {
    resizeComposerTextarea();
  }, [composerValue, resizeComposerTextarea]);

  const updateComposerValue = useCallback((nextValue: string) => {
    setComposerValue(nextValue);
    requestAnimationFrame(() => resizeComposerTextarea());
  }, [resizeComposerTextarea]);

  const handleComposerChange = useCallback((event: ChangeEvent<HTMLTextAreaElement>) => {
    resizeComposerTextarea(event.currentTarget);
    updateComposerValue(event.currentTarget.value);
  }, [resizeComposerTextarea, updateComposerValue]);

  const voiceInput = useVoiceInput({ message: composerValue, preferences: aiPreferences, updateMessage: updateComposerValue });
  const showVoiceAction = !composerValue.trim() || voiceInput.listening || voiceInput.voiceMode !== "idle";

  const updateAiPreference = useCallback((patch: Partial<AiPreferences>) => {
    const nextPreferences = mergeAiPreferences(aiPreferences, patch);
    setAiPreferences(nextPreferences);
    void luxCommands.settingsSet("user", AI_PREFERENCES_KEY, nextPreferences).catch(() => undefined);
  }, [aiPreferences, setAiPreferences]);

  const submitComposer = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!composerValue.trim()) return;
    updateComposerValue("");
  };

  return (
    <main className="agent-workspace" aria-label={t("agent.workspace.label")}>
      <aside className="agent-rail">
        <nav className="agent-nav" aria-label={t("agent.navigation.label")}>
          <AgentNavButton icon={<Plus size={15} />} label={t("agent.newChat")} />
          <AgentNavButton icon={<Search size={15} />} label={t("agent.search")} />
        </nav>

        <div className="agent-scroll-list">
          <AgentSidebarSection title={t("agent.sidebar.pinned")}>
            {workspace ? (
              <ProjectHeaderButton icon={<FolderOpen size={14} />} label={workspace.name} onClick={onOpenProject} />
            ) : (
              <ProjectHeaderButton icon={<FolderOpen size={14} />} label={t("agent.openProject")} onClick={onOpenProject} />
            )}
          </AgentSidebarSection>
        </div>

        <button className="agent-settings-link" type="button" onClick={() => setSettingsOpen(true)}>
          <Settings size={15} />
          <span>{t("agent.settings")}</span>
        </button>
      </aside>

      <section className="agent-home">
        <div className="agent-home-inner">
          <h1>{titleText}</h1>

          <form className="agent-prompt" onSubmit={submitComposer}>
            <textarea
              ref={textareaRef}
              value={composerValue}
              onChange={handleComposerChange}
              placeholder={t("agent.prompt.placeholder")}
              rows={1}
            />

            <div className="agent-prompt-actions">
              <div className="agent-prompt-left">
                <button className="agent-icon-button" type="button" aria-label={t("agent.attachFiles")} title={t("agent.attachFiles")}>
                  <Paperclip size={16} />
                </button>
              </div>

              <div className="agent-prompt-right">
                <CompactDropdown
                  className="agent-select"
                  icon={<Bot size={12} />}
                  label={t("common.agent")}
                  value={aiPreferences.selectedAgentId}
                  options={aiPreferences.agentProfiles.map((profile) => ({ label: profile.name, value: profile.id }))}
                  onChange={(selectedAgentId) => updateAiPreference({ selectedAgentId })}
                />
                <CompactDropdown
                  className="agent-select"
                  label={t("agent.model")}
                  value={selectedModel?.id ?? aiPreferences.selectedModelId}
                  options={selectedProvider?.models.map((model) => ({ label: model.name, value: model.id })) ?? []}
                  onChange={(selectedModelId) => updateAiPreference({ selectedModelId })}
                />
                {selectedModel && selectedModel.effortLevels.length > 0 && (
                  <CompactDropdown
                    className="agent-select"
                    icon={<Brain size={12} />}
                    label={t("agent.effort")}
                    value={selectedEffort?.id ?? aiPreferences.selectedEffortId}
                    options={selectedModel.effortLevels.map((effort) => ({ label: effort.label, value: effort.id }))}
                    onChange={(selectedEffortId) => updateAiPreference({ selectedEffortId })}
                  />
                )}
                <CompactDropdown<AiToolApprovalMode>
                  className="agent-select agent-tool-approval-select"
                  icon={<ShieldCheck size={12} />}
                  label={t("aiChat.toolApproval.label")}
                  value={aiPreferences.toolApprovalMode}
                  options={toolApprovalOptions}
                  onChange={(toolApprovalMode) => updateAiPreference({ toolApprovalMode })}
                />
                {showVoiceAction ? (
                  <button
                    className="agent-submit agent-voice-submit"
                    type="button"
                    aria-label={t("agent.voiceInput")}
                    title={voiceInput.voiceTitle}
                    data-recording={voiceInput.voiceMode === "recording" || voiceInput.listening}
                    data-transcribing={voiceInput.voiceMode === "transcribing"}
                    disabled={!voiceInput.canUseVoice || voiceInput.voiceMode === "transcribing"}
                    onClick={voiceInput.toggleVoiceInput}
                  >
                    <Mic size={16} />
                  </button>
                ) : (
                  <button className="agent-submit" type="submit" aria-label={t("agent.send")}>
                    <SendHorizontal size={16} />
                  </button>
                )}
              </div>
            </div>
          </form>
        </div>
      </section>
    </main>
  );
}

function AgentNavButton({ icon, label }: { icon: ReactNode; label: string }) {
  return (
    <button className="agent-nav-button" type="button">
      {icon}
      <span>{label}</span>
    </button>
  );
}

function AgentSidebarSection({ children, title }: { children: ReactNode; title: string }) {
  return (
    <section className="agent-sidebar-section">
      <h2>{title}</h2>
      {children}
    </section>
  );
}

function ProjectHeaderButton({ icon, label, onClick }: { icon: ReactNode; label: string; onClick: () => void }) {
  return (
    <button className="agent-project-row" type="button" onClick={onClick}>
      {icon}
      <span>{label}</span>
    </button>
  );
}
