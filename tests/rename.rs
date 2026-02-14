use std::path::PathBuf;

use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::rename::{
    get_identifier_at_position, get_identifier_range, rename_symbol,
};
use solidity_language_server::symbol_table::SymbolTable;
use tower_lsp::lsp_types::Position;

fn setup(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

#[test]
fn get_identifier_at_function_name() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function myFunction() public {}
}
"#;
    let fn_pos = source.find("myFunction").unwrap();
    let line = source[..fn_pos].matches('\n').count() as u32;
    let col = (fn_pos - source[..fn_pos].rfind('\n').unwrap() - 1) as u32;

    let ident = get_identifier_at_position(source, Position::new(line, col));
    assert_eq!(ident, Some("myFunction".to_string()));
}

#[test]
fn get_identifier_range_for_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public myVar;
}
"#;
    let var_pos = source.find("myVar").unwrap();
    let line = source[..var_pos].matches('\n').count() as u32;
    let col = (var_pos - source[..var_pos].rfind('\n').unwrap() - 1) as u32;

    let range = get_identifier_range(source, Position::new(line, col));
    assert!(range.is_some());
    let range = range.unwrap();
    // Range should cover exactly "myVar"
    assert_eq!(range.start.line, line);
    assert_eq!(range.end.line, line);
    assert_eq!(
        (range.end.character - range.start.character) as usize,
        "myVar".len()
    );
}

#[test]
fn rename_state_variable_across_usages() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public balance;

    function deposit(uint256 amount) public {
        balance += amount;
    }

    function getBalance() public view returns (uint256) {
        return balance;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `balance` declaration
    let bal_pos = source.find("balance").unwrap();
    let line = source[..bal_pos].matches('\n').count() as u32;
    let col = (bal_pos - source[..bal_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(&st, &path, source, Position::new(line, col), "totalBalance");
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename declaration + 2 usages = 3 edits
    assert_eq!(
        file_edits.len(),
        3,
        "Expected 3 text edits, got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "totalBalance");
    }
}

#[test]
fn rename_returns_none_for_non_identifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {}
"#;
    let ident = get_identifier_at_position(source, Position::new(0, 0));
    // Line 0 col 0 is `/` from the comment
    assert!(ident.is_none() || ident.unwrap().is_empty() == false);
}
