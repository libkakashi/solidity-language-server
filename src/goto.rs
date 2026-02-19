use std::path::Path;

use tower_lsp::lsp_types::{Location, Position, Url};

use crate::symbol_table::{DeclKind, SymbolTable};
use crate::utils::LineIndex;

/// Goto definition: resolve the identifier at `position` to its declaration location.
pub fn goto_definition(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Option<Location> {
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);

    // Check if cursor is on an import path string -> jump to the resolved file.
    if let Some(imp) = st.import_at(file, byte_offset) {
        if let Some(ref resolved) = imp.resolved_path {
            let uri = Url::from_file_path(resolved).ok()?;
            return Some(Location {
                uri,
                range: Default::default(),
            });
        }
    }

    // Resolve identifier at cursor to its declaration, with overload disambiguation.
    let decl = resolve_with_overloads(st, file, source, byte_offset)?;
    let target_path = st.resolve_path(decl.id.file);

    // Read the target file source to convert byte offsets to positions.
    let (target_source_owned, target_src, target_li_owned);
    let target_li;
    if target_path == file {
        target_src = source;
        target_li = line_index;
    } else {
        target_source_owned = std::fs::read_to_string(target_path).ok()?;
        target_src = &target_source_owned;
        target_li_owned = LineIndex::new(target_src);
        target_li = &target_li_owned;
    };

    let uri = Url::from_file_path(target_path).ok()?;
    let range = target_li.byte_range_to_lsp_range(target_src, decl.name_range.0, decl.name_range.1);

    Some(Location { uri, range })
}

/// Go to type definition: resolve the identifier at `position` and navigate
/// to the declaration of its type (e.g., from a variable to its struct/contract type).
pub fn goto_type_definition(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Option<Location> {
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);
    let decl = st.resolve_at(file, byte_offset)?;

    // Extract the type text from the declaration.
    let type_text = match decl.kind() {
        DeclKind::StateVariable
        | DeclKind::LocalVariable
        | DeclKind::Parameter
        | DeclKind::Constant => decl.type_text()?,
        // For functions, use the return type if there's exactly one.
        DeclKind::Function => {
            let ret = decl.return_parameters();
            if ret.len() == 1 {
                &ret[0].0
            } else {
                return None;
            }
        }
        // If already on a type, navigate to it.
        DeclKind::Contract
        | DeclKind::Interface
        | DeclKind::Library
        | DeclKind::Struct
        | DeclKind::Enum
        | DeclKind::UserDefinedType => {
            return make_location(st, &decl.id, file, source, line_index);
        }
        _ => return None,
    };

    // Strip array brackets, memory/storage/calldata suffixes.
    let base_type = strip_type_for_lookup(type_text);

    // Resolve the type name to a declaration.
    let type_decl_id = st.find_type_decl(file, base_type)?;
    make_location(st, &type_decl_id, file, source, line_index)
}

/// Go to implementation: given a contract/interface, find all contracts that
/// list it in their inheritance chain.
pub fn goto_implementation(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Vec<Location> {
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);
    let decl = match st.resolve_at(file, byte_offset) {
        Some(d) => d,
        None => return vec![],
    };

    // Only meaningful for contracts, interfaces, and libraries.
    let target_name = match decl.kind() {
        DeclKind::Contract | DeclKind::Interface | DeclKind::Library => &decl.name,
        // For functions, find overrides/implementations in derived contracts.
        DeclKind::Function => {
            return find_function_implementations(st, decl, file, source, line_index);
        }
        _ => return vec![],
    };

    let mut locations = Vec::new();

    // Search all indexed files for contracts that inherit from this one.
    for (_, fi) in &st.files {
        for d in fi.declarations.values() {
            if matches!(
                d.kind(),
                DeclKind::Contract | DeclKind::Interface | DeclKind::Library
            ) {
                if d.base_contracts().iter().any(|b| b == target_name) {
                    let fi_path = st.resolve_path(fi.file_id);
                    if let Some(loc) = make_location_for_decl(st, d, fi_path) {
                        locations.push(loc);
                    }
                }
            }
        }
    }

    locations
}

/// Find implementations/overrides of a function in derived contracts.
fn find_function_implementations(
    st: &SymbolTable,
    decl: &crate::symbol_table::Declaration,
    _file: &Path,
    _source: &str,
    _line_index: &LineIndex,
) -> Vec<Location> {
    let func_name = &decl.name;
    let mut locations = Vec::new();

    // Find the enclosing contract name.
    let parent_contract = st.get_declaration(&decl.id).and_then(|d| {
        let fi = st.files.get(&d.id.file)?;
        let scope = fi.scopes.get(d.scope)?;
        let owner_id = scope.owner?;
        st.get_declaration(&owner_id).map(|c| c.name.clone())
    });

    let parent_name = match parent_contract {
        Some(n) => n,
        None => return locations,
    };

    // Search for contracts that inherit from the parent.
    for (_, fi) in &st.files {
        for d in fi.declarations.values() {
            if matches!(
                d.kind(),
                DeclKind::Contract | DeclKind::Interface | DeclKind::Library
            ) {
                if d.base_contracts().iter().any(|b| *b == parent_name) {
                    // Check if this contract has a function with the same name.
                    for member in d.members() {
                        if member.kind == DeclKind::Function && member.name == *func_name {
                            let fi_path = st.resolve_path(fi.file_id);
                            if let Some(decl_id) = member.decl_id {
                                if let Some(target) = st.get_declaration(&decl_id) {
                                    if let Some(loc) = make_location_for_decl(st, target, fi_path) {
                                        locations.push(loc);
                                    }
                                }
                            } else if let Some(loc) =
                                make_location_from_range(fi_path, member.name_range)
                            {
                                locations.push(loc);
                            }
                        }
                    }
                }
            }
        }
    }

    locations
}

