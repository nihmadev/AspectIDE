//! Per-file parsing: turn one source string into the raw symbols it *defines*
//! and the raw references it *makes*, with source spans.
//!
//! This layer is intentionally dumb — it never resolves a reference to a
//! definition (that is `resolve`'s job in the graph build). It just reports what
//! the tree-sitter `tags` query saw.

use serde::{Deserialize, Serialize};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, QueryCursor};

use crate::lang::Lang;

/// What kind of thing a definition is. Derived from the `definition.<kind>`
/// capture name; unknown kinds fall back to [`SymbolKind::Other`] so a new query
/// capture never silently drops a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Interface,
    Class,
    Type,
    Constant,
    Module,
    Macro,
    Variable,
    Other,
}

impl SymbolKind {
    fn from_tag(tag: &str) -> Self {
        match tag {
            "function" => Self::Function,
            "method" => Self::Method,
            "struct" => Self::Struct,
            "enum" => Self::Enum,
            "interface" => Self::Interface,
            "class" => Self::Class,
            "type" => Self::Type,
            "constant" => Self::Constant,
            "module" => Self::Module,
            "macro" => Self::Macro,
            "variable" => Self::Variable,
            _ => Self::Other,
        }
    }
}

/// What kind of edge a reference will become once resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefKind {
    Call,
    Import,
    Implement,
    Reference,
}

impl RefKind {
    fn from_tag(tag: &str) -> Self {
        match tag {
            "call" => Self::Call,
            "import" => Self::Import,
            "implement" => Self::Implement,
            _ => Self::Reference,
        }
    }
}

/// A source range, stored compactly as `u32`s (byte offsets plus 0-based
/// line/column for both ends). Files larger than 4 GiB saturate — a non-issue for
/// source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_row: u32,
    pub start_col: u32,
    pub end_row: u32,
    pub end_col: u32,
}

impl Span {
    fn from_node(node: &Node) -> Self {
        let start = node.start_position();
        let end = node.end_position();
        Self {
            start_byte: clamp_u32(node.start_byte()),
            end_byte: clamp_u32(node.end_byte()),
            start_row: clamp_u32(start.row),
            start_col: clamp_u32(start.column),
            end_row: clamp_u32(end.row),
            end_col: clamp_u32(end.column),
        }
    }
}

fn clamp_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// A symbol this file defines. `span` is the definition's full lexical extent
/// (used to decide nesting/containment); `name_span` is just the identifier (used
/// for navigation and display).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub span: Span,
    pub name_span: Span,
}

/// A reference this file makes to some (as-yet unresolved) name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRef {
    pub name: String,
    pub kind: RefKind,
    pub span: Span,
}

/// Everything one parsed file contributes to the graph.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedFile {
    pub symbols: Vec<RawSymbol>,
    pub refs: Vec<RawRef>,
}

/// Failures that can occur while parsing a source string.
///
/// All are programmer- or grammar-level faults (a bad query, an unsupported
/// grammar), not per-file data errors — a syntactically broken file still parses
/// (tree-sitter is error tolerant) and simply yields whatever symbols it could
/// recover.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("failed to load grammar for {lang}: {source}")]
    Language {
        lang: &'static str,
        #[source]
        source: tree_sitter::LanguageError,
    },
    #[error("failed to compile tags query for {lang}: {source}")]
    Query {
        lang: &'static str,
        #[source]
        source: tree_sitter::QueryError,
    },
    #[error("tree-sitter produced no parse tree")]
    NoTree,
}

