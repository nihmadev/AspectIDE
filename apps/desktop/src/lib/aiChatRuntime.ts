import type { AiModelConfig, AiPreferences, AiProviderConfig } from "./aiPreferences";
import { buildLuxIdeSystemPrompt } from "./aiSystemPrompt";
import { isTauriRuntime, luxCommands, subscribeAiChatStream } from "./tauri";
import type { DocumentSnapshot, FsEntry, LspDocumentSymbol, LspLocation, LspWorkspaceSymbol, TerminalSessionInfo, WorkspaceDiagnostic, WorkspaceInfo } from "./types";

export type AiChatRole = "system" | "user" | "assistant" | "tool";

export type AiChatToolStatus = "approval" | "running" | "success" | "skipped" | "error";

export type AiToolApprovalDecision = "approved" | "rejected";

export type AiToolApprovalRequest = {
  id: string;
  tool: "Write" | "StrReplace" | "Delete" | "Shell" | "PatchEngine" | "Checkpoint";
  title: string;
  path: string;
  summary: string;
  preview: string;
  risk: "create" | "modify" | "delete" | "execute";
  approveLabel: string;
  rejectLabel: string;
};

export type AiToolApprovalState = AiToolApprovalRequest & {
  decision?: AiToolApprovalDecision;
};

export type AiChatToolCall = {
  id: string;
  tool: string;
  status: AiChatToolStatus;
  input?: string;
  output?: string;
  error?: string;
  approval?: AiToolApprovalState;
  startTime: number;
  endTime?: number;
  stats?: {
    linesAdded?: number;
    linesRemoved?: number;
    filesChanged?: number;
    filesCreated?: number;
    filesDeleted?: number;
  };
};

export type AiChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  toolCalls?: AiChatToolCall[];
  timestamp: number;
};

export type AiChatAttachmentInput = {
  name: string;
  size: number;
  text: string;
};

export type AiChatSendInput = {
  abortSignal: AbortSignal;
  activeDocument: DocumentSnapshot | null;
  attachments: AiChatAttachmentInput[];
  history: AiChatMessage[];
  message: string;
  openDocuments: DocumentSnapshot[];
  preferences: AiPreferences;
  provider: AiProviderConfig;
  selectedAgentInstructions: string;
  selectedAgentName: string;
  selectedModel: AiModelConfig;
  terminal: TerminalSessionInfo | null;
  workspace: WorkspaceInfo | null;
  onAssistantMessage: (message: AiChatMessage) => void;
  onAssistantMessageUpdate: (messageId: string, patch: Partial<AiChatMessage>) => void;
  onToolApproval: (request: AiToolApprovalRequest) => Promise<AiToolApprovalDecision>;
};

type ChatCompletionMessage = {
  role: AiChatRole;
  content: string | null;
  name?: string;
  tool_call_id?: string;
  tool_calls?: OpenAiToolCall[];
};

type OpenAiToolCall = {
  index?: number;
  id?: string;
  type?: "function";
  function?: {
    name?: string;
    arguments?: string;
  };
};

type ToolResult = {
  title: string;
  content: string;
  stats?: AiChatToolCall["stats"];
};

type FileToolResult = Awaited<ReturnType<typeof luxCommands.aiFileWrite>>;

type RuntimeToolDefinition = {
  type: "function";
  function: {
    name: RuntimeToolName;
    description: string;
    parameters: Record<string, unknown>;
  };
};

type RuntimeToolName = "FastContext" | "RepoMap" | "SemanticSearch" | "Glob" | "Read" | "Write" | "StrReplace" | "PatchEngine" | "Checkpoint" | "Delete" | "Shell" | "Grep" | "ReadLints" | "TodoWrite" | "WebFetch" | "SymbolContext" | "RelatedFiles" | "DiagnosticsContext" | "GitContext" | "TestHealth" | "FailureAnalyzer" | "WorkspaceIndex" | "ActiveContext" | "RulesContext" | "DocsContext" | "MemoryContext" | "ContextBudgeter" | "ImpactAnalysis" | "ReviewDiff" | "SecretGuard";

type RuntimePatchOperation = {
  action: string;
  path: string;
  text?: string;
  oldText?: string;
  newText?: string;
  expectedReplacements?: number;
  overwrite?: boolean;
};

type UnknownRecord = Record<string, unknown>;

type RelatedFileRelation = "same-directory" | "test" | "style" | "type-definition" | "route" | "schema" | "config" | "entrypoint" | "story" | "barrel" | "nearby-name" | "query-match";

type RelatedFileDescriptor = {
  entry?: FsEntry;
  path: string;
  relativePath: string;
  lower: string;
  relativeLower: string;
  dir: string;
  relativeDir: string;
  basename: string;
  basenameLower: string;
  extension: string;
  stem: string;
  stemLower: string;
  familyStem: string;
  familyStemLower: string;
};

type RelatedFileMatch = {
  descriptor: RelatedFileDescriptor;
  score: number;
  relations: Set<RelatedFileRelation>;
  queryHits: string[];
};

type FailureFinding = {
  source: string;
  kind: string;
  message: string;
  path?: string;
  line?: number;
  column?: number;
  evidence: string;
};

type SessionTodoStatus = "pending" | "in_progress" | "completed" | "blocked" | "cancelled";

type SessionTodoPriority = "low" | "medium" | "high";

type SessionTodo = {
  id: string;
  content: string;
  status: SessionTodoStatus;
  priority: SessionTodoPriority;
  notes?: string;
};

type CheckpointFileSnapshot = {
  path: string;
  relativePath: string;
  existed: boolean;
  text: string;
  size: number;
  truncated: boolean;
  source: "editor" | "disk" | "missing";
  error?: string;
};

type RuntimeCheckpoint = {
  id: string;
  label: string;
  workspaceRoot: string;
  createdAt: string;
  files: CheckpointFileSnapshot[];
  maxBytesPerFile: number;
};

type CheckpointAction = "create" | "list" | "diff" | "delete" | "restore";

type CheckpointCurrentFile = {
  existed: boolean;
  diskExists: boolean;
  text: string;
  size: number | null;
  truncated: boolean;
  source: "editor" | "disk" | "missing";
  error?: string;
};

type CheckpointFileDiff = {
  path: string;
  relativePath: string;
  status: "unchanged" | "modified" | "missing" | "created" | "truncated" | "error";
  existedAtCheckpoint: boolean;
  currentExists: boolean;
  diskExists: boolean;
  snapshotSource: CheckpointFileSnapshot["source"];
  currentSource: CheckpointCurrentFile["source"];
  snapshotSize: number;
  currentSize: number | null;
  snapshotTruncated: boolean;
  currentTruncated: boolean;
  lineDelta: number | null;
  beforePreview?: string;
  currentPreview?: string;
  error?: string;
};

type RuntimeToolSession = {
  todos: SessionTodo[];
};

type SemanticSearchResult = {
  type: "symbol" | "text" | "file";
  score: number;
  path: string;
  relativePath?: string;
  line?: number;
  column?: number;
  name?: string;
  kind?: string;
  containerName?: string | null;
  preview?: string;
  matchText?: string;
  source: string;
};

type SecretSeverity = "low" | "medium" | "high" | "critical";

type SecretFinding = {
  source: string;
  path?: string;
  kind: string;
  label: string;
  severity: SecretSeverity;
  confidence: "low" | "medium" | "high";
  line: number;
  column: number;
  matchLength: number;
  fingerprint: string;
  preview: string;
};

type SecretFindingInternal = SecretFinding & {
  start: number;
  end: number;
  replacement: string;
};

type SecretPattern = {
  kind: string;
  label: string;
  severity: SecretSeverity;
  confidence: "low" | "medium" | "high";
  regex: RegExp;
  secretGroup?: number;
  labelGroup?: number;
};

type ContextFile = {
  path: string;
  relativePath: string;
  size: number | null;
  truncated: boolean;
  text: string;
  error?: string;
};

type MemorySignal = {
  source: string;
  line: number;
  kind: "decision" | "preference" | "runtime" | "planning" | "heading";
  score: number;
  text: string;
};

type ContextBudgetItem = {
  id: string;
  kind: string;
  source: string;
  score: number;
  reason: string;
  content: string;
  path?: string;
  line?: number;
};

type ChatCompletionResult = {
  body: unknown;
  streamed: boolean;
};

type ToolExecutionUi = {
  setApproval: (approval: AiToolApprovalState) => void;
  setRunning: (approval?: AiToolApprovalState) => void;
};

