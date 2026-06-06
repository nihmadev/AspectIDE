import {
  browserActArgs,
  browserSessionName,
  buildBrowserInvokeRequest,
  ensureBrowserStream,
  screenshotPathFromBrowserData,
  tokenizeBrowserCommand,
} from "./agentBrowser";
import { formatBrowserCommandPreview, isReadOnlyBrowserArgs } from "./agentBrowserCommandCatalog";
import type { AiChatSendInput, AiToolApprovalRequest } from "./aiChatTypes";
import {
  createBrowserActApproval,
  createBrowserChatApproval,
  createBrowserCloseApproval,
  createBrowserInstallApproval,
  createBrowserOpenApproval,
} from "./aiRuntimeApprovals";
import { booleanArg, clamp, numberArg, stringArg, stringArrayArg, toolJson, type ToolResult, type UnknownRecord } from "./aiRuntimeShared";
import { luxCommands } from "./tauri";

type BrowserApprovalUi = {
  requireApproval: (approval: AiToolApprovalRequest) => Promise<void>;
};

function ensureBrowserEnabled(input: AiChatSendInput) {
  if (!input.preferences.agentBrowserEnabled) {
    throw new Error(
      "Browser automation is disabled in Lux AI settings. Enable agent-browser to use Browser* tools.",
    );
  }
}

function sessionFor(input: AiChatSendInput) {
  return browserSessionName(input.chatSessionId);
}

function commandPath(input: AiChatSendInput) {
  return input.preferences.agentBrowserCommand.trim() || undefined;
}

function normalizeBrowserUrl(url: string) {
  const trimmed = url.trim();
  if (!trimmed) return "";
  if (/^[a-z][a-z0-9+.-]*:/i.test(trimmed)) return trimmed;
  return `https://${trimmed}`;
}

async function invokeBrowser(
  input: AiChatSendInput,
  args: string[],
  overrides?: { headed?: boolean; timeoutSecs?: number },
) {
  const response = await luxCommands.agentBrowserInvoke(
    buildBrowserInvokeRequest(input.preferences, sessionFor(input), args, overrides),
  );
  return {
    session: response.session,
    command: response.command,
    success: response.success,
    data: response.data,
    text: response.text,
    elapsedMs: response.elapsedMs,
    truncated: response.truncated,
    exitCode: response.exitCode,
  };
}

async function visionForScreenshot(
  input: AiChatSendInput,
  data: unknown,
  text: string,
): Promise<string[] | undefined> {
  if (!input.preferences.includeImages) return undefined;
  const path = screenshotPathFromBrowserData(data, text);
  if (!path) return undefined;
  try {
    const image = await luxCommands.agentBrowserReadImage(path);
    return [image.dataUrl];
  } catch {
    return undefined;
  }
}

async function streamPortAfterAction(input: AiChatSendInput) {
  if (!input.preferences.agentBrowserAutoStreamPreview) return null;
  try {
    const status = await ensureBrowserStream(sessionFor(input), commandPath(input));
    return status.port;
  } catch {
    return null;
  }
}

const navigationCommandHeads = new Set(["open", "goto", "navigate"]);

function shouldEnableStreamAfterArgs(args: string[]) {
  const head = args[0]?.trim().toLowerCase();
  return head ? navigationCommandHeads.has(head) : false;
}

export async function browserStatusTool(input: AiChatSendInput): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const status = await luxCommands.agentBrowserStatus({
    commandPath: commandPath(input) ?? null,
    skipAutoUpdate: Boolean(commandPath(input)),
  });
  return toolJson("BrowserStatus", {
    available: status.available,
    commandPath: status.commandPath,
    version: status.version,
    latestVersion: status.latestVersion,
    updatePerformed: status.updatePerformed,
    updateDetail: status.updateDetail,
    detail: status.detail,
    sessions: status.sessions,
    chatSession: sessionFor(input),
    doctor: status.doctor,
  });
}

export async function browserOpenTool(
  args: UnknownRecord,
  input: AiChatSendInput,
  ui: BrowserApprovalUi,
): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const url = normalizeBrowserUrl(stringArg(args, "url", ""));
  const headed = booleanArg(args, "headed", input.preferences.agentBrowserHeaded);
  const session = sessionFor(input);
  const approval = createBrowserOpenApproval(input.locale, url, session, headed);
  await ui.requireApproval(approval);
  const commandArgs = url ? ["open", url] : ["open"];
  const result = await invokeBrowser(input, commandArgs, { headed, timeoutSecs: 120 });
  if (!result.success) throw new Error(result.text || "BrowserOpen failed.");
  const streamPort = input.preferences.agentBrowserAutoStreamPreview
    ? await streamPortAfterAction(input)
    : null;
  return toolJson("BrowserOpen", {
    ...result,
    url: url || null,
    headed,
    streamPort,
  }, { browserStreamPort: streamPort });
}

