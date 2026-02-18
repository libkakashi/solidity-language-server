use tower_lsp::lsp_types::*;
use tree_sitter::{Node, Tree};

use crate::utils::LineIndex;

/// Compute folding ranges for a document.
///
/// Foldable regions include:
/// - Contract / interface / library bodies
/// - Function / constructor / modifier / fallback / receive bodies
/// - Struct / enum bodies
/// - Block statements (if, for, while, etc.)
/// - Multi-line comments (block and NatSpec)
/// - Import groups (consecutive import directives)
pub fn folding_ranges(
    source: &str,
    line_index: &LineIndex,
    tree: Option<&Tree>,
) -> Vec<FoldingRange> {
    let tree = match tree {
        Some(t) => t,
        None => return vec![],
    };

    let mut ranges = Vec::new();
    collect_folding_ranges(tree.root_node(), source, line_index, &mut ranges);
    collect_import_groups(tree.root_node(), source, line_index, &mut ranges);
    ranges
}

fn collect_folding_ranges(
    node: Node,
    source: &str,
    line_index: &LineIndex,
    ranges: &mut Vec<FoldingRange>,
) {
    match node.kind() {
        // Declarations with bodies.
        "contract_declaration"
        | "interface_declaration"
        | "library_declaration"
        | "function_definition"
        | "constructor_definition"
        | "modifier_definition"
        | "fallback_receive_definition"
        | "struct_declaration"
        | "enum_declaration" => {
            if let Some(body) = node.child_by_field_name("body") {
                add_region_range(&body, source, line_index, FoldingRangeKind::Region, ranges);
            }
        }
        // Block statements.
        "block_statement" | "unchecked_block" => {
            add_region_range(&node, source, line_index, FoldingRangeKind::Region, ranges);
        }
        // If/else — fold the body blocks, not the if node itself.
        // (Children are walked recursively, so the block_statement children will be caught.)

        // Multi-line comments.
        "comment" => {
            let text = &source[node.start_byte()..node.end_byte()];
            if text.starts_with("/*") || text.starts_with("/**") {
                let (start_line, _) = line_index.byte_offset_to_position(source, node.start_byte());
                let (end_line, _) = line_index.byte_offset_to_position(source, node.end_byte());
                if end_line > start_line {
                    ranges.push(FoldingRange {
                        start_line,
                        start_character: None,
                        end_line,
                        end_character: None,
                        kind: Some(FoldingRangeKind::Comment),
                        collapsed_text: None,
                    });
                }
            }
        }
        // Event/error definitions can span multiple lines.
        "event_definition" | "error_declaration" => {
            let (start_line, _) = line_index.byte_offset_to_position(source, node.start_byte());
            let (end_line, _) = line_index.byte_offset_to_position(source, node.end_byte());
            if end_line > start_line {
                ranges.push(FoldingRange {
                    start_line,
                    start_character: None,
                    end_line,
                    end_character: None,
                    kind: Some(FoldingRangeKind::Region),
                    collapsed_text: None,
                });
            }
        }
        _ => {}
    }

    // Recurse into children.
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_folding_ranges(cursor.node(), source, line_index, ranges);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Add a folding range for a node that spans multiple lines.
fn add_region_range(
    node: &Node,
    source: &str,
    line_index: &LineIndex,
    kind: FoldingRangeKind,
    ranges: &mut Vec<FoldingRange>,
) {
    let (start_line, _) = line_index.byte_offset_to_position(source, node.start_byte());
    let (end_line, _) = line_index.byte_offset_to_position(source, node.end_byte());
    if end_line > start_line {
        ranges.push(FoldingRange {
            start_line,
            start_character: None,
            end_line,
            end_character: None,
            kind: Some(kind),
            collapsed_text: None,
        });
    }
}

/// Detect groups of consecutive import directives and create folding ranges for them.
fn collect_import_groups(
    root: Node,
    source: &str,
    line_index: &LineIndex,
    ranges: &mut Vec<FoldingRange>,
) {
    let mut import_start: Option<u32> = None;
    let mut import_end: u32 = 0;
    let mut count = 0u32;

    let mut cursor = root.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "import_directive" {
                let (line, _) = line_index.byte_offset_to_position(source, child.start_byte());
                let (end_line, _) = line_index.byte_offset_to_position(source, child.end_byte());
                if import_start.is_none() {
                    import_start = Some(line);
                }
                import_end = end_line;
                count += 1;
            } else if child.kind() != "comment" && import_start.is_some() {
                // End of import group (skip comments between imports).
                if count >= 2 {
                    ranges.push(FoldingRange {
                        start_line: import_start.unwrap(),
                        start_character: None,
                        end_line: import_end,
                        end_character: None,
                        kind: Some(FoldingRangeKind::Imports),
                        collapsed_text: None,
                    });
                }
                import_start = None;
                count = 0;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    // Handle trailing import group.
    if count >= 2 {
        if let Some(start) = import_start {
            ranges.push(FoldingRange {
                start_line: start,
                start_character: None,
                end_line: import_end,
                end_character: None,
                kind: Some(FoldingRangeKind::Imports),
                collapsed_text: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::TsParser;

    fn get_ranges(source: &str) -> Vec<FoldingRange> {
        let mut parser = TsParser::new();
        let tree = parser.parse(source, None).unwrap();
        let li = LineIndex::new(source);
        folding_ranges(source, &li, Some(&tree))
    }

    #[test]
    fn test_contract_body_folds() {
        let source = "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;\ncontract Foo {\n    uint256 x;\n    function bar() public {}\n}";
        let ranges = get_ranges(source);
        // Should have at least one region for the contract body.
        assert!(
            ranges
                .iter()
                .any(|r| r.kind == Some(FoldingRangeKind::Region)),
            "expected a region folding range for the contract body"
        );
    }

    #[test]
    fn test_multiline_comment_folds() {
        let source = "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;\n/*\n * A multi-line\n * comment\n */\ncontract Foo {}";
        let ranges = get_ranges(source);
        assert!(
            ranges
                .iter()
                .any(|r| r.kind == Some(FoldingRangeKind::Comment)),
            "expected a comment folding range"
        );
    }

    #[test]
    fn test_import_group_folds() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
import "./A.sol";
import "./B.sol";
import "./C.sol";
contract Foo {}"#;
        let ranges = get_ranges(source);
        assert!(
            ranges
                .iter()
                .any(|r| r.kind == Some(FoldingRangeKind::Imports)),
            "expected an imports folding range"
        );
    }

    #[test]
    fn test_no_fold_for_single_line() {
        let source = "pragma solidity ^0.8.0;\ncontract Foo {}";
        let ranges = get_ranges(source);
        // Single-line contract body should not produce a fold.
        assert!(
            ranges.is_empty(),
            "expected no folding ranges for single-line constructs"
        );
    }

    #[test]
    fn test_function_body_folds() {
        let source = "pragma solidity ^0.8.0;\ncontract Foo {\n    function bar() public {\n        uint256 x = 1;\n        uint256 y = 2;\n    }\n}";
        let ranges = get_ranges(source);
        // Should have folds for both the contract body and the function body.
        let region_count = ranges
            .iter()
            .filter(|r| r.kind == Some(FoldingRangeKind::Region))
            .count();
        assert!(
            region_count >= 2,
            "expected at least 2 region folds (contract + function body), got {}",
            region_count
        );
    }
}