const maxHistoryMessages = 16;
const maxToolRounds = 8;
const maxActiveDocumentChars = 24_000;
const maxAttachmentChars = 18_000;
const maxToolOutputChars = 24_000;
const maxCheckpointsPerWorkspace = 24;
const defaultCheckpointMaxFiles = 40;
const checkpointMaxFilesLimit = 80;
const defaultCheckpointMaxBytesPerFile = 500_000;
const checkpointMaxBytesPerFileLimit = 1_000_000;
const checkpointStoreByWorkspace = new Map<string, RuntimeCheckpoint[]>();
const relatedIgnoredPathPattern = /(^|\/)(node_modules|target|dist|build|out|coverage|\.git|\.next|\.turbo|vendor|venv|\.venv|__pycache__)(\/|$)/;
const relatedBinaryFilePattern = /\.(7z|avi|bmp|class|db|dll|dmg|exe|gif|gz|ico|jar|jpeg|jpg|lockb|mov|mp3|mp4|o|obj|pdf|png|rar|so|tar|ttf|webm|webp|woff2?|zip)$/;
const relatedSourceFilePattern = /\.(astro|c|cc|cpp|cs|css|cxx|go|graphql|gql|h|hpp|html|java|js|json|jsx|kt|kts|less|md|mdx|mjs|mts|php|proto|py|rb|rs|sass|scss|sql|svelte|swift|toml|ts|tsx|vue|xml|ya?ml)$/;
const relatedStopWords = new Set([
  "about", "after", "also", "and", "any", "are", "bug", "can", "code", "create", "default", "edit", "file", "files", "fix", "for", "from", "get", "has", "have", "into", "make", "need", "new", "not", "now", "please", "set", "that", "the", "this", "tool", "tools", "use", "with", "work",
]);
const relatedShortUsefulTokens = new Set(["ai", "api", "ci", "db", "fs", "gh", "ui", "ux"]);
const rulesContextFileNames = new Set(["agents.md", "claude.md", ".cursorrules", "cursor_rules.md", "cursor-rules.md", "codex.md"]);
const docsContextFilePattern = /(^|\/)(readme|contributing|changelog|architecture|docs?|package\.json|cargo\.toml|pyproject\.toml|go\.mod|pom\.xml|build\.gradle|vite\.config\.|tsconfig\.)/i;
const memoryContextFileNames = new Set(["memory.md", "memories.md", "project-memory.md", "decisions.md", "decision-log.md", "preferences.md", "notes.md", "todo.md", "todos.md", "roadmap.md"]);
const secretPreviewMask = "[REDACTED]";
const secretPatterns: SecretPattern[] = [
  { kind: "openai-api-key", label: "OpenAI API key", severity: "critical", confidence: "high", regex: /\b(sk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,})\b/g, secretGroup: 1 },
  { kind: "github-token", label: "GitHub token", severity: "critical", confidence: "high", regex: /\b((?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{30,})\b/g, secretGroup: 1 },
  { kind: "github-fine-grained-token", label: "GitHub fine-grained token", severity: "critical", confidence: "high", regex: /\b(github_pat_[A-Za-z0-9_]{40,})\b/g, secretGroup: 1 },
  { kind: "slack-token", label: "Slack token", severity: "critical", confidence: "high", regex: /\b(xox[baprs]-[A-Za-z0-9-]{20,})\b/g, secretGroup: 1 },
  { kind: "aws-access-key", label: "AWS access key", severity: "critical", confidence: "high", regex: /\b((?:AKIA|ASIA)[A-Z0-9]{16})\b/g, secretGroup: 1 },
  { kind: "google-api-key", label: "Google API key", severity: "critical", confidence: "high", regex: /\b(AIza[0-9A-Za-z_-]{35})\b/g, secretGroup: 1 },
  { kind: "stripe-key", label: "Stripe key", severity: "critical", confidence: "high", regex: /\b((?:sk|rk)_(?:live|test)_[A-Za-z0-9]{20,})\b/g, secretGroup: 1 },
  { kind: "jwt", label: "JWT", severity: "high", confidence: "medium", regex: /\b(eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,})\b/g, secretGroup: 1 },
  { kind: "private-key-block", label: "Private key block", severity: "critical", confidence: "high", regex: /-----BEGIN (?:RSA |DSA |EC |OPENSSH |PGP )?PRIVATE KEY-----[\s\S]*?-----END (?:RSA |DSA |EC |OPENSSH |PGP )?PRIVATE KEY-----/g },
  { kind: "connection-string-password", label: "Connection string password", severity: "high", confidence: "medium", regex: /\b((?:postgres|postgresql|mysql|mongodb(?:\+srv)?|redis):\/\/[^\s:@/]+:)([^\s@/]{8,})(@[^\s]*)/gi, secretGroup: 2 },
  { kind: "assigned-secret", label: "Assigned secret value", severity: "high", confidence: "medium", regex: /\b([A-Z0-9_.-]*(?:API[_-]?KEY|TOKEN|SECRET|PASSWORD|PASSWD|PRIVATE[_-]?KEY|AUTH|CREDENTIAL)[A-Z0-9_.-]*\s*[:=]\s*["']?)([^"'\s]{12,})/gi, secretGroup: 2, labelGroup: 1 },
  { kind: "bearer-token", label: "Bearer token", severity: "high", confidence: "medium", regex: /\b(Bearer\s+)([A-Za-z0-9._~+/=-]{20,})\b/g, secretGroup: 2, labelGroup: 1 },
];

export async function readChatAttachment(file: File): Promise<AiChatAttachmentInput> {
  const text = await file.text();
  return {
    name: file.name,
    size: file.size,
    text: truncateText(text, maxAttachmentChars),
  };
}

export async function sendAiChatMessage(input: AiChatSendInput): Promise<AiChatMessage> {
  const assistantMessage: AiChatMessage = {
    id: crypto.randomUUID(),
    role: "assistant",
    content: "",
    toolCalls: [],
    timestamp: Date.now(),
  };
  input.onAssistantMessage(assistantMessage);

  const messages = buildInitialMessages(input);
  const toolSession: RuntimeToolSession = { todos: [] };
  let lastAssistantContent = "";
  let toolCalls: AiChatToolCall[] = [];

  for (let round = 0; round <= maxToolRounds; round += 1) {
    throwIfAborted(input.abortSignal);
    const response = await requestChatCompletion(input, messages, (content) => {
      lastAssistantContent = content || lastAssistantContent;
      if (lastAssistantContent) {
        input.onAssistantMessageUpdate(assistantMessage.id, { content: lastAssistantContent, toolCalls });
      }
    });
    const choice = firstChoice(response.body);
    const assistant = normalizeAssistantMessage(choice?.message);
    lastAssistantContent = assistant.content ?? lastAssistantContent;

    if (lastAssistantContent) {
      input.onAssistantMessageUpdate(assistantMessage.id, { content: lastAssistantContent });
    }

    const requestedToolCalls = normalizeToolCalls(assistant.tool_calls);
    if (requestedToolCalls.length === 0) {
      const finalMessage = {
        ...assistantMessage,
        content: lastAssistantContent || "Done.",
        toolCalls,
      };
      input.onAssistantMessageUpdate(assistantMessage.id, finalMessage);
      return finalMessage;
    }

    messages.push({
      role: "assistant",
      content: assistant.content || null,
      tool_calls: requestedToolCalls,
    });

    const roundToolCalls = requestedToolCalls.map((call) => createRunningToolCall(call));
    toolCalls = [...toolCalls, ...roundToolCalls];
    input.onAssistantMessageUpdate(assistantMessage.id, { content: lastAssistantContent, toolCalls });

    const toolResults = await Promise.all(roundToolCalls.map(async (uiCall, index) => {
      const requestedCall = requestedToolCalls[index];
      try {
        const result = await runRuntimeTool(requestedCall, input, toolSession, {
          setApproval: (approval) => {
            const pending = { ...uiCall, status: "approval" as const, approval };
            toolCalls = toolCalls.map((candidate) => candidate.id === uiCall.id ? pending : candidate);
            input.onAssistantMessageUpdate(assistantMessage.id, { content: lastAssistantContent, toolCalls });
          },
          setRunning: (approval) => {
            const running = { ...uiCall, status: "running" as const, approval };
            toolCalls = toolCalls.map((candidate) => candidate.id === uiCall.id ? running : candidate);
            input.onAssistantMessageUpdate(assistantMessage.id, { content: lastAssistantContent, toolCalls });
          },
        });
        toolCalls = toolCalls.map((candidate) => candidate.id === uiCall.id
          ? { ...candidate, status: "success" as const, output: result.title, endTime: Date.now(), stats: result.stats }
          : candidate);
        input.onAssistantMessageUpdate(assistantMessage.id, { content: lastAssistantContent, toolCalls });
        return {
          role: "tool" as const,
          tool_call_id: requestedCall.id ?? uiCall.id,
          content: result.content,
        };
      } catch (error) {
        const message = readErrorMessage(error);
        const skipped = error instanceof ToolApprovalRejectedError;
        toolCalls = toolCalls.map((candidate) => candidate.id === uiCall.id
          ? { ...candidate, status: skipped ? "skipped" as const : "error" as const, error: message, endTime: Date.now() }
          : candidate);
        input.onAssistantMessageUpdate(assistantMessage.id, { content: lastAssistantContent, toolCalls });
        return {
          role: "tool" as const,
          tool_call_id: requestedCall.id ?? uiCall.id,
          content: JSON.stringify({ error: message }),
        };
      }
    }));

    messages.push(...toolResults);
  }

  const limitedMessage = {
    ...assistantMessage,
    content: lastAssistantContent || "Stopped after the tool round limit. Please narrow the request and try again.",
    toolCalls,
  };
  input.onAssistantMessageUpdate(assistantMessage.id, limitedMessage);
  return limitedMessage;
}

function buildInitialMessages(input: AiChatSendInput): ChatCompletionMessage[] {
  const system = buildLuxIdeSystemPrompt({
    preferences: input.preferences,
    provider: input.provider,
    runtimeToolsAvailable: isTauriRuntime(),
    selectedAgentInstructions: input.selectedAgentInstructions,
    selectedAgentName: input.selectedAgentName,
    selectedModel: input.selectedModel,
    workspace: input.workspace,
  });

  const messages: ChatCompletionMessage[] = [{ role: "system", content: system }];
  for (const message of input.history.slice(-maxHistoryMessages)) {
    messages.push({ role: message.role, content: message.content });
  }
  messages.push({ role: "user", content: buildUserContent(input) });
  return messages;
}

function buildUserContent(input: AiChatSendInput) {
  const sections = [`User request:\n${input.message.trim()}`];
  if (input.activeDocument) {
    const path = input.activeDocument.path ?? input.activeDocument.title;
    sections.push(`Active document (${path}, ${input.activeDocument.language_id}, dirty=${input.activeDocument.is_dirty}):\n\`\`\`${input.activeDocument.language_id}\n${truncateText(input.activeDocument.text, maxActiveDocumentChars)}\n\`\`\``);
  }
  if (input.attachments.length > 0) {
    sections.push(`Attachments:\n${input.attachments.map((attachment) => {
      return `### ${attachment.name} (${attachment.size} bytes)\n\`\`\`\n${attachment.text}\n\`\`\``;
    }).join("\n\n")}`);
  }
  return sections.join("\n\n");
}

async function requestChatCompletion(input: AiChatSendInput, messages: ChatCompletionMessage[], onStreamContent: (content: string) => void): Promise<ChatCompletionResult> {
  throwIfAborted(input.abortSignal);
  const desktopRuntime = isTauriRuntime();
  const payload = {
    model: input.selectedModel.alias || input.selectedModel.id,
    messages,
    temperature: 0.2,
    stream: false,
    ...reasoningPayload(input.preferences.selectedEffortId, input.provider),
    ...(desktopRuntime ? { tools: runtimeTools, tool_choice: "auto" } : {}),
  };
  if (desktopRuntime) {
    try {
      return await requestStreamingChatCompletion(input, payload, onStreamContent);
    } catch (error) {
      throwIfAborted(input.abortSignal);
      if (!isStreamFallbackAllowed(error)) throw error;
    }
  }
  const response = desktopRuntime
    ? await luxCommands.aiChatCompletion({
      baseUrl: input.provider.baseUrl,
      apiKey: input.provider.apiKey || null,
      payload,
    })
    : await requestBrowserChatCompletion(input, payload);
  throwIfAborted(input.abortSignal);
  return { body: response.body, streamed: false };
}

async function requestBrowserChatCompletion(input: AiChatSendInput, payload: UnknownRecord) {
  const endpoint = chatCompletionEndpoint(input.provider.baseUrl);
  const headers: Record<string, string> = {
    "Accept": "application/json",
    "Content-Type": "application/json",
  };
  const apiKey = input.provider.apiKey.trim();
  if (apiKey) headers.Authorization = `Bearer ${apiKey}`;

  const response = await fetch(endpoint, {
    body: JSON.stringify({ ...payload, stream: false }),
    headers,
    method: "POST",
    signal: input.abortSignal,
  });
  const body = await response.json().catch(async () => ({ error: { message: await response.text().catch(() => "AI provider returned a non-JSON response") } }));
  if (!response.ok) throw new Error(aiResponseError(response.status, body));
  return { status: response.status, body };
}

function chatCompletionEndpoint(baseUrl: string) {
  const trimmed = baseUrl.trim().replace(/\/+$/g, "");
  if (!trimmed) throw new Error("AI provider base URL is empty");
  const url = parseProviderBaseUrl(trimmed);
  if (!isTauriRuntime() && isLocalLoopbackUrl(url)) {
    const proxyPath = url.pathname.replace(/\/+$/g, "");
    const chatPath = proxyPath.endsWith("/chat/completions") ? proxyPath : `${proxyPath}/chat/completions`;
    return `/__lux_ai_proxy${chatPath}`;
  }
  return trimmed.endsWith("/chat/completions") ? trimmed : `${trimmed}/chat/completions`;
}

function parseProviderBaseUrl(value: string) {
  try {
    const url = new URL(value);
    if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error(`Unsupported AI provider URL scheme: ${url.protocol.replace(/:$/g, "")}`);
    return url;
  } catch (error) {
    throw new Error(error instanceof Error ? error.message : `Invalid AI provider URL: ${value}`);
  }
}

function isLocalLoopbackUrl(url: URL) {
  return url.protocol === "http:" && ["127.0.0.1", "localhost", "::1", "[::1]"].includes(url.hostname);
}

function aiResponseError(status: number, body: unknown) {
  const fallback = `AI provider returned HTTP ${status}`;
  if (!isRecord(body)) return fallback;
  if (isRecord(body.error)) {
    const message = typeof body.error.message === "string" ? body.error.message : null;
    return message ? `AI provider error ${status}: ${message}` : fallback;
  }
  const message = typeof body.message === "string" ? body.message : null;
  return message ? `AI provider error ${status}: ${message}` : fallback;
}

async function requestStreamingChatCompletion(input: AiChatSendInput, payload: UnknownRecord, onStreamContent: (content: string) => void): Promise<ChatCompletionResult> {
  const streamId = crypto.randomUUID();
  let started = false;
  let cleanup: (() => void) | undefined;
  let abortListener: (() => void) | undefined;

  try {
    const result = await new Promise<ChatCompletionResult>((resolve, reject) => {
      const accumulator = createStreamAccumulator();
      let settled = false;

      const settle = (callback: () => void) => {
        if (settled) return;
        settled = true;
        callback();
      };

      const abort = () => {
        void luxCommands.aiChatCompletionStreamCancel(streamId).catch(() => undefined);
        settle(() => reject(new DOMException("AI request was cancelled", "AbortError")));
      };

      if (input.abortSignal.aborted) {
        abort();
        return;
      }

      abortListener = () => abort();
      input.abortSignal.addEventListener("abort", abortListener, { once: true });

      const startStream = () => {
        void luxCommands.aiChatCompletionStream({
          baseUrl: input.provider.baseUrl,
          apiKey: input.provider.apiKey || null,
          payload: { ...payload, stream: true },
          streamId,
        }).catch((error) => {
          settle(() => reject(error));
        });
      };

      void subscribeAiChatStream((event) => {
        if (event.streamId !== streamId || settled) return;
        if (event.kind === "chunk") {
          started = true;
          try {
            const content = applyStreamChunk(accumulator, event.data);
            if (content) onStreamContent(content);
          } catch (error) {
            void luxCommands.aiChatCompletionStreamCancel(streamId).catch(() => undefined);
            settle(() => reject(markStreamingStarted(error)));
          }
          return;
        }
        if (event.kind === "done") {
          started = true;
          settle(() => resolve({ body: streamAccumulatorToCompletion(accumulator), streamed: true }));
          return;
        }
        if (event.kind === "cancelled") {
          settle(() => reject(new DOMException("AI request was cancelled", "AbortError")));
          return;
        }
        if (event.kind === "error") {
          const error = new Error(event.error || "AI stream failed");
          settle(() => reject(started ? markStreamingStarted(error) : error));
        }
      }).then((unlisten) => {
        cleanup = unlisten;
        if (settled) cleanup?.();
        else startStream();
      }).catch((error) => {
        settle(() => reject(error));
      });
    });
    throwIfAborted(input.abortSignal);
    return result;
  } catch (error) {
    if (started && !isAbortErrorLike(error)) throw markStreamingStarted(error);
    throw error;
  } finally {
    cleanup?.();
    if (abortListener) input.abortSignal.removeEventListener("abort", abortListener);
  }
}

type StreamAccumulator = {
  content: string;
  role: string;
  toolCalls: OpenAiToolCall[];
  finishReason: string | null;
};

function createStreamAccumulator(): StreamAccumulator {
  return {
    content: "",
    role: "assistant",
    toolCalls: [],
    finishReason: null,
  };
}

function applyStreamChunk(accumulator: StreamAccumulator, data: unknown) {
  const choice = firstChoice(data);
  if (!choice) return accumulator.content;
  if (typeof choice.finish_reason === "string") accumulator.finishReason = choice.finish_reason;
  const delta = isRecord(choice.delta) ? choice.delta : null;
  if (!delta) return accumulator.content;
  if (typeof delta.role === "string") accumulator.role = delta.role;
  if (typeof delta.content === "string") accumulator.content += delta.content;
  applyToolCallDeltas(accumulator, delta.tool_calls);
  return accumulator.content;
}

function applyToolCallDeltas(accumulator: StreamAccumulator, value: unknown) {
  if (!Array.isArray(value)) return;
  value.filter(isRecord).forEach((delta, fallbackIndex) => {
    const index = clamp(numberArg(delta, "index", fallbackIndex), 0, 128);
    const existing = accumulator.toolCalls[index] ?? { type: "function", function: { name: "", arguments: "" } };
    const next: OpenAiToolCall = {
      ...existing,
      index,
      type: "function",
      id: typeof delta.id === "string" ? delta.id : existing.id,
      function: {
        name: existing.function?.name ?? "",
        arguments: existing.function?.arguments ?? "",
      },
    };
    if (isRecord(delta.function)) {
      if (typeof delta.function.name === "string") {
        next.function = { ...next.function, name: `${next.function?.name ?? ""}${delta.function.name}` };
      }
      if (typeof delta.function.arguments === "string") {
        next.function = { ...next.function, arguments: `${next.function?.arguments ?? ""}${delta.function.arguments}` };
      }
    }
    accumulator.toolCalls[index] = next;
  });
}

function streamAccumulatorToCompletion(accumulator: StreamAccumulator) {
  return {
    choices: [{
      index: 0,
      finish_reason: accumulator.finishReason,
      message: {
        role: accumulator.role,
        content: accumulator.content,
        tool_calls: accumulator.toolCalls.filter(Boolean),
      },
    }],
  };
}

function isStreamFallbackAllowed(error: unknown) {
  return !hasStreamingStarted(error) && !isAbortErrorLike(error);
}

function markStreamingStarted(error: unknown) {
  if (error instanceof Error) {
    (error as Error & { streamingStarted?: boolean }).streamingStarted = true;
    return error;
  }
  const wrapped = new Error(String(error));
  (wrapped as Error & { streamingStarted?: boolean }).streamingStarted = true;
  return wrapped;
}

function hasStreamingStarted(error: unknown) {
  return error instanceof Error && Boolean((error as Error & { streamingStarted?: boolean }).streamingStarted);
}

const runtimeTools: RuntimeToolDefinition[] = [
  {
    type: "function",
    function: {
      name: "FastContext",
      description: "Collect a compact workspace context packet: active file, repo map, diagnostics, git state, and matching files for a query.",
      parameters: objectSchema({
        query: stringSchema("The task or topic to collect context for."),
      }, ["query"]),
    },
  },
  {
    type: "function",
    function: {
      name: "RepoMap",
      description: "Summarize the current workspace structure and important project files.",
      parameters: objectSchema({
        maxFiles: numberSchema("Maximum number of files to include, default 80."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "WorkspaceIndex",
      description: "Return a compact indexed snapshot of the workspace: file counts, language mix, important directories, configs, test files, source files, entrypoints, and largest files. Use to orient before broad changes or when deciding which tool to call next.",
      parameters: objectSchema({
        maxFiles: numberSchema("Maximum representative files per section, default 60."),
        maxScan: numberSchema("Maximum files to scan from the workspace index, default uses AI indexing settings."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "ActiveContext",
      description: "Return the current IDE state available to the AI: active document, open editor tabs, dirty files, attached files, selected model/provider/agent, approval mode, workspace, and terminal session. Use before acting on the user's current editor state.",
      parameters: objectSchema({
        includeActiveText: booleanSchema("Include a truncated copy of the active document text. Default false."),
        maxOpenDocuments: numberSchema("Maximum open documents to return, default 24."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "RulesContext",
      description: "Read project guidance files such as AGENTS.md, CLAUDE.md, .cursorrules, .cursor/rules, and top-level README snippets. Use before editing to follow local conventions and constraints.",
      parameters: objectSchema({
        query: stringSchema("Optional task/topic used to prioritize matching rule files."),
        maxFiles: numberSchema("Maximum rule files to include, default 12."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "DocsContext",
      description: "Collect local documentation and dependency/version context from README/docs/package manifests. Use when answering framework/API questions or before relying on library behavior.",
      parameters: objectSchema({
        query: stringSchema("Library, framework, feature, or file topic to prioritize."),
        maxFiles: numberSchema("Maximum docs/manifests to include, default 12."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "MemoryContext",
      description: "Collect durable local project memory: decisions, preferences, TODOs, roadmap notes, rule files, recent chat instructions, and current AI runtime defaults. Read-only and local to the workspace.",
      parameters: objectSchema({
        query: stringSchema("Optional topic or current task used to prioritize memory signals."),
        maxFiles: numberSchema("Maximum memory/rule files to inspect, default 14."),
        maxSignals: numberSchema("Maximum extracted memory signals to return, default 40."),
        includeRecentChat: booleanSchema("Include recent user/assistant instructions from this chat. Default true."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "ContextBudgeter",
      description: "Build a ranked, compressed context packet under a character budget from active editor state, files, diagnostics, git, rules, docs, memory, related files, and semantic search. Use before long or multi-file work to avoid noisy or oversized context.",
      parameters: objectSchema({
        query: stringSchema("Task, topic, symbol, or change description used to score context relevance."),
        targetChars: numberSchema("Approximate maximum characters for the returned context packet, default 16000, capped below the runtime output limit."),
        includeActiveText: booleanSchema("Include a trimmed excerpt from the active document. Default true."),
        includeOpenDocuments: booleanSchema("Include open editor tabs and dirty file excerpts. Default true."),
        includeToolContext: booleanSchema("Call read-only context tools such as MemoryContext, RulesContext, DocsContext, RelatedFiles, SemanticSearch, GitContext, and DiagnosticsContext. Default true."),
        maxItems: numberSchema("Maximum selected context items to return, default 28."),
      }, ["query"]),
    },
  },
  {
    type: "function",
    function: {
      name: "SemanticSearch",
      description: "Rank code locations by intent using language-server symbols, indexed text hits, and filename relevance. Use when the user asks where behavior is implemented, what owns a feature, or which files to inspect first.",
      parameters: objectSchema({
        query: stringSchema("Feature, symbol, API, error, or natural-language topic to search for."),
        path: stringSchema("Optional workspace-relative or absolute path fragment to prioritize or limit results."),
        maxResults: numberSchema("Maximum ranked results to return, default 24."),
      }, ["query"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Glob",
      description: "List workspace files whose full path contains a simple pattern or extension.",
      parameters: objectSchema({
        pattern: stringSchema("Case-insensitive path fragment, file name, or extension such as .tsx."),
        maxResults: numberSchema("Maximum number of files to return, default 80."),
      }, ["pattern"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Read",
      description: "Read a text file from disk without opening it in the editor.",
      parameters: objectSchema({
        path: stringSchema("Absolute path to the file."),
        maxBytes: numberSchema("Maximum bytes to read, default 120000."),
      }, ["path"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Write",
      description: "Create or fully rewrite a text file inside the workspace. Creates parent directories when needed.",
      parameters: objectSchema({
        path: stringSchema("Workspace-relative or absolute path inside the workspace."),
        text: stringSchema("Complete file contents to write."),
        overwrite: booleanSchema("Allow replacing an existing file. Default false."),
        saveToDisk: booleanSchema("Persist to disk. Default true."),
      }, ["path", "text"]),
    },
  },
  {
    type: "function",
    function: {
      name: "StrReplace",
      description: "Replace an exact text fragment in a workspace file. Fails if the occurrence count does not match expectedReplacements.",
      parameters: objectSchema({
        path: stringSchema("Workspace-relative or absolute path inside the workspace."),
        oldText: stringSchema("Exact text to replace."),
        newText: stringSchema("Replacement text."),
        expectedReplacements: numberSchema("Expected occurrence count, default 1."),
        saveToDisk: booleanSchema("Persist to disk. Default true."),
      }, ["path", "oldText", "newText"]),
    },
  },
  {
    type: "function",
    function: {
      name: "PatchEngine",
      description: "Apply a guarded multi-file patch with full preflight validation, one approval, rollback on disk-write failure, exact replacement counts, and optional dry-run. Prefer this over many separate Write/StrReplace/Delete calls for coordinated edits.",
      parameters: objectSchema({
        operations: {
          type: "array",
          description: "Ordered patch operations. Actions: create, rewrite, replace, delete. Create/rewrite use text; replace uses oldText/newText/expectedReplacements; delete removes one file.",
          items: objectSchema({
            action: stringSchema("create, rewrite, replace, or delete."),
            path: stringSchema("Workspace-relative or absolute path inside the workspace."),
            text: stringSchema("Complete file contents for create/rewrite."),
            oldText: stringSchema("Exact text to replace for replace operations."),
            newText: stringSchema("Replacement text for replace operations."),
            expectedReplacements: numberSchema("Expected occurrence count for replace operations, default 1."),
            overwrite: booleanSchema("Allow create to overwrite an existing file. Default false."),
          }, ["action", "path"]),
        },
        saveToDisk: booleanSchema("Persist to disk. Default true."),
        dryRun: booleanSchema("Validate and summarize without modifying files. Default false."),
      }, ["operations"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Checkpoint",
      description: "Create, list, diff, delete, or restore in-session text snapshots for workspace files. Use create before risky edits and restore to roll back through the guarded PatchEngine approval path.",
      parameters: objectSchema({
        action: stringSchema("create, list, diff, delete, or restore."),
        id: stringSchema("Checkpoint id for diff, delete, or restore. Defaults to the latest checkpoint."),
        label: stringSchema("Optional short label for create."),
        paths: arraySchema("Workspace-relative or absolute file paths to snapshot, diff, or restore. For create, omitted paths default to changed/open/active files; diff/restore default to all checkpoint files."),
        includeOpenDocuments: booleanSchema("For create with omitted paths, include open editor documents. Default true."),
        includeGitChanges: booleanSchema("For create with omitted paths, include current git changed files. Default true."),
        maxFiles: numberSchema("Maximum files to snapshot or inspect, default 40, maximum 80."),
        maxBytesPerFile: numberSchema("Maximum bytes read per file, default 500000, maximum 1000000. Truncated files cannot be restored."),
        saveToDisk: booleanSchema("For restore, persist to disk. Default true."),
        dryRun: booleanSchema("For restore, validate and preview operations without modifying files. Default false."),
      }, ["action"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Delete",
      description: "Delete a file or directory inside the workspace. Use only when the requested change clearly requires removal.",
      parameters: objectSchema({
        path: stringSchema("Workspace-relative or absolute path inside the workspace."),
      }, ["path"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Shell",
      description: "Run a non-interactive shell command in the workspace after explicit user approval. Use for build, test, lint, and diagnostic commands. Do not use for interactive, long-running, network credential, or destructive commands unless the user clearly requested them.",
      parameters: objectSchema({
        command: stringSchema("The exact shell command to run."),
        cwd: stringSchema("Optional workspace-relative or absolute working directory inside the workspace."),
        timeoutSecs: numberSchema("Optional timeout in seconds, default 120, maximum 600."),
      }, ["command"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Grep",
      description: "Search text in the current workspace using the IDE search index.",
      parameters: objectSchema({
        query: stringSchema("Text or regex to search for."),
        useRegex: booleanSchema("Treat query as a regular expression."),
        caseSensitive: booleanSchema("Use case-sensitive matching."),
        includeGlobs: arraySchema("Optional include glob patterns."),
        maxResults: numberSchema("Maximum search hits, default 50."),
      }, ["query"]),
    },
  },
  {
    type: "function",
    function: {
      name: "DiagnosticsContext",
      description: "Return current IDE diagnostics grouped as compiler/language-server findings.",
      parameters: objectSchema({
        maxResults: numberSchema("Maximum diagnostics to return, default 80."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "ReadLints",
      description: "Read current linter and language diagnostics with filters for path, severity, and source. Use after edits or before claiming code is clean.",
      parameters: objectSchema({
        path: stringSchema("Optional workspace-relative or absolute path filter."),
        severity: stringSchema("Optional severity filter: error, warning, information, or hint."),
        source: stringSchema("Optional diagnostic source filter such as eslint, typescript, rustc, or pylance."),
        maxResults: numberSchema("Maximum diagnostics to return, default 80."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "TodoWrite",
      description: "Replace the current AI session task list with structured todo items and progress states. Use for multi-step work to make progress visible; this does not edit project files.",
      parameters: objectSchema({
        todos: {
          type: "array",
          description: "Complete ordered task list for the current response.",
          items: objectSchema({
            id: stringSchema("Stable short id. If omitted, Lux creates one."),
            content: stringSchema("Concrete task description."),
            status: stringSchema("pending, in_progress, completed, blocked, or cancelled."),
            priority: stringSchema("low, medium, or high."),
            notes: stringSchema("Optional short context or result."),
          }, ["content", "status"]),
        },
      }, ["todos"]),
    },
  },
  {
    type: "function",
    function: {
      name: "WebFetch",
      description: "Fetch a specific HTTP/HTTPS URL and return cleaned text plus metadata. Use for current docs, release notes, error pages, and user-provided links. Private network hosts are blocked unless explicitly allowed.",
      parameters: objectSchema({
        url: stringSchema("The absolute HTTP or HTTPS URL to fetch."),
        maxBytes: numberSchema("Maximum response bytes to read, default 250000, maximum 1000000."),
        timeoutSecs: numberSchema("Request timeout in seconds, default 20, maximum 60."),
        allowPrivateHosts: booleanSchema("Allow localhost/private IP targets. Default false; use only for explicit local URLs."),
      }, ["url"]),
    },
  },
  {
    type: "function",
    function: {
      name: "SymbolContext",
      description: "Return semantic code intelligence from the active language servers: workspace symbols for a query, document symbols for a file, and hover/definition/reference/signature data for an exact position. Prefer this before editing unfamiliar code or when reasoning about APIs, call sites, or symbols.",
      parameters: objectSchema({
        query: stringSchema("Optional symbol name or topic to search in the workspace and filter document symbols."),
        path: stringSchema("Optional workspace-relative or absolute file path for document symbols or position context."),
        line: numberSchema("Optional 1-based line for hover/definition/references/signature context."),
        column: numberSchema("Optional 1-based UTF-16 column for hover/definition/references/signature context."),
        maxResults: numberSchema("Maximum symbols or locations to return, default 80."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "RelatedFiles",
      description: "Find files related to a target file or topic: tests, styles, types, stories, routes, schemas, configs, entrypoints, barrels, and nearby same-name modules. Use before editing to understand likely companions and validation targets.",
      parameters: objectSchema({
        path: stringSchema("Optional workspace-relative or absolute target file. Defaults to the active document."),
        query: stringSchema("Optional topic, symbol, feature, or filename fragments to prioritize."),
        maxResults: numberSchema("Maximum related files to return, default 40."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "GitContext",
      description: "Return the current git branch, ahead/behind counts, and changed files.",
      parameters: objectSchema({}),
    },
  },
  {
    type: "function",
    function: {
      name: "TestHealth",
      description: "Detect and run workspace tests and nearest validation commands across common languages and build systems, then return pass/fail status, command kind, duration, exit code, and compact logs.",
      parameters: objectSchema({}),
    },
  },
  {
    type: "function",
    function: {
      name: "FailureAnalyzer",
      description: "Analyze failing test output, compiler diagnostics, or pasted logs and return root-cause candidates, important evidence lines, affected files, and focused next actions. Use after TestHealth, Shell, or when the user provides error logs.",
      parameters: objectSchema({
        log: stringSchema("Optional raw test, build, CI, or terminal output to analyze."),
        includeTestHealth: booleanSchema("Run TestHealth and analyze its current output. Default true."),
        includeDiagnostics: booleanSchema("Include current IDE diagnostics in the analysis. Default true."),
        maxFindings: numberSchema("Maximum findings to return, default 12."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "ImpactAnalysis",
      description: "Estimate blast radius for a planned or active change: related files, tests, diagnostics, configs, entrypoints, and likely validation commands. Use before broad edits.",
      parameters: objectSchema({
        path: stringSchema("Optional workspace-relative or absolute target file. Defaults to the active document."),
        query: stringSchema("Change description or topic to analyze."),
        maxResults: numberSchema("Maximum related files to include, default 32."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "ReviewDiff",
      description: "Review the current workspace diff as a quality gate: summarize changed files, risk signals, missing tests, diagnostics, and recommended verification. Read-only.",
      parameters: objectSchema({
        includePatch: booleanSchema("Include a truncated patch excerpt. Default true."),
        maxFindings: numberSchema("Maximum review findings to return, default 12."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "SecretGuard",
      description: "Scan provided text and/or the current workspace diff for likely secrets, credentials, tokens, and private keys. Returns redacted previews and optional redacted text. Read-only.",
      parameters: objectSchema({
        text: stringSchema("Optional logs, shell output, patch, env text, or other content to scan."),
        path: stringSchema("Optional source path label for the provided text."),
        includeDiff: booleanSchema("Also scan the current workspace diff. Default true when text is empty, false otherwise."),
        returnRedactedText: booleanSchema("Return a redacted copy of the provided text. Default false."),
        maxFindings: numberSchema("Maximum findings to return, default 30."),
      }),
    },
  },
];

async function runRuntimeTool(call: OpenAiToolCall, input: AiChatSendInput, session: RuntimeToolSession, ui: ToolExecutionUi): Promise<ToolResult> {
  const name = call.function?.name as RuntimeToolName | undefined;
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
    case "Write":
      return writeFileTool(args, input, ui);
    case "StrReplace":
      return strReplaceTool(args, input, ui);
    case "PatchEngine":
      return patchEngineTool(args, input, ui);
    case "Checkpoint":
      return checkpointTool(args, input, ui);
    case "Delete":
      return deleteFileTool(stringArg(args, "path"), input, ui);
    case "Shell":
      return shellTool(args, input, ui);
    case "Grep":
      return grepTool(args);
    case "ReadLints":
      return readLints(args, input);
    case "TodoWrite":
      return todoWrite(args, session);
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
      return secretGuard(args);
    default:
      throw new Error(`Unknown tool: ${name ?? "missing"}`);
  }
}

async function fastContext(input: AiChatSendInput, query: string): Promise<ToolResult> {
  const [active, index, repo, rules, memory, diagnostics, git, related, impact, search] = await Promise.allSettled([
    activeContext({ maxOpenDocuments: 16 }, input),
    workspaceIndex({ maxFiles: 24, maxScan: 2_500 }, input),
    repoMap(48),
    rulesContext({ query, maxFiles: 8 }, input),
    memoryContext({ query, maxFiles: 8, maxSignals: 24, includeRecentChat: true }, input),
    diagnosticsContext(40),
    gitContext(),
    relatedFiles({ query, maxResults: 24 }, input),
    impactAnalysis({ query, maxResults: 18 }, input),
    query.trim() ? grepTool({ query, maxResults: 20, useRegex: false, caseSensitive: false }) : globFiles("", 40),
  ]);
  const parts = [
    `Active document: ${input.activeDocument?.path ?? input.activeDocument?.title ?? "none"}`,
    settledContent("ActiveContext", active),
    settledContent("WorkspaceIndex", index),
    settledContent("RepoMap", repo),
    settledContent("RulesContext", rules),
    settledContent("MemoryContext", memory),
    settledContent("DiagnosticsContext", diagnostics),
    settledContent("GitContext", git),
    settledContent("RelatedFiles", related),
    settledContent("ImpactAnalysis", impact),
    settledContent("Search", search),
  ];
  return toolJson("FastContext", { query, context: parts.join("\n\n") });
}

async function repoMap(maxFiles: number): Promise<ToolResult> {
  const files = await luxCommands.fsListFiles(clamp(maxFiles, 1, 500));
  const important = files
    .filter((entry) => entry.kind === "file")
    .sort((left, right) => scorePath(right.path) - scorePath(left.path) || left.path.localeCompare(right.path))
    .slice(0, clamp(maxFiles, 1, 500));
  return toolJson("RepoMap", {
    totalListed: files.length,
    files: important.map((entry) => ({ path: entry.path, size: entry.size, modifiedAt: entry.modified_at })),
  });
}

async function workspaceIndex(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const maxFiles = clamp(numberArg(args, "maxFiles", 60), 1, 180);
  const maxScan = clamp(numberArg(args, "maxScan", input.preferences.maxIndexedFiles), 500, 20_000);
  const entries = await luxCommands.fsListFiles(maxScan);
  const files = entries.filter((entry) => entry.kind === "file" && !isLowSignalRelatedPath(entry.path));
  const descriptors = files.map((entry) => createRelatedFileDescriptor(entry, input.workspace?.root ?? ""));
  const byLanguage = topCounts(descriptors.map((file) => languageForPath(file.basenameLower)), 20);
  const byDirectory = topCounts(descriptors.map((file) => topDirectory(file.relativePath)), 24);
  const important = descriptors
    .filter(isImportantProjectFile)
    .sort((left, right) => scorePath(right.relativePath) - scorePath(left.relativePath) || left.relativeLower.localeCompare(right.relativeLower))
    .slice(0, maxFiles);
  const tests = descriptors.filter(isTestFile).sort(compareRelatedDescriptors).slice(0, maxFiles);
  const source = descriptors.filter((file) => isSourcePath(file) && !isTestFile(file)).sort(compareRelatedDescriptors).slice(0, maxFiles);
  const entrypoints = descriptors.filter(isEntrypointFile).sort(compareRelatedDescriptors).slice(0, maxFiles);
  const largest = [...descriptors]
    .sort((left, right) => (right.entry?.size ?? 0) - (left.entry?.size ?? 0) || left.relativeLower.localeCompare(right.relativeLower))
    .slice(0, Math.min(20, maxFiles));

  return toolJson("WorkspaceIndex", {
    workspaceRoot: input.workspace?.root ?? null,
    scanned: entries.length,
    indexedFiles: descriptors.length,
    truncated: entries.length >= maxScan,
    indexSettings: {
      enabled: input.preferences.projectIndexingEnabled,
      realtime: input.preferences.realtimeIndexing,
      maxIndexedFiles: input.preferences.maxIndexedFiles,
      includeImages: input.preferences.includeImages,
    },
    languageMix: byLanguage,
    topDirectories: byDirectory,
    importantFiles: important.map(compactIndexedFile),
    entrypoints: entrypoints.map(compactIndexedFile),
    sourceFiles: source.map(compactIndexedFile),
    testFiles: tests.map(compactIndexedFile),
    largestFiles: largest.map(compactIndexedFile),
  });
}

function activeContext(args: UnknownRecord, input: AiChatSendInput): ToolResult {
  const includeActiveText = booleanArg(args, "includeActiveText", false);
  const maxOpenDocuments = clamp(numberArg(args, "maxOpenDocuments", 24), 1, 80);
  const activePath = input.activeDocument?.path ?? input.activeDocument?.title ?? null;
  const openDocuments = input.openDocuments.slice(0, maxOpenDocuments).map((document) => ({
    id: document.id,
    path: document.path,
    title: document.title,
    language: document.language_id,
    dirty: document.is_dirty,
    untitled: document.is_untitled,
    active: document.id === input.activeDocument?.id,
    size: document.text.length,
    lines: countLines(document.text),
  }));
  return toolJson("ActiveContext", {
    workspace: input.workspace ? { name: input.workspace.name, root: input.workspace.root } : null,
    activeDocument: input.activeDocument ? {
      id: input.activeDocument.id,
      path: input.activeDocument.path,
      title: input.activeDocument.title,
      language: input.activeDocument.language_id,
      dirty: input.activeDocument.is_dirty,
      untitled: input.activeDocument.is_untitled,
      size: input.activeDocument.text.length,
      lines: countLines(input.activeDocument.text),
      text: includeActiveText ? truncateText(input.activeDocument.text, maxActiveDocumentChars) : undefined,
    } : null,
    openDocuments,
    openDocumentCount: input.openDocuments.length,
    dirtyDocuments: input.openDocuments
      .filter((document) => document.is_dirty)
      .map((document) => document.path ?? document.title),
    attachments: input.attachments.map((attachment) => ({ name: attachment.name, size: attachment.size, textLength: attachment.text.length })),
    terminal: input.terminal ? {
      id: input.terminal.id,
      shell: input.terminal.shell,
      cwd: input.terminal.cwd,
      createdAt: input.terminal.created_at,
    } : null,
    chat: {
      currentMessage: input.message,
      historyMessages: input.history.length,
      lastUserMessage: [...input.history].reverse().find((message) => message.role === "user")?.content ?? null,
    },
    aiRuntime: {
      provider: input.provider.name,
      protocol: input.provider.protocol,
      baseUrl: input.provider.baseUrl,
      model: input.selectedModel.alias || input.selectedModel.id,
      reasoningEffort: input.preferences.selectedEffortId,
      agent: input.selectedAgentName || input.preferences.agentMode,
      toolApprovalMode: input.preferences.toolApprovalMode,
    },
    notes: [
      activePath ? `Active document is ${activePath}.` : "No active document is open.",
      input.preferences.toolApprovalMode === "full-access" ? "Dangerous tools auto-run inside workspace guards." : "Dangerous tools require explicit approval.",
    ],
  });
}

async function rulesContext(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message);
  const maxFiles = clamp(numberArg(args, "maxFiles", 12), 1, 40);
  const entries = await luxCommands.fsListFiles(clamp(input.preferences.maxIndexedFiles, 500, 20_000));
  const workspaceRoot = input.workspace?.root ?? "";
  const queryTokens = tokenizeRelatedQuery(query);
  const candidates = entries
    .filter((entry) => entry.kind === "file" && isRulesContextPath(entry.path, workspaceRoot))
    .map((entry) => createRelatedFileDescriptor(entry, workspaceRoot))
    .sort((left, right) => scoreRulesFile(right, queryTokens) - scoreRulesFile(left, queryTokens) || left.relativeLower.localeCompare(right.relativeLower))
    .slice(0, maxFiles);
  const files = await readContextFiles(candidates, 10_000);
  return toolJson("RulesContext", {
    workspaceRoot: input.workspace?.root ?? null,
    query,
    count: files.length,
    files,
    notes: files.length > 0
      ? ["Follow these local rules when choosing tools, editing code, and explaining changes."]
      : ["No dedicated project rule files were found in the current workspace scan."],
  });
}

async function docsContext(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message);
  const maxFiles = clamp(numberArg(args, "maxFiles", 12), 1, 40);
  const entries = await luxCommands.fsListFiles(clamp(input.preferences.maxIndexedFiles, 500, 20_000));
  const workspaceRoot = input.workspace?.root ?? "";
  const queryTokens = tokenizeRelatedQuery(query);
  const candidates = entries
    .filter((entry) => entry.kind === "file" && isDocsContextPath(entry.path, workspaceRoot))
    .map((entry) => createRelatedFileDescriptor(entry, workspaceRoot))
    .sort((left, right) => scoreDocsFile(right, queryTokens) - scoreDocsFile(left, queryTokens) || left.relativeLower.localeCompare(right.relativeLower))
    .slice(0, maxFiles);
  const files = await readContextFiles(candidates, 12_000);
  return toolJson("DocsContext", {
    workspaceRoot: input.workspace?.root ?? null,
    query,
    dependencies: files
      .filter((file) => /(^|\/)(package\.json|cargo\.toml|pyproject\.toml|go\.mod|pom\.xml|build\.gradle)$/.test(file.relativePath.toLowerCase()))
      .map((file) => summarizeManifest(file.relativePath, file.text)),
    count: files.length,
    files,
  });
}

async function memoryContext(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message);
  const maxFiles = clamp(numberArg(args, "maxFiles", 14), 1, 40);
  const maxSignals = clamp(numberArg(args, "maxSignals", 40), 1, 120);
  const includeRecentChat = booleanArg(args, "includeRecentChat", true);
  const entries = await luxCommands.fsListFiles(clamp(input.preferences.maxIndexedFiles, 500, 20_000));
  const workspaceRoot = input.workspace?.root ?? "";
  const queryTokens = tokenizeRelatedQuery(query);
  const candidates = entries
    .filter((entry) => entry.kind === "file" && isMemoryContextPath(entry.path, workspaceRoot))
    .map((entry) => createRelatedFileDescriptor(entry, workspaceRoot))
    .sort((left, right) => scoreMemoryFile(right, queryTokens) - scoreMemoryFile(left, queryTokens) || left.relativeLower.localeCompare(right.relativeLower))
    .slice(0, maxFiles);
  const files = await readContextFiles(candidates, 14_000);
  const fileSignals = files.flatMap((file) => extractMemorySignals(file, queryTokens));
  const chatSignals = includeRecentChat ? extractChatMemorySignals(input, queryTokens) : [];
  const runtimeSignals = buildRuntimeMemorySignals(input, queryTokens);
  const signals = rankMemorySignals([...runtimeSignals, ...chatSignals, ...fileSignals], queryTokens).slice(0, maxSignals);

  return toolJson("MemoryContext", {
    workspaceRoot: input.workspace?.root ?? null,
    query,
    filesScanned: files.length,
    signalsReturned: signals.length,
    runtime: {
      provider: input.provider.name,
      protocol: input.provider.protocol,
      baseUrl: input.provider.baseUrl,
      model: input.selectedModel.alias || input.selectedModel.id,
      reasoningEffort: input.preferences.selectedEffortId,
      agent: input.selectedAgentName || input.preferences.agentMode,
      toolApprovalMode: input.preferences.toolApprovalMode,
      indexing: {
        enabled: input.preferences.projectIndexingEnabled,
        realtime: input.preferences.realtimeIndexing,
        maxIndexedFiles: input.preferences.maxIndexedFiles,
      },
    },
    files: files.map((file) => ({
      path: file.path,
      relativePath: file.relativePath,
      size: file.size,
      truncated: file.truncated,
      error: file.error,
      signalCount: fileSignals.filter((signal) => signal.source === file.relativePath).length,
    })),
    signals: signals.map(({ score: _score, ...signal }) => signal),
    notes: [
      "MemoryContext is read-only; it does not persist new memories.",
      files.length > 0 ? "Use high-signal decisions and preferences before changing code or tool behavior." : "No dedicated local memory files were found; runtime preferences and recent chat were used instead.",
    ],
  });
}

async function contextBudgeter(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message).trim();
  if (!query) throw new Error("ContextBudgeter requires a non-empty query.");
  const requestedTargetChars = clamp(numberArg(args, "targetChars", 16_000), 2_000, 22_000);
  const targetChars = Math.min(requestedTargetChars, maxToolOutputChars - 8_000);
  const maxItems = clamp(numberArg(args, "maxItems", 28), 4, 80);
  const includeActiveText = booleanArg(args, "includeActiveText", true);
  const includeOpenDocuments = booleanArg(args, "includeOpenDocuments", true);
  const includeToolContext = booleanArg(args, "includeToolContext", true);
  const queryTokens = tokenizeRelatedQuery(query);
  const items: ContextBudgetItem[] = [];

  addDirectContextBudgetItems(items, input, query, queryTokens, includeActiveText, includeOpenDocuments);

  const toolResults: PromiseSettledResult<ToolResult>[] = includeToolContext ? await Promise.allSettled([
    memoryContext({ query, maxFiles: 8, maxSignals: 24, includeRecentChat: true }, input),
    rulesContext({ query, maxFiles: 6 }, input),
    docsContext({ query, maxFiles: 6 }, input),
    relatedFiles({ query, path: input.activeDocument?.path ?? "", maxResults: 24 }, input),
    semanticSearch({ query, maxResults: 18 }, input),
    diagnosticsContext(40),
    gitContext(),
  ]) : [];
  if (includeToolContext) addToolContextBudgetItems(items, toolResults, queryTokens);

  const rankedItems = rankContextBudgetItems(items, queryTokens);
  const selected = selectContextBudgetItems(rankedItems, targetChars, maxItems);
  const selectedChars = selected.reduce((sum, item) => sum + item.content.length, 0);
  const dropped = rankedItems.length - selected.length;
  const byKind = topCounts(selected.map((item) => item.kind), 16);
  const packet = selected.map((item, index) => ({
    index: index + 1,
    id: item.id,
    kind: item.kind,
    source: item.source,
    path: item.path,
    line: item.line,
    reason: item.reason,
    chars: item.content.length,
    content: item.content,
  }));

  return toolJson("ContextBudgeter", {
    workspaceRoot: input.workspace?.root ?? null,
    query,
    budget: {
      requestedTargetChars,
      targetChars,
      selectedChars,
      utilization: targetChars > 0 ? Number((selectedChars / targetChars).toFixed(3)) : 0,
      candidateItems: rankedItems.length,
      selectedItems: selected.length,
      droppedItems: dropped,
      truncatedItems: selected.filter((item) => item.content.includes("...[truncated ")).length,
      maxItems,
    },
    byKind,
    packet,
    nextActions: buildContextBudgeterNextActions(selected),
    unavailable: contextBudgeterUnavailable(toolResults),
    notes: [
      "ContextBudgeter is read-only and returns a compact packet for the next reasoning step.",
      "Scores combine source priority, query-token hits, active editor state, diagnostics, git status, project rules, docs, memory, and related files.",
    ],
  });
}

async function readContextFiles(files: RelatedFileDescriptor[], maxBytes: number): Promise<ContextFile[]> {
  const settled = await Promise.allSettled(files.map(async (file) => {
    const response = await luxCommands.fsReadText(file.path, maxBytes);
    return {
      path: response.path,
      relativePath: file.relativePath,
      size: response.size,
      truncated: response.truncated,
      text: truncateText(response.text, Math.min(maxBytes, 12_000)),
    } satisfies ContextFile;
  }));
  return settled.map((result, index): ContextFile => {
    if (result.status === "fulfilled") return result.value;
    return { path: files[index].path, relativePath: files[index].relativePath, size: files[index].entry?.size ?? null, truncated: false, error: readErrorMessage(result.reason), text: "" };
  });
}

async function globFiles(pattern: string, maxResults: number): Promise<ToolResult> {
  const files = await luxCommands.fsListFiles(Math.max(clamp(maxResults, 1, 500) * 4, 200));
  const needle = pattern.trim().toLowerCase();
  const matched = files
    .filter((entry) => entry.kind === "file")
    .filter((entry) => !needle || entry.path.toLowerCase().includes(needle))
    .slice(0, clamp(maxResults, 1, 500));
  return toolJson("Glob", {
    pattern,
    count: matched.length,
    files: matched.map((entry) => ({ path: entry.path, size: entry.size })),
  });
}

async function readFileTool(path: string, maxBytes: number): Promise<ToolResult> {
  const response = await luxCommands.fsReadText(path, clamp(maxBytes, 1_000, 1_000_000));
  return toolJson("Read", {
    path: response.path,
    size: response.size,
    truncated: response.truncated,
    text: truncateText(response.text, maxToolOutputChars),
  });
}

async function semanticSearch(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message).trim();
  if (!query) throw new Error("SemanticSearch requires a non-empty query.");
  const maxResults = clamp(numberArg(args, "maxResults", 24), 1, 80);
  const pathFilter = normalizePathSlashes(stringArg(args, "path", "")).toLowerCase();
  const queryTokens = tokenizeRelatedQuery(query);
  const workspaceRoot = input.workspace?.root ?? "";

  const [symbolsResult, searchResult, filesResult] = await Promise.allSettled([
    luxCommands.lspWorkspaceSymbols(query),
    luxCommands.searchQuery(query, {
      case_sensitive: false,
      whole_word: false,
      use_regex: false,
      include_hidden: false,
      include_globs: [],
      exclude_globs: [],
      max_results: Math.min(120, Math.max(maxResults * 4, 40)),
    }),
    luxCommands.fsListFiles(clamp(input.preferences.maxIndexedFiles, 500, 20_000)),
  ]);

  const results = new Map<string, SemanticSearchResult>();
  const symbols = symbolsResult.status === "fulfilled" ? symbolsResult.value : [];
  for (const symbol of symbols) {
    const path = normalizePathSlashes(symbol.location.path);
    if (!passesSemanticPathFilter(path, pathFilter)) continue;
    const score = scoreSemanticSymbol(symbol, query, queryTokens, path, workspaceRoot);
    upsertSemanticResult(results, {
      type: "symbol",
      source: "lsp-symbols",
      score,
      path,
      relativePath: createRelatedFileDescriptor({ path }, workspaceRoot).relativePath,
      line: symbol.location.range.start_line + 1,
      column: symbol.location.range.start_column + 1,
      name: symbol.name,
      kind: String(symbol.kind),
      containerName: symbol.container_name,
      preview: [symbol.container_name, symbol.name].filter(Boolean).join("."),
    });
  }

  const search = searchResult.status === "fulfilled" ? searchResult.value : null;
  for (const hit of search?.hits ?? []) {
    const path = normalizePathSlashes(hit.path);
    if (!passesSemanticPathFilter(path, pathFilter)) continue;
    const score = scoreSemanticTextHit(path, hit.preview, hit.match_text, queryTokens, workspaceRoot);
    upsertSemanticResult(results, {
      type: "text",
      source: "indexed-search",
      score,
      path,
      relativePath: createRelatedFileDescriptor({ path }, workspaceRoot).relativePath,
      line: hit.line,
      column: hit.column,
      matchText: hit.match_text,
      preview: hit.preview,
    });
  }

  const entries = filesResult.status === "fulfilled" ? filesResult.value : [];
  const fileCandidates = entries
    .filter((entry) => entry.kind === "file" && !isLowSignalRelatedPath(entry.path))
    .map((entry) => createRelatedFileDescriptor(entry, workspaceRoot))
    .filter((file) => passesSemanticPathFilter(file.path, pathFilter))
    .map((file) => ({ file, score: scoreSemanticFile(file, queryTokens) }))
    .filter((item) => item.score > 0)
    .sort((left, right) => right.score - left.score || left.file.relativeLower.localeCompare(right.file.relativeLower))
    .slice(0, Math.min(maxResults * 2, 80));
  for (const { file, score } of fileCandidates) {
    upsertSemanticResult(results, {
      type: "file",
      source: "workspace-index",
      score,
      path: file.path,
      relativePath: file.relativePath,
      name: file.basename,
      kind: languageForPath(file.basenameLower),
      preview: file.relativePath,
    });
  }

  const ranked = Array.from(results.values())
    .sort((left, right) => right.score - left.score || left.path.localeCompare(right.path) || (left.line ?? 0) - (right.line ?? 0))
    .slice(0, maxResults);

  return toolJson("SemanticSearch", {
    workspaceRoot: input.workspace?.root ?? null,
    query,
    pathFilter: pathFilter || null,
    count: ranked.length,
    results: ranked,
    unavailable: {
      symbols: symbolsResult.status === "rejected" ? readErrorMessage(symbolsResult.reason) : null,
      textSearch: searchResult.status === "rejected" ? readErrorMessage(searchResult.reason) : null,
      workspaceIndex: filesResult.status === "rejected" ? readErrorMessage(filesResult.reason) : null,
    },
  });
}

async function readLints(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const pathFilter = normalizePathSlashes(stringArg(args, "path", input.activeDocument?.path ?? "")).toLowerCase();
  const severity = stringArg(args, "severity", "").trim().toLowerCase();
  const source = stringArg(args, "source", "").trim().toLowerCase();
  const maxResults = clamp(numberArg(args, "maxResults", 80), 1, 500);
  const diagnostics = await luxCommands.diagnosticsSnapshot();
  const filtered = diagnostics
    .filter((diagnostic) => !pathFilter || normalizePathSlashes(diagnostic.path).toLowerCase().includes(pathFilter))
    .filter((diagnostic) => !severity || diagnostic.severity.toLowerCase() === severity)
    .filter((diagnostic) => !source || diagnostic.source.toLowerCase().includes(source))
    .sort(compareDiagnostics)
    .slice(0, maxResults);
  return toolJson("ReadLints", {
    workspaceRoot: input.workspace?.root ?? null,
    filters: {
      path: pathFilter || null,
      severity: severity || null,
      source: source || null,
    },
    totalDiagnostics: diagnostics.length,
    count: filtered.length,
    bySeverity: topCounts(diagnostics.map((diagnostic) => diagnostic.severity), 8),
    bySource: topCounts(diagnostics.map((diagnostic) => diagnostic.source || "unknown"), 16),
    diagnostics: filtered.map((diagnostic) => ({
      path: diagnostic.path,
      line: diagnostic.line,
      column: diagnostic.column,
      severity: diagnostic.severity,
      source: diagnostic.source,
      message: diagnostic.message,
    })),
  });
}

function todoWrite(args: UnknownRecord, session: RuntimeToolSession): ToolResult {
  const rawTodos = args.todos;
  if (!Array.isArray(rawTodos)) throw new Error("TodoWrite requires a todos array.");
  const todos = rawTodos.map(normalizeSessionTodo).filter((todo): todo is SessionTodo => Boolean(todo));
  if (todos.length === 0) throw new Error("TodoWrite requires at least one valid todo item.");
  session.todos = todos;
  const statusCounts = topCounts(todos.map((todo) => todo.status), 8);
  return toolJson("TodoWrite", {
    count: todos.length,
    statusCounts,
    todos,
    notes: ["This task list is scoped to the current AI response and does not modify workspace files."],
  });
}

async function webFetchTool(args: UnknownRecord): Promise<ToolResult> {
  const url = stringArg(args, "url", "").trim();
  if (!url) throw new Error("WebFetch requires a URL.");
  const maxBytes = clamp(numberArg(args, "maxBytes", 250_000), 1_024, 1_000_000);
  const timeoutSecs = clamp(numberArg(args, "timeoutSecs", 20), 1, 60);
  const allowPrivateHosts = booleanArg(args, "allowPrivateHosts", false);
  const response = await luxCommands.webFetch(url, maxBytes, timeoutSecs, allowPrivateHosts);
  const scan = scanSecrets(response.text, response.finalUrl || response.url);
  return toolJson("WebFetch", {
    url: response.url,
    finalUrl: response.finalUrl,
    status: response.status,
    contentType: response.contentType,
    title: response.title,
    bytesRead: response.bytesRead,
    truncated: response.truncated,
    elapsedMs: response.elapsedMs,
    text: scan.redactedText,
    secretGuard: {
      redacted: scan.findings.length > 0,
      findingCount: scan.findings.length,
      findings: scan.findings.slice(0, 20).map(publicSecretFinding),
    },
  });
}

async function secretGuard(args: UnknownRecord): Promise<ToolResult> {
  const text = stringArg(args, "text", "");
  const path = stringArg(args, "path", "provided-text");
  const includeDiff = booleanArg(args, "includeDiff", !text.trim());
  const returnRedactedText = booleanArg(args, "returnRedactedText", false);
  const maxFindings = clamp(numberArg(args, "maxFindings", 30), 1, 120);
  const scans: Array<{ source: string; text: string; redactedText: string; findings: SecretFindingInternal[] }> = [];

  if (text) scans.push(scanSecrets(text, path || "provided-text"));
  let diffUnavailable: string | null = null;
  if (includeDiff) {
    try {
      const diff = await luxCommands.gitDiff();
      scans.push(scanSecrets(diff.patch ?? "", "git.diff"));
    } catch (error) {
      diffUnavailable = readErrorMessage(error);
    }
  }

  const findings = scans.flatMap((scan) => scan.findings).sort(compareSecretFindings).slice(0, maxFindings);
  return toolJson("SecretGuard", {
    status: findings.length > 0 ? "secrets-detected" : "clean",
    scannedSources: scans.map((scan) => ({ source: scan.source, bytes: scan.text.length, findings: scan.findings.length })),
    findingCount: findings.length,
    highestSeverity: highestSecretSeverity(findings),
    findings: findings.map(publicSecretFinding),
    redactedText: returnRedactedText && text ? scanSecrets(text, path || "provided-text").redactedText : undefined,
    unavailable: { diff: diffUnavailable },
    notes: [
      "Findings are heuristic and may include false positives; do not paste unredacted matches into chat or logs.",
      "Shell and ReviewDiff tool outputs are automatically redacted with the same scanner.",
    ],
  });
}

async function writeFileTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const path = stringArg(args, "path");
  const text = stringArg(args, "text");
  const overwrite = booleanArg(args, "overwrite", false);
  const saveToDisk = booleanArg(args, "saveToDisk", true);
  const approval = createWriteApproval(path, text, overwrite, saveToDisk);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiFileWrite(
    path,
    text,
    overwrite,
    saveToDisk,
  );
  return toolResultFromFileOperation("Write", result);
}

async function strReplaceTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const path = stringArg(args, "path");
  const oldText = stringArg(args, "oldText");
  const newText = stringArg(args, "newText");
  const expectedReplacements = clamp(numberArg(args, "expectedReplacements", 1), 1, 1000);
  const saveToDisk = booleanArg(args, "saveToDisk", true);
  const approval = createStrReplaceApproval(path, oldText, newText, expectedReplacements, saveToDisk);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiFileStrReplace(
    path,
    oldText,
    newText,
    expectedReplacements,
    saveToDisk,
  );
  return toolResultFromFileOperation("StrReplace", result);
}

async function patchEngineTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const operations = patchOperationsArg(args);
  const saveToDisk = booleanArg(args, "saveToDisk", true);
  const dryRun = booleanArg(args, "dryRun", false);
  const approval = createPatchApproval(operations, saveToDisk, dryRun);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiFilePatch(operations, saveToDisk, dryRun);
  return toolResultFromFileOperation("PatchEngine", result);
}

async function checkpointTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const action = normalizeCheckpointAction(stringArg(args, "action", "list"));
  switch (action) {
    case "create":
      return createCheckpoint(args, input);
    case "list":
      return listCheckpoints(input);
    case "diff":
      return diffCheckpoint(args, input);
    case "delete":
      return deleteCheckpoint(args, input);
    case "restore":
      return restoreCheckpoint(args, input, ui);
    default:
      return toolJson("Checkpoint", { error: `Unsupported checkpoint action: ${stringArg(args, "action")}` });
  }
}

async function createCheckpoint(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const workspaceRoot = requireWorkspaceRoot(input);
  const maxFiles = clamp(numberArg(args, "maxFiles", defaultCheckpointMaxFiles), 1, checkpointMaxFilesLimit);
  const maxBytesPerFile = clamp(numberArg(args, "maxBytesPerFile", defaultCheckpointMaxBytesPerFile), 1_024, checkpointMaxBytesPerFileLimit);
  const paths = await checkpointTargetPaths(args, input, maxFiles);
  if (paths.length === 0) {
    return toolJson("Checkpoint", { status: "skipped", reason: "No file paths were available for checkpointing." });
  }

  const openByPath = openDocumentByAbsolutePath(input, workspaceRoot);
  const files = await Promise.all(paths.map((path) => snapshotCheckpointFile(path, workspaceRoot, openByPath, maxBytesPerFile)));
  const checkpoint: RuntimeCheckpoint = {
    id: `cp-${Date.now().toString(36)}-${crypto.randomUUID().slice(0, 8)}`,
    label: truncateText(stringArg(args, "label", "").trim() || `Checkpoint ${new Date().toLocaleString()}`, 120),
    workspaceRoot,
    createdAt: new Date().toISOString(),
    files,
    maxBytesPerFile,
  };
  const store = checkpointStore(workspaceRoot);
  store.unshift(checkpoint);
  store.splice(maxCheckpointsPerWorkspace);

  return toolJson("Checkpoint", {
    status: "created",
    checkpoint: checkpointSummary(checkpoint),
    files: files.map(compactCheckpointFile),
    warnings: checkpointWarnings(files),
  });
}

function listCheckpoints(input: AiChatSendInput): ToolResult {
  const workspaceRoot = requireWorkspaceRoot(input);
  const store = checkpointStore(workspaceRoot);
  return toolJson("Checkpoint", {
    workspaceRoot,
    count: store.length,
    checkpoints: store.map(checkpointSummary),
  });
}

async function diffCheckpoint(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const workspaceRoot = requireWorkspaceRoot(input);
  const checkpoint = selectCheckpoint(args, workspaceRoot);
  const pathFilter = checkpointPathFilter(args, workspaceRoot);
  const openByPath = openDocumentByAbsolutePath(input, workspaceRoot);
  const files = checkpoint.files.filter((file) => checkpointFileSelected(file, pathFilter));
  const diffs = await Promise.all(files.map((file) => diffCheckpointFile(file, workspaceRoot, openByPath, checkpoint.maxBytesPerFile)));
  return toolJson("Checkpoint", {
    status: "diffed",
    checkpoint: checkpointSummary(checkpoint),
    summary: checkpointDiffSummary(diffs),
    files: diffs,
  });
}

function deleteCheckpoint(args: UnknownRecord, input: AiChatSendInput): ToolResult {
  const workspaceRoot = requireWorkspaceRoot(input);
  const checkpoint = selectCheckpoint(args, workspaceRoot);
  const store = checkpointStore(workspaceRoot);
  const removedAt = store.findIndex((candidate) => candidate.id === checkpoint.id);
  if (removedAt >= 0) store.splice(removedAt, 1);
  return toolJson("Checkpoint", {
    status: "deleted",
    checkpoint: checkpointSummary(checkpoint),
    remaining: store.map(checkpointSummary),
  });
}

async function restoreCheckpoint(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const workspaceRoot = requireWorkspaceRoot(input);
  const checkpoint = selectCheckpoint(args, workspaceRoot);
  const pathFilter = checkpointPathFilter(args, workspaceRoot);
  const saveToDisk = booleanArg(args, "saveToDisk", true);
  const dryRun = booleanArg(args, "dryRun", false);
  const openByPath = openDocumentByAbsolutePath(input, workspaceRoot);
  const files = checkpoint.files.filter((file) => checkpointFileSelected(file, pathFilter));
  if (files.length === 0) {
    return toolJson("Checkpoint", { status: "skipped", checkpoint: checkpointSummary(checkpoint), reason: "No checkpoint files matched the requested paths." });
  }
  const blocked = files.filter((file) => file.truncated || file.error);
  if (blocked.length > 0) {
    return toolJson("Checkpoint", {
      status: "blocked",
      checkpoint: checkpointSummary(checkpoint),
      reason: "Restore refused because one or more snapshot files were truncated or failed to read.",
      blocked: blocked.map(compactCheckpointFile),
    });
  }

  const current = await Promise.all(files.map((file) => diffCheckpointFile(file, workspaceRoot, openByPath, checkpoint.maxBytesPerFile)));
  const operations = checkpointRestoreOperations(files, current);
  if (operations.length === 0) {
    return toolJson("Checkpoint", {
      status: "unchanged",
      checkpoint: checkpointSummary(checkpoint),
      summary: checkpointDiffSummary(current),
    });
  }

  const approval = createCheckpointRestoreApproval(checkpoint, operations, saveToDisk, dryRun);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiFilePatch(operations, saveToDisk, dryRun);
  return toolResultFromFileOperation("Checkpoint", result);
}

async function deleteFileTool(path: string, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const approval = createDeleteApproval(path);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiFileDelete(path);
  return toolResultFromFileOperation("Delete", result);
}

async function shellTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const command = stringArg(args, "command");
  const cwd = stringArg(args, "cwd", input.workspace?.root ?? "");
  const timeoutSecs = clamp(numberArg(args, "timeoutSecs", 120), 1, 600);
  const approval = createShellApproval(command, cwd, timeoutSecs);
  await requireToolApproval(input, ui, approval);
  const result = await luxCommands.aiShell(command, cwd || null, timeoutSecs);
  const stdoutScan = scanSecrets(result.stdout, "shell.stdout");
  const stderrScan = scanSecrets(result.stderr, "shell.stderr");
  const secretFindings = [...stdoutScan.findings, ...stderrScan.findings];
  return toolJson("Shell", {
    workspaceRoot: result.workspaceRoot,
    cwd: result.cwd,
    command: result.command,
    exitCode: result.exitCode,
    durationMs: result.durationMs,
    timedOut: result.timedOut,
    stdout: stdoutScan.redactedText,
    stderr: stderrScan.redactedText,
    secretGuard: {
      redacted: secretFindings.length > 0,
      findingCount: secretFindings.length,
      findings: secretFindings.slice(0, 20).map(publicSecretFinding),
    },
  });
}

async function requireToolApproval(input: AiChatSendInput, ui: ToolExecutionUi, approval: AiToolApprovalRequest) {
  throwIfAborted(input.abortSignal);
  if (input.preferences.toolApprovalMode === "full-access") {
    ui.setRunning({ ...approval, decision: "approved" });
    return;
  }
  ui.setApproval(approval);
  const decision = await input.onToolApproval(approval);
  throwIfAborted(input.abortSignal);
  const approvalState = { ...approval, decision };
  if (decision !== "approved") {
    ui.setApproval(approvalState);
    throw new ToolApprovalRejectedError(`${approval.tool} was rejected by the user.`);
  }
  ui.setRunning(approvalState);
}

function createWriteApproval(path: string, text: string, overwrite: boolean, saveToDisk: boolean): AiToolApprovalRequest {
  const lines = countLines(text);
  return {
    id: crypto.randomUUID(),
    tool: "Write",
    title: overwrite ? "Approve file rewrite" : "Approve file creation",
    path,
    summary: `${overwrite ? "Rewrite" : "Create"} ${path} with ${lines} line${lines === 1 ? "" : "s"}${saveToDisk ? " on disk" : " in editor only"}.`,
    preview: buildNumberedPreview(text, 80),
    risk: overwrite ? "modify" : "create",
    approveLabel: overwrite ? "Apply rewrite" : "Create file",
    rejectLabel: "Reject",
  };
}

function createStrReplaceApproval(path: string, oldText: string, newText: string, expectedReplacements: number, saveToDisk: boolean): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "StrReplace",
    title: "Approve exact text replacement",
    path,
    summary: `Replace ${expectedReplacements} occurrence${expectedReplacements === 1 ? "" : "s"} in ${path}${saveToDisk ? " on disk" : " in editor only"}.`,
    preview: buildReplacementPreview(oldText, newText),
    risk: "modify",
    approveLabel: "Apply edit",
    rejectLabel: "Reject",
  };
}

function createPatchApproval(operations: RuntimePatchOperation[], saveToDisk: boolean, dryRun: boolean): AiToolApprovalRequest {
  const counts = patchOperationCounts(operations);
  const paths = [...new Set(operations.map((operation) => operation.path))];
  return {
    id: crypto.randomUUID(),
    tool: "PatchEngine",
    title: dryRun ? "Approve patch dry-run" : "Approve multi-file patch",
    path: paths.length === 1 ? paths[0] : `${paths.length} paths`,
    summary: `${dryRun ? "Validate" : "Apply"} ${operations.length} operation${operations.length === 1 ? "" : "s"}: ${counts.create} create, ${counts.rewrite} rewrite, ${counts.replace} replace, ${counts.delete} delete${saveToDisk && !dryRun ? " on disk" : " in memory/validation only"}.`,
    preview: buildPatchPreview(operations),
    risk: counts.delete > 0 ? "delete" : counts.create > 0 && counts.rewrite === 0 && counts.replace === 0 ? "create" : "modify",
    approveLabel: dryRun ? "Run dry-run" : "Apply patch",
    rejectLabel: "Reject",
  };
}

function normalizeCheckpointAction(value: string): CheckpointAction | "" {
  const normalized = value.trim().toLowerCase().replace(/[-_\s]+/g, "");
  if (normalized === "create" || normalized === "snapshot" || normalized === "save") return "create";
  if (normalized === "list" || normalized === "ls") return "list";
  if (normalized === "diff" || normalized === "compare") return "diff";
  if (normalized === "delete" || normalized === "remove" || normalized === "drop") return "delete";
  if (normalized === "restore" || normalized === "rollback" || normalized === "revert") return "restore";
  return "";
}

function requireWorkspaceRoot(input: AiChatSendInput) {
  const root = normalizePathSlashes(input.workspace?.root ?? "").replace(/\/+$/, "");
  if (!root) throw new Error("Checkpoint requires an open workspace.");
  return root;
}

async function checkpointTargetPaths(args: UnknownRecord, input: AiChatSendInput, maxFiles: number) {
  const workspaceRoot = requireWorkspaceRoot(input);
  const selected = new Map<string, string>();
  let hasExplicitPaths = false;
  const addPath = (path: string | null | undefined) => {
    if (!path || !path.trim()) return;
    const resolved = resolveWorkspacePath(path, workspaceRoot);
    if (!isPathInsideWorkspace(resolved, workspaceRoot)) return;
    const normalized = normalizePathSlashes(resolved);
    selected.set(normalized.toLowerCase(), normalized);
  };

  for (const path of stringArrayArg(args, "paths")) {
    hasExplicitPaths = true;
    addPath(path);
  }
  if (typeof args.path === "string" && args.path.trim()) {
    hasExplicitPaths = true;
    addPath(args.path);
  }

  if (hasExplicitPaths) return Array.from(selected.values()).slice(0, maxFiles);

  const includeOpenDocuments = booleanArg(args, "includeOpenDocuments", true);
  if (includeOpenDocuments) {
    addPath(input.activeDocument?.path);
    for (const document of input.openDocuments) addPath(document.path);
  }

  if (booleanArg(args, "includeGitChanges", true)) {
    try {
      const [status, diff] = await Promise.all([luxCommands.gitStatus(), luxCommands.gitDiff()]);
      for (const file of mergeDiffAndStatusFiles(diff.files, status.files)) {
        addPath(file.path);
        addPath(file.old_path);
      }
    } catch {
      // Git data is opportunistic. Open and explicit paths still make the checkpoint useful.
    }
  }

  return Array.from(selected.values()).slice(0, maxFiles);
}

function openDocumentByAbsolutePath(input: AiChatSendInput, workspaceRoot: string) {
  const byPath = new Map<string, DocumentSnapshot>();
  const add = (document: DocumentSnapshot | null | undefined) => {
    if (!document?.path) return;
    const path = resolveWorkspacePath(document.path, workspaceRoot);
    if (!isPathInsideWorkspace(path, workspaceRoot)) return;
    byPath.set(normalizePathSlashes(path).toLowerCase(), document);
  };
  for (const document of input.openDocuments) add(document);
  add(input.activeDocument);
  return byPath;
}

async function snapshotCheckpointFile(path: string, workspaceRoot: string, openByPath: Map<string, DocumentSnapshot>, maxBytesPerFile: number): Promise<CheckpointFileSnapshot> {
  const normalized = normalizePathSlashes(resolveWorkspacePath(path, workspaceRoot));
  const relativePath = createRelatedFileDescriptor({ path: normalized }, workspaceRoot).relativePath;
  const openDocument = openByPath.get(normalized.toLowerCase());
  if (openDocument) {
    const text = openDocument.text.slice(0, maxBytesPerFile);
    return {
      path: normalized,
      relativePath,
      existed: !openDocument.is_untitled,
      text,
      size: openDocument.text.length,
      truncated: openDocument.text.length > maxBytesPerFile,
      source: "editor",
    };
  }

  try {
    const response = await luxCommands.fsReadText(normalized, maxBytesPerFile);
    return {
      path: normalizePathSlashes(response.path),
      relativePath,
      existed: true,
      text: response.text,
      size: response.size,
      truncated: response.truncated,
      source: "disk",
    };
  } catch (error) {
    return {
      path: normalized,
      relativePath,
      existed: false,
      text: "",
      size: 0,
      truncated: false,
      source: "missing",
      error: readErrorMessage(error),
    };
  }
}

function checkpointStore(workspaceRoot: string) {
  const key = normalizePathSlashes(workspaceRoot).replace(/\/+$/, "").toLowerCase();
  const existing = checkpointStoreByWorkspace.get(key);
  if (existing) return existing;
  const next: RuntimeCheckpoint[] = [];
  checkpointStoreByWorkspace.set(key, next);
  return next;
}

function checkpointSummary(checkpoint: RuntimeCheckpoint) {
  const files = checkpoint.files;
  return {
    id: checkpoint.id,
    label: checkpoint.label,
    workspaceRoot: checkpoint.workspaceRoot,
    createdAt: checkpoint.createdAt,
    fileCount: files.length,
    restorableFileCount: files.filter((file) => !file.truncated && !file.error).length,
    truncatedFileCount: files.filter((file) => file.truncated).length,
    errorFileCount: files.filter((file) => file.error).length,
    maxBytesPerFile: checkpoint.maxBytesPerFile,
  };
}

function compactCheckpointFile(file: CheckpointFileSnapshot) {
  return {
    path: file.path,
    relativePath: file.relativePath,
    existed: file.existed,
    source: file.source,
    size: file.size,
    lines: file.existed ? countLines(file.text) : 0,
    truncated: file.truncated,
    error: file.error,
  };
}

function checkpointWarnings(files: CheckpointFileSnapshot[]) {
  const warnings: string[] = [];
  const truncated = files.filter((file) => file.truncated);
  const errors = files.filter((file) => file.error);
  const missing = files.filter((file) => !file.existed && file.source === "missing");
  if (truncated.length > 0) warnings.push(`${truncated.length} file${truncated.length === 1 ? "" : "s"} exceeded the snapshot byte limit and cannot be restored.`);
  if (errors.length > 0) warnings.push(`${errors.length} file${errors.length === 1 ? "" : "s"} could not be read.`);
  if (missing.length > 0) warnings.push(`${missing.length} missing path${missing.length === 1 ? "" : "s"} recorded so restore can delete newly created files if needed.`);
  return warnings;
}

function selectCheckpoint(args: UnknownRecord, workspaceRoot: string) {
  const store = checkpointStore(workspaceRoot);
  if (store.length === 0) throw new Error("No checkpoints exist for this workspace.");
  const id = stringArg(args, "id", "").trim();
  if (!id) return store[0];
  const checkpoint = store.find((candidate) => candidate.id === id);
  if (!checkpoint) throw new Error(`Checkpoint not found: ${id}`);
  return checkpoint;
}

function checkpointPathFilter(args: UnknownRecord, workspaceRoot: string) {
  const paths = stringArrayArg(args, "paths");
  if (typeof args.path === "string" && args.path.trim()) paths.push(args.path);
  return paths
    .map((path) => resolveWorkspacePath(path, workspaceRoot))
    .filter((path) => isPathInsideWorkspace(path, workspaceRoot))
    .map((path) => normalizePathSlashes(path).toLowerCase());
}

function checkpointFileSelected(file: CheckpointFileSnapshot, pathFilter: string[]) {
  if (pathFilter.length === 0) return true;
  const lower = normalizePathSlashes(file.path).toLowerCase();
  return pathFilter.some((path) => lower === path || lower.endsWith(`/${path}`));
}

async function diffCheckpointFile(file: CheckpointFileSnapshot, workspaceRoot: string, openByPath: Map<string, DocumentSnapshot>, maxBytesPerFile: number): Promise<CheckpointFileDiff> {
  const current = await readCheckpointCurrentFile(file.path, workspaceRoot, openByPath, maxBytesPerFile);
  const status = checkpointDiffStatus(file, current);
  const lineDelta = current.existed && file.existed ? countLines(current.text) - countLines(file.text) : null;
  return {
    path: file.path,
    relativePath: file.relativePath,
    status,
    existedAtCheckpoint: file.existed,
    currentExists: current.existed,
    diskExists: current.diskExists,
    snapshotSource: file.source,
    currentSource: current.source,
    snapshotSize: file.size,
    currentSize: current.size,
    snapshotTruncated: file.truncated,
    currentTruncated: current.truncated,
    lineDelta,
    beforePreview: file.existed ? truncateText(buildNumberedPreview(file.text, 24), 2_400) : undefined,
    currentPreview: current.existed ? truncateText(buildNumberedPreview(current.text, 24), 2_400) : undefined,
    error: file.error ?? current.error,
  };
}

async function readCheckpointCurrentFile(path: string, workspaceRoot: string, openByPath: Map<string, DocumentSnapshot>, maxBytesPerFile: number): Promise<CheckpointCurrentFile> {
  const normalized = normalizePathSlashes(resolveWorkspacePath(path, workspaceRoot));
  const openDocument = openByPath.get(normalized.toLowerCase());
  if (openDocument) {
    const diskExists = await checkpointDiskExists(normalized, maxBytesPerFile);
    return {
      existed: true,
      diskExists,
      text: openDocument.text.slice(0, maxBytesPerFile),
      size: openDocument.text.length,
      truncated: openDocument.text.length > maxBytesPerFile,
      source: "editor",
    };
  }

  try {
    const response = await luxCommands.fsReadText(normalized, maxBytesPerFile);
    return {
      existed: true,
      diskExists: true,
      text: response.text,
      size: response.size,
      truncated: response.truncated,
      source: "disk",
    };
  } catch (error) {
    return {
      existed: false,
      diskExists: false,
      text: "",
      size: null,
      truncated: false,
      source: "missing",
      error: readErrorMessage(error),
    };
  }
}

async function checkpointDiskExists(path: string, maxBytesPerFile: number) {
  try {
    await luxCommands.fsReadText(path, Math.min(maxBytesPerFile, 1_024));
    return true;
  } catch {
    return false;
  }
}

function checkpointDiffStatus(file: CheckpointFileSnapshot, current: CheckpointCurrentFile): CheckpointFileDiff["status"] {
  if (file.error || current.error && current.existed) return "error";
  if (file.truncated || current.truncated) return "truncated";
  if (file.existed && !current.existed) return "missing";
  if (!file.existed && current.existed) return "created";
  if (!file.existed && !current.existed) return "unchanged";
  return file.text === current.text ? "unchanged" : "modified";
}

function checkpointDiffSummary(diffs: CheckpointFileDiff[]) {
  return {
    total: diffs.length,
    unchanged: diffs.filter((diff) => diff.status === "unchanged").length,
    modified: diffs.filter((diff) => diff.status === "modified").length,
    missing: diffs.filter((diff) => diff.status === "missing").length,
    created: diffs.filter((diff) => diff.status === "created").length,
    truncated: diffs.filter((diff) => diff.status === "truncated").length,
    errored: diffs.filter((diff) => diff.status === "error").length,
  };
}

function checkpointRestoreOperations(files: CheckpointFileSnapshot[], current: CheckpointFileDiff[]): RuntimePatchOperation[] {
  const currentByPath = new Map(current.map((file) => [normalizePathSlashes(file.path).toLowerCase(), file]));
  const operations: RuntimePatchOperation[] = [];

  for (const file of files) {
    const diff = currentByPath.get(normalizePathSlashes(file.path).toLowerCase());
    if (!diff || diff.status === "unchanged") continue;
    if (file.truncated || file.error || diff.currentTruncated || diff.status === "error" || diff.status === "truncated") continue;
    if (file.existed) {
      operations.push({
        action: diff.diskExists ? "rewrite" : "create",
        path: file.path,
        text: file.text,
        overwrite: diff.diskExists ? undefined : false,
      });
    } else if (diff.diskExists) {
      operations.push({ action: "delete", path: file.path });
    }
  }

  return operations;
}

function createCheckpointRestoreApproval(checkpoint: RuntimeCheckpoint, operations: RuntimePatchOperation[], saveToDisk: boolean, dryRun: boolean): AiToolApprovalRequest {
  const counts = patchOperationCounts(operations);
  const paths = [...new Set(operations.map((operation) => operation.path))];
  return {
    id: crypto.randomUUID(),
    tool: "Checkpoint",
    title: dryRun ? "Approve checkpoint restore dry-run" : "Approve checkpoint restore",
    path: paths.length === 1 ? paths[0] : `${paths.length} paths`,
    summary: `${dryRun ? "Validate restore from" : "Restore"} checkpoint ${checkpoint.id} (${checkpoint.label}) with ${operations.length} operation${operations.length === 1 ? "" : "s"}: ${counts.create} create, ${counts.rewrite} rewrite, ${counts.replace} replace, ${counts.delete} delete${saveToDisk && !dryRun ? " on disk" : " in memory/validation only"}.`,
    preview: buildPatchPreview(operations),
    risk: counts.delete > 0 ? "delete" : counts.create > 0 && counts.rewrite === 0 && counts.replace === 0 ? "create" : "modify",
    approveLabel: dryRun ? "Run dry-run" : "Restore checkpoint",
    rejectLabel: "Keep current files",
  };
}

function createDeleteApproval(path: string): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "Delete",
    title: "Approve file deletion",
    path,
    summary: `Delete ${path} from the workspace. This cannot be undone by Lux automatically.`,
    preview: `- ${path}`,
    risk: "delete",
    approveLabel: "Delete",
    rejectLabel: "Keep file",
  };
}

function createShellApproval(command: string, cwd: string, timeoutSecs: number): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "Shell",
    title: "Approve shell command",
    path: cwd || ".",
    summary: `Run a non-interactive shell command in ${cwd || "the workspace"} with a ${timeoutSecs}s timeout.`,
    preview: command,
    risk: "execute",
    approveLabel: "Run command",
    rejectLabel: "Reject",
  };
}

function buildReplacementPreview(oldText: string, newText: string) {
  const before = buildNumberedPreview(oldText, 40)
    .split("\n")
    .map((line) => `- ${line}`)
    .join("\n");
  const after = buildNumberedPreview(newText, 40)
    .split("\n")
    .map((line) => `+ ${line}`)
    .join("\n");
  return `${before}\n${after}`;
}

function patchOperationsArg(args: UnknownRecord) {
  const raw = args.operations;
  if (!Array.isArray(raw)) return [];
  return raw.filter(isRecord).map((operation) => {
    const action = normalizePatchAction(operation.action ?? operation.kind ?? operation.operation);
    const path = stringArg(operation, "path");
    const text = typeof operation.text === "string" ? operation.text : undefined;
    const oldText = typeof operation.oldText === "string" ? operation.oldText : typeof operation.old_text === "string" ? operation.old_text : undefined;
    const newText = typeof operation.newText === "string" ? operation.newText : typeof operation.new_text === "string" ? operation.new_text : undefined;
    const expectedReplacements = optionalPositiveNumberArg({ value: operation.expectedReplacements ?? operation.expected_replacements }, "value") ?? undefined;
    const overwrite = typeof operation.overwrite === "boolean" ? operation.overwrite : undefined;
    return { action, path, text, oldText, newText, expectedReplacements, overwrite };
  }).filter((operation) => operation.action && operation.path).slice(0, 80);
}

function normalizePatchAction(value: unknown) {
  if (typeof value !== "string") return "";
  const normalized = value.trim().toLowerCase().replace(/[-_\s]+/g, "");
  if (normalized === "create") return "create";
  if (normalized === "write" || normalized === "rewrite" || normalized === "replacefile") return "rewrite";
  if (normalized === "replace" || normalized === "strreplace") return "replace";
  if (normalized === "delete" || normalized === "remove") return "delete";
  return value.trim();
}

function patchOperationCounts(operations: RuntimePatchOperation[]) {
  return operations.reduce((counts, operation) => {
    if (operation.action === "create") counts.create += 1;
    else if (operation.action === "rewrite") counts.rewrite += 1;
    else if (operation.action === "replace") counts.replace += 1;
    else if (operation.action === "delete") counts.delete += 1;
    return counts;
  }, { create: 0, rewrite: 0, replace: 0, delete: 0 });
}

function buildPatchPreview(operations: RuntimePatchOperation[]) {
  const lines: string[] = [];
  for (const [index, operation] of operations.slice(0, 20).entries()) {
    const label = `${index + 1}. ${operation.action} ${operation.path}`;
    if (operation.action === "replace") {
      lines.push(`${label} (${operation.expectedReplacements ?? 1} expected)`);
      lines.push(truncateText(buildReplacementPreview(operation.oldText ?? "", operation.newText ?? ""), 1_600));
    } else if (operation.action === "create" || operation.action === "rewrite") {
      lines.push(`${label} (${countLines(operation.text ?? "")} lines${operation.overwrite ? ", overwrite allowed" : ""})`);
      lines.push(truncateText(buildNumberedPreview(operation.text ?? "", 24), 1_600));
    } else {
      lines.push(label);
    }
  }
  if (operations.length > 20) lines.push(`... ${operations.length - 20} more operation${operations.length - 20 === 1 ? "" : "s"}`);
  return truncateText(lines.join("\n"), 12_000);
}

function buildNumberedPreview(text: string, maxLines: number) {
  const lines = text.split(/\r?\n/);
  const visible = lines.slice(0, maxLines).map((line, index) => `${String(index + 1).padStart(3, " ")} | ${line}`);
  if (lines.length > maxLines) visible.push(`... ${lines.length - maxLines} more line${lines.length - maxLines === 1 ? "" : "s"}`);
  return visible.join("\n");
}

function countLines(text: string) {
  if (!text) return 0;
  return text.split(/\r?\n/).length;
}

class ToolApprovalRejectedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ToolApprovalRejectedError";
  }
}

async function grepTool(args: UnknownRecord): Promise<ToolResult> {
  const query = stringArg(args, "query");
  const maxResults = clamp(numberArg(args, "maxResults", 50), 1, 200);
  const response = await luxCommands.searchQuery(query, {
    case_sensitive: booleanArg(args, "caseSensitive", false),
    whole_word: false,
    use_regex: booleanArg(args, "useRegex", false),
    include_hidden: false,
    include_globs: stringArrayArg(args, "includeGlobs"),
    exclude_globs: [],
    max_results: maxResults,
  });
  return toolJson("Grep", {
    query: response.query,
    truncated: response.truncated,
    elapsedMs: response.elapsed_ms,
    hits: response.hits.map((hit) => ({
      path: hit.path,
      line: hit.line,
      column: hit.column,
      preview: hit.preview,
    })),
  });
}

async function diagnosticsContext(maxResults: number): Promise<ToolResult> {
  const diagnostics = await luxCommands.diagnosticsSnapshot();
  return toolJson("DiagnosticsContext", {
    count: diagnostics.length,
    diagnostics: diagnostics.slice(0, clamp(maxResults, 1, 500)).map((diagnostic) => ({
      path: diagnostic.path,
      line: diagnostic.line,
      column: diagnostic.column,
      severity: diagnostic.severity,
      source: diagnostic.source,
      message: diagnostic.message,
    })),
  });
}

async function symbolContext(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message);
  const path = stringArg(args, "path", input.activeDocument?.path ?? "");
  const line = optionalPositiveNumberArg(args, "line");
  const column = optionalPositiveNumberArg(args, "column");
  const maxResults = clamp(numberArg(args, "maxResults", 80), 1, 300);
  const response = await luxCommands.aiSymbolContext(
    query.trim() || null,
    path.trim() || null,
    line,
    column,
    maxResults,
  );
  return toolJson("SymbolContext", {
    workspaceRoot: response.workspaceRoot,
    query: response.query,
    path: response.path,
    position: response.position,
    workspaceSymbols: response.workspaceSymbols.map((symbol) => ({
      name: symbol.name,
      kind: symbol.kind,
      containerName: symbol.container_name,
      location: compactLocation(symbol.location),
    })),
    documentSymbols: response.documentSymbols.map(compactDocumentSymbol),
    hover: response.hover ? {
      contents: response.hover.contents,
      range: response.hover.range,
    } : null,
    definitions: response.definitions.map(compactLocation),
    references: response.references.map(compactLocation),
    signatureHelp: response.signatureHelp ? {
      activeSignature: response.signatureHelp.active_signature,
      activeParameter: response.signatureHelp.active_parameter,
      signatures: response.signatureHelp.signatures.slice(0, 12).map((signature) => ({
        label: signature.label,
        documentation: signature.documentation,
        parameters: signature.parameters.map((parameter) => ({
          label: parameter.label,
          documentation: parameter.documentation,
        })),
      })),
    } : null,
    diagnostics: response.diagnostics
      .filter((diagnostic) => !response.path || normalizePathForCompare(diagnostic.path) === normalizePathForCompare(response.path))
      .slice(0, 80)
      .map((diagnostic) => ({
        path: diagnostic.path,
        line: diagnostic.line,
        column: diagnostic.column,
        severity: diagnostic.severity,
        source: diagnostic.source,
        message: diagnostic.message,
      })),
    notes: response.notes,
  });
}

async function relatedFiles(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const path = stringArg(args, "path", input.activeDocument?.path ?? "");
  const query = stringArg(args, "query", input.message);
  const maxResults = clamp(numberArg(args, "maxResults", 40), 1, 120);
  const scanLimit = clamp(input.preferences.maxIndexedFiles, 500, 20_000);
  const entries = await luxCommands.fsListFiles(scanLimit);
  const workspaceRoot = input.workspace?.root ?? "";
  const targetPath = path.trim() ? resolveWorkspacePath(path, workspaceRoot) : "";
  const target = targetPath ? createRelatedFileDescriptor({ path: targetPath }, workspaceRoot) : null;
  const queryTokens = tokenizeRelatedQuery(query);
  const matches = new Map<string, RelatedFileMatch>();

  for (const entry of entries) {
    if (entry.kind !== "file" || isLowSignalRelatedPath(entry.path)) continue;
    const descriptor = createRelatedFileDescriptor(entry, workspaceRoot);
    if (target && descriptor.lower === target.lower) continue;

    const match = scoreRelatedFile(descriptor, target, queryTokens);
    if (match.score <= 0) continue;
    matches.set(descriptor.lower, match);
  }

  const related = Array.from(matches.values())
    .sort((left, right) => right.score - left.score || left.descriptor.relativeLower.localeCompare(right.descriptor.relativeLower))
    .slice(0, maxResults);

  return toolJson("RelatedFiles", {
    workspaceRoot: input.workspace?.root ?? null,
    target: target ? {
      path: target.path,
      relativePath: target.relativePath,
      basename: target.basename,
      familyStem: target.familyStem,
    } : null,
    query,
    scanned: entries.filter((entry) => entry.kind === "file").length,
    count: related.length,
    files: related.map((match) => ({
      path: match.descriptor.path,
      relativePath: match.descriptor.relativePath,
      relations: Array.from(match.relations).sort(),
      score: match.score,
      queryHits: match.queryHits,
      size: match.descriptor.entry?.size ?? null,
      modifiedAt: match.descriptor.entry?.modified_at ?? null,
    })),
  });
}

async function impactAnalysis(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message);
  const path = stringArg(args, "path", input.activeDocument?.path ?? "");
  const maxResults = clamp(numberArg(args, "maxResults", 32), 1, 100);
  const [relatedResult, diagnosticsResult, symbolsResult, rulesResult, docsResult] = await Promise.allSettled([
    relatedFiles({ path, query, maxResults }, input),
    diagnosticsContext(80),
    symbolContext({ query, path, maxResults: 80 }, input),
    rulesContext({ query, maxFiles: 6 }, input),
    docsContext({ query, maxFiles: 6 }, input),
  ]);
  const related = parseToolContent(relatedResult);
  const diagnostics = parseToolContent(diagnosticsResult);
  const symbols = parseToolContent(symbolsResult);
  const relatedFilesList = Array.isArray(related?.files) ? related.files.filter(isRecord) : [];
  const diagnosticsList = Array.isArray(diagnostics?.diagnostics) ? diagnostics.diagnostics.filter(isRecord) : [];
  const symbolFiles = collectSymbolFiles(symbols).slice(0, maxResults);
  const riskSignals = buildImpactRiskSignals(relatedFilesList, diagnosticsList, symbolFiles);
  const validation = buildImpactValidation(relatedFilesList, query);

  return toolJson("ImpactAnalysis", {
    workspaceRoot: input.workspace?.root ?? null,
    target: path || input.activeDocument?.path || input.activeDocument?.title || null,
    query,
    riskLevel: riskSignals.some((signal) => signal.level === "high") ? "high" : riskSignals.some((signal) => signal.level === "medium") ? "medium" : "low",
    affectedFiles: relatedFilesList.slice(0, maxResults).map((file) => ({
      path: file.path,
      relativePath: file.relativePath,
      relations: file.relations,
      score: file.score,
    })),
    symbolFiles,
    diagnostics: diagnosticsList.slice(0, 24),
    riskSignals,
    validation,
    rules: parseToolContent(rulesResult),
    docs: parseToolContent(docsResult),
  });
}

async function reviewDiff(args: UnknownRecord): Promise<ToolResult> {
  const includePatch = booleanArg(args, "includePatch", true);
  const maxFindings = clamp(numberArg(args, "maxFindings", 12), 1, 40);
  const [statusResult, diffResult, diagnosticsResult] = await Promise.allSettled([
    luxCommands.gitStatus(),
    luxCommands.gitDiff(),
    luxCommands.diagnosticsSnapshot(),
  ]);
  const status = statusResult.status === "fulfilled" ? statusResult.value : null;
  const diff = diffResult.status === "fulfilled" ? diffResult.value : null;
  const diagnostics = diagnosticsResult.status === "fulfilled" ? diagnosticsResult.value : [];
  const changedFiles = mergeDiffAndStatusFiles(diff?.files ?? [], status?.files ?? []);
  const secretScan = scanSecrets(diff?.patch ?? "", "git.diff");
  const findings = buildDiffReviewFindings(changedFiles, diagnostics, secretScan.findings).slice(0, maxFindings);

  return toolJson("ReviewDiff", {
    branch: status?.branch ?? null,
    ahead: status?.ahead ?? 0,
    behind: status?.behind ?? 0,
    changedFiles: changedFiles.map((file) => ({
      path: normalizePathSlashes(file.path),
      oldPath: file.old_path ? normalizePathSlashes(file.old_path) : null,
      status: file.status,
      additions: file.additions,
      deletions: file.deletions,
      binary: file.binary,
    })),
    totals: {
      files: changedFiles.length,
      additions: diff?.additions ?? 0,
      deletions: diff?.deletions ?? 0,
      diagnostics: diagnostics.length,
    },
    findings,
    secretGuard: {
      redacted: secretScan.findings.length > 0,
      findingCount: secretScan.findings.length,
      findings: secretScan.findings.slice(0, 20).map(publicSecretFinding),
    },
    recommendedChecks: buildReviewDiffChecks(changedFiles),
    patch: includePatch ? secretScan.redactedText : undefined,
    truncated: diff?.truncated ?? false,
    unavailable: {
      status: statusResult.status === "rejected" ? readErrorMessage(statusResult.reason) : null,
      diff: diffResult.status === "rejected" ? readErrorMessage(diffResult.reason) : null,
      diagnostics: diagnosticsResult.status === "rejected" ? readErrorMessage(diagnosticsResult.reason) : null,
    },
  });
}

function scoreRelatedFile(descriptor: RelatedFileDescriptor, target: RelatedFileDescriptor | null, queryTokens: string[]): RelatedFileMatch {
  const relations = new Set<RelatedFileRelation>();
  const queryHits: string[] = [];
  let score = 0;

  if (target) {
    const sameDirectory = descriptor.dir === target.dir;
    const sameFamily = Boolean(descriptor.familyStemLower && target.familyStemLower) && descriptor.familyStemLower === target.familyStemLower;
    const siblingFamily = Boolean(descriptor.familyStemLower && target.familyStemLower) && (
      descriptor.stemLower === target.familyStemLower ||
      descriptor.familyStemLower.includes(target.familyStemLower) ||
      target.familyStemLower.includes(descriptor.familyStemLower)
    );

    if (sameDirectory) {
      relations.add("same-directory");
      score += 16;
    }
    if (sameFamily) score += 42;
    else if (sameDirectory && siblingFamily) score += 24;

    const directoryDistance = relatedDirectoryDistance(target.relativeDir, descriptor.relativeDir);
    score += Math.max(0, 18 - directoryDistance * 4);

    if (sameDirectory && isBarrelFile(descriptor)) {
      relations.add("barrel");
      score += 25;
    }
    if (sameDirectory && target.familyStemLower && descriptor.stemLower.includes(target.familyStemLower) && descriptor.familyStemLower !== target.familyStemLower) {
      relations.add("nearby-name");
      score += 12;
    }
    if (sameFamily || sameDirectory || directoryDistance <= 2) {
      const kindScore = addRelatedKindRelations(descriptor, relations);
      score += kindScore;
    }
    if (sameFamily && isSourceCounterpart(descriptor, target)) {
      relations.add("nearby-name");
      score += 18;
    }
  } else {
    const kindScore = addRelatedKindRelations(descriptor, relations);
    score += kindScore > 0 ? Math.min(kindScore, 20) : 0;
    if (isImportantProjectFile(descriptor)) score += 35;
  }

  for (const token of queryTokens) {
    if (descriptor.relativeLower.includes(token)) {
      queryHits.push(token);
      relations.add("query-match");
      score += token.length >= 6 ? 18 : 12;
      if (descriptor.basenameLower.includes(token)) score += 10;
    }
  }

  if (isImportantProjectFile(descriptor)) {
    addImportantFileRelation(descriptor, relations);
    score += target ? 14 : 30;
  }
  if (target && queryHits.length === 0 && relations.size === 0) {
    return { descriptor, score: 0, relations, queryHits };
  }
  if (descriptor.relativeLower.includes("/src/") || descriptor.relativeLower.startsWith("src/")) score += 4;
  if (descriptor.relativeLower.includes("/test") || descriptor.relativeLower.includes("/spec")) score += 4;
  if (descriptor.basenameLower.endsWith(".lock")) score -= 20;

  return { descriptor, score, relations, queryHits };
}

function isRulesContextPath(path: string, workspaceRoot: string) {
  const file = createRelatedFileDescriptor({ path }, workspaceRoot);
  const lower = file.relativeLower;
  return rulesContextFileNames.has(file.basenameLower) || lower.startsWith(".cursor/rules/") || lower.includes("/.cursor/rules/") || lower.includes("/rules/") && /\.(md|mdx|txt)$/.test(lower);
}

function isDocsContextPath(path: string, workspaceRoot: string) {
  const file = createRelatedFileDescriptor({ path }, workspaceRoot);
  return docsContextFilePattern.test(file.relativeLower) && !isLowSignalRelatedPath(path);
}

function isMemoryContextPath(path: string, workspaceRoot: string) {
  const file = createRelatedFileDescriptor({ path }, workspaceRoot);
  const lower = file.relativeLower;
  if (isLowSignalRelatedPath(path)) return false;
  const isKnownMemoryName = memoryContextFileNames.has(file.basenameLower) || /^(agents\.md|claude\.md|codex\.md|\.cursorrules)$/.test(file.basenameLower);
  if (!/\.(md|mdx|txt|json|ya?ml|toml)$/.test(file.extension.toLowerCase()) && !isKnownExtensionlessProjectFile(lower) && !isKnownMemoryName) return false;
  return isKnownMemoryName ||
    /(^|\/)(adr|adrs|decisions?|memory|notes|roadmap|todos?|\.codex|\.cursor)(\/|$)/.test(lower) ||
    /(^|\/)(agents\.md|claude\.md|codex\.md|\.cursorrules)$/.test(lower);
}

function scoreRulesFile(file: RelatedFileDescriptor, queryTokens: string[]) {
  let score = 0;
  if (file.basenameLower === "agents.md") score += 120;
  if (file.basenameLower === ".cursorrules") score += 115;
  if (file.basenameLower === "claude.md") score += 100;
  if (file.relativeLower.startsWith(".cursor/rules/")) score += 90;
  if (file.relativeDir === "" || file.relativeDir === ".") score += 45;
  if (file.relativeLower.includes("rules")) score += 20;
  for (const token of queryTokens) {
    if (file.relativeLower.includes(token)) score += token.length >= 6 ? 18 : 10;
  }
  return score;
}

function scoreMemoryFile(file: RelatedFileDescriptor, queryTokens: string[]) {
  let score = 0;
  if (memoryContextFileNames.has(file.basenameLower)) score += 110;
  if (/adr|decision/.test(file.relativeLower)) score += 90;
  if (/memory|preference|notes/.test(file.relativeLower)) score += 85;
  if (/roadmap|todo/.test(file.relativeLower)) score += 60;
  if (/agents\.md|claude\.md|codex\.md|\.cursorrules/.test(file.basenameLower)) score += 72;
  if (file.relativeLower.startsWith(".codex/") || file.relativeLower.startsWith(".cursor/")) score += 58;
  if (file.relativeDir === "" || file.relativeDir === ".") score += 22;
  for (const token of queryTokens) {
    if (file.relativeLower.includes(token)) score += token.length >= 6 ? 22 : 12;
  }
  return score;
}

function scoreDocsFile(file: RelatedFileDescriptor, queryTokens: string[]) {
  let score = 0;
  if (/readme/i.test(file.basenameLower)) score += 80;
  if (/(package\.json|cargo\.toml|pyproject\.toml|go\.mod)$/.test(file.basenameLower)) score += 70;
  if (file.relativeLower.startsWith("docs/") || file.relativeLower.includes("/docs/")) score += 45;
  if (/architecture|contributing|changelog/i.test(file.basenameLower)) score += 30;
  if (file.relativeDir === "" || file.relativeDir === ".") score += 20;
  for (const token of queryTokens) {
    if (file.relativeLower.includes(token)) score += token.length >= 6 ? 22 : 12;
  }
  return score;
}

function passesSemanticPathFilter(path: string, pathFilter: string) {
  return !pathFilter || normalizePathSlashes(path).toLowerCase().includes(pathFilter);
}

function scoreSemanticSymbol(symbol: LspWorkspaceSymbol, query: string, queryTokens: string[], path: string, workspaceRoot: string) {
  const file = createRelatedFileDescriptor({ path }, workspaceRoot);
  const name = symbol.name.toLowerCase();
  const container = symbol.container_name?.toLowerCase() ?? "";
  const normalizedQuery = query.toLowerCase();
  let score = 80 + scorePath(file.relativePath);
  if (name === normalizedQuery) score += 90;
  else if (name.includes(normalizedQuery)) score += 55;
  if (container.includes(normalizedQuery)) score += 25;
  for (const token of queryTokens) {
    if (name.includes(token)) score += token.length >= 6 ? 24 : 16;
    if (container.includes(token)) score += 12;
    if (file.relativeLower.includes(token)) score += 10;
  }
  if (isTestFile(file)) score -= 10;
  if (isImportantProjectFile(file)) score += 8;
  return score;
}

function scoreSemanticTextHit(path: string, preview: string, matchText: string, queryTokens: string[], workspaceRoot: string) {
  const file = createRelatedFileDescriptor({ path }, workspaceRoot);
  const haystack = `${file.relativeLower}\n${preview}\n${matchText}`.toLowerCase();
  let score = 50 + scorePath(file.relativePath);
  for (const token of queryTokens) {
    if (haystack.includes(token)) score += token.length >= 6 ? 18 : 11;
    if (file.basenameLower.includes(token)) score += 10;
  }
  if (/function|class|interface|type|struct|enum|impl|export|const|async/i.test(preview)) score += 12;
  if (isTestFile(file)) score -= 8;
  if (isImportantProjectFile(file)) score += 6;
  return score;
}

function scoreSemanticFile(file: RelatedFileDescriptor, queryTokens: string[]) {
  let score = 0;
  for (const token of queryTokens) {
    if (file.basenameLower.includes(token)) score += token.length >= 6 ? 34 : 22;
    if (file.familyStemLower.includes(token)) score += 16;
    if (file.relativeLower.includes(token)) score += 10;
  }
  if (score === 0) return 0;
  score += Math.min(scorePath(file.relativePath), 30);
  if (isImportantProjectFile(file)) score += 16;
  if (isTestFile(file)) score -= 6;
  return score;
}

function upsertSemanticResult(results: Map<string, SemanticSearchResult>, result: SemanticSearchResult) {
  const key = `${result.type}:${normalizePathSlashes(result.path).toLowerCase()}:${result.line ?? 0}:${(result.name ?? result.matchText ?? "").toLowerCase()}`;
  const existing = results.get(key);
  if (!existing || result.score > existing.score) results.set(key, result);
}

function compareDiagnostics(left: WorkspaceDiagnostic, right: WorkspaceDiagnostic) {
  return diagnosticSeverityRank(right.severity) - diagnosticSeverityRank(left.severity) ||
    normalizePathSlashes(left.path).localeCompare(normalizePathSlashes(right.path)) ||
    left.line - right.line ||
    left.column - right.column;
}

function diagnosticSeverityRank(severity: string) {
  switch (severity) {
    case "error":
      return 4;
    case "warning":
      return 3;
    case "information":
      return 2;
    case "hint":
      return 1;
    default:
      return 0;
  }
}

function normalizeSessionTodo(value: unknown, index: number): SessionTodo | null {
  if (!isRecord(value)) return null;
  const content = typeof value.content === "string" ? value.content.trim() : "";
  if (!content) return null;
  const id = typeof value.id === "string" && value.id.trim() ? value.id.trim() : `todo-${index + 1}`;
  const status = normalizeSessionTodoStatus(value.status);
  const priority = normalizeSessionTodoPriority(value.priority);
  const notes = typeof value.notes === "string" && value.notes.trim() ? truncateText(value.notes.trim(), 500) : undefined;
  return { id, content: truncateText(content, 500), status, priority, notes };
}

function normalizeSessionTodoStatus(value: unknown): SessionTodoStatus {
  const normalized = typeof value === "string" ? value.toLowerCase().replace(/[-\s]+/g, "_") : "";
  switch (normalized) {
    case "in_progress":
    case "completed":
    case "blocked":
    case "cancelled":
      return normalized;
    default:
      return "pending";
  }
}

function normalizeSessionTodoPriority(value: unknown): SessionTodoPriority {
  const normalized = typeof value === "string" ? value.toLowerCase() : "";
  switch (normalized) {
    case "low":
    case "high":
      return normalized;
    default:
      return "medium";
  }
}

function summarizeManifest(relativePath: string, text: string) {
  const lower = relativePath.toLowerCase();
  if (lower.endsWith("package.json")) return summarizePackageJson(relativePath, text);
  if (lower.endsWith("cargo.toml")) return summarizeCargoToml(relativePath, text);
  return { path: relativePath, kind: "manifest", summary: truncateText(text, 1200) };
}

function summarizePackageJson(relativePath: string, text: string) {
  try {
    const parsed = JSON.parse(text) as unknown;
    if (!isRecord(parsed)) throw new Error("package.json is not an object");
    return {
      path: relativePath,
      kind: "package.json",
      name: typeof parsed.name === "string" ? parsed.name : null,
      version: typeof parsed.version === "string" ? parsed.version : null,
      scripts: isRecord(parsed.scripts) ? Object.keys(parsed.scripts).slice(0, 20) : [],
      dependencies: packageDependencySummary(parsed),
    };
  } catch (error) {
    return { path: relativePath, kind: "package.json", error: readErrorMessage(error), summary: truncateText(text, 1200) };
  }
}

function packageDependencySummary(parsed: UnknownRecord) {
  const result: Array<{ name: string; version: string; scope: string }> = [];
  for (const scope of ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"]) {
    const dependencies = parsed[scope];
    if (!isRecord(dependencies)) continue;
    for (const [name, version] of Object.entries(dependencies).slice(0, 40)) {
      result.push({ name, version: String(version), scope });
    }
  }
  return result.slice(0, 80);
}

function summarizeCargoToml(relativePath: string, text: string) {
  const packageName = text.match(/^name\s*=\s*"([^"]+)"/m)?.[1] ?? null;
  const version = text.match(/^version\s*=\s*"([^"]+)"/m)?.[1] ?? null;
  const dependencies = Array.from(text.matchAll(/^([A-Za-z0-9_-]+)\s*=\s*(.+)$/gm))
    .filter((match) => !["name", "version", "edition", "license", "authors"].includes(match[1]))
    .slice(0, 80)
    .map((match) => ({ name: match[1], spec: truncateText(match[2].trim(), 180) }));
  return { path: relativePath, kind: "Cargo.toml", name: packageName, version, dependencies };
}

function parseToolContent(result: PromiseSettledResult<ToolResult>) {
  if (result.status !== "fulfilled") return { error: readErrorMessage(result.reason) };
  try {
    const parsed = JSON.parse(result.value.content) as unknown;
    return isRecord(parsed) ? parsed : { value: parsed };
  } catch {
    return { text: result.value.content };
  }
}

function collectSymbolFiles(symbols: UnknownRecord) {
  const paths = new Set<string>();
  const collectLocation = (value: unknown) => {
    if (!isRecord(value)) return;
    if (typeof value.path === "string") paths.add(value.path);
    if (isRecord(value.location)) collectLocation(value.location);
  };
  for (const key of ["workspaceSymbols", "definitions", "references"]) {
    const values = symbols[key];
    if (Array.isArray(values)) values.forEach(collectLocation);
  }
  return Array.from(paths);
}

function buildImpactRiskSignals(relatedFiles: UnknownRecord[], diagnostics: UnknownRecord[], symbolFiles: string[]) {
  const signals: Array<{ level: "low" | "medium" | "high"; message: string }> = [];
  if (diagnostics.length > 0) signals.push({ level: "high", message: `${diagnostics.length} existing diagnostic(s) may mask or compound this change.` });
  if (relatedFiles.some((file) => Array.isArray(file.relations) && file.relations.includes("schema"))) signals.push({ level: "high", message: "Schema/model/migration files are in scope; check persistence and API contracts." });
  if (relatedFiles.some((file) => Array.isArray(file.relations) && file.relations.includes("entrypoint"))) signals.push({ level: "medium", message: "Entrypoints are related; test startup and core flows." });
  if (relatedFiles.some((file) => Array.isArray(file.relations) && file.relations.includes("test"))) signals.push({ level: "low", message: "Related tests were found and should be run after edits." });
  if (symbolFiles.length > 12) signals.push({ level: "medium", message: `${symbolFiles.length} symbol-linked file(s) suggest a broader API surface.` });
  if (signals.length === 0) signals.push({ level: "low", message: "No broad blast-radius signals found in the current indexed context." });
  return signals;
}

function buildImpactValidation(relatedFiles: UnknownRecord[], query: string) {
  const checks = new Set<string>();
  const paths = relatedFiles.map((file) => typeof file.relativePath === "string" ? file.relativePath.toLowerCase() : "");
  if (paths.some((path) => /package\.json|pnpm-lock|yarn\.lock|package-lock/.test(path))) checks.add("Run the package manager test/build commands affected by dependency or script changes.");
  if (paths.some((path) => path.endsWith("cargo.toml") || path.endsWith(".rs"))) checks.add("Run the relevant Cargo tests or cargo check for Rust changes.");
  if (paths.some((path) => /\.(ts|tsx|js|jsx)$/.test(path))) checks.add("Run TypeScript typecheck and the nearest JS/TS test suite.");
  if (paths.some((path) => /\.(css|scss|sass|less)$/.test(path))) checks.add("Verify the affected UI in browser at desktop and mobile widths.");
  if (/test|spec|coverage/i.test(query)) checks.add("Run focused tests first, then the broader suite if shared code changed.");
  if (checks.size === 0) checks.add("Run the smallest relevant build/test command, then broaden if shared files changed.");
  return Array.from(checks).slice(0, 8);
}

function addDirectContextBudgetItems(items: ContextBudgetItem[], input: AiChatSendInput, query: string, queryTokens: string[], includeActiveText: boolean, includeOpenDocuments: boolean) {
  items.push({
    id: "intent:user-message",
    kind: "intent",
    source: "current-user-message",
    score: 130,
    reason: "The current user request defines the task.",
    content: truncateText(input.message.trim() || query, 1_800),
  });

  const recentUser = [...input.history].reverse().find((message) => message.role === "user" && message.content.trim());
  if (recentUser) {
    items.push({
      id: "intent:recent-user-message",
      kind: "intent",
      source: "recent-user-message",
      score: 88,
      reason: "Recent user instruction may constrain the current task.",
      content: truncateText(recentUser.content, 1_200),
    });
  }

  items.push({
    id: "runtime:ai-settings",
    kind: "runtime",
    source: "ai-runtime-preferences",
    score: 92,
    reason: "Model, provider, reasoning, and tool approval mode affect AI behavior.",
    content: [
      `provider=${input.provider.name}`,
      `protocol=${input.provider.protocol}`,
      `baseUrl=${input.provider.baseUrl}`,
      `model=${input.selectedModel.alias || input.selectedModel.id}`,
      `reasoning=${input.preferences.selectedEffortId}`,
      `agent=${input.selectedAgentName || input.preferences.agentMode}`,
      `toolApprovalMode=${input.preferences.toolApprovalMode}`,
    ].join("\n"),
  });

  if (input.activeDocument) {
    const path = input.activeDocument.path ?? input.activeDocument.title;
    items.push({
      id: `active:${input.activeDocument.id}:metadata`,
      kind: "active-document",
      source: path,
      path,
      score: 112 + scoreContextTokens(`${path}\n${input.activeDocument.language_id}`, queryTokens),
      reason: "The active editor is the strongest local signal for the user's current focus.",
      content: [
        `path=${path}`,
        `language=${input.activeDocument.language_id}`,
        `dirty=${input.activeDocument.is_dirty}`,
        `lines=${countLines(input.activeDocument.text)}`,
      ].join("\n"),
    });
    if (includeActiveText && input.activeDocument.text.trim()) {
      items.push({
        id: `active:${input.activeDocument.id}:excerpt`,
        kind: "file-excerpt",
        source: path,
        path,
        score: 104 + scoreContextTokens(`${path}\n${input.activeDocument.text.slice(0, 3_000)}`, queryTokens),
        reason: "Active document excerpt provides immediate code context.",
        content: truncateContextAroundTokens(input.activeDocument.text, queryTokens, 2_800),
      });
    }
  }

  if (includeOpenDocuments) {
    for (const document of input.openDocuments.slice(0, 24)) {
      if (input.activeDocument?.id === document.id) continue;
      const path = document.path ?? document.title;
      const score = (document.is_dirty ? 88 : 58) + scoreContextTokens(`${path}\n${document.text.slice(0, 2_000)}`, queryTokens);
      items.push({
        id: `open:${document.id}`,
        kind: document.is_dirty ? "dirty-document" : "open-document",
        source: path,
        path,
        score,
        reason: document.is_dirty ? "Dirty open file may contain unsaved user work." : "Open editor tab may be relevant to the task.",
        content: [
          `path=${path}`,
          `language=${document.language_id}`,
          `dirty=${document.is_dirty}`,
          truncateContextAroundTokens(document.text, queryTokens, document.is_dirty ? 1_800 : 900),
        ].join("\n"),
      });
    }
  }

  for (const attachment of input.attachments.slice(0, 12)) {
    items.push({
      id: `attachment:${attachment.name}`,
      kind: "attachment",
      source: attachment.name,
      score: 82 + scoreContextTokens(`${attachment.name}\n${attachment.text.slice(0, 2_000)}`, queryTokens),
      reason: "User-attached files are explicit task context.",
      content: truncateContextAroundTokens(attachment.text, queryTokens, 1_800),
    });
  }
}

function addToolContextBudgetItems(items: ContextBudgetItem[], toolResults: PromiseSettledResult<ToolResult>[], queryTokens: string[]) {
  const [memory, rules, docs, related, semantic, diagnostics, git] = toolResults.map(parseToolContent);
  addMemoryBudgetItems(items, memory, queryTokens);
  addContextFilesBudgetItems(items, rules, "rule", "Project rule file constrains code/tool behavior.", queryTokens);
  addContextFilesBudgetItems(items, docs, "doc", "Local documentation or manifest grounds framework/API assumptions.", queryTokens);
  addRelatedBudgetItems(items, related, queryTokens);
  addSemanticBudgetItems(items, semantic, queryTokens);
  addDiagnosticsBudgetItems(items, diagnostics, queryTokens);
  addGitBudgetItems(items, git, queryTokens);
}

function addMemoryBudgetItems(items: ContextBudgetItem[], memory: unknown, queryTokens: string[]) {
  if (!isRecord(memory) || !Array.isArray(memory.signals)) return;
  for (const signal of memory.signals.filter(isRecord).slice(0, 24)) {
    const source = stringField(signal, "source", "memory");
    const text = stringField(signal, "text", "");
    if (!text.trim()) continue;
    items.push({
      id: `memory:${source}:${numberField(signal, "line", 0)}:${items.length}`,
      kind: "memory",
      source,
      line: numberField(signal, "line", 0) || undefined,
      score: 96 + scoreContextTokens(`${source}\n${text}`, queryTokens),
      reason: `Project memory signal: ${stringField(signal, "kind", "memory")}.`,
      content: truncateText(text, 900),
    });
  }
}

function addContextFilesBudgetItems(items: ContextBudgetItem[], value: unknown, kind: string, reason: string, queryTokens: string[]) {
  if (!isRecord(value) || !Array.isArray(value.files)) return;
  for (const file of value.files.filter(isRecord).slice(0, 12)) {
    const source = stringField(file, "relativePath", stringField(file, "path", kind));
    const text = stringField(file, "text", "");
    if (!text.trim()) continue;
    items.push({
      id: `${kind}:${source}`,
      kind,
      source,
      path: stringField(file, "path", source),
      score: (kind === "rule" ? 92 : 76) + scoreContextTokens(`${source}\n${text.slice(0, 2_000)}`, queryTokens),
      reason,
      content: truncateContextAroundTokens(text, queryTokens, kind === "rule" ? 1_200 : 1_000),
    });
  }
}

function addRelatedBudgetItems(items: ContextBudgetItem[], related: unknown, queryTokens: string[]) {
  if (!isRecord(related) || !Array.isArray(related.files)) return;
  for (const file of related.files.filter(isRecord).slice(0, 24)) {
    const source = stringField(file, "relativePath", stringField(file, "path", "related-file"));
    const relations = Array.isArray(file.relations) ? file.relations.filter((relation): relation is string => typeof relation === "string") : [];
    items.push({
      id: `related:${source}`,
      kind: "related-file",
      source,
      path: stringField(file, "path", source),
      score: 62 + numberField(file, "score", 0) / 2 + scoreContextTokens(`${source}\n${relations.join(" ")}`, queryTokens),
      reason: relations.length > 0 ? `Related by ${relations.join(", ")}.` : "Related file candidate from project structure.",
      content: [`path=${source}`, `relations=${relations.join(", ") || "none"}`, `size=${numberField(file, "size", 0) || "unknown"}`].join("\n"),
    });
  }
}

function addSemanticBudgetItems(items: ContextBudgetItem[], semantic: unknown, queryTokens: string[]) {
  if (!isRecord(semantic) || !Array.isArray(semantic.results)) return;
  for (const result of semantic.results.filter(isRecord).slice(0, 24)) {
    const source = stringField(result, "relativePath", stringField(result, "path", "semantic-result"));
    const preview = stringField(result, "preview", stringField(result, "name", source));
    items.push({
      id: `semantic:${stringField(result, "type", "result")}:${source}:${numberField(result, "line", 0)}`,
      kind: "semantic-hit",
      source,
      path: stringField(result, "path", source),
      line: numberField(result, "line", 0) || undefined,
      score: 72 + numberField(result, "score", 0) / 4 + scoreContextTokens(`${source}\n${preview}`, queryTokens),
      reason: `SemanticSearch ${stringField(result, "type", "hit")} hit from ${stringField(result, "source", "workspace")}.`,
      content: truncateText([
        `path=${source}`,
        `line=${numberField(result, "line", 0) || "unknown"}`,
        `name=${stringField(result, "name", "")}`,
        `preview=${preview}`,
      ].filter(Boolean).join("\n"), 900),
    });
  }
}

function addDiagnosticsBudgetItems(items: ContextBudgetItem[], diagnostics: unknown, queryTokens: string[]) {
  if (!isRecord(diagnostics) || !Array.isArray(diagnostics.diagnostics)) return;
  for (const diagnostic of diagnostics.diagnostics.filter(isRecord).slice(0, 40)) {
    const path = stringField(diagnostic, "path", "diagnostic");
    const message = stringField(diagnostic, "message", "");
    const severity = stringField(diagnostic, "severity", "diagnostic");
    items.push({
      id: `diagnostic:${path}:${numberField(diagnostic, "line", 0)}:${items.length}`,
      kind: "diagnostic",
      source: path,
      path,
      line: numberField(diagnostic, "line", 0) || undefined,
      score: (severity === "error" ? 96 : 72) + scoreContextTokens(`${path}\n${message}`, queryTokens),
      reason: `${severity} diagnostic can affect correctness or validation.`,
      content: `${severity} ${path}:${numberField(diagnostic, "line", 0) || "?"}:${numberField(diagnostic, "column", 0) || "?"} ${message}`,
    });
  }
}

function addGitBudgetItems(items: ContextBudgetItem[], git: unknown, queryTokens: string[]) {
  if (!isRecord(git)) return;
  const changedFiles = Array.isArray(git.changedFiles) ? git.changedFiles.filter(isRecord).slice(0, 60) : [];
  if (changedFiles.length === 0 && !stringField(git, "branch", "")) return;
  const content = [
    `branch=${stringField(git, "branch", "unknown")}`,
    `ahead=${numberField(git, "ahead", 0)}`,
    `behind=${numberField(git, "behind", 0)}`,
    ...changedFiles.map((file) => `${stringField(file, "indexStatus", " ")}${stringField(file, "worktreeStatus", " ")} ${stringField(file, "path", "")}`),
  ].join("\n");
  items.push({
    id: "git:status",
    kind: "git",
    source: "git-status",
    score: 78 + changedFiles.length * 2 + scoreContextTokens(content, queryTokens),
    reason: "Git status identifies changed files and branch state that should not be overwritten accidentally.",
    content: truncateText(content, 1_600),
  });
}

function rankContextBudgetItems(items: ContextBudgetItem[], queryTokens: string[]) {
  const seen = new Set<string>();
  return items
    .map((item) => ({ ...item, score: item.score + scoreContextTokens(`${item.source}\n${item.content}`, queryTokens) }))
    .filter((item) => {
      const key = `${item.kind}:${item.source}:${item.line ?? 0}:${item.content.slice(0, 120)}`.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .sort((left, right) => right.score - left.score || contextKindRank(right.kind) - contextKindRank(left.kind) || left.source.localeCompare(right.source));
}

function selectContextBudgetItems(items: ContextBudgetItem[], targetChars: number, maxItems: number) {
  const selected: ContextBudgetItem[] = [];
  let remaining = targetChars;
  for (const item of items) {
    if (selected.length >= maxItems || remaining <= 0) break;
    const reserved = selected.length === 0 ? 600 : 240;
    const maxItemChars = clamp(Math.min(Math.max(600, remaining - reserved), item.kind === "file-excerpt" ? 2_800 : 1_800), 300, 3_200);
    const content = truncateText(item.content, maxItemChars);
    const overhead = item.source.length + item.kind.length + item.reason.length + 160;
    if (content.length + overhead > remaining && selected.length > 0) continue;
    selected.push({ ...item, content });
    remaining -= content.length + overhead;
  }
  return selected;
}

function buildContextBudgeterNextActions(selected: ContextBudgetItem[]) {
  const kinds = new Set(selected.map((item) => item.kind));
  const actions = ["Use the packet in ranked order; earlier items are higher signal for this task."];
  if (kinds.has("diagnostic")) actions.push("Resolve or account for diagnostics before claiming the code is clean.");
  if (kinds.has("related-file") || kinds.has("semantic-hit")) actions.push("Read the highest-ranked related files before editing unfamiliar code.");
  if (kinds.has("git")) actions.push("Preserve existing changed files and avoid overwriting unrelated work.");
  if (kinds.has("rule") || kinds.has("memory")) actions.push("Apply project rules and remembered preferences when choosing tools or implementation style.");
  return actions.slice(0, 6);
}

function contextBudgeterUnavailable(toolResults: PromiseSettledResult<ToolResult>[]) {
  const names = ["MemoryContext", "RulesContext", "DocsContext", "RelatedFiles", "SemanticSearch", "DiagnosticsContext", "GitContext"];
  return toolResults
    .map((result, index) => result.status === "rejected" ? { tool: names[index] ?? `tool-${index + 1}`, error: readErrorMessage(result.reason) } : null)
    .filter((value): value is { tool: string; error: string } => Boolean(value));
}

function truncateContextAroundTokens(text: string, queryTokens: string[], maxChars: number) {
  if (text.length <= maxChars) return text;
  const lower = text.toLowerCase();
  const firstHit = queryTokens.map((token) => lower.indexOf(token)).filter((index) => index >= 0).sort((left, right) => left - right)[0];
  if (firstHit === undefined) return truncateText(text, maxChars);
  const start = Math.max(0, firstHit - Math.floor(maxChars * 0.35));
  const end = Math.min(text.length, start + maxChars);
  const prefix = start > 0 ? `...[truncated ${start} chars before]\n` : "";
  const suffix = end < text.length ? `\n...[truncated ${text.length - end} chars after]` : "";
  return `${prefix}${text.slice(start, end)}${suffix}`;
}

function scoreContextTokens(text: string, queryTokens: string[]) {
  if (queryTokens.length === 0) return 0;
  const lower = text.toLowerCase();
  let score = 0;
  for (const token of queryTokens) {
    if (lower.includes(token)) score += token.length >= 6 ? 18 : 10;
  }
  return score;
}

function contextKindRank(kind: string) {
  switch (kind) {
    case "intent":
      return 12;
    case "active-document":
    case "file-excerpt":
      return 11;
    case "diagnostic":
      return 10;
    case "memory":
    case "rule":
      return 9;
    case "semantic-hit":
    case "related-file":
      return 8;
    case "git":
      return 7;
    case "doc":
      return 6;
    default:
      return 0;
  }
}

function stringField(value: UnknownRecord, key: string, fallback = "") {
  const field = value[key];
  return typeof field === "string" ? field : fallback;
}

function numberField(value: UnknownRecord, key: string, fallback = 0) {
  const field = value[key];
  const numeric = typeof field === "number" ? field : Number(field);
  return Number.isFinite(numeric) ? numeric : fallback;
}

function extractMemorySignals(file: ContextFile, queryTokens: string[]): MemorySignal[] {
  if (file.error || !file.text.trim()) return [];
  const signals: MemorySignal[] = [];
  const lines = file.text.split(/\r?\n/);
  const windowLines = 1;
  for (let index = 0; index < lines.length; index += 1) {
    const rawLine = lines[index];
    const line = rawLine.trim();
    if (!line || line.length < 4) continue;
    const kind = classifyMemoryLine(line);
    if (!kind) continue;
    const contextStart = Math.max(0, index - windowLines);
    const contextEnd = Math.min(lines.length, index + windowLines + 1);
    const context = lines.slice(contextStart, contextEnd).map((candidate) => candidate.trim()).filter(Boolean).join("\n");
    signals.push({
      source: file.relativePath,
      line: index + 1,
      kind,
      score: scoreMemorySignal(file.relativePath, context || line, kind, queryTokens),
      text: truncateText(context || line, 700),
    });
  }
  return signals;
}

function extractChatMemorySignals(input: AiChatSendInput, queryTokens: string[]): MemorySignal[] {
  const recent = input.history.slice(-10);
  const signals: MemorySignal[] = [];
  for (const [index, message] of recent.entries()) {
    const content = message.content.trim();
    if (!content) continue;
    const kind = classifyChatMemory(content, message.role);
    if (!kind) continue;
    signals.push({
      source: `chat:${message.role}:${index + 1}`,
      line: 1,
      kind,
      score: scoreMemorySignal(`chat:${message.role}`, content, kind, queryTokens) + (message.role === "user" ? 16 : 6),
      text: truncateText(content, 900),
    });
  }

  const current = input.message.trim();
  if (current) {
    signals.push({
      source: "chat:current-user-message",
      line: 1,
      kind: "planning",
      score: scoreMemorySignal("chat:current-user-message", current, "planning", queryTokens) + 24,
      text: truncateText(current, 900),
    });
  }
  return signals;
}

function buildRuntimeMemorySignals(input: AiChatSendInput, queryTokens: string[]): MemorySignal[] {
  const model = input.selectedModel.alias || input.selectedModel.id;
  const approval = input.preferences.toolApprovalMode === "full-access"
    ? "Full Access: dangerous tools auto-approve, while workspace guards still apply."
    : "Default: dangerous tools require explicit approval.";
  const values = [
    `AI provider ${input.provider.name} (${input.provider.protocol}) base URL: ${input.provider.baseUrl}.`,
    `Selected model: ${model}; reasoning effort: ${input.preferences.selectedEffortId}.`,
    `Agent mode: ${input.selectedAgentName || input.preferences.agentMode}.`,
    approval,
    `Workspace indexing: enabled=${input.preferences.projectIndexingEnabled}, realtime=${input.preferences.realtimeIndexing}, maxIndexedFiles=${input.preferences.maxIndexedFiles}.`,
  ];
  return values.map((text, index) => ({
    source: "runtime-preferences",
    line: index + 1,
    kind: "runtime" as const,
    score: scoreMemorySignal("runtime-preferences", text, "runtime", queryTokens) + 30,
    text,
  }));
}

function classifyMemoryLine(line: string): MemorySignal["kind"] | null {
  const normalized = line.toLowerCase();
  if (/^#{1,6}\s+/.test(line)) {
    return /decision|preference|todo|roadmap|memory|rule|architecture|adr/.test(normalized) ? "heading" : null;
  }
  if (/\b(adr|decision|decided|chosen|choose|prefer|preference|convention|rule|policy|must|should|required|default|full access|approval mode)\b/i.test(line)) {
    return /prefer|preference|default|mode|setting|style|convention/i.test(line) ? "preference" : "decision";
  }
  if (/\b(todo|fixme|roadmap|next|planned|follow[- ]?up|remaining|blocked|in progress)\b/i.test(line)) return "planning";
  if (/^[-*]\s+\[[ xX-]\]/.test(line)) return "planning";
  return null;
}

function classifyChatMemory(content: string, role: AiChatMessage["role"]): MemorySignal["kind"] | null {
  if (role === "user") return /\b(need|нужно|сделай|добавь|не забудь|default|full access|proxy|model|reasoning|tools?)\b/i.test(content) ? "preference" : null;
  return /\b(done|implemented|changed|verified|remaining|blocked|todo|next)\b/i.test(content) ? "planning" : null;
}

function scoreMemorySignal(source: string, text: string, kind: MemorySignal["kind"], queryTokens: string[]) {
  const lower = `${source}\n${text}`.toLowerCase();
  let score = kind === "runtime" ? 70 : kind === "decision" ? 64 : kind === "preference" ? 60 : kind === "planning" ? 48 : 34;
  if (/full access|approval|proxy|model|reasoning|tool|test|production|prod/i.test(text)) score += 18;
  if (/\.codex|\.cursor|agents\.md|memory|decision|adr|roadmap|preference/i.test(source)) score += 14;
  for (const token of queryTokens) {
    if (lower.includes(token)) score += token.length >= 6 ? 22 : 12;
  }
  return score;
}

function rankMemorySignals(signals: MemorySignal[], queryTokens: string[]) {
  const seen = new Set<string>();
  return signals
    .map((signal) => ({ ...signal, score: signal.score + scoreMemorySignal(signal.source, signal.text, signal.kind, queryTokens) / 10 }))
    .filter((signal) => {
      const key = `${signal.source}:${signal.line}:${signal.text.slice(0, 120)}`.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .sort((left, right) => right.score - left.score || left.source.localeCompare(right.source) || left.line - right.line);
}

function buildDiffReviewFindings(changedFiles: Array<{ path: string; status: string; additions: number; deletions: number; binary: boolean }>, diagnostics: Awaited<ReturnType<typeof luxCommands.diagnosticsSnapshot>>, secrets: SecretFindingInternal[] = []) {
  const findings: Array<{ severity: "low" | "medium" | "high"; path?: string; message: string; evidence: string }> = [];
  const lowerPaths = changedFiles.map((file) => normalizePathSlashes(file.path).toLowerCase());
  if (diagnostics.length > 0) {
    findings.push({ severity: "high", message: "Workspace diagnostics are present while reviewing the diff.", evidence: `${diagnostics.length} diagnostic(s) reported.` });
  }
  if (secrets.length > 0) {
    findings.push({ severity: "high", message: "Potential secrets are present in the current diff.", evidence: `${secrets.length} secret-like value(s) detected and redacted by SecretGuard.` });
  }
  for (const file of changedFiles) {
    const path = normalizePathSlashes(file.path);
    const lower = path.toLowerCase();
    const churn = file.additions + file.deletions;
    if (file.binary) findings.push({ severity: "medium", path, message: "Binary file changed; automated text review cannot inspect content.", evidence: `${file.status} ${path}` });
    if (churn > 500) findings.push({ severity: "medium", path, message: "Large file churn; review for generated output or unrelated changes.", evidence: `+${file.additions} -${file.deletions}` });
    if (/package\.json|cargo\.toml|lock|pnpm-lock|yarn\.lock|package-lock/.test(lower)) findings.push({ severity: "medium", path, message: "Dependency or script metadata changed; verify install/build/test behavior.", evidence: `${file.status} ${path}` });
    if (/migration|schema|model|entity|\.sql$|\.graphql$|\.proto$/.test(lower)) findings.push({ severity: "high", path, message: "Schema or persistence contract changed; verify compatibility and migrations.", evidence: `${file.status} ${path}` });
    if (file.status === "D") findings.push({ severity: "high", path, message: "File deletion needs explicit justification and related references checked.", evidence: `${file.status} ${path}` });
  }
  const hasSource = lowerPaths.some((path) => /\.(ts|tsx|js|jsx|rs|py|go|java|kt|cs)$/.test(path));
  const hasTests = lowerPaths.some((path) => /(^|\/)(__tests__|tests?|specs?)(\/|$)|[._-](test|spec)\./.test(path));
  if (hasSource && !hasTests) findings.push({ severity: "medium", message: "Source files changed without nearby test changes in the current diff.", evidence: "No test/spec paths found among changed files." });
  if (findings.length === 0) findings.push({ severity: "low", message: "No obvious diff risks found from metadata, diagnostics, or file mix.", evidence: `${changedFiles.length} changed file(s).` });
  return findings;
}

function mergeDiffAndStatusFiles(
  diffFiles: Array<{ path: string; old_path: string | null; status: string; additions: number; deletions: number; binary: boolean }>,
  statusFiles: Array<{ path: string; index_status: string; worktree_status: string }>,
) {
  const byPath = new Map<string, { path: string; old_path: string | null; status: string; additions: number; deletions: number; binary: boolean }>();
  for (const file of diffFiles) byPath.set(normalizePathSlashes(file.path).toLowerCase(), file);
  for (const file of statusFiles) {
    const key = normalizePathSlashes(file.path).toLowerCase();
    if (byPath.has(key)) continue;
    const status = file.index_status !== " " && file.index_status !== "?" ? file.index_status : file.worktree_status;
    byPath.set(key, {
      path: file.path,
      old_path: null,
      status: status === "?" ? "A" : status || "M",
      additions: 0,
      deletions: 0,
      binary: false,
    });
  }
  return Array.from(byPath.values()).sort((left, right) => normalizePathSlashes(left.path).localeCompare(normalizePathSlashes(right.path)));
}

function buildReviewDiffChecks(changedFiles: Array<{ path: string }>) {
  const checks = new Set<string>();
  const lowerPaths = changedFiles.map((file) => normalizePathSlashes(file.path).toLowerCase());
  if (lowerPaths.some((path) => /\.(ts|tsx|js|jsx)$/.test(path) || path.endsWith("package.json"))) checks.add("pnpm --filter @lux/desktop typecheck");
  if (lowerPaths.some((path) => /\.(ts|tsx|js|jsx|css|scss)$/.test(path) || path.endsWith("package.json"))) checks.add("pnpm --filter @lux/desktop build");
  if (lowerPaths.some((path) => path.endsWith(".rs") || path.endsWith("cargo.toml"))) checks.add("cargo test --workspace");
  if (lowerPaths.some((path) => /components|\.css$|\.tsx$/.test(path))) checks.add("Browser smoke test for the changed UI flow");
  if (checks.size === 0) checks.add("Run the smallest project-specific test/build command covering the changed files");
  return Array.from(checks);
}

function scanSecrets(text: string, source: string) {
  if (!text) return { source, text, redactedText: text, findings: [] as SecretFindingInternal[] };
  const findings: SecretFindingInternal[] = [];
  const occupied = new Set<number>();

  for (const pattern of secretPatterns) {
    pattern.regex.lastIndex = 0;
    for (const match of text.matchAll(pattern.regex)) {
      const rawMatch = match[0] ?? "";
      const secret = pattern.secretGroup ? match[pattern.secretGroup] ?? rawMatch : rawMatch;
      if (!secret || !isLikelySecretValue(secret, pattern.kind)) continue;
      const matchIndex = match.index ?? 0;
      const relativeSecretIndex = rawMatch.indexOf(secret);
      const start = matchIndex + Math.max(relativeSecretIndex, 0);
      const end = start + secret.length;
      if (rangeHasOccupiedIndex(occupied, start, end)) continue;
      for (let index = start; index < end; index += 1) occupied.add(index);
      const position = offsetToLineColumn(text, start);
      findings.push({
        source,
        kind: pattern.kind,
        label: pattern.label,
        severity: pattern.severity,
        confidence: pattern.confidence,
        line: position.line,
        column: position.column,
        matchLength: secret.length,
        fingerprint: fingerprintSecret(secret),
        preview: secretFindingPreview(text, start, end),
        start,
        end,
        replacement: `${secretPreviewMask}:${pattern.kind}:${fingerprintSecret(secret)}`,
      });
    }
  }

  findings.sort((left, right) => left.start - right.start || right.matchLength - left.matchLength);
  return {
    source,
    text,
    redactedText: redactSecretFindings(text, findings),
    findings,
  };
}

function redactSecretFindings(text: string, findings: SecretFindingInternal[]) {
  if (findings.length === 0) return text;
  let output = "";
  let cursor = 0;
  for (const finding of findings) {
    if (finding.start < cursor) continue;
    output += text.slice(cursor, finding.start);
    output += finding.replacement;
    cursor = finding.end;
  }
  return output + text.slice(cursor);
}

function publicSecretFinding(finding: SecretFindingInternal): SecretFinding {
  const { start: _start, end: _end, replacement: _replacement, ...publicFinding } = finding;
  return publicFinding;
}

function compareSecretFindings(left: SecretFindingInternal, right: SecretFindingInternal) {
  return secretSeverityRank(right.severity) - secretSeverityRank(left.severity) ||
    confidenceRank(right.confidence) - confidenceRank(left.confidence) ||
    left.source.localeCompare(right.source) ||
    left.line - right.line ||
    left.column - right.column;
}

function highestSecretSeverity(findings: SecretFindingInternal[]) {
  return findings.reduce<SecretSeverity | "none">((highest, finding) => {
    if (highest === "none") return finding.severity;
    return secretSeverityRank(finding.severity) > secretSeverityRank(highest) ? finding.severity : highest;
  }, "none");
}

function secretSeverityRank(severity: SecretSeverity) {
  switch (severity) {
    case "critical":
      return 4;
    case "high":
      return 3;
    case "medium":
      return 2;
    case "low":
      return 1;
    default:
      return 0;
  }
}

function confidenceRank(confidence: SecretFinding["confidence"]) {
  switch (confidence) {
    case "high":
      return 3;
    case "medium":
      return 2;
    case "low":
      return 1;
    default:
      return 0;
  }
}

function isLikelySecretValue(value: string, kind: string) {
  if (kind === "private-key-block") return true;
  if (value.length < 12) return false;
  if (/^(true|false|null|undefined|localhost|example|changeme|password|secret|token|apikey|api_key)$/i.test(value)) return false;
  if (/^[0-9.:-]+$/.test(value)) return false;
  const uniqueChars = new Set(value).size;
  if (uniqueChars < 8 && value.length > 16) return false;
  return true;
}

function rangeHasOccupiedIndex(occupied: Set<number>, start: number, end: number) {
  for (let index = start; index < end; index += 1) {
    if (occupied.has(index)) return true;
  }
  return false;
}

function offsetToLineColumn(text: string, offset: number) {
  let line = 1;
  let column = 1;
  for (let index = 0; index < offset && index < text.length; index += 1) {
    if (text[index] === "\n") {
      line += 1;
      column = 1;
    } else {
      column += 1;
    }
  }
  return { line, column };
}

function secretFindingPreview(text: string, start: number, end: number) {
  const lineStart = Math.max(text.lastIndexOf("\n", start - 1) + 1, 0);
  const nextLine = text.indexOf("\n", end);
  const lineEnd = nextLine === -1 ? text.length : nextLine;
  const before = text.slice(lineStart, start);
  const after = text.slice(end, lineEnd);
  return truncateText(`${before}${secretPreviewMask}${after}`.trim(), 500);
}

function fingerprintSecret(secret: string) {
  let hash = 2166136261;
  for (let index = 0; index < secret.length; index += 1) {
    hash ^= secret.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

function addRelatedKindRelations(descriptor: RelatedFileDescriptor, relations: Set<RelatedFileRelation>) {
  let score = 0;
  if (isTestFile(descriptor)) {
    relations.add("test");
    score += 35;
  }
  if (isStyleFile(descriptor)) {
    relations.add("style");
    score += 30;
  }
  if (isTypeDefinitionFile(descriptor)) {
    relations.add("type-definition");
    score += 28;
  }
  if (isRouteFile(descriptor)) {
    relations.add("route");
    score += 24;
  }
  if (isSchemaFile(descriptor)) {
    relations.add("schema");
    score += 24;
  }
  if (isConfigFile(descriptor)) {
    relations.add("config");
    score += 18;
  }
  if (isEntrypointFile(descriptor)) {
    relations.add("entrypoint");
    score += 18;
  }
  if (isStoryFile(descriptor)) {
    relations.add("story");
    score += 22;
  }
  if (isBarrelFile(descriptor)) {
    relations.add("barrel");
    score += 14;
  }
  return score;
}

function createRelatedFileDescriptor(entry: Pick<FsEntry, "path"> & Partial<FsEntry>, workspaceRoot: string): RelatedFileDescriptor {
  const path = normalizePathSlashes(entry.path);
  const root = normalizePathSlashes(workspaceRoot).replace(/\/+$/, "");
  const relativePath = root && path.toLowerCase().startsWith(`${root.toLowerCase()}/`)
    ? path.slice(root.length + 1)
    : path;
  const basename = path.split("/").pop() ?? path;
  const dir = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  const relativeDir = relativePath.includes("/") ? relativePath.slice(0, relativePath.lastIndexOf("/")) : "";
  const extension = fileExtension(basename);
  const stem = basename.slice(0, basename.length - extension.length);
  const familyStem = familyStemFromBasename(basename);
  return {
    entry: entry.kind ? entry as FsEntry : undefined,
    path,
    relativePath,
    lower: path.toLowerCase(),
    relativeLower: relativePath.toLowerCase(),
    dir,
    relativeDir,
    basename,
    basenameLower: basename.toLowerCase(),
    extension,
    stem,
    stemLower: stem.toLowerCase(),
    familyStem,
    familyStemLower: familyStem.toLowerCase(),
  };
}

function tokenizeRelatedQuery(query: string) {
  const tokens = new Set<string>();
  query
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .toLowerCase()
    .split(/[^a-z0-9_-]+/i)
    .map((token) => token.trim().replace(/^[-_]+|[-_]+$/g, ""))
    .filter(Boolean)
    .forEach((token) => {
      if (token.length < 3 && !relatedShortUsefulTokens.has(token)) return;
      if (relatedStopWords.has(token)) return;
      tokens.add(token);
    });
  return Array.from(tokens).slice(0, 12);
}

function familyStemFromBasename(basename: string) {
  return basename
    .replace(/(\.d)?\.[^.]+$/, "")
    .replace(/\.(test|spec|stories|story|module|types|schema|route|routes|model|models|entity|entities|service|controller|view|styles?|style|component|page|layout|hook|hooks|util|utils|helper|helpers)$/i, "")
    .replace(/[-_.](test|spec|stories|story|module|types|schema|route|routes|model|models|entity|entities|service|controller|view|styles?|style|component|page|layout|hook|hooks|util|utils|helper|helpers)$/i, "");
}

function fileExtension(basename: string) {
  const lowered = basename.toLowerCase();
  if (lowered.endsWith(".d.ts")) return ".d.ts";
  if (lowered.endsWith(".d.mts")) return ".d.mts";
  if (lowered.endsWith(".d.cts")) return ".d.cts";
  const dot = basename.lastIndexOf(".");
  return dot > 0 ? basename.slice(dot) : "";
}

function resolveWorkspacePath(path: string, workspaceRoot: string) {
  const normalized = normalizePathSlashes(path.trim());
  if (!workspaceRoot || /^[a-z]:\//i.test(normalized) || normalized.startsWith("/")) return normalized;
  return `${normalizePathSlashes(workspaceRoot).replace(/\/+$/, "")}/${normalized.replace(/^\/+/, "")}`;
}

function normalizePathSlashes(path: string) {
  return path.replaceAll("\\", "/");
}

function isPathInsideWorkspace(path: string, workspaceRoot: string) {
  const root = normalizePathSlashes(workspaceRoot).replace(/\/+$/, "").toLowerCase();
  const normalized = normalizePathSlashes(path).replace(/\/+$/, "").toLowerCase();
  return Boolean(root) && (normalized === root || normalized.startsWith(`${root}/`));
}

function relatedDirectoryDistance(left: string, right: string) {
  if (left === right) return 0;
  const leftParts = left.split("/").filter(Boolean);
  const rightParts = right.split("/").filter(Boolean);
  let common = 0;
  while (leftParts[common] && leftParts[common] === rightParts[common]) common += 1;
  return (leftParts.length - common) + (rightParts.length - common);
}

function isLowSignalRelatedPath(path: string) {
  const lower = normalizePathSlashes(path).toLowerCase();
  return relatedIgnoredPathPattern.test(lower) || relatedBinaryFilePattern.test(lower) || (!relatedSourceFilePattern.test(lower) && !isKnownExtensionlessProjectFile(lower));
}

function isKnownExtensionlessProjectFile(lowerPath: string) {
  const basename = lowerPath.split("/").pop() ?? lowerPath;
  return /^(dockerfile|makefile|readme|license|notice|procfile|gemfile|rakefile)$/.test(basename);
}

function isTestFile(file: RelatedFileDescriptor) {
  return /(^|[._-])(test|spec|tests|specs)([._-]|$)/.test(file.basenameLower) || /(^|\/)(__tests__|tests?|specs?)(\/|$)/.test(file.relativeLower);
}

function isStyleFile(file: RelatedFileDescriptor) {
  return /\.(css|scss|sass|less)$/.test(file.extension.toLowerCase()) || /(^|[._-])(styles?|theme|tokens)([._-]|$)/.test(file.basenameLower);
}

function isTypeDefinitionFile(file: RelatedFileDescriptor) {
  return /\.d\.(ts|mts|cts)$/.test(file.basenameLower) || /(^|[._-])(types?|interfaces?|dto|defs)([._-]|$)/.test(file.basenameLower);
}

function isRouteFile(file: RelatedFileDescriptor) {
  return /(^|[._-])(route|routes|router|page|layout)([._-]|$)/.test(file.basenameLower) || /(^|\/)(app|pages|routes?)(\/|$)/.test(file.relativeLower);
}

function isSchemaFile(file: RelatedFileDescriptor) {
  return /(^|[._-])(schema|schemas|model|models|entity|entities|migration|prisma|graphql|proto)([._-]|$)/.test(file.basenameLower) || /\.(graphql|gql|proto|sql)$/.test(file.extension.toLowerCase());
}

function isConfigFile(file: RelatedFileDescriptor) {
  return /(^|[._-])(config|conf|rc|settings|eslint|prettier|vite|webpack|rollup|tsconfig|jsconfig|cargo|package|pyproject)([._-]|$)/.test(file.basenameLower) || /(^|\/)(package\.json|cargo\.toml|pyproject\.toml|go\.mod|pom\.xml|build\.gradle|vite\.config\.)/.test(file.relativeLower);
}

function isEntrypointFile(file: RelatedFileDescriptor) {
  return /(^|\/)(main|index|app|lib|mod)\.(ts|tsx|js|jsx|rs|go|py|java|cs|kt|swift)$/.test(file.relativeLower) || /(^|\/)(src\/main\.rs|src-tauri\/src\/lib\.rs)$/.test(file.relativeLower);
}

function isStoryFile(file: RelatedFileDescriptor) {
  return /(^|[._-])(stories|story)([._-]|$)/.test(file.basenameLower);
}

function isBarrelFile(file: RelatedFileDescriptor) {
  return /^(index|mod|lib)\.(ts|tsx|js|jsx|rs)$/.test(file.basenameLower);
}

function isImportantProjectFile(file: RelatedFileDescriptor) {
  return /(^|\/)(package\.json|cargo\.toml|pyproject\.toml|go\.mod|pom\.xml|build\.gradle|vite\.config\.|tsconfig\.|jsconfig\.|readme|dockerfile|makefile|\.env\.example)/.test(file.relativeLower);
}

function addImportantFileRelation(file: RelatedFileDescriptor, relations: Set<RelatedFileRelation>) {
  if (isConfigFile(file)) relations.add("config");
  if (isEntrypointFile(file)) relations.add("entrypoint");
  if (/readme|license|notice/.test(file.basenameLower)) relations.add("nearby-name");
}

function isSourceCounterpart(file: RelatedFileDescriptor, target: RelatedFileDescriptor) {
  if (file.extension.toLowerCase() === target.extension.toLowerCase()) return false;
  const relatedExtensions = new Set([".ts", ".tsx", ".js", ".jsx", ".css", ".scss", ".sass", ".less", ".d.ts"]);
  return relatedExtensions.has(file.extension.toLowerCase()) && relatedExtensions.has(target.extension.toLowerCase());
}

function topCounts(values: string[], limit: number) {
  const counts = new Map<string, number>();
  for (const value of values) counts.set(value, (counts.get(value) ?? 0) + 1);
  return Array.from(counts.entries())
    .sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]))
    .slice(0, limit)
    .map(([name, count]) => ({ name, count }));
}

function topDirectory(path: string) {
  const parts = normalizePathSlashes(path).split("/").filter(Boolean);
  if (parts.length === 0) return ".";
  if (parts[0].startsWith(".")) return parts[0];
  return parts.length > 1 && ["apps", "crates", "packages", "src"].includes(parts[0]) ? `${parts[0]}/${parts[1]}` : parts[0];
}

function languageForPath(path: string) {
  const lower = path.toLowerCase();
  if (lower.endsWith(".tsx") || lower.endsWith(".ts") || lower.endsWith(".mts") || lower.endsWith(".cts")) return "typescript";
  if (lower.endsWith(".jsx") || lower.endsWith(".js") || lower.endsWith(".mjs") || lower.endsWith(".cjs")) return "javascript";
  if (lower.endsWith(".rs")) return "rust";
  if (lower.endsWith(".py")) return "python";
  if (lower.endsWith(".go")) return "go";
  if (lower.endsWith(".java") || lower.endsWith(".kt") || lower.endsWith(".kts")) return "jvm";
  if (lower.endsWith(".cs")) return "csharp";
  if (/\.(css|scss|sass|less)$/.test(lower)) return "styles";
  if (/\.(json|ya?ml|toml|xml)$/.test(lower)) return "config-data";
  if (/\.(md|mdx)$/.test(lower) || /readme|license|notice/.test(lower)) return "docs";
  if (/\.(html|vue|svelte|astro)$/.test(lower)) return "web";
  if (/\.(sql|graphql|gql|proto)$/.test(lower)) return "schema";
  return "other";
}

function isSourcePath(file: RelatedFileDescriptor) {
  return file.relativeLower.includes("/src/") || file.relativeLower.startsWith("src/") || /\.(ts|tsx|js|jsx|rs|py|go|java|kt|cs|vue|svelte|astro)$/.test(file.extension.toLowerCase());
}

function compareRelatedDescriptors(left: RelatedFileDescriptor, right: RelatedFileDescriptor) {
  return scorePath(right.relativePath) - scorePath(left.relativePath) || left.relativeLower.localeCompare(right.relativeLower);
}

function compactIndexedFile(file: RelatedFileDescriptor) {
  return {
    path: file.path,
    relativePath: file.relativePath,
    language: languageForPath(file.basenameLower),
    size: file.entry?.size ?? null,
    modifiedAt: file.entry?.modified_at ?? null,
  };
}

async function gitContext(): Promise<ToolResult> {
  const status = await luxCommands.gitStatus();
  return toolJson("GitContext", {
    branch: status.branch,
    ahead: status.ahead,
    behind: status.behind,
    changedFiles: status.files.map((file) => ({
      path: file.path,
      indexStatus: file.index_status,
      worktreeStatus: file.worktree_status,
    })),
  });
}

async function testHealth(): Promise<ToolResult> {
  const health = await luxCommands.testHealth();
  return toolJson("TestHealth", {
    workspaceRoot: health.workspaceRoot,
    status: health.status,
    summary: health.summary,
    runners: health.runners.map((runner) => ({
      path: runner.workspaceRelativePath,
      status: runner.status,
      kind: runner.kind,
      language: runner.language,
      framework: runner.framework,
      command: runner.command,
      exitCode: runner.exitCode,
      durationMs: runner.durationMs,
      timedOut: runner.timedOut,
      stdout: runner.stdout,
      stderr: runner.stderr,
    })),
    language: health.language,
    framework: health.framework,
    command: health.command,
    exitCode: health.exitCode,
    durationMs: health.durationMs,
    timedOut: health.timedOut,
    stdout: health.stdout,
    stderr: health.stderr,
  });
}

async function failureAnalyzer(args: UnknownRecord): Promise<ToolResult> {
  const rawLog = stringArg(args, "log");
  const includeTestHealth = booleanArg(args, "includeTestHealth", true);
  const includeDiagnostics = booleanArg(args, "includeDiagnostics", true);
  const maxFindings = clamp(numberArg(args, "maxFindings", 12), 1, 40);
  const [healthResult, diagnosticsResult] = await Promise.allSettled([
    includeTestHealth ? luxCommands.testHealth() : Promise.resolve(null),
    includeDiagnostics ? luxCommands.diagnosticsSnapshot() : Promise.resolve([]),
  ]);
  const health = healthResult.status === "fulfilled" ? healthResult.value : null;
  const diagnostics = diagnosticsResult.status === "fulfilled" ? diagnosticsResult.value : [];
  const sections = collectFailureSections(rawLog, health);
  const findings = rankFailureFindings([
    ...diagnostics.slice(0, 80).map((diagnostic): FailureFinding => ({
      source: "diagnostics",
      kind: diagnostic.severity || "diagnostic",
      message: diagnostic.message,
      path: diagnostic.path,
      line: diagnostic.line,
      column: diagnostic.column,
      evidence: `${diagnostic.path}:${diagnostic.line}:${diagnostic.column} ${diagnostic.message}`,
    })),
    ...sections.flatMap((section) => extractFailureFindings(section.source, section.text)),
  ]).slice(0, maxFindings);
  const affectedFiles = Array.from(new Set(findings.flatMap((finding) => finding.path ? [finding.path] : extractPathsFromText(finding.evidence)))).slice(0, 24);

  return toolJson("FailureAnalyzer", {
    status: health?.status ?? (findings.length > 0 ? "failed" : "unknown"),
    summary: buildFailureSummary(findings, health),
    testHealth: health ? {
      status: health.status,
      summary: health.summary,
      runners: health.runners.map((runner) => ({
        path: runner.workspaceRelativePath,
        status: runner.status,
        kind: runner.kind,
        language: runner.language,
        framework: runner.framework,
        command: runner.command,
        exitCode: runner.exitCode,
        timedOut: runner.timedOut,
      })),
    } : null,
    diagnosticsUnavailable: diagnosticsResult.status === "rejected" ? readErrorMessage(diagnosticsResult.reason) : null,
    testHealthUnavailable: healthResult.status === "rejected" ? readErrorMessage(healthResult.reason) : null,
    affectedFiles,
    findings,
    nextActions: buildFailureNextActions(findings, health),
  });
}

function collectFailureSections(rawLog: string, health: Awaited<ReturnType<typeof luxCommands.testHealth>> | null) {
  const sections: Array<{ source: string; text: string }> = [];
  if (rawLog.trim()) sections.push({ source: "provided-log", text: rawLog });
  if (!health) return sections;
  if (health.stderr || health.stdout) sections.push({ source: "test-health", text: [health.stderr, health.stdout].filter(Boolean).join("\n") });
  for (const runner of health.runners) {
    const text = [runner.stderr, runner.stdout].filter(Boolean).join("\n");
    if (text.trim()) sections.push({ source: `runner:${runner.workspaceRelativePath || runner.framework}`, text });
  }
  return sections;
}

function extractFailureFindings(source: string, text: string): FailureFinding[] {
  const lines = text.split(/\r?\n/);
  const findings: FailureFinding[] = [];
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index].trimEnd();
    if (!isFailureSignalLine(line)) continue;
    const context = compactFailureContext(lines, index);
    const location = firstFailureLocation(context) ?? firstFailureLocation(line);
    findings.push({
      source,
      kind: classifyFailureLine(line),
      message: compactFailureMessage(line),
      path: location?.path,
      line: location?.line,
      column: location?.column,
      evidence: context,
    });
  }
  return findings;
}

function isFailureSignalLine(line: string) {
  return /\b(error|failed|failure|panic|exception|traceback|assertion|expected|received|mismatch|cannot find|not found|undefined|timed out|timeout|exit code|compilation failed|typeerror|referenceerror|syntaxerror)\b/i.test(line) ||
    /^\s*(E\s+|FAIL\b|FAILED\b|error\[E\d+\]|\[ERROR\]|Caused by:)/.test(line) ||
    /\s+at\s+[^\s]+\(.+?:\d+:\d+\)/.test(line);
}

function classifyFailureLine(line: string) {
  if (/traceback|exception|typeerror|referenceerror|syntaxerror/i.test(line)) return "runtime-exception";
  if (/assert|expected|received|mismatch|should|failed/i.test(line)) return "test-assertion";
  if (/cannot find|not found|undefined|missing module|module not found/i.test(line)) return "missing-reference";
  if (/timed out|timeout/i.test(line)) return "timeout";
  if (/panic|error\[E\d+\]|compilation failed|ts\d{4}/i.test(line)) return "compiler-error";
  return "failure";
}

function compactFailureMessage(line: string) {
  return truncateText(line.trim().replace(/^\s*(FAIL|FAILED|ERROR|E)\s*:?\s*/i, ""), 260);
}

function compactFailureContext(lines: string[], index: number) {
  const start = Math.max(0, index - 2);
  const end = Math.min(lines.length, index + 5);
  return truncateText(lines.slice(start, end).join("\n").trim(), 1600);
}

function firstFailureLocation(text: string) {
  const patterns = [
    /([A-Za-z]:[\\/][^\s:()<>"']+):(\d+):(\d+)/,
    /([A-Za-z]:[\\/][^\s:()<>"']+):(\d+)/,
    /([./]?[\w@~ -][\w@~./\\ -]+\.[A-Za-z][\w]+):(\d+):(\d+)/,
    /([./]?[\w@~ -][\w@~./\\ -]+\.[A-Za-z][\w]+):(\d+)/,
  ];
  for (const pattern of patterns) {
    const match = text.match(pattern);
    if (!match) continue;
    return {
      path: normalizePathSlashes(match[1]),
      line: Number(match[2]),
      column: match[3] ? Number(match[3]) : undefined,
    };
  }
  return null;
}

function extractPathsFromText(text: string) {
  const paths = new Set<string>();
  const pattern = /(?:[A-Za-z]:[\\/]|\.\.?[\\/]|[\w@~-]+[\\/])[\w@~./\\ -]+\.[A-Za-z][\w]+/g;
  for (const match of text.matchAll(pattern)) paths.add(normalizePathSlashes(match[0]));
  return Array.from(paths);
}

function rankFailureFindings(findings: FailureFinding[]) {
  const seen = new Set<string>();
  return findings
    .filter((finding) => {
      const key = `${finding.kind}:${finding.path ?? ""}:${finding.line ?? ""}:${finding.message}`.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .sort((left, right) => failureFindingScore(right) - failureFindingScore(left));
}

function failureFindingScore(finding: FailureFinding) {
  let score = 0;
  if (finding.path) score += 30;
  if (finding.line) score += 12;
  if (finding.source === "diagnostics") score += 18;
  if (finding.kind === "compiler-error" || finding.kind === "runtime-exception") score += 16;
  if (finding.kind === "test-assertion") score += 12;
  if (finding.kind === "timeout") score += 8;
  if (/node_modules|target|dist|build/.test(finding.evidence.toLowerCase())) score -= 18;
  return score;
}

function buildFailureSummary(findings: FailureFinding[], health: Awaited<ReturnType<typeof luxCommands.testHealth>> | null) {
  if (findings.length === 0) return health?.status === "passed" ? "No failures detected in diagnostics or test output." : "No specific failure signal was extracted.";
  const top = findings[0];
  const location = top.path ? ` at ${top.path}${top.line ? `:${top.line}` : ""}` : "";
  return `${findings.length} failure signal${findings.length === 1 ? "" : "s"}; top candidate: ${top.kind}${location}.`;
}

function buildFailureNextActions(findings: FailureFinding[], health: Awaited<ReturnType<typeof luxCommands.testHealth>> | null) {
  const actions = new Set<string>();
  const top = findings[0];
  if (top?.path) actions.add(`Open ${top.path}${top.line ? `:${top.line}` : ""} and inspect the reported code path.`);
  if (findings.some((finding) => finding.kind === "compiler-error" || finding.source === "diagnostics")) actions.add("Fix compiler/language diagnostics before rerunning tests.");
  if (findings.some((finding) => finding.kind === "missing-reference")) actions.add("Check imports, generated files, package names, and workspace-relative paths for missing references.");
  if (findings.some((finding) => finding.kind === "test-assertion")) actions.add("Compare the failing assertion's expected and received values, then inspect the nearest related source and test files.");
  if (findings.some((finding) => finding.kind === "timeout") || health?.timedOut) actions.add("Look for async waits, deadlocks, hanging dev servers, or tests that need tighter timeouts/mocks.");
  if (health && health.status !== "passed" && health.command) actions.add(`Rerun the focused failing command after changes: ${health.command}`);
  return Array.from(actions).slice(0, 6);
}

function normalizeAssistantMessage(value: unknown) {
  if (!isRecord(value)) return { role: "assistant" as const, content: "", tool_calls: [] as OpenAiToolCall[] };
  return {
    role: "assistant" as const,
    content: typeof value.content === "string" ? value.content : "",
    tool_calls: normalizeToolCalls(value.tool_calls),
  };
}

function normalizeToolCalls(value: unknown): OpenAiToolCall[] {
  if (!Array.isArray(value)) return [];
  return value.filter(isRecord).map((call, index) => ({
    id: typeof call.id === "string" ? call.id : `tool-${Date.now()}-${index}`,
    type: call.type === "function" ? "function" : "function",
    function: isRecord(call.function) ? {
      name: typeof call.function.name === "string" ? call.function.name : "",
      arguments: typeof call.function.arguments === "string" ? call.function.arguments : "{}",
    } : { name: "", arguments: "{}" },
  }));
}

function firstChoice(value: unknown) {
  if (!isRecord(value) || !Array.isArray(value.choices)) return null;
  return value.choices.find(isRecord) ?? null;
}

function createRunningToolCall(call: OpenAiToolCall): AiChatToolCall {
  const name = call.function?.name || "Tool";
  return {
    id: call.id ?? crypto.randomUUID(),
    tool: name,
    status: "running",
    input: summarizeToolInput(call.function?.arguments),
    startTime: Date.now(),
  };
}

function summarizeToolInput(value: string | undefined) {
  if (!value) return "";
  try {
    const parsed = JSON.parse(value) as unknown;
    if (isRecord(parsed)) {
      return Object.entries(parsed).map(([key, entry]) => `${key}: ${String(entry)}`).join(", ");
    }
  } catch {
    return truncateText(value, 180);
  }
  return truncateText(value, 180);
}

function parseToolArguments(value: string | undefined): UnknownRecord {
  if (!value) return {};
  try {
    const parsed = JSON.parse(value) as unknown;
    return isRecord(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function reasoningPayload(effortId: string, provider: AiProviderConfig) {
  if (!effortId) return {};
  const normalizedEffort = effortId === "xhigh" && provider.protocol !== "local-proxy" ? "high" : effortId;
  return {
    reasoning_effort: normalizedEffort,
    reasoning: { effort: normalizedEffort },
  };
}

function toolJson(title: string, value: unknown): ToolResult {
  const content = JSON.stringify(value, null, 2);
  return {
    title,
    content: truncateText(content, maxToolOutputChars),
  };
}

function toolResultFromFileOperation(title: string, result: FileToolResult): ToolResult {
  return {
    title: result.message,
    content: truncateText(JSON.stringify({
      operation: result.operation,
      path: result.path,
      savedToDisk: result.savedToDisk,
      changedPaths: result.changedPaths,
      stats: result.stats,
      message: result.message,
    }, null, 2), maxToolOutputChars),
    stats: result.stats,
  };
}

function settledContent(name: string, result: PromiseSettledResult<ToolResult>) {
  if (result.status === "fulfilled") return `## ${name}\n${result.value.content}`;
  return `## ${name}\n${JSON.stringify({ error: readErrorMessage(result.reason) })}`;
}

function objectSchema(properties: Record<string, unknown>, required: string[] = []) {
  return {
    type: "object",
    properties,
    required,
    additionalProperties: false,
  };
}

function stringSchema(description: string) {
  return { type: "string", description };
}

function numberSchema(description: string) {
  return { type: "number", description };
}

function booleanSchema(description: string) {
  return { type: "boolean", description };
}

function arraySchema(description: string) {
  return { type: "array", description, items: { type: "string" } };
}

function stringArg(args: UnknownRecord, key: string, fallback = "") {
  const value = args[key];
  return typeof value === "string" ? value : fallback;
}

function numberArg(args: UnknownRecord, key: string, fallback: number) {
  const value = args[key];
  const numeric = typeof value === "number" ? value : Number(value);
  return Number.isFinite(numeric) ? numeric : fallback;
}

function booleanArg(args: UnknownRecord, key: string, fallback: boolean) {
  const value = args[key];
  return typeof value === "boolean" ? value : fallback;
}

function stringArrayArg(args: UnknownRecord, key: string) {
  const value = args[key];
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function optionalPositiveNumberArg(args: UnknownRecord, key: string) {
  const value = args[key];
  if (value === undefined || value === null || value === "") return null;
  const numeric = typeof value === "number" ? value : Number(value);
  return Number.isFinite(numeric) && numeric > 0 ? Math.round(numeric) : null;
}

function compactLocation(location: LspLocation) {
  return {
    path: location.path,
    range: location.range,
  };
}

function compactDocumentSymbol(symbol: LspDocumentSymbol): unknown {
  return {
    name: symbol.name,
    detail: symbol.detail,
    kind: symbol.kind,
    range: symbol.range,
    selectionRange: symbol.selection_range,
    children: symbol.children.map(compactDocumentSymbol),
  };
}

function normalizePathForCompare(path: string) {
  return path.replaceAll("\\", "/").toLowerCase();
}

function scorePath(path: string) {
  const lower = path.toLowerCase().replaceAll("\\", "/");
  let score = 0;
  if (/package\.json$|cargo\.toml$|vite\.config\.|tsconfig\.|readme|src\/app\.|src\/main\.|src-tauri\/src\/lib\.rs/.test(lower)) score += 100;
  if (lower.includes("/src/")) score += 25;
  if (lower.includes("/components/")) score += 10;
  if (lower.includes("/node_modules/") || lower.includes("/target/") || lower.includes("/dist/")) score -= 200;
  return score;
}

function truncateText(text: string, maxChars: number) {
  if (text.length <= maxChars) return text;
  return `${text.slice(0, maxChars)}\n...[truncated ${text.length - maxChars} chars]`;
}

function clamp(value: number, min: number, max: number) {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.round(value)));
}

function throwIfAborted(signal: AbortSignal) {
  if (signal.aborted) throw new DOMException("AI request was cancelled", "AbortError");
}

function readErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function isAbortErrorLike(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