export async function browserSnapshotTool(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const interactive = booleanArg(args, "interactive", true);
  const compact = booleanArg(args, "compact", true);
  const depth = clamp(numberArg(args, "depth", 8), 1, 24);
  const selector = stringArg(args, "selector", "").trim();
  const includeUrls = booleanArg(args, "includeUrls", false);
  const snapshotArgs = ["snapshot"];
  if (interactive) snapshotArgs.push("-i");
  if (compact) snapshotArgs.push("-c");
  snapshotArgs.push("-d", String(depth));
  if (selector) snapshotArgs.push("-s", selector);
  if (includeUrls) snapshotArgs.push("-u");
  const result = await invokeBrowser(input, snapshotArgs, { timeoutSecs: 90 });
  if (!result.success) throw new Error(result.text || "BrowserSnapshot failed.");
  return toolJson("BrowserSnapshot", result);
}

export async function browserActTool(
  args: UnknownRecord,
  input: AiChatSendInput,
  ui: BrowserApprovalUi,
): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const command = stringArg(args, "command", "").trim();
  const batchCommands = Array.isArray(args.batchCommands)
    ? args.batchCommands.filter((entry): entry is string => typeof entry === "string" && entry.trim().length > 0)
    : [];
  const preview = batchCommands.length > 0 ? batchCommands.join("\n") : command;
  if (!preview) throw new Error("BrowserAct requires command or batchCommands.");
  const approval = createBrowserActApproval(input.locale, preview, sessionFor(input));
  await ui.requireApproval(approval);
  const actArgs = browserActArgs(command, batchCommands.length > 0 ? batchCommands : undefined);
  const result = await invokeBrowser(input, actArgs, {
    timeoutSecs: 120,
  });
  if (!result.success) throw new Error(result.text || "BrowserAct failed.");
  const opensBrowser = actArgs[0] === "batch"
    ? batchCommands.some((entry) => shouldEnableStreamAfterArgs(tokenizeBrowserCommand(entry)))
    : shouldEnableStreamAfterArgs(actArgs);
  const streamPort = opensBrowser ? await streamPortAfterAction(input) : null;
  return toolJson("BrowserAct", { ...result, command: preview, streamPort }, { browserStreamPort: streamPort });
}

export async function browserScreenshotTool(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const path = stringArg(args, "path", "").trim();
  const annotate = booleanArg(args, "annotate", false);
  const fullPage = booleanArg(args, "fullPage", false);
  const attachVision = booleanArg(args, "attachVision", true);
  const screenshotArgs = ["screenshot"];
  if (path) screenshotArgs.push(path);
  if (annotate) screenshotArgs.push("--annotate");
  if (fullPage) screenshotArgs.push("--full");
  const result = await invokeBrowser(input, screenshotArgs, { timeoutSecs: 90 });
  if (!result.success) throw new Error(result.text || "BrowserScreenshot failed.");
  const visionImageUrls = attachVision
    ? await visionForScreenshot(input, result.data, result.text)
    : undefined;
  return toolJson("BrowserScreenshot", {
    ...result,
    annotate,
    fullPage,
    path: path || screenshotPathFromBrowserData(result.data, result.text),
    visionAttached: Boolean(visionImageUrls?.length),
  }, { visionImageUrls });
}

export async function browserCloseTool(
  args: UnknownRecord,
  input: AiChatSendInput,
  ui: BrowserApprovalUi,
): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const all = booleanArg(args, "all", false);
  const session = sessionFor(input);
  if (all) {
    const approval = createBrowserCloseApproval(input.locale, true, session);
    await ui.requireApproval(approval);
  }
  const closeArgs = all ? ["close", "--all"] : ["close"];
  const result = await invokeBrowser(input, closeArgs, { timeoutSecs: 45 });
  if (!result.success) throw new Error(result.text || "BrowserClose failed.");
  return toolJson("BrowserClose", { ...result, all });
}

