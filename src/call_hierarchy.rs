use std::path::Path;

use tower_lsp::lsp_types::*;

use crate::symbol_table::{DeclId, DeclKind, Declaration, FileId, SymbolTable};
use crate::utils::LineIndex;

/// Prepare a call hierarchy item for the function/modifier at the cursor.
pub fn prepare(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Option<Vec<CallHierarchyItem>> {
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);
    let decl = st.resolve_at(file, byte_offset)?;

    if !is_callable(decl.kind()) {
        return None;
    }

    let item = decl_to_item(st, decl, file, source, line_index)?;
    Some(vec![item])
}

/// Find all incoming calls (callers) of the given call hierarchy item.
pub fn incoming_calls(
    st: &SymbolTable,
    item: &CallHierarchyItem,
) -> Vec<CallHierarchyIncomingCall> {
    let uri = &item.uri;
    let file_path = match uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_id = match st.lookup_file_id(&file_path) {
        Some(id) => id,
        None => return vec![],
    };

    // Find the declaration matching this item.
    let decl_id = match find_decl_by_name_and_range(st, file_id, &item.name, item.selection_range) {
        Some(id) => id,
        None => return vec![],
    };

    // Get all references to this declaration.
    let refs = st.find_references(&decl_id);

    // Group references by their enclosing function.
    let mut callers: Vec<(DeclId, Vec<Range>)> = Vec::new();

    for (ref_path, start, end) in &refs {
        let ref_file_id = match st.lookup_file_id(ref_path) {
            Some(id) => id,
            None => continue,
        };

        // Find the enclosing function for this reference.
        if let Some(enclosing_id) = find_enclosing_callable(st, ref_file_id, *start) {
            // Get the source and line index for this file.
            let (ref_source, ref_li) = match read_file_source(st, ref_file_id, ref_path) {
                Some(v) => v,
                None => continue,
            };
            let range = ref_li.byte_range_to_lsp_range(&ref_source, *start, *end);

            if let Some(entry) = callers.iter_mut().find(|(id, _)| *id == enclosing_id) {
                entry.1.push(range);
            } else {
                callers.push((enclosing_id, vec![range]));
            }
        }
    }

    // Convert to CallHierarchyIncomingCall items.
    let mut results = Vec::new();
    for (caller_id, from_ranges) in callers {
        if let Some(caller_decl) = st.get_declaration(&caller_id) {
            let caller_path = st.resolve_path(caller_id.file);
            if let Some((src, li)) = read_file_source(st, caller_id.file, caller_path) {
                if let Some(item) = decl_to_item(st, caller_decl, caller_path, &src, &li) {
                    results.push(CallHierarchyIncomingCall {
                        from: item,
                        from_ranges,
                    });
                }
            }
        }
    }

    results
}

