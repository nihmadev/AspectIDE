/// Opening / closing inline-thinking tags recognized in streamed `content`.
pub const THINK_OPEN_TAGS: [&str; 2] = ["<think>", "<thinking>"];
pub const THINK_CLOSE_TAGS: [&str; 2] = ["</think>", "</thinking>"];

/// First (lowest-index) occurrence of any `tags` entry in `haystack`,
/// matched case-insensitively. Returns `(byte_index, tag_len)`.
pub fn find_tag(haystack: &str, tags: &[&str]) -> Option<(usize, usize)> {
    let lower = haystack.to_ascii_lowercase();
    tags.iter()
        .filter_map(|tag| lower.find(tag).map(|index| (index, tag.len())))
        .min_by_key(|(index, _)| *index)
}

/// `Some(tag_len)` when `text` starts with one of `tags` (case-insensitive).
pub fn prefix_tag(text: &str, tags: &[&str]) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    tags.iter()
        .find(|tag| lower.starts_with(**tag))
        .map(|tag| tag.len())
}

/// True when `text` is a non-empty proper prefix of some tag.
pub fn is_tag_prefix(text: &str, tags: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    !lower.is_empty()
        && tags
            .iter()
            .any(|tag| lower.len() < tag.len() && tag.starts_with(&lower))
}

/// Length of the longest suffix of `s` that is a prefix of some tag.
pub fn partial_tag_tail(s: &str, tags: &[&str]) -> usize {
    let lower = s.to_ascii_lowercase();
    let n = lower.len();
    let max = tags
        .iter()
        .map(|t| t.len())
        .max()
        .unwrap_or(0)
        .saturating_sub(1)
        .min(n);
    for k in (1..=max).rev() {
        if !s.is_char_boundary(n - k) {
            continue;
        }
        let suffix = &lower[n - k..];
        if tags.iter().any(|tag| tag.starts_with(suffix)) {
            return k;
        }
    }
    0
}
