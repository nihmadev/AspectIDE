import { ChevronRight } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { defaultAiPreferences, type AiPreferences } from '../../lib/aspector/utils/preferences';
import type { TranslateFn } from '../../lib/i18n/useTranslation';
import { luxCommands, type AgentBrowserStatusResponse } from '../../lib/tauri/commands';
import { NumberSetting, SettingsGrid, SettingsPanel, TextSetting, ToggleSetting } from "./SettingsControls";

/** Advanced fields, in display order вЂ” also drives the disclosure's count badge
 *  and the "customized" dot (any key here off its default lights the dot). */
const ADVANCED_KEYS = [
  "agentBrowserContentBoundaries",
  "agentBrowserAllowFileAccess",
  "agentBrowserIgnoreHttpsErrors",
  "agentBrowserDashboardPort",
  "agentBrowserCommand",
  "agentBrowserAllowedDomains",
  "agentBrowserMaxOutput",
  "agentBrowserProfile",
  "agentBrowserStatePath",
  "agentBrowserProvider",
  "agentBrowserProxy",
] as const satisfies ReadonlyArray<keyof AiPreferences>;

/**
 * Settings for the Vercel agent-browser runtime: install/status banner, then the
 * everyday toggles (enable, headed, live preview, persist session) up front, with
 * the 11 rarely-touched knobs (domains, provider, proxy, ports, paths, вЂ¦) tucked
 * behind an "Advanced" disclosure so the common path isn't buried in a wall of
 * fields. The disclosure's open/closed state is intentionally ephemeral (not a
 * persisted preference) вЂ” it's a viewing convenience, not a setting.
 */
export function AgentBrowserSection({ onChange, preferences, t }: { onChange: (patch: Partial<AiPreferences>) => void; preferences: AiPreferences; t: TranslateFn }) {
  const [status, setStatus] = useState<AgentBrowserStatusResponse | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [installResult, setInstallResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const refreshStatus = useCallback(async (options: { full?: boolean } = {}) => {
    setChecking(true);
    try {
      const response = await luxCommands.agentBrowserStatus({
        commandPath: preferences.agentBrowserCommand.trim() || undefined,
        skipAutoUpdate: true,
        lightweight: options.full ? false : true,
      });
      if (response.updatePerformed) {
        void import("../../lib/agent-browser/skills-cache").then(({ invalidateAgentBrowserSkillsCache }) => invalidateAgentBrowserSkillsCache());
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

  const advancedCustomized = useMemo(
    () => ADVANCED_KEYS.some((key) => preferences[key] !== defaultAiPreferences[key]),
    [preferences],
  );

  return (
    // Title omitted: it would repeat the section header directly above. The
    // description stays вЂ” it explains the agent-browser runtime, not the nav.
    <SettingsPanel description={t("settings.agentBrowser.description")}>
      <section className="settings-banner" data-state={diagnosticState}>
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
            disabled={checking || installing}
            onClick={() => {
              setInstalling(true);
              setInstallResult(null);
              void luxCommands.agentBrowserInstall({
                commandPath: preferences.agentBrowserCommand.trim() || null,
                withDeps: false,
              }).then((response) => {
                // The backend reports partial failures via success:false + detail
                // (e.g. npm missing, network down). Surface that text verbatim вЂ”
                // the old handler swallowed it and the button looked like a no-op.
                setInstallResult(response.success
                  ? { ok: true, text: t("settings.agentBrowser.install.success", { path: response.commandPath ?? "agent-browser" }) }
                  : { ok: false, text: t("settings.agentBrowser.install.failed", { detail: response.detail }) });
                if (response.success) {
                  void import("../../lib/agent-browser/skills-cache").then(({ invalidateAgentBrowserSkillsCache }) => invalidateAgentBrowserSkillsCache());
                }
              }).catch((error: unknown) => {
                setInstallResult({
                  ok: false,
                  text: t("settings.agentBrowser.install.failed", { detail: error instanceof Error ? error.message : String(error) }),
                });
              }).finally(() => {
                setInstalling(false);
                void refreshStatus();
              });
            }}
          >
            {installing ? t("settings.agentBrowser.status.checking") : t("settings.agentBrowser.install.action")}
          </button>
        </div>
      </section>
      {installing && (
        <p className="settings-banner agent-browser-install-note" data-state="checking" role="status">
          {t("settings.agentBrowser.install.running")}
        </p>
      )}
      {installResult && !installing && (
        <p className="settings-banner agent-browser-install-note" data-state={installResult.ok ? "ok" : "error"} role="alert">
          {installResult.text}
        </p>
      )}
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
      </SettingsGrid>

      <div className="settings-advanced">
        <button
          type="button"
          className="settings-advanced-toggle"
          aria-expanded={advancedOpen}
          onClick={() => setAdvancedOpen((open) => !open)}
        >
          <ChevronRight size={14} className="settings-advanced-caret" data-open={advancedOpen || undefined} />
          <span className="settings-advanced-label">{t("settings.agentBrowser.advanced.label")}</span>
          <span className="settings-advanced-badge">{t("settings.agentBrowser.advanced.count", { count: ADVANCED_KEYS.length })}</span>
          {advancedCustomized && (
            <span className="settings-advanced-dot" title={t("settings.agentBrowser.advanced.customized")} aria-label={t("settings.agentBrowser.advanced.customized")} />
          )}
        </button>
        <div className="settings-advanced-content" data-open={advancedOpen || undefined}>
          <div className="settings-advanced-content-inner">
            <SettingsGrid>
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
          </div>
        </div>
      </div>
    </SettingsPanel>
  );
}
