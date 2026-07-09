use std::collections::HashSet;

pub fn is_parallel_safe_read_tool(name: &str) -> bool {
    matches!(
        name,
        "Read"
            | "Grep"
            | "Glob"
            | "InspectFile"
            | "SymbolContext"
            | "SemanticSearch"
            | "RelatedFiles"
            | "RepoMap"
            | "WorkspaceIndex"
            | "GitContext"
            | "DiagnosticsContext"
            | "ReadLints"
            | "RulesContext"
            | "DocsContext"
            | "MemoryContext"
            | "RecallMemory"
            | "ListSkills"
            | "ActiveContext"
            | "CodeGraphDefinition"
            | "CodeGraphCallers"
            | "CodeGraphCallees"
            | "CodeGraphExplain"
            | "CodeGraphOverview"
    )
}

pub fn parallel_task_call_ids<'a>(
    calls: &'a [(&'a str, &'a str)],
) -> Vec<&'a str> {
    let all_ids: HashSet<&str> = calls.iter().map(|(id, _)| *id).collect();
    if all_ids.len() != calls.len() {
        return Vec::new();
    }
    let task_ids: Vec<&str> = calls
        .iter()
        .filter(|(_, name)| *name == "Task")
        .map(|(id, _)| *id)
        .collect();
    let read_ids: Vec<&str> = calls
        .iter()
        .filter(|(_, name)| is_parallel_safe_read_tool(name))
        .map(|(id, _)| *id)
        .collect();
    let mut ids = Vec::new();
    if task_ids.len() >= 2 {
        ids.extend(task_ids);
    }
    if read_ids.len() >= 2 {
        ids.extend(read_ids);
    }
    ids
}
