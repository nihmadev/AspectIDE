import { isRecord, optionalPositiveNumberArg, stringArg, truncateText, type UnknownRecord } from "./aiRuntimeShared";

export type RuntimePatchOperation = {
  action: string;
  path: string;
  text?: string;
  oldText?: string;
  newText?: string;
  expectedReplacements?: number;
  overwrite?: boolean;
};

export function patchOperationsArg(args: UnknownRecord) {
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

export function patchOperationCounts(operations: RuntimePatchOperation[]) {
  return operations.reduce((counts, operation) => {
    if (operation.action === "create") counts.create += 1;
    else if (operation.action === "rewrite") counts.rewrite += 1;
    else if (operation.action === "replace") counts.replace += 1;
    else if (operation.action === "delete") counts.delete += 1;
    return counts;
  }, { create: 0, rewrite: 0, replace: 0, delete: 0 });
}

export function buildPatchPreview(operations: RuntimePatchOperation[]) {
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

export function buildReplacementPreview(oldText: string, newText: string) {
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

export function buildNumberedPreview(text: string, maxLines: number) {
  const lines = text.split(/\r?\n/);
  const visible = lines.slice(0, maxLines).map((line, index) => `${String(index + 1).padStart(3, " ")} | ${line}`);
  if (lines.length > maxLines) visible.push(`... ${lines.length - maxLines} more line${lines.length - maxLines === 1 ? "" : "s"}`);
  return visible.join("\n");
}

export function countLines(text: string) {
  if (!text) return 0;
  return text.split(/\r?\n/).length;
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
