//! Export a [`CodeGraph`] to graphify-compatible artifacts: `graph.json` (the
//! queryable graph) and `GRAPH_REPORT.md` (the human summary).
//!
//! The JSON is written with a small insertion-order serializer that mirrors
//! Python's `json.dump(indent=2, ensure_ascii=True, sort_keys=False)` — 2-space
//! indent, non-ASCII escaped to `\uXXXX`, keys in insertion order, no trailing
//! newline — so the output drops into graphify's `graphify-out/` and is readable
//! by its tooling. Node/edge schema and enum values match graphify's exactly
//! (see `export.py`): nodes carry `label`/`file_type`/`source_file`/`id`/
//! `community`/`norm_label`; edges carry `source`/`target`/`relation`/
//! `confidence`/`confidence_score`, with `_src`/`_tgt` resolved into
//! `source`/`target` and never emitted.

use std::fmt::Write as _;
use std::path::Path;

use crate::community::Community;
use crate::detect::{
    god_nodes, import_cycles, surprising_connections, DEFAULT_GOD_NODES, DEFAULT_SURPRISES,
};
use crate::graph::{CodeGraph, EdgeKind, NodeId};
use crate::metrics::confidence_counts;
use crate::parse::SymbolKind;

mod html;
mod json;
use html::GRAPH_HTML_TEMPLATE;
use json::Json;

/// The graphify `relation` string for one of our edge kinds.
const fn relation_str(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Defines => "contains",
        EdgeKind::Calls => "calls",
        EdgeKind::Imports => "imports",
        EdgeKind::References => "references",
        EdgeKind::Implements => "implements",
    }
}

/// The graphify `node_type` string for one of our symbol kinds.
const fn node_type_str(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Interface => "interface",
        SymbolKind::Class => "class",
        SymbolKind::Type => "type",
        SymbolKind::Constant => "constant",
        SymbolKind::Module => "module",
        SymbolKind::Macro => "macro",
        SymbolKind::Variable => "variable",
        SymbolKind::Other => "symbol",
    }
}

