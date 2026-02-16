use std::path::Path;

use tower_lsp::lsp_types::{Location, Position, Range, Url};

use crate::symbol_table::SymbolTable;
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
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 0,
                    },
                },
            });
        }
    }

    // Resolve identifier at cursor to its declaration.
    let decl = st.resolve_at(file, byte_offset)?;
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

    let (start_line, start_col) = target_li.byte_offset_to_position(target_src, decl.name_range.0);
    let (end_line, end_col) = target_li.byte_offset_to_position(target_src, decl.name_range.1);
    let uri = Url::from_file_path(target_path).ok()?;

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