export async function browserChatTool(
  args: UnknownRecord,
  input: AiChatSendInput,
  ui: BrowserApprovalUi,
): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const instruction = stringArg(args, "instruction", "").trim();
  if (!instruction) throw new Error("BrowserChat requires instruction.");
  const quiet = booleanArg(args, "quiet", true);
  const approval = createBrowserChatApproval(input.locale, instruction, sessionFor(input));
  await ui.requireApproval(approval);
  const chatArgs = quiet ? ["-q", "chat", instruction] : ["chat", instruction];
  const result = await invokeBrowser(input, chatArgs, { timeoutSecs: 180 });
  if (!result.success) throw new Error(result.text || "BrowserChat failed.");
  return toolJson("BrowserChat", { ...result, instruction, quiet });
}

export async function browserDashboardTool(
  args: UnknownRecord,
  input: AiChatSendInput,
): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const action = stringArg(args, "action", "status").trim().toLowerCase() || "status";
  const port = clamp(numberArg(args, "port", input.preferences.agentBrowserDashboardPort), 1024, 65_535);
  const openInBrowser = booleanArg(args, "openInBrowser", action === "start");
  const response = await luxCommands.agentBrowserDashboard({
    action,
    port,
    commandPath: commandPath(input) ?? null,
  });
  if (!response.success) throw new Error(response.detail || `BrowserDashboard ${action} failed.`);
  if (openInBrowser && response.url) {
    await luxCommands.fileOpenExternal(response.url);
  }
  return toolJson("BrowserDashboard", response);
}

export async function browserInstallTool(
  args: UnknownRecord,
  input: AiChatSendInput,
  ui: BrowserApprovalUi,
): Promise<ToolResult> {
  const withDeps = booleanArg(args, "withDeps", false);
  const approval = createBrowserInstallApproval(input.locale, withDeps);
  await ui.requireApproval(approval);
  const response = await luxCommands.agentBrowserInstall({
    commandPath: commandPath(input) ?? null,
    withDeps,
  });
  if (!response.success) throw new Error(response.detail);
  return toolJson("BrowserInstall", response);
}

export async function browserInvokeTool(
  args: UnknownRecord,
  input: AiChatSendInput,
  ui: BrowserApprovalUi,
): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const cmdArgs = stringArrayArg(args, "args");
  if (cmdArgs.length === 0) throw new Error("BrowserInvoke requires args array.");
  if (!isReadOnlyBrowserArgs(cmdArgs)) {
    const approval = createBrowserActApproval(input.locale, formatBrowserCommandPreview(cmdArgs), sessionFor(input));
    await ui.requireApproval(approval);
  }
  const result = await invokeBrowser(input, cmdArgs, { timeoutSecs: 120 });
  if (!result.success) throw new Error(result.text || "BrowserInvoke failed.");
  const visionImageUrls = cmdArgs[0] === "screenshot"
    ? await visionForScreenshot(input, result.data, result.text)
    : undefined;
  const streamPort = shouldEnableStreamAfterArgs(cmdArgs) ? await streamPortAfterAction(input) : null;
  return toolJson("BrowserInvoke", { ...result, args: cmdArgs, streamPort }, { visionImageUrls, browserStreamPort: streamPort });
}

export async function browserDoctorTool(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const fix = booleanArg(args, "fix", false);
  const offline = booleanArg(args, "offline", true);
  const quick = booleanArg(args, "quick", true);
  const doctorArgs = ["doctor", "--json"];
  if (fix) doctorArgs.push("--fix");
  if (offline) doctorArgs.push("--offline");
  if (quick) doctorArgs.push("--quick");
  const result = await invokeBrowser(input, doctorArgs, { timeoutSecs: fix ? 300 : 90 });
  if (!result.success) throw new Error(result.text || "BrowserDoctor failed.");
  return toolJson("BrowserDoctor", { ...result, fix, offline, quick });
}

export async function browserHelpTool(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  ensureBrowserEnabled(input);
  const topic = stringArg(args, "topic", "").trim();
  if (topic === "skills" || topic === "skill") {
    const skills = await luxCommands.agentBrowserSkills({
      name: stringArg(args, "skill", "core") || "core",
      all: booleanArg(args, "allSkills", false),
      commandPath: commandPath(input) ?? null,
    });
    return toolJson("BrowserHelp", skills);
  }
  const helpArgs = topic ? [topic, "--help"] : ["--help"];
  const result = await invokeBrowser(input, helpArgs, { timeoutSecs: 45 });
  if (!result.success) throw new Error(result.text || "BrowserHelp failed.");
  return toolJson("BrowserHelp", result);
}