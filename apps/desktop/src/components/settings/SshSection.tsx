import { AlertTriangle, Loader2, RefreshCw, Server, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import { luxCommands, type SshOverview } from "../../lib/tauri";
import { NumberSetting, SaveIndicator, SettingsGrid, ToggleSetting, type SaveState } from "./SettingsControls";

const STRICT_HOST_KEY = "ai.ssh.strictHostKey";
const CONNECT_TIMEOUT_KEY = "ai.ssh.connectTimeoutSecs";
const TIMEOUT_MIN = 1;
const TIMEOUT_MAX = 120;

function clampTimeout(value: number) {
  if (!Number.isFinite(value)) return 12;
  return Math.min(TIMEOUT_MAX, Math.max(TIMEOUT_MIN, Math.round(value)));
}

/**
 * Settings for Lux's SSH integration (the Ssh* agent tools). SSH works out of the
 * box through the system OpenSSH client and the user's ~/.ssh/config — the only
 * choices here are the host-key policy and the connect timeout. The panel also
 * surfaces whether OpenSSH is detected and the hosts it can already reach.
 */
export function SshSection({ t }: { t: TranslateFn }) {
  const [overview, setOverview] = useState<SshOverview | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [strict, setStrict] = useState(false);
  const [connectTimeout, setConnectTimeout] = useState(12);
  const [saveState, setSaveState] = useState<SaveState>("idle");

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      const result = await luxCommands.sshList();
      setOverview(result);
      setStrict(result.strictHostKey);
      setConnectTimeout(clampTimeout(result.connectTimeoutSecs));
    } catch {
      setOverview(null);
    } finally {
      setLoaded(true);
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const persist = useCallback(async (key: string, value: unknown) => {
    setSaveState("saving");
    try {
      await luxCommands.settingsSet("user", key, value);
      setSaveState("saved");
      window.setTimeout(() => setSaveState("idle"), 1500);
    } catch {
      setSaveState("idle");
    }
  }, []);

  const onStrictChange = useCallback((next: boolean) => {
    setStrict(next);
    void persist(STRICT_HOST_KEY, next);
  }, [persist]);

  const onTimeoutChange = useCallback((next: number) => {
    const clamped = clampTimeout(next);
    setConnectTimeout(clamped);
    void persist(CONNECT_TIMEOUT_KEY, clamped);
  }, [persist]);

  const available = overview?.available ?? false;
  const hosts = overview?.configHosts ?? [];
  const sessionCount = overview?.sessions.length ?? 0;

  return (
    <div className="lux-research">
      <div className="lux-research-intro">
        {!loaded ? <Loader2 size={16} className="lux-spin" /> : available ? <ShieldCheck size={16} /> : <AlertTriangle size={16} />}
        <p>
          {!loaded
            ? t("settings.ssh.checking")
            : available
              ? t("settings.ssh.available", { version: overview?.version ?? "OpenSSH" })
              : t("settings.ssh.unavailable")}
        </p>
      </div>

      {loaded && available && (
        <p className="lux-research-note">{t("settings.ssh.summary", { hosts: hosts.length, sessions: sessionCount })}</p>
      )}

      <SettingsGrid>
        <ToggleSetting
          checked={strict}
          label={t("settings.ssh.strictLabel")}
          detail={t("settings.ssh.strictHint")}
          onChange={onStrictChange}
        />
        <NumberSetting
          value={connectTimeout}
          min={TIMEOUT_MIN}
          max={TIMEOUT_MAX}
          label={t("settings.ssh.timeoutLabel")}
          detail={t("settings.ssh.timeoutHint")}
          onChange={onTimeoutChange}
        />
      </SettingsGrid>
      <SaveIndicator state={saveState} t={t} />

      <div className="lux-research-field">
        <div className="lux-research-status-row">
          <span className="lux-research-label">
            <Server size={14} /> {t("settings.ssh.hostsLabel")}
          </span>
          <button type="button" className="lux-research-test" onClick={() => void refresh()} disabled={refreshing}>
            {refreshing ? <Loader2 size={13} className="lux-spin" /> : <RefreshCw size={13} />}
            {t("settings.ssh.refresh")}
          </button>
        </div>
        {hosts.length === 0 ? (
          <small className="lux-research-hint">{t("settings.ssh.hostsEmpty")}</small>
        ) : (
          <div className="lux-ssh-hosts">
            {hosts.map((host) => (
              <div key={host.alias} className="lux-research-hint">
                <strong>{host.alias}</strong>
                {host.hostname ? ` → ${host.user ? `${host.user}@` : ""}${host.hostname}${host.port ? `:${host.port}` : ""}` : ""}
              </div>
            ))}
          </div>
        )}
      </div>

      <p className="lux-research-note">{t("settings.ssh.note")}</p>
    </div>
  );
}