// ---------------------------------------------------------------------------
// Overload resolution for go-to-definition
// ---------------------------------------------------------------------------

/// Resolve a symbol at `byte_offset`, disambiguating overloads by counting
/// the number of arguments at the call site (if any).
fn resolve_with_overloads<'a>(
    st: &'a SymbolTable,
    file: &Path,
    source: &str,
    byte_offset: usize,
) -> Option<&'a crate::symbol_table::Declaration> {
    let decl = st.resolve_at(file, byte_offset)?;

    // Only try overload resolution for callable declarations.
    if !matches!(
        decl.kind(),
        DeclKind::Function | DeclKind::Event | DeclKind::Error
    ) {
        return Some(decl);
    }

    let overloads = st.find_overloads(file, byte_offset);
    if overloads.len() <= 1 {
        return Some(decl);
    }

    // Count arguments at the call site by scanning source text.
    if let Some(arg_count) = count_call_args(source, byte_offset) {
        // Find the best matching overload by parameter count.
        // Prefer exact match, then closest with >= params.
        let mut best = None;
        let mut best_diff = i32::MAX;
        for overload in &overloads {
            let param_count = overload.parameters().len() as i32;
            let diff = (param_count - arg_count as i32).abs();
            if diff < best_diff || (diff == best_diff && param_count == arg_count as i32) {
                best_diff = diff;
                best = Some(*overload);
            }
        }
        return best.or(Some(decl));
    }

    Some(decl)
}

/// Count the number of arguments at a call site by scanning forward from the
/// identifier at `byte_offset` for the opening `(` and counting commas.
fn count_call_args(source: &str, byte_offset: usize) -> Option<usize> {
    let bytes = source.as_bytes();

    // Skip past the identifier.
    let mut pos = byte_offset;
    while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
        pos += 1;
    }

    // Skip whitespace.
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }

    // Must find opening paren.
    if pos >= bytes.len() || bytes[pos] != b'(' {
        return None;
    }
    pos += 1; // skip '('

    // Count commas at depth 0 to determine argument count.
    let mut depth: i32 = 1;
    let mut commas: usize = 0;
    let mut has_content = false;

    while pos < bytes.len() && depth > 0 {
        match bytes[pos] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            b',' if depth == 1 => commas += 1,
            b if !b.is_ascii_whitespace() && depth == 1 => has_content = true,
            _ => {}
        }
        pos += 1;
    }

    if !has_content && commas == 0 {
        Some(0) // empty argument list
    } else {
        Some(commas + 1)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_location(
    st: &SymbolTable,
    decl_id: &crate::symbol_table::DeclId,
    current_file: &Path,
    current_source: &str,
    current_line_index: &LineIndex,
) -> Option<Location> {
    let decl = st.get_declaration(decl_id)?;
    let target_path = st.resolve_path(decl.id.file);

    let (target_src, target_li_owned);
    let target_li;
    if target_path == current_file {
        target_li = current_line_index;
        let range =
            target_li.byte_range_to_lsp_range(current_source, decl.name_range.0, decl.name_range.1);
        let uri = Url::from_file_path(target_path).ok()?;
        return Some(Location { uri, range });
    }

    target_src = std::fs::read_to_string(target_path).ok()?;
    target_li_owned = LineIndex::new(&target_src);
    target_li = &target_li_owned;

    let uri = Url::from_file_path(target_path).ok()?;
    let range =
        target_li.byte_range_to_lsp_range(&target_src, decl.name_range.0, decl.name_range.1);
    Some(Location { uri, range })
}

fn make_location_for_decl(
    _st: &SymbolTable,
    decl: &crate::symbol_table::Declaration,
    file_path: &Path,
) -> Option<Location> {
    let source = std::fs::read_to_string(file_path).ok()?;
    let li = LineIndex::new(&source);
    let uri = Url::from_file_path(file_path).ok()?;
    let range = li.byte_range_to_lsp_range(&source, decl.name_range.0, decl.name_range.1);
    Some(Location { uri, range })
}

fn make_location_from_range(file_path: &Path, name_range: (usize, usize)) -> Option<Location> {
    let source = std::fs::read_to_string(file_path).ok()?;
    let li = LineIndex::new(&source);
    let uri = Url::from_file_path(file_path).ok()?;
    let range = li.byte_range_to_lsp_range(&source, name_range.0, name_range.1);
    Some(Location { uri, range })
}

/// Strip array brackets and memory/storage/calldata/payable suffixes for type lookup.
fn strip_type_for_lookup(type_text: &str) -> &str {
    let s = type_text.trim();
    let s = s
        .strip_suffix(" memory")
        .or_else(|| s.strip_suffix(" storage"))
        .or_else(|| s.strip_suffix(" calldata"))
        .or_else(|| s.strip_suffix(" payable"))
        .unwrap_or(s);
    if let Some(bracket_pos) = s.find('[') {
        &s[..bracket_pos]
    } else {
        s
    }
    .trim()
}
