//! Reference resolution: turn the unresolved [`RawRef`]s a file makes into graph
//! edges.
//!
//! It does this by (a) finding the *enclosing definition* a reference sits inside
//! (the edge's source) and (b) linking its name to a *target definition* (the
//! edge's sink), preferring locality.
//!
//! This layer is deliberately scope-light: it has no type system and no import
//! graph, so it resolves by name with a locality heuristic. That is enough for
//! the structural questions the graph answers ("who calls X", "what does Y call")
//! and is honest about ambiguity — when a name has several same-named definitions
//! and no local one wins, every candidate is linked so recall is preserved rather
//! than silently guessing one.

use crate::graph::NodeId;
use crate::parse::Span;

/// Cap on how many target definitions a single ambiguous reference may link to.
///
/// A name defined in dozens of files (e.g. `new`, `render`) would otherwise
/// explode the edge count with low-signal links; this bounds the blast radius
/// while still capturing the common 2–3-way ambiguity.
pub const MAX_AMBIGUOUS_TARGETS: usize = 8;

/// A definition placed in a file, paired with the byte span it occupies.
///
/// Used to decide which definition lexically *encloses* a given reference.
#[derive(Debug, Clone, Copy)]
pub struct Placed {
    pub node: NodeId,
    pub span: Span,
}

/// Find the definition that most tightly encloses `reference` among `defs` (all
/// from the same file).
///
/// "Tightest" = the container with the smallest byte range that still covers the
/// reference's start. Returns `None` for a reference that sits at file scope
/// (inside no definition).
///
/// `defs` need not be sorted. Ties on range size are broken by the later start
/// (the more deeply nested of two equal-width spans), then by node id, so the
/// result is deterministic.
#[must_use]
pub fn enclosing_def(defs: &[Placed], reference: Span, exclude: Option<NodeId>) -> Option<NodeId> {
    defs.iter()
        .filter(|d| {
            // A definition encloses the reference when its span covers it. Self is
            // excluded by node identity (not by a start-byte strict-less hack),
            // so a genuine parent that shares the child's start byte — e.g. a
            // decorated/exported wrapper extent — is still recognized.
            Some(d.node) != exclude
                && d.span.start_byte <= reference.start_byte
                && d.span.end_byte >= reference.end_byte
        })
        .min_by(|a, b| {
            let width = |s: Span| u64::from(s.end_byte) - u64::from(s.start_byte);
            width(a.span)
                .cmp(&width(b.span))
                .then(b.span.start_byte.cmp(&a.span.start_byte))
                .then(a.node.cmp(&b.node))
        })
        .map(|d| d.node)
}

/// How a reference resolved to its target(s) — the basis for an edge's
/// [`crate::graph::Confidence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Linked to same-file definition(s): the strongest signal.
    Local,
    /// Linked to exactly one definition elsewhere in the workspace.
    GlobalUnique,
    /// Linked to several same-named candidates: recall kept, precision uncertain.
    Ambiguous,
}

/// Pick the definition node(s) a reference's name resolves to, plus *how* it
/// resolved, given the candidates sharing that name in the same file and in the
/// graph at large.
///
/// Resolution order:
/// 1. **Local wins** — if any candidate is defined in the same file, link those
///    (almost always exactly one) and stop. Same-file definitions are the
///    strongest signal we have without real scope resolution.
/// 2. **Unique global** — otherwise, if the name resolves to exactly one
///    definition graph-wide, link it.
/// 3. **Ambiguous global** — otherwise link up to [`MAX_AMBIGUOUS_TARGETS`]
///    candidates (sorted for determinism), preserving recall.
///
/// A name with no candidates (external / standard-library symbol) yields an empty
/// vec and contributes no edge — expected, not an error.
///
/// `self_node` is the reference's own enclosing definition when known; when
/// provided, that node is excluded from the result set so non-recursive name
/// captures don't create accidental self-loops. Pass `None` for calls and
/// references where self-loops are semantically valid (recursive functions).
#[must_use]
pub fn resolve_targets(
    same_file: &[NodeId],
    global: &[NodeId],
    self_node: Option<NodeId>,
) -> (Vec<NodeId>, Resolution) {
    let exclude = |candidates: &[NodeId]| -> Vec<NodeId> {
        let mut out: Vec<NodeId> = candidates
            .iter()
            .copied()
            .filter(|&n| Some(n) != self_node)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    };

    let local = exclude(same_file);
    if !local.is_empty() {
        return (local, Resolution::Local);
    }
    let mut globals = exclude(global);
    if globals.len() > 1 {
        globals.truncate(MAX_AMBIGUOUS_TARGETS);
        (globals, Resolution::Ambiguous)
    } else {
        // Zero candidates (external name) or exactly one — both are "unique" in
        // that no ambiguity had to be resolved.
        (globals, Resolution::GlobalUnique)
    }
}

