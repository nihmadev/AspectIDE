//! Passage selection: pick the most query-relevant passage from a page instead
//! of blindly taking the head.

use std::collections::HashSet;

use super::util::{take_chars, term_counts, trim_to_chars};

/// Cap on characters scanned when selecting the best citation passage.
const MAX_PASSAGE_SCAN_CHARS: usize = 120_000;

/// Pick the most query-relevant passage of `content` (up to `max_chars`) instead
/// of blindly taking the head of the page.
pub(crate) fn best_passage(content: &str, query_terms: &HashSet<String>, max_chars: usize) -> String {
    let trimmed = content.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    if query_terms.is_empty() {
        return trim_to_chars(trimmed, max_chars);
    }
    let scan = take_chars(trimmed, MAX_PASSAGE_SCAN_CHARS);
    let chunks = split_chunks(scan);
    if chunks.is_empty() {
        return trim_to_chars(trimmed, max_chars);
    }
    let scores: Vec<usize> = chunks
        .iter()
        .map(|chunk| chunk_score(chunk, query_terms))
        .collect();
    let best = scores
        .iter()
        .enumerate()
        .max_by_key(|(index, score)| (**score, std::cmp::Reverse(*index)))
        .map_or(0, |(index, _)| index);
    if scores[best] == 0 {
        return trim_to_chars(trimmed, max_chars);
    }

    let mut start = best;
    let mut end = best;
    let mut total = chunks[best].chars().count();
    loop {
        let can_next =
            end + 1 < chunks.len() && total + 1 + chunks[end + 1].chars().count() <= max_chars;
        let can_prev = start > 0 && total + 1 + chunks[start - 1].chars().count() <= max_chars;
        match (can_next, can_prev) {
            (true, true) => {
                if scores[end + 1] >= scores[start - 1] {
                    end += 1;
                    total += 1 + chunks[end].chars().count();
                } else {
                    start -= 1;
                    total += 1 + chunks[start].chars().count();
                }
            }
            (true, false) => {
                end += 1;
                total += 1 + chunks[end].chars().count();
            }
            (false, true) => {
                start -= 1;
                total += 1 + chunks[start].chars().count();
            }
            (false, false) => break,
        }
    }

    let mut passage = chunks[start..=end].join(" ");
    if passage.chars().count() > max_chars {
        passage = trim_around_terms(&passage, query_terms, max_chars);
    } else if end + 1 < chunks.len() || scan.len() < trimmed.len() {
        passage.push('…');
    }
    if start > 0 && !passage.starts_with('…') {
        passage.insert(0, '…');
    }
    passage
}

/// Trim `text` to `max_chars` centered on the first query-term occurrence.
fn trim_around_terms(text: &str, query_terms: &HashSet<String>, max_chars: usize) -> String {
    let lower = text.to_lowercase();
    let mut earliest: Option<usize> = None;
    for term in query_terms {
        let mut from = 0_usize;
        while let Some(relative) = lower[from..].find(term.as_str()) {
            let at = from + relative;
            let before_ok = at == 0
                || !lower[..at]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
            let after = at + term.len();
            let after_ok = after >= lower.len()
                || !lower[after..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric);
            if before_ok && after_ok {
                earliest = Some(earliest.map_or(at, |current| current.min(at)));
                break;
            }
            from = after;
        }
    }
    let Some(term_byte) = earliest else {
        return trim_to_chars(text, max_chars);
    };
    let term_char = lower[..term_byte].chars().count();
    let window_start_char = term_char.saturating_sub(max_chars / 4);
    if window_start_char == 0 {
        return trim_to_chars(text, max_chars);
    }
    let start_byte = text
        .char_indices()
        .nth(window_start_char)
        .map_or(0, |(byte, _)| byte);
    let snapped = text[start_byte..]
        .find(char::is_whitespace)
        .map_or(start_byte, |relative| start_byte + relative);
    let tail = text[snapped..].trim_start();
    format!("…{}", trim_to_chars(tail, max_chars.saturating_sub(1)))
}

/// Split into paragraph chunks; long paragraphs split on sentence boundaries.
fn split_chunks(text: &str) -> Vec<&str> {
    const TARGET_CHARS: usize = 400;
    let mut chunks = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        if paragraph.chars().count() <= TARGET_CHARS {
            chunks.push(paragraph);
            continue;
        }
        let mut start_byte = 0_usize;
        let mut cursor = 0_usize;
        let mut length_chars = 0_usize;
        for piece in paragraph.split_inclusive(". ") {
            let piece_chars = piece.chars().count();
            if length_chars > 0 && length_chars + piece_chars > TARGET_CHARS {
                let chunk = paragraph[start_byte..cursor].trim();
                if !chunk.is_empty() {
                    chunks.push(chunk);
                }
                start_byte = cursor;
                length_chars = 0;
            }
            cursor += piece.len();
            length_chars += piece_chars;
        }
        if start_byte < paragraph.len() {
            let tail = paragraph[start_byte..].trim();
            if !tail.is_empty() {
                chunks.push(tail);
            }
        }
    }
    chunks
}

/// Query-term density of one chunk: unique matched terms weighted over raw
/// occurrences.
fn chunk_score(chunk: &str, query_terms: &HashSet<String>) -> usize {
    let lower = chunk.to_lowercase();
    let counts = term_counts(&lower);
    let mut unique = 0_usize;
    let mut occurrences = 0_usize;
    for term in query_terms {
        if let Some(count) = counts.get(term.as_str()) {
            unique += 1;
            occurrences += *count;
        }
    }
    unique * 3 + occurrences
}
