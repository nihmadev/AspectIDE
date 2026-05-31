import * as Dialog from "@radix-ui/react-dialog";
import { Check, ChevronDown, ChevronLeft, ChevronRight, Cpu, Database, Eye, Globe, Minus, Plus, RotateCcw, Search, Settings, Sparkles, Trash2, X } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AI_PREFERENCES_KEY,
  AI_PROVIDER_PRESETS,
  buildLocalProxyBaseUrl,
  createAiEffortConfig,
  createAiModelConfig,
  createAiProviderConfig,
  defaultAiPreferences,
  getAiAgentProfile,
  getAiModel,
  getAiProvider,
  isDefaultAiAgentProfile,
  mergeAiPreferences,
  normalizeAiPreferences,
  type AiAgentMode,
  type AiEffortConfig,
  type AiModelConfig,
  type AiPreferences,
  type AiProviderConfig,
  type AiProviderPresetId,
  type AiProviderProtocol,
  type AiToolApprovalMode,
} from "../lib/aiPreferences";
import {
  defaultEditorPreferences,
  EDITOR_PREFERENCES_KEY,
  mergeEditorPreferences,
  normalizeEditorPreferences,
  type EditorPreferences,
  type RenderWhitespaceSetting,
  type WordWrapSetting,
} from "../lib/editorPreferences";
import { LOCALES, UI_LOCALE_KEY, type Locale, type MessageKey } from "../lib/i18n";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
import { useLuxStore } from "../lib/store";
import { luxCommands } from "../lib/tauri";

const scope = "user" as const;

// Settings are organized into three flat tabs. Each tab renders one or more stacked panels.
type SettingsSectionId = "general" | "editor" | "ai";
type SaveState = "idle" | "saving" | "saved" | "error";

type SettingsSection = {
  id: SettingsSectionId;
  titleKey: MessageKey;
  descriptionKey: MessageKey;
  icon: ReactNode;
  keywords: string[];
};

// Provider preset id → localized description key. Brand names stay verbatim; only the
// human-readable description is translated.
const PROVIDER_PRESET_DESCRIPTION_KEYS: Record<string, MessageKey> = {
  openai: "settings.providerPreset.openai.description",
  anthropic: "settings.providerPreset.anthropic.description",
  openrouter: "settings.providerPreset.openrouter.description",
  google: "settings.providerPreset.google.description",
  mistral: "settings.providerPreset.mistral.description",
  groq: "settings.providerPreset.groq.description",
  cohere: "settings.providerPreset.cohere.description",
  deepseek: "settings.providerPreset.deepseek.description",
  xai: "settings.providerPreset.xai.description",
  "azure-openai": "settings.providerPreset.azureOpenai.description",
  ollama: "settings.providerPreset.ollama.description",
  "lm-studio": "settings.providerPreset.lmStudio.description",
  "local-proxy": "settings.providerPreset.localProxy.description",
  custom: "settings.providerPreset.custom.description",
};

const settingsSections: SettingsSection[] = [
  {
    id: "general",
    titleKey: "settings.general.title",
    descriptionKey: "settings.general.description",
    icon: <Globe size={16} />,
    keywords: ["language", "locale", "russian", "english", "язык", "general", "язык", "общие"],
  },
  {
    id: "editor",
    titleKey: "settings.group.editor",
    descriptionKey: "settings.editor.description",
    icon: <Eye size={16} />,
    keywords: ["font", "line", "tab", "whitespace", "unicode", "minimap", "word wrap", "mouse", "zoom", "smooth", "ligatures", "appearance", "behavior", "редактор", "шрифт"],
  },
  {
    id: "ai",
    titleKey: "settings.group.ai",
    descriptionKey: "settings.ai.description",
    icon: <Sparkles size={16} />,
    keywords: ["agent", "mode", "model", "effort", "reasoning", "openai", "anthropic", "openrouter", "gemini", "proxy", "api key", "base url", "models", "index", "files", "images", "metadata", "context", "провайдер", "модель", "индекс"],
  },
];

const sectionById = new Map(settingsSections.map((section) => [section.id, section]));

