import { Check, ChevronLeft, ChevronRight, Cpu, GripVertical, Plus, RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  AI_PROVIDER_PRESETS,
  buildLocalProxyBaseUrl,
  createAiEffortConfig,
  createAiModelConfig,
  createAiProviderConfig,
  getAiModel,
  getAiProvider,
  type AiEffortConfig,
  type AiModelConfig,
  type AiPreferences,
  type AiProviderConfig,
  type AiProviderPresetId,
  type AiProviderProtocol,
} from "../../lib/aiPreferences";
import { formatCompactTokens } from "../../lib/aiChatContextUsage";
import {
  MAX_CONTEXT_AUTO_COMPACT_THRESHOLD,
  MIN_CONTEXT_AUTO_COMPACT_THRESHOLD,
  resolveModelContextTokens,
} from "../../lib/aiModelContext";
import { fetchProviderModelConfigs, isFreeModelId, mergeRefreshedModels } from "../../lib/aiProviderModels";
import { aspectCommands, type AiProviderDiagnosticResponse } from "../../lib/tauri";
import type { MessageKey } from "../../lib/i18n";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import { CompactDropdown } from "../CompactDropdown";
import { NumberSetting, SelectSetting, SettingsGrid, SettingsPanel, TextSetting } from "./SettingsControls";

// Provider preset id в†’ localized description key. Brand names stay verbatim; only the
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
  together: "settings.providerPreset.together.description",
  fireworks: "settings.providerPreset.fireworks.description",
  cerebras: "settings.providerPreset.cerebras.description",
  moonshot: "settings.providerPreset.moonshot.description",
  zai: "settings.providerPreset.zai.description",
  minimax: "settings.providerPreset.minimax.description",
  alibaba: "settings.providerPreset.alibaba.description",
  huggingface: "settings.providerPreset.huggingface.description",
  "github-models": "settings.providerPreset.githubModels.description",
  "github-copilot": "settings.providerPreset.githubCopilot.description",
  "vercel-gateway": "settings.providerPreset.vercelGateway.description",
  nvidia: "settings.providerPreset.nvidia.description",
  deepinfra: "settings.providerPreset.deepinfra.description",
  novita: "settings.providerPreset.novita.description",
  perplexity: "settings.providerPreset.perplexity.description",
  siliconflow: "settings.providerPreset.siliconflow.description",
  nebius: "settings.providerPreset.nebius.description",
  baseten: "settings.providerPreset.baseten.description",
  venice: "settings.providerPreset.venice.description",
  "cloudflare-workers-ai": "settings.providerPreset.cloudflareWorkersAi.description",
  "meta-llama": "settings.providerPreset.metaLlama.description",
  "ollama-cloud": "settings.providerPreset.ollamaCloud.description",
  ollama: "settings.providerPreset.ollama.description",
  "lm-studio": "settings.providerPreset.lmStudio.description",
  "local-proxy": "settings.providerPreset.localProxy.description",
  custom: "settings.providerPreset.custom.description",
};

/** Snap a user-entered override percent to the engine's valid auto-compact band
 *  (50вЂ“95%) and return the 0..1 fraction вЂ” so the stored value equals what the
 *  compaction pipeline actually applies (its floor is 50%). */
function clampOverridePercent(percent: number): number {
  const fraction = percent / 100;
  return Math.min(MAX_CONTEXT_AUTO_COMPACT_THRESHOLD, Math.max(MIN_CONTEXT_AUTO_COMPACT_THRESHOLD, fraction));
}

function providerPresetDescription(providerType: string, t: TranslateFn) {
  const descriptionKey = PROVIDER_PRESET_DESCRIPTION_KEYS[providerType];
  return descriptionKey ? t(descriptionKey) : t("settings.providers.customProvider");
}

