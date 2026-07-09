pub fn writes_to_block_device(normalized: &str) -> bool {
    normalized
        .split(' ')
        .any(|token| token.strip_prefix("of=").is_some_and(is_block_device_path))
}

pub fn redirects_to_block_device(normalized: &str) -> bool {
    let mut tokens = normalized.split(' ').peekable();
    while let Some(token) = tokens.next() {
        if token == ">" || token == ">>" {
            if let Some(next) = tokens.peek() {
                if is_block_device_path(next) {
                    return true;
                }
            }
        } else if let Some(rest) = token.strip_prefix(">>") {
            if is_block_device_path(rest) {
                return true;
            }
        } else if let Some(rest) = token.strip_prefix('>') {
            if is_block_device_path(rest) {
                return true;
            }
        }
    }
    false
}

fn is_block_device_path(path: &str) -> bool {
    let path = path.trim_matches(|c| c == '"' || c == '\'');
    path.starts_with("/dev/sd")
        || path.starts_with("/dev/nvme")
        || path.starts_with("/dev/hd")
        || path.starts_with("/dev/disk")
        || path.starts_with("/dev/vd")
        || path == "/dev/mem"
}

pub fn mentions_windows_drive_root(normalized: &str) -> bool {
    normalized.split([' ', '"']).any(|token| {
        let token = token.trim();
        let bare = token.trim_end_matches(['\\', '/']).trim_matches('"');
        if matches!(bare, "%systemdrive%" | "%systemroot%" | "%windir%") {
            return true;
        }
        let bytes = token.as_bytes();
        bytes.len() >= 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (token.len() == 2 || matches!(bytes.get(2), Some(b'\\' | b'/')))
    })
}