export function SettingsDialog() {
  const { t } = useTranslation();
  const open = useLuxStore((state) => state.settingsOpen);
  const setOpen = useLuxStore((state) => state.setSettingsOpen);
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const aiIndex = useLuxStore((state) => state.aiIndex);
  const setAiPreferences = useLuxStore((state) => state.setAiPreferences);
  const updateAiPreferences = useLuxStore((state) => state.updateAiPreferences);
  const editorPreferences = useLuxStore((state) => state.editorPreferences);
  const setEditorPreferences = useLuxStore((state) => state.setEditorPreferences);
  const updateEditorPreferences = useLuxStore((state) => state.updateEditorPreferences);
  const locale = useLuxStore((state) => state.locale);
  const setLocale = useLuxStore((state) => state.setLocale);
  const [activeSectionId, setActiveSectionId] = useState<SettingsSectionId>("general");
  const [query, setQuery] = useState("");
  const [saveState, setSaveState] = useState<SaveState>("idle");

  const persistLocale = useCallback(
    (nextLocale: Locale) => {
      setLocale(nextLocale);
      setSaveState("saving");
      void luxCommands.settingsSet(scope, UI_LOCALE_KEY, nextLocale)
        .then(() => setSaveState("saved"))
        .catch(() => setSaveState("error"));
    },
    [setLocale],
  );

  useEffect(() => {
    if (!open) return;

    let cancelled = false;
    void luxCommands.settingsGet(scope, AI_PREFERENCES_KEY).then((setting) => {
      if (!cancelled && setting) setAiPreferences(normalizeAiPreferences(setting.value));
    }).catch(() => undefined);
    void luxCommands.settingsGet(scope, EDITOR_PREFERENCES_KEY).then((setting) => {
      if (!cancelled && setting) setEditorPreferences(normalizeEditorPreferences(setting.value));
    }).catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, [open, setAiPreferences, setEditorPreferences]);

  const persistAiPreferences = useCallback(
    (nextPreferences: AiPreferences) => {
      setAiPreferences(nextPreferences);
      setSaveState("saving");
      void luxCommands.settingsSet(scope, AI_PREFERENCES_KEY, nextPreferences)
        .then(() => setSaveState("saved"))
        .catch(() => setSaveState("error"));
    },
    [setAiPreferences],
  );

  const persistEditorPreferences = useCallback(
    (nextPreferences: EditorPreferences) => {
      setEditorPreferences(nextPreferences);
      setSaveState("saving");
      void luxCommands.settingsSet(scope, EDITOR_PREFERENCES_KEY, nextPreferences)
        .then(() => setSaveState("saved"))
        .catch(() => setSaveState("error"));
    },
    [setEditorPreferences],
  );

  const updateEditorPreference = useCallback(
    (patch: Partial<EditorPreferences>) => {
      const nextPreferences = mergeEditorPreferences(editorPreferences, patch);
      updateEditorPreferences(patch);
      persistEditorPreferences(nextPreferences);
    },
    [editorPreferences, persistEditorPreferences, updateEditorPreferences],
  );

  const updateAiPreference = useCallback(
    (patch: Partial<AiPreferences>) => {
      const nextPreferences = mergeAiPreferences(aiPreferences, patch);
      updateAiPreferences(patch);
      persistAiPreferences(nextPreferences);
    },
    [aiPreferences, persistAiPreferences, updateAiPreferences],
  );

  const filteredSections = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) return settingsSections;
    return settingsSections.filter((section) => sectionMatchesQuery(section, normalizedQuery, t));
  }, [query, t]);

  useEffect(() => {
    if (filteredSections.some((section) => section.id === activeSectionId)) return;
    setActiveSectionId(filteredSections[0]?.id ?? "general");
  }, [activeSectionId, filteredSections]);

  const activeSection = sectionById.get(activeSectionId) ?? settingsSections[0];

  return (
    <Dialog.Root open={open} onOpenChange={setOpen}>
      <Dialog.Portal>
        <Dialog.Overlay className="settings-overlay" />
        <Dialog.Content className="settings-dialog" aria-describedby={undefined}>
          <div className="settings-shell">
            <header className="settings-header">
              <div className="settings-title">
                <Settings size={17} />
                <Dialog.Title>{t("settings.title")}</Dialog.Title>
                <SaveIndicator state={saveState} t={t} />
              </div>
              <button className="icon-button compact" type="button" aria-label={t("settings.close")} title={t("settings.close")} onClick={() => setOpen(false)}>
                <X size={15} />
              </button>
            </header>

            <aside className="settings-sidebar" aria-label={t("settings.sections.aria")}>
              <label className="settings-search">
                <Search size={15} />
                <input aria-label={t("settings.search.aria")} value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("settings.search.placeholder")} />
              </label>
              <SettingsSectionNav sections={filteredSections} activeSectionId={activeSectionId} onSelect={setActiveSectionId} t={t} />
            </aside>

            <main className="settings-main">
              <div className="settings-main-header">
                <div>
                  <h2>{t(activeSection.titleKey)}</h2>
                  <p>{t(activeSection.descriptionKey)}</p>
                </div>
                {activeSectionId !== "general" && (
                  <button className="settings-reset-button" type="button" onClick={() => resetSection(activeSectionId, persistEditorPreferences, persistAiPreferences)}>
                    <RotateCcw size={14} /> {t("settings.reset", { group: t(activeSection.titleKey) })}
                  </button>
                )}
              </div>

              <div className="settings-scroll-area">
                {activeSectionId === "general" && <GeneralSection locale={locale} onChangeLocale={persistLocale} t={t} />}
                {activeSectionId === "editor" && (
                  <div className="settings-section-stack">
                    <EditorAppearanceSection preferences={editorPreferences} onChange={updateEditorPreference} t={t} />
                    <EditorBehaviorSection preferences={editorPreferences} onChange={updateEditorPreference} t={t} />
                  </div>
                )}
                {activeSectionId === "ai" && (
                  <div className="settings-section-stack">
                    <AiActiveCard preferences={aiPreferences} onChange={updateAiPreference} t={t} />
                    <AiProvidersSection preferences={aiPreferences} onChange={updateAiPreference} t={t} />
                    <AiIndexingSection aiIndex={aiIndex} preferences={aiPreferences} onChange={updateAiPreference} t={t} />
                  </div>
                )}
              </div>
            </main>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function SettingsSectionNav({ activeSectionId, onSelect, sections, t }: { activeSectionId: SettingsSectionId; onSelect: (sectionId: SettingsSectionId) => void; sections: SettingsSection[]; t: TranslateFn }) {
  return (
    <div className="settings-nav-groups">
      <section className="settings-nav-group">
        {sections.map((section) => (
          <button className="settings-nav-item" type="button" key={section.id} data-active={section.id === activeSectionId} onClick={() => onSelect(section.id)}>
            <span className="settings-nav-icon">{section.icon}</span>
            <span>
              <strong>{t(section.titleKey)}</strong>
              <small>{t(section.descriptionKey)}</small>
            </span>
          </button>
        ))}
      </section>
    </div>
  );
}

