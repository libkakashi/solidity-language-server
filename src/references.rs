use std::path::Path;

use tower_lsp::lsp_types::{Location, Position, Url};

use crate::symbol_table::{DeclId, SymbolTable};
use crate::utils::{LineIndex, SourceCache};

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

    // Collect all references using the reverse index. (Fix #18)
    let refs = st.find_references(&decl_id);
    let mut cache = SourceCache::new(file, source, line_index);

    // Include the declaration itself.
    if include_declaration {
        if let Some(loc) = decl_id_to_location(st, &decl_id, &mut cache) {
            locations.push(loc);
        }
    }

    for (path, start, end) in &refs {
        if let Some((ref_source, ref_li)) = cache.get(path.as_path()) {
            if let Ok(uri) = Url::from_file_path(path) {
                let range = ref_li.byte_range_to_lsp_range(ref_source, *start, *end);
                locations.push(Location { uri, range });
            }
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

fn decl_id_to_location<'a>(
    st: &'a SymbolTable,
    decl_id: &DeclId,
    cache: &mut SourceCache<'a>,
) -> Option<Location> {
    let decl = st.get_declaration(decl_id)?;
    let decl_path = st.resolve_path(decl.id.file);
    let (src, li) = cache.get(decl_path)?;
    let uri = Url::from_file_path(decl_path).ok()?;
    let range = li.byte_range_to_lsp_range(src, decl.name_range.0, decl.name_range.1);
    Some(Location { uri, range })
}
