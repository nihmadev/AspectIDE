import { Loader2, Package, Play, RefreshCw, Search, ShieldAlert } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { useTranslation, type TranslateFn } from "../../lib/i18n/useTranslation";
import { useAspectStore } from "../../lib/store";
import { aspectCommands } from "../../lib/tauri";
import type { ExtensionActivationPlan, ExtensionContributionRegistry, ExtensionInfo } from "../../lib/types";
import { readErrorMessage, TreeMessage } from "./SidebarShared";

export function ExtensionsPanel() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [openError, setOpenError] = useState<string | null>(null);
  const upsertDocument = useAspectStore((state) => state.upsertDocument);

  const extensionsMutation = useMutation({
    mutationFn: aspectCommands.extensionsList,
  });

  const activationPlanMutation = useMutation({
    mutationFn: aspectCommands.extensionsActivationPlan,
  });

  const contributionRegistryMutation = useMutation({
    mutationFn: aspectCommands.extensionsContributionRegistry,
  });

  const openExtensionManifestMutation = useMutation({
    mutationFn: (extension: ExtensionInfo) => aspectCommands.editorOpenFile(extension.manifest_path),
    onSuccess: (document) => {
      setOpenError(null);
      upsertDocument(document);
    },
    onError: (error) => setOpenError(readErrorMessage(error, t)),
  });

  useEffect(() => {
    extensionsMutation.mutate();
    activationPlanMutation.mutate();
  }, []);

  const extensions = extensionsMutation.data ?? [];
  const activationPlan = activationPlanMutation.data ?? null;
  const visibleExtensions = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) return extensions;
    return extensions.filter((extension) =>
      `${extension.name} ${extension.id} ${extension.version} ${extension.contributes.join(" ")} ${extension.contribution_points.map((point) => point.id).join(" ")}`
        .toLowerCase()
        .includes(normalizedQuery),
    );
  }, [extensions, query]);

  return (
    <div className="panel-content extensions-panel-content utility-panel-content">
      <form
        className="search-form extensions-search-form"
        onSubmit={(event) => {
          event.preventDefault();
          extensionsMutation.mutate();
          activationPlanMutation.mutate();
          contributionRegistryMutation.reset();
        }}
      >
        <Search size={15} />
        <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("sidebar.extensions.search.placeholder")} />
      </form>
      <div className="panel-caption extensions-caption">
        <span>{extensionCaption(extensionsMutation.isPending, visibleExtensions.length, extensions.length, t)}</span>
        <button
          className="icon-button compact"
          type="button"
          aria-label={t("sidebar.extensions.actions.refresh")}
          title={t("sidebar.extensions.actions.refresh")}
          onClick={() => {
            extensionsMutation.mutate();
            activationPlanMutation.mutate();
            contributionRegistryMutation.reset();
          }}
          disabled={extensionsMutation.isPending || activationPlanMutation.isPending || contributionRegistryMutation.isPending}
        >
          {extensionsMutation.isPending ? <Loader2 size={14} className="spin-icon" /> : <RefreshCw size={14} />}
        </button>
      </div>
      <div className="extensions-caption extensions-activation-caption">
        <span>{activationPlanCaption(activationPlanMutation.isPending, activationPlan, t)}</span>
        <button
          className="icon-button compact"
          type="button"
          aria-label={t("sidebar.extensions.actions.activate")}
          title={t("sidebar.extensions.actions.activate")}
          onClick={() => contributionRegistryMutation.mutate()}
          disabled={!activationPlan || activationPlan.candidates.length === 0 || contributionRegistryMutation.isPending}
        >
          {contributionRegistryMutation.isPending ? <Loader2 size={14} className="spin-icon" /> : <Play size={14} />}
        </button>
      </div>
      <div className="extensions-caption extensions-runtime-caption">
        <span>{contributionRegistryCaption(contributionRegistryMutation.isPending, contributionRegistryMutation.data ?? null, t)}</span>
      </div>
      <div className="extensions-list" role="list" aria-label={t("sidebar.extensions.installed")}>
        {extensionsMutation.error ? <TreeMessage depth={0} tone="error" text={readErrorMessage(extensionsMutation.error, t)} /> : null}
        {activationPlanMutation.error ? <TreeMessage depth={0} tone="error" text={readErrorMessage(activationPlanMutation.error, t)} /> : null}
        {contributionRegistryMutation.error ? <TreeMessage depth={0} tone="error" text={readErrorMessage(contributionRegistryMutation.error, t)} /> : null}
        {openError ? <TreeMessage depth={0} tone="error" text={openError} /> : null}
        {!extensionsMutation.isPending && visibleExtensions.length === 0 ? (
          <div className="extensions-empty-state">
            <Package size={17} />
            <span>{query.trim() ? t("sidebar.extensions.empty.noSearchMatches") : t("sidebar.extensions.empty.noManifests")}</span>
          </div>
        ) : null}
        {visibleExtensions.map((extension) => (
          <ExtensionRow
            extension={extension}
            key={`${extension.root}-${extension.id}`}
            openManifest={() => openExtensionManifestMutation.mutate(extension)}
          />
        ))}
      </div>
    </div>
  );
}