/// Find all outgoing calls (callees) from the given call hierarchy item.
pub fn outgoing_calls(
    st: &SymbolTable,
    item: &CallHierarchyItem,
) -> Vec<CallHierarchyOutgoingCall> {
    let uri = &item.uri;
    let file_path = match uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_id = match st.lookup_file_id(&file_path) {
        Some(id) => id,
        None => return vec![],
    };

    // Find the declaration matching this item.
    let decl_id = match find_decl_by_name_and_range(st, file_id, &item.name, item.selection_range) {
        Some(id) => id,
        None => return vec![],
    };

    let decl = match st.get_declaration(&decl_id) {
        Some(d) => d,
        None => return vec![],
    };

    let fi = match st.files.get(&file_id) {
        Some(fi) => fi,
        None => return vec![],
    };

    let source = match st.get_source(file_id) {
        Some(s) => s,
        None => return vec![],
    };
    let line_index = LineIndex::new(source);

    // Find all references within this function's body that resolve to callable declarations.
    let body_start = decl.full_range.0;
    let body_end = decl.full_range.1;

    let mut callees: Vec<(DeclId, Vec<Range>)> = Vec::new();

    for reference in &fi.references {
        let ref_start = reference.range.0 as usize;
        if ref_start < body_start || ref_start >= body_end {
            continue;
        }

        let resolved_id = match &reference.resolved {
            Some(id) => *id,
            None => continue,
        };

        let target = match st.get_declaration(&resolved_id) {
            Some(d) => d,
            None => continue,
        };

        if !is_callable(target.kind()) {
            continue;
        }

        // Skip if this is the same declaration (recursive self-reference at definition).
        if resolved_id == decl_id {
            // Allow recursive calls but skip the definition name itself.
            if ref_start == decl.name_range.0 {
                continue;
            }
        }

        let range = line_index.byte_range_to_lsp_range(
            source,
            reference.range.0 as usize,
            reference.range.1 as usize,
        );

        if let Some(entry) = callees.iter_mut().find(|(id, _)| *id == resolved_id) {
            entry.1.push(range);
        } else {
            callees.push((resolved_id, vec![range]));
        }
    }

    // Convert to CallHierarchyOutgoingCall items.
    let mut results = Vec::new();
    for (callee_id, from_ranges) in callees {
        if let Some(callee_decl) = st.get_declaration(&callee_id) {
            let callee_path = st.resolve_path(callee_id.file);
            if let Some((src, li)) = read_file_source(st, callee_id.file, callee_path) {
                if let Some(item) = decl_to_item(st, callee_decl, callee_path, &src, &li) {
                    results.push(CallHierarchyOutgoingCall {
                        to: item,
                        from_ranges,
                    });
                }
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_callable(kind: DeclKind) -> bool {
    matches!(
        kind,
        DeclKind::Function | DeclKind::Constructor | DeclKind::Modifier | DeclKind::FallbackReceive
    )
}

fn decl_to_item(
    st: &SymbolTable,
    decl: &Declaration,
    file: &Path,
    source: &str,
    line_index: &LineIndex,
) -> Option<CallHierarchyItem> {
    let uri = Url::from_file_path(file).ok()?;
    let target_path = st.resolve_path(decl.id.file);

    let (src, li);
    let (use_source, use_li) = if target_path == file {
        (source, line_index)
    } else {
        let content = std::fs::read_to_string(target_path).ok()?;
        li = LineIndex::new(&content);
        src = content;
        (src.as_str(), &li)
    };

    let selection_range =
        use_li.byte_range_to_lsp_range(use_source, decl.name_range.0, decl.name_range.1);
    let range = use_li.byte_range_to_lsp_range(use_source, decl.full_range.0, decl.full_range.1);

    let target_uri = if target_path == file {
        uri
    } else {
        Url::from_file_path(target_path).ok()?
    };

    let kind = match decl.kind() {
        DeclKind::Function => SymbolKind::FUNCTION,
        DeclKind::Constructor => SymbolKind::CONSTRUCTOR,
        DeclKind::Modifier => SymbolKind::FUNCTION,
        DeclKind::FallbackReceive => SymbolKind::FUNCTION,
        _ => SymbolKind::FUNCTION,
    };

    Some(CallHierarchyItem {
        name: decl.name.clone(),
        kind,
        tags: None,
        detail: build_detail(decl),
        uri: target_uri,
        range,
        selection_range,
        data: None,
    })
}

fn build_detail(decl: &Declaration) -> Option<String> {
    let params = decl.parameters();
    if params.is_empty() {
        return None;
    }
    let param_str = params
        .iter()
        .map(|(t, n)| {
            if n.is_empty() {
                t.clone()
            } else {
                format!("{t} {n}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("({param_str})"))
}

/// Find a declaration by name and selection range within a file.
fn find_decl_by_name_and_range(
    st: &SymbolTable,
    file_id: FileId,
    name: &str,
    selection_range: Range,
) -> Option<DeclId> {
    let fi = st.files.get(&file_id)?;
    let source = st.get_source(file_id)?;
    let li = LineIndex::new(source);

    for decl in fi.declarations.values() {
        if decl.name == name {
            let decl_range =
                li.byte_range_to_lsp_range(source, decl.name_range.0, decl.name_range.1);
            if decl_range == selection_range {
                return Some(decl.id);
            }
        }
    }
    None
}

/// Find the enclosing callable (function/constructor/modifier) for a byte offset.
fn find_enclosing_callable(
    st: &SymbolTable,
    file_id: FileId,
    byte_offset: usize,
) -> Option<DeclId> {
    let fi = st.files.get(&file_id)?;

    let mut best: Option<&Declaration> = None;

    for decl in fi.declarations.values() {
        if !is_callable(decl.kind()) {
            continue;
        }
        if decl.full_range.0 <= byte_offset && byte_offset < decl.full_range.1 {
            // Pick the most specific (smallest) enclosing callable.
            if let Some(current_best) = best {
                let current_size = current_best.full_range.1 - current_best.full_range.0;
                let new_size = decl.full_range.1 - decl.full_range.0;
                if new_size < current_size {
                    best = Some(decl);
                }
            } else {
                best = Some(decl);
            }
        }
    }

    best.map(|d| d.id)
}

/// Read file source, preferring the cached source in SymbolTable.
fn read_file_source(
    st: &SymbolTable,
    file_id: FileId,
    file_path: &Path,
) -> Option<(String, LineIndex)> {
    if let Some(cached) = st.get_source(file_id) {
        let li = LineIndex::new(cached);
        return Some((cached.to_string(), li));
    }
    let source = std::fs::read_to_string(file_path).ok()?;
    let li = LineIndex::new(&source);
    Some((source, li))
}
