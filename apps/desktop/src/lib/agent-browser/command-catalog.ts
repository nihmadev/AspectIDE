/** Read-only agent-browser subcommands (no approval in Default mode). */
export const READONLY_BROWSER_COMMANDS = new Set([
  "snapshot",
  "get",
  "is",
  "session",
  "stream",
  "skills",
  "console",
  "errors",
  "tab",
  "diff",
  "network",
  "state",
  "profiles",
  "doctor",
  "dashboard",
  "vitals",
  "react",
  "wait",
  "help",
]);

export function isReadOnlyBrowserArgs(args: string[]) {
  if (args.length === 0) return false;
  const head = args[0]?.trim().toLowerCase();
  if (!head) return false;
  if (head === "--help" || head === "-h") return true;
  return READONLY_BROWSER_COMMANDS.has(head);
}

export function formatBrowserCommandPreview(args: string[]) {
  return args.join(" ").trim();
}