function GeneralSection({ locale, onChangeLocale, t }: { locale: Locale; onChangeLocale: (locale: Locale) => void; t: TranslateFn }) {
  return (
    <SettingsPanel>
      <SettingsGrid>
        <SelectSetting<Locale>
          label={t("settings.language.label")}
          detail={t("settings.language.detail")}
          value={locale}
          options={LOCALES.map((entry) => ({ label: entry.nativeLabel, value: entry.id }))}
          onChange={onChangeLocale}
        />
      </SettingsGrid>
    </SettingsPanel>
  );
}

function EditorAppearanceSection({ onChange, preferences, t }: { onChange: (patch: Partial<EditorPreferences>) => void; preferences: EditorPreferences; t: TranslateFn }) {
  return (
    <SettingsPanel title={t("settings.appearance.title")} description={t("settings.appearance.description")}>
      <SettingsGrid>
        <NumberSetting label={t("settings.appearance.fontSize.label")} detail={t("settings.appearance.fontSize.detail")} value={preferences.fontSize} min={8} max={32} onChange={(fontSize) => onChange({ fontSize })} />
        <NumberSetting label={t("settings.appearance.lineHeight.label")} detail={t("settings.appearance.lineHeight.detail")} value={preferences.lineHeight} min={12} max={48} onChange={(lineHeight) => onChange({ lineHeight })} />
        <NumberSetting label={t("settings.appearance.tabSize.label")} detail={t("settings.appearance.tabSize.detail")} value={preferences.tabSize} min={2} max={8} onChange={(tabSize) => onChange({ tabSize })} />
        <SegmentedSetting<RenderWhitespaceSetting> label={t("settings.appearance.whitespace.label")} detail={t("settings.appearance.whitespace.detail")} value={preferences.renderWhitespace} options={[
          { label: t("settings.appearance.whitespace.none"), value: "none" },
          { label: t("settings.appearance.whitespace.selection"), value: "selection" },
          { label: t("settings.appearance.whitespace.all"), value: "all" },
        ]} onChange={(renderWhitespace) => onChange({ renderWhitespace })} />
        <ToggleSetting label={t("settings.appearance.minimap.label")} detail={t("settings.appearance.minimap.detail")} checked={preferences.minimap} onChange={(minimap) => onChange({ minimap })} />
        <ToggleSetting label={t("settings.appearance.fontLigatures.label")} detail={t("settings.appearance.fontLigatures.detail")} checked={preferences.fontLigatures} onChange={(fontLigatures) => onChange({ fontLigatures })} />
        <ToggleSetting label={t("settings.appearance.unicode.label")} detail={t("settings.appearance.unicode.detail")} checked={preferences.unicodeHighlightAmbiguousCharacters} onChange={(unicodeHighlightAmbiguousCharacters) => onChange({ unicodeHighlightAmbiguousCharacters })} />
      </SettingsGrid>
    </SettingsPanel>
  );
}

function EditorBehaviorSection({ onChange, preferences, t }: { onChange: (patch: Partial<EditorPreferences>) => void; preferences: EditorPreferences; t: TranslateFn }) {
  return (
    <SettingsPanel title={t("settings.behavior.title")} description={t("settings.behavior.description")}>
      <SettingsGrid>
        <SegmentedSetting<WordWrapSetting> label={t("settings.behavior.wordWrap.label")} detail={t("settings.behavior.wordWrap.detail")} value={preferences.wordWrap} options={[
          { label: t("settings.behavior.wordWrap.off"), value: "off" },
          { label: t("settings.behavior.wordWrap.on"), value: "on" },
        ]} onChange={(wordWrap) => onChange({ wordWrap })} />
        <ToggleSetting label={t("settings.behavior.mouseWheelZoom.label")} detail={t("settings.behavior.mouseWheelZoom.detail")} checked={preferences.mouseWheelZoom} onChange={(mouseWheelZoom) => onChange({ mouseWheelZoom })} />
        <ToggleSetting label={t("settings.behavior.smoothScrolling.label")} detail={t("settings.behavior.smoothScrolling.detail")} checked={preferences.smoothScrolling} onChange={(smoothScrolling) => onChange({ smoothScrolling })} />
      </SettingsGrid>
    </SettingsPanel>
  );
}

