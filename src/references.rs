use std::path::Path;

use rustc_hash::FxHashMap;
use tower_lsp::lsp_types::{Location, Position, Url};

use crate::symbol_table::{DeclId, SymbolTable};
use crate::utils::LineIndex;

/// Find all references to the symbol at `position`.
pub fn find_references(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    include_declaration: bool,
    line_index: &LineIndex,
) -> Vec<Location> {
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);

    let decl = match st.resolve_at(file, byte_offset) {
        Some(d) => d,
        None => return vec![],
    };

    let decl_id = decl.id;
    let mut locations = Vec::new();

    // Include the declaration itself.
    if include_declaration {
        if let Some(loc) = decl_id_to_location(st, &decl_id, file, source, line_index) {
            locations.push(loc);
        }
    }

    // Collect all references using the reverse index. (Fix #18)
    let refs = st.find_references(&decl_id);

    // Cache file reads + LineIndex to avoid re-reading and re-indexing. (Fix #19)
    let mut source_cache: FxHashMap<&Path, (String, LineIndex)> = FxHashMap::default();

    for (path, start, end) in &refs {
        let (ref_source, ref_li) = if path.as_path() == file {
            (source, line_index)
        } else {
            if !source_cache.contains_key(path.as_path()) {
                match std::fs::read_to_string(path) {
                    Ok(s) => {
                        let li = LineIndex::new(&s);
                        source_cache.insert(path.as_path(), (s, li));
                    }
                    Err(_) => continue,
                }
            }
            match source_cache.get(path.as_path()) {
                Some((s, li)) => (s.as_str(), li),
                None => continue,
            }
        };
        if let Ok(uri) = Url::from_file_path(path) {
            let range = ref_li.byte_range_to_lsp_range(ref_source, *start, *end);
            locations.push(Location { uri, range });
        }
    }

    // Deduplicate by (uri, range).
    let mut seen = rustc_hash::FxHashSet::default();
    locations.retain(|loc| {
        seen.insert((
            loc.uri.clone(),
            loc.range.start.line,
            loc.range.start.character,
            loc.range.end.line,
            loc.range.end.character,
        ))
    });

    locations
}

fn decl_id_to_location(
    st: &SymbolTable,
    decl_id: &DeclId,
    current_file: &Path,
    current_source: &str,
    current_line_index: &LineIndex,
) -> Option<Location> {
    let decl = st.get_declaration(decl_id)?;
    let decl_path = st.resolve_path(decl.id.file);
    let (source_owned, src, li_owned);
    let li;
    if decl_path == current_file {
        src = current_source;
        li = current_line_index;
    } else {
        source_owned = std::fs::read_to_string(decl_path).ok()?;
        src = &source_owned;
        li_owned = LineIndex::new(src);
        li = &li_owned;
    };
    let uri = Url::from_file_path(decl_path).ok()?;
    let range = li.byte_range_to_lsp_range(src, decl.name_range.0, decl.name_range.1);
    Some(Location { uri, range })
}
