use crate::path::normalize_slashes;

pub struct PathFilter {
    fragments: Vec<String>,
}

impl PathFilter {
    pub fn new(filter: &str) -> Self {
        let fragments = filter
            .split(['*', '?', '[', ']', '{', '}'])
            .map(str::trim)
            .filter(|fragment| !fragment.is_empty() && *fragment != "/")
            .map(str::to_string)
            .collect();
        Self { fragments }
    }

    pub fn matches(&self, path_lower: &str) -> bool {
        let mut cursor = 0;
        for fragment in &self.fragments {
            let Some(found) = path_lower[cursor..].find(fragment.as_str()) else {
                return false;
            };
            cursor += found + fragment.len();
        }
        true
    }
}

pub fn passes_path_filter(path: &str, filter: Option<&PathFilter>) -> bool {
    filter.is_none_or(|f| f.matches(&normalize_slashes(path).to_lowercase()))
}
