const taskSignalPattern =
  /\b(fix|add|create|implement|build|refactor|debug|test|review|deploy|run|write|patch|delete|ship|demo|аквариум|игр|фикс|сделай|добав|создай|исправ|реализ|проверь|напиш|удали|собери|запусти|улучш|почини|баг|ошибк)\b/i;

const greetingPrefixes = [
  "привет",
  "прив",
  "здаров",
  "здорово",
  "хай",
  "hello",
  "hi",
  "hey",
  "yo",
  "sup",
  "howdy",
  "салют",
  "хелло",
  "добрый",
  "доброе",
  "добрая",
  "доброй",
  "спасибо",
  "thanks",
  "thx",
  "пасиб",
  "благодар",
];

/** Greeting / small-talk without an engineering task — Automatic should not mandate tools. */
export function isAutomaticSocialOnlyMessage(message: string): boolean {
  const trimmed = message.trim();
  if (!trimmed || trimmed.length > 120) return false;
  if (taskSignalPattern.test(trimmed)) return false;
  if (trimmed.includes("?") && trimmed.length > 24) return false;
  if (trimmed.includes("/")) return false;
  const normalized = trimmed.toLowerCase().replace(/[!.,]+$/u, "");
  const words = normalized.split(/\s+/).filter(Boolean);
  if (words.length > 6) return false;
  return greetingPrefixes.some((prefix) => normalized === prefix || normalized.startsWith(`${prefix} `));
}