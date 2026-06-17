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
/// `self_node` is the reference's own enclosing definition when known; it is
/// never returned, so a recursive call does not create a self-loop unless the
/// function genuinely has a same-named sibling.
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

#[cfg(test)]
mod tests {
    use super::{enclosing_def, resolve_targets, Placed, Resolution};
    use crate::graph::NodeId;
    use crate::parse::Span;

    fn span(start: u32, end: u32) -> Span {
        Span {
            start_byte: start,
            end_byte: end,
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 0,
        }
    }

    fn node(raw: u32) -> NodeId {
        // NodeId is created via the graph in real use; for resolution unit tests
        // we only need distinct handles, which `from_raw` provides.
        NodeId::from_raw(raw)
    }

    #[test]
    fn tightest_enclosing_definition_wins() {
        // outer [0,100) contains inner [40,60); a ref at 50 belongs to inner.
        let defs = [
            Placed {
                node: node(1),
                span: span(0, 100),
            },
            Placed {
                node: node(2),
                span: span(40, 60),
            },
        ];
        assert_eq!(enclosing_def(&defs, span(50, 52), None), Some(node(2)));
    }

    #[test]
    fn reference_at_file_scope_has_no_encloser() {
        let defs = [Placed {
            node: node(1),
            span: span(40, 60),
        }];
        // ref at byte 10 is before the only definition.
        assert_eq!(enclosing_def(&defs, span(10, 12), None), None);
    }

    #[test]
    fn definition_does_not_enclose_itself() {
        // A def's own extent [10,20) covers its name [10,15), but excluding self by
        // identity means it is not reported as its own parent.
        let defs = [Placed {
            node: node(1),
            span: span(10, 20),
        }];
        assert_eq!(enclosing_def(&defs, span(10, 15), Some(node(1))), None);
    }

    #[test]
    fn same_start_parent_is_still_recognized() {
        // A wrapper [10,80) and an inner def [10,40) share start byte 10. The inner
        // def's parent must be the wrapper — not dropped by a start-byte hack.
        let defs = [
            Placed {
                node: node(1),
                span: span(10, 80),
            },
            Placed {
                node: node(2),
                span: span(10, 40),
            },
        ];
        assert_eq!(
            enclosing_def(&defs, span(10, 40), Some(node(2))),
            Some(node(1))
        );
    }

    #[test]
    fn local_candidates_beat_global() {
        let same_file = [node(5)];
        let global = [node(5), node(9), node(12)];
        let (targets, res) = resolve_targets(&same_file, &global, None);
        assert_eq!(targets, vec![node(5)]);
        assert_eq!(res, Resolution::Local);
    }

    #[test]
    fn unique_global_resolves() {
        let (targets, res) = resolve_targets(&[], &[node(7)], None);
        assert_eq!(targets, vec![node(7)]);
        assert_eq!(res, Resolution::GlobalUnique);
    }

    #[test]
    fn ambiguous_global_links_all_sorted() {
        let global = [node(9), node(3), node(6)];
        let (targets, res) = resolve_targets(&[], &global, None);
        assert_eq!(targets, vec![node(3), node(6), node(9)]);
        assert_eq!(res, Resolution::Ambiguous);
    }

    #[test]
    fn self_reference_is_excluded() {
        // Only candidate is the caller itself → no edge (not a self-loop).
        let (targets, _) = resolve_targets(&[], &[node(4)], Some(node(4)));
        assert_eq!(targets, vec![]);
    }

    #[test]
    fn external_name_resolves_to_nothing() {
        let (targets, _) = resolve_targets(&[], &[], None);
        assert_eq!(targets, Vec::<NodeId>::new());
    }
}
