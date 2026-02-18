use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range};
use tree_sitter::{Parser, Tree, TreeCursor};

use crate::utils::LineIndex;

pub struct TsParser {
    parser: Parser,
}

impl TsParser {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_solidity::LANGUAGE.into())
            .expect("failed to load solidity grammar");
        Self { parser }
    }

    pub fn parse(&mut self, source: &str, old_tree: Option<&Tree>) -> Option<Tree> {
        self.parser.parse(source, old_tree)
    }
}

pub fn collect_parse_errors(tree: &Tree, source: &str) -> Vec<Diagnostic> {
    let line_index = LineIndex::new(source);
    let mut errors = Vec::new();
    let mut cursor = tree.walk();
    walk_errors(&mut cursor, source, &line_index, &mut errors);
    errors
}

fn walk_errors(
    cursor: &mut TreeCursor,
    source: &str,
    line_index: &LineIndex,
    errors: &mut Vec<Diagnostic>,
) {
    let node = cursor.node();
    if node.is_error() || node.is_missing() {
        let range = node_to_lsp_range(node.start_byte(), node.end_byte(), source, line_index);
        errors.push(Diagnostic {
            range,
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("ts-parse".into()),
            message: if node.is_missing() {
                format!("Missing {}", node.kind())
            } else {
                "Syntax error".into()
            },
            ..Default::default()
        });
    }
    if cursor.goto_first_child() {
        loop {
            walk_errors(cursor, source, line_index, errors);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

pub fn node_to_lsp_range(
    start_byte: usize,
    end_byte: usize,
    source: &str,
    line_index: &LineIndex,
) -> Range {
    line_index.byte_range_to_lsp_range(source, start_byte, end_byte)
}
