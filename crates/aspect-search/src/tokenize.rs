const STOP_WORDS: &[&str] = &[
    "about", "after", "also", "and", "any", "are", "bug", "can", "code",
    "create", "default", "edit", "file", "files", "fix", "for", "from", "get",
    "has", "have", "into", "make", "need", "new", "not", "now", "please", "set",
    "that", "the", "this", "tool", "tools", "use", "with", "work",
];

const SHORT_USEFUL: &[&str] = &["ai", "api", "ci", "db", "fs", "gh", "ui", "ux"];

pub fn tokenize(query: &str) -> Vec<String> {
    let mut spaced = String::with_capacity(query.len() + 8);
    let chars: Vec<char> = query.chars().collect();
    for (index, ch) in chars.iter().enumerate() {
        if index > 0 {
            let prev = chars[index - 1];
            if (prev.is_ascii_lowercase() || prev.is_ascii_digit()) && ch.is_ascii_uppercase() {
                spaced.push(' ');
            }
        }
        spaced.push(*ch);
    }
    let lowered = spaced.to_lowercase();

    let mut seen: Vec<String> = Vec::new();
    for raw in lowered.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
        let token = raw.trim_matches(|c| c == '-' || c == '_');
        if token.is_empty() {
            continue;
        }
        let owned = token.to_string();
        if owned.len() < 3 && !SHORT_USEFUL.contains(&owned.as_str()) {
            continue;
        }
        if STOP_WORDS.contains(&owned.as_str()) {
            continue;
        }
        if !seen.iter().any(|t| t == &owned) {
            seen.push(owned);
        }
        if seen.len() >= 12 {
            break;
        }
    }
    seen
}
