import { Bug, Loader2, Play, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { useTranslation } from "../../lib/i18n/useTranslation";
import { useLuxStore } from "../../lib/store";
import { luxCommands } from "../../lib/tauri";
import type { DebugAdapterInfo, DebugConfiguration, DebugWorkspaceInfo } from "../../lib/types";
import { PanelHeader, readErrorMessage, TreeMessage } from "./SidebarShared";

export function RunDebugPanel() {
  const { t } = useTranslation();
  const workspace = useLuxStore((state) => state.workspace);
  const [debugInfo, setDebugInfo] = useState<DebugWorkspaceInfo | null>(null);
  const [debugError, setDebugError] = useState<string | null>(null);
  const [selectedConfigName, setSelectedConfigName] = useState<string | null>(null);

  const debugMutation = useMutation({
    mutationFn: luxCommands.debugWorkspaceInfo,
    onSuccess: (info) => {
      setDebugInfo(info);
      setDebugError(null);
      setSelectedConfigName((current) => current ?? info.configurations[0]?.name ?? null);
    },
    onError: (error) => setDebugError(readErrorMessage(error, t)),
  });

  useEffect(() => {
    if (!workspace) {
      setDebugInfo(null);
      setSelectedConfigName(null);
      return;
    }
    debugMutation.mutate();
  }, [workspace?.root]);

  const selectedConfiguration = debugInfo?.configurations.find((configuration) => configuration.name === selectedConfigName) ?? debugInfo?.configurations[0] ?? null;
  const selectedAdapter = selectedConfiguration
    ? debugInfo?.adapters.find((adapter) => adapter.id === selectedConfiguration.type || adapter.command === selectedConfiguration.type) ?? null
    : null;

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
            launchJsonPath={debugInfo.launch_json_path}
            setSelectedConfigName={setSelectedConfigName}
          />
          <DebugAdaptersBlock adapters={debugInfo.adapters} />
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
  launchJsonPath,
  setSelectedConfigName,
}: {
  adapter: DebugAdapterInfo | null;
  configuration: DebugConfiguration | null;
  configurations: DebugConfiguration[];
  launchJsonPath: string | null;
  setSelectedConfigName: (name: string) => void;
}) {
  const { t } = useTranslation();
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
      <button className="debug-run-button" type="button" disabled title={t("sidebar.runDebug.start.disabledTitle")}>
        <Play size={15} /> {t("sidebar.runDebug.startDebugging")}
      </button>
      <div className="debug-meta-list">
        <DebugMeta label={t("sidebar.runDebug.meta.configuration")} value={configuration ? `${configuration.request} / ${configuration.type}` : t("sidebar.runDebug.meta.notConfigured")} />
        <DebugMeta label={t("sidebar.runDebug.meta.adapter")} value={adapter ? `${adapter.name} (${adapter.status})` : configuration ? t("sidebar.runDebug.meta.noMatchingAdapter") : t("sidebar.runDebug.meta.notSelected")} tone={adapter?.status === "missing" ? "warning" : undefined} />
        <DebugMeta label="launch.json" value={launchJsonPath ?? t("sidebar.runDebug.meta.missingLaunchJson")} tone={launchJsonPath ? undefined : "muted"} />
      </div>
    </section>
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
              <small>{adapter.command}{adapter.args.length > 0 ? ` ${adapter.args.join(" ")}` : ""}</small>
            </span>
            <span className="debug-adapter-status">{adapter.status}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function DebugMeta({ label, tone, value }: { label: string; tone?: "muted" | "warning"; value: string }) {
  return (
    <div className="debug-meta-row" data-tone={tone}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
