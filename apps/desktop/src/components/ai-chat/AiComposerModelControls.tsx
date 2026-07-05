import { Bot, Brain, Paperclip, Sparkles } from "lucide-react";
import { memo } from "react";
import type { ReactNode, RefObject } from "react";
import { CompactDropdown } from "../CompactDropdown";
import type { AiComposerSelectOption } from "./aiComposerTypes";
import { getAiProvider, type AiPreferences } from "../../lib/aiPreferences";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiComposerModelControlsProps = {
  disabled: boolean;
  fileInputRef: RefObject<HTMLInputElement | null>;
  attachFiles: (files: FileList | File[] | null) => void;
  attachmentCount: number;
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

/** Left composer actions: attach + agent/model/effort selection. Provider choice
 *  lives inside the unified model picker (models are grouped per provider). */
export const AiComposerModelControls = memo(function AiComposerModelControls({
  disabled,
  fileInputRef,
  attachFiles,
  attachmentCount,
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
  // Provider shown as a quiet second line UNDER the model name in the picker
  // trigger (centered two-line box) — so a long provider never crowds or truncates
  // the model, and the model stays the primary label.
  const providerName = getAiProvider(preferences.providers, preferences.selectedProviderId)?.name?.trim() ?? "";
  const providerSubLabel = providerName
    ? <span className="ai-composer-model-provider" title={providerName}>{providerName}</span>
    : undefined;
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
      <CompactDropdown
        className="ai-composer-select"
        icon={<Bot size={13} />}
        label={t("aiChat.mode.agent")}
        value={preferences.selectedAgentId}
        options={agentOptions}
        onChange={(selectedAgentId) => updateAiPreference({ selectedAgentId })}
      />
      <CompactDropdown
        className="ai-composer-select ai-composer-select-model"
        icon={<Sparkles size={13} />}
        label={t("aiChat.model.label")}
        value={selectedModelId}
        options={modelOptions}
        onChange={updateModel}
        triggerSubLabel={providerSubLabel}
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
