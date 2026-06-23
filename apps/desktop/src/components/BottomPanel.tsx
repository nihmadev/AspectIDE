import { ChevronDown, ChevronUp, Eraser, Filter, ListFilter, Plus, TerminalSquare, Trash2, X } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { displayPath } from "../lib/fileTree";
import type { MessageKey } from "../lib/i18n";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
import { useLuxStore, type BottomPanelTab } from "../lib/store";
import { isTauriRuntime, luxCommands } from "../lib/tauri";
import type { TerminalSessionInfo, WorkspaceDiagnostic } from "../lib/types";
import { XtermTerminal } from "./XtermTerminal";

const tabs: Array<{ id: BottomPanelTab; labelKey: MessageKey }> = [
  { id: "problems", labelKey: "panel.tab.problems" },
  { id: "output", labelKey: "panel.tab.output" },
  { id: "terminal", labelKey: "panel.tab.terminal" },
];

type OutputEntry = {
  channel: string;
  level: "info" | "warn" | "error";
  time: string;
  text: string;
};

type BottomPanelProps = {
  isMaximized?: boolean;
  onToggleMaximized?: () => void;
};

export function BottomPanel({ isMaximized = false, onToggleMaximized }: BottomPanelProps) {
  const { t } = useTranslation();
  const terminal = useLuxStore((state) => state.terminal);
  const terminalSessions = useLuxStore((state) => state.terminalSessions);
  const activeTerminalId = useLuxStore((state) => state.activeTerminalId);
  const terminalOutputBuffers = useLuxStore((state) => state.terminalOutputBuffers);
  const upsertTerminalSession = useLuxStore((state) => state.upsertTerminalSession);
  const setActiveTerminal = useLuxStore((state) => state.setActiveTerminal);
  const closeTerminalSession = useLuxStore((state) => state.closeTerminalSession);
  const clearTerminalOutput = useLuxStore((state) => state.clearTerminalOutput);
  const activeTab = useLuxStore((state) => state.bottomPanelTab);
  const setActiveTab = useLuxStore((state) => state.setBottomPanelTab);
  const setBottomPanelOpen = useLuxStore((state) => state.setBottomPanelOpen);
  const diagnosticsByPath = useLuxStore((state) => state.diagnosticsByPath);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);
  const setPendingEditorReveal = useLuxStore((state) => state.setPendingEditorReveal);

  const [problemsFilter, setProblemsFilter] = useState("");
  const [outputFilter, setOutputFilter] = useState("");
  const [outputChannel, setOutputChannel] = useState("All Channels");
  const [outputEntries, setOutputEntries] = useState<OutputEntry[]>([]);
  const [terminalError, setTerminalError] = useState<string | null>(null);
  const [problemOpenError, setProblemOpenError] = useState<string | null>(null);
  // Per-session clear tokens so each xterm clears independently
  const [terminalClearTokens, setTerminalClearTokens] = useState<Record<string, number>>({});
  // Track which session xterms are mounted (keep them alive once created so
  // session switching doesn't replay raw bytes into a fresh terminal, which
  // garbles ANSI state).
  const [mountedSessionIds, setMountedSessionIds] = useState<Set<string>>(new Set());

  // Keep all active sessions mounted
  useEffect(() => {
    setMountedSessionIds((prev) => {
      const next = new Set(prev);
      for (const session of terminalSessions) next.add(session.id);
      return next;
    });
  }, [terminalSessions]);

  const terminalMutation = useMutation({
    mutationFn: async () => {
      if (!isTauriRuntime()) return null;
      const created = await luxCommands.terminalCreate(undefined, undefined, undefined, undefined);
      return created;
    },
    onSuccess: (session) => {
      setTerminalError(null);
      if (session) {
        upsertTerminalSession(session, true);
        setMountedSessionIds((prev) => new Set(prev).add(session.id));
      }
    },
    onError: (error) => setTerminalError(readErrorMessage(error)),
  });

  // Auto-spawn the first terminal when the user opens the Terminal tab and there
  // are no sessions yet (keeps the "open terminal → shell is ready" UX). Guarded
  // by a ref so it fires at most once per "empty" state; resets when sessions exist
  // so closing every terminal and reopening the tab spawns a fresh one.
  const autoSpawnedRef = useRef(false);
  const spawnTerminal = terminalMutation.mutate;
  const spawnPending = terminalMutation.isPending;
  useEffect(() => {
    if (activeTab !== "terminal") return;
    if (terminalSessions.length > 0) { autoSpawnedRef.current = false; return; }
    if (autoSpawnedRef.current || spawnPending) return;
    if (!isTauriRuntime()) return;
    autoSpawnedRef.current = true;
    spawnTerminal();
  }, [activeTab, terminalSessions.length, spawnTerminal, spawnPending]);

  const clearActiveTerminal = useCallback(() => {
    if (activeTerminalId) {
      clearTerminalOutput(activeTerminalId);
      setTerminalClearTokens((prev) => ({ ...prev, [activeTerminalId]: (prev[activeTerminalId] ?? 0) + 1 }));
    }
  }, [activeTerminalId, clearTerminalOutput]);

  const closeActiveTerminal = useCallback(() => {
    if (!terminal) return;
    const terminalId = terminal.id;
    setMountedSessionIds((prev) => {
      const next = new Set(prev);
      next.delete(terminalId);
      return next;
    });
    if (isTauriRuntime()) {
      void luxCommands.terminalClose(terminalId).catch(() => undefined).finally(() => closeTerminalSession(terminalId));
    } else {
      closeTerminalSession(terminalId);
    }
  }, [terminal, closeTerminalSession]);

  const openProblemMutation = useMutation({
    mutationFn: async (problem: WorkspaceDiagnostic) => ({ problem, document: await luxCommands.editorOpenFile(problem.path) }),
    onSuccess: ({ document, problem }) => {
      setProblemOpenError(null);
      upsertDocument(document);
      setPendingEditorReveal({ documentId: document.id, line: problem.line, column: problem.column });
    },
    onError: (error) => setProblemOpenError(readErrorMessage(error)),
  });

  const filteredProblems = useMemo(() => {
    const diagnostics = Object.values(diagnosticsByPath).flat();
    const query = problemsFilter.trim().toLowerCase();
    if (!query) return diagnostics;
    return diagnostics.filter((problem) =>
      `${displayPath(problem.path)} ${problem.message} ${problem.source} ${problem.severity}`.toLowerCase().includes(query),
    );
  }, [diagnosticsByPath, problemsFilter]);

  const filteredOutput = useMemo(() => {
    const query = outputFilter.trim().toLowerCase();
    return outputEntries.filter((entry) => {
      const channelMatches = outputChannel === "All Channels" || entry.channel === outputChannel;
      const queryMatches = !query || `${entry.channel} ${entry.level} ${entry.text}`.toLowerCase().includes(query);
      return channelMatches && queryMatches;
    });
  }, [outputChannel, outputEntries, outputFilter]);

  return (
    <section className="bottom-panel" data-maximized={isMaximized}>
      <div className="bottom-resize-handle" aria-hidden="true" />
      <div className="bottom-tabs">
        {tabs.map((tab) => (
          <button className="bottom-tab" type="button" data-active={activeTab === tab.id} key={tab.id} onClick={() => setActiveTab(tab.id)}>
            {tab.id === "terminal" && <TerminalSquare size={15} />}
            {t(tab.labelKey)}
          </button>
        ))}
      </div>
      <div className="bottom-actions">
        {activeTab === "problems" && <ProblemsActions filter={problemsFilter} setFilter={setProblemsFilter} />}
        {activeTab === "output" && (
          <OutputActions
            channel={outputChannel}
            filter={outputFilter}
            setChannel={setOutputChannel}
            setFilter={setOutputFilter}
            clear={() => setOutputEntries([])}
          />
        )}
        {activeTab === "terminal" && (
          <>
            <div className="terminal-profile"><TerminalSquare size={15} /> {terminalShellLabel(terminal, t)}</div>
            <select
              className="panel-select terminal-session-select"
              aria-label={t("panel.terminal.activeSession")}
              disabled={terminalSessions.length === 0}
              value={activeTerminalId ?? ""}
              onChange={(event) => setActiveTerminal(event.target.value)}
            >
              {terminalSessions.length === 0 ? <option value="">{t("panel.terminal.noSessions")}</option> : null}
              {terminalSessions.map((session, index) => (
                <option key={session.id} value={session.id}>{terminalSessionLabel(session, index)}</option>
              ))}
            </select>
            <PanelIconButton label={t("panel.terminal.new")} onClick={() => terminalMutation.mutate()} icon={<Plus size={15} />} />            <PanelIconButton label={t("panel.terminal.clear")} onClick={clearActiveTerminal} icon={<Eraser size={15} />} disabled={!terminal} />
            <PanelIconButton label={t("panel.terminal.closeActive")} onClick={closeActiveTerminal} icon={<Trash2 size={15} />} disabled={!terminal} />
          </>
        )}
        <PanelIconButton
          label={isMaximized ? t("panel.restoreSize") : t("panel.maximize")}
          onClick={onToggleMaximized}
          icon={isMaximized ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
        />
        <PanelIconButton label={t("panel.close")} onClick={() => setBottomPanelOpen(false)} icon={<X size={15} />} />
      </div>
      <PanelContent
        activeTab={activeTab}
        filteredOutput={filteredOutput}
        filteredProblems={filteredProblems}
        openProblem={(problem) => openProblemMutation.mutate(problem)}
        problemOpenError={problemOpenError}
        outputEntries={outputEntries}
        activeTerminalId={activeTerminalId}
        mountedSessionIds={mountedSessionIds}
        terminalSessions={terminalSessions}
        terminalOutputBuffers={terminalOutputBuffers}
        terminalClearTokens={terminalClearTokens}
        terminalError={terminalError}
        t={t}
      />
    </section>
  );
}

