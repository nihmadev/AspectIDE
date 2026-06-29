//! `lux-codegraph` — a structural code graph for the workspace.
//!
//! Where the existing project index is lexical (token/name scoring), this crate
//! builds a real graph of code structure with tree-sitter: nodes for the symbols
//! a codebase defines (functions, types, modules, …) and kinded edges between
//! them (defines / calls / imports / references / implements). That makes
//! structural questions answerable precisely — "who calls X", "what breaks if I
//! change Y", "which symbols are central" — instead of approximately.
//!
//! It is a complement, not a replacement: the lexical index remains the fallback
//! while the graph is still building or for languages without a grammar. Parsing
//! is fully local and static — no network, no LLM in the core.
//!
//! ## Layers
//! * [`lang`] — language registry: extension → grammar + `tags` query.
//! * [`parse`] — one source string → the symbols it defines and references it makes.
//! * [`graph`] — the compact, interned, CSR-backed [`graph::CodeGraph`].
//! * [`resolve`] — link references to definitions by name with locality priority.
//! * [`index`] — walk + parallel-parse a workspace into a resolved graph, with
//!   single-file incremental updates.
//!
//! The query API builds on these in a later phase.

#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::module_name_repetitions)]
// Graph algorithms convert freely between node counts, indices, and scores. Every
// such cast is bounded by the node/edge count (well under 2^32 by the MAX_NODES
// cap) or is a deliberate score approximation, so the precision/truncation cast
// lints are noise here and would otherwise bury the math in `#[allow]`s.
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]

pub mod cache;
pub mod community;
pub mod detect;
pub mod export;
pub mod graph;
pub mod index;
pub mod lang;
pub mod metrics;
pub mod parse;
pub mod query;
pub mod resolve;

pub use cache::{FileMeta, CACHE_VERSION};
pub use community::{detect as detect_communities, Community};
pub use detect::{
    file_dependency_cycles, god_nodes, import_cycles, surprising_connections, GodNode, ImportCycle,
    Surprise,
};
pub use export::{to_graph_html, to_graph_json, to_report};
pub use graph::{Adjacent, CodeGraph, Confidence, Edge, EdgeKind, FileId, Node, NodeId, Symbol};
pub use index::{Index, IndexError};
pub use lang::Lang;
pub use metrics::{betweenness, confidence_counts, degree, degrees, ConfidenceCounts};
pub use parse::{
    parse_source, ParseError, ParsedFile, RawRef, RawSymbol, RefKind, Span, SymbolKind,
};
pub use query::{
    callees, callers, explain, neighbors, resolve as resolve_symbol, resolve_one, shortest_path,
    Direction, Explanation, Neighbor, NodeRef, Path, PathStep,
};
pub use resolve::{enclosing_def, resolve_targets, Placed, Resolution};
