//! agent-browser version string parsing and comparison.

pub fn normalize_agent_browser_version(raw: &str) -> String {
    raw.trim()
        .strip_prefix("agent-browser")
        .unwrap_or(raw)
        .trim()
        .to_string()
}

pub fn parse_version_parts(version: &str) -> Option<(u32, u32, u32)> {
    let core = version.split('+').next()?.split('-').next()?.trim();
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

pub fn version_is_older(current: &str, latest: &str) -> bool {
    match (parse_version_parts(current), parse_version_parts(latest)) {
        (Some(current_parts), Some(latest_parts)) => current_parts < latest_parts,
        _ => current != latest,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_version_parts, version_is_older};

    #[test]
    fn version_parsing_orders_correctly() {
        assert_eq!(parse_version_parts("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version_parts("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version_parts("1"), Some((1, 0, 0)));
        assert!(parse_version_parts("").is_none());
        assert!(version_is_older("1.0.0", "1.0.1"));
        assert!(version_is_older("1.0.0", "2.0.0"));
        assert!(!version_is_older("1.0.1", "1.0.0"));
        assert!(!version_is_older("1.0.0", "1.0.0"));
    }
}
