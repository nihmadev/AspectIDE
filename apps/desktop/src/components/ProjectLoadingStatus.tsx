import { useEffect, useRef } from "react";
import type { ProjectLoadSummary } from "../lib/projectLoadPresentation";
import { useTranslation } from "../lib/i18n/useTranslation";

type ProjectLoadingStatusProps = {
  onDismissError: () => void;
  summary: ProjectLoadSummary;
};

type FallingFileParticle = {
  sprite: HTMLCanvasElement;
  alpha: number;
  drift: number;
  extension: string;
  height: number;
  life: number;
  maxLife: number;
  rotation: number;
  rotationSpeed: number;
  speed: number;
  width: number;
  x: number;
  y: number;
};

const FALLING_FILE_EXTENSIONS = [".rs", ".ts", ".tsx", ".json", ".toml", ".md", ".css", ".lock", ".yml", ".env"];

export function ProjectLoadingStatus({ onDismissError, summary }: ProjectLoadingStatusProps) {
  const { t } = useTranslation();
  if (!summary.active && summary.stage !== "error") return null;

  const boundedProgress = Math.max(0, Math.min(100, summary.progress));
  const progressLabel = t("projectLoading.progressLabel", { progress: Math.round(boundedProgress) });

  return (
    <div className="project-loading-overlay" data-stage={summary.stage} role="status" aria-live="polite" aria-busy={summary.active}>
      <FallingFiles active={summary.active} />
      <div className="project-loading-vignette" />
      <section className="project-loading-content" aria-label={t("projectLoading.screenLabel")}>
        <div className="project-loading-status">
          <div className="project-loading-label">{t(summary.labelKey)}</div>
          <div className="project-loading-path">{summary.workspaceName ?? summary.root ?? t("projectLoading.workspacePending")}</div>
        </div>
        <div className="project-loading-bar-track" aria-label={progressLabel} aria-valuemax={100} aria-valuemin={0} aria-valuenow={Math.round(boundedProgress)} role="progressbar">
          <div className="project-loading-bar" style={{ width: boundedProgress + "%" }} />
        </div>
        <div className="project-loading-percent">{Math.round(boundedProgress)}%</div>
      </section>
      {summary.error && (
        <div className="project-loading-error-block">
          <p>{summary.error}</p>
          <button type="button" onClick={onDismissError}>{t("projectLoading.dismissError")}</button>
        </div>
      )}
    </div>
  );
}

