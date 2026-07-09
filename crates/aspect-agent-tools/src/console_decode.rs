pub fn decode_console_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => {
            #[cfg(windows)]
            {
                let (cp866, _, _) = encoding_rs::IBM866.decode(bytes);
                let (cp1251, _, _) = encoding_rs::WINDOWS_1251.decode(bytes);
                let best = if legacy_decode_score(&cp1251) >= legacy_decode_score(&cp866) {
                    cp1251
                } else {
                    cp866
                };
                best.into_owned()
            }
            #[cfg(not(windows))]
            {
                String::from_utf8_lossy(bytes).into_owned()
            }
        }
    }
}

#[cfg(windows)]
fn legacy_decode_score(text: &str) -> i64 {
    let mut score: i64 = 0;
    for ch in text.chars() {
        if ch.is_alphabetic() || ch.is_ascii_digit() || ch.is_ascii_whitespace() {
            score += 1;
        } else if ch == char::REPLACEMENT_CHARACTER
            || (ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t')
        {
            score -= 4;
        }
    }
    score
}
