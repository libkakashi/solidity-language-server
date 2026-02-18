use std::path::Path;

use tower_lsp::lsp_types::*;

use crate::symbol_table::SymbolTable;
use crate::utils::LineIndex;

/// Return all highlights for the symbol under the cursor, scoped to the current file.
///
/// The declaration site is marked as `Write`, all other references as `Read`.
pub fn document_highlight(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Vec<DocumentHighlight> {
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);

    let decl = match st.resolve_at(file, byte_offset) {
        Some(d) => d,
        None => return vec![],
    };

    let decl_id = decl.id;
    let file_id = match st.lookup_file_id(file) {
        Some(id) => id,
        None => return vec![],
    };

    let mut highlights = Vec::new();

    // Include the declaration itself if it's in the current file.
    if decl_id.file == file_id {
        let range =
            line_index.byte_range_to_lsp_range(source, decl.name_range.0, decl.name_range.1);
        highlights.push(DocumentHighlight {
            range,
            kind: Some(DocumentHighlightKind::WRITE),
        });
    }

    // Collect all references, filtering to the current file only.
    let refs = st.find_references(&decl_id);
    for (ref_path, start, end) in &refs {
        if ref_path.as_path() != file {
            continue;
        }
        let range = line_index.byte_range_to_lsp_range(source, *start, *end);
        highlights.push(DocumentHighlight {
            range,
            kind: Some(DocumentHighlightKind::READ),
        });
    }

    // Deduplicate by range (declaration might also appear in references).
    highlights.sort_by_key(|h| (h.range.start.line, h.range.start.character));
    highlights.dedup_by_key(|h| (h.range.start.line, h.range.start.character));

    highlights
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_resolver::ImportResolver;
    use crate::parser::TsParser;
    use crate::symbol_table::SymbolTable;
    use std::path::PathBuf;

    fn setup(source: &str) -> (SymbolTable, PathBuf, LineIndex) {
        let path = PathBuf::from("/tmp/test_highlight.sol");
        let resolver = ImportResolver::new(std::path::Path::new("/tmp"));
        let mut st = SymbolTable::new(resolver);
        let mut parser = TsParser::new();
        st.index_file(&path, source, &mut parser);
        st.resolve_file_references(&path, &mut parser);
        let li = LineIndex::new(source);
        (st, path, li)
    }

    #[test]
    fn test_highlight_state_variable() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
contract Foo {
    uint256 public value;
    function get() public view returns (uint256) {
        return value;
    }
    function set(uint256 v) public {
        value = v;
    }
}"#;
        let (st, path, li) = setup(source);
        // Position on the declaration of `value` (line 3, col ~23)
        let pos = Position {
            line: 3,
            character: 19,
        };
        let highlights = document_highlight(&st, &path, source, pos, &li);
        // Should find the declaration + usages in get() and set()
        assert!(
            highlights.len() >= 3,
            "expected at least 3 highlights, got {}",
            highlights.len()
        );
        // First one should be WRITE (declaration)
        assert_eq!(highlights[0].kind, Some(DocumentHighlightKind::WRITE));
    }

    #[test]
    fn test_highlight_no_match() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
contract Foo {
    function bar() public {}
}"#;
        let (st, path, li) = setup(source);
        // Position on whitespace
        let pos = Position {
            line: 0,
            character: 0,
        };
        let highlights = document_highlight(&st, &path, source, pos, &li);
        assert!(highlights.is_empty());
    }

    #[test]
    fn test_highlight_function_name() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
contract Foo {
    function greet() public pure returns (string memory) {
        return "hello";
    }
    function caller() public pure returns (string memory) {
        return greet();
    }
}"#;
        let (st, path, li) = setup(source);
        // Position on `greet` in declaration (line 3)
        let pos = Position {
            line: 3,
            character: 13,
        };
        let highlights = document_highlight(&st, &path, source, pos, &li);
        assert!(
            highlights.len() >= 2,
            "expected at least 2 highlights, got {}",
            highlights.len()
        );
    }
}