/// Parse `source` as `lang`, returning the symbols it defines and references it
/// makes. Error-tolerant at the data level: a malformed file yields partial
/// results rather than failing.
pub fn parse_source(lang: Lang, source: &str) -> Result<ParsedFile, ParseError> {
    let language = lang.language();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|source| ParseError::Language {
            lang: lang.name(),
            source,
        })?;
    let Some(tree) = parser.parse(source, None) else {
        return Err(ParseError::NoTree);
    };
    // Reuse the process-wide compiled query (one per language, behind a OnceLock)
    // instead of recompiling the tags query for every parsed file — query
    // compilation is the expensive tree-sitter step and dominated cold builds.
    let query = lang.compiled_tags_query();

    let capture_names = query.capture_names();
    let bytes = source.as_bytes();
    let mut parsed = ParsedFile::default();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), bytes);

    while let Some(m) = matches.next() {
        // A definition match carries two captures: `name.<kind>` (the identifier)
        // and `definition.<kind>` (the full extent). A reference match carries a
        // single `reference.<kind>` capture. Collect the pieces, then emit.
        let mut def_name: Option<(&str, &str, Span)> = None; // (kind, text, name_span)
        let mut def_extent: Option<Span> = None;

        for capture in m.captures {
            let Some(capture_name) = capture_names.get(capture.index as usize) else {
                continue;
            };
            let Some((role, kind)) = capture_name.split_once('.') else {
                continue;
            };
            let span = Span::from_node(&capture.node);
            match role {
                "name" => {
                    if let Ok(text) = capture.node.utf8_text(bytes) {
                        if !text.is_empty() {
                            def_name = Some((kind, text, span));
                        }
                    }
                }
                "definition" => def_extent = Some(span),
                "reference" => {
                    if let Ok(text) = capture.node.utf8_text(bytes) {
                        if !text.is_empty() {
                            parsed.refs.push(RawRef {
                                name: text.to_owned(),
                                kind: RefKind::from_tag(kind),
                                span,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some((kind, text, name_span)) = def_name {
            // Fall back to the name span as the extent if the grammar pattern
            // lacked an outer `@definition` capture (keeps containment sane).
            let extent = def_extent.unwrap_or(name_span);
            parsed.symbols.push(RawSymbol {
                name: text.to_owned(),
                kind: SymbolKind::from_tag(kind),
                span: extent,
                name_span,
            });
        }
    }

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::{parse_source, RefKind, SymbolKind};
    use crate::lang::Lang;

    fn symbol_named<'a>(parsed: &'a super::ParsedFile, name: &str) -> Option<&'a super::RawSymbol> {
        parsed.symbols.iter().find(|s| s.name == name)
    }

    #[test]
    fn parses_rust_definitions_and_calls() {
        let src = r#"
            fn helper() {}
            struct Thing;
            enum Color { Red }
            const N: u32 = 1;
            fn main() {
                helper();
                println!("hi");
            }
        "#;
        let parsed = parse_source(Lang::Rust, src).expect("rust parse");

        assert_eq!(
            symbol_named(&parsed, "helper").map(|s| s.kind),
            Some(SymbolKind::Function)
        );
        assert_eq!(
            symbol_named(&parsed, "main").map(|s| s.kind),
            Some(SymbolKind::Function)
        );
        assert_eq!(
            symbol_named(&parsed, "Thing").map(|s| s.kind),
            Some(SymbolKind::Struct)
        );
        assert_eq!(
            symbol_named(&parsed, "Color").map(|s| s.kind),
            Some(SymbolKind::Enum)
        );
        assert_eq!(
            symbol_named(&parsed, "N").map(|s| s.kind),
            Some(SymbolKind::Constant)
        );

        let calls: Vec<&str> = parsed
            .refs
            .iter()
            .filter(|r| r.kind == RefKind::Call)
            .map(|r| r.name.as_str())
            .collect();
        assert!(
            calls.contains(&"helper"),
            "expected call to helper, got {calls:?}"
        );
        assert!(
            calls.contains(&"println"),
            "expected macro call, got {calls:?}"
        );
    }

    #[test]
    fn parses_typescript_definitions_and_calls() {
        let src = r"
            function greet(name: string) { return name; }
            class Greeter { hello() { greet('x'); } }
            interface Named { name: string }
            const run = () => greet('y');
        ";
        let parsed = parse_source(Lang::TypeScript, src).expect("ts parse");

        assert_eq!(
            symbol_named(&parsed, "greet").map(|s| s.kind),
            Some(SymbolKind::Function)
        );
        assert_eq!(
            symbol_named(&parsed, "Greeter").map(|s| s.kind),
            Some(SymbolKind::Class)
        );
        assert_eq!(
            symbol_named(&parsed, "hello").map(|s| s.kind),
            Some(SymbolKind::Method)
        );
        assert_eq!(
            symbol_named(&parsed, "Named").map(|s| s.kind),
            Some(SymbolKind::Interface)
        );
        // `const run = () => ...` is reported as a function-valued definition.
        assert!(
            symbol_named(&parsed, "run").is_some(),
            "arrow const should be a symbol"
        );

        let greet_calls = parsed
            .refs
            .iter()
            .filter(|r| r.kind == RefKind::Call && r.name == "greet")
            .count();
        assert_eq!(greet_calls, 2, "both greet() calls should be captured");
    }

    #[test]
    fn parses_python_definitions_and_calls() {
        let src = "\
def greet(name):
    return name

class Greeter:
    def hello(self):
        greet('x')
";
        let parsed = parse_source(Lang::Python, src).expect("py parse");

        assert_eq!(
            symbol_named(&parsed, "greet").map(|s| s.kind),
            Some(SymbolKind::Function)
        );
        assert_eq!(
            symbol_named(&parsed, "Greeter").map(|s| s.kind),
            Some(SymbolKind::Class)
        );
        assert_eq!(
            symbol_named(&parsed, "hello").map(|s| s.kind),
            Some(SymbolKind::Function)
        );

        assert!(
            parsed
                .refs
                .iter()
                .any(|r| r.kind == RefKind::Call && r.name == "greet"),
            "expected a call to greet"
        );
    }

    #[test]
    fn spans_cover_extent_and_name() {
        let src = "fn alpha() {}";
        let parsed = parse_source(Lang::Rust, src).expect("rust parse");
        let alpha = symbol_named(&parsed, "alpha").expect("alpha symbol");
        // name_span covers exactly the identifier...
        let name = &src[alpha.name_span.start_byte as usize..alpha.name_span.end_byte as usize];
        assert_eq!(name, "alpha");
        // ...while span covers the whole definition (starts at `fn`).
        let extent = &src[alpha.span.start_byte as usize..alpha.span.end_byte as usize];
        assert_eq!(extent, "fn alpha() {}");
    }
}
