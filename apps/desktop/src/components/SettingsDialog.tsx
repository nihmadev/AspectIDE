import * as Dialog from "@radix-ui/react-dialog";
import { ArrowUpCircle, Award, Brain, Braces, Cable, ChartColumn, Cloud, Compass, Download, FileCode, FileText, HelpCircle, Key, Loader2, Play, RefreshCw, ScrollText, Search, Settings2, Share2, Trash2 } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { FontSelectSetting, NumberSetting, SegmentedSetting, SelectSetting, SettingsGrid, SettingsPanel, TextareaSetting, TextSetting, ToggleSetting, ToolRoundLimitSetting, type SaveState } from "./settings/SettingsControls";
import { SkillsSection } from "./settings/SkillsSection";
import { MemorySection } from "./settings/MemorySection";
import { SshSection } from "./settings/SshSection";
import { McpSection } from "./settings/McpSection";
import { AiProvidersSection } from "./settings/AiProvidersSection";
import { AgentBrowserSection } from "./settings/AgentBrowserSection";
import {
  AI_PREFERENCES_KEY,
  aiToolRoundLimitMax,
  aiToolRoundLimitMin,
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
  type AiPreferences,
  type AiFileEditTrustMode,
  type AiToolApprovalMode,
  type AiVisionImageFormatPreference,
  type AiScanConcurrency,
} from "../lib/aiPreferences";
import { AI_VISION_IMAGE_FORMATS } from "../lib/aiVisionFormat";
import { AiUsageSection } from "./settings/AiUsageSection";

