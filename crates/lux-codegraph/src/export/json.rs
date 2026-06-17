//! A minimal, insertion-order JSON value + pretty serializer that reproduces
//! Python's `json.dump(obj, indent=2, ensure_ascii=True, sort_keys=False)` byte
//! for byte, so exported `graph.json` matches what graphify writes.
//!
//! Reproduced conventions:
//! * 2-space indentation; `": "` after keys; `","` (no trailing space) between
//!   items, each on its own line.
//! * Empty arrays/objects render inline as `[]` / `{}`.
//! * `ensure_ascii`: every non-ASCII scalar is escaped to a lowercase `\uXXXX`
//!   sequence (surrogate pair for astral codepoints).
//! * Floats use Python's `repr`-style shortest form (`1.0`, `0.5`).
//! * No trailing newline.

use std::fmt::Write as _;

/// A JSON value. Object key order is preserved exactly as inserted — the export
/// layer relies on this for graphify schema parity.
#[derive(Debug, Clone)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl Json {
    /// Serialize with 2-space indent, matching Python `json.dump(indent=2)`.
    #[must_use]
    pub fn to_string_pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Int(n) => {
                let _ = write!(out, "{n}");
            }
            Self::Float(f) => out.push_str(&format_float(*f)),
            Self::Str(s) => write_escaped(out, s),
            Self::Array(items) => write_array(out, items, indent),
            Self::Object(entries) => write_object(out, entries, indent),
        }
    }
}

fn write_array(out: &mut String, items: &[Json], indent: usize) {
    if items.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    let inner = indent + 1;
    for (i, item) in items.iter().enumerate() {
        push_indent(out, inner);
        item.write(out, inner);
        if i + 1 < items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(out, indent);
    out.push(']');
}

fn write_object(out: &mut String, entries: &[(String, Json)], indent: usize) {
    if entries.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push_str("{\n");
    let inner = indent + 1;
    for (i, (key, value)) in entries.iter().enumerate() {
        push_indent(out, inner);
        write_escaped(out, key);
        out.push_str(": ");
        value.write(out, inner);
        if i + 1 < entries.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(out, indent);
    out.push('}');
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Write a JSON string literal with Python `ensure_ascii=True` escaping.
fn write_escaped(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            // Printable ASCII (0x20..=0x7E) only. DEL (0x7F) is `is_ascii()` but
            // Python's ensure_ascii escapes it, so it falls through below.
            c if (0x20..=0x7e).contains(&(c as u32)) => out.push(c),
            // Non-ASCII → \uXXXX (surrogate pair for astral planes), matching
            // Python's ensure_ascii.
            c => {
                let code = c as u32;
                if code <= 0xFFFF {
                    let _ = write!(out, "\\u{code:04x}");
                } else {
                    let v = code - 0x1_0000;
                    let high = 0xD800 + (v >> 10);
                    let low = 0xDC00 + (v & 0x3FF);
                    let _ = write!(out, "\\u{high:04x}\\u{low:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// Format a float for JSON output. Always includes a decimal point (so `1`
/// serializes as `1.0`).
///
/// Parity scope: the values this crate emits are confidence scores in
/// {1.0, 0.5, 0.2}, which print identically to Python's `json`. This does **not**
/// reproduce Python's scientific-notation threshold (it switches to `1e-05` /
/// `1e+16` for very small/large magnitudes); if astronomically-scaled floats are
/// ever emitted, revisit this.
fn format_float(f: f64) -> String {
    if f.is_nan() || f.is_infinite() {
        // Python would emit NaN/Infinity (invalid JSON); our scores are finite,
        // so map these to null defensively rather than produce invalid output.
        return "null".to_string();
    }
    let mut s = format!("{f}");
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        s.push_str(".0");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::Json;

    #[test]
    fn empty_containers_are_inline() {
        assert_eq!(Json::Array(vec![]).to_string_pretty(), "[]");
        assert_eq!(Json::Object(vec![]).to_string_pretty(), "{}");
    }

    #[test]
    fn object_uses_two_space_indent_and_insertion_order() {
        let value = Json::Object(vec![
            ("b".into(), Json::Int(1)),
            ("a".into(), Json::Bool(true)),
        ]);
        // Keys stay in insertion order (b before a), 2-space indent.
        assert_eq!(value.to_string_pretty(), "{\n  \"b\": 1,\n  \"a\": true\n}");
    }

    #[test]
    fn floats_always_have_a_decimal_point() {
        assert_eq!(Json::Float(1.0).to_string_pretty(), "1.0");
        assert_eq!(Json::Float(0.5).to_string_pretty(), "0.5");
        assert_eq!(Json::Float(0.2).to_string_pretty(), "0.2");
    }

    #[test]
    fn non_ascii_is_escaped() {
        assert_eq!(Json::Str("é".into()).to_string_pretty(), "\"\\u00e9\"");
        // Astral plane → surrogate pair.
        assert_eq!(
            Json::Str("😀".into()).to_string_pretty(),
            "\"\\ud83d\\ude00\""
        );
    }

    #[test]
    fn control_and_quote_escapes() {
        assert_eq!(Json::Str("a\"b".into()).to_string_pretty(), "\"a\\\"b\"");
        assert_eq!(Json::Str("\n".into()).to_string_pretty(), "\"\\n\"");
        assert_eq!(Json::Str("\u{01}".into()).to_string_pretty(), "\"\\u0001\"");
        // DEL (0x7F) is ASCII but Python's ensure_ascii escapes it.
        assert_eq!(Json::Str("\u{7f}".into()).to_string_pretty(), "\"\\u007f\"");
    }

    #[test]
    fn nested_structure_renders() {
        let value = Json::Object(vec![(
            "items".into(),
            Json::Array(vec![Json::Int(1), Json::Null]),
        )]);
        assert_eq!(
            value.to_string_pretty(),
            "{\n  \"items\": [\n    1,\n    null\n  ]\n}"
        );
    }
}
