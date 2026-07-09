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

use std::sync::OnceLock;
use tree_sitter::{Language, Query};

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
    /// Every language variant, for callers that must iterate the whole registry
    /// (e.g. fingerprinting all `tags` queries for the parse cache).
    pub const ALL: [Self; 4] = [Self::Rust, Self::TypeScript, Self::Tsx, Self::Python];

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

    /// The compiled tags query for this language, cached behind a `OnceLock`.
    /// A `tree_sitter::Query` is bound to the exact grammar it was compiled
    /// against, so even though TS and TSX share one `.scm` source, each must be
    /// compiled against its own grammar — hence one cache slot per variant (4),
    /// compiled at most once each regardless of how many files are parsed across
    /// cold opens and incremental edits. The `.scm` sources are `include_str!`ed
    /// and test-asserted to compile, so a failure here is a logic bug worth
    /// panicking on.
    #[must_use]
    pub fn compiled_tags_query(self) -> &'static Query {
        static RUST_TAGS: OnceLock<Query> = OnceLock::new();
        static TYPESCRIPT_TAGS: OnceLock<Query> = OnceLock::new();
        static TSX_TAGS: OnceLock<Query> = OnceLock::new();
        static PYTHON_TAGS: OnceLock<Query> = OnceLock::new();
        let slot = match self {
            Self::Rust => &RUST_TAGS,
            Self::TypeScript => &TYPESCRIPT_TAGS,
            Self::Tsx => &TSX_TAGS,
            Self::Python => &PYTHON_TAGS,
        };
        slot.get_or_init(|| {
            Query::new(&self.language(), self.tags_query()).unwrap_or_else(|error| {
                panic!("{} tags query failed to compile: {error}", self.name())
            })
        })
    }
}