const AI_SCAN_CONCURRENCY_OPTIONS: readonly AiScanConcurrency[] = ["auto", "all", "half"];
import { formatCompactTokens } from "../lib/aiChatContextUsage";
import {
  clearLspInstallError,
  ensureLspInstallSubscription,
  getLspInstallProgressSnapshot,
  onLspInstallFinished,
  setLspUninstallIntent,
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
import { getFolderIconSvg } from "../lib/fileIconMap";
import { extensionForLanguage, fileIconForName } from "../lib/fileIcons";
import { displayNameForLanguage } from "../lib/languageLabels";
import { loadDictionary, LOCALES, UI_LOCALE_KEY, type Locale, type MessageKey } from "../lib/i18n";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
import { isRulesContextPath } from "../lib/aiRuntimeFileContext";
import { useLuxStore } from "../lib/store";
import { isTauriRuntime, luxCommands, type LspCatalogEntry, type RuntimeCatalogEntry } from "../lib/tauri";
import { useUpdater } from "../lib/useUpdater";
import type { FsEntry, WorkspaceInfo } from "../lib/types";

const scope = "user" as const;

// AI configuration is split into focused sections so runtime, instructions,
// providers, and indexing do not compete in one mixed settings list.
type SettingsSectionId = "general" | "editor" | "lsp" | "ai-runtime" | "ai-browser" | "ai-instructions" | "ai-skills" | "ai-memory" | "ai-ssh" | "ai-mcp" | "ai-providers" | "ai-indexing" | "ai-usage";

type SettingsSection = {
  id: SettingsSectionId;
  titleKey: MessageKey;
  descriptionKey: MessageKey;
  icon: ReactNode;
  keywords: string[];
};

const settingsSections: SettingsSection[] = [
  {
    id: "general",
    titleKey: "settings.general.title",
    descriptionKey: "settings.general.description",
    icon: <Settings2 size={16} />,
    keywords: ["language", "locale", "russian", "english", "язык", "general", "язык", "общие"],
  },
  {
    id: "editor",
    titleKey: "settings.group.editor",
    descriptionKey: "settings.editor.description",
    icon: <FileCode size={16} />,
    keywords: ["font", "line", "tab", "whitespace", "unicode", "minimap", "word wrap", "mouse", "zoom", "smooth", "ligatures", "appearance", "behavior", "редактор", "шрифт"],
  },
  {
    id: "lsp",
    titleKey: "settings.lsp.title",
    descriptionKey: "settings.lsp.description",
    icon: <Braces size={16} />,
    keywords: ["lsp", "language server", "rust-analyzer", "gopls", "ty", "pyright", "typescript", "clangd", "intellisense", "completion", "hover", "языковой сервер"],
  },
  {
    id: "ai-runtime",
    titleKey: "settings.aiRuntime.title",
    descriptionKey: "settings.aiRuntime.description",
    icon: <Play size={16} />,
    keywords: ["ai", "agent", "mode", "model", "effort", "reasoning", "tools", "tool rounds", "runtime", "compact", "context"],
  },
  {
    id: "ai-browser",
    titleKey: "settings.agentBrowser.nav.title",
    descriptionKey: "settings.agentBrowser.nav.description",
    icon: <Compass size={16} />,
    keywords: ["browser", "agent-browser", "chromium", "chrome", "automation", "stream", "preview", "браузер"],
  },
  {
    id: "ai-instructions",
    titleKey: "settings.instructions.title",
    descriptionKey: "settings.instructions.description",
    icon: <ScrollText size={16} />,
    keywords: ["ai", "instructions", "system", "prompt", "profile", "behavior", "agent", "plan", "ask"],
  },
  {
    id: "ai-skills",
    titleKey: "settings.skills.title",
    descriptionKey: "settings.skills.description",
    icon: <Award size={16} />,
    keywords: ["skill", "skills", "procedure", "playbook", "reusable", "instructions", "навык", "навыки"],
  },
  {
    id: "ai-memory",
    titleKey: "settings.memory.title",
    descriptionKey: "settings.memory.description",
    icon: <Brain size={16} />,
    keywords: ["memory", "memories", "remember", "recall", "durable", "context", "память", "запомнить"],
  },
  {
    id: "ai-ssh",
    titleKey: "settings.ssh.title",
    descriptionKey: "settings.ssh.description",
    icon: <Key size={16} />,
    keywords: ["ssh", "scp", "sftp", "remote", "server", "openssh", "host", "known_hosts", "identity", "key", "ssh-agent", "удалённый", "сервер"],
  },
  {
    id: "ai-mcp",
    titleKey: "settings.mcp.title",
    descriptionKey: "settings.mcp.description",
    icon: <Cable size={16} />,
    keywords: ["mcp", "model context protocol", "server", "servers", "tools", "stdio", "integration", "сервер", "инструменты"],
  },
  {
    id: "ai-providers",
    titleKey: "settings.providers.title",
    descriptionKey: "settings.providers.description",
    icon: <Cloud size={16} />,
    keywords: ["ai", "provider", "providers", "model", "models", "openai", "anthropic", "openrouter", "gemini", "local", "proxy", "api key", "base url"],
  },
  {
    id: "ai-indexing",
    titleKey: "settings.indexing.title",
    descriptionKey: "settings.indexing.description",
    icon: <Search size={16} />,
    keywords: ["ai", "index", "indexing", "files", "images", "metadata", "context", "workspace"],
  },
  {
    id: "ai-usage",
    titleKey: "settings.usage.title",
    descriptionKey: "settings.usage.description",
    icon: <ChartColumn size={16} />,
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
  const [activeTab, setActiveTab] = useState<SettingsSectionId>("general");
  const [saveState, setSaveState] = useState<SaveState>("idle");

  const persistLocale = useCallback(
    (nextLocale: Locale) => {
      void loadDictionary(nextLocale).finally(() => setLocale(nextLocale));
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
      setActiveTab(settingsInitialSection as SettingsSectionId);
    }
  }, [open, settingsInitialSection]);

  useEffect(() => {
    if (!open) return;

    let cancelled = false;
    void luxCommands.settingsGet(scope, AI_PREFERENCES_KEY).then((setting) => {
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

  const activeSection = sectionById.get(activeTab) ?? settingsSections[0];
  const bodyRef = useRef<HTMLDivElement>(null);

  const renderSidebarItem = (id: SettingsSectionId, icon: ReactNode, label: string) => (
    <button
      className={`settings__sidebar-item ${activeTab === id ? "active" : ""}`}
      onClick={() => setActiveTab(id)}
    >
      {icon}
      <span>{label}</span>
    </button>
  );

  return (
    <Dialog.Root open={open} onOpenChange={setOpen}>
      <Dialog.Portal>
        <Dialog.Overlay asChild>
          <div className="settings__overlay" />
        </Dialog.Overlay>
        <Dialog.Content
          className="settings__container"
          aria-describedby={undefined}
          onEscapeKeyDown={(event) => {
            const active = document.activeElement;
            if (active instanceof HTMLElement && active.closest(".compact-dropdown-menu")) {
              event.preventDefault();
            }
          }}
        >
          <div className="settings__sidebar">
            <div className="settings__sidebar-group">
              <div className="settings__sidebar-title">Workspace</div>
              {renderSidebarItem("general", <Settings2 size={14} />, "General")}
              {renderSidebarItem("editor", <FileCode size={14} />, "Editor")}
              {renderSidebarItem("lsp", <Braces size={14} />, "LSP")}
            </div>
            <div className="settings__sidebar-group">
              <div className="settings__sidebar-title">AI</div>
              {renderSidebarItem("ai-runtime", <Play size={14} />, "Runtime")}
              {renderSidebarItem("ai-browser", <Compass size={14} />, "Browser")}
              {renderSidebarItem("ai-instructions", <ScrollText size={14} />, "Instructions")}
              {renderSidebarItem("ai-providers", <Cloud size={14} />, "Providers")}
              {renderSidebarItem("ai-indexing", <Search size={14} />, "Indexing")}
              {renderSidebarItem("ai-usage", <ChartColumn size={14} />, "Usage")}
              {renderSidebarItem("ai-skills", <Award size={14} />, "Skills")}
              {renderSidebarItem("ai-memory", <Brain size={14} />, "Memory")}
              {renderSidebarItem("ai-ssh", <Key size={14} />, "SSH")}
              {renderSidebarItem("ai-mcp", <Cable size={14} />, "MCP")}
            </div>
            <div className="settings__sidebar-footer">
              <div className="settings__app-info">
                Lux IDE
              </div>
            </div>
          </div>

          <div className="settings__content">
            <div className="settings__content-header">
              <h2>{t(activeSection.titleKey)}</h2>
              <button className="settings__close" onClick={() => setOpen(false)}>×</button>
            </div>
            <div className="settings__content-body" ref={bodyRef}>
              {activeTab === "general" && <GeneralSection locale={locale} onChangeLocale={persistLocale} t={t} />}
              {activeTab === "editor" && (
                <div className="settings-section-stack">
                  <FontsSection preferences={editorPreferences} onChange={updateEditorPreference} t={t} />
                  <EditorAppearanceSection preferences={editorPreferences} onChange={updateEditorPreference} t={t} />
                  <EditorBehaviorSection preferences={editorPreferences} onChange={updateEditorPreference} t={t} />
                </div>
              )}
              {activeTab === "lsp" && <LanguageServersSection preferences={aiPreferences} onChange={updateAiPreference} t={t} />}
              {activeTab === "ai-runtime" && (
                <AiActiveCard preferences={aiPreferences} onChange={updateAiPreference} t={t} />
              )}
              {activeTab === "ai-browser" && (
                <AgentBrowserSection preferences={aiPreferences} onChange={updateAiPreference} t={t} />
              )}
              {activeTab === "ai-instructions" && <AiInstructionsSection fileEntries={fileEntries} preferences={aiPreferences} workspace={workspace} onChange={updateAiPreference} t={t} />}
              {activeTab === "ai-providers" && <AiProvidersSection preferences={aiPreferences} onChange={updateAiPreference} t={t} />}
              {activeTab === "ai-indexing" && <AiIndexingSection aiIndex={aiIndex} preferences={aiPreferences} onChange={updateAiPreference} t={t} />}
              {activeTab === "ai-skills" && <SkillsSection workspace={workspace} t={t} />}
              {activeTab === "ai-memory" && <MemorySection workspace={workspace} t={t} />}
              {activeTab === "ai-ssh" && <SshSection t={t} />}
              {activeTab === "ai-mcp" && <McpSection t={t} />}
              {activeTab === "ai-usage" && <AiUsageSection workspace={workspace} t={t} />}
            </div>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
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

let systemFontFamiliesCache: string[] | null = null;
let systemFontFamiliesPromise: Promise<string[]> | null = null;

function useSystemFontFamilies(): string[] {
  const [fonts, setFonts] = useState<string[]>(() => systemFontFamiliesCache ?? []);
  useEffect(() => {
    if (systemFontFamiliesCache) return;
    systemFontFamiliesPromise ??= luxCommands.listSystemFontFamilies().then((families) => {
      systemFontFamiliesCache = families;
      return families;
    }).catch(() => {
      systemFontFamiliesPromise = null;
      return [] as string[];
    });
    let cancelled = false;
    void systemFontFamiliesPromise.then((families) => {
      if (!cancelled && families.length > 0) setFonts(families);
    });
    return () => { cancelled = true; };
  }, []);
  return fonts;
}

function FontsSection({ onChange, preferences, t }: { onChange: (patch: Partial<EditorPreferences>) => void; preferences: EditorPreferences; t: TranslateFn }) {
  const fonts = useSystemFontFamilies();
  return (
    <SettingsPanel title={t("settings.fonts.title")} description={t("settings.fonts.description")}>
      <SettingsGrid>
        <FontSelectSetting
          label={t("settings.fonts.ui.label")}
          detail={t("settings.fonts.ui.detail")}
          value={preferences.uiFontFamily}
          fonts={fonts}
          defaultLabel={t("settings.fonts.default")}
          searchPlaceholder={t("settings.fonts.searchPlaceholder")}
          searchEmptyLabel={t("settings.fonts.searchEmpty")}
          onChange={(uiFontFamily) => onChange({ uiFontFamily })}
        />
        <FontSelectSetting
          label={t("settings.fonts.editor.label")}
          detail={t("settings.fonts.editor.detail")}
          value={preferences.fontFamily}
          fonts={fonts}
          defaultLabel={t("settings.fonts.default")}
          searchPlaceholder={t("settings.fonts.searchPlaceholder")}
          searchEmptyLabel={t("settings.fonts.searchEmpty")}
          onChange={(fontFamily) => onChange({ fontFamily })}
        />
        <FontSelectSetting
          label={t("settings.fonts.chat.label")}
          detail={t("settings.fonts.chat.detail")}
          value={preferences.chatFontFamily}
          fonts={fonts}
          defaultLabel={t("settings.fonts.default")}
          searchPlaceholder={t("settings.fonts.searchPlaceholder")}
          searchEmptyLabel={t("settings.fonts.searchEmpty")}
          onChange={(chatFontFamily) => onChange({ chatFontFamily })}
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
        <ToggleSetting label={t("settings.behavior.autoOpenEditedFiles.label")} detail={t("settings.behavior.autoOpenEditedFiles.detail")} checked={preferences.autoOpenEditedFiles} onChange={(autoOpenEditedFiles) => onChange({ autoOpenEditedFiles })} />
      </SettingsGrid>
    </SettingsPanel>
  );
}

function LanguageServersSection({ onChange, preferences, t }: { onChange: (patch: Partial<AiPreferences>) => void; preferences: AiPreferences; t: TranslateFn }) {
  const [catalog, setCatalog] = useState<LspCatalogEntry[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [progress, setProgress] = useState<Record<string, LspInstallProgress>>(getLspInstallProgressSnapshot);
  const [uninstallNotes, setUninstallNotes] = useState<Record<string, string>>({});

  const refreshCatalog = useCallback(() => {
    luxCommands.lspServerCatalog()
      .then((entries) => { setCatalog(entries); setLoadError(null); })
      .catch((error) => setLoadError(error instanceof Error ? error.message : String(error)));
  }, []);

  useEffect(() => {
    ensureLspInstallSubscription();
    refreshCatalog();
    const stopProgress = subscribeLspInstallProgress(() => setProgress({ ...getLspInstallProgressSnapshot() }));
    const stopFinish = onLspInstallFinished(() => refreshCatalog());
    return () => { stopProgress(); stopFinish(); };
  }, [refreshCatalog]);

  const installServer = (languageId: string) => {
    clearLspInstallError(languageId);
    setUninstallNotes((notes) => (languageId in notes ? withoutKey(notes, languageId) : notes));
    setProgress({ ...getLspInstallProgressSnapshot(), [languageId]: { status: "installing", percent: 0, step: "Starting" } });
    void luxCommands.lspInstallServer(languageId).catch(() => undefined);
  };

  const uninstallServer = (languageId: string, name: string) => {
    if (!window.confirm(t("settings.lsp.uninstallConfirm", { name }))) return;
    clearLspInstallError(languageId);
    setUninstallNotes((notes) => (languageId in notes ? withoutKey(notes, languageId) : notes));
    setLspUninstallIntent(languageId);
    setProgress({ ...getLspInstallProgressSnapshot() });
    void luxCommands.lspUninstallServer(languageId)
      .then((result) => setUninstallNotes((notes) => ({ ...notes, [languageId]: result })))
      .catch(() => undefined);
  };

  const installedCount = catalog?.filter((entry) => entry.installed).length ?? 0;

  return (
    <SettingsPanel>
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
                note={uninstallNotes[entry.languageId] ?? null}
                onInstall={() => installServer(entry.languageId)}
                onUninstall={() => uninstallServer(entry.languageId, entry.name)}
                t={t}
              />
            );
          })}
        </ul>
      )}
    </SettingsPanel>
  );
}

function withoutKey<T extends Record<string, string>>(obj: T, key: string): T {
  const { [key]: _removed, ...rest } = obj;
  return rest as T;
}

function LanguageServerRow({ entry, progress, note, onInstall, onUninstall, t }: {
  entry: LspCatalogEntry;
  progress: LspInstallProgress | null;
  note: string | null;
  onInstall: () => void;
  onUninstall: () => void;
  t: TranslateFn;
}) {
  const installing = progress?.status === "installing" && !progress.uninstalling;
  const uninstalling = progress?.status === "installing" && progress.uninstalling === true;
  const busy = installing || uninstalling;
  const errored = progress?.status === "error";
  const state = busy ? "installing" : errored ? "error" : entry.installed ? (entry.managed ? "managed" : "path") : "missing";
  const isManual = entry.installMethod === "manual";
  const canUninstall = entry.managed && entry.installed && !isManual;
  const [showError, setShowError] = useState(false);

  return (
    <li className="lsp-server-row" data-state={state}>
      <div className="lsp-server-row-main">
        <div className="lsp-server-row-title">
          <span className="lsp-server-dot" data-state={state} aria-hidden="true" />
          <strong>{entry.name}</strong>
          {errored && (
            <button type="button" className="lsp-server-error-btn" onClick={() => setShowError(!showError)} title={progress?.error}>
              <HelpCircle size={12} />
            </button>
          )}
        </div>
        {busy && (
          <div className="lsp-server-progress" role="progressbar" aria-valuenow={progress?.percent ?? 0} aria-valuemin={0} aria-valuemax={100}>
            <div className="lsp-server-progress-fill" style={{ width: `${progress?.percent ?? 0}%` }} />
            <span className="lsp-server-progress-step">{progress?.step}</span>
          </div>
        )}
        {errored && showError && <p className="lsp-server-row-error">{progress?.error}</p>}
        {isManual && !entry.installed && !busy && <p className="lsp-server-row-hint">{entry.manualHint}</p>}
        {!busy && !errored && note && <p className="lsp-server-row-note">{note}</p>}
      </div>
      <div className="lsp-server-row-action">
        {isManual ? (
          <span className="lsp-server-manual-tag">{t("settings.lsp.manualTag")}</span>
        ) : (
          <>
            {canUninstall && (
              <button type="button" className="lsp-server-uninstall" onClick={onUninstall} disabled={busy}>
                <Trash2 size={14} />
                {uninstalling ? t("settings.lsp.uninstalling") : t("settings.lsp.uninstall")}
              </button>
            )}
            <button type="button" className="lsp-server-install" data-installed={entry.installed || undefined} onClick={onInstall} disabled={busy}>
              {installing ? (
                <Loader2 size={14} className="spin-icon" />
              ) : entry.installed ? (
                <RefreshCw size={14} />
              ) : (
                <Download size={14} />
              )}
              {installing
                ? t("settings.lsp.installing")
                : entry.installed
                  ? t("settings.lsp.reinstall")
                  : t("settings.lsp.install")}
            </button>
          </>
        )}
      </div>
    </li>
  );
}

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

  return (
    <li className="lsp-server-row" data-state={state}>
      <div className="lsp-server-row-main">
        <div className="lsp-server-row-title">
          <span className="lsp-server-dot" data-state={state} aria-hidden="true" />
          <strong>{entry.name}</strong>
        </div>
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
          <button type="button" className="lsp-server-install" data-installed={entry.installed || undefined} onClick={onProvision} disabled={installing}>
            {installing ? (
              <Loader2 size={14} className="spin-icon" />
            ) : entry.installed ? (
              <RefreshCw size={14} />
            ) : (
              <Download size={14} />
            )}
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

const AI_TOOL_APPROVAL_MODES: AiToolApprovalMode[] = ["default", "full-access"];
const AI_FILE_EDIT_TRUST_MODES: AiFileEditTrustMode[] = ["preview-before-apply", "apply-immediately"];

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
        <ToolRoundLimitSetting
          label={t("settings.aiRuntime.goalRunMaxTokens.label")}
          detail={t("settings.aiRuntime.goalRunMaxTokens.detail")}
          value={preferences.goalRunMaxTokens}
          min={10_000}
          max={500_000}
          step={10_000}
          fallbackLimitedValue={200_000}
          unlimitedLabel={t("settings.aiRuntime.limit.off")}
          limitedLabel={t("settings.aiRuntime.limit.custom")}
          onChange={(goalRunMaxTokens) => onChange({ goalRunMaxTokens })}
        />
        <ToolRoundLimitSetting
          label={t("settings.aiRuntime.goalRunMaxRounds.label")}
          detail={t("settings.aiRuntime.goalRunMaxRounds.detail")}
          value={preferences.goalRunMaxRounds}
          min={8}
          max={80}
          step={2}
          fallbackLimitedValue={32}
          unlimitedLabel={t("settings.aiRuntime.limit.default")}
          limitedLabel={t("settings.aiRuntime.limit.custom")}
          onChange={(goalRunMaxRounds) => onChange({ goalRunMaxRounds })}
        />
        <ToolRoundLimitSetting
          label={t("settings.aiRuntime.automaticModeHardStop.label")}
          detail={t("settings.aiRuntime.automaticModeHardStop.detail")}
          value={preferences.automaticModeHardStopMinutes}
          min={15}
          max={480}
          step={15}
          fallbackLimitedValue={60}
          unlimitedLabel={t("settings.aiRuntime.limit.unlimited")}
          limitedLabel={t("settings.aiRuntime.limit.custom")}
          onChange={(automaticModeHardStopMinutes) => onChange({ automaticModeHardStopMinutes })}
        />
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
        <ToggleSetting label={t("settings.aiRuntime.tokenSpeed.label")} detail={t("settings.aiRuntime.tokenSpeed.detail")} checked={preferences.showTokenSpeed} onChange={(showTokenSpeed) => onChange({ showTokenSpeed })} />
        <ToggleSetting label={t("settings.aiRuntime.liveWorkTicker.label")} detail={t("settings.aiRuntime.liveWorkTicker.detail")} checked={preferences.liveWorkTicker} onChange={(liveWorkTicker) => onChange({ liveWorkTicker })} />
        <ToggleSetting label={t("settings.aiRuntime.smoothStream.label")} detail={t("settings.aiRuntime.smoothStream.detail")} checked={preferences.chatSmoothStream} onChange={(chatSmoothStream) => onChange({ chatSmoothStream })} />
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
          commitOnBlur
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

function CodeGraphStatusCard({ t }: { t: TranslateFn }) {
  const state = useSyncExternalStore(subscribeCodeGraphState, getCodeGraphStateSnapshot, getCodeGraphStateSnapshot);
  const [busy, setBusy] = useState(false);
  const [vizBusy, setVizBusy] = useState(false);
  const [vizError, setVizError] = useState<string | null>(null);

  useEffect(() => {
    ensureCodeGraphSubscription();
    luxCommands
      .codeGraphStatus()
      .then(applyCodeGraphStatus)
      .catch(() => undefined);
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
  const [liveLanguages, setLiveLanguages] = useState<Array<{ count: number; label: string }> | null>(null);
  useEffect(() => {
    if (!isTauriRuntime()) return;
    let cancelled = false;
    luxCommands.aiIndexLanguages().then((langs) => {
      if (!cancelled) setLiveLanguages(langs.map((l) => ({ label: l.key, count: l.count })));
    }).catch(() => undefined);
    return () => { cancelled = true; };
  }, []);
  const languages = (liveLanguages ?? aiIndex.languageCounts).filter(
    (l) => !["other", "icns", "ico", "idx", "lock", "sample"].includes(l.label.toLowerCase())
  );
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
          <IndexMetric label={t("settings.indexing.metric.configFiles")} value={formatInteger(aiIndex.configFiles)} />
          <IndexMetric label={t("settings.indexing.metric.ignored")} value={formatInteger(aiIndex.ignoredFiles)} />
          <IndexMetric label={t("settings.indexing.metric.truncated")} value={formatInteger(aiIndex.truncatedFiles)} />
          <IndexMetric label={t("settings.indexing.metric.duration")} value={formatIndexDuration(aiIndex.durationMs)} />
          <IndexMetric label={t("settings.indexing.metric.bytes")} value={formatIndexBytes(aiIndex.totalBytes)} />
          <IndexMetric label={t("settings.indexing.metric.updated")} value={updatedLabel} />
        </div>
        {aiIndex.lastError && <p className="index-error-line">{t("settings.indexing.metric.error")}: {aiIndex.lastError}</p>}
        <div className="index-insights">
          <IndexBucketList buckets={languages} emptyLabel={t("settings.indexing.emptyList")} title={t("settings.indexing.languages")} variant="language" />
          <IndexBucketList buckets={aiIndex.topDirectories} emptyLabel={t("settings.indexing.emptyList")} title={t("settings.indexing.directories")} variant="directory" />
          <IndexImportantFiles files={aiIndex.importantFiles} emptyLabel={t("settings.indexing.emptyList")} title={t("settings.indexing.importantFiles")} />
        </div>
      </section>
      <SettingsPanel>
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

function IndexBucketList({ buckets, emptyLabel, title, variant }: { buckets: Array<{ count: number; label: string }>; emptyLabel: string; title: string; variant?: "language" | "directory" }) {
  return (
    <div className="index-insight-list">
      <h4>{title}</h4>
      {buckets.length === 0 ? <p>{emptyLabel}</p> : buckets.map((bucket) => {
        const displayLabel = variant === "language"
          ? displayNameForLanguage(bucket.label)
          : bucket.label;
        const iconEl = variant === "language"
          ? (() => {
              const lower = bucket.label.toLowerCase();
              const ext = extensionForLanguage(lower);
              const meta = ext
                ? fileIconForName(`file.${ext}`)
                : fileIconForName(`index.${lower}`);
              return meta.imgSrc
                ? <img src={meta.imgSrc} width={14} height={14} className={meta.className} alt="" />
                : <meta.Icon size={14} className={meta.className} />;
            })()
          : variant === "directory"
            ? (() => {
                const folderName = bucket.label.includes("/") ? bucket.label.split("/").at(-1)! : bucket.label;
                return <img src={getFolderIconSvg(folderName)} width={14} height={14} className="folder-icon" alt="" />;
              })()
            : null;
        return (
          <div className="index-bucket-row" key={bucket.label}>
            <div className="index-bucket-label">
              {iconEl}
              <span>{displayLabel}</span>
            </div>
            <strong>{formatInteger(bucket.count)}</strong>
          </div>
        );
      })}
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
      fileEditTrustMode: defaultAiPreferences.fileEditTrustMode,
      tokenEconomyEnabled: defaultAiPreferences.tokenEconomyEnabled,
      toolRoundLimit: defaultAiPreferences.toolRoundLimit,
      maxParallelSubagents: defaultAiPreferences.maxParallelSubagents,
      goalRunMaxTokens: defaultAiPreferences.goalRunMaxTokens,
      goalRunMaxRounds: defaultAiPreferences.goalRunMaxRounds,
      automaticModeHardStopMinutes: defaultAiPreferences.automaticModeHardStopMinutes,
      hiddenModelIds: defaultAiPreferences.hiddenModelIds,
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
