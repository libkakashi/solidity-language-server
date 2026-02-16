use std::path::Path;

use tower_lsp::lsp_types::{DocumentLink, Position, Range, Url};

use crate::symbol_table::SymbolTable;
use crate::utils::LineIndex;

/// Extract document links for import directives (path strings -> resolved files).
pub fn document_links(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    line_index: &LineIndex,
) -> Vec<DocumentLink> {
    let fi = match st.get_file_index(file) {
        Some(fi) => fi,
        None => return vec![],
    };

    let mut links = Vec::new();

    for imp in &fi.imports {
        let resolved = match &imp.resolved_path {
            Some(p) => p,
            None => continue,
        };

        let uri = match Url::from_file_path(resolved) {
            Ok(u) => u,
            Err(_) => continue,
        };

        let start = imp.path_range.0;
        let end = imp.path_range.1;
        let src_bytes = source.as_bytes();

        let inner_start =
            if start < src_bytes.len() && (src_bytes[start] == b'"' || src_bytes[start] == b'\'') {
                start + 1
            } else {
                start
            };
        let inner_end = if end > 0
            && end <= src_bytes.len()
            && (src_bytes[end - 1] == b'"' || src_bytes[end - 1] == b'\'')
        {
            end - 1
        } else {
            end
        };

        let (sl, sc) = line_index.byte_offset_to_position(source, inner_start);
        let (el, ec) = line_index.byte_offset_to_position(source, inner_end);

        links.push(DocumentLink {
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
            target: Some(uri),
            tooltip: Some(imp.source_path.clone()),
            data: None,
        });
    }

    links.sort_by(|a, b| {
        a.range
            .start
            .line
            .cmp(&b.range.start.line)
            .then(a.range.start.character.cmp(&b.range.start.character))
    });

    links
}
