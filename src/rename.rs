use std::collections::HashMap;
use std::path::Path;

use tower_lsp::lsp_types::{Position, Range, TextEdit, Url, WorkspaceEdit};

use crate::symbol_table::SymbolTable;
use crate::utils::{LineIndex, SourceCache};

/// Find the byte span (start, end) of the identifier at `position`.
fn find_identifier_span(
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Option<(usize, usize)> {
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

    if start == end || bytes[start].is_ascii_digit() {
        return None;
    }

    Some((start, end))
}

/// Get the identifier at a given position (for prepare-rename).
pub fn get_identifier_at_position(
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Option<String> {
    let (start, end) = find_identifier_span(source, position, line_index)?;
    Some(source[start..end].to_string())
}

/// Get the range of the identifier at position (for prepare-rename).
pub fn get_identifier_range(
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Option<Range> {
    let (start, end) = find_identifier_span(source, position, line_index)?;
    Some(line_index.byte_range_to_lsp_range(source, start, end))
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
    let refs = st.find_references(&decl_id);
    let mut cache = SourceCache::new(file, source, line_index);

    // Rename the declaration itself.
    {
        let decl_path = st.resolve_path(decl_id.file);
        if let Some((decl_src, decl_li)) = cache.get(decl_path) {
            if let Ok(uri) = Url::from_file_path(decl_path) {
                let range =
                    decl_li.byte_range_to_lsp_range(decl_src, decl.name_range.0, decl.name_range.1);
                changes.entry(uri).or_default().push(TextEdit {
                    range,
                    new_text: new_name.to_string(),
                });
            }
        }
    }

    // Rename all references.
    for (path, start, end) in &refs {
        if let Some((ref_source, ref_li)) = cache.get(path.as_path()) {
            if let Ok(uri) = Url::from_file_path(path) {
                let range = ref_li.byte_range_to_lsp_range(ref_source, *start, *end);
                changes.entry(uri).or_default().push(TextEdit {
                    range,
                    new_text: new_name.to_string(),
                });
            }
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
