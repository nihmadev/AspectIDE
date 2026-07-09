/**
 * Codex-style workspace skeleton: a shimmering ghost of the real IDE shell
 * (sidebar file tree + editor tabs + code lines) shown while a project loads, so
 * the user sees the app's shape forming instead of a blank void. Purely
 * decorative — `aria-hidden` keeps it out of the accessibility tree (the
 * ProjectLoadingStatus card carries the live status).
 */
export function WorkspaceSkeleton() {
  return (
    <div className="workspace-skeleton" aria-hidden="true">
      <aside className="ws-skeleton-sidebar">
        <div className="ws-skeleton-sidebar-head">
          <span className="ws-skeleton-bar" style={{ width: "46%" }} />
        </div>
        <div className="ws-skeleton-tree">
          {SKELETON_TREE.map((row, index) => (
            <div
              key={index}
              className="ws-skeleton-tree-row"
              style={{ paddingLeft: `${8 + row.depth * 14}px` }}
            >
              <span className="ws-skeleton-dot" />
              <span className="ws-skeleton-bar" style={{ width: `${row.width}%` }} />
            </div>
          ))}
        </div>
      </aside>

      <section className="ws-skeleton-editor">
        <div className="ws-skeleton-tabs">
          <span className="ws-skeleton-tab" data-active="true">
            <span className="ws-skeleton-dot" />
            <span className="ws-skeleton-bar" style={{ width: "54px" }} />
          </span>
          <span className="ws-skeleton-tab">
            <span className="ws-skeleton-bar" style={{ width: "44px" }} />
          </span>
          <span className="ws-skeleton-tab">
            <span className="ws-skeleton-bar" style={{ width: "62px" }} />
          </span>
        </div>
        <div className="ws-skeleton-code">
          {SKELETON_CODE.map((line, index) => (
            <div className="ws-skeleton-code-line" key={index}>
              <span className="ws-skeleton-gutter">{index + 1}</span>
              <span
                className="ws-skeleton-bar"
                style={{ width: `${line.width}%`, marginLeft: `${line.indent * 16}px` }}
              />
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

// A believable little file tree (depth + bar width per row).
const SKELETON_TREE = [
  { depth: 0, width: 52 },
  { depth: 1, width: 64 },
  { depth: 1, width: 48 },
  { depth: 2, width: 58 },
  { depth: 2, width: 42 },
  { depth: 1, width: 56 },
  { depth: 0, width: 40 },
  { depth: 1, width: 60 },
  { depth: 1, width: 46 },
  { depth: 0, width: 50 },
  { depth: 1, width: 38 },
] as const;

// A believable code body (indent steps + line widths).
const SKELETON_CODE = [
  { indent: 0, width: 42 },
  { indent: 0, width: 64 },
  { indent: 1, width: 56 },
  { indent: 1, width: 48 },
  { indent: 2, width: 60 },
  { indent: 2, width: 38 },
  { indent: 1, width: 52 },
  { indent: 0, width: 30 },
  { indent: 0, width: 58 },
  { indent: 1, width: 66 },
  { indent: 1, width: 44 },
  { indent: 2, width: 50 },
  { indent: 1, width: 36 },
  { indent: 0, width: 28 },
] as const;
