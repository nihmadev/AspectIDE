#![allow(dead_code)]
//! Dependency-free failure-log analysis for the `FailureAnalyzer` tool.
//!
//! Turns raw failing output (Python tracebacks, Rust panics and compiler
//! diagnostics, Node stack traces, tsc/pytest/jest output, exit codes) into a
//! structured analysis: the error class + message, the extracted assertion,
//! and ranked root-cause candidates mapped to WORKSPACE files — so the tool
//! adds real signal instead of echoing the log back at the model.
//!
//! Hand-rolled line scanning on `std` only (`src-tauri` has no regex crate);
//! every parser is a small state machine over `lines()`.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One ranked root-cause candidate: a source location the failure points at.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureCandidate {
    pub path: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// True when the path resolves inside the workspace (vendor/stdlib frames rank far lower).
    pub in_workspace: bool,
    pub exists: bool,
    pub score: i32,
    pub reason: String,
    /// The log line this candidate was extracted from.
    pub evidence: String,
}

/// An extracted assertion (expected vs actual), when the log carries one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssertionInfo {
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureAnalysis {
    pub error_class: Option<String>,
    pub error_message: Option<String>,
    pub assertion: Option<AssertionInfo>,
    pub exit_codes: Vec<i32>,
    /// Root-cause candidates, best first.
    pub candidates: Vec<FailureCandidate>,
    pub next_actions: Vec<String>,
    pub summary: String,
}

/// Path fragments that mark non-workspace code: frames there explain the crash
/// site, not the root cause the model should edit.
const VENDOR_FRAGMENTS: &[&str] = &[
    "site-packages",
    "node_modules",
    "/usr/lib/",
    "lib/python",
    ".cargo/registry",
    ".rustup/",
    "/std/src/",
    "internal/modules",
    "internal/process",
];

fn is_vendor_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_lowercase();
    VENDOR_FRAGMENTS
        .iter()
        .any(|fragment| normalized.contains(fragment))
}

/// Resolve a logged path against the workspace: absolute paths are checked
/// directly, relative ones against the root.
fn resolve_logged_path(raw: &str, root: Option<&Path>) -> (bool, bool, String) {
    let cleaned = raw.trim().trim_matches('"');
    let path = PathBuf::from(cleaned);
    let (candidate, display) = if path.is_absolute() {
        (path.clone(), cleaned.to_string())
    } else if let Some(root) = root {
        (root.join(&path), cleaned.to_string())
    } else {
        (path.clone(), cleaned.to_string())
    };
    let exists = candidate.exists();
    let in_workspace = match root {
        Some(root) => candidate.starts_with(root) || path.is_relative(),
        None => path.is_relative(),
    };
    (in_workspace, exists, display.replace('\\', "/"))
}

/// Unscored candidate extracted by a parser, before workspace/vendor weighting.
struct CandidateSeed<'a> {
    raw_path: &'a str,
    line: Option<u32>,
    column: Option<u32>,
    base_score: i32,
    reason: &'a str,
    evidence: &'a str,
}

fn push_candidate(out: &mut Vec<FailureCandidate>, seed: &CandidateSeed<'_>, root: Option<&Path>) {
    if seed.raw_path.trim().is_empty() || seed.raw_path.starts_with('<') {
        return; // "<string>", "<module>", "<anonymous>"
    }
    let vendor = is_vendor_path(seed.raw_path);
    let (in_workspace, exists, display) = resolve_logged_path(seed.raw_path, root);
    let mut score = seed.base_score;
    if vendor {
        score -= 60;
    }
    if in_workspace {
        score += 30;
    }
    if exists {
        score += 20;
    }
    out.push(FailureCandidate {
        path: display,
        line: seed.line,
        column: seed.column,
        in_workspace: in_workspace && !vendor,
        exists,
        score,
        reason: seed.reason.to_string(),
        evidence: seed.evidence.trim().to_string(),
    });
}

/// `  File "app/main.py", line 42, in handler` → (path, line)
fn parse_python_frame(line: &str) -> Option<(String, u32)> {
    let rest = line.trim_start().strip_prefix("File \"")?;
    let (path, tail) = rest.split_once('"')?;
    let tail = tail.strip_prefix(", line ")?;
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    Some((path.to_string(), digits.parse().ok()?))
}