function ProblemsActions({ filter, setFilter }: { filter: string; setFilter: (value: string) => void }) {
  const { t } = useTranslation();
  return (
    <>
      <div className="panel-filter problems-filter">
        <input aria-label={t("panel.filter.problems")} value={filter} onChange={(event) => setFilter(event.target.value)} placeholder={t("panel.filter.problems.placeholder")} />
        <Filter size={15} />
      </div>
    </>
  );
}

function OutputActions({
  channel,
  clear,
  filter,
  setChannel,
  setFilter,
}: {
  channel: string;
  clear: () => void;
  filter: string;
  setChannel: (value: string) => void;
  setFilter: (value: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <>
      <select className="panel-select" aria-label={t("panel.output.channel.label")} value={channel} onChange={(event) => setChannel(event.target.value)}>
        <option value="All Channels">{t("panel.output.channel.allChannels")}</option>
        <option value="Lux Core">{t("panel.output.channel.luxCore")}</option>
        <option value="Tauri">{t("panel.output.channel.tauri")}</option>
        <option value="Extensions">{t("panel.output.channel.extensions")}</option>
      </select>
      <div className="panel-filter output-filter">
        <input aria-label={t("panel.filter.output")} value={filter} onChange={(event) => setFilter(event.target.value)} placeholder={t("panel.filter.output")} />
        <ListFilter size={15} />
      </div>
      <PanelIconButton label={t("panel.output.clear")} onClick={clear} icon={<Trash2 size={14} />} />
    </>
  );
}

function PanelIconButton({ disabled = false, icon, label, onClick }: { disabled?: boolean; icon: ReactNode; label: string; onClick?: () => void }) {
  return (
    <button className="icon-button compact" type="button" aria-label={label} title={label} disabled={disabled || !onClick} onClick={onClick}>
      {icon}
    </button>
  );
}

function PanelContent({
  activeTab,
  filteredOutput,
  filteredProblems,
  openProblem,
  problemOpenError,
  outputEntries,
  activeTerminalId,
  mountedSessionIds,
  terminalSessions,
  terminalOutputBuffers,
  terminalClearTokens,
  terminalError,
  t,
}: {
  activeTab: BottomPanelTab;
  filteredOutput: OutputEntry[];
  filteredProblems: WorkspaceDiagnostic[];
  openProblem: (problem: WorkspaceDiagnostic) => void;
  problemOpenError: string | null;
  outputEntries: OutputEntry[];
  activeTerminalId: string | null;
  mountedSessionIds: Set<string>;
  terminalSessions: TerminalSessionInfo[];
  terminalOutputBuffers: Record<string, { text: string }>;
  terminalClearTokens: Record<string, number>;
  terminalError: string | null;
  t: TranslateFn;
}) {
  return (
    <div className="bottom-panel-pages">
      {activeTab === "problems" && (
        <div className="bottom-panel-page" data-active="true">
          <ProblemsPanel error={problemOpenError} onOpenProblem={openProblem} problems={filteredProblems} />
        </div>
      )}
      {activeTab === "output" && (
        <div className="bottom-panel-page" data-active="true">
          <OutputPanel entries={filteredOutput} hasAnyEntries={outputEntries.length > 0} />
        </div>
      )}
      {/* Keep the terminal page mounted whenever sessions exist (hidden when another
          tab is active) so switching tabs never disposes the xterms — disposing and
          re-creating them would replay raw PTY bytes and garble the display. */}
      {(activeTab === "terminal" || terminalSessions.length > 0) && (
        <div className="bottom-panel-page" aria-hidden={activeTab !== "terminal"} data-active={activeTab === "terminal"}>
          <TerminalPanel
            activeTerminalId={activeTerminalId}
            mountedSessionIds={mountedSessionIds}
            terminalSessions={terminalSessions}
            terminalOutputBuffers={terminalOutputBuffers}
            terminalClearTokens={terminalClearTokens}
            terminalError={terminalError}
            t={t}
          />
        </div>
      )}
    </div>
  );
}

function TerminalPanel({
  activeTerminalId,
  mountedSessionIds,
  terminalSessions,
  terminalOutputBuffers,
  terminalClearTokens,
  terminalError,
  t,
}: {
  activeTerminalId: string | null;
  mountedSessionIds: Set<string>;
  terminalSessions: TerminalSessionInfo[];
  terminalOutputBuffers: Record<string, { text: string }>;
  terminalClearTokens: Record<string, number>;
  terminalError: string | null;
  t: TranslateFn;
}) {
  if (terminalSessions.length === 0) {
    return (
      <div className="bottom-panel-content empty-bottom-state">
        <span>{t("panel.terminal.noSessions")}</span>
        <span className="terminal-hint">{t("panel.terminal.pressPlusHint")}</span>
      </div>
    );
  }

  return (
    <div className="terminal-surface">
      {terminalError ? <div className="terminal-error">{terminalError}</div> : null}
      <div className="terminal-sessions-stack">
        {terminalSessions.map((session) => {
          // Only render xterm for sessions that have ever been mounted (keep them
          // alive so ANSI state is preserved on switch). Unmounted sessions get a
          // placeholder.
          const mounted = mountedSessionIds.has(session.id);
          const active = session.id === activeTerminalId;
          return (
            <div
              key={session.id}
              className="terminal-session-slot"
              data-active={active || undefined}
              aria-hidden={!active}
            >
              {mounted ? (
                <XtermTerminal
                  bufferText={terminalOutputBuffers[session.id]?.text ?? ""}
                  clearToken={terminalClearTokens[session.id] ?? 0}
                  session={session}
                />
              ) : (
                <div className="terminal-session-placeholder">
                  <span>{t("panel.terminal.notMounted")}</span>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function ProblemsPanel({ error, onOpenProblem, problems }: { error: string | null; onOpenProblem: (problem: WorkspaceDiagnostic) => void; problems: WorkspaceDiagnostic[] }) {
  const { t } = useTranslation();
  if (problems.length === 0) {
    return (
      <div className="bottom-panel-content empty-bottom-state">
        {error ? <span className="panel-inline-error">{error}</span> : t("panel.empty.noProblems")}
      </div>
    );
  }

  return (
    <div className="bottom-panel-content table-panel-content">
      {error ? <div className="panel-inline-error">{error}</div> : null}
      {problems.map((problem, index) => (
        <button className="problem-row" type="button" key={`${problem.path}-${problem.line}-${problem.column}-${problem.message}-${index}`} onClick={() => onOpenProblem(problem)}>
          <span data-severity={problem.severity}>{problem.severity}</span>
          <strong>{problem.message}</strong>
          <small>{displayPath(problem.path)}:{problem.line}:{problem.column}</small>
          <small>{problem.source}</small>
        </button>
      ))}
    </div>
  );
}

function OutputPanel({ entries, hasAnyEntries }: { entries: OutputEntry[]; hasAnyEntries: boolean }) {
  const { t } = useTranslation();
  if (!hasAnyEntries) {
    return <div className="bottom-panel-content empty-bottom-state muted-panel-content">{t("panel.empty.noOutputYet")}</div>;
  }
  if (entries.length === 0) {
    return <div className="bottom-panel-content empty-bottom-state muted-panel-content">{t("panel.empty.noOutputMatchesFilter")}</div>;
  }

  return (
    <div className="bottom-panel-content log-panel-content">
      {entries.map((entry, index) => (
        <div className="output-row" data-level={entry.level} key={`${entry.time}-${entry.channel}-${index}`}>
          <span>{entry.time}</span>
          <strong>{entry.channel}</strong>
          <code>{entry.text}</code>
        </div>
      ))}
    </div>
  );
}

function terminalShellLabel(session: TerminalSessionInfo | null, t: TranslateFn) {
  if (!session) return t("panel.terminal.shellFallback");
  const normalized = session.shell.replace(/\\/g, "/");
  return normalized.split("/").pop()?.replace(/\.exe$/i, "") || session.shell;
}

function terminalSessionLabel(session: TerminalSessionInfo, index: number) {
  const normalized = session.shell.replace(/\\/g, "/");
  const shell = normalized.split("/").pop()?.replace(/\.exe$/i, "") || session.shell;
  return `${index + 1}: ${shell}`;
}

function readErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
