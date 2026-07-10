import { Check, ChevronDown, GitBranch, Loader2, Minus, Plus, Trash2, Undo2, ArrowDown, ArrowUp, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { displayPath, joinPath } from '../../lib/explorer/file-tree';
import { categorizeGitFile, gitDecoBadge, type GitDecoStatus } from '../../lib/explorer/git-decorations';
import { useTranslation } from '../../lib/i18n/useTranslation';
import { useLuxStore } from '../../lib/store';
import { luxCommands } from '../../lib/tauri/commands';
import type { GitFileStatus, GitStatus } from '../../lib/types';
import { GitDiffModal } from "./GitDiffModal";
import { readErrorMessage, TreeMessage } from "./SidebarShared";

type DiffCount = { additions: number; deletions: number };
type SectionAction = { icon: React.ReactNode; label: string; danger?: boolean; onClick: () => void };

// Fixed height of a single git file row + a section header row, in px (matches
// .git-file-open height and the .git-section-head sticky band in app.css).
const GIT_ROW_HEIGHT = 24;
const GIT_HEADER_HEIGHT = 28;
// Total changed/staged files above which the file lists render through a single
// windowed list instead of mounting every row. Below this the eager render with
// sticky section headers is kept verbatim вЂ” the common, fast case.
const GIT_VIRTUALIZE_THRESHOLD = 200;
const GIT_OVERSCAN = 12;

// One flattened, ready-to-render row of the source-control change list. Section
// headers and their stage/unstage/discard actions are carried inline so a single
// virtualizer can window headers and file rows together.
type GitRow =
  | { kind: "header"; key: string; title: string; count: number; action: SectionAction; secondaryAction?: SectionAction }
  | { kind: "file"; key: string; file: GitFileStatus; staged: boolean };

export function GitPanel() {
  const { t } = useTranslation();
  const gitStatus = useLuxStore((state) => state.gitStatus);
  const setGitStatus = useLuxStore((state) => state.setGitStatus);
  const workspace = useLuxStore((state) => state.workspace);

  const [commitMessage, setCommitMessage] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [diffCounts, setDiffCounts] = useState<Map<string, DiffCount>>(new Map());
  const [branchMenuOpen, setBranchMenuOpen] = useState(false);
  const [branches, setBranches] = useState<string[]>([]);
  const [diffTarget, setDiffTarget] = useState<{ path: string; name: string } | null>(null);
  const branchMenuRef = useRef<HTMLDivElement | null>(null);

  const files = useMemo(() => gitStatus?.files ?? [], [gitStatus]);
  const staged = useMemo(() => files.filter(isStaged), [files]);
  const changes = useMemo(() => files.filter(isChanged), [files]);
  const repoRoot = workspace?.root ?? null;

  // Stable signature of the current file-set; gitDiff is only refetched when
  // the set of changed/staged paths actually changes, not on every poll tick.
  const fileSetSignature = files.map((f) => `${f.path}:${f.index_status}:${f.worktree_status}`).join("|");
  const lastFetchedSignatureRef = useRef<string | null>(null);

  // Per-file +/в€’ line counts (HEAD vs working).  Uses a lightweight signature
  // to skip redundant full-diff fetches when only metadata (e.g. timestamps)
  // changed without altering which paths are modified.
  useEffect(() => {
    if (!gitStatus || files.length === 0) {
      setDiffCounts(new Map());
      lastFetchedSignatureRef.current = null;
      return;
    }
    // Skip if the file-set hasn't changed since the last successful fetch.
    if (lastFetchedSignatureRef.current === fileSetSignature) return;
    let cancelled = false;
    luxCommands.gitDiff()
      .then((diff) => {
        if (cancelled) return;
        lastFetchedSignatureRef.current = fileSetSignature;
        const map = new Map<string, DiffCount>();
        for (const file of diff.files) map.set(normalizeKey(file.path), { additions: file.additions, deletions: file.deletions });
        setDiffCounts(map);
      })
      .catch(() => { if (!cancelled) setDiffCounts(new Map()); });
    return () => { cancelled = true; };
  }, [gitStatus, fileSetSignature]); // eslint-disable-line react-hooks/exhaustive-deps

  // Close the branch menu on outside click / Escape.
  useEffect(() => {
    if (!branchMenuOpen) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!branchMenuRef.current?.contains(event.target as Node | null)) setBranchMenuOpen(false);
    };
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") setBranchMenuOpen(false); };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [branchMenuOpen]);

  const runOp = useCallback(async (label: string, op: () => Promise<GitStatus>) => {
    setBusy(label);
    setError(null);
    try {
      setGitStatus(await op());
    } catch (cause) {
      setError(readErrorMessage(cause, t));
    } finally {
      setBusy(null);
    }
  }, [setGitStatus, t]);

  const refresh = useCallback(() => void runOp("refresh", () => luxCommands.gitStatus()), [runOp]);
  const stage = useCallback((paths: string[]) => void runOp("stage", () => luxCommands.gitStage(paths)), [runOp]);
  const unstage = useCallback((paths: string[]) => void runOp("unstage", () => luxCommands.gitUnstage(paths)), [runOp]);
  const discard = useCallback((paths: string[], label: string) => {
    if (!window.confirm(t("sidebar.git.discardConfirm", { target: label }))) return;
    void runOp("discard", () => luxCommands.gitDiscard(paths));
  }, [runOp, t]);

  const commit = useCallback(() => {
    const message = commitMessage.trim();
    if (!message) return;
    void runOp("commit", async () => {
      const status = await luxCommands.gitCommit(message);
      setCommitMessage("");
      return status;
    });
  }, [commitMessage, runOp]);

  const commitAll = useCallback(() => {
    const message = commitMessage.trim();
    if (!message) return;
    void runOp("commit", async () => {
      await luxCommands.gitStage([]);
      const status = await luxCommands.gitCommit(message);
      setCommitMessage("");
      return status;
    });
  }, [commitMessage, runOp]);

  const toggleBranchMenu = useCallback(() => {
    setBranchMenuOpen((open) => {
      const next = !open;
      if (next) void luxCommands.gitBranches().then(setBranches).catch(() => setBranches([]));
      return next;
    });
  }, []);

  const switchBranch = useCallback((name: string) => {
    setBranchMenuOpen(false);
    void runOp("branch", () => luxCommands.gitCheckoutBranch(name));
  }, [runOp]);

  const createBranch = useCallback(() => {
    setBranchMenuOpen(false);
    const name = window.prompt(t("sidebar.git.newBranchPrompt"))?.trim();
    if (!name) return;
    void runOp("branch", () => luxCommands.gitCreateBranch(name));
  }, [runOp, t]);

  const openDiff = useCallback((file: GitFileStatus) => {
    setDiffTarget({ path: file.path, name: baseName(file.path) });
  }, []);

  // Flatten staged + changed sections into a single windowable row array so very
  // large repos (thousands of generated/changed files) render a bounded window
  // instead of every DOM row. Section headers carry their bulk actions inline.
  const gitRows = useMemo<GitRow[]>(() => {
    const rows: GitRow[] = [];
    if (staged.length > 0) {
      rows.push({
        kind: "header",
        key: "header-staged",
        title: t("sidebar.git.stagedChanges"),
        count: staged.length,
        action: { icon: <Minus size={13} />, label: t("sidebar.git.unstageAll"), onClick: () => unstage([]) },
      });
      for (const file of staged) rows.push({ kind: "file", key: `staged-${file.path}`, file, staged: true });
    }
    if (changes.length > 0) {
      rows.push({
        kind: "header",
        key: "header-changes",
        title: t("sidebar.git.changes"),
        count: changes.length,
        action: { icon: <Plus size={13} />, label: t("sidebar.git.stageAll"), onClick: () => stage([]) },
        secondaryAction: { icon: <Undo2 size={13} />, label: t("sidebar.git.discardAll"), danger: true, onClick: () => discard(changes.map((file) => file.path), t("sidebar.git.allChanges")) },
      });
      for (const file of changes) rows.push({ kind: "file", key: `changes-${file.path}`, file, staged: false });
    }
    return rows;
  }, [changes, discard, stage, staged, t, unstage]);
  const useVirtualGit = files.length >= GIT_VIRTUALIZE_THRESHOLD;

  const gitScrollRef = useRef<HTMLDivElement | null>(null);
  const gitVirtualizer = useVirtualizer({
    count: gitRows.length,
    getScrollElement: () => gitScrollRef.current,
    getItemKey: (index) => gitRows[index]?.key ?? index,
    estimateSize: (index) => (gitRows[index]?.kind === "header" ? GIT_HEADER_HEIGHT : GIT_ROW_HEIGHT),
    overscan: GIT_OVERSCAN,
  });

  if (!workspace) {
    return (
      <div className="panel-content git-panel-content">
        <div className="branch-summary"><GitBranch size={16} /><span>{t("sidebar.git.noWorkspace")}</span></div>
      </div>
    );
  }
  if (!gitStatus || gitStatus.branch === null) {
    return (
      <div className="panel-content git-panel-content">
        <div className="branch-summary"><GitBranch size={16} /><span>{t("sidebar.git.noRepository")}</span></div>
      </div>
    );
  }

  const ahead = gitStatus.ahead;
  const behind = gitStatus.behind;
  const canCommit = staged.length > 0 && commitMessage.trim().length > 0 && busy === null;

  return (
    <div className="panel-content git-panel-content">
      <div className="git-toolbar">
        <div className="git-branch" ref={branchMenuRef}>
          <button className="git-branch-button" type="button" onClick={toggleBranchMenu} title={t("sidebar.git.switchBranch")}>
            <GitBranch size={14} />
            <span>{gitStatus.branch}</span>
            <ChevronDown size={13} />
          </button>
          {branchMenuOpen && (
            <div className="git-branch-menu" role="menu">
              {branches.length === 0 ? (
                <div className="git-branch-menu-empty">{t("sidebar.git.noBranches")}</div>
              ) : branches.map((name) => (
                <button key={name} type="button" className="git-branch-menu-item" data-current={name === gitStatus.branch || undefined} onClick={() => switchBranch(name)}>
                  {name === gitStatus.branch && <Check size={12} />}
                  <span>{name}</span>
                </button>
              ))}
              <button type="button" className="git-branch-menu-item git-branch-menu-create" onClick={createBranch}>
                <Plus size={12} /><span>{t("sidebar.git.newBranch")}</span>
              </button>
            </div>
          )}
        </div>
        <div className="git-sync">
          <button className="git-sync-button" type="button" disabled={busy !== null} title={t("sidebar.git.pull")} onClick={() => void runOp("pull", () => luxCommands.gitPull())}>
            <ArrowDown size={13} />{behind > 0 && <span>{behind}</span>}
          </button>
          <button className="git-sync-button" type="button" disabled={busy !== null} title={t("sidebar.git.push")} onClick={() => void runOp("push", () => luxCommands.gitPush())}>
            <ArrowUp size={13} />{ahead > 0 && <span>{ahead}</span>}
          </button>
          <button className="git-sync-button" type="button" disabled={busy !== null} title={t("sidebar.git.refresh")} onClick={refresh}>
            {busy ? <Loader2 size={13} className="spin-icon" /> : <RefreshCw size={13} />}
          </button>
        </div>
      </div>

      <div className="git-commit-box">
        <textarea
          className="git-commit-input"
          value={commitMessage}
          placeholder={t("sidebar.git.commitPlaceholder")}
          rows={2}
          onChange={(event) => setCommitMessage(event.target.value)}
          onKeyDown={(event) => {
            if ((event.ctrlKey || event.metaKey) && event.key === "Enter") { event.preventDefault(); commit(); }
          }}
        />
        <div className="git-commit-actions">
          <button className="git-commit-button" type="button" disabled={!canCommit} onClick={commit}>
            {busy === "commit" ? <Loader2 size={13} className="spin-icon" /> : <Check size={13} />}
            <span>{t("sidebar.git.commit")}</span>
          </button>
          <button className="git-commit-button secondary" type="button" disabled={busy !== null || commitMessage.trim().length === 0 || files.length === 0} title={t("sidebar.git.commitAllHint")} onClick={commitAll}>
            {t("sidebar.git.commitAll")}
          </button>
        </div>
      </div>

      {error && <TreeMessage depth={0} tone="error" text={error} />}

      <div className="git-sections" ref={gitScrollRef}>
        {files.length === 0 ? (
          <div className="git-clean">
            <Check size={16} />
            <span>{t("sidebar.git.clean")}</span>
          </div>
        ) : useVirtualGit ? (
          <div className="git-virtual-list" style={{ height: gitVirtualizer.getTotalSize() }}>
            {gitVirtualizer.getVirtualItems().map((item) => {
              const row = gitRows[item.index];
              if (!row) return null;
              return (
                <div
                  key={item.key}
                  className="git-virtual-row"
                  data-index={item.index}
                  ref={gitVirtualizer.measureElement}
                  style={{ transform: `translateY(${item.start}px)` }}
                >
                  {row.kind === "header" ? (
                    <GitSectionHead title={row.title} count={row.count} action={row.action} secondaryAction={row.secondaryAction} />
                  ) : (
                    <GitFileRow
                      file={row.file}
                      staged={row.staged}
                      count={diffCounts.get(normalizeKey(row.file.path))}
                      onOpen={() => openDiff(row.file)}
                      onPrimary={() => (row.staged ? unstage([row.file.path]) : stage([row.file.path]))}
                      onDiscard={() => discard([row.file.path], baseName(row.file.path))}
                      t={t}
                    />
                  )}
                </div>
              );
            })}
          </div>
        ) : (
          <>
            {staged.length > 0 && (
              <GitSection
                title={t("sidebar.git.stagedChanges")}
                count={staged.length}
                action={{ icon: <Minus size={13} />, label: t("sidebar.git.unstageAll"), onClick: () => unstage([]) }}
              >
                {staged.map((file) => (
                  <GitFileRow
                    key={`staged-${file.path}`}
                    file={file}
                    staged
                    count={diffCounts.get(normalizeKey(file.path))}
                    onOpen={() => openDiff(file)}
                    onPrimary={() => unstage([file.path])}
                    onDiscard={() => discard([file.path], baseName(file.path))}
                    t={t}
                  />
                ))}
              </GitSection>
            )}
            {changes.length > 0 && (
              <GitSection
                title={t("sidebar.git.changes")}
                count={changes.length}
                action={{ icon: <Plus size={13} />, label: t("sidebar.git.stageAll"), onClick: () => stage([]) }}
                secondaryAction={{ icon: <Undo2 size={13} />, label: t("sidebar.git.discardAll"), danger: true, onClick: () => discard(changes.map((file) => file.path), t("sidebar.git.allChanges")) }}
              >
                {changes.map((file) => (
                  <GitFileRow
                    key={`changes-${file.path}`}
                    file={file}
                    count={diffCounts.get(normalizeKey(file.path))}
                    onOpen={() => openDiff(file)}
                    onPrimary={() => stage([file.path])}
                    onDiscard={() => discard([file.path], baseName(file.path))}
                    t={t}
                  />
                ))}
              </GitSection>
            )}
          </>
        )}
      </div>

      {diffTarget && (
        <GitDiffModal
          path={diffTarget.path}
          displayName={repoRoot ? displayPath(joinPath(repoRoot, diffTarget.path)) : diffTarget.name}
          onClose={() => setDiffTarget(null)}
        />
      )}
    </div>
  );
}

function GitSection({ title, count, action, secondaryAction, children }: {
  title: string;
  count: number;
  action?: SectionAction;
  secondaryAction?: SectionAction;
  children: React.ReactNode;
}) {
  return (
    <section className="git-section">
      <GitSectionHead title={title} count={count} action={action} secondaryAction={secondaryAction} />
      {children}
    </section>
  );
}

// Section header band (title, count badge, bulk stage/unstage/discard actions).
// Shared by the eager sectioned render and the virtualized header rows.
function GitSectionHead({ title, count, action, secondaryAction }: {
  title: string;
  count: number;
  action?: SectionAction;
  secondaryAction?: SectionAction;
}) {
  return (
    <header className="git-section-head">
      <h3>{title}</h3>
      <span className="git-section-count">{count}</span>
      <div className="git-section-actions">
        {secondaryAction && (
          <button type="button" data-danger={secondaryAction.danger || undefined} title={secondaryAction.label} aria-label={secondaryAction.label} onClick={secondaryAction.onClick}>
            {secondaryAction.icon}
          </button>
        )}
        {action && (
          <button type="button" title={action.label} aria-label={action.label} onClick={action.onClick}>
            {action.icon}
          </button>
        )}
      </div>
    </header>
  );
}

function GitFileRow({ file, staged = false, count, onOpen, onPrimary, onDiscard, t }: {
  file: GitFileStatus;
  staged?: boolean;
  count?: DiffCount;
  onOpen: () => void;
  onPrimary: () => void;
  onDiscard: () => void;
  t: ReturnType<typeof useTranslation>["t"];
}) {
  const code = (staged ? file.index_status : file.worktree_status).trim() || file.index_status.trim() || "M";
  const category: GitDecoStatus = staged
    ? categorizeGitFile(file.index_status, "")
    : categorizeGitFile("", file.worktree_status);
  const name = baseName(file.path);
  const dir = dirName(file.path);

  return (
    <div className="git-file-row" data-git={category}>
      <button className="git-file-open" type="button" title={file.path} onClick={onOpen}>
        <span className="git-file-letter">{badgeLetter(code, category)}</span>
        <span className="git-file-name">{name}</span>
        {dir && <span className="git-file-dir">{dir}</span>}
        {count && (count.additions > 0 || count.deletions > 0) && (
          <span className="git-file-counts">
            {count.additions > 0 && <span className="git-add">+{count.additions}</span>}
            {count.deletions > 0 && <span className="git-del">в€’{count.deletions}</span>}
          </span>
        )}
      </button>
      <div className="git-file-actions">
        <button type="button" className="git-file-danger" title={t("sidebar.git.discard")} aria-label={t("sidebar.git.discard")} onClick={onDiscard}>
          <Trash2 size={12} />
        </button>
        <button type="button" title={staged ? t("sidebar.git.unstage") : t("sidebar.git.stage")} aria-label={staged ? t("sidebar.git.unstage") : t("sidebar.git.stage")} onClick={onPrimary}>
          {staged ? <Minus size={13} /> : <Plus size={13} />}
        </button>
      </div>
    </div>
  );
}

function badgeLetter(code: string, category: GitDecoStatus): string {
  const trimmed = code.trim();
  if (trimmed && trimmed !== "?") return trimmed.toUpperCase();
  return gitDecoBadge(category);
}

function isStaged(file: GitFileStatus): boolean {
  const index = file.index_status.trim();
  return index.length > 0 && index !== "?";
}

function isChanged(file: GitFileStatus): boolean {
  return file.worktree_status.trim().length > 0;
}

function normalizeKey(path: string): string {
  return path.replace(/\\/g, "/").toLowerCase();
}

function baseName(path: string): string {
  const cleaned = path.replace(/\\/g, "/").replace(/\/+$/, "");
  const slash = cleaned.lastIndexOf("/");
  return slash === -1 ? cleaned : cleaned.slice(slash + 1);
}

function dirName(path: string): string {
  const cleaned = path.replace(/\\/g, "/").replace(/\/+$/, "");
  const slash = cleaned.lastIndexOf("/");
  return slash === -1 ? "" : cleaned.slice(0, slash);
}

