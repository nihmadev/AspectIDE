import { Loader2, Plug, PlugZap, Plus, RefreshCw, Server, Trash2, Wrench } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import { luxCommands, MCP_SERVERS_KEY, type McpServerConfig, type McpServerStatus } from "../../lib/tauri";

const EMPTY_DRAFT = { name: "", command: "", args: "" };

const errorMessage = (cause: unknown) => (cause instanceof Error ? cause.message : String(cause));

/**
 * Settings for real-time MCP (Model Context Protocol) servers. Configure stdio
 * servers (command + args), connect them live, and the agent gets their tools
 * (namespaced `mcp__<server>__<tool>`) in Agent/Automatic mode automatically.
 *
 * Single source of truth: every mutation goes through the backend `mcpAdd` /
 * `mcpRemove` / `mcpEnable` commands, which atomically persist the config AND
 * update the live connection (the same path the agent's `McpManage` tool uses).
 * The UI never writes `ai.mcp.servers` directly — that bypass let config and live
 * tooling drift apart and swallowed failures. After each op we reload both the
 * persisted config and live status from the backend so the panel always mirrors
 * what the turn loop can actually call.
 */
export function McpSection({ t }: { t: TranslateFn }) {
  const [servers, setServers] = useState<McpServerConfig[]>([]);
  const [statuses, setStatuses] = useState<Record<string, McpServerStatus>>({});
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [draft, setDraft] = useState(EMPTY_DRAFT);
  const [error, setError] = useState<string | null>(null);

  // Reload the persisted config + live status together so the list and the
  // connection state can never diverge after a mutation. Config still comes from
  // settings (the backend's store of record); we only ever *read* it here.
  const reload = useCallback(async () => {
    const [configResult, statusResult] = await Promise.allSettled([
      luxCommands.settingsGet("user", MCP_SERVERS_KEY),
      luxCommands.mcpStatus(),
    ]);
    if (configResult.status === "fulfilled") {
      const value = configResult.value;
      setServers(Array.isArray(value?.value) ? (value.value as McpServerConfig[]) : []);
    }
    if (statusResult.status === "fulfilled") {
      setStatuses(Object.fromEntries(statusResult.value.map((status) => [status.id, status])));
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      const live = await luxCommands.mcpStatus();
      setStatuses(Object.fromEntries(live.map((status) => [status.id, status])));
    } catch {
      /* status is best-effort */
    }
  }, []);

  useEffect(() => {
    let active = true;
    void reload().finally(() => active && setLoaded(true));
    return () => {
      active = false;
    };
  }, [reload]);

  const addServer = useCallback(async () => {
    const name = draft.name.trim();
    const command = draft.command.trim();
    if (!name || !command) return;
    const id = `${name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "mcp"}-${Math.random().toString(36).slice(2, 6)}`;
    const args = draft.args.trim() ? draft.args.trim().split(/\s+/) : [];
    const config: McpServerConfig = { id, name, command, args, env: {}, enabled: true };
    setBusy(id);
    setError(null);
    try {
      // mcpAdd persists then connects atomically; a handshake failure comes back as
      // an error-state status (not a throw), so reload() surfaces it on the row.
      await luxCommands.mcpAdd(config);
      setDraft(EMPTY_DRAFT);
      await reload();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }, [draft, reload]);

  const toggleEnabled = useCallback(async (config: McpServerConfig) => {
    setBusy(config.id);
    setError(null);
    try {
      // mcpEnable persists the flag AND connects/disconnects in one backend step,
      // so the persisted config can't say "enabled" while the server is dead.
      await luxCommands.mcpEnable(config.id, !config.enabled);
      await reload();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }, [reload]);

  const reconnect = useCallback(async (config: McpServerConfig) => {
    // Reconnect is a live-only action (no config change), so mcpConnect is the right
    // call; we still reload status afterwards to reflect the result.
    setBusy(config.id);
    setError(null);
    try {
      await luxCommands.mcpConnect({ ...config, enabled: true });
      await refreshStatus();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }, [refreshStatus]);

  const removeServer = useCallback(async (config: McpServerConfig) => {
    setBusy(config.id);
    setError(null);
    try {
      // mcpRemove disconnects + deletes the persisted config in one backend step.
      await luxCommands.mcpRemove(config.id);
      await reload();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }, [reload]);

  const totalTools = useMemo(
    () => Object.values(statuses).reduce((sum, status) => sum + (status.tools?.length ?? 0), 0),
    [statuses],
  );

  return (
    <div className="lux-mcp">
      <div className="lux-mcp-intro">
        <Server size={16} />
        <p>{t("settings.mcp.intro")}</p>
        <button type="button" className="lux-mcp-refresh" onClick={() => void refreshStatus()} title={t("settings.mcp.refresh")}>
          <RefreshCw size={13} />
        </button>
      </div>

      {error && <p className="lux-mcp-error" role="alert">{error}</p>}

      {loaded && servers.length > 0 && (
        <ul className="lux-mcp-list">
          {servers.map((server) => {
            const status = statuses[server.id];
            const state = server.enabled ? status?.state ?? "disconnected" : "disconnected";
            const isBusy = busy === server.id;
            return (
              <li key={server.id} className="lux-mcp-item" data-state={state}>
                <div className="lux-mcp-item-main">
                  <span className="lux-mcp-dot" data-state={state} aria-hidden="true" />
                  <div className="lux-mcp-item-copy">
                    <strong>{server.name}</strong>
                    <code title={`${server.command} ${server.args.join(" ")}`}>
                      {server.command} {server.args.join(" ")}
                    </code>
                    {status?.error && <small className="lux-mcp-error">{status.error}</small>}
                  </div>
                  {state === "connected" && (
                    <span className="lux-mcp-tools" title={t("settings.mcp.toolsList", { tools: (status?.tools ?? []).map((tool) => tool.name).join(", ") || "—" })}>
                      <Wrench size={11} />
                      {status?.tools?.length ?? 0}
                    </span>
                  )}
                </div>
                <div className="lux-mcp-item-actions">
                  {isBusy ? (
                    <Loader2 size={14} className="lux-spin" />
                  ) : (
                    <>
                      {server.enabled && (
                        <button type="button" title={t("settings.mcp.reconnect")} onClick={() => void reconnect(server)}>
                          <RefreshCw size={13} />
                        </button>
                      )}
                      <button
                        type="button"
                        title={server.enabled ? t("settings.mcp.disable") : t("settings.mcp.enable")}
                        data-on={server.enabled || undefined}
                        onClick={() => void toggleEnabled(server)}
                      >
                        {server.enabled ? <PlugZap size={13} /> : <Plug size={13} />}
                      </button>
                      <button type="button" className="lux-mcp-danger" title={t("settings.mcp.remove")} onClick={() => void removeServer(server)}>
                        <Trash2 size={13} />
                      </button>
                    </>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}

      <div className="lux-mcp-add">
        <input
          className="lux-mcp-input"
          placeholder={t("settings.mcp.namePlaceholder")}
          value={draft.name}
          onChange={(event) => setDraft((prev) => ({ ...prev, name: event.target.value }))}
        />
        <input
          className="lux-mcp-input lux-mcp-input-command"
          placeholder={t("settings.mcp.commandPlaceholder")}
          value={draft.command}
          onChange={(event) => setDraft((prev) => ({ ...prev, command: event.target.value }))}
        />
        <input
          className="lux-mcp-input lux-mcp-input-args"
          placeholder={t("settings.mcp.argsPlaceholder")}
          value={draft.args}
          onChange={(event) => setDraft((prev) => ({ ...prev, args: event.target.value }))}
        />
        <button type="button" className="lux-mcp-add-button" onClick={() => void addServer()} disabled={!draft.name.trim() || !draft.command.trim()}>
          <Plus size={14} />
          {t("settings.mcp.add")}
        </button>
      </div>

      <p className="lux-mcp-note">{t("settings.mcp.note", { count: Object.keys(statuses).length, tools: totalTools })}</p>
    </div>
  );
}
