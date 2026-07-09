import { Bug, ChevronRight, CircleDot, CornerDownRight, Loader2, Play, Plus, RefreshCw, Send, Square, StepForward, Trash2, Undo2 } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import { useTranslation } from "../../lib/i18n/useTranslation";
import { useAspectStore } from "../../lib/store";
import { aspectCommands, subscribeAspectEvents } from "../../lib/tauri";
import type { DebugAdapterInfo, DebugConfiguration, DebugEvaluateContext, DebugEvaluateResult, DebugExecutionAction, DebugFrameScopes, DebugResolvedBreakpoint, DebugSourceBreakpoint, DebugStackFrame, DebugStackTrace, DebugSessionInfo, DebugVariables, DebugWorkspaceInfo } from "../../lib/types";
import { PanelHeader, readErrorMessage, TreeMessage } from "./SidebarShared";

type DebugWatchExpression = {
  id: string;
  expression: string;
  result: DebugEvaluateResult | null;
  error: string | null;
};

export function RunDebugPanel() {
  const { t } = useTranslation();
  const workspace = useAspectStore((state) => state.workspace);
  const debugSourceBreakpointsByPath = useAspectStore((state) => state.debugSourceBreakpointsByPath);
  const debugResolvedBreakpointsByPath = useAspectStore((state) => state.debugResolvedBreakpointsByPath);
  const setDebugResolvedBreakpoints = useAspectStore((state) => state.setDebugResolvedBreakpoints);
  const [debugInfo, setDebugInfo] = useState<DebugWorkspaceInfo | null>(null);
  const [debugError, setDebugError] = useState<string | null>(null);
  const [selectedConfigName, setSelectedConfigName] = useState<string | null>(null);
  const [sessions, setSessions] = useState<DebugSessionInfo[]>([]);
  const [stackTrace, setStackTrace] = useState<DebugStackTrace | null>(null);
  const [selectedFrameId, setSelectedFrameId] = useState<number | null>(null);
  const [frameScopes, setFrameScopes] = useState<DebugFrameScopes | null>(null);
  const [variablesByReference, setVariablesByReference] = useState<Record<number, DebugVariables>>({});
  const [watchExpressions, setWatchExpressions] = useState<DebugWatchExpression[]>([]);
  const [evaluateResults, setEvaluateResults] = useState<DebugEvaluateResult[]>([]);
  const syncedBreakpointPathsRef = useRef<Set<string>>(new Set());
  const watchRefreshSignatureRef = useRef<string | null>(null);

  const debugMutation = useMutation({
    mutationFn: aspectCommands.debugWorkspaceInfo,
    onSuccess: (info) => {
      setDebugInfo(info);
      setDebugError(null);
      setSelectedConfigName((current) => current ?? info.configurations[0]?.name ?? null);
    },
    onError: (error) => setDebugError(readErrorMessage(error, t)),
  });

  const startMutation = useMutation({
    mutationFn: ({ breakpoints, configuration }: { breakpoints: DebugSourceBreakpoint[]; configuration: DebugConfiguration }) => aspectCommands.debugStart(configuration, breakpoints),
    onSuccess: (session) => {
      setDebugError(null);
      upsertSession(setSessions, session);
    },
    onError: (error) => setDebugError(readErrorMessage(error, t)),
  });

  const stopMutation = useMutation({
    mutationFn: aspectCommands.debugStop,
    onSuccess: (session) => {
      setDebugError(null);
      upsertSession(setSessions, session);
    },
    onError: (error) => setDebugError(readErrorMessage(error, t)),
  });

  const stackTraceMutation = useMutation({
    mutationFn: aspectCommands.debugStackTrace,
    onSuccess: (trace) => {
      setDebugError(null);
      setStackTrace(trace);
      const firstFrame = trace.frames[0] ?? null;
      setSelectedFrameId(firstFrame?.id ?? null);
      setFrameScopes(null);
      setVariablesByReference({});
    },
    onError: (error) => setDebugError(readErrorMessage(error, t)),
  });

  const executeMutation = useMutation({
    mutationFn: ({ action, sessionId }: { action: DebugExecutionAction; sessionId: string }) => aspectCommands.debugExecute(sessionId, action),
    onSuccess: (session) => {
      setDebugError(null);
      setStackTrace(null);
      setSelectedFrameId(null);
      setFrameScopes(null);
      setVariablesByReference({});
      upsertSession(setSessions, session);
    },
    onError: (error) => setDebugError(readErrorMessage(error, t)),
  });

  const setBreakpointsMutation = useMutation({
    mutationFn: ({ breakpoints, path, sessionId }: { breakpoints: DebugSourceBreakpoint[]; path: string; sessionId: string }) => aspectCommands.debugSetBreakpoints(sessionId, path, breakpoints),
    onSuccess: (update) => {
      setDebugError(null);
      setDebugResolvedBreakpoints(update);
    },
    onError: (error) => setDebugError(readErrorMessage(error, t)),
  });

  const scopesMutation = useMutation({
    mutationFn: ({ frameId, sessionId }: { frameId: number; sessionId: string }) => aspectCommands.debugScopes(sessionId, frameId),
    onSuccess: (scopes) => {
      setDebugError(null);
      setFrameScopes(scopes);
      setVariablesByReference({});
    },
    onError: (error) => setDebugError(readErrorMessage(error, t)),
  });

  const variablesMutation = useMutation({
    mutationFn: ({ sessionId, variablesReference }: { sessionId: string; variablesReference: number }) => aspectCommands.debugVariables(sessionId, variablesReference),
    onSuccess: (variables) => {
      setDebugError(null);
      setVariablesByReference((current) => ({ ...current, [variables.variables_reference]: variables }));
    },
    onError: (error) => setDebugError(readErrorMessage(error, t)),
  });

  const evaluateMutation = useMutation({
    mutationFn: ({ context, expression, frameId, sessionId }: { context: DebugEvaluateContext; expression: string; frameId: number | null; sessionId: string; watchId?: string }) =>
      aspectCommands.debugEvaluate(sessionId, expression, frameId, context),
    onSuccess: (result, variables) => {
      setDebugError(null);
      if (variables.watchId) {
        setWatchExpressions((items) => items.map((item) => item.id === variables.watchId ? { ...item, result, error: null } : item));
        return;
      }
      setEvaluateResults((current) => [result, ...current.filter((item) => item.expression !== result.expression || item.session_id !== result.session_id)].slice(0, 30));
    },
    onError: (error, variables) => {
      const message = readErrorMessage(error, t);
      if (variables.watchId) {
        setWatchExpressions((items) => items.map((item) => item.id === variables.watchId ? { ...item, result: null, error: message } : item));
        return;
      }
      setDebugError(message);
    },
  });

  useEffect(() => {
    if (!workspace) {
      setDebugInfo(null);
      setSelectedConfigName(null);
      setSessions([]);
      setStackTrace(null);
      setSelectedFrameId(null);
      setFrameScopes(null);
      setVariablesByReference({});
      setWatchExpressions([]);
      setEvaluateResults([]);
      return;
    }
    let cancelled = false;
    debugMutation.mutate();
    void aspectCommands.debugSessions().then((next) => { if (!cancelled) setSessions(next); }).catch(() => { if (!cancelled) setSessions([]); });
    return () => { cancelled = true; };
  }, [workspace?.root]);

  useEffect(() => {
    let active = true;
    let dispose: (() => void) | undefined;
    void subscribeAspectEvents((event) => {
      if (event.type === "debugSessionChanged") upsertSession(setSessions, event.session);
      if (event.type === "debugBreakpointsChanged") setDebugResolvedBreakpoints(event.update);
    }).then((unlisten) => {
      if (!active) unlisten();
      else dispose = unlisten;
    });
    return () => { active = false; dispose?.(); };
  }, [setDebugResolvedBreakpoints]);

  const selectedConfiguration = debugInfo?.configurations.find((configuration) => configuration.name === selectedConfigName) ?? debugInfo?.configurations[0] ?? null;
  const selectedAdapter = selectedConfiguration
    ? debugInfo?.adapters.find((adapter) => adapterMatchesConfiguration(adapter, selectedConfiguration)) ?? null
    : null;
  const activeSession = selectedConfiguration
    ? sessions.find((session) => session.configuration_name === selectedConfiguration.name && session.status !== "stopped" && session.status !== "error") ?? null
    : null;
  const activeStackTrace = activeSession && stackTrace?.session_id === activeSession.id ? stackTrace : null;
  const sourceBreakpoints = Object.values(debugSourceBreakpointsByPath).flat();
  const resolvedBreakpoints = Object.values(debugResolvedBreakpointsByPath).flat();
  const watchExpressionSignature = watchExpressions.map((item) => `${item.id}:${item.expression}`).join("|");

  // Per-path breakpoint content signatures: JSON-stable hash of lines+conditions
  // used to detect which paths actually changed so we skip no-op re-syncs.
  const breakpointSignaturesRef = useRef<Map<string, string>>(new Map());

  useEffect(() => {
    const liveSessions = sessions.filter((session) => session.status !== "stopped" && session.status !== "error");
    const currentPaths = new Set(Object.keys(debugSourceBreakpointsByPath));

    if (liveSessions.length === 0) {
      // No live sessions вЂ” update our tracking state and bail early.
      syncedBreakpointPathsRef.current = currentPaths;
      breakpointSignaturesRef.current = new Map();
      return;
    }

    // Compute the set of paths whose breakpoint content changed or that were
    // removed since last sync. Only these paths need to be sent to adapters.
    const removedPaths = [...syncedBreakpointPathsRef.current].filter((p) => !currentPaths.has(p));
    const changedPaths = [...currentPaths].filter((path) => {
      const bps = debugSourceBreakpointsByPath[path] ?? [];
      const sig = JSON.stringify(bps.map((bp) => `${bp.line}:${bp.column ?? ""}:${bp.condition ?? ""}:${bp.log_message ?? ""}`).sort());
      if (breakpointSignaturesRef.current.get(path) === sig) return false;
      breakpointSignaturesRef.current.set(path, sig);
      return true;
    });

    const pathsToSync = new Set([...changedPaths, ...removedPaths]);
    if (pathsToSync.size === 0) return;

    for (const session of liveSessions) {
      for (const path of pathsToSync) {
        setBreakpointsMutation.mutate({ sessionId: session.id, path, breakpoints: debugSourceBreakpointsByPath[path] ?? [] });
      }
    }

    // Clean up signatures for removed paths
    for (const path of removedPaths) breakpointSignaturesRef.current.delete(path);
    syncedBreakpointPathsRef.current = currentPaths;
  // eslint-disable-next-line react-hooks/exhaustive-deps -- session status is intentionally inlined into the dep
  }, [debugSourceBreakpointsByPath, sessions.map((session) => `${session.id}:${session.status}`).join("|")]);

  useEffect(() => {
    if (!activeSession || activeSession.status !== "paused" || watchExpressions.length === 0) {
      watchRefreshSignatureRef.current = null;
      return;
    }
    const signature = `${activeSession.id}:${selectedFrameId ?? "no-frame"}:${watchExpressionSignature}`;
    if (watchRefreshSignatureRef.current === signature) return;
    watchRefreshSignatureRef.current = signature;
    for (const item of watchExpressions) {
      evaluateMutation.mutate({ sessionId: activeSession.id, expression: item.expression, frameId: selectedFrameId, context: "watch", watchId: item.id });
    }
  }, [activeSession?.id, activeSession?.status, selectedFrameId, watchExpressionSignature]);

  return (
    <div className="panel-content utility-panel-content run-debug-panel-content">
      <PanelHeader
        title={t("sidebar.runDebug.title")}
        actions={[{ label: t("sidebar.runDebug.actions.refreshConfiguration"), icon: debugMutation.isPending ? <Loader2 size={14} className="spin-icon" /> : <RefreshCw size={14} />, onClick: () => debugMutation.mutate(), disabled: !workspace || debugMutation.isPending }]}
      />
      {!workspace ? <TreeMessage depth={0} text={t("sidebar.runDebug.empty.openWorkspace")} /> : null}
      {debugError ? <TreeMessage depth={0} tone="error" text={debugError} /> : null}
      {workspace && !debugMutation.isPending && debugInfo && (
        <>
          <DebugLaunchBlock
            adapter={selectedAdapter}
            configuration={selectedConfiguration}
            configurations={debugInfo.configurations}
            isStarting={startMutation.isPending}
            isStopping={stopMutation.isPending}
            launchJsonPath={debugInfo.launch_json_path}
            onStart={(configuration) => startMutation.mutate({ configuration, breakpoints: sourceBreakpoints })}
            onStop={(sessionId) => stopMutation.mutate(sessionId)}
            onRefreshStack={(sessionId) => stackTraceMutation.mutate(sessionId)}
            onExecute={(sessionId, action) => executeMutation.mutate({ action, sessionId })}
            onLoadScopes={(sessionId, frameId) => {
              setSelectedFrameId(frameId);
              scopesMutation.mutate({ sessionId, frameId });
            }}
            onLoadVariables={(sessionId, variablesReference) => variablesMutation.mutate({ sessionId, variablesReference })}
            onEvaluate={(sessionId, expression, frameId, context) => evaluateMutation.mutate({ sessionId, expression, frameId, context })}
            onRefreshWatch={(sessionId, watchId, expression, frameId) => {
              evaluateMutation.mutate({ sessionId, expression, frameId, context: "watch", watchId });
            }}
            session={activeSession}
            executionPending={executeMutation.isPending}
            evaluatePending={evaluateMutation.isPending}
            evaluateResults={activeSession ? evaluateResults.filter((result) => result.session_id === activeSession.id) : []}
            frameScopes={frameScopes}
            scopeLoading={scopesMutation.isPending}
            selectedFrameId={selectedFrameId}
            stackTrace={activeStackTrace}
            stackTraceLoading={stackTraceMutation.isPending}
            variablesByReference={variablesByReference}
            variablesLoadingReference={variablesMutation.isPending ? variablesMutation.variables?.variablesReference ?? null : null}
            watchExpressions={watchExpressions}
            setWatchExpressions={setWatchExpressions}
            setSelectedConfigName={setSelectedConfigName}
          />
          <DebugAdaptersBlock adapters={debugInfo.adapters} />
          <DebugBreakpointsBlock breakpoints={sourceBreakpoints} resolvedBreakpoints={resolvedBreakpoints} />
        </>
      )}
      {workspace && debugMutation.isPending ? <TreeMessage depth={0} text={t("sidebar.runDebug.scanning")} /> : null}
    </div>
  );
}

