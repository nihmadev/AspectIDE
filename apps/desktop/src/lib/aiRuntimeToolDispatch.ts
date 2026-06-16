import type { AiChatSendInput } from "./aiChatTypes";
import type { OpenAiToolCall } from "./aiChatTransport";
import { deleteFileTool, patchEngineTool, strReplaceTool, toolResultFromFileOperation, writeFileTool } from "./aiRuntimeFileTools";
import { checkpointTool } from "./aiRuntimeCheckpoints";
import { docsContext, memoryContext, rulesContext } from "./aiRuntimeContextSources";
import {
  activeContext,
  contextBudgeter,
  fastContext,
  repoMap,
  workspaceIndex,
} from "./aiRuntimeContextTools";
import {
  diagnosticsContext,
  failureAnalyzer,
  gitContext,
  readLints,
  reviewDiff,
  testHealth,
} from "./aiRuntimeDiagnostics";
import {
  globFiles,
  grepTool,
  impactAnalysis,
  inspectFileTool,
  readFileTool,
  relatedFiles,
  symbolContext,
  webFetchTool,
} from "./aiRuntimeExploreTools";
import {
  browserActTool,
  browserChatTool,
  browserCloseTool,
  browserDashboardTool,
  browserDoctorTool,
  browserHelpTool,
  browserInstallTool,
  browserInvokeTool,
  browserOpenTool,
  browserScreenshotTool,
  browserSnapshotTool,
  browserStatusTool,
} from "./aiRuntimeBrowser";
import { semanticSearch } from "./aiRuntimeSemanticSearch";
import { secretGuard as runSecretGuard } from "./aiRuntimeSecretGuard";
import { shellTool, terminalContextTool, terminalWriteTool } from "./aiRuntimeShellTools";
import { requireToolApproval, type ToolExecutionUi } from "./aiRuntimeToolApproval";
import { parseToolArguments } from "./aiRuntimeToolBridge";
import { agentMessageTool, askUserTool, goalWrite, presentPlanTool, taskSubagentTool, todoWrite, type RuntimeToolSession } from "./aiRuntimeToolSession";
import { clamp, numberArg, stringArg, type ToolResult } from "./aiRuntimeShared";
import { isRuntimeToolAllowed, readOnlyAgentModeToolDenyReason, type RuntimeToolName } from "./aiRuntimeTools";
import { luxCommands } from "./tauri";

function browserApprovalUi(input: AiChatSendInput, ui: ToolExecutionUi) {
  return {
    requireApproval: (approval: Parameters<typeof requireToolApproval>[2]) => requireToolApproval(input, ui, approval),
  };
}

export async function runRuntimeTool(
  call: OpenAiToolCall,
  input: AiChatSendInput,
  session: RuntimeToolSession,
  ui: ToolExecutionUi,
): Promise<ToolResult> {
  const name = call.function?.name as RuntimeToolName | undefined;
  if (!name) throw new Error("Tool call is missing a function name.");
  const readOnlyDeny = readOnlyAgentModeToolDenyReason(name, input.preferences.agentMode);
  if (readOnlyDeny) throw new Error(readOnlyDeny);
  if (!isRuntimeToolAllowed(name, input.preferences)) {
    throw new Error(`Tool "${name}" is not available in the current agent mode or settings.`);
  }
  const args = parseToolArguments(call.function?.arguments);
  switch (name) {
    case "FastContext":
      return fastContext(input, stringArg(args, "query", input.message));
    case "RepoMap":
      return repoMap(numberArg(args, "maxFiles", 80));
    case "WorkspaceIndex":
      return workspaceIndex(args, input);
    case "ActiveContext":
      return activeContext(args, input);
    case "RulesContext":
      return rulesContext(args, input);
    case "DocsContext":
      return docsContext(args, input);
    case "MemoryContext":
      return memoryContext(args, input);
    case "ContextBudgeter":
      return contextBudgeter(args, input);
    case "SemanticSearch":
      return semanticSearch(args, input);
    case "Glob":
      return globFiles(stringArg(args, "pattern"), numberArg(args, "maxResults", 80));
    case "Read":
      return readFileTool(stringArg(args, "path"), numberArg(args, "maxBytes", 120_000));
    case "InspectFile":
      return inspectFileTool(args);
    case "Write":
      return writeFileTool(args, input, ui);
    case "StrReplace":
      return strReplaceTool(args, input, ui);
    case "PatchEngine":
      return patchEngineTool(args, input, ui);
    case "Checkpoint":
      return checkpointTool(args, input, {
        requireApproval: (approval) => requireToolApproval(input, ui, approval),
        applyPatch: async (operations, saveToDisk, dryRun) => {
          const result = await luxCommands.aiFilePatch(operations, saveToDisk, dryRun);
          return toolResultFromFileOperation("Checkpoint", result);
        },
      });
    case "Delete":
      return deleteFileTool(stringArg(args, "path"), input, ui);
    case "Shell":
      return shellTool(args, input, ui);
    case "TerminalContext":
      return terminalContextTool(args, input);
    case "TerminalWrite":
      return terminalWriteTool(args, input, ui);
    case "Grep":
      return grepTool(args);
    case "ReadLints":
      return readLints(args, input);
    case "Goal":
      return goalWrite(args, input);
    case "TodoWrite":
      return todoWrite(args, session, input);
    case "Task":
      return taskSubagentTool(args, input, session);
    case "AgentMessage":
      return agentMessageTool(args, input, session);
    case "AskUser":
      return askUserTool(args, input, call.id ?? `ask-${Date.now()}`);
    case "PresentPlan":
      return presentPlanTool(args, input, call.id ?? `plan-${Date.now()}`);
    case "WebFetch":
      return webFetchTool(args);
    case "SymbolContext":
      return symbolContext(args, input);
    case "RelatedFiles":
      return relatedFiles(args, input);
    case "DiagnosticsContext":
      return diagnosticsContext(numberArg(args, "maxResults", 80));
    case "GitContext":
      return gitContext();
    case "TestHealth":
      return testHealth();
    case "FailureAnalyzer":
      return failureAnalyzer(args);
    case "ImpactAnalysis":
      return impactAnalysis(args, input);
    case "ReviewDiff":
      return reviewDiff(args);
    case "SecretGuard":
      return runSecretGuard(args);
    case "BrowserStatus":
      return browserStatusTool(input);
    case "BrowserOpen":
      return browserOpenTool(args, input, browserApprovalUi(input, ui));
    case "BrowserSnapshot":
      return browserSnapshotTool(args, input);
    case "BrowserAct":
      return browserActTool(args, input, browserApprovalUi(input, ui));
    case "BrowserScreenshot":
      return browserScreenshotTool(args, input);
    case "BrowserClose":
      return browserCloseTool(args, input, browserApprovalUi(input, ui));
    case "BrowserChat":
      return browserChatTool(args, input, browserApprovalUi(input, ui));
    case "BrowserDashboard":
      return browserDashboardTool(args, input);
    case "BrowserInstall":
      return browserInstallTool(args, input, browserApprovalUi(input, ui));
    case "BrowserHelp":
      return browserHelpTool(args, input);
    case "BrowserDoctor":
      return browserDoctorTool(args, input);
    case "BrowserInvoke":
      return browserInvokeTool(args, input, browserApprovalUi(input, ui));
    default:
      throw new Error(`Unknown tool: ${name}`);
  }
}