// A single focused "active model" card: the thing a user changes often (model + effort +
// mode) shown at a glance, with the selected agent's behavior editable inline. Provider/model
// management itself lives in the Providers section, so nothing is configured in two places.
const AI_AGENT_MODES: AiAgentMode[] = ["agent", "plan", "ask"];
const AI_TOOL_APPROVAL_MODES: AiToolApprovalMode[] = ["default", "full-access"];

function AiActiveCard({ onChange, preferences, t }: { onChange: (patch: Partial<AiPreferences>) => void; preferences: AiPreferences; t: TranslateFn }) {
  const selectedAgent = getAiAgentProfile(preferences.agentProfiles, preferences.selectedAgentId) ?? preferences.agentProfiles[0];
  const selectedProvider = getAiProvider(preferences.providers, preferences.selectedProviderId) ?? preferences.providers[0];
  const selectedModel = getAiModel(selectedProvider, preferences.selectedModelId) ?? selectedProvider.models[0];
  const selectedEffort = selectedModel.effortLevels.find((effort) => effort.id === preferences.selectedEffortId) ?? selectedModel.effortLevels[0] ?? null;
  const modeLabel = t(`settings.aiRuntime.mode.${selectedAgent.mode}` as MessageKey);

  const selectProvider = (providerId: string) => {
    const provider = getAiProvider(preferences.providers, providerId) ?? preferences.providers[0];
    onChange({ selectedProviderId: provider.id, selectedModelId: provider.models[0].id, selectedEffortId: provider.models[0].effortLevels[0]?.id ?? "" });
  };
  const selectModel = (modelId: string) => {
    const model = getAiModel(selectedProvider, modelId) ?? selectedProvider.models[0];
    onChange({ selectedProviderId: selectedProvider.id, selectedModelId: model.id, selectedEffortId: model.effortLevels[0]?.id ?? "" });
  };
  const selectMode = (mode: AiAgentMode) => {
    const profile = preferences.agentProfiles.find((candidate) => candidate.mode === mode && isDefaultAiAgentProfile(candidate.id))
      ?? preferences.agentProfiles.find((candidate) => candidate.mode === mode);
    if (profile) onChange({ selectedAgentId: profile.id });
  };
  const updateInstructions = (instructions: string) => {
    onChange({ agentProfiles: preferences.agentProfiles.map((profile) => profile.id === selectedAgent.id ? { ...profile, instructions } : profile) });
  };

  return (
    <section className="ai-active-card">
      <div className="ai-active-hero">
        <span className="ai-active-icon"><Sparkles size={20} /></span>
        <div className="ai-active-headline">
          <strong>{selectedModel.name}</strong>
          <span>{selectedProvider.name} · {modeLabel}</span>
        </div>
        {selectedEffort && <span className="ai-active-badge">{selectedEffort.label}</span>}
      </div>
      <SettingsGrid>
        <SelectSetting label={t("settings.aiRuntime.provider.label")} value={selectedProvider.id} options={preferences.providers.map((provider) => ({ label: provider.name, value: provider.id }))} onChange={selectProvider} />
        <SelectSetting label={t("settings.aiRuntime.model.label")} value={selectedModel.id} options={selectedProvider.models.map((model) => ({ label: model.name, value: model.id }))} onChange={selectModel} />
        {selectedModel.effortLevels.length > 0 && (
          <SegmentedSetting label={t("settings.aiRuntime.effort.label")} value={selectedEffort?.id ?? ""} options={selectedModel.effortLevels.map((effort) => ({ label: effort.label, value: effort.id }))} onChange={(selectedEffortId) => onChange({ selectedEffortId })} />
        )}
        <SegmentedSetting<AiAgentMode> label={t("settings.aiRuntime.mode.label")} value={selectedAgent.mode} options={AI_AGENT_MODES.map((mode) => ({ label: t(`settings.aiRuntime.mode.${mode}` as MessageKey), value: mode }))} onChange={selectMode} />
        <SegmentedSetting<AiToolApprovalMode> label={t("settings.aiRuntime.toolApproval.label")} detail={t("settings.aiRuntime.toolApproval.detail")} value={preferences.toolApprovalMode} options={AI_TOOL_APPROVAL_MODES.map((mode) => ({ label: t(`settings.aiRuntime.toolApproval.${mode}` as MessageKey), value: mode }))} onChange={(toolApprovalMode) => onChange({ toolApprovalMode })} />
        <TextSetting label={t("settings.aiRuntime.instructions.label")} detail={t("settings.aiRuntime.instructions.detail")} value={selectedAgent.instructions} onChange={updateInstructions} wide />
      </SettingsGrid>
    </section>
  );
}

