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
