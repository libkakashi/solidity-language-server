use std::path::Path;

use tower_lsp::lsp_types::{Location, Position, Range, Url};

use crate::symbol_table::SymbolTable;
use crate::utils::{byte_offset_to_position, position_to_byte_offset};

/// Goto definition: resolve the identifier at `position` to its declaration location.
pub fn goto_definition(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
) -> Option<Location> {
    let byte_offset = position_to_byte_offset(source, position.line, position.character);

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
    let target_source = if target_path == file {
        source.to_string()
    } else {
        std::fs::read_to_string(target_path).ok()?
    };

    let (start_line, start_col) = byte_offset_to_position(&target_source, decl.name_range.0);
    let (end_line, end_col) = byte_offset_to_position(&target_source, decl.name_range.1);
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
