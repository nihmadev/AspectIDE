//! Language registry: the single source of truth for which languages the code
//! graph understands.
//!
//! It knows how to recognize a file as one of them, and where to find each
//! language's tree-sitter grammar and symbol-extraction (`tags`) query.
//!
//! Adding a language is deliberately tiny: add a `Lang` variant, map its
//! extensions in [`Lang::from_extension`], return its grammar in
//! [`Lang::language`], and ship a `queries/<lang>/tags.scm`. Everything
//! downstream (parsing, the graph, queries) is language-agnostic and needs no
//! further change.

use tree_sitter::Language;

/// A source language the code graph can parse.
///
/// Distinct variants exist only where the grammar differs; several file
/// extensions can map to one variant (e.g. JS and JSX ride the TSX grammar,
/// which is a strict superset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    TypeScript,
    /// TSX grammar — also used for plain JS/JSX (superset of both).
    Tsx,
    Python,
}

impl Lang {
    /// Recognize a language from a lowercase-or-mixed file extension (no dot),
    /// e.g. `"rs"`, `"tsx"`, `"PY"`. Returns `None` for unsupported extensions.
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        // Match case-insensitively without allocating for the common (lowercase) path.
        let lower;
        let ext = if ext.bytes().any(|b| b.is_ascii_uppercase()) {
            lower = ext.to_ascii_lowercase();
            lower.as_str()
        } else {
            ext
        };
        match ext {
            "rs" => Some(Self::Rust),
            "ts" | "mts" | "cts" => Some(Self::TypeScript),
            // The TSX grammar parses TS, JS and JSX too — reuse it for all of them.
            "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(Self::Tsx),
            "py" | "pyi" => Some(Self::Python),
            _ => None,
        }
    }

    /// Recognize a language from a path's extension. Convenience over
    /// [`Lang::from_extension`].
    #[must_use]
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
    }

    /// The tree-sitter grammar for this language. Cheap to construct (a thin
    /// wrapper over a static function pointer in the grammar crate).
    #[must_use]
    pub fn language(self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }

    /// The symbol-extraction `tags` query source for this language, baked into the
    /// binary at compile time.
    #[must_use]
    pub const fn tags_query(self) -> &'static str {
        match self {
            Self::Rust => include_str!("../queries/rust/tags.scm"),
            // TS and TSX share one query set (TSX node kinds are a superset).
            Self::TypeScript | Self::Tsx => include_str!("../queries/typescript/tags.scm"),
            Self::Python => include_str!("../queries/python/tags.scm"),
        }
    }

    /// A short, stable display name for the language.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript",
            Self::Tsx => "TSX",
            Self::Python => "Python",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Lang;

    #[test]
    fn extensions_map_to_expected_languages() {
        assert_eq!(Lang::from_extension("rs"), Some(Lang::Rust));
        assert_eq!(Lang::from_extension("ts"), Some(Lang::TypeScript));
        assert_eq!(Lang::from_extension("tsx"), Some(Lang::Tsx));
        assert_eq!(Lang::from_extension("jsx"), Some(Lang::Tsx));
        assert_eq!(Lang::from_extension("py"), Some(Lang::Python));
        assert_eq!(Lang::from_extension("txt"), None);
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        assert_eq!(Lang::from_extension("RS"), Some(Lang::Rust));
        assert_eq!(Lang::from_extension("Py"), Some(Lang::Python));
    }

    #[test]
    fn from_path_uses_extension() {
        assert_eq!(
            Lang::from_path(std::path::Path::new("src/main.rs")),
            Some(Lang::Rust)
        );
        assert_eq!(Lang::from_path(std::path::Path::new("Makefile")), None);
    }

    #[test]
    fn every_language_has_a_loadable_grammar_and_query() {
        for lang in [Lang::Rust, Lang::TypeScript, Lang::Tsx, Lang::Python] {
            let language = lang.language();
            // A non-empty tags query that actually compiles against the grammar is
            // the real contract — exercise it so a malformed `.scm` fails loudly.
            assert!(
                !lang.tags_query().is_empty(),
                "{} has no tags query",
                lang.name()
            );
            tree_sitter::Query::new(&language, lang.tags_query())
                .unwrap_or_else(|e| panic!("{} tags query failed to compile: {e}", lang.name()));
        }
    }
}