// Provider management uses a master→detail flow: a list of provider tiles, and a focused
// editor screen reached by opening one. `openProviderId === null` means the list is shown.
function AiProvidersSection({ onChange, preferences, t }: { onChange: (patch: Partial<AiPreferences>) => void; preferences: AiPreferences; t: TranslateFn }) {
  const [openProviderId, setOpenProviderId] = useState<string | null>(null);
  const [providerPresetId, setProviderPresetId] = useState<AiProviderPresetId>("openai");

  const openProvider = openProviderId ? getAiProvider(preferences.providers, openProviderId) : null;

  // If the open provider was removed elsewhere, fall back to the list.
  useEffect(() => {
    if (openProviderId && !preferences.providers.some((provider) => provider.id === openProviderId)) {
      setOpenProviderId(null);
    }
  }, [openProviderId, preferences.providers]);

  const updateProviders = (providers: AiProviderConfig[], selectedProviderId = preferences.selectedProviderId, selectedModelId = preferences.selectedModelId, selectedEffortId = preferences.selectedEffortId) => {
    onChange({ providers, selectedProviderId, selectedModelId, selectedEffortId });
  };
  const activateProvider = (provider: AiProviderConfig) => {
    updateProviders(preferences.providers, provider.id, provider.models[0].id, provider.models[0].effortLevels[0]?.id ?? "");
  };
  const addProvider = () => {
    const nextProvider = createAiProviderConfig(preferences.providers, providerPresetId);
    updateProviders([...preferences.providers, nextProvider]);
    setOpenProviderId(nextProvider.id);
  };
  const removeProvider = (provider: AiProviderConfig) => {
    if (preferences.providers.length <= 1) return;
    const nextProviders = preferences.providers.filter((candidate) => candidate.id !== provider.id);
    const fallback = nextProviders[0];
    const isRemovingActiveProvider = provider.id === preferences.selectedProviderId;
    updateProviders(
      nextProviders,
      isRemovingActiveProvider ? fallback.id : preferences.selectedProviderId,
      isRemovingActiveProvider ? fallback.models[0].id : preferences.selectedModelId,
      isRemovingActiveProvider ? fallback.models[0].effortLevels[0]?.id ?? "" : preferences.selectedEffortId,
    );
    setOpenProviderId(null);
  };

  if (openProvider) {
    return (
      <AiProviderEditor
        provider={openProvider}
        preferences={preferences}
        isActive={openProvider.id === preferences.selectedProviderId}
        canRemove={preferences.providers.length > 1}
        onBack={() => setOpenProviderId(null)}
        onActivate={() => activateProvider(openProvider)}
        onRemove={() => removeProvider(openProvider)}
        updateProviders={updateProviders}
        t={t}
      />
    );
  }

  return (
    <SettingsPanel title={t("settings.providers.title")} description={t("settings.providers.description")}>
      <div className="provider-create-row">
        <label className="settings-select-control provider-template-select">
          <span className="provider-template-label">{t("settings.providers.template")}</span>
          <select value={providerPresetId} onChange={(event) => setProviderPresetId(event.currentTarget.value as AiProviderPresetId)}>
            {AI_PROVIDER_PRESETS.map((preset) => <option key={preset.id} value={preset.id}>{preset.name}</option>)}
          </select>
          <ChevronDown size={14} />
        </label>
        <button type="button" className="provider-add-button" onClick={addProvider}><Plus size={15} /> {t("settings.providers.add")}</button>
      </div>
      <div className="provider-grid">
        {preferences.providers.map((provider) => {
          const isActive = provider.id === preferences.selectedProviderId;
          return (
            <button
              type="button"
              className="provider-tile"
              key={provider.id}
              data-active={isActive}
              onClick={() => setOpenProviderId(provider.id)}
              aria-label={t("settings.providers.editProvider", { name: provider.name })}
            >
              <span className="provider-tile-avatar"><Cpu size={16} /></span>
              <span className="provider-tile-body">
                <span className="provider-tile-title">{provider.name}</span>
                <span className="provider-tile-meta">{t("settings.providers.modelsCount", { count: provider.models.length })}</span>
              </span>
              <span className="provider-tile-side">
                <span className="provider-status-pill" data-active={isActive}>{isActive ? t("settings.providers.active") : t("settings.providers.ready")}</span>
                <ChevronRight size={16} className="provider-tile-chevron" />
              </span>
            </button>
          );
        })}
      </div>
    </SettingsPanel>
  );
}

