import { Bot, Brain, Paperclip, Server, Sparkles } from "lucide-react";
import { memo } from "react";
import type { ReactNode, RefObject } from "react";
import { CompactDropdown } from "../CompactDropdown";
import type { AiComposerSelectOption } from "./aiComposerTypes";
import type { AiPreferences } from "../../lib/aiPreferences";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiComposerModelControlsProps = {
  disabled: boolean;
  fileInputRef: RefObject<HTMLInputElement | null>;
  attachFiles: (files: FileList | File[] | null) => void;
  attachmentCount: number;
  providerOptions: AiComposerSelectOption[];
  selectedProviderId: string;
  updateProvider: (selectedProviderId: string) => void;
  agentOptions: AiComposerSelectOption[];
  modelOptions: AiComposerSelectOption[];
  selectedModelId: string;
  updateModel: (selectedModelId: string) => void;
  modelSupportsEffort: boolean;
  effortOptions: AiComposerSelectOption[];
  modelSearchPlaceholder?: string;
  modelSearchEmptyHint?: string;
  onHideModel?: (value: string) => void;
  hideModelLabel?: string;
  modelFooter?: ReactNode;
  preferences: AiPreferences;
  updateAiPreference: (patch: Partial<AiPreferences>) => void;
  t: TranslateFn;
};

/** Left composer actions: attach + provider/agent/model/effort selection. */
export const AiComposerModelControls = memo(function AiComposerModelControls({
  disabled,
  fileInputRef,
  attachFiles,
  attachmentCount,
  providerOptions,
  selectedProviderId,
  updateProvider,
  agentOptions,
  modelOptions,
  selectedModelId,
  updateModel,
  modelSupportsEffort,
  effortOptions,
  modelSearchPlaceholder,
  modelSearchEmptyHint,
  onHideModel,
  hideModelLabel,
  modelFooter,
  preferences,
  updateAiPreference,
  t,
}: AiComposerModelControlsProps) {
  return (
    <div className="ai-composer-left-actions">
      <input
        ref={fileInputRef}
        className="sr-only"
        type="file"
        multiple
        disabled={disabled}
        onChange={(event) => {
          attachFiles(event.currentTarget.files);
          event.currentTarget.value = "";
        }}
      />
      <button
        className="icon-button compact"
        type="button"
        aria-label={t("aiChat.attachFiles")}
        title={t("aiChat.attachFiles")}
        disabled={disabled}
        onClick={() => fileInputRef.current?.click()}
      >
        <Paperclip size={15} />
      </button>
      {providerOptions.length > 1 && (
        <CompactDropdown
          className="ai-composer-select ai-composer-select-provider"
          icon={<Server size={13} />}
          label={t("aiChat.provider.label")}
          value={selectedProviderId}
          options={providerOptions}
          onChange={updateProvider}
        />
      )}
      <CompactDropdown
        className="ai-composer-select"
        icon={<Bot size={13} />}
        label={t("aiChat.mode.agent")}
        value={preferences.selectedAgentId}
        options={agentOptions}
        onChange={(selectedAgentId) => updateAiPreference({ selectedAgentId })}
      />
      <CompactDropdown
        className="ai-composer-select"
        icon={<Sparkles size={13} />}
        label={t("aiChat.model.label")}
        value={selectedModelId}
        options={modelOptions}
        onChange={updateModel}
        searchable
        searchPlaceholder={modelSearchPlaceholder}
        searchEmptyLabel={modelSearchEmptyHint}
        onHideOption={onHideModel}
        hideOptionLabel={hideModelLabel}
        footer={modelFooter}
      />
      {modelSupportsEffort && (
        <CompactDropdown
          className="ai-composer-select"
          icon={<Brain size={13} />}
          label={t("aiChat.reasoningEffort.label")}
          value={preferences.selectedEffortId}
          options={effortOptions}
          onChange={(selectedEffortId) => updateAiPreference({ selectedEffortId })}
        />
      )}
      {attachmentCount > 0 && <span className="ai-attachment-count">{attachmentCount}</span>}
    </div>
  );
});
