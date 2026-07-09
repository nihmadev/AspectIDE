import { spawn } from "node:child_process";

const child = spawn(
  "pnpm",
  ["exec", "vite", "--host", "127.0.0.1", "--port", "5173", "--strictPort"],
  {
    env: { ...process.env, VITE_ASPECT_BROWSER_PREVIEW: "1" },
    shell: process.platform === "win32",
    stdio: "inherit",
  },
);

child.on("exit", (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  process.exit(code ?? 0);
});