/// Render `path` relative to `root` with forward slashes (graphify's
/// `source_file` convention). Falls back to the path as-is when not under root.
fn rel_path(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

/// A stable, unique node id string: relative path, symbol name, 1-based line, and
/// the node index. The trailing index guarantees uniqueness even when two
/// same-named defs share a line (macro-generated / overloaded items) — a plain
/// `file::name::line` would collide. Mirrors graphify's qualified-id idea.
fn node_id(graph: &CodeGraph, node: NodeId, root: &Path) -> String {
    let Some(data) = graph.node(node) else {
        return format!("node{}", node.index());
    };
    let file = graph
        .file_path(data.file)
        .map_or_else(String::new, |p| rel_path(p, root));
    let name = graph.name_of(node).unwrap_or("");
    format!(
        "{file}::{name}::{}::{}",
        data.name_span.start_row + 1,
        node.index()
    )
}

/// Serialize the whole graph to a graphify-compatible `graph.json` string.
///
/// `communities` may be empty (every `community` field is then `null`).
#[must_use]
pub fn to_graph_json(graph: &CodeGraph, communities: &[Community], root: &Path) -> String {
    let community_of = crate::community::node_community_map(communities);

    let mut nodes = Vec::with_capacity(graph.node_count());
    for i in 0..graph.node_count() {
        let id = NodeId::from_raw(u32::try_from(i).unwrap_or(u32::MAX));
        let Some(data) = graph.node(id) else { continue };
        let name = graph.name_of(id).unwrap_or("").to_string();
        let source_file = graph
            .file_path(data.file)
            .map_or_else(String::new, |p| rel_path(p, root));

        // Node object — key order matches export.py: stored attrs, id, community,
        // [community_name], norm_label.
        let mut obj: Vec<(String, Json)> = vec![
            ("label".into(), Json::Str(name.clone())),
            ("file_type".into(), Json::Str("code".into())),
            ("source_file".into(), Json::Str(source_file)),
            (
                "source_location".into(),
                Json::Str(format!("L{}", data.name_span.start_row + 1)),
            ),
            (
                "node_type".into(),
                Json::Str(node_type_str(data.kind).into()),
            ),
            ("_origin".into(), Json::Str("ast".into())),
            ("id".into(), Json::Str(node_id(graph, id, root))),
        ];
        match community_of.get(&id) {
            Some(&cid) => {
                obj.push(("community".into(), Json::Int(cid as i64)));
                obj.push((
                    "community_name".into(),
                    Json::Str(format!("Community {cid}")),
                ));
            }
            None => obj.push(("community".into(), Json::Null)),
        }
        obj.push(("norm_label".into(), Json::Str(name.to_lowercase())));
        nodes.push(Json::Object(obj));
    }

    let mut links = Vec::with_capacity(graph.edge_count());
    for i in 0..graph.node_count() {
        let from = NodeId::from_raw(u32::try_from(i).unwrap_or(u32::MAX));
        let Some(from_data) = graph.node(from) else {
            continue;
        };
        let source_file = graph
            .file_path(from_data.file)
            .map_or_else(String::new, |p| rel_path(p, root));
        for adj in graph.out_neighbors(from) {
            // Edge object — relation/confidence/source_file then source/target,
            // confidence_score appended. _src/_tgt are resolved into source/target
            // and never emitted (export.py parity).
            let obj: Vec<(String, Json)> = vec![
                ("relation".into(), Json::Str(relation_str(adj.kind).into())),
                ("confidence".into(), Json::Str(adj.confidence.tag().into())),
                (
                    "confidence_score".into(),
                    Json::Float(adj.confidence.score()),
                ),
                ("source_file".into(), Json::Str(source_file.clone())),
                ("source".into(), Json::Str(node_id(graph, from, root))),
                ("target".into(), Json::Str(node_id(graph, adj.node, root))),
            ];
            links.push(Json::Object(obj));
        }
    }

    let document = Json::Object(vec![
        ("directed".into(), Json::Bool(true)),
        ("multigraph".into(), Json::Bool(false)),
        ("graph".into(), Json::Object(Vec::new())),
        ("nodes".into(), Json::Array(nodes)),
        ("links".into(), Json::Array(links)),
        ("hyperedges".into(), Json::Array(Vec::new())),
    ]);
    document.to_string_pretty()
}

/// Above this node count the visualization renders only the most-connected
/// nodes — an O(n²)-per-frame force layout in the browser bogs down past a few
/// thousand, and a hairball that large is unreadable anyway. The page shows a
/// notice when this trims the graph.
const MAX_VIZ_NODES: usize = 1500;

/// Render the graph as a single self-contained interactive `code-graph.html`.
///
/// The page embeds the graph data plus a dependency-free canvas force-directed
/// renderer — no network, no CDN, no third-party script — so it opens offline and
/// is safe to ship. Nodes are colored by community and sized by degree; edges are
/// shaded by confidence. Hover for details, click to focus a node's neighborhood,
/// drag to reposition, wheel to zoom. For graphs over [`MAX_VIZ_NODES`] only the
/// most-connected nodes (and edges between them) are emitted.
#[must_use]
pub fn to_graph_html(
    graph: &CodeGraph,
    communities: &[Community],
    root: &Path,
    title: &str,
) -> String {
    use std::collections::HashSet;

    let community_of = crate::community::node_community_map(communities);

    // Pick the nodes to render: all of them, unless the graph is large, in which
    // case keep the top `MAX_VIZ_NODES` by degree (most structurally central).
    let mut all: Vec<(NodeId, u32)> = (0..graph.node_count())
        .filter_map(|i| {
            let id = NodeId::from_raw(u32::try_from(i).ok()?);
            graph
                .node(id)
                .map(|_| (id, crate::metrics::degree(graph, id)))
        })
        .collect();
    let total_nodes = all.len();
    let truncated = total_nodes > MAX_VIZ_NODES;
    if truncated {
        all.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.index().cmp(&b.0.index())));
        all.truncate(MAX_VIZ_NODES);
    }
    let kept: HashSet<NodeId> = all.iter().map(|(id, _)| *id).collect();

    let mut nodes = Vec::with_capacity(all.len());
    for (id, degree) in &all {
        let id = *id;
        let label = graph.name_of(id).unwrap_or("").to_string();
        let source_file = graph
            .file_path(
                graph
                    .node(id)
                    .map_or_else(|| crate::graph::FileId::from_raw(0), |n| n.file),
            )
            .map_or_else(String::new, |p| rel_path(p, root));
        let kind = graph.node(id).map_or("symbol", |n| node_type_str(n.kind));
        let community = community_of
            .get(&id)
            .map_or(Json::Null, |&cid| Json::Int(cid as i64));
        nodes.push(Json::Object(vec![
            ("id".into(), Json::Str(node_id(graph, id, root))),
            ("label".into(), Json::Str(label)),
            ("type".into(), Json::Str(kind.into())),
            ("file".into(), Json::Str(source_file)),
            ("community".into(), community),
            ("degree".into(), Json::Int(i64::from(*degree))),
        ]));
    }

    let mut links = Vec::new();
    for (from, _) in &all {
        let from = *from;
        for adj in graph.out_neighbors(from) {
            if !kept.contains(&adj.node) {
                continue; // edge to a trimmed node — drop it
            }
            links.push(Json::Object(vec![
                ("source".into(), Json::Str(node_id(graph, from, root))),
                ("target".into(), Json::Str(node_id(graph, adj.node, root))),
                ("relation".into(), Json::Str(relation_str(adj.kind).into())),
                ("confidence".into(), Json::Str(adj.confidence.tag().into())),
            ]));
        }
    }

    let edge_count = links.len();
    let data = Json::Object(vec![
        ("nodes".into(), Json::Array(nodes)),
        ("links".into(), Json::Array(links)),
    ])
    .to_string_pretty();
    // `</` → `<\/` keeps the data from prematurely closing the <script> tag while
    // staying valid JSON (`\/` parses back to `/`). The data lives in a
    // type="application/json" block, so it is never executed regardless.
    let data = data.replace("</", "<\\/");

    GRAPH_HTML_TEMPLATE
        .replace("__TITLE__", &html_escape(title))
        .replace("__TOTAL_NODES__", &total_nodes.to_string())
        .replace("__SHOWN_NODES__", &all.len().to_string())
        .replace("__EDGE_COUNT__", &edge_count.to_string())
        .replace("__COMMUNITY_COUNT__", &communities.len().to_string())
        .replace("__TRUNCATED__", if truncated { "true" } else { "false" })
        // Data substituted last so the placeholder swaps above never scan the blob.
        .replace("__GRAPH_DATA__", &data)
}