function AiProviderEditor({ canRemove, isActive, onActivate, onBack, onRemove, preferences, provider, t, updateProviders }: {
  canRemove: boolean;
  isActive: boolean;
  onActivate: () => void;
  onBack: () => void;
  onRemove: () => void;
  preferences: AiPreferences;
  provider: AiProviderConfig;
  t: TranslateFn;
  updateProviders: (providers: AiProviderConfig[], selectedProviderId?: string, selectedModelId?: string, selectedEffortId?: string) => void;
}) {
  const [editingModelId, setEditingModelId] = useState(provider.models[0]?.id ?? "");
  const editingModel = getAiModel(provider, editingModelId) ?? provider.models[0];
  const canRemoveModel = provider.models.length > 1;

  useEffect(() => {
    if (provider.models.some((model) => model.id === editingModelId)) return;
    setEditingModelId(provider.models[0].id);
  }, [editingModelId, provider]);

  const updateEditingProvider = (patch: Partial<AiProviderConfig>) => {
    updateProviders(preferences.providers.map((candidate) => candidate.id === provider.id ? { ...candidate, ...patch } : candidate));
  };
  const updateEditingModel = (patch: Partial<AiModelConfig>) => {
    updateEditingProvider({ models: provider.models.map((model) => model.id === editingModel.id ? { ...model, ...patch } : model) });
  };
  const updateEfforts = (effortLevels: AiEffortConfig[], selectedEffortId = preferences.selectedEffortId) => {
    const isEditingActiveModel = provider.id === preferences.selectedProviderId && editingModel.id === preferences.selectedModelId;
    updateProviders(
      preferences.providers.map((candidate) => candidate.id === provider.id
        ? { ...candidate, models: candidate.models.map((model) => model.id === editingModel.id ? { ...model, effortLevels } : model) }
        : candidate),
      preferences.selectedProviderId,
      preferences.selectedModelId,
      isEditingActiveModel ? selectedEffortId : preferences.selectedEffortId,
    );
  };
  const updateLocalProxyEndpoint = (patch: Partial<Pick<AiProviderConfig, "localHost" | "localPort" | "localPath">>) => {
    const next = { ...provider, ...patch };
    updateEditingProvider({ ...patch, baseUrl: buildLocalProxyBaseUrl(next.localHost, next.localPort, next.localPath) });
  };
  const addModel = () => {
    const nextModel = createAiModelConfig(provider.models);
    setEditingModelId(nextModel.id);
    updateProviders(preferences.providers.map((candidate) => candidate.id === provider.id ? { ...candidate, models: [...candidate.models, nextModel] } : candidate));
  };
  const removeModel = () => {
    if (!canRemoveModel) return;
    const nextModels = provider.models.filter((model) => model.id !== editingModel.id);
    const nextModel = nextModels[0];
    const isRemovingActiveModel = provider.id === preferences.selectedProviderId && editingModel.id === preferences.selectedModelId;
    setEditingModelId(nextModel.id);
    updateProviders(
      preferences.providers.map((candidate) => candidate.id === provider.id ? { ...candidate, models: nextModels } : candidate),
      isRemovingActiveModel ? provider.id : preferences.selectedProviderId,
      isRemovingActiveModel ? nextModel.id : preferences.selectedModelId,
      isRemovingActiveModel ? nextModel.effortLevels[0]?.id ?? "" : preferences.selectedEffortId,
    );
  };

  return (
    <div className="provider-detail">
      <div className="provider-detail-head">
        <button type="button" className="provider-back" onClick={onBack} aria-label={t("settings.providers.back")} title={t("settings.providers.back")}>
          <ChevronLeft size={16} />
        </button>
        <span className="provider-detail-avatar"><Cpu size={18} /></span>
        <div className="provider-detail-titles">
          <h3>{provider.name || t("settings.providers.untitled")}</h3>
          <p>{providerPresetDescription(provider.providerType, t)}</p>
        </div>
        <div className="provider-detail-actions">
          <button type="button" className="provider-activate-button" data-active={isActive} disabled={isActive} onClick={onActivate}>
            <Check size={14} /> {isActive ? t("settings.providers.active") : t("settings.providers.setActive")}
          </button>
          <button type="button" className="icon-action danger-button" disabled={!canRemove} onClick={onRemove} aria-label={t("common.remove")} title={t("common.remove")}>
            <Trash2 size={15} />
          </button>
        </div>
      </div>

      <SettingsPanel title={t("settings.providers.connection.title")}>
        <SettingsGrid>
          <TextSetting label={t("settings.providers.providerName.label")} value={provider.name} onChange={(name) => updateEditingProvider({ name })} />
          <SelectSetting<AiProviderProtocol> label={t("settings.providers.protocol.label")} value={provider.protocol} options={[
            { label: t("settings.providers.protocol.openaiCompatible"), value: "openai-compatible" },
            { label: t("settings.providers.protocol.anthropic"), value: "anthropic" },
            { label: t("settings.providers.protocol.google"), value: "google" },
            { label: t("settings.providers.protocol.azureOpenai"), value: "azure-openai" },
            { label: t("settings.providers.protocol.localProxy"), value: "local-proxy" },
          ]} onChange={(protocol) => {
            if (protocol === "local-proxy") {
              const localHost = provider.localHost || "127.0.0.1";
              const localPort = provider.localPort || "8080";
              const localPath = provider.localPath || "/v1";
              updateEditingProvider({ protocol, localHost, localPort, localPath, baseUrl: buildLocalProxyBaseUrl(localHost, localPort, localPath) });
              return;
            }
            updateEditingProvider({ protocol });
          }} />
          {provider.protocol === "local-proxy" ? (
            <>
              <TextSetting label={t("settings.providers.localIp.label")} value={provider.localHost} placeholder="127.0.0.1" onChange={(localHost) => updateLocalProxyEndpoint({ localHost })} />
              <TextSetting label={t("settings.providers.port.label")} value={provider.localPort} placeholder="8080" onChange={(localPort) => updateLocalProxyEndpoint({ localPort })} />
              <TextSetting label={t("settings.providers.apiPath.label")} value={provider.localPath} placeholder="/v1" onChange={(localPath) => updateLocalProxyEndpoint({ localPath })} />
              <TextSetting label={t("settings.providers.resolvedUrl.label")} value={provider.baseUrl} onChange={() => undefined} readOnly wide />
            </>
          ) : (
            <TextSetting label={t("settings.providers.baseUrl.label")} value={provider.baseUrl} onChange={(baseUrl) => updateEditingProvider({ baseUrl })} wide />
          )}
          <TextSetting label={t("settings.providers.apiKey.label")} value={provider.apiKey} onChange={(apiKey) => updateEditingProvider({ apiKey })} password wide />
        </SettingsGrid>
      </SettingsPanel>

      <SettingsPanel title={t("settings.providers.models.title")} description={t("settings.providers.models.description")}>
        <div className="model-toolbar">
          <div className="model-tabs">
            {provider.models.map((model) => (
              <button key={model.id} type="button" data-active={model.id === editingModel.id} onClick={() => setEditingModelId(model.id)}>{model.name}</button>
            ))}
          </div>
          <div>
            <button type="button" onClick={addModel}><Plus size={14} /> {t("settings.providers.model")}</button>
            <button type="button" disabled={!canRemoveModel} onClick={removeModel}><Trash2 size={14} /> {t("settings.providers.model")}</button>
          </div>
        </div>
        <SettingsGrid>
          <TextSetting label={t("settings.providers.modelName.label")} value={editingModel.name} onChange={(name) => updateEditingModel({ name })} />
          <TextSetting label={t("settings.providers.modelAlias.label")} value={editingModel.alias} onChange={(alias) => updateEditingModel({ alias })} />
        </SettingsGrid>
        <div className="effort-editor">
          <div>
            <strong>{t("settings.providers.thinkingEffort")}</strong>
            <button type="button" onClick={() => {
              const nextEffort = createAiEffortConfig(editingModel.effortLevels);
              updateEfforts([...editingModel.effortLevels, nextEffort], nextEffort.id);
            }}><Plus size={14} /> {t("settings.providers.addEffort")}</button>
          </div>
          {editingModel.effortLevels.length === 0 ? <p>{t("settings.providers.noEffortSelector")}</p> : editingModel.effortLevels.map((effort) => (
            <div className="effort-row" key={effort.id}>
              <input value={effort.label} aria-label={t("settings.providers.effortLabelAria", { id: effort.id })} onChange={(event) => {
                updateEfforts(editingModel.effortLevels.map((candidate) => candidate.id === effort.id ? { ...candidate, label: event.currentTarget.value } : candidate), preferences.selectedEffortId);
              }} />
              <button type="button" onClick={() => {
                const nextEfforts = editingModel.effortLevels.filter((candidate) => candidate.id !== effort.id);
                updateEfforts(nextEfforts, nextEfforts[0]?.id ?? "");
              }}><Trash2 size={14} /></button>
            </div>
          ))}
        </div>
      </SettingsPanel>
    </div>
  );
}

