import * as Dialog from "@radix-ui/react-dialog";
import { ArrowUpCircle, BarChart3, Bot, Check, ChevronDown, ChevronLeft, ChevronRight, Code2, Cpu, Database, FileText, Globe, Languages, Loader2, Plus, RefreshCw, RotateCcw, Search, Settings, Share2, Trash2, X } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { NumberSetting, SaveIndicator, SegmentedSetting, SelectSetting, SettingsGrid, SettingsPanel, TextareaSetting, TextSetting, ToggleSetting, ToolRoundLimitSetting, type SaveState } from "./settings/SettingsControls";
import {
  AI_PREFERENCES_KEY,
  AI_PROVIDER_PRESETS,
  aiToolRoundLimitMax,
  aiToolRoundLimitMin,
  buildLocalProxyBaseUrl,
  createAiEffortConfig,
  createAiModelConfig,
  createAiProviderConfig,
  defaultLimitedAiToolRoundLimit,
  defaultAiPreferences,
  getAiAgentProfile,
  getAiModel,
  getAiProjectInstructions,
  getAiProvider,
  isDefaultAiAgentProfile,
  AI_AGENT_MODE_ORDER,
  mergeAiPreferences,
  normalizeAiPreferences,
  workspaceInstructionsKey,
  type AiAgentMode,
  type AiEffortConfig,
  type AiModelConfig,
  type AiPreferences,
  type AiProviderConfig,
  type AiProviderPresetId,
  type AiProviderProtocol,
  type AiFileEditTrustMode,
  type AiToolApprovalMode,
  type AiVisionImageFormatPreference,
  type AiScanConcurrency,
} from "../lib/aiPreferences";
import { AI_VISION_IMAGE_FORMATS } from "../lib/aiVisionFormat";
import { aggregateUsageByProject, clearAiUsageLog, loadAiUsageLog, usageEntryTokensPerSecond, type AiUsageLogEntry } from "../lib/aiUsageLog";

const AI_SCAN_CONCURRENCY_OPTIONS: readonly AiScanConcurrency[] = ["auto", "all", "half"];
import { formatCompactTokens } from "../lib/aiChatContextUsage";
import { resolveModelContextTokens } from "../lib/aiModelContext";
import { fetchProviderModelConfigs, isFreeModelId, mergeRefreshedModels } from "../lib/aiProviderModels";
import {
  clearLspInstallError,
  ensureLspInstallSubscription,
  getLspInstallProgressSnapshot,
  onLspInstallFinished,
  subscribeLspInstallProgress,
  type LspInstallProgress,
} from "../lib/lspInstallStore";
import {
  clearRuntimeProvisionError,
  ensureRuntimeProvisionSubscription,
  getRuntimeProvisionProgressSnapshot,
  onRuntimeProvisionFinished,
  subscribeRuntimeProvisionProgress,
  type RuntimeProvisionProgress,
} from "../lib/runtimeProvisionStore";
import {
  applyCodeGraphStatus,
  clearCodeGraphError,
  ensureCodeGraphSubscription,
  getCodeGraphStateSnapshot,
  onCodeGraphBuildFinished,
  subscribeCodeGraphState,
} from "../lib/codeGraphStore";
import {
  defaultEditorPreferences,
  EDITOR_PREFERENCES_KEY,
  mergeEditorPreferences,
  normalizeEditorPreferences,
  type EditorPreferences,
  type RenderWhitespaceSetting,
  type WordWrapSetting,
} from "../lib/editorPreferences";
import { displayPath } from "../lib/fileTree";
import { LOCALES, UI_LOCALE_KEY, type Locale, type MessageKey } from "../lib/i18n";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
import { isRulesContextPath } from "../lib/aiRuntimeFileContext";
import { useLuxStore } from "../lib/store";
import { isTauriRuntime, luxCommands, type AgentBrowserStatusResponse, type AiProviderDiagnosticResponse, type LspCatalogEntry, type RuntimeCatalogEntry } from "../lib/tauri";
import { useUpdater } from "../lib/useUpdater";
import type { FsEntry, WorkspaceInfo } from "../lib/types";

const scope = "user" as const;

// AI configuration is split into focused sections so runtime, instructions,
// providers, and indexing do not compete in one mixed settings list.
type SettingsSectionId = "general" | "editor" | "lsp" | "ai-runtime" | "ai-browser" | "ai-instructions" | "ai-providers" | "ai-indexing" | "ai-usage";

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

const settingsNavGroups: Array<{ labelKey: MessageKey; sectionIds: SettingsSectionId[] }> = [
  { labelKey: "settings.group.workspace", sectionIds: ["general"] },
  { labelKey: "settings.group.editor", sectionIds: ["editor", "lsp"] },
  { labelKey: "settings.group.ai", sectionIds: ["ai-runtime", "ai-browser", "ai-instructions", "ai-providers", "ai-indexing", "ai-usage"] },
];

const settingsSections: SettingsSection[] = [
  {
    id: "general",
    titleKey: "settings.general.title",
    descriptionKey: "settings.general.description",
    icon: <Languages size={16} />,
    keywords: ["language", "locale", "russian", "english", "язык", "general", "язык", "общие"],
  },
  {
    id: "editor",
    titleKey: "settings.group.editor",
    descriptionKey: "settings.editor.description",
    icon: <Code2 size={16} />,
    keywords: ["font", "line", "tab", "whitespace", "unicode", "minimap", "word wrap", "mouse", "zoom", "smooth", "ligatures", "appearance", "behavior", "редактор", "шрифт"],
  },
  {
    id: "lsp",
    titleKey: "settings.lsp.title",
    descriptionKey: "settings.lsp.description",
    icon: <Cpu size={16} />,
    keywords: ["lsp", "language server", "rust-analyzer", "gopls", "ty", "pyright", "typescript", "clangd", "intellisense", "completion", "hover", "языковой сервер"],
  },
  {
    id: "ai-runtime",
    titleKey: "settings.aiRuntime.title",
    descriptionKey: "settings.aiRuntime.description",
    icon: <Bot size={16} />,
    keywords: ["ai", "agent", "mode", "model", "effort", "reasoning", "tools", "tool rounds", "runtime", "compact", "context"],
  },
  {
    id: "ai-browser",
    titleKey: "settings.agentBrowser.nav.title",
    descriptionKey: "settings.agentBrowser.nav.description",
    icon: <Globe size={16} />,
    keywords: ["browser", "agent-browser", "chromium", "chrome", "automation", "stream", "preview", "браузер"],
  },
  {
    id: "ai-instructions",
    titleKey: "settings.instructions.title",
    descriptionKey: "settings.instructions.description",
    icon: <FileText size={16} />,
    keywords: ["ai", "instructions", "system", "prompt", "profile", "behavior", "agent", "plan", "ask"],
  },
  {
    id: "ai-providers",
    titleKey: "settings.providers.title",
    descriptionKey: "settings.providers.description",
    icon: <Cpu size={16} />,
    keywords: ["ai", "provider", "providers", "model", "models", "openai", "anthropic", "openrouter", "gemini", "local", "proxy", "api key", "base url"],
  },
  {
    id: "ai-indexing",
    titleKey: "settings.indexing.title",
    descriptionKey: "settings.indexing.description",
    icon: <Database size={16} />,
    keywords: ["ai", "index", "indexing", "files", "images", "metadata", "context", "workspace"],
  },
  {
    id: "ai-usage",
    titleKey: "settings.usage.title",
    descriptionKey: "settings.usage.description",
    icon: <BarChart3 size={16} />,
    keywords: ["ai", "usage", "history", "tokens", "cost", "spend", "speed", "requests", "стоимость", "история", "токены"],
  },
];

const sectionById = new Map(settingsSections.map((section) => [section.id, section]));