/// Minimal HTML-text escape for values dropped into element text / attributes
/// (the project title). Identifiers and paths never contain these, but a defensive
/// escape costs nothing.
fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Build the `GRAPH_REPORT.md` summary for the graph.
///
/// Sections mirror graphify's `report.py`: title, summary stats, god nodes,
/// surprising connections, and import cycles. `title` is the project/root name
/// shown in the header; `date` is a caller-supplied `YYYY-MM-DD` (the crate has
/// no clock, so the timestamp is injected).
#[must_use]
pub fn to_report(graph: &CodeGraph, communities: &[Community], title: &str, date: &str) -> String {
    use crate::graph::Confidence;

    let mut out = String::new();
    let _ = writeln!(out, "# Graph Report - {title}  ({date})");
    let _ = writeln!(out);

    // ── Summary ──
    let counts = confidence_counts(graph);
    let _ = writeln!(out, "## Summary");
    let _ = writeln!(
        out,
        "- {} nodes · {} edges · {} communities",
        graph.node_count(),
        graph.edge_count(),
        communities.len()
    );
    let _ = writeln!(
        out,
        "- Extraction: {}% EXTRACTED · {}% INFERRED · {}% AMBIGUOUS",
        counts.percent(Confidence::Extracted),
        counts.percent(Confidence::Inferred),
        counts.percent(Confidence::Ambiguous),
    );
    let _ = writeln!(out);

    // ── God nodes ──
    let _ = writeln!(
        out,
        "## God Nodes (most connected - your core abstractions)"
    );
    let gods = god_nodes(graph, DEFAULT_GOD_NODES);
    if gods.is_empty() {
        let _ = writeln!(out, "- None detected.");
    } else {
        for (rank, god) in gods.iter().enumerate() {
            let _ = writeln!(out, "{}. `{}` - {} edges", rank + 1, god.name, god.degree);
        }
    }
    let _ = writeln!(out);

    // ── Surprising connections ──
    let _ = writeln!(
        out,
        "## Surprising Connections (you probably didn't know these)"
    );
    let surprises = surprising_connections(graph, communities, DEFAULT_SURPRISES);
    if surprises.is_empty() {
        let _ = writeln!(
            out,
            "- None detected - all connections are within the same source files."
        );
    } else {
        for surprise in &surprises {
            let _ = writeln!(
                out,
                "- `{}` --{}--> `{}`  [{}]",
                surprise.source_name,
                relation_str(surprise.relation),
                surprise.target_name,
                surprise.confidence.tag(),
            );
            let _ = writeln!(out, "  {}", surprise.why);
        }
    }
    let _ = writeln!(out);

    // ── Import cycles ──
    let _ = writeln!(out, "## Import Cycles");
    let cycles = import_cycles(graph);
    if cycles.is_empty() {
        let _ = writeln!(out, "- None detected.");
    } else {
        for cycle in &cycles {
            let mut chain = cycle.files.clone();
            if let Some(first) = chain.first().cloned() {
                chain.push(first); // close the loop for display
            }
            let _ = writeln!(
                out,
                "- {}-file cycle: `{}`",
                cycle.files.len(),
                chain.join(" -> ")
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{to_graph_json, to_report};
    use crate::community;
    use crate::index::Index;
    use std::io::Write;

    fn workspace(files: &[(&str, &str)]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "lux-codegraph-export-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        );
        let root = std::env::temp_dir().join(unique);
        let _ = std::fs::remove_dir_all(&root);
        for (rel, contents) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::File::create(&path)
                .unwrap()
                .write_all(contents.as_bytes())
                .unwrap();
        }
        root
    }

    #[test]
    fn graph_json_has_graphify_shape() {
        let root = workspace(&[("lib.rs", "fn helper() {}\nfn caller() { helper(); }\n")]);
        let index = Index::build(&root).expect("build");
        let communities = community::detect(index.graph());
        let json = to_graph_json(index.graph(), &communities, &root);

        // Top-level keys present in graphify order.
        assert!(json.starts_with("{\n  \"directed\": true,"));
        assert!(json.contains("\"multigraph\": false"));
        assert!(json.contains("\"nodes\": ["));
        assert!(json.contains("\"links\": ["));
        assert!(json.contains("\"hyperedges\": []"));
        // Node fields.
        assert!(json.contains("\"file_type\": \"code\""));
        assert!(json.contains("\"_origin\": \"ast\""));
        assert!(json.contains("\"norm_label\": \"helper\""));
        // Edge fields: a calls edge with confidence + score.
        assert!(json.contains("\"relation\": \"calls\""));
        assert!(json.contains("\"confidence\": \"EXTRACTED\""));
        assert!(json.contains("\"confidence_score\": 1.0"));
        // _src/_tgt must never appear.
        assert!(!json.contains("_src"));
        assert!(!json.contains("_tgt"));
        // No trailing newline.
        assert!(!json.ends_with('\n'));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn report_lists_god_nodes_and_summary() {
        let root = workspace(&[(
            "lib.rs",
            "fn core() {}\nfn a() { core(); }\nfn b() { core(); }\n",
        )]);
        let index = Index::build(&root).expect("build");
        let communities = community::detect(index.graph());
        let report = to_report(index.graph(), &communities, "demo", "2026-06-16");

        assert!(report.starts_with("# Graph Report - demo  (2026-06-16)"));
        assert!(report.contains("## Summary"));
        assert!(report.contains("## God Nodes"));
        assert!(
            report.contains("`core`"),
            "core should be a god node:\n{report}"
        );
        assert!(report.contains("## Import Cycles"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn graph_html_is_self_contained_and_safe() {
        use super::to_graph_html;
        let root = workspace(&[("lib.rs", "fn helper() {}\nfn caller() { helper(); }\n")]);
        let index = Index::build(&root).expect("build");
        let communities = community::detect(index.graph());
        let html = to_graph_html(index.graph(), &communities, &root, "demo");

        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("Code Graph — demo"));
        // The data block is present and carries a known node label.
        assert!(html.contains("id=\"graph-data\""));
        assert!(html.contains("\"helper\""));
        // Fully offline: no external script/style/link references.
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(!html.contains("src=\"http"));
        // The injected JSON must never contain a literal </script that could close
        // the data block early (the </ escape turns it into <\/).
        let data_start =
            html.find("id=\"graph-data\">").expect("data block") + "id=\"graph-data\">".len();
        let data_end = html[data_start..]
            .find("</script>")
            .expect("data block end")
            + data_start;
        assert!(
            !html[data_start..data_end].contains("</"),
            "data must not contain a raw </"
        );
        // Placeholders all substituted.
        assert!(
            !html.contains("__GRAPH_DATA__")
                && !html.contains("__TITLE__")
                && !html.contains("__TRUNCATED__")
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn json_escapes_non_ascii() {
        // A symbol with a non-ASCII name must be \u-escaped (ensure_ascii parity).
        let root = workspace(&[("lib.rs", "fn naïve() {}\n")]);
        let index = Index::build(&root).expect("build");
        let json = to_graph_json(index.graph(), &[], &root);
        assert!(
            json.contains("na\\u00efve"),
            "non-ascii must be escaped:\n{json}"
        );
        assert!(!json.contains("naïve"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ambiguous_confidence_score_serializes_as_python_repr() {
        // Two files define `dup`; a third calls it with no local def → AMBIGUOUS
        // edges whose confidence_score (0.2) must serialize as "0.2", not the
        // f32→f64 widening "0.20000000298...".
        let root = workspace(&[
            ("a.rs", "pub fn dup() {}\n"),
            ("b.rs", "pub fn dup() {}\n"),
            ("c.rs", "fn caller() { dup(); }\n"),
        ]);
        let index = Index::build(&root).expect("build");
        let json = to_graph_json(index.graph(), &[], &root);
        assert!(
            json.contains("\"confidence\": \"AMBIGUOUS\""),
            "expected an ambiguous edge:\n{json}"
        );
        assert!(
            json.contains("\"confidence_score\": 0.2"),
            "0.2 must be exact:\n{json}"
        );
        assert!(
            !json.contains("0.20000"),
            "no f32-widening artifact:\n{json}"
        );

        std::fs::remove_dir_all(&root).ok();
    }
}