function ExtensionRow({ extension, openManifest }: { extension: ExtensionInfo; openManifest: () => void }) {
  const { t } = useTranslation();
  const invalid = extension.status === "invalid";
  const active = extension.status === "active";

  return (
    <button className="extension-row" type="button" role="listitem" data-invalid={invalid} data-active={active} title={extension.error ?? extension.manifest_path} onClick={openManifest}>
      <span className="extension-row-icon">{invalid ? <ShieldAlert size={16} /> : <Package size={16} />}</span>
      <span className="extension-row-main">
        <span className="extension-row-title">
          <strong>{extension.name}</strong>
          <small>{extension.version}</small>
        </span>
        <span className="extension-row-id">{extension.id}</span>
        <span className="extension-row-contributes">{formatExtensionContributes(extension, t)}</span>
      </span>
      <span className="extension-status" data-invalid={invalid} data-active={active}>{extensionStatusLabel(extension.status, t)}</span>
    </button>
  );
}

function extensionStatusLabel(status: ExtensionInfo["status"], t: TranslateFn) {
  if (status === "active") return t("sidebar.extensions.status.active");
  if (status === "invalid") return t("sidebar.extensions.status.invalid");
  return t("sidebar.extensions.status.discovered");
}

function extensionCaption(loading: boolean, visibleCount: number, totalCount: number, t: TranslateFn) {
  if (loading) return t("sidebar.extensions.scanning");
  if (totalCount === 0) return t("sidebar.extensions.installed");
  if (visibleCount === totalCount) return t("sidebar.extensions.installedCount", { count: totalCount });
  return t("sidebar.extensions.visibleCount", { visibleCount, totalCount });
}

function activationPlanCaption(loading: boolean, plan: ExtensionActivationPlan | null, t: TranslateFn) {
  if (loading) return t("sidebar.extensions.activation.scanning");
  if (!plan) return t("sidebar.extensions.activation.notLoaded");
  if (plan.candidates.length === 0 && plan.blocked.length === 0) return t("sidebar.extensions.activation.empty");
  const abiVersions = [...new Set(plan.candidates.map((candidate) => candidate.host_contract.abi.version))].join(", ") || "-";
  const imports = plan.candidates.reduce((total, candidate) => total + candidate.host_contract.abi.imports.length, 0);
  return t("sidebar.extensions.activation.summary", {
    candidates: plan.candidates.length,
    blocked: plan.blocked.length,
    abiVersions,
    imports,
  });
}

function contributionRegistryCaption(loading: boolean, registry: ExtensionContributionRegistry | null, t: TranslateFn) {
  if (loading) return t("sidebar.extensions.activation.running");
  if (!registry) return t("sidebar.extensions.activation.runtimeNotRun");
  return t("sidebar.extensions.activation.registrySummary", {
    registered: registry.registered.length,
    unavailable: registry.unavailable.length,
    activated: registry.activation.activated.length,
    failed: registry.activation.failed.length,
    blocked: registry.activation.plan.blocked.length,
  });
}

function formatExtensionContributes(extension: ExtensionInfo, t: TranslateFn) {
  if (extension.error) return extension.error;
  if (extension.contribution_points.length === 0) return t("sidebar.extensions.noContributionPoints");
  return extension.contribution_points.map((point) => point.id).join(", ");
}
