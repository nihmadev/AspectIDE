import type { MessageKey } from "../../i18n";
import type { TranslateFn } from "../../i18n/useTranslation";
import {
  isGoalClearCommand,
  parseGoalRunFlagArgs,
  type GoalRunLimits,
} from "./../session/goal/run-limits";
import type { ProjectSlashCommand } from "./project-commands";

export type AiChatSlashCommandId =
  | "compact"
  | "clear"
  | "undo"
  | "help"
  | "goal"
  | "model"
  | "agent"
  | "settings"
  | "index";

export type AiChatSlashCommand = {
  id: AiChatSlashCommandId;
  name: string;
  descriptionKey: MessageKey;
  keywords: string[];
};

export const AI_CHAT_SLASH_COMMANDS: AiChatSlashCommand[] = [
  {
    id: "compact",
    name: "compact",
    descriptionKey: "aiChat.slash.compact.description",
    keywords: ["compress", "context", "summarize", "сжать", "компакт"],
  },
  {
    id: "clear",
    name: "clear",
    descriptionKey: "aiChat.slash.clear.description",
    keywords: ["reset", "new", "очистить"],
  },
  {
    id: "undo",
    name: "undo",
    descriptionKey: "aiChat.slash.undo.description",
    keywords: ["revert", "rollback", "отмена", "откат"],
  },
  {
    id: "help",
    name: "help",
    descriptionKey: "aiChat.slash.help.description",
    keywords: ["commands", "?", "помощь"],
  },
  {
    id: "goal",
    name: "goal",
    descriptionKey: "aiChat.slash.goal.description",
    keywords: ["pin", "objective", "target", "цель", "задача"],
  },
  {
    id: "model",
    name: "model",
    descriptionKey: "aiChat.slash.model.description",
    keywords: ["provider", "switch", "модель"],
  },
  {
    id: "agent",
    name: "agent",
    descriptionKey: "aiChat.slash.agent.description",
    keywords: ["profile", "mode", "агент"],
  },
  {
    id: "settings",
    name: "settings",
    descriptionKey: "aiChat.slash.settings.description",
    keywords: ["preferences", "config", "настройки"],
  },
  {
    id: "index",
    name: "index",
    descriptionKey: "aiChat.slash.index.description",
    keywords: ["indexing", "semantic", "индекс"],
  },
];

export type BuiltinSlashCommandMatch = AiChatSlashCommand & {
  kind: "builtin";
  score: number;
};

export type ProjectSlashCommandMatch = ProjectSlashCommand & {
  score: number;
};

export type SlashCommandMatch = BuiltinSlashCommandMatch | ProjectSlashCommandMatch;

export function parseSlashQuery(message: string) {
  const trimmed = message.trimStart();
  if (!trimmed.startsWith("/")) return null;
  const body = trimmed.slice(1);
  const space = body.search(/\s/);
  const commandPart = (space === -1 ? body : body.slice(0, space)).toLowerCase();
  const args = space === -1 ? "" : body.slice(space + 1).trim();
  return { commandPart, args, raw: trimmed };
}

function scoreSlashName(query: string, name: string, keywords: string[]) {
  if (name.startsWith(query)) return 120 - (name.length - query.length);
  if (name.includes(query)) return 80;
  if (keywords.some((keyword) => keyword.startsWith(query))) return 60;
  if (keywords.some((keyword) => keyword.includes(query))) return 40;
  return 0;
}

export function filterSlashCommands(
  message: string,
  t: TranslateFn,
  projectCommands: ProjectSlashCommand[] = [],
): SlashCommandMatch[] {
  const parsed = parseSlashQuery(message);
  if (!parsed) return [];
  const query = parsed.commandPart;
  if (!query) {
    const builtins = AI_CHAT_SLASH_COMMANDS.map((command) => ({ ...command, kind: "builtin" as const, score: 100 }));
    const projects = projectCommands.map((command) => ({ ...command, score: 90 }));
    return [...builtins, ...projects].sort((left, right) => right.score - left.score || left.name.localeCompare(right.name));
  }
  const matches: SlashCommandMatch[] = [];
  for (const command of AI_CHAT_SLASH_COMMANDS) {
    const score = scoreSlashName(query, command.name, command.keywords);
    if (score > 0) matches.push({ ...command, kind: "builtin", score });
  }
  for (const command of projectCommands) {
    const score = scoreSlashName(query, command.name, command.keywords);
    if (score > 0) matches.push({ ...command, score });
  }
  return matches.sort((left, right) => right.score - left.score || left.name.localeCompare(right.name));
}

export function slashCommandLabel(command: SlashCommandMatch, _t: TranslateFn) {
  return `/${command.name}`;
}

export function slashCommandDescription(command: SlashCommandMatch, t: TranslateFn) {
  if (command.kind === "project") return command.description;
  return t(command.descriptionKey);
}

export function isExactSlashCommand(message: string, commandId: AiChatSlashCommandId) {
  const parsed = parseSlashQuery(message);
  if (!parsed || parsed.args.trim()) return false;
  const command = AI_CHAT_SLASH_COMMANDS.find((entry) => entry.id === commandId);
  return command ? parsed.commandPart === command.name : false;
}

/** Composer text after picking a slash command from the menu (does not send). */
export function composerTextAfterSlashPick(
  commandId: AiChatSlashCommandId,
  currentMessage = "",
): string | null {
  const parsed = parseSlashQuery(currentMessage.trimStart());
  switch (commandId) {
    case "help":
      if (parsed?.commandPart === "help" && parsed.args.trim()) {
        return `/help ${parsed.args}`;
      }
      return "/help ";
    case "goal":
      if (parsed?.commandPart === "goal" && parsed.args.trim()) {
        return `/goal ${parsed.args}`;
      }
      return "/goal ";
    default:
      return null;
  }
}

export type GoalSlashCommand =
  | { kind: "set"; goal: string; limits: Partial<GoalRunLimits>; extraMessage?: string }
  | { kind: "clear" }
  | { kind: "incomplete" }
  | { kind: "status" }
  | { kind: "history" }
  | { kind: "pause" }
  | { kind: "resume" }
  | { kind: "flagError"; errors: string[] };

export function parseGoalSlashCommand(message: string): GoalSlashCommand | null {
  const parsed = parseSlashQuery(message.trim());
  if (!parsed || parsed.commandPart !== "goal") return null;
  const args = parsed.args.trim();
  if (!args) return { kind: "incomplete" };
  const lower = args.toLowerCase();
  if (isGoalClearCommand(args)) return { kind: "clear" };
  if (lower === "status") return { kind: "status" };
  if (lower === "history") return { kind: "history" };
  if (lower === "pause") return { kind: "pause" };
  if (lower === "resume") return { kind: "resume" };

  const flagged = parseGoalRunFlagArgs(args);
  if (flagged.errors.length > 0) {
    return { kind: "flagError", errors: flagged.errors };
  }

  const split = flagged.condition.split(/\s+::\s+/, 2);
  const goal = split[0]?.trim() ?? "";
  if (!goal) return null;
  const extraMessage = split[1]?.trim();
  return {
    kind: "set",
    goal: goal.slice(0, 2_000),
    limits: flagged.limits,
    extraMessage: extraMessage ? extraMessage.slice(0, 4_000) : undefined,
  };
}