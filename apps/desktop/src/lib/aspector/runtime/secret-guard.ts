import { booleanArg, clamp, numberArg, readErrorMessage, stringArg, toolJson, truncateText, type ToolResult, type UnknownRecord } from "./shared";
import { luxCommands } from "./../../tauri/commands";

export type SecretSeverity = "low" | "medium" | "high" | "critical";

export type SecretFinding = {
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

export type SecretFindingInternal = SecretFinding & {
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

const secretPreviewMask = "[REDACTED]";

const secretPatterns: SecretPattern[] = [
  { kind: "openai-api-key", label: "OpenAI API key", severity: "critical", confidence: "high", regex: /\b(sk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,})\b/gd, secretGroup: 1 },
  { kind: "github-token", label: "GitHub token", severity: "critical", confidence: "high", regex: /\b((?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{30,})\b/gd, secretGroup: 1 },
  { kind: "github-fine-grained-token", label: "GitHub fine-grained token", severity: "critical", confidence: "high", regex: /\b(github_pat_[A-Za-z0-9_]{40,})\b/gd, secretGroup: 1 },
  { kind: "slack-token", label: "Slack token", severity: "critical", confidence: "high", regex: /\b(xox[baprs]-[A-Za-z0-9-]{20,})\b/gd, secretGroup: 1 },
  { kind: "aws-access-key", label: "AWS access key", severity: "critical", confidence: "high", regex: /\b((?:AKIA|ASIA)[A-Z0-9]{16})\b/gd, secretGroup: 1 },
  { kind: "google-api-key", label: "Google API key", severity: "critical", confidence: "high", regex: /\b(AIza[0-9A-Za-z_-]{35})\b/gd, secretGroup: 1 },
  { kind: "stripe-key", label: "Stripe key", severity: "critical", confidence: "high", regex: /\b((?:sk|rk)_(?:live|test)_[A-Za-z0-9]{20,})\b/gd, secretGroup: 1 },
  { kind: "jwt", label: "JWT", severity: "high", confidence: "medium", regex: /\b(eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,})\b/gd, secretGroup: 1 },
  { kind: "private-key-block", label: "Private key block", severity: "critical", confidence: "high", regex: /-----BEGIN (?:RSA |DSA |EC |OPENSSH |PGP )?PRIVATE KEY-----[\s\S]*?-----END (?:RSA |DSA |EC |OPENSSH |PGP )?PRIVATE KEY-----/gd },
  { kind: "connection-string-password", label: "Connection string password", severity: "high", confidence: "medium", regex: /\b((?:postgres|postgresql|mysql|mongodb(?:\+srv)?|redis):\/\/[^\s:@/]+:)([^\s@/]{8,})(@[^\s]*)/gid, secretGroup: 2 },
  { kind: "assigned-secret", label: "Assigned secret value", severity: "high", confidence: "medium", regex: /\b([A-Z0-9_.-]*(?:API[_-]?KEY|TOKEN|SECRET|PASSWORD|PASSWD|PRIVATE[_-]?KEY|AUTH|CREDENTIAL)[A-Z0-9_.-]*\s*[:=]\s*["']?)([^"'\s]{12,})/gid, secretGroup: 2, labelGroup: 1 },
  { kind: "bearer-token", label: "Bearer token", severity: "high", confidence: "medium", regex: /\b(Bearer\s+)([A-Za-z0-9._~+/=-]{20,})\b/gd, secretGroup: 2, labelGroup: 1 },
];

export async function secretGuard(args: UnknownRecord): Promise<ToolResult> {
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

export function scanSecrets(text: string, source: string) {
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
      const groupSpan = pattern.secretGroup
        ? (match as unknown as { indices?: Array<[number, number] | undefined> }).indices?.[pattern.secretGroup]
        : undefined;
      const start = groupSpan ? groupSpan[0] : matchIndex + Math.max(rawMatch.indexOf(secret), 0);
      const end = groupSpan ? groupSpan[1] : start + secret.length;
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

export function publicSecretFinding(finding: SecretFindingInternal): SecretFinding {
  const { start: _start, end: _end, replacement: _replacement, ...publicFinding } = finding;
  return publicFinding;
}

export function compareSecretFindings(left: SecretFindingInternal, right: SecretFindingInternal) {
  return secretSeverityRank(right.severity) - secretSeverityRank(left.severity) ||
    confidenceRank(right.confidence) - confidenceRank(left.confidence) ||
    left.source.localeCompare(right.source) ||
    left.line - right.line ||
    left.column - right.column;
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
