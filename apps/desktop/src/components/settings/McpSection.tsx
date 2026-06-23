import { Loader2, Plug, PlugZap, Plus, RefreshCw, Server, Trash2, Wrench } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import { luxCommands, MCP_SERVERS_KEY, type McpServerConfig, type McpServerStatus } from "../../lib/tauri";

const EMPTY_DRAFT = { name: "", command: "", args: "" };

/**
 * Settings for real-time MCP (Model Context Protocol) servers. Configure stdio
 * servers (command + args), connect them live, and the agent gets their tools
 * (namespaced `mcp__<server>__<tool>`) in Agent/Automatic mode automatically.
 */
export function McpSection({ t }: { t: TranslateFn }) {
  const [servers, setServers] = useState<McpServerConfig[]>([]);
  const [statuses, setStatuses] = useState<Record<string, McpServerStatus>>({});
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [draft, setDraft] = useState(EMPTY_DRAFT);

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
    void luxCommands
      .settingsGet("user", MCP_SERVERS_KEY)
      .then((value) => {
        if (!active) return;
        const stored = Array.isArray(value?.value) ? (value.value as McpServerConfig[]) : [];
        setServers(stored);
      })
      .catch(() => undefined)
      .finally(() => active && setLoaded(true));
    void refreshStatus();
    return () => {
      active = false;
    };
  }, [refreshStatus]);

  const persist = useCallback(async (next: McpServerConfig[]) => {
    setServers(next);
    await luxCommands.settingsSet("user", MCP_SERVERS_KEY, next).catch(() => undefined);
  }, []);

  const addServer = useCallback(async () => {
    const name = draft.name.trim();
    const command = draft.command.trim();
    if (!name || !command) return;
    const id = `${name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "mcp"}-${Math.random().toString(36).slice(2, 6)}`;
    const args = draft.args.trim() ? draft.args.trim().split(/\s+/) : [];
    const config: McpServerConfig = { id, name, command, args, env: {}, enabled: true };
    await persist([...servers, config]);
    setDraft(EMPTY_DRAFT);
    setBusy(id);
    try {
      const status = await luxCommands.mcpConnect(config);
      setStatuses((prev) => ({ ...prev, [id]: status }));
    } catch {
      /* surfaced via status */
    } finally {
      setBusy(null);
    }
  }, [draft, persist, servers]);

  const toggleEnabled = useCallback(async (config: McpServerConfig) => {
    const next = servers.map((server) => (server.id === config.id ? { ...server, enabled: !server.enabled } : server));
    await persist(next);
    setBusy(config.id);
    try {
      if (config.enabled) {
        await luxCommands.mcpDisconnect(config.id);
        setStatuses((prev) => {
          const copy = { ...prev };
          delete copy[config.id];
          return copy;
        });
      } else {
        const status = await luxCommands.mcpConnect({ ...config, enabled: true });
        setStatuses((prev) => ({ ...prev, [config.id]: status }));
      }
    } finally {
      setBusy(null);
    }
  }, [persist, servers]);

  const reconnect = useCallback(async (config: McpServerConfig) => {
    setBusy(config.id);
    try {
      const status = await luxCommands.mcpConnect({ ...config, enabled: true });
      setStatuses((prev) => ({ ...prev, [config.id]: status }));
    } finally {
      setBusy(null);
    }
  }, []);

  const removeServer = useCallback(async (config: McpServerConfig) => {
    await luxCommands.mcpDisconnect(config.id).catch(() => undefined);
    await persist(servers.filter((server) => server.id !== config.id));
    setStatuses((prev) => {
      const copy = { ...prev };
      delete copy[config.id];
      return copy;
    });
  }, [persist, servers]);

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
