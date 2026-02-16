use std::collections::HashMap;
use std::path::Path;

use rustc_hash::FxHashMap;
use tower_lsp::lsp_types::{Position, Range, TextEdit, Url, WorkspaceEdit};

use crate::symbol_table::SymbolTable;
use crate::utils::LineIndex;

/// Get the identifier at a given position (for prepare-rename).
pub fn get_identifier_at_position(
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Option<String> {
    let abs_offset = line_index.position_to_byte_offset(source, position.line, position.character);
    let bytes = source.as_bytes();

    if abs_offset >= bytes.len() {
        return None;
    }

    let mut start = abs_offset;
    let mut end = abs_offset;

    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }

    if start == end {
        return None;
    }
    if bytes[start].is_ascii_digit() {
        return None;
    }

    Some(source[start..end].to_string())
}

/// Get the range of the identifier at position (for prepare-rename).
pub fn get_identifier_range(
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Option<Range> {
    let abs_offset = line_index.position_to_byte_offset(source, position.line, position.character);
    let bytes = source.as_bytes();

    if abs_offset >= bytes.len() {
        return None;
    }

    let mut start = abs_offset;
    let mut end = abs_offset;

    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }

    if start == end {
        return None;
    }
    if bytes[start].is_ascii_digit() {
        return None;
    }

    let (start_line, start_col) = line_index.byte_offset_to_position(source, start);
    let (end_line, end_col) = line_index.byte_offset_to_position(source, end);

    Some(Range {
        start: Position {
            line: start_line,
            character: start_col,
        },
        end: Position {
            line: end_line,
            character: end_col,
        },
    })
}

/// Rename the symbol at `position` to `new_name`.
pub fn rename_symbol(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    new_name: &str,
    line_index: &LineIndex,
) -> Option<WorkspaceEdit> {
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);

    let decl = st.resolve_at(file, byte_offset)?;
    let decl_id = decl.id;

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

    // Rename the declaration itself.
    {
        let decl_path = st.resolve_path(decl_id.file);
        let (decl_source_owned, decl_src, decl_li_owned);
        let decl_li;
        if decl_path == file {
            decl_src = source;
            decl_li = line_index;
        } else {
            decl_source_owned = std::fs::read_to_string(decl_path).ok()?;
            decl_src = &decl_source_owned;
            decl_li_owned = LineIndex::new(decl_src);
            decl_li = &decl_li_owned;
        };
        let (sl, sc) = decl_li.byte_offset_to_position(decl_src, decl.name_range.0);
        let (el, ec) = decl_li.byte_offset_to_position(decl_src, decl.name_range.1);
        let uri = Url::from_file_path(decl_path).ok()?;
        changes.entry(uri).or_default().push(TextEdit {
            range: Range {
                start: Position {
                    line: sl,
                    character: sc,
                },
                end: Position {
                    line: el,
                    character: ec,
                },
            },
            new_text: new_name.to_string(),
        });
    }

    // Rename all references, caching file reads + LineIndex. (Fix #19)
    let refs = st.find_references(&decl_id);
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
        let (sl, sc) = ref_li.byte_offset_to_position(ref_source, *start);
        let (el, ec) = ref_li.byte_offset_to_position(ref_source, *end);
        if let Ok(uri) = Url::from_file_path(path) {
            changes.entry(uri).or_default().push(TextEdit {
                range: Range {
                    start: Position {
                        line: sl,
                        character: sc,
                    },
                    end: Position {
                        line: el,
                        character: ec,
                    },
                },
                new_text: new_name.to_string(),
            });
        }
    }

    // Deduplicate edits per file.
    for edits in changes.values_mut() {
        edits.sort_by(|a, b| {
            a.range
                .start
                .line
                .cmp(&b.range.start.line)
                .then(a.range.start.character.cmp(&b.range.start.character))
        });
        edits.dedup_by(|a, b| a.range == b.range);
    }

    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}