/// `path:12:34` / `path:12` suffix parse (Rust panic + generic `at path:line:col`).
fn parse_path_line_col(token: &str) -> Option<(String, u32, Option<u32>)> {
    let token = token.trim().trim_end_matches([':', ',', ')']);
    let mut parts = token.rsplitn(3, ':');
    let last = parts.next()?;
    let middle = parts.next()?;
    let rest = parts.next();
    // path:line:col
    if let (Ok(column), Ok(line)) = (last.parse::<u32>(), middle.parse::<u32>()) {
        if let Some(path) = rest {
            if path.len() > 1 {
                return Some((path.to_string(), line, Some(column)));
            }
        }
    }
    // path:line — re-split with two segments so a drive letter survives.
    let (path, last) = token.rsplit_once(':')?;
    if let Ok(line) = last.parse::<u32>() {
        if path.len() > 1 {
            return Some((path.to_string(), line, None));
        }
    }
    None
}

/// `path(12,5)` — the tsc location form.
fn parse_tsc_location(token: &str) -> Option<(String, u32, Option<u32>)> {
    let open = token.rfind('(')?;
    let close = token[open..].find(')')? + open;
    let (line_text, column_text) = token[open + 1..close].split_once(',')?;
    let line = line_text.trim().parse().ok()?;
    let column = column_text.trim().parse().ok();
    Some((token[..open].to_string(), line, column))
}

fn first_int_after<'a>(line: &'a str, markers: &[&str]) -> Option<(i32, &'a str)> {
    for marker in markers {
        if let Some(index) = line.find(marker) {
            let tail = &line[index + marker.len()..];
            let digits: String = tail
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            if let Ok(code) = digits.parse::<i32>() {
                return Some((code, line));
            }
        }
    }
    None
}

