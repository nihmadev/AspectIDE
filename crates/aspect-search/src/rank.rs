use aspect_core::SearchHit;

const LOW_VALUE_PATH_FRAGMENTS: &[&str] = &[
    "node_modules",
    "/target/",
    "/dist/",
    "/build/",
    "/out/",
    "/vendor/",
    "/.next/",
    ".min.",
    ".lock",
    "generated",
];

pub fn relevance_score(hit: &SearchHit, lower_query: &str) -> i64 {
    let mut score = 0_i64;
    let path = hit.path.to_string_lossy().to_lowercase().replace('\\', "/");

    if let Some(file_name) = hit.path.file_name().and_then(|name| name.to_str()) {
        let file_name = file_name.to_lowercase();
        if file_name == lower_query {
            score += 1_000;
        } else if file_name.contains(lower_query) {
            score += 400;
        }
    }
    if path.contains(lower_query) {
        score += 80;
    }
    if is_word_boundary_match(&hit.match_text.to_lowercase(), lower_query) {
        score += 120;
    }
    if is_low_value_path(&path) {
        score -= 300;
    }
    score -= i64::try_from(path.len() / 16).unwrap_or(i64::MAX);
    score -= i64::try_from(hit.line.min(10_000) / 200).unwrap_or(i64::MAX);
    score
}

fn is_word_boundary_match(text: &str, query: &str) -> bool {
    text == query
}

fn is_low_value_path(path: &str) -> bool {
    LOW_VALUE_PATH_FRAGMENTS
        .iter()
        .any(|fragment| path.contains(fragment))
}
