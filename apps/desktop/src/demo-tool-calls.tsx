import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AiToolCallsGroup, type AiMessage } from "./components/AiToolCall";
import "./styles/tokens.css";
import "./styles/ai-tool-calls.css";

// Demo messages with tool calls
const demoMessages: AiMessage[] = [
  {
    id: "1",
    role: "user",
    content: "Find all TypeScript files in the src directory",
    timestamp: Date.now() - 5000,
  },
  {
    id: "2",
    role: "assistant",
    content: "I'll search for TypeScript files in the src directory.",
    toolCalls: [
      {
        id: "tool-1",
        tool: "Glob",
        status: "success",
        input: "src/**/*.ts",
        output: "Found 42 files:\nsrc/main.ts\nsrc/App.tsx\nsrc/components/AiChatPanel.tsx\n...",
        startTime: Date.now() - 4500,
        endTime: Date.now() - 4200,
      },
    ],
    timestamp: Date.now() - 4000,
  },
  {
    id: "3",
    role: "user",
    content: "Add error handling to the API calls",
    timestamp: Date.now() - 3000,
  },
  {
    id: "4",
    role: "assistant",
    content: "I'll add try-catch blocks and error handling.",
    toolCalls: [
      {
        id: "tool-2",
        tool: "Read",
        status: "success",
        input: "src/api/client.ts",
        output: 'export async function fetchData() {\n  const response = await fetch("/api/data");\n  return response.json();\n}',
        startTime: Date.now() - 2800,
        endTime: Date.now() - 2700,
      },
      {
        id: "tool-3",
        tool: "StrReplace",
        status: "success",
        input: "src/api/client.ts",
        startTime: Date.now() - 2600,
        endTime: Date.now() - 2400,
        stats: {
          linesAdded: 8,
          linesRemoved: 3,
          filesChanged: 1,
        },
      },
    ],
    timestamp: Date.now() - 2000,
  },
  {
    id: "5",
    role: "user",
    content: "Create a new component for the settings page",
    timestamp: Date.now() - 1500,
  },
  {
    id: "6",
    role: "assistant",
    content: "Creating the SettingsPage component...",
    toolCalls: [
      {
        id: "tool-4",
        tool: "Write",
        status: "success",
        input: "src/components/SettingsPage.tsx",
        startTime: Date.now() - 1200,
        endTime: Date.now() - 900,
        stats: {
          linesAdded: 45,
          filesCreated: 1,
        },
      },
    ],
    timestamp: Date.now() - 800,
  },
  {
    id: "7",
    role: "user",
    content: "Search for all console.log statements and remove them",
    timestamp: Date.now() - 500,
  },
  {
    id: "8",
    role: "assistant",
    content: "Searching for console.log statements...",
    toolCalls: [
      {
        id: "tool-5",
        tool: "Grep",
        status: "running",
        input: "console\\.log",
        startTime: Date.now() - 200,
      },
    ],
    timestamp: Date.now() - 200,
  },
  {
    id: "9",
    role: "assistant",
    content: "I need approval before changing files.",
    toolCalls: [
      {
        id: "tool-6",
        tool: "StrReplace",
        status: "approval",
        input: "src/api/client.ts",
        startTime: Date.now() - 100,
        approval: {
          id: "approval-demo",
          tool: "StrReplace",
          title: "Approve exact text replacement",
          path: "src/api/client.ts",
          summary: "Replace 1 occurrence in src/api/client.ts on disk.",
          preview: "-   1 | return response.json();\n+   1 | return await response.json();",
          risk: "modify",
          approveLabel: "Apply edit",
          rejectLabel: "Reject",
        },
      },
    ],
    timestamp: Date.now() - 100,
  },
  {
    id: "10",
    role: "assistant",
    content: "I can run the test command after approval.",
    toolCalls: [
      {
        id: "tool-7",
        tool: "Shell",
        status: "approval",
        input: "pnpm --filter @lux/desktop typecheck",
        startTime: Date.now() - 80,
        approval: {
          id: "approval-shell-demo",
          tool: "Shell",
          title: "Approve shell command",
          path: ".",
          summary: "Run a non-interactive shell command in the workspace with a 120s timeout.",
          preview: "pnpm --filter @lux/desktop typecheck",
          risk: "execute",
          approveLabel: "Run command",
          rejectLabel: "Reject",
        },
      },
    ],
    timestamp: Date.now() - 80,
  },
];

function DemoApp() {
  return (
    <div style={{
      width: "100%",
      height: "100vh",
      background: "#181818",
      padding: "40px",
      overflow: "auto"
    }}>
      <div style={{
        maxWidth: "800px",
        margin: "0 auto",
        display: "grid",
        gap: "20px"
      }}>
        <div style={{
          color: "#e9e9e9",
          fontSize: "24px",
          fontWeight: "700",
          marginBottom: "20px"
        }}>
          AI Tool Calls Demo - Cursor Style
        </div>

        {demoMessages.map((message) => (
          <div key={message.id} style={{
            padding: "16px",
            borderRadius: "8px",
            background: message.role === "user" ? "#1e1e1e" : "#181818",
            border: "1px solid #2b2b2b",
          }}>
            <div style={{
              color: message.role === "user" ? "#3b9eff" : "#4ec98a",
              fontSize: "11px",
              fontWeight: "700",
              marginBottom: "8px",
              textTransform: "uppercase",
            }}>
              {message.role}
            </div>
            <div style={{
              color: "#cccccc",
              fontSize: "13px",
              lineHeight: "1.5",
              marginBottom: message.toolCalls ? "12px" : "0",
            }}>
              {message.content}
            </div>
            {message.toolCalls && (
              <AiToolCallsGroup toolCalls={message.toolCalls} />
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <DemoApp />
    </StrictMode>
  );
}