function AiIndexingSection({ aiIndex, onChange, preferences, t }: { aiIndex: ReturnType<typeof useLuxStore.getState>["aiIndex"]; onChange: (patch: Partial<AiPreferences>) => void; preferences: AiPreferences; t: TranslateFn }) {
  const statusLabel = aiIndex.status === "ready"
    ? t("settings.indexing.status.ready")
    : aiIndex.status === "indexing"
      ? t("settings.indexing.status.indexing")
      : aiIndex.status === "disabled"
        ? t("settings.indexing.status.disabled")
        : t("settings.indexing.status.waiting");
  return (
    <div className="settings-section-stack">
      <section className="index-status-card">
        <div>
          <Database size={18} />
          <div>
            <strong>{statusLabel}</strong>
            <span>{t("settings.indexing.filesIndexed", { indexed: aiIndex.indexedFiles, total: aiIndex.totalFiles })}</span>
          </div>
        </div>
        <div className="index-progress"><span style={{ width: `${aiIndex.progress}%` }} /></div>
      </section>
      <SettingsPanel title={t("settings.indexing.title")} description={t("settings.indexing.description")}>
        <SettingsGrid>
          <ToggleSetting label={t("settings.indexing.projectIndexing.label")} detail={t("settings.indexing.projectIndexing.detail")} checked={preferences.projectIndexingEnabled} onChange={(projectIndexingEnabled) => onChange({ projectIndexingEnabled })} />
          <ToggleSetting label={t("settings.indexing.realtime.label")} detail={t("settings.indexing.realtime.detail")} checked={preferences.realtimeIndexing} onChange={(realtimeIndexing) => onChange({ realtimeIndexing })} />
          <ToggleSetting label={t("settings.indexing.imageMetadata.label")} detail={t("settings.indexing.imageMetadata.detail")} checked={preferences.includeImages} onChange={(includeImages) => onChange({ includeImages })} />
          <NumberSetting label={t("settings.indexing.maxFiles.label")} detail={t("settings.indexing.maxFiles.detail")} value={preferences.maxIndexedFiles} min={500} max={20000} step={500} onChange={(maxIndexedFiles) => onChange({ maxIndexedFiles })} />
        </SettingsGrid>
      </SettingsPanel>
    </div>
  );
}