function DebugLaunchBlock({
  adapter,
  configuration,
  configurations,
  isStarting,
  isStopping,
  launchJsonPath,
  onStart,
  onStop,
  onRefreshStack,
  onExecute,
  onLoadScopes,
  onLoadVariables,
  onEvaluate,
  onRefreshWatch,
  session,
  executionPending,
  evaluatePending,
  evaluateResults,
  frameScopes,
  scopeLoading,
  selectedFrameId,
  stackTrace,
  stackTraceLoading,
  variablesByReference,
  variablesLoadingReference,
  watchExpressions,
  setWatchExpressions,
  setSelectedConfigName,
}: {
  adapter: DebugAdapterInfo | null;
  configuration: DebugConfiguration | null;
  configurations: DebugConfiguration[];
  isStarting: boolean;
  isStopping: boolean;
  launchJsonPath: string | null;
  onStart: (configuration: DebugConfiguration) => void;
  onStop: (sessionId: string) => void;
  onRefreshStack: (sessionId: string) => void;
  onExecute: (sessionId: string, action: DebugExecutionAction) => void;
  onLoadScopes: (sessionId: string, frameId: number) => void;
  onLoadVariables: (sessionId: string, variablesReference: number) => void;
  onEvaluate: (sessionId: string, expression: string, frameId: number | null, context: DebugEvaluateContext) => void;
  onRefreshWatch: (sessionId: string, watchId: string, expression: string, frameId: number | null) => void;
  session: DebugSessionInfo | null;
  executionPending: boolean;
  evaluatePending: boolean;
  evaluateResults: DebugEvaluateResult[];
  frameScopes: DebugFrameScopes | null;
  scopeLoading: boolean;
  selectedFrameId: number | null;
  stackTrace: DebugStackTrace | null;
  stackTraceLoading: boolean;
  variablesByReference: Record<number, DebugVariables>;
  variablesLoadingReference: number | null;
  watchExpressions: DebugWatchExpression[];
  setWatchExpressions: (updater: (items: DebugWatchExpression[]) => DebugWatchExpression[]) => void;
  setSelectedConfigName: (name: string) => void;
}) {
  const { t } = useTranslation();
  const disabledReason = debugStartDisabledReason(configuration, adapter, t);
  const canStart = !session && configuration && !disabledReason && !isStarting;
  return (
    <section className="debug-section">
      <div className="debug-section-title">{t("sidebar.runDebug.start.heading")}</div>
      {configurations.length > 0 ? (
        <select className="debug-config-select" value={configuration?.name ?? ""} onChange={(event) => setSelectedConfigName(event.target.value)} aria-label={t("sidebar.runDebug.aria.debugConfiguration")}>
          {configurations.map((item) => (
            <option value={item.name} key={`${item.type}-${item.request}-${item.name}`}>{item.name}</option>
          ))}
        </select>
      ) : (
        <div className="debug-empty-card">
          <Bug size={16} />
          <span>{t("sidebar.runDebug.empty.noLaunchConfigurations")}</span>
        </div>
      )}
      {session ? (
        <button className="debug-run-button debug-stop-button" type="button" disabled={isStopping} title={t("sidebar.runDebug.stopDebugging")} onClick={() => onStop(session.id)}>
          {isStopping ? <Loader2 size={15} className="spin-icon" /> : <Square size={15} />} {t("sidebar.runDebug.stopDebugging")}
        </button>
      ) : (
        <button className="debug-run-button" type="button" disabled={!canStart} title={disabledReason ?? t("sidebar.runDebug.startDebugging")} onClick={() => configuration && onStart(configuration)}>
          {isStarting ? <Loader2 size={15} className="spin-icon" /> : <Play size={15} />} {t("sidebar.runDebug.startDebugging")}
        </button>
      )}
      <div className="debug-meta-list">
        <DebugMeta label={t("sidebar.runDebug.meta.configuration")} value={configuration ? `${configuration.request} / ${configuration.type}` : t("sidebar.runDebug.meta.notConfigured")} />
        <DebugMeta label={t("sidebar.runDebug.meta.adapter")} value={adapter ? `${adapter.name} (${adapter.status}, ${adapter.transport})` : configuration ? t("sidebar.runDebug.meta.noMatchingAdapter") : t("sidebar.runDebug.meta.notSelected")} tone={adapterTone(adapter)} />
        <DebugMeta label={t("sidebar.runDebug.meta.session")} value={session ? `${session.status}${session.last_event ? ` / ${session.last_event}` : ""}` : disabledReason ?? t("sidebar.runDebug.meta.notRunning")} tone={session?.status === "error" || disabledReason ? "warning" : session ? undefined : "muted"} />
        <DebugMeta label="launch.json" value={launchJsonPath ?? t("sidebar.runDebug.meta.missingLaunchJson")} tone={launchJsonPath ? undefined : "muted"} />
      </div>
      {session ? (
        <>
          <DebugExecutionControls
            isPending={executionPending}
            onExecute={(action) => onExecute(session.id, action)}
            session={session}
          />
          <DebugStackBlock
            isLoading={stackTraceLoading}
            onLoadScopes={(frameId) => onLoadScopes(session.id, frameId)}
            onRefresh={() => onRefreshStack(session.id)}
            selectedFrameId={selectedFrameId}
            session={session}
            stackTrace={stackTrace}
          />
          <DebugVariablesBlock
            frameScopes={frameScopes}
            isLoading={scopeLoading}
            onLoadVariables={(variablesReference) => onLoadVariables(session.id, variablesReference)}
            selectedFrameId={selectedFrameId}
            variablesByReference={variablesByReference}
            variablesLoadingReference={variablesLoadingReference}
          />
          <DebugWatchBlock
            isPending={evaluatePending}
            onLoadVariables={(variablesReference) => onLoadVariables(session.id, variablesReference)}
            onRefreshWatch={(watchId, expression) => onRefreshWatch(session.id, watchId, expression, selectedFrameId)}
            selectedFrameId={selectedFrameId}
            setWatchExpressions={setWatchExpressions}
            session={session}
            variablesByReference={variablesByReference}
            variablesLoadingReference={variablesLoadingReference}
            watchExpressions={watchExpressions}
          />
          <DebugEvaluateBlock
            isPending={evaluatePending}
            onEvaluate={(expression, context) => onEvaluate(session.id, expression, selectedFrameId, context)}
            onLoadVariables={(variablesReference) => onLoadVariables(session.id, variablesReference)}
            results={evaluateResults}
            selectedFrameId={selectedFrameId}
            session={session}
            variablesByReference={variablesByReference}
            variablesLoadingReference={variablesLoadingReference}
          />
        </>
      ) : null}
    </section>
  );
}

