use aspect_core::LspDocumentSymbol;

pub fn truncate_document_symbols(symbols: &mut Vec<LspDocumentSymbol>, max_results: usize) {
    let mut remaining = max_results;
    symbols.retain_mut(|symbol| retain_symbol_with_budget(symbol, &mut remaining));
}

pub fn count_document_symbols(symbols: &[LspDocumentSymbol]) -> usize {
    symbols
        .iter()
        .map(|symbol| 1 + count_document_symbols(&symbol.children))
        .sum()
}

fn retain_symbol_with_budget(symbol: &mut LspDocumentSymbol, remaining: &mut usize) -> bool {
    if *remaining == 0 {
        return false;
    }
    *remaining -= 1;
    symbol
        .children
        .retain_mut(|child| retain_symbol_with_budget(child, remaining));
    true
}

pub fn filter_document_symbols(
    symbols: &[LspDocumentSymbol],
    query: &str,
    max_results: usize,
) -> Vec<LspDocumentSymbol> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        let mut symbols = symbols.to_vec();
        truncate_document_symbols(&mut symbols, max_results);
        return symbols;
    }

    let mut remaining = max_results;
    symbols
        .iter()
        .filter_map(|symbol| filter_symbol_with_budget(symbol, &needle, &mut remaining))
        .collect()
}

fn filter_symbol_with_budget(
    symbol: &LspDocumentSymbol,
    needle: &str,
    remaining: &mut usize,
) -> Option<LspDocumentSymbol> {
    if *remaining == 0 {
        return None;
    }
    let matches = symbol.name.to_ascii_lowercase().contains(needle)
        || symbol
            .detail
            .as_deref()
            .is_some_and(|detail| detail.to_ascii_lowercase().contains(needle));
    *remaining -= 1;
    let children = symbol
        .children
        .iter()
        .filter_map(|child| filter_symbol_with_budget(child, needle, remaining))
        .collect::<Vec<_>>();
    if !matches && children.is_empty() {
        *remaining += 1;
        return None;
    }
    let mut filtered = symbol.clone();
    filtered.children = children;
    Some(filtered)
}
