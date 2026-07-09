use aspect_codegraph::CodeGraph;

use crate::path::normalize_slashes;
use crate::types::AiSemanticResult;

pub fn apply_graph_boost(graph: &CodeGraph, ranked: &mut [AiSemanticResult]) {
    let mut path_to_file: std::collections::HashMap<String, aspect_codegraph::FileId> =
        std::collections::HashMap::with_capacity(graph.file_count());
    for (file_id, path) in graph.files() {
        path_to_file.insert(
            normalize_slashes(&path.to_string_lossy()).to_lowercase(),
            file_id,
        );
    }

    for result in ranked.iter_mut() {
        let degree = graph_degree_for(graph, &path_to_file, result);
        if degree > 0 {
            result.score += degree_to_boost(degree);
        }
    }
}

fn graph_degree_for(
    graph: &CodeGraph,
    path_to_file: &std::collections::HashMap<String, aspect_codegraph::FileId>,
    result: &AiSemanticResult,
) -> u32 {
    let result_path = result.path.to_lowercase();

    if let Some(name) = result.name.as_deref().filter(|name| !name.is_empty()) {
        let nodes = graph.nodes_by_name(name);
        if !nodes.is_empty() {
            let mut best = 0u32;
            let mut same_file: Option<u32> = None;
            for &node in nodes {
                let degree = aspect_codegraph::degree(graph, node);
                best = best.max(degree);
                let in_result_file = graph
                    .node(node)
                    .and_then(|n| graph.file_path(n.file))
                    .is_some_and(|path| {
                        normalize_slashes(&path.to_string_lossy()).to_lowercase() == result_path
                    });
                if in_result_file {
                    same_file = Some(same_file.map_or(degree, |d| d.max(degree)));
                }
            }
            return same_file.unwrap_or(best);
        }
    }

    if let Some(&file_id) = path_to_file.get(&result_path) {
        return graph
            .nodes_in_file(file_id)
            .iter()
            .map(|&node| aspect_codegraph::degree(graph, node))
            .max()
            .unwrap_or(0);
    }

    0
}

const fn degree_to_boost(degree: u32) -> i64 {
    match degree {
        0 => 0,
        1..=2 => 6,
        3..=5 => 14,
        6..=10 => 24,
        11..=20 => 34,
        _ => 45,
    }
}
