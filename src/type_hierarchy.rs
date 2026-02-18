use std::path::Path;

use tower_lsp::lsp_types::*;

use crate::symbol_table::{DeclKind, Declaration, FileId, SymbolTable};
use crate::utils::LineIndex;

/// Prepare a type hierarchy item for the contract/interface/library at the cursor.
pub fn prepare(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Option<Vec<TypeHierarchyItem>> {
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);
    let decl = st.resolve_at(file, byte_offset)?;

    if !is_type_kind(decl.kind()) {
        return None;
    }

    let item = decl_to_item(st, decl, file, source, line_index)?;
    Some(vec![item])
}

/// Find all supertypes (base contracts) of the given type hierarchy item.
pub fn supertypes(st: &SymbolTable, item: &TypeHierarchyItem) -> Vec<TypeHierarchyItem> {
    let file_path = match item.uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_id = match st.lookup_file_id(&file_path) {
        Some(id) => id,
        None => return vec![],
    };

    let decl_id = match find_decl_by_name_and_range(st, file_id, &item.name, item.selection_range) {
        Some(id) => id,
        None => return vec![],
    };

    let decl = match st.get_declaration(&decl_id) {
        Some(d) => d,
        None => return vec![],
    };

    let mut results = Vec::new();
    for base_name in decl.base_contracts() {
        // Try to find the base contract declaration.
        if let Some(base_decl_id) = find_type_by_name(st, file_id, base_name) {
            if let Some(base_decl) = st.get_declaration(&base_decl_id) {
                let base_path = st.resolve_path(base_decl_id.file);
                if let Some((src, li)) = read_file_source(st, base_decl_id.file, base_path) {
                    if let Some(item) = decl_to_item(st, base_decl, base_path, &src, &li) {
                        results.push(item);
                    }
                }
            }
        }
    }
    results
}

/// Find all subtypes (derived contracts) of the given type hierarchy item.
pub fn subtypes(st: &SymbolTable, item: &TypeHierarchyItem) -> Vec<TypeHierarchyItem> {
    let file_path = match item.uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_id = match st.lookup_file_id(&file_path) {
        Some(id) => id,
        None => return vec![],
    };

    let _decl_id = match find_decl_by_name_and_range(st, file_id, &item.name, item.selection_range)
    {
        Some(id) => id,
        None => return vec![],
    };

    let target_name = &item.name;
    let mut results = Vec::new();

    // Search all indexed files for contracts that inherit from this one.
    for (&fid, fi) in &st.files {
        for decl in fi.declarations.values() {
            if is_type_kind(decl.kind()) && decl.base_contracts().iter().any(|b| b == target_name) {
                let decl_path = st.resolve_path(fid);
                if let Some((src, li)) = read_file_source(st, fid, decl_path) {
                    if let Some(item) = decl_to_item(st, decl, decl_path, &src, &li) {
                        results.push(item);
                    }
                }
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_type_kind(kind: DeclKind) -> bool {
    matches!(
        kind,
        DeclKind::Contract | DeclKind::Interface | DeclKind::Library | DeclKind::Struct
    )
}

fn decl_to_item(
    st: &SymbolTable,
    decl: &Declaration,
    file: &Path,
    source: &str,
    line_index: &LineIndex,
) -> Option<TypeHierarchyItem> {
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

    let target_uri = Url::from_file_path(target_path).ok()?;

    let kind = match decl.kind() {
        DeclKind::Contract => SymbolKind::CLASS,
        DeclKind::Interface => SymbolKind::INTERFACE,
        DeclKind::Library => SymbolKind::NAMESPACE,
        DeclKind::Struct => SymbolKind::STRUCT,
        _ => SymbolKind::CLASS,
    };

    let detail = if decl.base_contracts().is_empty() {
        None
    } else {
        Some(format!("is {}", decl.base_contracts().join(", ")))
    };

    Some(TypeHierarchyItem {
        name: decl.name.clone(),
        kind,
        tags: None,
        detail,
        uri: target_uri,
        range,
        selection_range,
        data: None,
    })
}

fn find_decl_by_name_and_range(
    st: &SymbolTable,
    file_id: FileId,
    name: &str,
    selection_range: Range,
) -> Option<crate::symbol_table::DeclId> {
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

/// Find a type declaration by name, searching the given file and its imports.
fn find_type_by_name(
    st: &SymbolTable,
    file_id: FileId,
    name: &str,
) -> Option<crate::symbol_table::DeclId> {
    // Use the symbol table's find_type_decl which handles imports.
    let file_path = st.resolve_path(file_id);
    st.find_type_decl(file_path, name)
}

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