function SettingsPanel({ children, description, title }: { children: ReactNode; description?: string; title?: string }) {
  return (
    <section className="settings-panel">
      {(title || description) && (
        <div className="settings-panel-head">
          {title && <h3>{title}</h3>}
          {description && <p>{description}</p>}
        </div>
      )}
      {children}
    </section>
  );
}

function SettingsGrid({ children }: { children: ReactNode }) {
  return <div className="settings-control-grid">{children}</div>;
}

function NumberSetting({ detail, label, max, min, onChange, step = 1, value }: { detail?: string; label: string; max: number; min: number; onChange: (value: number) => void; step?: number; value: number }) {
  return (
    <SettingField detail={detail ?? `${min}-${max}`} label={label}>
      <div className="settings-stepper">
        <button type="button" aria-label={`Decrease ${label}`} disabled={value <= min} onClick={() => onChange(value - step)}><Minus size={13} /></button>
        <input aria-label={label} type="number" min={min} max={max} value={value} onChange={(event) => onChange(Number(event.target.value))} />
        <button type="button" aria-label={`Increase ${label}`} disabled={value >= max} onClick={() => onChange(value + step)}><Plus size={13} /></button>
      </div>
    </SettingField>
  );
}

function SelectSetting<T extends string>({ detail, label, onChange, options, value }: { detail?: string; label: string; onChange: (value: T) => void; options: Array<{ label: string; value: T }>; value: T }) {
  return (
    <SettingField detail={detail} label={label}>
      <label className="settings-select-control">
        <select aria-label={label} value={value} onChange={(event) => onChange(event.currentTarget.value as T)}>
          {options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
        </select>
        <ChevronDown size={14} />
      </label>
    </SettingField>
  );
}

function TextSetting({ detail, label, onChange, password = false, placeholder, readOnly = false, value, wide = false }: { detail?: string; label: string; onChange: (value: string) => void; password?: boolean; placeholder?: string; readOnly?: boolean; value: string; wide?: boolean }) {
  return (
    <SettingField detail={detail} label={label} wide={wide}>
      <input className="settings-input-control" aria-label={label} type={password ? "password" : "text"} value={value} placeholder={placeholder} readOnly={readOnly} spellCheck={false} onChange={(event) => onChange(event.currentTarget.value)} />
    </SettingField>
  );
}

function SegmentedSetting<T extends string>({ detail, label, onChange, options, value }: { detail?: string; label: string; onChange: (value: T) => void; options: Array<{ label: string; value: T }>; value: T }) {
  return (
    <SettingField detail={detail} label={label}>
      <div className="settings-segmented" role="radiogroup" aria-label={label}>
        {options.map((option) => (
          <button key={option.value} type="button" role="radio" aria-checked={option.value === value} data-active={option.value === value} onClick={() => onChange(option.value)}>{option.label}</button>
        ))}
      </div>
    </SettingField>
  );
}

function ToggleSetting({ checked, detail, label, onChange }: { checked: boolean; detail?: string; label: string; onChange: (checked: boolean) => void }) {
  return (
    <label className="settings-field settings-toggle-field">
      <span className="settings-field-copy">
        <strong>{label}</strong>
        {detail && <small>{detail}</small>}
      </span>
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
      <span className="settings-switch" aria-hidden="true" />
    </label>
  );
}

function SettingField({ children, detail, label, wide = false }: { children: ReactNode; detail?: string; label: string; wide?: boolean }) {
  return (
    <label className="settings-field" data-wide={wide}>
      <span className="settings-field-copy">
        <strong>{label}</strong>
        {detail && <small>{detail}</small>}
      </span>
      {children}
    </label>
  );
}

function SaveIndicator({ state, t }: { state: SaveState; t: TranslateFn }) {
  if (state === "idle") return <span className="settings-save-state">{t("settings.save.userSettings")}</span>;
  if (state === "saving") return <span className="settings-save-state">{t("settings.save.saving")}</span>;
  if (state === "error") return <span className="settings-save-state" data-tone="error">{t("settings.save.failed")}</span>;
  return <span className="settings-save-state" data-tone="saved"><Check size={12} /> {t("settings.save.saved")}</span>;
}

function providerPresetDescription(providerType: string, t: TranslateFn) {
  const descriptionKey = PROVIDER_PRESET_DESCRIPTION_KEYS[providerType];
  return descriptionKey ? t(descriptionKey) : t("settings.providers.customProvider");
}

function sectionMatchesQuery(section: SettingsSection, query: string, t: TranslateFn) {
  return [t(section.titleKey), t(section.descriptionKey), ...section.keywords].some((value) => value.toLowerCase().includes(query));
}

function resetSection(sectionId: SettingsSectionId, resetEditor: (preferences: EditorPreferences) => void, resetAi: (preferences: AiPreferences) => void) {
  if (sectionId === "editor") resetEditor(defaultEditorPreferences);
  else if (sectionId === "ai") resetAi(defaultAiPreferences);
}