export function SettingsDialog() {
  const { t } = useTranslation();
  const open = useLuxStore((state) => state.settingsOpen);
  const setOpen = useLuxStore((state) => state.setSettingsOpen);
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const aiIndex = useLuxStore((state) => state.aiIndex);
  const fileEntries = useLuxStore((state) => state.fileEntries);
  const setAiPreferences = useLuxStore((state) => state.setAiPreferences);
  const updateAiPreferences = useLuxStore((state) => state.updateAiPreferences);
  const editorPreferences = useLuxStore((state) => state.editorPreferences);
  const setEditorPreferences = useLuxStore((state) => state.setEditorPreferences);
  const updateEditorPreferences = useLuxStore((state) => state.updateEditorPreferences);
  const locale = useLuxStore((state) => state.locale);
  const setLocale = useLuxStore((state) => state.setLocale);
  const workspace = useLuxStore((state) => state.workspace);
  const settingsInitialSection = useLuxStore((state) => state.settingsInitialSection);
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
    if (!open || !settingsInitialSection) return;
    if (sectionById.has(settingsInitialSection as SettingsSectionId)) {
      setActiveSectionId(settingsInitialSection as SettingsSectionId);
    }
  }, [open, settingsInitialSection]);

  useEffect(() => {
    if (!open) return;

    let cancelled = false;
    void luxCommands.settingsGet(scope, AI_PREFERENCES_KEY).then((setting) => {
      // preserveText: saved user prefs (custom prompt + instructions) must load
      // verbatim, not be run through display normalization that trims/replaces them.
      if (!cancelled && setting) setAiPreferences(normalizeAiPreferences(setting.value, { preserveText: true }));
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
      // Re-apply the scan/search CPU budget immediately (idempotent atomic set)
      // so a changed setting takes effect without an app restart.
      void luxCommands.setScanConcurrency(nextPreferences.scanConcurrency).catch(() => undefined);
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
                {activeSectionId !== "general" && activeSectionId !== "ai-instructions" && activeSectionId !== "ai-usage" && (
                  <button className="settings-reset-button" type="button" onClick={() => resetSection(activeSectionId, persistEditorPreferences, persistAiPreferences, aiPreferences)}>
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
                {activeSectionId === "lsp" && <LanguageServersSection preferences={aiPreferences} onChange={updateAiPreference} t={t} />}
                {activeSectionId === "ai-runtime" && (
                  <AiActiveCard preferences={aiPreferences} onChange={updateAiPreference} t={t} />
                )}
                {activeSectionId === "ai-browser" && (
                  <AgentBrowserSection preferences={aiPreferences} onChange={updateAiPreference} t={t} />
                )}
                {activeSectionId === "ai-instructions" && <AiInstructionsSection fileEntries={fileEntries} preferences={aiPreferences} workspace={workspace} onChange={updateAiPreference} t={t} />}
                {activeSectionId === "ai-providers" && <AiProvidersSection preferences={aiPreferences} onChange={updateAiPreference} t={t} />}
                {activeSectionId === "ai-indexing" && <AiIndexingSection aiIndex={aiIndex} preferences={aiPreferences} onChange={updateAiPreference} t={t} />}
                {activeSectionId === "ai-usage" && <AiUsageSection workspace={workspace} t={t} />}
              </div>
            </main>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function SettingsSectionNav({ activeSectionId, onSelect, sections, t }: { activeSectionId: SettingsSectionId; onSelect: (sectionId: SettingsSectionId) => void; sections: SettingsSection[]; t: TranslateFn }) {
  const visibleIds = new Set(sections.map((section) => section.id));
  return (
    <div className="settings-nav-groups">
      {settingsNavGroups.map((group) => {
        const groupSections = group.sectionIds
          .map((id) => sectionById.get(id))
          .filter((section): section is SettingsSection => Boolean(section && visibleIds.has(section.id)));
        if (groupSections.length === 0) return null;
        return (
          <section className="settings-nav-group" key={group.labelKey}>
            <h3 className="settings-nav-group-label">{t(group.labelKey)}</h3>
            {groupSections.map((section) => (
              <button
                className="settings-nav-item"
                type="button"
                key={section.id}
                data-active={section.id === activeSectionId}
                title={t(section.descriptionKey)}
                onClick={() => onSelect(section.id)}
              >
                <span className="settings-nav-icon">{section.icon}</span>
                <span>
                  <strong>{t(section.titleKey)}</strong>
                  <small>{t(section.descriptionKey)}</small>
                </span>
              </button>
            ))}
          </section>
        );
      })}
    </div>
  );
}

function GeneralSection({ locale, onChangeLocale, t }: { locale: Locale; onChangeLocale: (locale: Locale) => void; t: TranslateFn }) {
  return (
    <>
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
      <UpdatesSection t={t} />
    </>
  );
}

function UpdatesSection({ t }: { t: TranslateFn }) {
  // Shared singleton updater: the same state the corner UpdateNotice renders, so
  // a check or install started here is reflected everywhere (and vice versa).
  const { state, check, install } = useUpdater();

  const updateAvailable = state.status === "available";
  const downloading = state.status === "downloading";
  const relaunching = state.status === "relaunching";
  const checking = state.status === "checking";
  const busy = checking || downloading || relaunching;
  const percent = state.progress === null ? null : Math.round(state.progress * 100);

  const statusLine = (() => {
    switch (state.status) {
      case "checking":
        return t("update.checking");
      case "available":
        return t("update.available.body", { version: state.availableVersion ?? "" });
      case "upToDate":
        return t("update.upToDate");
      case "downloading":
        return percent === null
          ? t("update.downloading.preparing")
          : t("update.downloading.body", { version: state.availableVersion ?? "", percent });
      case "relaunching":
        return t("update.relaunching.body");
      case "error":
        return state.error ?? t("update.error.title");
      default:
        return state.lastCheckedAt === null
          ? t("update.settings.lastCheckedNever")
          : t("update.settings.lastChecked", { time: new Date(state.lastCheckedAt).toLocaleTimeString() });
    }
  })();

  return (
    <SettingsPanel title={t("update.settings.title")} description={t("update.settings.detail")}>
      <div className="settings-update-row" data-status={state.status}>
        <div className="settings-update-info">
          <span className="settings-update-version">
            {state.currentVersion ? t("update.settings.currentVersion", { version: state.currentVersion }) : t("update.settings.title")}
          </span>
          <span className="settings-update-status">{statusLine}</span>
        </div>
        <div className="settings-update-actions">
          {/* When an update is found, install right here — no waiting for the
              corner toast. Mid-download/relaunch the button reflects progress. */}
          {(updateAvailable || downloading || relaunching) && (
            <button
              type="button"
              className="settings-update-install"
              disabled={downloading || relaunching || !isTauriRuntime()}
              onClick={() => void install()}
            >
              {relaunching ? (
                <>
                  <Loader2 size={14} className="settings-update-spin" />
                  {t("update.relaunching.title")}
                </>
              ) : downloading ? (
                <>
                  <Loader2 size={14} className="settings-update-spin" />
                  {percent === null ? t("update.downloading.preparing") : `${percent}%`}
                </>
              ) : (
                <>
                  <ArrowUpCircle size={14} />
                  {t("update.settings.updateNow")}
                </>
              )}
            </button>
          )}
          <button
            type="button"
            className="settings-update-check"
            disabled={busy || !isTauriRuntime()}
            onClick={() => void check()}
          >
            {t("update.settings.check")}
          </button>
        </div>
      </div>
      {downloading && (
        <div
          className="settings-update-bar-track"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent ?? undefined}
        >
          <div
            className="settings-update-bar"
            data-indeterminate={percent === null ? "true" : undefined}
            style={percent === null ? undefined : { width: `${percent}%` }}
          />
        </div>
      )}
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

function LanguageServersSection({ onChange, preferences, t }: { onChange: (patch: Partial<AiPreferences>) => void; preferences: AiPreferences; t: TranslateFn }) {
  const [catalog, setCatalog] = useState<LspCatalogEntry[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [progress, setProgress] = useState<Record<string, LspInstallProgress>>(getLspInstallProgressSnapshot);

  const refreshCatalog = useCallback(() => {
    luxCommands.lspServerCatalog()
      .then((entries) => { setCatalog(entries); setLoadError(null); })
      .catch((error) => setLoadError(error instanceof Error ? error.message : String(error)));
  }, []);

  useEffect(() => {
    ensureLspInstallSubscription();
    refreshCatalog();
    const stopProgress = subscribeLspInstallProgress(() => setProgress({ ...getLspInstallProgressSnapshot() }));
    // Re-pull the catalog whenever an install finishes so the badge flips to Installed.
    const stopFinish = onLspInstallFinished(() => refreshCatalog());
    return () => { stopProgress(); stopFinish(); };
  }, [refreshCatalog]);

  const installServer = (languageId: string) => {
    clearLspInstallError(languageId);
    setProgress({ ...getLspInstallProgressSnapshot(), [languageId]: { status: "installing", percent: 0, step: "Starting" } });
    void luxCommands.lspInstallServer(languageId).catch(() => undefined);
  };

  const installedCount = catalog?.filter((entry) => entry.installed).length ?? 0;

  return (
    <SettingsPanel title={t("settings.lsp.title")} description={t("settings.lsp.description")}>
      <SettingsGrid>
        <ToggleSetting
          label={t("settings.lsp.autoInstall.label")}
          detail={t("settings.lsp.autoInstall.detail")}
          checked={preferences.lspAutoInstall}
          onChange={(lspAutoInstall) => onChange({ lspAutoInstall })}
        />
        <ToggleSetting
          label={t("settings.runtimes.autoProvision.label")}
          detail={t("settings.runtimes.autoProvision.detail")}
          checked={preferences.runtimeAutoProvision}
          onChange={(runtimeAutoProvision) => onChange({ runtimeAutoProvision })}
        />
      </SettingsGrid>

      <RuntimesPanel t={t} />

      <section className="lsp-servers-head">
        <strong>{t("settings.lsp.listTitle")}</strong>
        <div className="lsp-servers-head-actions">
          {catalog && <span className="lsp-servers-count">{t("settings.lsp.installedCount", { installed: installedCount, total: catalog.length })}</span>}
          <button type="button" onClick={refreshCatalog} title={t("settings.lsp.refresh")} aria-label={t("settings.lsp.refresh")}>
            <RefreshCw size={14} /> {t("settings.lsp.refresh")}
          </button>
        </div>
      </section>

      {loadError && <p className="lsp-servers-error" role="alert">{loadError}</p>}

      {catalog === null ? (
        <p className="lsp-servers-loading">{t("settings.lsp.loading")}</p>
      ) : (
        <ul className="lsp-servers-list">
          {catalog.map((entry) => {
            const live = progress[entry.languageId] ?? null;
            return (
              <LanguageServerRow
                key={entry.languageId}
                entry={entry}
                progress={live}
                onInstall={() => installServer(entry.languageId)}
                t={t}
              />
            );
          })}
        </ul>
      )}
    </SettingsPanel>
  );
}

function LanguageServerRow({ entry, progress, onInstall, t }: {
  entry: LspCatalogEntry;
  progress: LspInstallProgress | null;
  onInstall: () => void;
  t: TranslateFn;
}) {
  const installing = progress?.status === "installing";
  const errored = progress?.status === "error";
  // Status precedence: installing > error > installed(managed/PATH) > missing.
  const state = installing ? "installing" : errored ? "error" : entry.installed ? (entry.managed ? "managed" : "path") : "missing";
  const isManual = entry.installMethod === "manual";

  const statusLabel = installing
    ? t("settings.lsp.status.installing", { percent: progress?.percent ?? 0 })
    : errored
      ? t("settings.lsp.status.error")
      : entry.installed
        ? entry.managed ? t("settings.lsp.status.installed") : t("settings.lsp.status.onPath")
        : isManual ? t("settings.lsp.status.manual") : t("settings.lsp.status.missing");

  return (
    <li className="lsp-server-row" data-state={state}>
      <div className="lsp-server-row-main">
        <div className="lsp-server-row-title">
          <span className="lsp-server-dot" data-state={state} aria-hidden="true" />
          <strong>{entry.name}</strong>
          <code className="lsp-server-ext">{entry.extensions.slice(0, 4).map((e) => `.${e}`).join(" ")}</code>
        </div>
        <span className="lsp-server-status" data-state={state}>{statusLabel}</span>
        {installing && (
          <div className="lsp-server-progress" role="progressbar" aria-valuenow={progress?.percent ?? 0} aria-valuemin={0} aria-valuemax={100}>
            <div className="lsp-server-progress-fill" style={{ width: `${progress?.percent ?? 0}%` }} />
            <span className="lsp-server-progress-step">{progress?.step}</span>
          </div>
        )}
        {errored && <p className="lsp-server-row-error" title={progress?.error}>{progress?.error}</p>}
        {isManual && !entry.installed && !installing && <p className="lsp-server-row-hint">{entry.manualHint}</p>}
      </div>
      <div className="lsp-server-row-action">
        {isManual ? (
          <span className="lsp-server-manual-tag">{t("settings.lsp.manualTag")}</span>
        ) : (
          <button type="button" className="lsp-server-install" onClick={onInstall} disabled={installing}>
            {installing
              ? t("settings.lsp.installing")
              : entry.installed
                ? t("settings.lsp.reinstall")
                : t("settings.lsp.install")}
          </button>
        )}
      </div>
    </li>
  );
}

/**
 * Managed language runtimes (Node / Rust / Python). These are the host toolchains
 * the LSP installers need; the IDE can provision them into a self-contained dir so
 * a clean machine needs zero manual setup. Mirrors the server list visually.
 */
function RuntimesPanel({ t }: { t: TranslateFn }) {
  const [catalog, setCatalog] = useState<RuntimeCatalogEntry[] | null>(null);
  const [progress, setProgress] = useState<Record<string, RuntimeProvisionProgress>>(getRuntimeProvisionProgressSnapshot);

  const refreshCatalog = useCallback(() => {
    luxCommands.runtimeCatalog()
      .then((entries) => setCatalog(entries))
      .catch(() => setCatalog([]));
  }, []);

  useEffect(() => {
    ensureRuntimeProvisionSubscription();
    refreshCatalog();
    const stopProgress = subscribeRuntimeProvisionProgress(() => setProgress({ ...getRuntimeProvisionProgressSnapshot() }));
    const stopFinish = onRuntimeProvisionFinished(() => refreshCatalog());
    return () => { stopProgress(); stopFinish(); };
  }, [refreshCatalog]);

  const provision = (id: string) => {
    clearRuntimeProvisionError(id);
    setProgress({ ...getRuntimeProvisionProgressSnapshot(), [id]: { status: "installing", percent: 0, step: "Starting" } });
    void luxCommands.runtimeProvision(id).catch(() => undefined);
  };

  const installedCount = catalog?.filter((entry) => entry.installed).length ?? 0;

  return (
    <>
      <section className="lsp-servers-head">
        <strong>{t("settings.runtimes.title")}</strong>
        <div className="lsp-servers-head-actions">
          {catalog && <span className="lsp-servers-count">{t("settings.lsp.installedCount", { installed: installedCount, total: catalog.length })}</span>}
        </div>
      </section>
      <p className="lsp-servers-subtitle">{t("settings.runtimes.description")}</p>
      {catalog === null ? (
        <p className="lsp-servers-loading">{t("settings.lsp.loading")}</p>
      ) : (
        <ul className="lsp-servers-list">
          {catalog.map((entry) => (
            <RuntimeRow key={entry.id} entry={entry} progress={progress[entry.id] ?? null} onProvision={() => provision(entry.id)} t={t} />
          ))}
        </ul>
      )}
    </>
  );
}

function RuntimeRow({ entry, progress, onProvision, t }: {
  entry: RuntimeCatalogEntry;
  progress: RuntimeProvisionProgress | null;
  onProvision: () => void;
  t: TranslateFn;
}) {
  const installing = progress?.status === "installing";
  const errored = progress?.status === "error";
  const state = installing ? "installing" : errored ? "error" : entry.installed ? (entry.managed ? "managed" : "path") : "missing";

  const statusLabel = installing
    ? t("settings.lsp.status.installing", { percent: progress?.percent ?? 0 })
    : errored
      ? t("settings.lsp.status.error")
      : entry.installed
        ? entry.managed ? t("settings.runtimes.status.managed") : t("settings.runtimes.status.system")
        : entry.canAuto ? t("settings.lsp.status.missing") : t("settings.lsp.status.manual");

  return (
    <li className="lsp-server-row" data-state={state}>
      <div className="lsp-server-row-main">
        <div className="lsp-server-row-title">
          <span className="lsp-server-dot" data-state={state} aria-hidden="true" />
          <strong>{entry.name}</strong>
        </div>
        <span className="lsp-server-status" data-state={state}>{statusLabel}</span>
        {installing && (
          <div className="lsp-server-progress" role="progressbar" aria-valuenow={progress?.percent ?? 0} aria-valuemin={0} aria-valuemax={100}>
            <div className="lsp-server-progress-fill" style={{ width: `${progress?.percent ?? 0}%` }} />
            <span className="lsp-server-progress-step">{progress?.step}</span>
          </div>
        )}
        {errored && <p className="lsp-server-row-error" title={progress?.error}>{progress?.error}</p>}
        {!entry.canAuto && !entry.installed && !installing && <p className="lsp-server-row-hint">{entry.manualHint}</p>}
      </div>
      <div className="lsp-server-row-action">
        {entry.canAuto ? (
          <button type="button" className="lsp-server-install" onClick={onProvision} disabled={installing}>
            {installing
              ? t("settings.lsp.installing")
              : entry.installed
                ? t("settings.lsp.reinstall")
                : t("settings.runtimes.install")}
          </button>
        ) : (
          <span className="lsp-server-manual-tag">{t("settings.lsp.manualTag")}</span>
        )}
      </div>
    </li>
  );
}

// A single focused "active model" card: the thing a user changes often (model + effort +
// mode) shown at a glance, with the selected agent's behavior editable inline. Provider/model
// management itself lives in the Providers section, so nothing is configured in two places.
const AI_TOOL_APPROVAL_MODES: AiToolApprovalMode[] = ["default", "full-access"];
const AI_FILE_EDIT_TRUST_MODES: AiFileEditTrustMode[] = ["preview-before-apply", "apply-immediately"];

function AgentBrowserSection({ onChange, preferences, t }: { onChange: (patch: Partial<AiPreferences>) => void; preferences: AiPreferences; t: TranslateFn }) {
  const [status, setStatus] = useState<AgentBrowserStatusResponse | null>(null);
  const [checking, setChecking] = useState(false);

  const refreshStatus = useCallback(async (options: { full?: boolean } = {}) => {
    setChecking(true);
    try {
      const response = await luxCommands.agentBrowserStatus({
        commandPath: preferences.agentBrowserCommand.trim() || undefined,
        skipAutoUpdate: true,
        lightweight: options.full ? false : true,
      });
      if (response.updatePerformed) {
        void import("../lib/agentBrowserSkillsCache").then(({ invalidateAgentBrowserSkillsCache }) => invalidateAgentBrowserSkillsCache());
      }
      setStatus(response);
    } catch (error) {
      setStatus({
        available: false,
        commandPath: null,
        version: null,
        latestVersion: null,
        updatePerformed: false,
        updateDetail: null,
        detail: error instanceof Error ? error.message : String(error),
        sessions: [],
        doctor: null,
      });
    } finally {
      setChecking(false);
    }
  }, [preferences.agentBrowserCommand]);

  // Sync the real install/version state the moment the section opens, so the card
  // reflects what's actually installed instead of sitting on "unavailable" until the
  // user clicks Refresh. Lightweight: resolves CLI + version only, never launches Chromium.
  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  const diagnosticState = checking
    ? "checking"
    : status?.available
      ? "ok"
      : status
        ? "error"
        : "idle";
  const statusLabel = checking
    ? t("settings.agentBrowser.status.checking")
    : status?.available
      ? t("settings.agentBrowser.status.ready", { version: status.version ?? "agent-browser" })
      : status
        ? t("settings.agentBrowser.status.issue")
        : t("settings.agentBrowser.status.unavailable");

  return (
    <SettingsPanel title={t("settings.agentBrowser.title")} description={t("settings.agentBrowser.description")}>
      <section className="settings-banner agent-browser-status-card" data-state={diagnosticState}>
        <div className="settings-banner-main">
          <strong>{statusLabel}</strong>
          {status && <span>{t("settings.agentBrowser.status.detail", { detail: status.detail })}</span>}
          {status && status.sessions.length > 0 && (
            <span>{t("settings.agentBrowser.status.sessions", { count: status.sessions.length })}</span>
          )}
          <span className="agent-browser-install-hint">{t("settings.agentBrowser.install.hint")}</span>
        </div>
        <div className="settings-banner-actions">
          <button type="button" disabled={checking} onClick={() => void refreshStatus()}>
            {t("settings.agentBrowser.status.refresh")}
          </button>
          <button type="button" disabled={checking} onClick={() => void refreshStatus({ full: true })}>
            {t("settings.agentBrowser.status.fullCheck")}
          </button>
          <button
            type="button"
            disabled={checking}
            onClick={() => {
              setChecking(true);
              void luxCommands.agentBrowserInstall({
                commandPath: preferences.agentBrowserCommand.trim() || null,
                withDeps: false,
              }).then(() => {
                void import("../lib/agentBrowserSkillsCache").then(({ invalidateAgentBrowserSkillsCache }) => invalidateAgentBrowserSkillsCache());
              }).catch(() => undefined).finally(() => {
                // refreshStatus resets `checking` and surfaces success/error state,
                // so it covers both the resolved and rejected install paths. The
                // .catch clears the rejection (finally alone would re-throw it).
                void refreshStatus();
              });
            }}
          >
            {t("settings.agentBrowser.install.action")}
          </button>
        </div>
      </section>
      <SettingsGrid>
        <ToggleSetting
          label={t("settings.agentBrowser.enabled.label")}
          detail={t("settings.agentBrowser.enabled.detail")}
          checked={preferences.agentBrowserEnabled}
          onChange={(agentBrowserEnabled) => onChange({ agentBrowserEnabled })}
        />
        <ToggleSetting
          label={t("settings.agentBrowser.headed.label")}
          detail={t("settings.agentBrowser.headed.detail")}
          checked={preferences.agentBrowserHeaded}
          onChange={(agentBrowserHeaded) => onChange({ agentBrowserHeaded })}
        />
        <ToggleSetting
          label={t("settings.agentBrowser.autoStream.label")}
          detail={t("settings.agentBrowser.autoStream.detail")}
          checked={preferences.agentBrowserAutoStreamPreview}
          onChange={(agentBrowserAutoStreamPreview) => onChange({ agentBrowserAutoStreamPreview })}
        />
        <ToggleSetting
          label={t("settings.agentBrowser.persistSession.label")}
          detail={t("settings.agentBrowser.persistSession.detail")}
          checked={preferences.agentBrowserPersistSession}
          onChange={(agentBrowserPersistSession) => onChange({ agentBrowserPersistSession })}
        />
        <ToggleSetting
          label={t("settings.agentBrowser.contentBoundaries.label")}
          detail={t("settings.agentBrowser.contentBoundaries.detail")}
          checked={preferences.agentBrowserContentBoundaries}
          onChange={(agentBrowserContentBoundaries) => onChange({ agentBrowserContentBoundaries })}
        />
        <ToggleSetting
          label={t("settings.agentBrowser.allowFileAccess.label")}
          detail={t("settings.agentBrowser.allowFileAccess.detail")}
          checked={preferences.agentBrowserAllowFileAccess}
          onChange={(agentBrowserAllowFileAccess) => onChange({ agentBrowserAllowFileAccess })}
        />
        <ToggleSetting
          label={t("settings.agentBrowser.ignoreHttps.label")}
          detail={t("settings.agentBrowser.ignoreHttps.detail")}
          checked={preferences.agentBrowserIgnoreHttpsErrors}
          onChange={(agentBrowserIgnoreHttpsErrors) => onChange({ agentBrowserIgnoreHttpsErrors })}
        />
        <NumberSetting
          label={t("settings.agentBrowser.dashboardPort.label")}
          detail={t("settings.agentBrowser.dashboardPort.detail")}
          value={preferences.agentBrowserDashboardPort}
          min={1024}
          max={65_535}
          step={1}
          onChange={(agentBrowserDashboardPort) => onChange({ agentBrowserDashboardPort })}
        />
        <TextSetting
          label={t("settings.agentBrowser.command.label")}
          detail={t("settings.agentBrowser.command.detail")}
          value={preferences.agentBrowserCommand}
          placeholder={t("settings.agentBrowser.command.placeholder")}
          onChange={(agentBrowserCommand) => onChange({ agentBrowserCommand })}
          wide
        />
        <TextSetting
          label={t("settings.agentBrowser.allowedDomains.label")}
          detail={t("settings.agentBrowser.allowedDomains.detail")}
          value={preferences.agentBrowserAllowedDomains}
          placeholder={t("settings.agentBrowser.allowedDomains.placeholder")}
          onChange={(agentBrowserAllowedDomains) => onChange({ agentBrowserAllowedDomains })}
          wide
        />
        <NumberSetting
          label={t("settings.agentBrowser.maxOutput.label")}
          detail={t("settings.agentBrowser.maxOutput.detail")}
          value={preferences.agentBrowserMaxOutput}
          min={4_000}
          max={120_000}
          step={1_000}
          onChange={(agentBrowserMaxOutput) => onChange({ agentBrowserMaxOutput })}
        />
        <TextSetting
          label={t("settings.agentBrowser.profile.label")}
          detail={t("settings.agentBrowser.profile.detail")}
          value={preferences.agentBrowserProfile}
          placeholder={t("settings.agentBrowser.profile.placeholder")}
          onChange={(agentBrowserProfile) => onChange({ agentBrowserProfile })}
          wide
        />
        <TextSetting
          label={t("settings.agentBrowser.statePath.label")}
          detail={t("settings.agentBrowser.statePath.detail")}
          value={preferences.agentBrowserStatePath}
          placeholder={t("settings.agentBrowser.statePath.placeholder")}
          onChange={(agentBrowserStatePath) => onChange({ agentBrowserStatePath })}
          wide
        />
        <TextSetting
          label={t("settings.agentBrowser.provider.label")}
          detail={t("settings.agentBrowser.provider.detail")}
          value={preferences.agentBrowserProvider}
          placeholder={t("settings.agentBrowser.provider.placeholder")}
          onChange={(agentBrowserProvider) => onChange({ agentBrowserProvider })}
          wide
        />
        <TextSetting
          label={t("settings.agentBrowser.proxy.label")}
          detail={t("settings.agentBrowser.proxy.detail")}
          value={preferences.agentBrowserProxy}
          placeholder={t("settings.agentBrowser.proxy.placeholder")}
          onChange={(agentBrowserProxy) => onChange({ agentBrowserProxy })}
          wide
        />
      </SettingsGrid>
    </SettingsPanel>
  );
}

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
  return (
    <SettingsPanel title={t("settings.aiRuntime.activeRuntime.title")} description={t("settings.aiRuntime.activeRuntime.description")}>
      <SettingsGrid>
        <SelectSetting label={t("settings.aiRuntime.provider.label")} value={selectedProvider.id} options={preferences.providers.map((provider) => ({ label: provider.name, value: provider.id }))} onChange={selectProvider} />
        <SelectSetting label={t("settings.aiRuntime.model.label")} value={selectedModel.id} options={selectedProvider.models.map((model) => ({ label: model.name, value: model.id }))} onChange={selectModel} />
        {selectedModel.effortLevels.length > 0 && (
          <SegmentedSetting label={t("settings.aiRuntime.effort.label")} value={selectedEffort?.id ?? ""} options={selectedModel.effortLevels.map((effort) => ({ label: effort.label, value: effort.id }))} onChange={(selectedEffortId) => onChange({ selectedEffortId })} />
        )}
        <SegmentedSetting<AiAgentMode> label={t("settings.aiRuntime.mode.label")} value={selectedAgent.mode} options={AI_AGENT_MODE_ORDER.map((mode) => ({ label: t(`settings.aiRuntime.mode.${mode}` as MessageKey), value: mode }))} onChange={selectMode} />
        <SegmentedSetting<AiToolApprovalMode> label={t("settings.aiRuntime.toolApproval.label")} detail={t("settings.aiRuntime.toolApproval.detail")} value={preferences.toolApprovalMode} options={AI_TOOL_APPROVAL_MODES.map((mode) => ({ label: t(`settings.aiRuntime.toolApproval.${mode}` as MessageKey), value: mode }))} onChange={(toolApprovalMode) => onChange({ toolApprovalMode })} />
        <SegmentedSetting<AiFileEditTrustMode> label={t("settings.aiRuntime.fileEditTrust.label")} detail={t("settings.aiRuntime.fileEditTrust.detail")} value={preferences.fileEditTrustMode} options={AI_FILE_EDIT_TRUST_MODES.map((mode) => ({ label: t(`settings.aiRuntime.fileEditTrust.${mode}` as MessageKey), value: mode }))} onChange={(fileEditTrustMode) => onChange({ fileEditTrustMode })} />
        <ToolRoundLimitSetting label={t("settings.aiRuntime.toolRoundLimit.label")} detail={t("settings.aiRuntime.toolRoundLimit.detail")} value={preferences.toolRoundLimit} min={aiToolRoundLimitMin} max={aiToolRoundLimitMax} step={1} fallbackLimitedValue={defaultLimitedAiToolRoundLimit} unlimitedLabel={t("settings.aiRuntime.toolRoundLimit.unlimited")} limitedLabel={t("settings.aiRuntime.toolRoundLimit.limited")} onChange={(toolRoundLimit) => onChange({ toolRoundLimit })} />
        <NumberSetting
          label={t("settings.aiRuntime.maxParallelSubagents.label")}
          detail={t("settings.aiRuntime.maxParallelSubagents.detail")}
          value={preferences.maxParallelSubagents}
          min={1}
          max={16}
          step={1}
          onChange={(maxParallelSubagents) => onChange({ maxParallelSubagents })}
        />
        <ToggleSetting label={t("settings.aiRuntime.responseDuration.label")} detail={t("settings.aiRuntime.responseDuration.detail")} checked={preferences.showResponseDuration} onChange={(showResponseDuration) => onChange({ showResponseDuration })} />
        <ToggleSetting label={t("settings.aiRuntime.tokenEconomy.label")} detail={t("settings.aiRuntime.tokenEconomy.detail")} checked={preferences.tokenEconomyEnabled} onChange={(tokenEconomyEnabled) => onChange({ tokenEconomyEnabled })} />
        <ToggleSetting
          label={t("settings.aiRuntime.contextAutoCompact.label")}
          detail={t("settings.aiRuntime.contextAutoCompact.detail")}
          checked={preferences.contextAutoCompactEnabled}
          onChange={(contextAutoCompactEnabled) => onChange({ contextAutoCompactEnabled })}
        />
        <NumberSetting
          label={t("settings.aiRuntime.contextAutoCompactThreshold.label")}
          detail={t("settings.aiRuntime.contextAutoCompactThreshold.detail", { model: selectedModel.name })}
          value={Math.round(preferences.contextAutoCompactThreshold * 100)}
          min={50}
          max={95}
          step={5}
          onChange={(percent) => onChange({ contextAutoCompactThreshold: percent / 100 })}
        />
      </SettingsGrid>
      <SettingsGrid>
        <TextareaSetting
          label={t("settings.aiRuntime.permissionRules.label")}
          detail={t("settings.aiRuntime.permissionRules.detail")}
          placeholder={t("settings.aiRuntime.permissionRules.placeholder")}
          value={preferences.toolPermissionRules.join("\n")}
          rows={5}
          onChange={(value) => onChange({ toolPermissionRules: value.split("\n") })}
          wide
        />
      </SettingsGrid>
    </SettingsPanel>
  );
}

function AiInstructionsSection({ fileEntries, onChange, preferences, t, workspace }: { fileEntries: FsEntry[]; onChange: (patch: Partial<AiPreferences>) => void; preferences: AiPreferences; t: TranslateFn; workspace: WorkspaceInfo | null }) {
  const projectInstructions = getAiProjectInstructions(preferences, workspace?.root);
  const detectedInstructionFiles = useMemo(() => detectWorkspaceInstructionFiles(fileEntries, workspace), [fileEntries, workspace]);

  const updateProjectInstructions = (instructions: string) => {
    if (!workspace) return;
    const key = workspaceInstructionsKey(workspace.root);
    const projectInstructionsByWorkspace = { ...preferences.projectInstructionsByWorkspace };
    if (instructions.trim()) projectInstructionsByWorkspace[key] = instructions;
    else delete projectInstructionsByWorkspace[key];
    onChange({ projectInstructionsByWorkspace });
  };

  return (
    <div className="settings-section-stack instructions-section-stack">
      <SettingsPanel title={t("settings.instructions.customPrompt.title")} description={t("settings.instructions.customPrompt.description")}>
        <SettingsGrid>
          <ToggleSetting
            label={t("settings.instructions.customPrompt.toggle.label")}
            detail={t("settings.instructions.customPrompt.toggle.detail")}
            checked={preferences.customSystemPromptEnabled}
            onChange={(customSystemPromptEnabled) => onChange({ customSystemPromptEnabled })}
          />
        </SettingsGrid>
        {preferences.customSystemPromptEnabled && (
          <SettingsGrid>
            <TextareaSetting
              label={t("settings.instructions.customPrompt.body.label")}
              detail={t("settings.instructions.customPrompt.body.detail")}
              placeholder={t("settings.instructions.customPrompt.body.placeholder")}
              value={preferences.customSystemPrompt}
              rows={12}
              onChange={(customSystemPrompt) => onChange({ customSystemPrompt })}
              wide
            />
          </SettingsGrid>
        )}
      </SettingsPanel>

      <SettingsPanel title={t("settings.instructions.global.title")} description={t("settings.instructions.global.description")}>
        <SettingsGrid>
          <TextareaSetting
            label={t("settings.instructions.global.label")}
            detail={t("settings.instructions.global.detail")}
            placeholder={t("settings.instructions.global.placeholder")}
            value={preferences.globalInstructions}
            rows={8}
            onChange={(globalInstructions) => onChange({ globalInstructions })}
            wide
          />
        </SettingsGrid>
      </SettingsPanel>

      <SettingsPanel title={t("settings.instructions.project.title")} description={t("settings.instructions.project.description")}>
        {workspace ? (
          <>
            <div className="instruction-workspace-card">
              <span>{t("settings.instructions.project.currentWorkspace")}</span>
              <strong>{workspace.name}</strong>
              <small>{displayPath(workspace.root)}</small>
            </div>
            <SettingsGrid>
              <TextareaSetting
                label={t("settings.instructions.project.label")}
                detail={t("settings.instructions.project.detail")}
                placeholder={t("settings.instructions.project.placeholder")}
                value={projectInstructions}
                rows={9}
                onChange={updateProjectInstructions}
                wide
              />
            </SettingsGrid>
          </>
        ) : (
          <div className="settings-empty-note">{t("settings.instructions.project.noWorkspace")}</div>
        )}
      </SettingsPanel>

      <SettingsPanel title={t("settings.instructions.detected.title")} description={t("settings.instructions.detected.description")}>
        {workspace && detectedInstructionFiles.length > 0 ? (
          <div className="instruction-file-list" aria-label={t("settings.instructions.detected.title")}>
            {detectedInstructionFiles.map((entry) => (
              <div className="instruction-file-row" key={entry.path} title={displayPath(entry.path)}>
                <FileText size={14} />
                <span>{relativeInstructionPath(entry.path, workspace.root)}</span>
              </div>
            ))}
          </div>
        ) : (
          <div className="settings-empty-note">{workspace ? t("settings.instructions.detected.empty") : t("settings.instructions.project.noWorkspace")}</div>
        )}
      </SettingsPanel>
    </div>
  );
}

function detectWorkspaceInstructionFiles(fileEntries: FsEntry[], workspace: WorkspaceInfo | null) {
  if (!workspace) return [];
  return fileEntries
    .filter((entry) => entry.kind === "file" && isRulesContextPath(entry.path, workspace.root))
    .sort((left, right) => relativeInstructionPath(left.path, workspace.root).localeCompare(relativeInstructionPath(right.path, workspace.root), undefined, { numeric: true, sensitivity: "base" }));
}

function relativeInstructionPath(path: string, workspaceRoot: string) {
  const normalizedPath = displayPath(path);
  const normalizedRoot = displayPath(workspaceRoot).replace(/\/+$/g, "");
  const lowerPath = normalizedPath.toLowerCase();
  const lowerRoot = normalizedRoot.toLowerCase();
  return lowerPath.startsWith(`${lowerRoot}/`) ? normalizedPath.slice(normalizedRoot.length + 1) : normalizedPath;
}

// Provider management uses a master-to-detail flow: a list of provider tiles, and a focused
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
          const activeModel = isActive
            ? getAiModel(provider, preferences.selectedModelId) ?? provider.models[0]
            : provider.models[0];
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
                <span className="provider-tile-meta">{t("settings.providers.protocolWithModels", { protocol: provider.protocol, count: provider.models.length })}</span>
                <span className="provider-tile-url">{provider.protocol === "local-proxy" ? provider.baseUrl : activeModel.name}</span>
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
  const [providerDiagnostic, setProviderDiagnostic] = useState<AiProviderDiagnosticResponse | null>(null);
  const [runningModelId, setRunningModelId] = useState<string | null>(null);
  const [refreshingModels, setRefreshingModels] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const modelIdRef = useRef(editingModelId);
  const editingModel = getAiModel(provider, editingModelId) ?? provider.models[0];
  const canRemoveModel = provider.models.length > 1;
  const diagnosticState = runningModelId === editingModel.id ? "checking" : providerDiagnostic?.ok === false ? "error" : providerDiagnostic?.ok ? "ok" : "idle";
  const diagnosticLabel = runningModelId === editingModel.id
    ? t("settings.providers.diagnostic.checking")
    : providerDiagnostic
      ? providerDiagnostic.ok
        ? t("settings.providers.diagnostic.ok", { latency: providerDiagnostic.latencyMs })
        : t("settings.providers.diagnostic.failed")
      : t("settings.providers.diagnostic.notRun");

  useEffect(() => {
    if (provider.models.some((model) => model.id === editingModelId)) return;
    setEditingModelId(provider.models[0].id);
  }, [editingModelId, provider]);

  // Clear the diagnostic banner whenever the edited model changes so an "OK"
  // result for one model never lingers on another, and track the current model
  // so an in-flight check can be discarded if the user switches mid-request.
  useEffect(() => {
    modelIdRef.current = editingModel.id;
    setProviderDiagnostic(null);
  }, [editingModel.id]);

  // Auto-pull the live catalog for OpenCode Zen so the real (free-first) model list
  // appears without a manual Refresh. Fires when (a) still on the offline bootstrap
  // placeholder, or (b) a free model has a sub-1M context — the signature of an
  // earlier buggy fetch that inferred e.g. DeepSeek's 128k for "…-free". A single
  // re-fetch self-heals the stored context to the real 1M window.
  const autoRefreshedRef = useRef(false);
  useEffect(() => {
    if (autoRefreshedRef.current) return;
    if (provider.providerType !== "opencode-zen") return;
    const stillBootstrapped = provider.models.length === 1 && provider.models[0].id === "opencode-zen-auto";
    const hasStaleFreeContext = provider.models.some(
      (model) => isFreeModelId(model.id)
        && typeof model.contextTokens === "number"
        && model.contextTokens > 0
        && model.contextTokens < 1_000_000,
    );
    if (!stillBootstrapped && !hasStaleFreeContext) return;
    autoRefreshedRef.current = true;
    void refreshModels();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider.id, provider.providerType]);

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
  const refreshModels = async () => {
    if (refreshingModels) return;
    setRefreshingModels(true);
    setRefreshError(null);
    try {
      const fetched = await fetchProviderModelConfigs(provider);
      if (fetched.length === 0) {
        setRefreshError(t("settings.providers.models.refreshEmpty"));
        return;
      }
      const merged = mergeRefreshedModels(provider, fetched);
      // Keep the selected model if it still exists, else fall back to the first (a free
      // model, since the list is free-first). Reset effort to that model's first level.
      const stillThere = merged.models.some((model) => model.id === preferences.selectedModelId);
      const isActiveProvider = provider.id === preferences.selectedProviderId;
      const nextSelectedModelId = stillThere ? preferences.selectedModelId : merged.models[0].id;
      const nextSelectedModel = getAiModel(merged, nextSelectedModelId) ?? merged.models[0];
      updateProviders(
        preferences.providers.map((candidate) => candidate.id === provider.id ? merged : candidate),
        preferences.selectedProviderId,
        isActiveProvider ? nextSelectedModelId : preferences.selectedModelId,
        isActiveProvider && !stillThere ? (nextSelectedModel.effortLevels[0]?.id ?? "") : preferences.selectedEffortId,
      );
      if (!merged.models.some((model) => model.id === editingModelId)) {
        setEditingModelId(merged.models[0].id);
      }
    } catch (error) {
      setRefreshError(error instanceof Error ? error.message : String(error));
    } finally {
      setRefreshingModels(false);
    }
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
  const runProviderDiagnostic = async () => {
    if (!editingModel || runningModelId) return;
    const targetModelId = editingModel.id;
    setRunningModelId(targetModelId);
    setProviderDiagnostic(null);
    try {
      const result = await luxCommands.aiProviderDiagnostic({
        baseUrl: provider.baseUrl,
        apiKey: provider.apiKey || null,
        payload: {
          model: editingModel.alias || editingModel.id,
          messages: [{ role: "user", content: "Reply with OK." }],
          max_tokens: 8,
          stream: false,
          temperature: 0,
        },
      });
      // Discard the result if the user switched models (or left) mid-request,
      // so a passing check for one model is never bound to another.
      if (modelIdRef.current !== targetModelId) return;
      setProviderDiagnostic(result);
    } catch (error) {
      if (modelIdRef.current !== targetModelId) return;
      setProviderDiagnostic({
        ok: false,
        status: null,
        latencyMs: 0,
        error: error instanceof Error ? error.message : String(error),
        model: editingModel.alias || editingModel.id,
        baseUrl: provider.baseUrl,
      });
    } finally {
      // Only one diagnostic can be in flight (the button is disabled while
      // running), so the running id is always safe to clear here.
      setRunningModelId(null);
    }
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

      <section className="settings-banner" data-state={diagnosticState}>
        <div className="settings-banner-main">
          <strong>{diagnosticLabel}</strong>
          <span>{providerDiagnostic?.error ?? (providerDiagnostic?.ok ? t("settings.providers.diagnostic.okDetail", { status: providerDiagnostic.status ?? "-", latency: providerDiagnostic.latencyMs }) : t("settings.providers.diagnostic.description"))}</span>
        </div>
        <div className="settings-banner-actions">
          <button type="button" disabled={!editingModel || runningModelId !== null} onClick={() => void runProviderDiagnostic()}>
            {t("settings.providers.diagnostic.check")}
          </button>
        </div>
      </section>

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
        <div className="provider-model-manager">
          <div className="provider-model-list">
            <div className="provider-model-list-head">
              <strong>{t("settings.providers.models.listTitle")}</strong>
              <div className="provider-model-list-actions">
                <button type="button" onClick={() => void refreshModels()} disabled={refreshingModels} title={t("settings.providers.models.refreshHint")}>
                  <RefreshCw size={14} className={refreshingModels ? "spin-icon" : undefined} /> {t("settings.providers.models.refresh")}
                </button>
                <button type="button" onClick={addModel}><Plus size={14} /> {t("settings.providers.addModel")}</button>
              </div>
            </div>
            {refreshError && <p className="provider-model-refresh-error" role="alert">{refreshError}</p>}
            <div className="provider-model-rows" role="listbox" aria-label={t("settings.providers.models.listTitle")}>
              {provider.models.map((model) => {
                const activeModel = provider.id === preferences.selectedProviderId && model.id === preferences.selectedModelId;
                return (
                  <button
                    key={model.id}
                    type="button"
                    role="option"
                    aria-selected={model.id === editingModel.id}
                    className="provider-model-row"
                    data-active={model.id === editingModel.id}
                    onClick={() => setEditingModelId(model.id)}
                  >
                    <span className="provider-model-row-main">
                      <strong>{model.name || t("settings.providers.untitledModel")}</strong>
                      <small className="provider-model-row-alias">{model.alias || model.id}</small>
                    </span>
                    <span className="provider-model-row-side">
                      {activeModel ? <span className="provider-model-active-badge">{t("settings.providers.activeModel")}</span> : null}
                      {model.effortLevels.length > 0 ? (
                        <small className="provider-model-effort-meta">{t("settings.providers.effortCount", { count: model.effortLevels.length })}</small>
                      ) : null}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          <div className="provider-model-detail">
            <div className="provider-model-detail-head">
              <div className="provider-model-detail-title">
                <strong>{t("settings.providers.modelDetails")}</strong>
                <span className="provider-model-detail-id">{editingModel.alias || editingModel.id}</span>
              </div>
              <button type="button" className="icon-action danger-button" disabled={!canRemoveModel} onClick={removeModel} aria-label={t("settings.providers.removeModel")} title={t("settings.providers.removeModel")}>
                <Trash2 size={15} />
              </button>
            </div>
            <SettingsGrid>
              <TextSetting label={t("settings.providers.modelName.label")} value={editingModel.name} onChange={(name) => updateEditingModel({ name })} />
              <TextSetting label={t("settings.providers.modelAlias.label")} value={editingModel.alias} onChange={(alias) => updateEditingModel({ alias })} />
              <NumberSetting
                label={t("settings.providers.modelContextTokens.label")}
                detail={t("settings.providers.modelContextTokens.detail", {
                  effective: formatCompactTokens(resolveModelContextTokens(editingModel)),
                })}
                value={editingModel.contextTokens ?? 0}
                min={0}
                max={2_000_000}
                step={1_000}
                onChange={(contextTokens) => updateEditingModel({ contextTokens: contextTokens > 0 ? contextTokens : null })}
              />
              <NumberSetting
                label={t("settings.providers.modelInputPrice.label")}
                detail={t("settings.providers.modelInputPrice.detail")}
                value={editingModel.inputPricePerMillion ?? 0}
                min={0}
                max={1_000}
                step={0.5}
                onChange={(price) => updateEditingModel({ inputPricePerMillion: price > 0 ? price : null })}
              />
              <NumberSetting
                label={t("settings.providers.modelOutputPrice.label")}
                detail={t("settings.providers.modelOutputPrice.detail")}
                value={editingModel.outputPricePerMillion ?? 0}
                min={0}
                max={1_000}
                step={0.5}
                onChange={(price) => updateEditingModel({ outputPricePerMillion: price > 0 ? price : null })}
              />
            </SettingsGrid>
            <div className="effort-editor">
              <div className="effort-editor-head">
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
                  <button type="button" aria-label={t("settings.providers.removeEffort", { label: effort.label || effort.id })} title={t("settings.providers.removeEffort", { label: effort.label || effort.id })} onClick={() => {
                    const nextEfforts = editingModel.effortLevels.filter((candidate) => candidate.id !== effort.id);
                    updateEfforts(nextEfforts, nextEfforts[0]?.id ?? "");
                  }}><Trash2 size={14} /></button>
                </div>
              ))}
            </div>
          </div>
        </div>
      </SettingsPanel>
    </div>
  );
}

function CodeGraphStatusCard({ t }: { t: TranslateFn }) {
  const state = useSyncExternalStore(subscribeCodeGraphState, getCodeGraphStateSnapshot, getCodeGraphStateSnapshot);
  const [busy, setBusy] = useState(false);
  const [vizBusy, setVizBusy] = useState(false);
  const [vizError, setVizError] = useState<string | null>(null);

  useEffect(() => {
    ensureCodeGraphSubscription();
    // Seed from the persisted status once on mount.
    luxCommands
      .codeGraphStatus()
      .then(applyCodeGraphStatus)
      .catch(() => undefined);
    // Refresh the status counts whenever a build finishes.
    return onCodeGraphBuildFinished(() => {
      luxCommands
        .codeGraphStatus()
        .then(applyCodeGraphStatus)
        .catch(() => undefined);
    });
  }, []);

  const rebuild = useCallback(() => {
    clearCodeGraphError();
    setBusy(true);
    luxCommands
      .codeGraphBuild()
      .catch(() => undefined)
      .finally(() => setBusy(false));
  }, []);

  const openVisualization = useCallback(() => {
    setVizError(null);
    setVizBusy(true);
    luxCommands
      .codeGraphExportHtml()
      .then((path) => luxCommands.fileOpenExternal(path))
      .catch((error) => setVizError(error instanceof Error ? error.message : String(error)))
      .finally(() => setVizBusy(false));
  }, []);

  const statusLabel = state.status === "ready"
    ? t("settings.codeGraph.status.ready")
    : state.status === "building"
      ? t("settings.codeGraph.status.building")
      : state.status === "error"
        ? t("settings.codeGraph.status.error")
        : t("settings.codeGraph.status.idle");

  return (
    <section className="index-status-card" data-status={state.status}>
      <div className="index-status-head">
        <div>
          <strong>{t("settings.codeGraph.title")}: {statusLabel}</strong>
          <span>{t("settings.codeGraph.counts", { nodes: state.nodeCount, edges: state.edgeCount })}</span>
        </div>
        <div className="index-status-actions">
          <button
            className="settings-reset-button"
            type="button"
            disabled={vizBusy || state.status !== "ready"}
            onClick={openVisualization}
            title={t("settings.codeGraph.visualizeHint")}
          >
            <Share2 size={14} /> {vizBusy ? t("settings.codeGraph.visualizeBusy") : t("settings.codeGraph.visualize")}
          </button>
          <button className="settings-reset-button" type="button" disabled={busy || state.status === "building"} onClick={rebuild}>
            <RefreshCw size={14} /> {t("settings.codeGraph.rebuild")}
          </button>
        </div>
      </div>
      {state.status === "building" && <div className="index-progress"><span style={{ width: `${state.percent}%` }} /></div>}
      <p className="index-summary-line">{state.status === "building" ? state.step : t("settings.codeGraph.description")}</p>
      {state.error && <p className="index-error-line">{state.error}</p>}
      {vizError && <p className="index-error-line">{vizError}</p>}
    </section>
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
  const qualityLabel = t(`settings.indexing.quality.${aiIndex.quality}` as MessageKey);
  const updatedLabel = aiIndex.updatedAt ? formatIndexUpdatedAt(aiIndex.updatedAt) : t("settings.indexing.neverUpdated");
  const scanSourceLabel = t(`settings.indexing.source.${aiIndex.source}` as MessageKey);
  const scanLimitLabel = aiIndex.scanLimit === null ? t("settings.indexing.noLimit") : formatInteger(aiIndex.scanLimit);
  const scanTruncatedLabel = aiIndex.scanTruncated ? t("settings.indexing.yes") : t("settings.indexing.no");
  return (
    <div className="settings-section-stack">
      <CodeGraphStatusCard t={t} />
      <section className="index-status-card" data-status={aiIndex.status} data-quality={aiIndex.quality}>
        <div className="index-status-head">
          <div>
            <strong>{statusLabel}</strong>
            <span>{t("settings.indexing.filesIndexed", { indexed: aiIndex.indexedFiles, total: aiIndex.totalFiles })} · {qualityLabel}</span>
          </div>
          <em>{Math.round(aiIndex.progress)}%</em>
        </div>
        <div className="index-progress"><span style={{ width: `${aiIndex.progress}%` }} /></div>
        <p className="index-summary-line">{t("settings.indexing.summary", { docs: aiIndex.docsFiles, memory: aiIndex.memoryFiles, rules: aiIndex.rulesFiles, source: aiIndex.sourceFiles, tests: aiIndex.testFiles })}</p>
        <div className="index-metrics">
          <IndexMetric label={t("settings.indexing.metric.source")} value={scanSourceLabel} />
          <IndexMetric label={t("settings.indexing.metric.scanLimit")} value={scanLimitLabel} />
          <IndexMetric label={t("settings.indexing.metric.scanTruncated")} value={scanTruncatedLabel} />
          <IndexMetric label={t("settings.indexing.metric.ignored")} value={formatInteger(aiIndex.ignoredFiles)} />
          <IndexMetric label={t("settings.indexing.metric.truncated")} value={formatInteger(aiIndex.truncatedFiles)} />
          <IndexMetric label={t("settings.indexing.metric.duration")} value={formatIndexDuration(aiIndex.durationMs)} />
          <IndexMetric label={t("settings.indexing.metric.bytes")} value={formatIndexBytes(aiIndex.totalBytes)} />
          <IndexMetric label={t("settings.indexing.metric.updated")} value={updatedLabel} />
        </div>
        {aiIndex.lastError && <p className="index-error-line">{t("settings.indexing.metric.error")}: {aiIndex.lastError}</p>}
        <div className="index-insights">
          <IndexBucketList buckets={aiIndex.languageCounts} emptyLabel={t("settings.indexing.emptyList")} title={t("settings.indexing.languages")} />
          <IndexBucketList buckets={aiIndex.topDirectories} emptyLabel={t("settings.indexing.emptyList")} title={t("settings.indexing.directories")} />
          <IndexImportantFiles files={aiIndex.importantFiles} emptyLabel={t("settings.indexing.emptyList")} title={t("settings.indexing.importantFiles")} />
        </div>
      </section>
      <SettingsPanel title={t("settings.indexing.title")} description={t("settings.indexing.description")}>
        <SettingsGrid>
          <ToggleSetting label={t("settings.indexing.projectIndexing.label")} detail={t("settings.indexing.projectIndexing.detail")} checked={preferences.projectIndexingEnabled} onChange={(projectIndexingEnabled) => onChange({ projectIndexingEnabled })} />
          <ToggleSetting label={t("settings.indexing.realtime.label")} detail={t("settings.indexing.realtime.detail")} checked={preferences.realtimeIndexing} onChange={(realtimeIndexing) => onChange({ realtimeIndexing })} />
          <ToggleSetting label={t("settings.indexing.imageMetadata.label")} detail={t("settings.indexing.imageMetadata.detail")} checked={preferences.includeImages} onChange={(includeImages) => onChange({ includeImages })} />
          <SelectSetting<AiVisionImageFormatPreference>
            label={t("settings.indexing.visionFormat.label")}
            detail={t("settings.indexing.visionFormat.detail")}
            value={preferences.visionImageFormat}
            options={AI_VISION_IMAGE_FORMATS.map((format) => ({ label: t(`settings.indexing.visionFormat.${format}` as MessageKey), value: format }))}
            onChange={(visionImageFormat) => onChange({ visionImageFormat })}
          />
          <SelectSetting<AiScanConcurrency>
            label={t("settings.indexing.scanConcurrency.label")}
            detail={t("settings.indexing.scanConcurrency.detail")}
            value={preferences.scanConcurrency}
            options={AI_SCAN_CONCURRENCY_OPTIONS.map((mode) => ({ label: t(`settings.indexing.scanConcurrency.${mode}` as MessageKey), value: mode }))}
            onChange={(scanConcurrency) => onChange({ scanConcurrency })}
          />
          <NumberSetting label={t("settings.indexing.maxFiles.label")} detail={t("settings.indexing.maxFiles.detail")} value={preferences.maxIndexedFiles} min={500} max={20000} step={500} onChange={(maxIndexedFiles) => onChange({ maxIndexedFiles })} />
        </SettingsGrid>
      </SettingsPanel>
    </div>
  );
}

function IndexMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="index-metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

const maxRecentUsageRows = 60;

// AI Usage: persisted per-request history (model, project, speed, tokens, cost).
// Read-only review surface backed by aiUsageLog; supports clearing the log.
function AiUsageSection({ t, workspace }: { t: TranslateFn; workspace: WorkspaceInfo | null }) {
  const [entries, setEntries] = useState<AiUsageLogEntry[] | null>(null);
  const currentKey = workspaceInstructionsKey(workspace?.root);

  useEffect(() => {
    let active = true;
    void loadAiUsageLog().then((loaded) => { if (active) setEntries(loaded); });
    return () => { active = false; };
  }, []);

  const projects = useMemo(() => entries ? aggregateUsageByProject(entries) : [], [entries]);
  const recent = useMemo(() => entries ? entries.slice(-maxRecentUsageRows).reverse() : [], [entries]);
  const totals = useMemo(() => projects.reduce((sum, project) => ({
    requestCount: sum.requestCount + project.requestCount,
    totalTokens: sum.totalTokens + project.totalTokens,
    estimatedCostUsd: sum.estimatedCostUsd + project.estimatedCostUsd,
    totalDurationMs: sum.totalDurationMs + project.totalDurationMs,
  }), { requestCount: 0, totalTokens: 0, estimatedCostUsd: 0, totalDurationMs: 0 }), [projects]);

  const clearLog = () => { void clearAiUsageLog().then(setEntries); };

  if (entries === null) {
    return <div className="settings-empty-note">{t("settings.usage.loading")}</div>;
  }
  if (entries.length === 0) {
    return (
      <div className="settings-section-stack">
        <div className="settings-empty-note">{t("settings.usage.empty")}</div>
      </div>
    );
  }

  return (
    <div className="settings-section-stack ai-usage-section">
      <SettingsPanel title={t("settings.usage.totals.title")} description={t("settings.usage.totals.description")}>
        <div className="index-metrics">
          <IndexMetric label={t("settings.usage.metric.requests")} value={formatInteger(totals.requestCount)} />
          <IndexMetric label={t("settings.usage.metric.tokens")} value={formatCompactTokens(totals.totalTokens)} />
          <IndexMetric label={t("settings.usage.metric.cost")} value={formatUsageCost(totals.estimatedCostUsd, t)} />
          <IndexMetric label={t("settings.usage.metric.time")} value={formatUsageDuration(totals.totalDurationMs)} />
        </div>
      </SettingsPanel>

      <SettingsPanel title={t("settings.usage.byProject.title")} description={t("settings.usage.byProject.description")}>
        <div className="ai-usage-project-list">
          {projects.map((project) => (
            <div className="ai-usage-project-row" key={project.workspaceKey || "__none__"} data-active={project.workspaceKey === currentKey}>
              <div className="ai-usage-project-main">
                <strong>{project.workspaceName || projectKeyLabel(project.workspaceKey, t)}</strong>
                <small>{t("settings.usage.byProject.requests", { count: project.requestCount })}</small>
              </div>
              <div className="ai-usage-project-stats">
                <span>{formatCompactTokens(project.totalTokens)} {t("settings.usage.tok")}</span>
                <span>{formatUsageCost(project.estimatedCostUsd, t)}</span>
                <span>{formatUsageDuration(project.totalDurationMs)}</span>
              </div>
            </div>
          ))}
        </div>
      </SettingsPanel>

      <SettingsPanel title={t("settings.usage.recent.title")} description={t("settings.usage.recent.description", { count: recent.length })}>
        <div className="ai-usage-table" role="table" aria-label={t("settings.usage.recent.title")}>
          <div className="ai-usage-table-head" role="row">
            <span role="columnheader">{t("settings.usage.col.when")}</span>
            <span role="columnheader">{t("settings.usage.col.model")}</span>
            <span role="columnheader">{t("settings.usage.col.tokens")}</span>
            <span role="columnheader">{t("settings.usage.col.speed")}</span>
            <span role="columnheader">{t("settings.usage.col.cost")}</span>
          </div>
          {recent.map((entry) => (
            <div className="ai-usage-table-row" role="row" key={entry.id} title={`${entry.provider} · ${entry.agentMode}`}>
              <span role="cell">{formatUsageTimestamp(entry.timestamp)}</span>
              <span role="cell" className="ai-usage-model" title={entry.model}>{entry.model}</span>
              <span role="cell">{formatCompactTokens(entry.totalTokens)}</span>
              <span role="cell">{formatUsageSpeed(usageEntryTokensPerSecond(entry), t)}</span>
              <span role="cell">{formatUsageCost(entry.estimatedCostUsd, t)}</span>
            </div>
          ))}
        </div>
      </SettingsPanel>

      <div className="ai-usage-actions">
        <button type="button" className="settings-reset-button" onClick={clearLog}>
          <Trash2 size={14} /> {t("settings.usage.clear")}
        </button>
      </div>
    </div>
  );
}

function projectKeyLabel(key: string, t: TranslateFn) {
  if (!key) return t("settings.usage.noProject");
  const segments = key.split("/").filter(Boolean);
  return segments[segments.length - 1] || key;
}

function formatUsageCost(usd: number | null, t: TranslateFn) {
  if (usd === null || usd <= 0) return t("settings.usage.costUnknown");
  if (usd < 0.01) return "<$0.01";
  return `$${usd.toFixed(usd < 1 ? 3 : 2)}`;
}

function formatUsageDuration(ms: number) {
  if (ms <= 0) return "—";
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${Math.round(seconds % 60)}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

function formatUsageSpeed(tokensPerSecond: number, t: TranslateFn) {
  if (tokensPerSecond <= 0) return "—";
  return t("settings.usage.tokPerSec", { value: tokensPerSecond < 10 ? tokensPerSecond.toFixed(1) : String(Math.round(tokensPerSecond)) });
}

function formatUsageTimestamp(timestamp: number) {
  const date = new Date(timestamp);
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  const time = date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  if (sameDay) return time;
  return `${date.toLocaleDateString(undefined, { month: "short", day: "numeric" })} ${time}`;
}

function IndexBucketList({ buckets, emptyLabel, title }: { buckets: Array<{ count: number; label: string }>; emptyLabel: string; title: string }) {
  return (
    <div className="index-insight-list">
      <h4>{title}</h4>
      {buckets.length === 0 ? <p>{emptyLabel}</p> : buckets.map((bucket) => (
        <div className="index-bucket-row" key={bucket.label}>
          <span>{bucket.label}</span>
          <strong>{formatInteger(bucket.count)}</strong>
        </div>
      ))}
    </div>
  );
}

function IndexImportantFiles({ emptyLabel, files, title }: { emptyLabel: string; files: Array<{ language: string; relativePath: string }>; title: string }) {
  return (
    <div className="index-insight-list index-important-files">
      <h4>{title}</h4>
      {files.length === 0 ? <p>{emptyLabel}</p> : files.slice(0, 6).map((file) => (
        <div className="index-file-row" key={file.relativePath} title={file.relativePath}>
          <span>{file.relativePath}</span>
          <strong>{file.language}</strong>
        </div>
      ))}
    </div>
  );
}

function formatInteger(value: number) {
  return new Intl.NumberFormat().format(value);
}

function formatIndexDuration(value: number | null) {
  if (value === null) return "-";
  if (value < 1_000) return `${value} ms`;
  return `${(value / 1_000).toFixed(2)} s`;
}

function formatIndexBytes(bytes: number) {
  if (bytes < 1_000) return `${bytes} B`;
  if (bytes < 1_000_000) return `${(bytes / 1_000).toFixed(1)} KB`;
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

function formatIndexUpdatedAt(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(date);
}

function providerPresetDescription(providerType: string, t: TranslateFn) {
  const descriptionKey = PROVIDER_PRESET_DESCRIPTION_KEYS[providerType];
  return descriptionKey ? t(descriptionKey) : t("settings.providers.customProvider");
}

function sectionMatchesQuery(section: SettingsSection, query: string, t: TranslateFn) {
  return [t(section.titleKey), t(section.descriptionKey), ...section.keywords].some((value) => value.toLowerCase().includes(query));
}

function resetSection(sectionId: SettingsSectionId, resetEditor: (preferences: EditorPreferences) => void, resetAi: (preferences: AiPreferences) => void, currentAiPreferences: AiPreferences) {
  if (sectionId === "editor") resetEditor(defaultEditorPreferences);
  else if (sectionId === "ai-runtime") {
    resetAi(mergeAiPreferences(currentAiPreferences, {
      selectedAgentId: defaultAiPreferences.selectedAgentId,
      selectedProviderId: defaultAiPreferences.selectedProviderId,
      selectedModelId: defaultAiPreferences.selectedModelId,
      selectedEffortId: defaultAiPreferences.selectedEffortId,
      toolApprovalMode: defaultAiPreferences.toolApprovalMode,
      toolRoundLimit: defaultAiPreferences.toolRoundLimit,
      maxParallelSubagents: defaultAiPreferences.maxParallelSubagents,
      showResponseDuration: defaultAiPreferences.showResponseDuration,
      contextAutoCompactEnabled: defaultAiPreferences.contextAutoCompactEnabled,
      contextAutoCompactThreshold: defaultAiPreferences.contextAutoCompactThreshold,
    }));
  } else if (sectionId === "ai-browser") {
    resetAi(mergeAiPreferences(currentAiPreferences, {
      agentBrowserEnabled: defaultAiPreferences.agentBrowserEnabled,
      agentBrowserCommand: defaultAiPreferences.agentBrowserCommand,
      agentBrowserHeaded: defaultAiPreferences.agentBrowserHeaded,
      agentBrowserAllowedDomains: defaultAiPreferences.agentBrowserAllowedDomains,
      agentBrowserMaxOutput: defaultAiPreferences.agentBrowserMaxOutput,
      agentBrowserPersistSession: defaultAiPreferences.agentBrowserPersistSession,
      agentBrowserProfile: defaultAiPreferences.agentBrowserProfile,
      agentBrowserStatePath: defaultAiPreferences.agentBrowserStatePath,
      agentBrowserContentBoundaries: defaultAiPreferences.agentBrowserContentBoundaries,
      agentBrowserIgnoreHttpsErrors: defaultAiPreferences.agentBrowserIgnoreHttpsErrors,
      agentBrowserAutoStreamPreview: defaultAiPreferences.agentBrowserAutoStreamPreview,
      agentBrowserDashboardPort: defaultAiPreferences.agentBrowserDashboardPort,
      agentBrowserAllowFileAccess: defaultAiPreferences.agentBrowserAllowFileAccess,
      agentBrowserProvider: defaultAiPreferences.agentBrowserProvider,
      agentBrowserProxy: defaultAiPreferences.agentBrowserProxy,
    }));
  } else if (sectionId === "ai-instructions") {
    resetAi(mergeAiPreferences(currentAiPreferences, {
      selectedAgentId: defaultAiPreferences.selectedAgentId,
      agentProfiles: defaultAiPreferences.agentProfiles,
    }));
  } else if (sectionId === "ai-providers") {
    resetAi(mergeAiPreferences(currentAiPreferences, {
      providers: defaultAiPreferences.providers,
      selectedProviderId: defaultAiPreferences.selectedProviderId,
      selectedModelId: defaultAiPreferences.selectedModelId,
      selectedEffortId: defaultAiPreferences.selectedEffortId,
    }));
  } else if (sectionId === "ai-indexing") {
    resetAi(mergeAiPreferences(currentAiPreferences, {
      projectIndexingEnabled: defaultAiPreferences.projectIndexingEnabled,
      realtimeIndexing: defaultAiPreferences.realtimeIndexing,
      includeImages: defaultAiPreferences.includeImages,
      maxIndexedFiles: defaultAiPreferences.maxIndexedFiles,
    }));
  }
}