function DebugExecutionControls({
  isPending,
  onExecute,
  session,
}: {
  isPending: boolean;
  onExecute: (action: DebugExecutionAction) => void;
  session: DebugSessionInfo;
}) {
  const { t } = useTranslation();
  const disabled = session.status !== "paused" || isPending;
  const actions: Array<{ action: DebugExecutionAction; icon: ReactNode; label: string }> = [
    { action: "continue", icon: <Play size={13} />, label: t("sidebar.runDebug.controls.continue") },
    { action: "stepOver", icon: <StepForward size={13} />, label: t("sidebar.runDebug.controls.stepOver") },
    { action: "stepIn", icon: <CornerDownRight size={13} />, label: t("sidebar.runDebug.controls.stepIn") },
    { action: "stepOut", icon: <Undo2 size={13} />, label: t("sidebar.runDebug.controls.stepOut") },
  ];

  return (
    <div className="debug-execution-controls" aria-label={t("sidebar.runDebug.controls.heading")}>
      {actions.map((item) => (
        <button type="button" key={item.action} disabled={disabled} title={item.label} aria-label={item.label} onClick={() => onExecute(item.action)}>
          {isPending ? <Loader2 size={13} className="spin-icon" /> : item.icon}
        </button>
      ))}
    </div>
  );
}

