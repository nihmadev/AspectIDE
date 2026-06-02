import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const transportPath = resolve("src/lib/aiChatTransport.ts");
const tauriRuntimePath = resolve("src/lib/tauri.ts");
const browserPreviewScriptPath = resolve("scripts/start-browser-preview.mjs");
const packagePath = resolve("package.json");
const [source, tauriRuntimeSource, browserPreviewScriptSource, packageSource] = await Promise.all([
  readFile(transportPath, "utf8"),
  readFile(tauriRuntimePath, "utf8"),
  readFile(browserPreviewScriptPath, "utf8"),
  readFile(packagePath, "utf8"),
]);

const errors = [];

if (source.includes("isStreamFallbackAllowed")) {
  errors.push("AI chat transport must not silently allow streaming-to-non-streaming fallback.");
}

const browserPreviewGate = tauriRuntimeSource.match(/export const isBrowserPreviewRuntime\s*=\s*\(\)\s*=>\s*([^;]+);/);
if (!browserPreviewGate) {
  errors.push("isBrowserPreviewRuntime gate was not found.");
} else {
  const gateExpression = browserPreviewGate[1];
  if (gateExpression.includes("import.meta.env.DEV")) {
    errors.push("Browser preview runtime must not be enabled implicitly by Vite dev mode.");
  }
  if (!gateExpression.includes('import.meta.env.VITE_LUX_BROWSER_PREVIEW === "1"')) {
    errors.push("Browser preview runtime must require VITE_LUX_BROWSER_PREVIEW=1 explicitly.");
  }
}

if (!browserPreviewScriptSource.includes('VITE_LUX_BROWSER_PREVIEW: "1"')) {
  errors.push("Browser preview launcher must set VITE_LUX_BROWSER_PREVIEW=1 explicitly.");
}

const packageJson = JSON.parse(packageSource);
if (!String(packageJson.scripts?.dev ?? "").includes("start-browser-preview.mjs")) {
  errors.push("Desktop web dev script must route through the explicit browser preview launcher.");
}

const requestChatCompletion = source.match(/export async function requestChatCompletion[\s\S]*?\n}\n\nexport function firstChoice/);
if (!requestChatCompletion) {
  errors.push("requestChatCompletion function block was not found.");
} else {
  const body = requestChatCompletion[0];
  const desktopBranch = body.match(/if \(desktopRuntime\) \{([\s\S]*?)\n  }\n\n  const startedAtMs/);
  if (!desktopBranch) {
    errors.push("requestChatCompletion desktop runtime branch was not found.");
  } else {
    const desktopSource = desktopBranch[1];
    if (!desktopSource.includes("requestStreamingChatCompletion")) {
      errors.push("Desktop AI chat transport must use the streaming command path.");
    }
    if (desktopSource.includes("aiChatCompletion")) {
      errors.push("Desktop AI chat transport must not call the non-streaming completion command as a fallback.");
    }
    if (/catch\s*\(/.test(desktopSource)) {
      errors.push("Desktop AI chat transport must fail explicitly instead of catching stream setup errors for fallback.");
    }
  }

  const browserBranch = body.slice(body.indexOf("const startedAtMs"));
  if (!browserBranch.includes("requestBrowserChatCompletion")) {
    errors.push("Browser preview AI chat transport must remain explicit and isolated from desktop runtime.");
  }
}

if (errors.length > 0) {
  throw new Error(`AI transport verification failed:\n- ${errors.join("\n- ")}`);
}

console.log("AI transport verification passed (desktop streaming fallback is disabled).");
