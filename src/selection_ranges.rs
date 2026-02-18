use tower_lsp::lsp_types::*;
use tree_sitter::Tree;

use crate::utils::LineIndex;

/// Compute selection ranges for the given positions.
///
/// For each position, returns a nested chain of ranges from the innermost
/// (most specific) to the outermost (entire file), allowing the editor to
/// progressively expand/shrink the selection.
pub fn selection_ranges(
    source: &str,
    positions: &[Position],
    line_index: &LineIndex,
    tree: Option<&Tree>,
) -> Vec<SelectionRange> {
    let tree = match tree {
        Some(t) => t,
        None => {
            return positions
                .iter()
                .map(|_| default_selection(source, line_index))
                .collect();
        }
    };

    positions
        .iter()
        .map(|pos| build_selection_range(source, *pos, line_index, tree))
        .collect()
}

/// Build a selection range chain for a single position by walking up the CST.
fn build_selection_range(
    source: &str,
    position: Position,
    line_index: &LineIndex,
    tree: &Tree,
) -> SelectionRange {
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);
    let root = tree.root_node();

    // Find the deepest node at this position.
    let deepest = match root.descendant_for_byte_range(byte_offset, byte_offset) {
        Some(n) => n,
        None => return default_selection(source, line_index),
    };

    // Walk up from the deepest node, collecting ranges.
    // Skip nodes that have the same range as their child (no expansion).
    let mut ranges: Vec<Range> = Vec::new();
    let mut current = Some(deepest);

    while let Some(node) = current {
        let range = line_index.byte_range_to_lsp_range(source, node.start_byte(), node.end_byte());

        // Only add if the range is different from the last one (avoids duplicate levels).
        if ranges.last() != Some(&range) {
            ranges.push(range);
        }

        current = node.parent();
    }

    // Build the nested SelectionRange from outermost to innermost.
    // We reverse so we build from outermost first, then wrap with inner.
    if ranges.is_empty() {
        return default_selection(source, line_index);
    }

    // Start from the outermost range (last in our vec).
    let mut result = SelectionRange {
        range: *ranges.last().unwrap(),
        parent: None,
    };

    // Wrap progressively with inner ranges.
    for i in (0..ranges.len() - 1).rev() {
        result = SelectionRange {
            range: ranges[i],
            parent: Some(Box::new(result)),
        };
    }

    result
}

/// Default selection covering the whole file.
fn default_selection(source: &str, line_index: &LineIndex) -> SelectionRange {
    let (end_line, end_char) = line_index.byte_offset_to_position(source, source.len());
    SelectionRange {
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: end_line,
                character: end_char,
            },
        },
        parent: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::TsParser;

    fn get_selection(source: &str, line: u32, character: u32) -> SelectionRange {
        let mut parser = TsParser::new();
        let tree = parser.parse(source, None).unwrap();
        let li = LineIndex::new(source);
        let pos = Position { line, character };
        let results = selection_ranges(source, &[pos], &li, Some(&tree));
        results.into_iter().next().unwrap()
    }

    #[test]
    fn test_selection_range_has_parent_chain() {
        let source = "pragma solidity ^0.8.0;\ncontract Foo {\n    function bar() public {\n        uint256 x = 1;\n    }\n}";
        // Position on `x` (line 3, col 16)
        let sel = get_selection(source, 3, 16);
        // Should have a parent chain: x -> declaration -> body -> function -> contract body -> contract -> source_file
        assert!(sel.parent.is_some(), "expected parent chain");
        let mut depth = 0;
        let mut current = &sel;
        loop {
            depth += 1;
            match &current.parent {
                Some(p) => current = p,
                None => break,
            }
        }
        assert!(
            depth >= 3,
            "expected at least 3 levels of nesting, got {}",
            depth
        );
    }

    #[test]
    fn test_selection_range_innermost_covers_position() {
        let source = "pragma solidity ^0.8.0;\ncontract Foo {\n    uint256 value;\n}";
        // Position on `value` (line 2, col 12)
        let sel = get_selection(source, 2, 12);
        // The innermost range should cover the `value` token.
        assert!(sel.range.start.line == 2);
        assert!(sel.range.start.character <= 12);
        assert!(sel.range.end.character >= 12);
    }

    #[test]
    fn test_selection_range_no_duplicate_levels() {
        let source = "pragma solidity ^0.8.0;\ncontract Foo {}";
        // Position on `Foo` (line 1, col 9)
        let sel = get_selection(source, 1, 9);
        // Walk the chain and ensure no two consecutive levels have the same range.
        let mut current = &sel;
        loop {
            if let Some(ref parent) = current.parent {
                assert_ne!(
                    current.range, parent.range,
                    "duplicate range in selection chain"
                );
                current = parent;
            } else {
                break;
            }
        }
    }
}
