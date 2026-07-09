use serde::Serialize;

const SECRET_MARKERS: &[(&str, &str)] = &[
    ("-----BEGIN OPENSSH", "OpenSSH private key"),
    ("-----BEGIN", "PEM private key block"),
    ("AKIA", "AWS access key id"),
    ("aws_secret_access_key", "AWS secret access key"),
    ("github_pat_", "GitHub fine-grained PAT"),
    ("ghp_", "GitHub personal access token"),
    ("xoxb-", "Slack bot token"),
    ("xoxp-", "Slack user token"),
    ("sk-", "OpenAI-style secret key"),
    ("AIza", "Google API key"),
];

#[derive(Serialize)]
pub struct SecretFinding {
    pub marker: String,
    pub description: String,
    pub line: usize,
}

const fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '+' | '=' | '.')
}

pub fn scan_secrets(
    text: &str,
    max_findings: usize,
    return_redacted: bool,
) -> (Vec<SecretFinding>, Option<String>) {
    let mut spans: Vec<(usize, usize, &'static str, &'static str)> = Vec::new();
    for (marker, description) in SECRET_MARKERS {
        let mut search_from = 0usize;
        while let Some(rel) = text.get(search_from..).and_then(|hay| hay.find(marker)) {
            let start = search_from + rel;
            let mut end = start + marker.len();
            for c in text[end..].chars() {
                if is_token_char(c) {
                    end += c.len_utf8();
                } else {
                    break;
                }
            }
            spans.push((start, end, marker, description));
            search_from = end;
        }
    }
    spans.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    let mut kept: Vec<(usize, usize, &'static str, &'static str)> = Vec::new();
    let mut covered_to = 0usize;
    for span in spans {
        if span.0 >= covered_to {
            covered_to = span.1;
            kept.push(span);
        }
    }

    let mut line_starts: Vec<usize> = vec![0];
    for (idx, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(idx + 1);
        }
    }
    let line_of = |byte_pos: usize| -> usize {
        line_starts
            .partition_point(|&start| start <= byte_pos)
            .max(1)
    };

    let findings: Vec<SecretFinding> = kept
        .iter()
        .take(max_findings)
        .map(|(start, _end, marker, description)| SecretFinding {
            marker: (*marker).to_string(),
            description: (*description).to_string(),
            line: line_of(*start),
        })
        .collect();

    let redacted = if return_redacted && !kept.is_empty() {
        let mut out = String::with_capacity(text.len());
        let mut cursor = 0usize;
        for (start, end, _marker, _description) in &kept {
            if *start >= cursor {
                out.push_str(&text[cursor..*start]);
                out.push_str("***REDACTED***");
                cursor = *end;
            }
        }
        out.push_str(&text[cursor..]);
        Some(out)
    } else {
        None
    };

    (findings, redacted)
}
