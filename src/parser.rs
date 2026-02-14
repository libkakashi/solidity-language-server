use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use tree_sitter::{Parser, Tree, TreeCursor};

use crate::utils::byte_offset_to_position;

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
    let mut errors = Vec::new();
    let mut cursor = tree.walk();
    walk_errors(&mut cursor, source, &mut errors);
    errors
}

fn walk_errors(cursor: &mut TreeCursor, source: &str, errors: &mut Vec<Diagnostic>) {
    let node = cursor.node();
    if node.is_error() || node.is_missing() {
        let range = node_to_lsp_range(node.start_byte(), node.end_byte(), source);
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
            walk_errors(cursor, source, errors);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

pub fn node_to_lsp_range(start_byte: usize, end_byte: usize, source: &str) -> Range {
    let (start_line, start_col) = byte_offset_to_position(source, start_byte);
    let (end_line, end_col) = byte_offset_to_position(source, end_byte);
    Range {
        start: Position {
            line: start_line,
            character: start_col,
        },
        end: Position {
            line: end_line,
            character: end_col,
        },
    }
}
