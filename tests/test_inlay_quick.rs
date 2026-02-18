use std::path::PathBuf;
use solidity_language_server::inlay_hints::inlay_hints;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::{Position, Range, InlayHintLabel};

fn setup(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

fn full_range() -> Range {
    Range { start: Position::new(0, 0), end: Position::new(u32::MAX, u32::MAX) }
}

fn get_hints(source: &str) -> Vec<tower_lsp::lsp_types::InlayHint> {
    let (st, path) = setup(source);
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let li = LineIndex::new(source);
    inlay_hints(&st, &path, source, full_range(), &li, Some(&tree))
}

fn hint_label(hint: &tower_lsp::lsp_types::InlayHint) -> String {
    match &hint.label {
        InlayHintLabel::String(s) => s.clone(),
        InlayHintLabel::LabelParts(parts) => parts.iter().map(|p| p.value.as_str()).collect::<String>(),
    }
}

#[test]
fn test_constructor() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    constructor(string memory _name, string memory _symbol) {}
}

contract Factory {
    function create() public returns (Token) {
        return new Token("MyToken", "MTK");
    }
}
"#;
    let hints = get_hints(source);
    eprintln!("constructor hints: {}", hints.len());
    for h in &hints {
        eprintln!("  label='{}' pos={}:{}", hint_label(h), h.position.line, h.position.character);
    }
    assert_eq!(hints.len(), 2);
    assert_eq!(hint_label(&hints[0]), "_name:");
    assert_eq!(hint_label(&hints[1]), "_symbol:");
}

#[test]
fn test_modifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    modifier onlyRole(bytes32 role) { _; }
    function admin() public onlyRole(0x00) {}
}
"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 1);
    assert_eq!(hint_label(&hints[0]), "role:");
}
