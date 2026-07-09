use std::collections::HashSet;

use super::normalize::normalize_tool_name;

pub fn closest_tool_names(target: &str, candidates: &HashSet<String>) -> Vec<String> {
    let t = normalize_tool_name(target);
    if t.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .filter_map(|c| {
            let n = normalize_tool_name(c);
            let score = if n == t {
                1000
            } else if n.contains(&t) || t.contains(&n) {
                500_usize.saturating_sub(n.len().abs_diff(t.len()))
            } else {
                let prefix = n.bytes().zip(t.bytes()).take_while(|(a, b)| a == b).count();
                if prefix >= 4 {
                    prefix * 10
                } else {
                    0
                }
            };
            (score > 0).then_some((score, c))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(3).map(|(_, c)| c.clone()).collect()
}
