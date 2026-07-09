import { Check, Loader2, Pencil, Plug, PlugZap, Plus, RefreshCw, Server, Trash2, Wrench, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import { aspectCommands, MCP_SERVERS_KEY, type McpServerConfig, type McpServerStatus } from "../../lib/tauri";

const EMPTY_DRAFT = { name: "", command: "", args: "", env: "" };

const errorMessage = (cause: unknown) => (cause instanceof Error ? cause.message : String(cause));

/** `env` record Р Р†РІР‚В РІР‚в„ў editable "KEY=VALUE per line" text. */
function envToText(env: Record<string, string> | undefined) {
  return Object.entries(env ?? {})
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
}

/** Parse "KEY=VALUE per line" text; returns the offending line on malformed input. */
function parseEnvText(text: string): { env: Record<string, string>; invalidLine: string | null } {
  const env: Record<string, string> = {};
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) continue;
    const separator = line.indexOf("=");
    if (separator <= 0) return { env, invalidLine: line };
    env[line.slice(0, separator).trim()] = line.slice(separator + 1).trim();
  }
  return { env, invalidLine: null };
}

/**
 * Settings for real-time MCP (Model Context Protocol) servers. Configure stdio
 * servers (command + args + env), connect them live, and the agent gets their
 * tools (namespaced `mcp__<server>__<tool>`) in Agent/Automatic mode automatically.
 *
 * Single source of truth: every mutation goes through the backend `mcpAdd` /
 * `mcpRemove` / `mcpEnable` commands, which atomically persist the config AND
 * update the live connection (the same path the agent's `McpManage` tool uses).
 * Editing reuses `mcpAdd`'s upsert-by-id semantics: saving an edited server
 * persists the new command/args/env and reconnects in one backend step.
 * The UI never writes `ai.mcp.servers` directly Р Р†Р вЂљРІР‚Сњ that bypass let config and live
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
  const [editingId, setEditingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Reload the persisted config + live status together so the list and the
  // connection state can never diverge after a mutation. Config still comes from
  // settings (the backend's store of record); we only ever *read* it here.
  const reload = useCallback(async () => {
    const [configResult, statusResult] = await Promise.allSettled([
      aspectCommands.settingsGet("user", MCP_SERVERS_KEY),
      aspectCommands.mcpStatus(),
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
      const live = await aspectCommands.mcpStatus();
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

  const resetDraft = useCallback(() => {
    setDraft(EMPTY_DRAFT);
    setEditingId(null);
  }, []);

  const beginEdit = useCallback((config: McpServerConfig) => {
    setEditingId(config.id);
    setError(null);
    setDraft({
      name: config.name,
      command: config.command,
      args: config.args.join(" "),
      env: envToText(config.env),
    });
  }, []);

  // Add and edit share one path: mcpAdd is an upsert by id, so saving an edit
  // with the original id atomically persists the changes and reconnects.
  const saveServer = useCallback(async () => {
    const name = draft.name.trim();
    const command = draft.command.trim();
    if (!name || !command) return;
    const { env, invalidLine } = parseEnvText(draft.env);
    if (invalidLine) {
      setError(t("settings.mcp.envInvalid", { line: invalidLine }));
      return;
    }
    const existing = editingId ? servers.find((server) => server.id === editingId) : undefined;
    const id = existing?.id
      ?? `${name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "mcp"}-${Math.random().toString(36).slice(2, 6)}`;
    const args = draft.args.trim() ? draft.args.trim().split(/\s+/) : [];
    // An edit keeps the server's enabled flag; a new server starts enabled.
    const config: McpServerConfig = { id, name, command, args, env, enabled: existing?.enabled ?? true };
    setBusy(id);
    setError(null);
    try {
      // mcpAdd persists then connects atomically; a handshake failure comes back as
      // an error-state status (not a throw), so reload() surfaces it on the row.
      await aspectCommands.mcpAdd(config);
      resetDraft();
      await reload();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }, [draft, editingId, reload, resetDraft, servers, t]);

  const toggleEnabled = useCallback(async (config: McpServerConfig) => {
    setBusy(config.id);
    setError(null);
    try {
      // mcpEnable persists the flag AND connects/disconnects in one backend step,
      // so the persisted config can't say "enabled" while the server is dead.
      await aspectCommands.mcpEnable(config.id, !config.enabled);
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
      await aspectCommands.mcpConnect({ ...config, enabled: true });
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
      await aspectCommands.mcpRemove(config.id);
      if (editingId === config.id) resetDraft();
      await reload();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }, [editingId, reload, resetDraft]);

  const totalTools = useMemo(
    () => Object.values(statuses).reduce((sum, status) => sum + (status.tools?.length ?? 0), 0),
    [statuses],
  );

  const editingName = editingId ? servers.find((server) => server.id === editingId)?.name ?? "" : "";

  return (
    <div className="aspect-mcp">
      <div className="aspect-mcp-intro">
        <Server size={16} />
        <p>{t("settings.mcp.intro")}</p>
        <button type="button" className="aspect-mcp-refresh" onClick={() => void refreshStatus()} title={t("settings.mcp.refresh")}>
          <RefreshCw size={13} />
        </button>
      </div>

      {error && <p className="aspect-mcp-error" role="alert">{error}</p>}

      {loaded && servers.length > 0 && (
        <ul className="aspect-mcp-list">
          {servers.map((server) => {
            const status = statuses[server.id];
            const state = server.enabled ? status?.state ?? "disconnected" : "disconnected";
            const isBusy = busy === server.id;
            const envCount = Object.keys(server.env ?? {}).length;
            return (
              <li key={server.id} className="aspect-mcp-item" data-state={state} data-editing={editingId === server.id || undefined}>
                <div className="aspect-mcp-item-main">
                  <span className="aspect-mcp-dot" data-state={state} aria-hidden="true" />
                  <div className="aspect-mcp-item-copy">
                    <strong>{server.name}</strong>
                    <code title={`${server.command} ${server.args.join(" ")}`}>
                      {server.command} {server.args.join(" ")}
                    </code>
                    {envCount > 0 && <small className="aspect-mcp-env-note">{t("settings.mcp.envCount", { count: envCount })}</small>}
                    {status?.error && <small className="aspect-mcp-error">{status.error}</small>}
                  </div>
                  {state === "connected" && (
                    <span className="aspect-mcp-tools" title={t("settings.mcp.toolsList", { tools: (status?.tools ?? []).map((tool) => tool.name).join(", ") || "Р Р†Р вЂљРІР‚Сњ" })}>
                      <Wrench size={11} />
                      {status?.tools?.length ?? 0}
                    </span>
                  )}
                </div>
                <div className="aspect-mcp-item-actions">
                  {isBusy ? (
                    <Loader2 size={14} className="aspect-spin" />
                  ) : (
                    <>
                      <button type="button" title={t("settings.mcp.edit")} onClick={() => beginEdit(server)}>
                        <Pencil size={13} />
                      </button>
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
                      <button type="button" className="aspect-mcp-danger" title={t("settings.mcp.remove")} onClick={() => void removeServer(server)}>
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

      {editingId && (
        <p className="aspect-mcp-editing-note" role="status">
          <Pencil size={12} aria-hidden="true" />
          {t("settings.mcp.editing", { name: editingName })}
        </p>
      )}

      <div className="aspect-mcp-add" data-editing={Boolean(editingId) || undefined}>
        <input
          className="aspect-mcp-input"
          placeholder={t("settings.mcp.namePlaceholder")}
          value={draft.name}
          onChange={(event) => setDraft((prev) => ({ ...prev, name: event.target.value }))}
        />
        <input
          className="aspect-mcp-input aspect-mcp-input-command"
          placeholder={t("settings.mcp.commandPlaceholder")}
          value={draft.command}
          onChange={(event) => setDraft((prev) => ({ ...prev, command: event.target.value }))}
        />
        <input
          className="aspect-mcp-input aspect-mcp-input-args"
          placeholder={t("settings.mcp.argsPlaceholder")}
          value={draft.args}
          onChange={(event) => setDraft((prev) => ({ ...prev, args: event.target.value }))}
        />
        <div className="aspect-mcp-add-actions">
          {editingId && (
            <button type="button" className="aspect-mcp-add-button" onClick={resetDraft} title={t("settings.mcp.cancelEdit")}>
              <X size={14} />
            </button>
          )}
          <button
            type="button"
            className="aspect-mcp-add-button aspect-mcp-save-button"
            onClick={() => void saveServer()}
            disabled={!draft.name.trim() || !draft.command.trim() || busy !== null}
          >
            {editingId ? <Check size={14} /> : <Plus size={14} />}
            {editingId ? t("settings.mcp.save") : t("settings.mcp.add")}
          </button>
        </div>
        <textarea
          className="aspect-mcp-input aspect-mcp-env-input"
          rows={2}
          placeholder={t("settings.mcp.envPlaceholder")}
          value={draft.env}
          onChange={(event) => setDraft((prev) => ({ ...prev, env: event.target.value }))}
          spellCheck={false}
        />
      </div>

      <p className="aspect-mcp-note">{t("settings.mcp.note", { count: Object.keys(statuses).length, tools: totalTools })}</p>
    </div>
  );
}
