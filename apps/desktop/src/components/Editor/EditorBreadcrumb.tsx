import { ChevronDown, ChevronRight } from "lucide-react";
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fileIconForName } from '../../lib/explorer/file-icons';
import { buildFileTreeDirectories, displayPath, normalizePath, sortFsEntries } from '../../lib/explorer/file-tree';
import { useLuxStore } from '../../lib/store/index';
import { luxCommands } from '../../lib/tauri/commands';
import type { FsEntry } from '../../lib/types/index';

type CrumbSegment = {
  label: string;
  fullPath: string;
  isFile: boolean;
};

function buildSegments(documentPath: string, workspaceRoot: string | null): CrumbSegment[] {
  const normDoc = displayPath(documentPath);
  let relative = normDoc;
  if (workspaceRoot) {
    const normRoot = displayPath(workspaceRoot).replace(/\/+$/, "");
    if (normalizePath(normDoc).startsWith(normalizePath(normRoot) + "/")) {
      relative = normDoc.slice(normRoot.length + 1);
    }
  }
  const parts = relative.split("/");
  let acc = workspaceRoot ? displayPath(workspaceRoot) : "";
  return parts.map((part, i) => {
    acc = acc ? `${acc}/${part}` : part;
    return { label: part, fullPath: acc, isFile: i === parts.length - 1 };
  });
}

export function EditorBreadcrumb({
  documentPath,
  workspaceRoot,
}: {
  documentPath: string;
  workspaceRoot: string | null;
}) {
  const upsertDocument = useLuxStore((s) => s.upsertDocument);
  const segments = buildSegments(documentPath, workspaceRoot);
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  const [popupAnchor, setPopupAnchor] = useState<DOMRect | null>(null);
  const btnRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const openFile = useCallback(
    async (path: string) => {
      try {
        const doc = await luxCommands.editorOpenFile(path);
        upsertDocument(doc);
      } catch { /* ignore */ }
    },
    [upsertDocument],
  );

  const handleClick = (i: number, isFile: boolean) => {
    if (isFile) return;
    if (openIndex === i) { setOpenIndex(null); return; }
    const btn = btnRefs.current[i];
    if (btn) setPopupAnchor(btn.getBoundingClientRect());
    setOpenIndex(i);
  };

  if (segments.length <= 1) return null;

  return (
    <div className="editor-breadcrumb">
      {segments.map((seg, i) => (
        <Fragment key={i}>
          {i > 0 && <ChevronRight size={11} className="breadcrumb-sep" />}
          <button
            ref={(el) => { btnRefs.current[i] = el; }}
            className="breadcrumb-segment"
            type="button"
            data-file={seg.isFile || undefined}
            onClick={() => handleClick(i, seg.isFile)}
            title={seg.fullPath}
          >
            {seg.isFile && (
              (() => {
                const m = fileIconForName(seg.label);
                return m.imgSrc
                  ? <img src={m.imgSrc} width={14} height={14} className={m.className} alt="" />
                  : <m.Icon size={14} className={m.className} />;
              })()
            )}
            <span>{seg.label}</span>
          </button>
          {openIndex === i && popupAnchor && (
            <BreadcrumbPopup
              dirPath={seg.fullPath}
              anchorRect={popupAnchor}
              onOpenFile={openFile}
              onClose={() => setOpenIndex(null)}
            />
          )}
        </Fragment>
      ))}
    </div>
  );
}

type FlatRow = {
  entry: FsEntry;
  depth: number;
};

function BreadcrumbPopup({
  dirPath,
  anchorRect,
  onOpenFile,
  onClose,
}: {
  dirPath: string;
  anchorRect: DOMRect;
  onOpenFile: (path: string) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [position, setPosition] = useState({ left: anchorRect.left, top: anchorRect.bottom + 4 });
  const [dirs, setDirs] = useState<Map<string, FsEntry[]>>(new Map());
  const [loading, setLoading] = useState(true);
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());

  useEffect(() => {
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      onClose();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", close);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", close);
    };
  }, [onClose]);

  useEffect(() => {
    setLoading(true);
    setExpandedPaths(new Set());
    luxCommands.fsReadTree(dirPath)
      .then((entries) => {
        const dirs = buildFileTreeDirectories(dirPath, entries);
        setDirs(new Map(Object.entries(dirs)));
        setLoading(false);
      })
      .catch(() => setLoading(false));
  }, [dirPath]);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setPosition({
      left: Math.max(4, Math.min(anchorRect.left, window.innerWidth - rect.width - 8)),
      top: Math.min(anchorRect.bottom + 4, window.innerHeight - rect.height - 8),
    });
  }, [anchorRect, dirs, loading]);

  const toggleExpand = useCallback((path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const rootEntries = dirs.get(normalizePath(dirPath)) ?? [];
  const rows = useMemo(
    () => flattenTree(rootEntries, expandedPaths, dirs),
    [rootEntries, expandedPaths, dirs],
  );

  return (
    <div
      className="breadcrumb-popup"
      ref={ref}
      style={{ left: position.left, top: position.top }}
      onPointerDown={(e) => e.stopPropagation()}
    >
      {loading ? (
        <div className="breadcrumb-popup-loading">LoadingвЂ¦</div>
      ) : (
        <>
          {rows.map((row) => (
            <button
              className="breadcrumb-popup-item"
              type="button"
              key={row.entry.path}
              style={{ paddingLeft: `${6 + row.depth * 12}px` }}
              onClick={() => {
                if (row.entry.kind === "directory") {
                  const children = dirs.get(normalizePath(row.entry.path));
                  if (children && children.length > 0) toggleExpand(row.entry.path);
                } else {
                  onClose();
                  onOpenFile(row.entry.path);
                }
              }}
            >
              <span className="breadcrumb-popup-chevron">
                {row.entry.kind === "directory" && (
                  dirs.get(normalizePath(row.entry.path))?.length
                    ? (expandedPaths.has(row.entry.path) ? <ChevronDown size={12} /> : <ChevronRight size={12} />)
                    : <span className="breadcrumb-popup-chevron-empty" />
                )}
              </span>
              {row.entry.kind === "file" && (
                (() => {
                  const m = fileIconForName(row.entry.name);
                  return m.imgSrc
                    ? <img src={m.imgSrc} width={14} height={14} className={m.className} alt="" />
                    : <m.Icon size={14} className={m.className} />;
                })()
              )}
              <span className="breadcrumb-popup-item-name">{row.entry.name}</span>
            </button>
          ))}
          {!loading && rows.length === 0 && (
            <div className="breadcrumb-popup-empty">Empty directory</div>
          )}
        </>
      )}
    </div>
  );
}

function flattenTree(
  entries: FsEntry[],
  expandedPaths: Set<string>,
  dirs: Map<string, FsEntry[]>,
  depth = 0,
): FlatRow[] {
  const result: FlatRow[] = [];
  for (const entry of entries) {
    result.push({ entry, depth });
    if (entry.kind === "directory" && expandedPaths.has(entry.path)) {
      const children = dirs.get(normalizePath(entry.path));
      if (children) {
        result.push(...flattenTree(children, expandedPaths, dirs, depth + 1));
      }
    }
  }
  return result;
}