function FallingFiles({ active }: { active: boolean }) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !active) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const prefersReducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    let width = 0;
    let height = 0;
    let raf = 0;
    let lastTime = 0;
    let spawnAccumulator = 0;
    let particles: FallingFileParticle[] = [];

    /* ── Sprite cache: pre-render each unique file card once ── */
    const spriteCache = new Map<string, HTMLCanvasElement>();
    const drawCtx = ctx;

    function buildSprite(ext: string, w: number, h: number): HTMLCanvasElement {
      const key = ext + "|" + w.toFixed(1) + "|" + h.toFixed(1);
      const cached = spriteCache.get(key);
      if (cached) return cached;

      const sw = Math.ceil(w * dpr);
      const sh = Math.ceil(h * dpr);
      const sprite = document.createElement("canvas");
      sprite.width = sw;
      sprite.height = sh;

      const s = sprite.getContext("2d")!;
      s.scale(dpr, dpr);

      const fold = Math.min(10, w * 0.24);
      const ba = 0.75;

      s.globalAlpha = ba;
      s.strokeStyle = "rgba(255,255,255,0.5)";
      s.lineWidth = 1;
      s.strokeRect(0.5, 0.5, w - 1, h - 1);

      s.globalAlpha = ba * 0.33;
      s.fillStyle = "rgba(255,255,255,0.12)";
      s.fillRect(0.5, 0.5, w - 1, h - 1);

      s.globalAlpha = ba * 0.8;
      s.beginPath();
      s.moveTo(w - fold, 0);
      s.lineTo(w - fold, fold);
      s.lineTo(w, fold);
      s.closePath();
      s.stroke();

      s.globalAlpha = ba * 0.45;
      const lineEnds = [w - 11, w - 15, w - 19];
      s.beginPath();
      for (let row = 0; row < 3; row++) {
        const ly = 14 + row * 7;
        s.moveTo(7, ly);
        s.lineTo(lineEnds[row], ly);
      }
      s.stroke();

      s.globalAlpha = ba * 0.9;
      s.font = "600 8px -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif";
      s.textAlign = "left";
      s.textBaseline = "middle";
      s.fillStyle = "rgba(255,255,255,0.72)";
      s.fillText(ext, 7, h - 10);

      spriteCache.set(key, sprite);
      return sprite;
    }

    function resize(): void {
      width = window.innerWidth;
      height = window.innerHeight;
      canvas.width = Math.max(1, Math.floor(width * dpr));
      canvas.height = Math.max(1, Math.floor(height * dpr));
      canvas.style.width = width + "px";
      canvas.style.height = height + "px";
      drawCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
    }

    function spawn(): void {
      const scale = 0.74 + Math.random() * 0.56;
      const pw = 42 * scale;
      const ph = 56 * scale;
      const ext = FALLING_FILE_EXTENSIONS[Math.floor(Math.random() * FALLING_FILE_EXTENSIONS.length)];

      particles.push({
        sprite: buildSprite(ext, pw, ph),
        alpha: 0.2 + Math.random() * 0.45,
        drift: (Math.random() - 0.5) * 18,
        extension: ext,
        height: ph,
        life: 0,
        maxLife: 4.2 + Math.random() * 2.4,
        rotation: (Math.random() - 0.5) * 0.7,
        rotationSpeed: (Math.random() - 0.5) * 0.9,
        speed: prefersReducedMotion ? 40 + Math.random() * 30 : 62 + Math.random() * 76,
        width: pw,
        x: width * (0.12 + Math.random() * 0.76),
        y: -ph - Math.random() * 80,
      });
    }

    const maxParticles = prefersReducedMotion ? 14 : 20;
    const spawnRate = prefersReducedMotion ? 1.8 : 4.5;

    function animate(time: number): void {
      const dt = lastTime ? Math.min((time - lastTime) / 1000, 0.045) : 0.016;
      lastTime = time;

      drawCtx.clearRect(0, 0, width, height);
      spawnAccumulator += dt * spawnRate;
      while (spawnAccumulator >= 1 && particles.length < maxParticles) {
        spawn();
        spawnAccumulator -= 1;
      }

      for (let i = particles.length - 1; i >= 0; i--) {
        const p = particles[i];
        if (p.life >= p.maxLife || p.y > height + p.height) {
          particles.splice(i, 1);
          continue;
        }
        p.life += dt;
        p.y += (p.speed + p.drift * 0.04) * dt;
        p.x += Math.sin(p.life * 1.25) * p.drift * dt;
        p.rotation += p.rotationSpeed * dt;

        const appear = Math.min(1, p.life / 0.7);
        const fadeOut = Math.max(0, 1 - Math.max(0, p.y - height * 0.55) / (height * 0.42));
        const alpha = 0.3 * p.alpha * appear * fadeOut;
        if (alpha <= 0.008) continue;

        drawCtx.save();
        drawCtx.globalAlpha = alpha;
        drawCtx.translate(p.x, p.y);
        drawCtx.rotate(p.rotation);
        drawCtx.drawImage(p.sprite, -p.width / 2, -p.height / 2, p.width, p.height);
        drawCtx.restore();
      }

      raf = requestAnimationFrame(animate);
    }

    /* ── Bootstrap ── */
    resize();
    for (let i = 0; i < 10; i++) spawn();
    for (const p of particles) {
      p.y = Math.random() * height * 0.7;
      p.life = Math.random() * 2.0;
    }

    raf = requestAnimationFrame(animate);

    window.addEventListener("resize", resize);

    return () => {
      window.removeEventListener("resize", resize);
      cancelAnimationFrame(raf);
      particles = [];
      spriteCache.clear();
    };
  }, [active]);

  return <canvas ref={canvasRef} className="project-loading-canvas" aria-hidden="true" />;
}
