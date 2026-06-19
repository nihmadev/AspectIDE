import type { MessageKey } from "./i18n";
import type { AiIndexState, AiIndexStatus, ProjectLoadStage, ProjectLoadState } from "./store";

export type ProjectLoadChecklistItem = {
  active: boolean;
  done: boolean;
  key: MessageKey;
};

export type ProjectLoadDetail = {
  key: MessageKey;
  params?: Record<string, string | number>;
};

export type ProjectLoadSummary = ProjectLoadState & {
  checklist: ProjectLoadChecklistItem[];
  detail: ProjectLoadDetail | null;
  labelKey: MessageKey;
};

type ProjectLoadSignals = {
  aiIndex: Pick<AiIndexState, "indexedFiles" | "progress" | "status" | "totalFiles">;
  aiIndexStatus: AiIndexStatus;
  fileEntryCount: number;
  fileTreeLoading: boolean;
  languageServerCount: number;
  languageServersLoading: boolean;
  projectIndexingEnabled: boolean;
  projectLoad: ProjectLoadState;
};

export function buildProjectLoadSummary(signals: ProjectLoadSignals): ProjectLoadSummary {
  const stage = deriveProjectLoadStage(signals);
  const progress = deriveProjectLoadProgress({ ...signals, stage });
  const active = stage !== "idle" && stage !== "ready" && stage !== "error";

  return {
    ...signals.projectLoad,
    active,
    progress,
    stage,
    labelKey: projectLoadLabelKey(stage),
    detail: buildProjectLoadDetail(stage, signals),
    checklist: buildProjectLoadChecklist(stage, progress, signals),
  };
}

function deriveProjectLoadStage({
  aiIndexStatus,
  fileTreeLoading,
  projectIndexingEnabled,
  projectLoad,
}: ProjectLoadSignals): ProjectLoadStage {
  if (projectLoad.stage === "error") return "error";
  if (projectLoad.stage === "opening" && projectLoad.active) return "opening";
  // Only the INITIAL workspace scan drives the full-screen splash. On open,
  // projectLoad.stage is set to "files" and flips to "ready"/"indexing" once the
  // scan's finally() runs. Incremental file-tree refreshes (delete/create/rename/
  // paste/move all flip the low-level `fileTreeLoading` flag too) happen while the
  // project is already "ready"/"indexing" — they must stay in the background and
  // never re-raise the overlay. Gating on stage === "files" is what keeps them quiet.
  if (fileTreeLoading && projectLoad.stage === "files") return "files";
  // Language servers start (and may be missing / installing) entirely in the
  // background — they MUST NOT gate the loading screen. Previously a stuck
  // `languageServersLoading` flag, or the old `"services"` fallback, left the
  // splash on "Starting language services" forever. Files ready ⇒ project ready
  // (or indexing); LSP fills in asynchronously via its own store slice.
  if (projectIndexingEnabled && aiIndexStatus === "indexing") return "indexing";
  return "ready";
}

function buildProjectLoadChecklist(
  stage: ProjectLoadStage,
  progress: number,
  signals: ProjectLoadSignals,
): ProjectLoadChecklistItem[] {
  const indexingDone =
    !signals.projectIndexingEnabled || signals.aiIndexStatus === "ready" || stage === "ready";
  const indexingActive =
    signals.projectIndexingEnabled && (stage === "indexing" || signals.aiIndexStatus === "indexing");
  const steps: ProjectLoadChecklistItem[] = [
    { key: "projectLoading.step.opening", active: stage === "opening", done: progress >= 28 || stage !== "opening" },
    { key: "projectLoading.step.files", active: stage === "files", done: !signals.fileTreeLoading && progress >= 34 },
    // Language services never block the splash: this step is informational, marked
    // done once files are scanned (LSP keeps starting in the background after ready).
    { key: "projectLoading.step.services", active: false, done: !signals.fileTreeLoading },
  ];
  if (signals.projectIndexingEnabled) {
    steps.push({
      key: "projectLoading.step.index",
      active: indexingActive,
      done: indexingDone,
    });
  }
  return steps;
}

function deriveProjectLoadProgress({
  aiIndex,
  fileTreeLoading,
  projectIndexingEnabled,
  projectLoad,
  stage,
}: ProjectLoadSignals & { stage: ProjectLoadStage }): number {
  if (stage === "error") return projectLoad.progress;
  if (stage === "opening") return Math.max(projectLoad.progress, 8);
  if (fileTreeLoading) return Math.max(projectLoad.progress, 34);
  if (stage === "indexing" && projectIndexingEnabled) {
    const indexSlice = Math.min(100, Math.max(0, aiIndex.progress)) * 0.2;
    return Math.max(projectLoad.progress, 78 + indexSlice);
  }
  if (stage === "ready") return 100;
  return projectLoad.progress;
}

function buildProjectLoadDetail(stage: ProjectLoadStage, signals: ProjectLoadSignals): ProjectLoadDetail | null {
  if (stage === "files") {
    if (signals.fileEntryCount > 0) {
      return { key: "projectLoading.detail.filesReady", params: { count: signals.fileEntryCount } };
    }
    return { key: "projectLoading.detail.scanningTree" };
  }
  if (stage === "services") {
    if (signals.languageServerCount > 0) {
      return {
        key: "projectLoading.detail.languageServers",
        params: { count: signals.languageServerCount },
      };
    }
    return { key: "projectLoading.detail.startingLanguageServers" };
  }
  if (stage === "indexing" && signals.projectIndexingEnabled) {
    const { indexedFiles, totalFiles } = signals.aiIndex;
    if (totalFiles > 0) {
      return {
        key: "projectLoading.detail.indexProgress",
        params: { done: indexedFiles, total: totalFiles },
      };
    }
    return { key: "projectLoading.detail.indexScanning" };
  }
  if (stage === "opening") return { key: "projectLoading.detail.resolvingWorkspace" };
  return null;
}

function projectLoadLabelKey(stage: ProjectLoadStage): MessageKey {
  if (stage === "opening") return "projectLoading.opening";
  if (stage === "files") return "projectLoading.files";
  if (stage === "services") return "projectLoading.services";
  if (stage === "indexing") return "projectLoading.indexing";
  if (stage === "error") return "projectLoading.error";
  return "projectLoading.ready";
}