function DebugStackBlock({
  isLoading,
  onLoadScopes,
  onRefresh,
  selectedFrameId,
  session,
  stackTrace,
}: {
  isLoading: boolean;
  onLoadScopes: (frameId: number) => void;
  onRefresh: () => void;
  selectedFrameId: number | null;
  session: DebugSessionInfo;
  stackTrace: DebugStackTrace | null;
}) {
  const { t } = useTranslation();
  const canRefresh = session.status === "paused";
  return (
    <div className="debug-stack-block">
      <div className="debug-stack-header">
        <span>{t("sidebar.runDebug.stack.heading")}</span>
        <button type="button" disabled={!canRefresh || isLoading} onClick={onRefresh} title={t("sidebar.runDebug.stack.refresh")}>
          {isLoading ? <Loader2 size={13} className="spin-icon" /> : <RefreshCw size={13} />}
        </button>
      </div>
      {!canRefresh ? <div className="debug-stack-empty">{t("sidebar.runDebug.stack.waitingForPause")}</div> : null}
      {canRefresh && !stackTrace ? <div className="debug-stack-empty">{t("sidebar.runDebug.stack.refreshHint")}</div> : null}
      {stackTrace ? (
        <div className="debug-stack-list">
          <div className="debug-stack-thread">{stackTrace.thread.name}</div>
          {stackTrace.frames.length === 0 ? <div className="debug-stack-empty">{t("sidebar.runDebug.stack.empty")}</div> : null}
          {stackTrace.frames.map((frame) => (
            <button className="debug-stack-frame" data-selected={frame.id === selectedFrameId} key={frame.id} title={frame.source_path ?? frame.name} type="button" onClick={() => onLoadScopes(frame.id)}>
              <strong>{frame.name}</strong>
              <small>{frame.source_path ? `${frame.source_path}:${frame.line}:${frame.column}` : `${frame.line}:${frame.column}`}</small>
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function DebugVariablesBlock({
  frameScopes,
  isLoading,
  onLoadVariables,
  selectedFrameId,
  variablesByReference,
  variablesLoadingReference,
}: {
  frameScopes: DebugFrameScopes | null;
  isLoading: boolean;
  onLoadVariables: (variablesReference: number) => void;
  selectedFrameId: number | null;
  variablesByReference: Record<number, DebugVariables>;
  variablesLoadingReference: number | null;
}) {
  const { t } = useTranslation();
  const scopesMatchFrame = frameScopes && frameScopes.frame_id === selectedFrameId;
  return (
    <div className="debug-variables-block">
      <div className="debug-stack-header">
        <span>{t("sidebar.runDebug.variables.heading")}</span>
        {isLoading ? <Loader2 size={13} className="spin-icon" /> : null}
      </div>
      {selectedFrameId === null ? <div className="debug-stack-empty">{t("sidebar.runDebug.variables.noFrame")}</div> : null}
      {selectedFrameId !== null && !scopesMatchFrame && !isLoading ? <div className="debug-stack-empty">{t("sidebar.runDebug.variables.selectFrame")}</div> : null}
      {scopesMatchFrame && frameScopes.scopes.length === 0 ? <div className="debug-stack-empty">{t("sidebar.runDebug.variables.emptyScopes")}</div> : null}
      {scopesMatchFrame && frameScopes.scopes.length > 0 ? (
        <div className="debug-variable-scope-list">
          {frameScopes.scopes.map((scope) => {
            const variables = variablesByReference[scope.variables_reference] ?? null;
            const isVariablesLoading = variablesLoadingReference === scope.variables_reference;
            return (
              <div className="debug-variable-scope" key={scope.variables_reference}>
                <button type="button" onClick={() => onLoadVariables(scope.variables_reference)} title={scope.expensive ? t("sidebar.runDebug.variables.expensiveScope") : scope.name}>
                  <ChevronRight size={13} data-open={Boolean(variables)} />
                  <span>{scope.name}</span>
                  {isVariablesLoading ? <Loader2 size={12} className="spin-icon" /> : null}
                </button>
                {variables ? <DebugVariableList variables={variables.variables} /> : null}
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

function DebugVariableList({ variables }: { variables: DebugVariables["variables"] }) {
  const { t } = useTranslation();
  if (variables.length === 0) return <div className="debug-stack-empty">{t("sidebar.runDebug.variables.empty")}</div>;
  return (
    <div className="debug-variable-list">
      {variables.map((variable) => (
        <div className="debug-variable-row" key={`${variable.name}:${variable.evaluate_name ?? variable.value}`} title={variable.type_name ?? variable.value}>
          <span>{variable.name}</span>
          <strong>{variable.value}</strong>
        </div>
      ))}
    </div>
  );
}

function DebugWatchBlock({
  isPending,
  onLoadVariables,
  onRefreshWatch,
  selectedFrameId,
  session,
  setWatchExpressions,
  variablesByReference,
  variablesLoadingReference,
  watchExpressions,
}: {
  isPending: boolean;
  onLoadVariables: (variablesReference: number) => void;
  onRefreshWatch: (watchId: string, expression: string) => void;
  selectedFrameId: number | null;
  session: DebugSessionInfo;
  setWatchExpressions: (updater: (items: DebugWatchExpression[]) => DebugWatchExpression[]) => void;
  variablesByReference: Record<number, DebugVariables>;
  variablesLoadingReference: number | null;
  watchExpressions: DebugWatchExpression[];
}) {
  const { t } = useTranslation();
  const [draftExpression, setDraftExpression] = useState("");
  const canRefresh = session.status === "paused" && !isPending;

  return (
    <div className="debug-watch-block">
      <div className="debug-stack-header">
        <span>{t("sidebar.runDebug.watch.heading")}</span>
        <small>{selectedFrameId ? t("sidebar.runDebug.evaluate.frame", { frameId: selectedFrameId }) : t("sidebar.runDebug.evaluate.noFrame")}</small>
      </div>
      <form
        className="debug-watch-form"
        onSubmit={(event) => {
          event.preventDefault();
          const expression = draftExpression.trim();
          if (!expression) return;
          const id = `${Date.now().toString(36)}:${expression}`;
          setWatchExpressions((items) => [{ id, expression, result: null, error: null }, ...items.filter((item) => item.expression !== expression)].slice(0, 30));
          setDraftExpression("");
        }}
      >
        <input
          value={draftExpression}
          onChange={(event) => setDraftExpression(event.target.value)}
          placeholder={t("sidebar.runDebug.watch.placeholder")}
          disabled={session.status !== "paused"}
        />
        <button type="submit" disabled={session.status !== "paused" || draftExpression.trim().length === 0} title={t("sidebar.runDebug.watch.add")}>
          <Plus size={13} />
        </button>
      </form>
      {session.status !== "paused" ? <div className="debug-stack-empty">{t("sidebar.runDebug.evaluate.waitingForPause")}</div> : null}
      {session.status === "paused" && watchExpressions.length === 0 ? <div className="debug-stack-empty">{t("sidebar.runDebug.watch.empty")}</div> : null}
      {watchExpressions.length > 0 ? (
        <div className="debug-watch-list">
          {watchExpressions.map((item) => {
            const result = item.result;
            const variablesReference = result?.variables_reference ?? 0;
            const variables = variablesReference > 0 ? variablesByReference[variablesReference] ?? null : null;
            const isVariablesLoading = variablesLoadingReference === variablesReference;
            return (
              <div className="debug-watch-row" key={item.id} data-error={Boolean(item.error)}>
                <button type="button" disabled={variablesReference === 0} onClick={() => onLoadVariables(variablesReference)} title={item.expression}>
                  <ChevronRight size={13} data-open={Boolean(variables)} />
                  <span>{item.expression}</span>
                  {isVariablesLoading ? <Loader2 size={12} className="spin-icon" /> : null}
                </button>
                <strong title={item.error ?? result?.type_name ?? result?.result ?? t("sidebar.runDebug.watch.notEvaluated")}>
                  {item.error ?? result?.result ?? t("sidebar.runDebug.watch.notEvaluated")}
                </strong>
                <span className="debug-watch-actions">
                  <button type="button" disabled={!canRefresh} onClick={() => onRefreshWatch(item.id, item.expression)} title={t("sidebar.runDebug.watch.refresh")}>
                    {isPending ? <Loader2 size={12} className="spin-icon" /> : <RefreshCw size={12} />}
                  </button>
                  <button type="button" onClick={() => setWatchExpressions((items) => items.filter((candidate) => candidate.id !== item.id))} title={t("sidebar.runDebug.watch.remove")}>
                    <Trash2 size={12} />
                  </button>
                </span>
                {variables ? <DebugVariableList variables={variables.variables} /> : null}
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

function DebugEvaluateBlock({
  isPending,
  onEvaluate,
  onLoadVariables,
  results,
  selectedFrameId,
  session,
  variablesByReference,
  variablesLoadingReference,
}: {
  isPending: boolean;
  onEvaluate: (expression: string, context: DebugEvaluateContext) => void;
  onLoadVariables: (variablesReference: number) => void;
  results: DebugEvaluateResult[];
  selectedFrameId: number | null;
  session: DebugSessionInfo;
  variablesByReference: Record<number, DebugVariables>;
  variablesLoadingReference: number | null;
}) {
  const { t } = useTranslation();
  const [expression, setExpression] = useState("");
  const [context, setContext] = useState<DebugEvaluateContext>("watch");
  const canEvaluate = session.status === "paused" && expression.trim().length > 0 && !isPending;

  return (
    <div className="debug-evaluate-block">
      <div className="debug-stack-header">
        <span>{t("sidebar.runDebug.evaluate.heading")}</span>
        <small>{selectedFrameId ? t("sidebar.runDebug.evaluate.frame", { frameId: selectedFrameId }) : t("sidebar.runDebug.evaluate.noFrame")}</small>
      </div>
      <form
        className="debug-evaluate-form"
        onSubmit={(event) => {
          event.preventDefault();
          const value = expression.trim();
          if (!value) return;
          onEvaluate(value, context);
        }}
      >
        <select value={context} onChange={(event) => setContext(event.target.value as DebugEvaluateContext)} aria-label={t("sidebar.runDebug.evaluate.context")}>
          <option value="watch">{t("sidebar.runDebug.evaluate.context.watch")}</option>
          <option value="repl">{t("sidebar.runDebug.evaluate.context.repl")}</option>
          <option value="hover">{t("sidebar.runDebug.evaluate.context.hover")}</option>
        </select>
        <input
          value={expression}
          onChange={(event) => setExpression(event.target.value)}
          placeholder={t("sidebar.runDebug.evaluate.placeholder")}
          disabled={session.status !== "paused"}
        />
        <button type="submit" disabled={!canEvaluate} title={t("sidebar.runDebug.evaluate.run")}>
          {isPending ? <Loader2 size={13} className="spin-icon" /> : <Send size={13} />}
        </button>
      </form>
      {session.status !== "paused" ? <div className="debug-stack-empty">{t("sidebar.runDebug.evaluate.waitingForPause")}</div> : null}
      {session.status === "paused" && results.length === 0 ? <div className="debug-stack-empty">{t("sidebar.runDebug.evaluate.empty")}</div> : null}
      {results.length > 0 ? (
        <div className="debug-evaluate-results">
          {results.map((result) => {
            const variables = result.variables_reference > 0 ? variablesByReference[result.variables_reference] ?? null : null;
            const isVariablesLoading = variablesLoadingReference === result.variables_reference;
            return (
              <div className="debug-evaluate-result" key={`${result.expression}:${result.result}`} title={result.type_name ?? result.result}>
                <button type="button" disabled={result.variables_reference === 0} onClick={() => onLoadVariables(result.variables_reference)}>
                  <ChevronRight size={13} data-open={Boolean(variables)} />
                  <span>{result.expression}</span>
                  {isVariablesLoading ? <Loader2 size={12} className="spin-icon" /> : null}
                </button>
                <strong>{result.result}</strong>
                {result.type_name ? <small>{result.type_name}</small> : null}
                {variables ? <DebugVariableList variables={variables.variables} /> : null}
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

function DebugAdaptersBlock({ adapters }: { adapters: DebugAdapterInfo[] }) {
  const { t } = useTranslation();
  return (
    <section className="debug-section">
      <div className="debug-section-title">{t("sidebar.runDebug.adapters.heading")}</div>
      {adapters.length === 0 ? <TreeMessage depth={0} text={t("sidebar.runDebug.adapters.empty")} /> : null}
      <div className="debug-adapter-list">
        {adapters.map((adapter) => (
          <div className="debug-adapter-row" data-status={adapter.status} key={adapter.id} title={adapter.error ?? adapter.command}>
            <span className="debug-adapter-icon"><Bug size={15} /></span>
            <span className="debug-adapter-main">
              <strong>{adapter.name}</strong>
              <small>{adapter.command}{adapter.args.length > 0 ? ` ${adapter.args.join(" ")}` : ""} / {adapter.transport}</small>
            </span>
            <span className="debug-adapter-status">{adapter.status}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function DebugBreakpointsBlock({ breakpoints, resolvedBreakpoints }: { breakpoints: DebugSourceBreakpoint[]; resolvedBreakpoints: DebugResolvedBreakpoint[] }) {
  const { t } = useTranslation();
  const resolvedByLocation = new Map(resolvedBreakpoints.map((breakpoint) => [breakpointKey(breakpoint.path, breakpoint.line), breakpoint]));
  return (
    <section className="debug-section">
      <div className="debug-section-title">{t("sidebar.runDebug.breakpoints.heading")}</div>
      {breakpoints.length === 0 ? <TreeMessage depth={0} text={t("sidebar.runDebug.breakpoints.empty")} /> : null}
      {breakpoints.length > 0 ? (
        <div className="debug-breakpoint-list">
          {breakpoints.map((breakpoint) => {
            const resolved = resolvedByLocation.get(breakpointKey(breakpoint.path, breakpoint.line));
            return (
              <div className="debug-breakpoint-row" data-verified={resolved?.verified ?? false} key={breakpointKey(breakpoint.path, breakpoint.line)} title={resolved?.message ?? breakpoint.path}>
                <CircleDot size={13} />
                <span>{breakpoint.path}</span>
                <strong>{breakpoint.line}</strong>
              </div>
            );
          })}
        </div>
      ) : null}
    </section>
  );
}

function adapterMatchesConfiguration(adapter: DebugAdapterInfo, configuration: DebugConfiguration) {
  const configuredType = configuration.type.toLowerCase();
  return adapter.id.toLowerCase() === configuredType
    || adapter.command.toLowerCase() === configuredType
    || adapter.configuration_types.some((adapterType) => adapterType.toLowerCase() === configuredType);
}

function debugStartDisabledReason(configuration: DebugConfiguration | null, adapter: DebugAdapterInfo | null, t: TranslateFn) {
  if (!configuration) return t("sidebar.runDebug.empty.noLaunchConfigurations");
  if (!adapter) return t("sidebar.runDebug.meta.noMatchingAdapter");
  if (adapter.status !== "available") return adapter.error ?? t("sidebar.runDebug.start.adapterMissing");
  return null;
}

function adapterTone(adapter: DebugAdapterInfo | null): "muted" | "warning" | undefined {
  if (!adapter) return "muted";
  if (adapter.status !== "available") return "warning";
  return undefined;
}

function upsertSession(setSessions: (updater: (sessions: DebugSessionInfo[]) => DebugSessionInfo[]) => void, session: DebugSessionInfo) {
  setSessions((sessions) => {
    const next = sessions.filter((item) => item.id !== session.id);
    next.unshift(session);
    return next.slice(0, 20);
  });
}

function breakpointKey(path: string, line: number) {
  return `${path}:${line}`;
}

function DebugMeta({ label, tone, value }: { label: string; tone?: "muted" | "warning"; value: string }) {
  return (
    <div className="debug-meta-row" data-tone={tone}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
