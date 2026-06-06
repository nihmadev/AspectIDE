/** Mirrors pruneStaleToolOutputs from aiChatContextCompaction.ts */
const TOOL_OUTPUT_PRUNE_MARKER = "[Lux · tool output pruned";
const PRESERVE_FULL_TOOL_OUTPUT_ASSISTANT_TURNS = 3;
const MIN_TOOL_OUTPUT_CHARS_TO_PRUNE = 320;

function estimateTokens(value) {
  const trimmed = value.trim();
  if (!trimmed) return 0;
  return Math.ceil(trimmed.length / 4);
}

function pruneToolCallOutput(call) {
  const output = call.output ?? "";
  if (!output || output.includes(TOOL_OUTPUT_PRUNE_MARKER) || output.length < MIN_TOOL_OUTPUT_CHARS_TO_PRUNE) {
    return call;
  }
  const tokens = estimateTokens(output);
  return {
    ...call,
    output: `${TOOL_OUTPUT_PRUNE_MARKER} · ~${tokens} tokens · re-run the tool to reload the full payload]`,
  };
}

function pruneAssistantMessageToolOutputs(message) {
  const toolCalls = message.toolCalls?.map(pruneToolCallOutput);
  const changedTools = toolCalls && toolCalls.some((call, index) => call !== message.toolCalls[index]);
  if (!changedTools) return message;
  return { ...message, toolCalls };
}

function pruneStaleToolOutputs(messages, preserveRecentAssistantTurns = PRESERVE_FULL_TOOL_OUTPUT_ASSISTANT_TURNS) {
  const assistantIndices = [];
  for (let index = 0; index < messages.length; index += 1) {
    if (messages[index]?.role === "assistant") assistantIndices.push(index);
  }
  if (assistantIndices.length <= preserveRecentAssistantTurns) return messages;
  const cutoffIndex = assistantIndices[assistantIndices.length - preserveRecentAssistantTurns - 1] ?? -1;
  if (cutoffIndex < 0) return messages;
  let changed = false;
  const next = messages.map((message, index) => {
    if (index > cutoffIndex || message.role !== "assistant") return message;
    const pruned = pruneAssistantMessageToolOutputs(message);
    if (pruned !== message) changed = true;
    return pruned;
  });
  return changed ? next : messages;
}

const longOutput = "x".repeat(500);
const messages = [
  { id: "u1", role: "user", content: "go" },
  { id: "a1", role: "assistant", content: "ok", toolCalls: [{ id: "t1", tool: "Read", status: "success", output: longOutput }] },
  { id: "u2", role: "user", content: "go2" },
  { id: "a2", role: "assistant", content: "ok2", toolCalls: [{ id: "t2", tool: "Read", status: "success", output: longOutput }] },
  { id: "u3", role: "user", content: "go3" },
  { id: "a3", role: "assistant", content: "ok3", toolCalls: [{ id: "t3", tool: "Read", status: "success", output: longOutput }] },
  { id: "u4", role: "user", content: "go4" },
  { id: "a4", role: "assistant", content: "ok4", toolCalls: [{ id: "t4", tool: "Read", status: "success", output: longOutput }] },
];

const pruned = pruneStaleToolOutputs(messages);
if (pruned[1].toolCalls[0].output === longOutput) {
  console.error("oldest assistant output should be pruned");
  process.exit(1);
}
if (!pruned[7].toolCalls[0].output.includes(longOutput.slice(0, 20))) {
  console.error("latest assistant output should stay full");
  process.exit(1);
}
if (!pruned[1].toolCalls[0].output.includes(TOOL_OUTPUT_PRUNE_MARKER)) {
  console.error("pruned output should contain marker");
  process.exit(1);
}

console.log("compaction prune verification passed");