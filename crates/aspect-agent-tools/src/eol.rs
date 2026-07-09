pub fn detect_eol(text: &str) -> &'static str {
    match text.find(['\n', '\r']) {
        Some(idx) if text.as_bytes()[idx] == b'\r' => {
            if text.as_bytes().get(idx + 1) == Some(&b'\n') {
                "\r\n"
            } else {
                "\r"
            }
        }
        _ => "\n",
    }
}

pub fn normalize_eol(text: &str, eol: &str) -> String {
    let lf = text.replace("\r\n", "\n").replace('\r', "\n");
    match eol {
        "\r\n" => lf.replace('\n', "\r\n"),
        "\r" => lf.replace('\n', "\r"),
        _ => lf,
    }
}
