// `mermaid` (+ its cytoscape/d3/dagre transitive tree, ~2.6 MB) is loaded ONLY
// on first diagram render via dynamic import, so it never lands in the eager
// startup graph. The import promise is cached so the chunk is fetched + the
// engine initialized exactly once.
type MermaidModule = typeof import("mermaid")["default"];
let mermaidPromise: Promise<MermaidModule> | null = null;

function loadMermaid(): Promise<MermaidModule> {
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then((module) => {
      const mermaid = module.default;
      mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "strict" });
      return mermaid;
    });
  }
  return mermaidPromise;
}

export function isMermaidDiagramPath(path: string | null) {
  if (!path) return false;
  const lower = path.toLowerCase();
  return lower.endsWith(".mmd") || lower.endsWith(".mermaid");
}

export async function renderDiagramPreview(source: string, path: string | null) {
  const trimmed = source.trim();
  if (!trimmed) {
    return { html: "<p class=\"diagram-preview-empty\">Empty diagram source.</p>", error: null as string | null };
  }

  if (isMermaidDiagramPath(path)) {
    try {
      const mermaid = await loadMermaid();
      const id = `lux-mermaid-${Math.random().toString(36).slice(2)}`;
      const { svg } = await mermaid.render(id, trimmed);
      return { html: `<div class="diagram-preview-mermaid">${svg}</div>`, error: null };
    } catch (error) {
      return {
        html: `<pre class="diagram-preview-source">${escapeHtml(trimmed)}</pre>`,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }

  return {
    html: `<pre class="diagram-preview-source">${escapeHtml(trimmed)}</pre><p class="diagram-preview-note">Live render is bundled for Mermaid (.mmd). PlantUML, Graphviz DOT, and Draw.io sources are edited as text; use InspectFile or an external renderer for complex layouts.</p>`,
    error: null,
  };
}

function escapeHtml(value: string) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}