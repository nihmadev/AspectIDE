import mermaid from "mermaid";

let mermaidReady = false;

function ensureMermaid() {
  if (mermaidReady) return;
  mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "strict" });
  mermaidReady = true;
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
      ensureMermaid();
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