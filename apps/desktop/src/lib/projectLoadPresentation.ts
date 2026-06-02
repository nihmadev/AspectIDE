import type { MessageKey } from "./i18n";
import type { AiIndexStatus, ProjectLoadStage, ProjectLoadState } from "./store";

export type ProjectLoadChecklistItem = {
  active: boolean;
  done: boolean;
  key: MessageKey;
};

export type ProjectLoadSummary = ProjectLoadState & {
  checklist: ProjectLoadChecklistItem[];
  labelKey: MessageKey;
};

type ProjectLoadSignals = {
  aiIndexStatus: AiIndexStatus;
  fileTreeLoading: boolean;
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
    checklist: [
      { key: "projectLoading.step.opening", active: stage === "opening", done: progress >= 28 },
      { key: "projectLoading.step.files", active: stage === "files", done: !signals.fileTreeLoading && progress >= 56 },
      { key: "projectLoading.step.services", active: stage === "services", done: !signals.languageServersLoading && progress >= 68 },
      { key: "projectLoading.step.index", active: false, done: true },
    ],
  };
}

function deriveProjectLoadStage({ aiIndexStatus, fileTreeLoading, languageServersLoading, projectIndexingEnabled, projectLoad }: ProjectLoadSignals): ProjectLoadStage {
  if (projectLoad.stage === "error") return "error";
  if (projectLoad.stage === "opening" && projectLoad.active) return "opening";
  if (fileTreeLoading) return "files";
  if (languageServersLoading) return "services";
  // AI indexing is a background operation; do not block UI
  // if (projectIndexingEnabled && aiIndexStatus === "indexing") return "indexing";
  if (projectLoad.active && projectLoad.stage !== "indexing") return projectLoad.stage;
  return projectLoad.stage === "ready" ? "ready" : "idle";
}

function deriveProjectLoadProgress({ aiIndexStatus, fileTreeLoading, languageServersLoading, projectIndexingEnabled, projectLoad, stage }: ProjectLoadSignals & { stage: ProjectLoadStage }): number {
  if (stage === "error") return projectLoad.progress;
  if (stage === "opening") return Math.max(projectLoad.progress, 8);
  if (fileTreeLoading) return Math.max(projectLoad.progress, 34);
  if (languageServersLoading) return Math.max(projectLoad.progress, 58);
  // Background indexing does not block progress
  // if (projectIndexingEnabled && aiIndexStatus === "indexing") return Math.max(projectLoad.progress, 76);
  if (stage === "ready") return 100;
  return projectLoad.progress;
}

function projectLoadLabelKey(stage: ProjectLoadStage): MessageKey {
  if (stage === "opening") return "projectLoading.opening";
  if (stage === "files") return "projectLoading.files";
  if (stage === "services") return "projectLoading.services";
  if (stage === "indexing") return "projectLoading.indexing";
  if (stage === "error") return "projectLoading.error";
  return "projectLoading.ready";
}
