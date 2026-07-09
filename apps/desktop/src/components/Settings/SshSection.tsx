import { Loader2, RefreshCw, Server } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import { aspectCommands, type SshOverview } from "../../lib/tauri";
import { NumberSetting, SettingsGrid, ToggleSetting, type SaveState } from "./SettingsControls";

const STRICT_HOST_KEY = "ai.ssh.strictHostKey";
const CONNECT_TIMEOUT_KEY = "ai.ssh.connectTimeoutSecs";
const TIMEOUT_MIN = 1;
const TIMEOUT_MAX = 120;

function clampTimeout(value: number) {
  if (!Number.isFinite(value)) return 12;
  return Math.min(TIMEOUT_MAX, Math.max(TIMEOUT_MIN, Math.round(value)));
}

/**
 * Settings for Aspect's SSH integration (the Ssh* agent tools). SSH works out of the
 * box through the system OpenSSH client and the user's ~/.ssh/config РІР‚вЂќ the only
 * choices here are the host-key policy and the connect timeout.
 */
export function SshSection({ t }: { t: TranslateFn }) {
  const [overview, setOverview] = useState<SshOverview | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [strict, setStrict] = useState(false);
  const [connectTimeout, setConnectTimeout] = useState(12);
  const [saveState, setSaveState] = useState<SaveState>("idle");

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      const result = await aspectCommands.sshList();
      setOverview(result);
      setStrict(result.strictHostKey);
      setConnectTimeout(clampTimeout(result.connectTimeoutSecs));
    } catch {
      setOverview(null);
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const persist = useCallback(async (key: string, value: unknown) => {
    setSaveState("saving");
    try {
      await aspectCommands.settingsSet("user", key, value);
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

  const hosts = overview?.configHosts ?? [];

  return (
    <div className="aspect-research">
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
      <div className="aspect-research-field">
        <div className="aspect-research-status-row">
          <span className="aspect-research-label">
            <Server size={14} /> {t("settings.ssh.hostsLabel")}
          </span>
          <button type="button" className="aspect-research-test" onClick={() => void refresh()} disabled={refreshing}>
            {refreshing ? <Loader2 size={13} className="aspect-spin" /> : <RefreshCw size={13} />}
            {t("settings.ssh.refresh")}
          </button>
        </div>
        {hosts.length === 0 ? (
          <small className="aspect-research-hint">{t("settings.ssh.hostsEmpty")}</small>
        ) : (
          <div className="aspect-ssh-hosts">
            {hosts.map((host) => (
              <div key={host.alias} className="aspect-research-hint">
                <strong>{host.alias}</strong>
                {host.hostname ? ` РІвЂ вЂ™ ${host.user ? `${host.user}@` : ""}${host.hostname}${host.port ? `:${host.port}` : ""}` : ""}
              </div>
            ))}
          </div>
        )}
      </div>

      <p className="aspect-research-note">{t("settings.ssh.note")}</p>
    </div>
  );
}