/// Analyze raw failing output into a structured report. `max_candidates` bounds
/// the ranked list (the full frame set of a deep traceback is noise).
#[must_use]
pub fn analyze(log: &str, root: Option<&Path>, max_candidates: usize) -> FailureAnalysis {
    let lines: Vec<&str> = log.lines().collect();
    let mut candidates: Vec<FailureCandidate> = Vec::new();
    let mut error_class: Option<String> = None;
    let mut error_message: Option<String> = None;
    let mut assertion: Option<AssertionInfo> = None;
    let mut exit_codes: Vec<i32> = Vec::new();
    let mut expected_pending: Option<String> = None;

    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();

        // ── Python traceback ──
        if trimmed.starts_with("Traceback (most recent call last):") {
            let mut frame_rank = 0i32;
            let mut cursor = index + 1;
            while cursor < lines.len() {
                if let Some((path, frame_line)) = parse_python_frame(lines[cursor]) {
                    // Later frames are closer to the raise site → rank higher.
                    frame_rank += 1;
                    push_candidate(
                        &mut candidates,
                        &CandidateSeed {
                            raw_path: &path,
                            line: Some(frame_line),
                            column: None,
                            base_score: 40 + frame_rank * 5,
                            reason: "python traceback frame (deepest frames closest to the raise)",
                            evidence: lines[cursor],
                        },
                        root,
                    );
                    cursor += 1;
                    // Skip the source-echo line under the frame if present.
                    if cursor < lines.len()
                        && lines[cursor].starts_with("    ")
                        && parse_python_frame(lines[cursor]).is_none()
                    {
                        cursor += 1;
                    }
                    continue;
                }
                let tail = lines[cursor].trim();
                if !tail.is_empty() && !tail.starts_with("File ") {
                    // `ValueError: invalid literal ...`
                    if let Some((class, message)) = tail.split_once(':') {
                        if class
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
                            && !class.is_empty()
                        {
                            error_class = Some(class.trim().to_string());
                            error_message = Some(message.trim().to_string());
                        }
                    } else if tail.ends_with("Error") || tail.ends_with("Exception") {
                        error_class = Some(tail.to_string());
                    }
                    break;
                }
                cursor += 1;
            }
            index = cursor + 1;
            continue;
        }

        // ── Rust panic (new + old formats) ──
        if let Some(at_index) = trimmed.find("panicked at ") {
            let tail = &trimmed[at_index + "panicked at ".len()..];
            if let Some((path, panic_line, column)) = parse_path_line_col(tail) {
                // New format: `panicked at src/x.rs:12:34:` with the message on
                // the following line(s).
                push_candidate(
                    &mut candidates,
                    &CandidateSeed {
                        raw_path: &path,
                        line: Some(panic_line),
                        column,
                        base_score: 90,
                        reason: "rust panic location",
                        evidence: line,
                    },
                    root,
                );
                error_class.get_or_insert_with(|| "panic".to_string());
                if let Some(message_line) = lines.get(index + 1) {
                    let message = message_line.trim();
                    if !message.is_empty() {
                        error_message.get_or_insert_with(|| message.to_string());
                    }
                }
            } else if let Some(rest) = tail.strip_prefix('\'') {
                // Old format: `panicked at 'message', src/x.rs:12:34`
                if let Some((message, location)) = rest.split_once("', ") {
                    error_class.get_or_insert_with(|| "panic".to_string());
                    error_message.get_or_insert_with(|| message.to_string());
                    if let Some((path, panic_line, column)) = parse_path_line_col(location) {
                        push_candidate(
                            &mut candidates,
                            &CandidateSeed {
                                raw_path: &path,
                                line: Some(panic_line),
                                column,
                                base_score: 90,
                                reason: "rust panic location",
                                evidence: line,
                            },
                            root,
                        );
                    }
                }
            }
        }

        // ── Rust assert_eq! payload ──
        if let Some(value) = trimmed.strip_prefix("left: ") {
            expected_pending = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("right: ") {
            if let Some(left) = expected_pending.take() {
                assertion.get_or_insert_with(|| AssertionInfo {
                    expected: Some(value.trim().to_string()),
                    actual: Some(left),
                    raw: "assert_eq! left/right".to_string(),
                });
            }
        }

        // ── Rust/cargo compiler diagnostics: `error[E0308]: ...` + ` --> path:l:c` ──
        if trimmed.starts_with("error[") || trimmed.starts_with("error:") {
            if error_class.is_none() {
                error_class = Some("compile error".to_string());
                error_message = Some(
                    trimmed
                        .split_once(':')
                        .map_or(trimmed, |(_, m)| m.trim())
                        .to_string(),
                );
            }
            if let Some(location_line) = lines.get(index + 1) {
                if let Some(rest) = location_line.trim().strip_prefix("--> ") {
                    if let Some((path, diag_line, column)) = parse_path_line_col(rest) {
                        push_candidate(
                            &mut candidates,
                            &CandidateSeed {
                                raw_path: &path,
                                line: Some(diag_line),
                                column,
                                base_score: 80,
                                reason: "compiler diagnostic location",
                                evidence: location_line,
                            },
                            root,
                        );
                    }
                }
            }
        }

        // ── tsc: `path(12,5): error TS2345: msg` ──
        if let Some(error_index) = trimmed.find(": error TS") {
            if let Some((path, diag_line, column)) = parse_tsc_location(&trimmed[..error_index]) {
                push_candidate(
                    &mut candidates,
                    &CandidateSeed {
                        raw_path: &path,
                        line: Some(diag_line),
                        column,
                        base_score: 80,
                        reason: "tsc diagnostic",
                        evidence: line,
                    },
                    root,
                );
                error_class.get_or_insert_with(|| "typescript error".to_string());
                error_message.get_or_insert_with(|| trimmed[error_index + 2..].trim().to_string());
            }
        }

        // ── Node/JS stack frame: `at fn (path:12:34)` / `at path:12:34` ──
        if let Some(after_at) = trimmed.strip_prefix("at ") {
            let frame = after_at.trim();
            let token = frame
                .rfind('(')
                .map_or(frame, |open| frame[open + 1..].trim_end_matches(')'));
            if let Some((path, frame_line, column)) = parse_path_line_col(token) {
                if path.contains('/') || path.contains('\\') || path.contains('.') {
                    push_candidate(
                        &mut candidates,
                        &CandidateSeed {
                            raw_path: &path,
                            line: Some(frame_line),
                            column,
                            base_score: 50,
                            reason: "js/node stack frame (top frames closest to the throw)",
                            evidence: line,
                        },
                        root,
                    );
                }
            }
        } else if trimmed.contains("Error:") && error_class.is_none() && !trimmed.starts_with('{') {
            // `TypeError: x is not a function` — head of a JS stack.
            if let Some((class, message)) = trimmed.split_once(':') {
                let class = class.trim();
                if class.ends_with("Error") && class.chars().all(|c| c.is_ascii_alphanumeric()) {
                    error_class = Some(class.to_string());
                    error_message = Some(message.trim().to_string());
                }
            }
        }

        // ── pytest assertion / jest expected-received ──
        if let Some(rest) = trimmed.strip_prefix("E ") {
            let rest = rest.trim();
            if rest.starts_with("assert") && assertion.is_none() {
                assertion = Some(AssertionInfo {
                    expected: None,
                    actual: None,
                    raw: rest.to_string(),
                });
            }
        }
        if let Some(value) = trimmed.strip_prefix("Expected:") {
            expected_pending = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("Received:") {
            if assertion.is_none() {
                assertion = Some(AssertionInfo {
                    expected: expected_pending.take(),
                    actual: Some(value.trim().to_string()),
                    raw: "jest Expected/Received".to_string(),
                });
            }
        }

        // ── Exit codes ──
        if let Some((code, _)) = first_int_after(
            trimmed,
            &[
                "exit code:",
                "exit code",
                "exited with code",
                "process exited with",
                "Process completed with exit code",
            ],
        ) {
            if code != 0 && !exit_codes.contains(&code) {
                exit_codes.push(code);
            }
        }

        index += 1;
    }

    // Rank: score desc, then workspace-first, then stable order.
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.in_workspace.cmp(&a.in_workspace))
    });
    candidates.dedup_by(|a, b| a.path == b.path && a.line == b.line);
    candidates.truncate(max_candidates);

    let mut next_actions: Vec<String> = Vec::new();
    if let Some(best) = candidates.iter().find(|c| c.in_workspace) {
        next_actions.push(format!(
            "Read {}{} — the highest-ranked workspace location this failure points at.",
            best.path,
            best.line.map(|l| format!(":{l}")).unwrap_or_default()
        ));
    }
    if assertion.is_some() {
        next_actions.push(
            "Compare the extracted expected/actual values against the assertion site before changing code.".to_string(),
        );
    }
    if candidates.iter().all(|c| !c.in_workspace) && !candidates.is_empty() {
        next_actions.push(
            "Every frame resolves outside the workspace (vendor/stdlib) — the root cause is likely in how workspace code CALLS into it; search the workspace for the deepest vendor API named in the log.".to_string(),
        );
    }
    if candidates.is_empty() && error_class.is_none() {
        next_actions.push(
            "No recognizable traceback/panic/diagnostic shape found — treat the log as free text and search the workspace for its distinctive phrases.".to_string(),
        );
    }

    let summary = match (&error_class, candidates.first()) {
        (Some(class), Some(best)) => format!(
            "{class}{}: strongest candidate {}{}",
            error_message
                .as_deref()
                .map(|m| format!(" — {m}"))
                .unwrap_or_default(),
            best.path,
            best.line.map(|l| format!(":{l}")).unwrap_or_default()
        ),
        (Some(class), None) => format!(
            "{class}{} (no file locations found in the log)",
            error_message
                .as_deref()
                .map(|m| format!(" — {m}"))
                .unwrap_or_default()
        ),
        (None, Some(best)) => format!(
            "failure points at {}{}",
            best.path,
            best.line.map(|l| format!(":{l}")).unwrap_or_default()
        ),
        (None, None) => "no structured failure shape recognized in the log".to_string(),
    };

    FailureAnalysis {
        error_class,
        error_message,
        assertion,
        exit_codes,
        candidates,
        next_actions,
        summary,
    }
}

