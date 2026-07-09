use crate::types::SecretFinding;

const SECRET_MARKERS: &[(&str, &str)] = &[
    ("AWS Access Key", "AKIA[0-9A-Z]{16}"),
    ("AWS Secret Key", "(?i)aws(.{0,20})?(?-i)['\"][0-9a-zA-Z/+]{40}['\"]"),
    ("GitHub Token", "(?i)gh[pousr]_[A-Za-z0-9_]{36,255}"),
    ("GitHub App Token", "(?i)(ghs|gho|ghu|ghs_|ghr_)[A-Za-z0-9_]{36,255}"),
    ("Slack Token", "(?i)xox[bpras]-[0-9a-zA-Z-]{10,200}"),
    ("Generic API Key", "(?i)(api[-_]?key|apikey|api_secret|api[-_]?token)['\"]?\\s*[:=]\\s*['\"][0-9a-zA-Z_\\-]{16,}['\"]"),
    ("JWT Token", "eyJ[a-zA-Z0-9_-]{10,}\\.[a-zA-Z0-9_-]{10,}\\.[a-zA-Z0-9_-]{10,}"),
    ("Private Key", "-----BEGIN (RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----"),
    ("Heroku API Key", "(?i)heroku[a-z0-9_\\-]{0,20}['\"]?\\s*[:=]\\s*['\"][0-9a-fA-F-]{36,45}['\"]"),
    ("Google OAuth", "[0-9]+-[0-9a-zA-Z_]{32}\\.apps\\.googleusercontent\\.com"),
    ("OpenAI API Key", "sk-[a-zA-Z0-9]{20,60}"),
    ("Anthropic API Key", "sk-ant-[a-zA-Z0-9]{20,60}"),
    ("Generic Password", "(?i)(password|pwd|passwd|secret|db_password|db_pass)['\"]?\\s*[:=]\\s*['\"][^'\"\\s]{8,}['\"]"),
    ("Connection String", "(?i)(mongodb|postgresql|mysql|redis)://[^\\s]{10,}"),
];

pub fn scan_secrets(
    text: &str,
    max_findings: usize,
    return_redacted: bool,
) -> (Vec<SecretFinding>, Option<String>) {
    let mut findings = Vec::new();
    for (pattern_name, pattern_str) in SECRET_MARKERS {
        if findings.len() >= max_findings {
            break;
        }
        let Ok(re) = regex::Regex::new(pattern_str) else {
            continue;
        };
        for cap in re.find_iter(text) {
            if findings.len() >= max_findings {
                break;
            }
            let line_num = text[..cap.start()].matches('\n').count() + 1;
            let line_start = text[..cap.start()].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let col = cap.start() - line_start + 1;
            let line_end = text[cap.start()..]
                .find('\n')
                .map(|i| cap.start() + i)
                .unwrap_or(text.len());
            let snippet = text[line_start..line_end].to_string();
            let redacted_str = if return_redacted {
                let matched = cap.as_str();
                let redacted: String = matched
                    .chars()
                    .enumerate()
                    .map(|(i, c)| {
                        if i < matched.len() / 3 || c == '/' || c == ':' || c == '_' || c == '-' {
                            c
                        } else {
                            '*'
                        }
                    })
                    .collect();
                redacted
            } else {
                String::new()
            };
            findings.push(SecretFinding {
                pattern: pattern_name.to_string(),
                line: line_num,
                column: col,
                snippet,
                redacted: redacted_str,
            });
        }
    }
    let redacted = if return_redacted && !findings.is_empty() {
        let mut redacted_text = text.to_string();
        for finding in &findings {
            if !finding.redacted.is_empty() {
                redacted_text = redacted_text.replace(&finding.snippet, &finding.redacted);
            }
        }
        Some(redacted_text)
    } else {
        None
    };
    (findings, redacted)
}