// Provider management uses a master-to-detail flow: a list of provider tiles, and a focused
// editor screen reached by opening one. `openProviderId === null` means the list is shown.
export function AiProvidersSection({ onChange, preferences, t }: { onChange: (patch: Partial<AiPreferences>) => void; preferences: AiPreferences; t: TranslateFn }) {
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
    // No panel title/description: the section header above renders the same
    // "Providers" title + description, so repeating them here reads as a glitch.
    <SettingsPanel>
      <div className="provider-create-row">
        <label className="provider-template-select">
          <span className="provider-template-label">{t("settings.providers.template")}</span>
          <CompactDropdown
            className="provider-template-dropdown"
            label={t("settings.providers.template")}
            value={providerPresetId}
            options={AI_PROVIDER_PRESETS.map((preset) => ({ label: preset.name, value: preset.id }))}
            onChange={(value) => setProviderPresetId(value as AiProviderPresetId)}
          />
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
  // Effort drag-reorder: which row is being dragged and where it would land.
  // The row is draggable only when armed from the grip handle, so text selection
  // inside the label input never starts an accidental drag.
  const [effortDrag, setEffortDrag] = useState<{ id: string; overId: string | null; after: boolean } | null>(null);
  const effortDragArmedRef = useRef(false);
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
  // placeholder, or (b) a free model has a sub-1M context вЂ” the signature of an
  // earlier buggy fetch that inferred e.g. DeepSeek's 128k for "вЂ¦-free". A single
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
      // Reasoning models reject an explicit `temperature` and the legacy
      // `max_tokens` name (OpenAI o-series / gpt-5 return HTTP 400), and burn
      // tokens thinking before they emit text вЂ” give them headroom via
      // `max_completion_tokens` and no temperature; standard models keep the
      // tight, cheap probe.
      const isReasoningModel = editingModel.effortLevels.length > 0;
      const result = await aspectCommands.aiProviderDiagnostic({
        baseUrl: provider.baseUrl,
        apiKey: provider.apiKey || null,
        protocol: provider.protocol,
        payload: {
          model: editingModel.alias || editingModel.id,
          messages: [{ role: "user", content: "Reply with OK." }],
          stream: false,
          ...(isReasoningModel
            ? { max_completion_tokens: 256 }
            : { max_tokens: 8, temperature: 0 }),
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
          {/* commitOnBlur on every live-runtime field: provider name/URL/key and the
              local-proxy endpoint parts feed directly into in-flight chat sends, so a
              half-typed alias/URL/key must not reach the runtime mid-typing. The value
              is committed on blur/Enter once it is complete. */}
          <TextSetting label={t("settings.providers.providerName.label")} value={provider.name} commitOnBlur onChange={(name) => updateEditingProvider({ name })} />
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
              <TextSetting label={t("settings.providers.localIp.label")} value={provider.localHost} placeholder="127.0.0.1" commitOnBlur onChange={(localHost) => updateLocalProxyEndpoint({ localHost })} />
              <TextSetting label={t("settings.providers.port.label")} value={provider.localPort} placeholder="8080" commitOnBlur onChange={(localPort) => updateLocalProxyEndpoint({ localPort })} />
              <TextSetting label={t("settings.providers.apiPath.label")} value={provider.localPath} placeholder="/v1" commitOnBlur onChange={(localPath) => updateLocalProxyEndpoint({ localPath })} />
              <TextSetting label={t("settings.providers.resolvedUrl.label")} value={provider.baseUrl} onChange={() => undefined} readOnly wide />
            </>
          ) : (
            <TextSetting label={t("settings.providers.baseUrl.label")} value={provider.baseUrl} commitOnBlur onChange={(baseUrl) => updateEditingProvider({ baseUrl })} wide />
          )}
          <TextSetting label={t("settings.providers.apiKey.label")} value={provider.apiKey} commitOnBlur onChange={(apiKey) => updateEditingProvider({ apiKey })} password wide />
          {provider.protocol !== "anthropic" && (
            <TextSetting
              label={t("settings.providers.embeddingModel.label")}
              detail={t("settings.providers.embeddingModel.detail")}
              value={provider.embeddingModel}
              placeholder="text-embedding-3-small"
              commitOnBlur
              onChange={(embeddingModel) => updateEditingProvider({ embeddingModel })}
              wide
            />
          )}
          <NumberSetting
            label={t("settings.providers.providerAutoCompact.label")}
            detail={t("settings.providers.providerAutoCompact.detail")}
            value={Math.round((provider.contextAutoCompactThreshold ?? 0) * 100)}
            min={0}
            max={95}
            step={5}
            onChange={(percent) => updateEditingProvider({ contextAutoCompactThreshold: percent > 0 ? clampOverridePercent(percent) : null })}
          />
        </SettingsGrid>
      </SettingsPanel>

      <SettingsPanel title={t("settings.providers.models.title")} description={t("settings.providers.models.description")}>
        <div className="provider-model-manager">
          <div className="provider-model-left">
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

            <div className="effort-editor">
              <div className="effort-editor-head">
                <strong>{t("settings.providers.thinkingEffort")}</strong>
                <button type="button" onClick={() => {
                  const nextEffort = createAiEffortConfig(editingModel.effortLevels);
                  updateEfforts([...editingModel.effortLevels, nextEffort], nextEffort.id);
                }}><Plus size={14} /> {t("settings.providers.addEffort")}</button>
              </div>
              {editingModel.effortLevels.length === 0 ? <p>{t("settings.providers.noEffortSelector")}</p> : editingModel.effortLevels.map((effort) => (
                <div
                  className="effort-row"
                  key={effort.id}
                  draggable={editingModel.effortLevels.length > 1}
                  data-dragging={effortDrag?.id === effort.id || undefined}
                  data-drop={effortDrag && effortDrag.overId === effort.id && effortDrag.id !== effort.id
                    ? (effortDrag.after ? "after" : "before")
                    : undefined}
                  onDragStart={(event) => {
                    if (!effortDragArmedRef.current) {
                      event.preventDefault();
                      return;
                    }
                    effortDragArmedRef.current = false;
                    event.dataTransfer.effectAllowed = "move";
                    setEffortDrag({ id: effort.id, overId: null, after: false });
                  }}
                  onDragOver={(event) => {
                    if (!effortDrag) return;
                    event.preventDefault();
                    event.dataTransfer.dropEffect = "move";
                    const rect = event.currentTarget.getBoundingClientRect();
                    const after = event.clientY > rect.top + rect.height / 2;
                    setEffortDrag((current) => current && (current.overId !== effort.id || current.after !== after)
                      ? { ...current, overId: effort.id, after }
                      : current);
                  }}
                  onDrop={(event) => {
                    event.preventDefault();
                    if (effortDrag && effortDrag.id !== effort.id) {
                      updateEfforts(
                        reorderEffortLevels(editingModel.effortLevels, effortDrag.id, effort.id, effortDrag.after),
                        preferences.selectedEffortId,
                      );
                    }
                    effortDragArmedRef.current = false;
                    setEffortDrag(null);
                  }}
                  onDragEnd={() => {
                    effortDragArmedRef.current = false;
                    setEffortDrag(null);
                  }}
                >
                  <span
                    className="effort-drag-handle"
                    title={t("settings.providers.effortDragHint")}
                    aria-hidden="true"
                    data-disabled={editingModel.effortLevels.length < 2 || undefined}
                    onPointerDown={() => {
                      effortDragArmedRef.current = true;
                      const disarm = () => { effortDragArmedRef.current = false; };
                      window.addEventListener("pointerup", disarm, { once: true });
                      window.addEventListener("pointercancel", disarm, { once: true });
                    }}
                  >
                    <GripVertical size={14} />
                  </span>
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
              {/* Name/alias also feed live sends (alias is the wire model id) вЂ” blur-commit. */}
              <TextSetting label={t("settings.providers.modelName.label")} value={editingModel.name} commitOnBlur onChange={(name) => updateEditingModel({ name })} />
              <TextSetting label={t("settings.providers.modelAlias.label")} value={editingModel.alias} commitOnBlur onChange={(alias) => updateEditingModel({ alias })} />
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
              <SelectSetting<AiProviderProtocol | "inherit">
                label={t("settings.providers.modelProtocol.label")}
                detail={t("settings.providers.modelProtocol.detail", { protocol: provider.protocol })}
                value={editingModel.protocol ?? "inherit"}
                options={[
                  { label: t("settings.providers.modelProtocol.inherit", { protocol: provider.protocol }), value: "inherit" },
                  { label: t("settings.providers.protocol.openaiCompatible"), value: "openai-compatible" },
                  { label: t("settings.providers.protocol.anthropic"), value: "anthropic" },
                  { label: t("settings.providers.protocol.google"), value: "google" },
                  { label: t("settings.providers.protocol.azureOpenai"), value: "azure-openai" },
                  { label: t("settings.providers.protocol.localProxy"), value: "local-proxy" },
                ]}
                onChange={(protocol) => updateEditingModel({ protocol: protocol === "inherit" ? null : protocol })}
                wide
              />
              <NumberSetting
                label={t("settings.providers.modelAutoCompact.label")}
                detail={t("settings.providers.modelAutoCompact.detail")}
                value={Math.round((editingModel.contextAutoCompactThreshold ?? 0) * 100)}
                min={0}
                max={95}
                step={5}
                // 0 = inherit; any real override is clamped to the engine's valid
                // 50вЂ“95% band so the stored value equals what actually applies
                // (the compaction floor is 50% вЂ” a lower number would silently snap).
                onChange={(percent) => updateEditingModel({ contextAutoCompactThreshold: percent > 0 ? clampOverridePercent(percent) : null })}
              />
            </SettingsGrid>
          </div>
        </div>
      </SettingsPanel>
    </div>
  );
}

/**
 * Move the dragged effort next to the drop target: before it (`after` false) or
 * after it. Returns the input array unchanged for no-op drops (unknown ids,
 * self-drop), so callers can pass the result straight to state.
 */
function reorderEffortLevels(
  levels: AiEffortConfig[],
  dragId: string,
  targetId: string,
  after: boolean,
): AiEffortConfig[] {
  if (dragId === targetId) return levels;
  const from = levels.findIndex((level) => level.id === dragId);
  if (from < 0 || !levels.some((level) => level.id === targetId)) return levels;
  const next = [...levels];
  const [moved] = next.splice(from, 1);
  if (!moved) return levels;
  const insertAt = next.findIndex((level) => level.id === targetId) + (after ? 1 : 0);
  next.splice(insertAt, 0, moved);
  return next;
}
