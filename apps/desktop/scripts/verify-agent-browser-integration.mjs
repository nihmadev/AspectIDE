import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const files = [
  "src-tauri/src/agent_browser.rs",
  "src-tauri/src/lib.rs",
  "src/lib/aiRuntimeBrowser.ts",
  "src/lib/agentBrowserStream.ts",
  "src/lib/aiChatTurnRuntime.ts",
  "src/lib/agentBrowserCommandReference.ts",
  "src/lib/aiChatRuntime.ts",
  "src/components/ai-chat/AgentBrowserPreview.tsx",
  "src/components/AiChatPanel.tsx",
];

const requiredSnippets = [
  "agent_browser_install",
  "agent_browser_stream_status",
  "agent_browser_dashboard",
  "agent_browser_read_image",
  "allow_file_access",
  "BrowserChat",
  "BrowserInvoke",
  "BrowserDashboard",
  "visionImageUrls",
  "bumpBrowserStreamRefresh",
  "browserStreamRefreshToken",
  "AgentBrowserPreview",
  "AgentBrowserStreamClient",
  "agent-browser-preview-dashboard",
  "BrowserDoctor",
  "AGENT_BROWSER_COMMAND_REFERENCE",
  "provider",
];

const errors = [];
const combined = await Promise.all(
  files.map((file) => readFile(resolve(desktopRoot, file), "utf8")),
).then((parts) => parts.join("\n"));

for (const snippet of requiredSnippets) {
  if (!combined.includes(snippet)) errors.push(`Missing integration snippet: ${snippet}`);
}

const cliEntry = resolve(desktopRoot, "node_modules/agent-browser/bin/agent-browser.js");
const bundledCli = resolve(desktopRoot, "node_modules/.bin/agent-browser.cmd");
const bundledCliPosix = resolve(desktopRoot, "node_modules/.bin/agent-browser");
const hasCli = existsSync(cliEntry) || existsSync(bundledCli) || existsSync(bundledCliPosix);

function runAgentBrowser(args) {
  if (existsSync(cliEntry)) {
    return spawnSync(process.execPath, [cliEntry, ...args], { encoding: "utf8", timeout: 60_000 });
  }
  const shim = existsSync(bundledCli) ? bundledCli : bundledCliPosix;
  return spawnSync(shim, args, {
    encoding: "utf8",
    timeout: 60_000,
    shell: process.platform === "win32",
    windowsVerbatimArguments: true,
  });
}

if (hasCli) {
  const version = runAgentBrowser(["--version"]);
  if (version.status !== 0) {
    errors.push(`Bundled agent-browser --version failed: ${version.stderr || version.stdout || version.error?.message || "unknown"}`);
  } else {
    const doctor = runAgentBrowser(["doctor", "--json", "--offline", "--quick"]);
    if (doctor.status !== 0) {
      errors.push(`Bundled agent-browser doctor failed: ${doctor.stderr || doctor.stdout}`);
    } else {
      try {
        const payload = JSON.parse(doctor.stdout.trim());
        if (payload.success !== true) {
          errors.push("agent-browser doctor reported success=false");
        }
      } catch {
        errors.push("agent-browser doctor returned non-JSON output");
      }
    }
  }
} else {
  errors.push("Bundled agent-browser CLI not found. Run pnpm install in apps/desktop.");
}

if (!process.env.SKIP_AGENT_BROWSER_E2E && hasCli) {
  const session = "Aspect-verify-e2e";
  const open = runAgentBrowser(["--session", session, "open", "https://example.com"]);
  if (open.status !== 0) {
    errors.push(`E2E open failed: ${open.stderr || open.stdout}`);
  } else {
    const snapshot = runAgentBrowser(["--session", session, "snapshot", "-i", "-c", "-d", "4"]);
    if (snapshot.status !== 0) {
      errors.push(`E2E snapshot failed: ${snapshot.stderr || snapshot.stdout}`);
    }
    const close = runAgentBrowser(["--session", session, "close"]);
    if (close.status !== 0) {
      errors.push(`E2E close failed: ${close.stderr || close.stdout}`);
    }
  }
}

if (errors.length > 0) {
  throw new Error(`agent-browser integration verification failed:\n- ${errors.join("\n- ")}`);
}

console.log("agent-browser integration verification passed (static + CLI smoke + E2E workflow).");