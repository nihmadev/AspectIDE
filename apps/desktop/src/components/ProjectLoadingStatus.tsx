import { useEffect, useRef } from "react";
import type { ProjectLoadSummary } from "../lib/projectLoadPresentation";
import { useTranslation } from "../lib/i18n/useTranslation";

type ProjectLoadingStatusProps = {
  onDismissError: () => void;
  summary: ProjectLoadSummary;
};

type FallingFileParticle = {
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
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !active) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const targetCanvas = canvas;
    const context = ctx;

    const prefersReducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const dpr = window.devicePixelRatio || 1;
    let width = 0;
    let height = 0;
    let particles: FallingFileParticle[] = [];
    let raf = 0;
    let lastTime = 0;
    let spawnAccumulator = 0;

    function resize() {
      width = window.innerWidth;
      height = window.innerHeight;
      targetCanvas.width = Math.max(1, Math.floor(width * dpr));
      targetCanvas.height = Math.max(1, Math.floor(height * dpr));
      targetCanvas.style.width = width + "px";
      targetCanvas.style.height = height + "px";
      context.setTransform(dpr, 0, 0, dpr, 0, 0);
    }

    function spawn() {
      const scale = 0.74 + Math.random() * 0.62;
      const fileWidth = 42 * scale;
      const fileHeight = 56 * scale;

      particles.push({
        alpha: 0.2 + Math.random() * 0.45,
        drift: (Math.random() - 0.5) * 18,
        extension: FALLING_FILE_EXTENSIONS[Math.floor(Math.random() * FALLING_FILE_EXTENSIONS.length)],
        height: fileHeight,
        life: 0,
        maxLife: 4.2 + Math.random() * 2.4,
        rotation: (Math.random() - 0.5) * 0.7,
        rotationSpeed: (Math.random() - 0.5) * 0.9,
        speed: 62 + Math.random() * 76,
        width: fileWidth,
        x: width * (0.18 + Math.random() * 0.64),
        y: -fileHeight - Math.random() * 80,
      });
    }

    function animate(time: number) {
      const dt = lastTime ? Math.min((time - lastTime) / 1000, 0.045) : 0.016;
      lastTime = time;

      context.clearRect(0, 0, width, height);
      spawnAccumulator += dt * (prefersReducedMotion ? 1.6 : 8.6);
      while (spawnAccumulator >= 1 && particles.length < 90) {
        spawn();
        spawnAccumulator -= 1;
      }

      for (let i = particles.length - 1; i >= 0; i--) {
        const p = particles[i];
        p.life += dt;
        p.y += p.speed * dt;
        p.x += Math.sin(p.life * 1.25) * p.drift * dt;
        p.rotation += p.rotationSpeed * dt;

        if (p.life >= p.maxLife || p.y > height + p.height) {
          particles.splice(i, 1);
          continue;
        }

        const appear = Math.min(1, p.life / 0.7);
        const disappearByLife = Math.max(0, 1 - Math.max(0, p.life - p.maxLife + 1.15) / 1.15);
        const disappearByDepth = Math.max(0, 1 - Math.max(0, p.y - height * 0.62) / (height * 0.34));
        const alpha = p.alpha * appear * Math.min(disappearByLife, disappearByDepth);

        drawFallingFile(context, p, alpha);
      }

      raf = requestAnimationFrame(animate);
    }

    resize();
    for (let i = 0; i < 24; i++) {
      spawn();
      particles[i].y = Math.random() * height * 0.78;
      particles[i].life = Math.random() * 2.2;
    }

    window.addEventListener("resize", resize);
    raf = requestAnimationFrame(animate);

    return () => {
      window.removeEventListener("resize", resize);
      cancelAnimationFrame(raf);
      particles = [];
    };
  }, [active]);

  return <canvas ref={canvasRef} className="project-loading-canvas" aria-hidden="true" />;
}

function drawFallingFile(ctx: CanvasRenderingContext2D, particle: FallingFileParticle, alpha: number) {
  if (alpha <= 0.01) return;

  const fold = Math.min(10, particle.width * 0.24);
  const radius = Math.min(6, particle.width * 0.12);
  const x = -particle.width / 2;
  const y = -particle.height / 2;

  ctx.save();
  ctx.translate(particle.x, particle.y);
  ctx.rotate(particle.rotation);
  ctx.globalAlpha = alpha;

  ctx.shadowBlur = 22;
  ctx.shadowColor = "rgba(255, 255, 255, 0.16)";
  ctx.fillStyle = "rgba(255, 255, 255, 0.08)";
  ctx.strokeStyle = "rgba(255, 255, 255, 0.42)";
  ctx.lineWidth = 1;

  ctx.beginPath();
  ctx.moveTo(x + radius, y);
  ctx.lineTo(x + particle.width - fold, y);
  ctx.lineTo(x + particle.width, y + fold);
  ctx.lineTo(x + particle.width, y + particle.height - radius);
  ctx.quadraticCurveTo(x + particle.width, y + particle.height, x + particle.width - radius, y + particle.height);
  ctx.lineTo(x + radius, y + particle.height);
  ctx.quadraticCurveTo(x, y + particle.height, x, y + particle.height - radius);
  ctx.lineTo(x, y + radius);
  ctx.quadraticCurveTo(x, y, x + radius, y);
  ctx.closePath();
  ctx.fill();
  ctx.stroke();

  ctx.shadowBlur = 0;
  ctx.beginPath();
  ctx.moveTo(x + particle.width - fold, y);
  ctx.lineTo(x + particle.width - fold, y + fold);
  ctx.lineTo(x + particle.width, y + fold);
  ctx.strokeStyle = "rgba(255, 255, 255, 0.34)";
  ctx.stroke();

  ctx.fillStyle = "rgba(255, 255, 255, 0.72)";
  ctx.font = "600 8px -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif";
  ctx.textAlign = "left";
  ctx.textBaseline = "middle";
  ctx.fillText(particle.extension, x + 7, y + particle.height - 10);

  ctx.strokeStyle = "rgba(255, 255, 255, 0.22)";
  ctx.lineWidth = 1;
  for (let row = 0; row < 3; row++) {
    const lineY = y + 14 + row * 7;
    const lineEnd = x + particle.width - 11 - row * 4;
    ctx.beginPath();
    ctx.moveTo(x + 7, lineY);
    ctx.lineTo(lineEnd, lineY);
    ctx.stroke();
  }

  ctx.restore();
}
