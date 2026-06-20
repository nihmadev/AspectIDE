import { Code2, Copy, Eye } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type ArtifactView = "preview" | "code";

type HtmlArtifactProps = {
  /** Raw HTML document (or fragment) authored by the model. */
  html: string;
  /** Caption label (e.g. the fenced info-string after `html`, or a tool title). */
  title?: string;
  /** `html preview`/`html live` open in Preview; bare `html` opens in Code (one click to render). */
  autoPreview?: boolean;
  /** False while the fenced block is still streaming — keep code until the turn settles. */
  settled?: boolean;
  t: TranslateFn;
};

/**
 * The single source of truth for rendering model-authored HTML/CSS/JS (including 3D
 * via canvas/WebGL/three.js) live inside chat. The preview runs in an iframe with
 * `sandbox="allow-scripts"` and NO `allow-same-origin`, so the document gets an
 * opaque origin: scripts and CDN/network requests work (needed for 3D libs), but it
 * can never reach the host app's DOM, storage, cookies, or the Tauri bridge. Used
 * both for fenced ```html blocks in messages and for the AskUser `htmlPreview` field.
 */
export function HtmlArtifact({ html, title, autoPreview = false, settled = true, t }: HtmlArtifactProps) {
  // Never mount the iframe while the block is still streaming: a half-written
  // <script> throws inside the sandbox and the frame reloads on every token.
  const canRender = settled && html.trim().length > 0;
  const [view, setView] = useState<ArtifactView>(autoPreview ? "preview" : "code");
  const showPreview = view === "preview" && canRender;
  const label = title?.trim() || t("aiChat.artifact.title");

  // Performance: don't pay for the iframe (parse + run scripts + WebGL/rAF loop)
  // until the artifact actually scrolls near the viewport. We mount on first
  // intersection (with a margin so it's ready just before it's seen) and then KEEP
  // it mounted — re-scrolling to it is instant and stateful, never a reload. So an
  // artifact the user never scrolls to costs nothing.
  const ref = useRef<HTMLElement | null>(null);
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    if (seen) return;
    const el = ref.current;
    if (!el) return;
    if (typeof IntersectionObserver === "undefined") {
      setSeen(true);
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setSeen(true);
          io.disconnect();
        }
      },
      { rootMargin: "300px 0px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [seen]);

  return (
    <figure ref={ref} className="ai-artifact" data-view={showPreview ? "preview" : "code"}>
      <figcaption className="ai-artifact-bar">
        <span className="ai-artifact-title" title={label}>{label}</span>
        <div className="ai-artifact-actions">
          <div className="ai-artifact-toggle" role="tablist" aria-label={t("aiChat.artifact.title")}>
            <button
              type="button"
              role="tab"
              aria-selected={view === "preview"}
              data-active={view === "preview" || undefined}
              onClick={() => setView("preview")}
            >
              <Eye size={12} />
              {t("aiChat.artifact.preview")}
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={view === "code"}
              data-active={view === "code" || undefined}
              onClick={() => setView("code")}
            >
              <Code2 size={12} />
              {t("aiChat.artifact.code")}
            </button>
          </div>
          <button
            type="button"
            className="ai-artifact-copy"
            aria-label={t("aiChat.artifact.copy")}
            title={t("aiChat.artifact.copy")}
            onClick={() => void copyArtifactHtml(html)}
          >
            <Copy size={12} />
          </button>
        </div>
      </figcaption>
      {showPreview ? (
        seen ? (
          <iframe
            className="ai-artifact-frame"
            title={label}
            srcDoc={html}
            // SECURITY: allow-scripts ONLY. No allow-same-origin → opaque origin, so the
            // document cannot touch host storage/cookies/DOM/Tauri even though it may
            // fetch CDN scripts. Do NOT add allow-same-origin / allow-top-navigation.
            sandbox="allow-scripts"
            loading="lazy"
            referrerPolicy="no-referrer"
          />
        ) : (
          // Reserve the frame's height so mounting on scroll-in causes no layout shift.
          <div className="ai-artifact-frame ai-artifact-frame-idle" aria-hidden="true" />
        )
      ) : (
        <pre className="ai-chat-code-block ai-artifact-code" data-language="html">
          <code>{html}</code>
        </pre>
      )}
      {view === "preview" && !canRender && (
        <div className="ai-artifact-pending">{t("aiChat.artifact.rendering")}</div>
      )}
    </figure>
  );
}

function copyArtifactHtml(html: string): Promise<void> {
  const clipboard = navigator.clipboard;
  if (!clipboard) return Promise.resolve();
  return clipboard.writeText(html).catch(() => undefined);
}
