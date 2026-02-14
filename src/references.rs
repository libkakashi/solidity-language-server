use std::collections::HashMap;
use std::path::Path;

use tower_lsp::lsp_types::{Location, Position, Range, Url};

use crate::symbol_table::{DeclId, SymbolTable};
use crate::utils::{byte_offset_to_position, position_to_byte_offset};

/// Find all references to the symbol at `position`.
pub fn find_references(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    include_declaration: bool,
) -> Vec<Location> {
    let byte_offset = position_to_byte_offset(source, position.line, position.character);

    let decl = match st.resolve_at(file, byte_offset) {
        Some(d) => d,
        None => return vec![],
    };

    let decl_id = decl.id;
    let mut locations = Vec::new();

    // Include the declaration itself.
    if include_declaration {
        if let Some(loc) = decl_id_to_location(st, &decl_id, file, source) {
            locations.push(loc);
        }
    }

    // Collect all references using the reverse index. (Fix #18)
    let refs = st.find_references(&decl_id);

    // Cache file reads to avoid reading the same file multiple times. (Fix #19)
    let mut source_cache: HashMap<&Path, String> = HashMap::new();

    for (path, start, end) in &refs {
        let ref_source = if path.as_path() == file {
            source
        } else {
            if !source_cache.contains_key(path.as_path()) {
                match std::fs::read_to_string(path) {
                    Ok(s) => {
                        source_cache.insert(path.as_path(), s);
                    }
                    Err(_) => continue,
                }
            }
            match source_cache.get(path.as_path()) {
                Some(s) => s.as_str(),
                None => continue,
            }
        };
        let (start_line, start_col) = byte_offset_to_position(ref_source, *start);
        let (end_line, end_col) = byte_offset_to_position(ref_source, *end);
        if let Ok(uri) = Url::from_file_path(path) {
            locations.push(Location {
                uri,
                range: Range {
                    start: Position {
                        line: start_line,
                        character: start_col,
                    },
                    end: Position {
                        line: end_line,
                        character: end_col,
                    },
                },
            });
        }
    }

    // Deduplicate by (uri, range).
    let mut seen = std::collections::HashSet::new();
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
) -> Option<Location> {
    let decl = st.get_declaration(decl_id)?;
    let decl_path = st.resolve_path(decl.id.file);
    let source = if decl_path == current_file {
        current_source.to_string()
    } else {
        std::fs::read_to_string(decl_path).ok()?
    };
    let (start_line, start_col) = byte_offset_to_position(&source, decl.name_range.0);
    let (end_line, end_col) = byte_offset_to_position(&source, decl.name_range.1);
    let uri = Url::from_file_path(decl_path).ok()?;
    Some(Location {
        uri,
        range: Range {
            start: Position {
                line: start_line,
                character: start_col,
            },
            end: Position {
                line: end_line,
                character: end_col,
            },
        },
    })
}
