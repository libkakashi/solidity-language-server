use std::path::PathBuf;

use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::symbols::{document_symbols, workspace_symbols};
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::SymbolKind;

fn setup(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

/// Setup that also writes source to disk (needed for workspace_symbols which reads from disk).
fn setup_on_disk(source: &str) -> (SymbolTable, PathBuf, tempfile::NamedTempFile) {
    use std::io::Write;
    let mut tmp = tempfile::Builder::new().suffix(".sol").tempfile().unwrap();
    tmp.write_all(source.as_bytes()).unwrap();
    tmp.flush().unwrap();
    let path = tmp.path().to_path_buf();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(path.parent().unwrap().to_path_buf());
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path, tmp)
}

#[test]
fn document_symbols_shows_contract_hierarchy() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract ERC20 {
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;

    event Transfer(address indexed from, address indexed to, uint256 amount);
    error InsufficientBalance(uint256 available, uint256 required);

    function transfer(address to, uint256 amount) public returns (bool) {
        return true;
    }

    function balanceOf(address account) public view returns (uint256) {
        return balanceOf[account];
    }
}
"#;
    let (st, path) = setup(source);
    let syms = document_symbols(&st, &path, source, &LineIndex::new(source));

    // Should have exactly one top-level symbol: ERC20 contract
    assert_eq!(
        syms.len(),
        1,
        "Expected 1 top-level symbol, got {}",
        syms.len()
    );
    assert_eq!(syms[0].name, "ERC20");
    assert_eq!(syms[0].kind, SymbolKind::CLASS);

    // Contract should have children
    let children = syms[0]
        .children
        .as_ref()
        .expect("ERC20 should have children");
    let child_names: Vec<&str> = children.iter().map(|s| s.name.as_str()).collect();
    assert!(
        child_names.contains(&"totalSupply"),
        "Missing totalSupply, got: {child_names:?}"
    );
    assert!(child_names.contains(&"transfer"), "Missing transfer");
    assert!(child_names.contains(&"Transfer"), "Missing Transfer event");
    assert!(
        child_names.contains(&"InsufficientBalance"),
        "Missing InsufficientBalance error"
    );
}

#[test]
fn document_symbols_shows_struct_and_enum() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct Entry {
        address addr;
        uint256 value;
    }

    enum Status { Active, Inactive }
}
"#;
    let (st, path) = setup(source);
    let syms = document_symbols(&st, &path, source, &LineIndex::new(source));

    assert_eq!(syms.len(), 1);
    let children = syms[0].children.as_ref().unwrap();
    let entry = children
        .iter()
        .find(|s| s.name == "Entry")
        .expect("Missing Entry struct");
    assert_eq!(entry.kind, SymbolKind::STRUCT);

    let status = children
        .iter()
        .find(|s| s.name == "Status")
        .expect("Missing Status enum");
    assert_eq!(status.kind, SymbolKind::ENUM);
}

#[test]
fn workspace_symbols_filters_by_query() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract TokenA {
    function transferFrom() public {}
}

contract TokenB {
    function transferTo() public {}
}
"#;
    let (st, _path, _tmp) = setup_on_disk(source);

    // Query for "transfer" — should match both functions
    let results = workspace_symbols(&st, "transfer");
    let names: Vec<&str> = results.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"transferFrom"),
        "Missing transferFrom, got: {names:?}"
    );
    assert!(names.contains(&"transferTo"), "Missing transferTo");

    // Query for "TokenA" — should match only TokenA
    let results = workspace_symbols(&st, "TokenA");
    let names: Vec<&str> = results.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"TokenA"), "Missing TokenA");
    assert!(!names.contains(&"TokenB"), "Should not include TokenB");
}

#[test]
fn workspace_symbols_excludes_locals_and_params() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar(uint256 localParam) public pure {
        uint256 localVar = localParam;
    }
}
"#;
    let (st, _path, _tmp) = setup_on_disk(source);

    let results = workspace_symbols(&st, "");
    let names: Vec<&str> = results.iter().map(|s| s.name.as_str()).collect();
    assert!(!names.contains(&"localParam"), "Should exclude parameters");
    assert!(
        !names.contains(&"localVar"),
        "Should exclude local variables"
    );
    assert!(names.contains(&"bar"), "Should include function bar");
    assert!(names.contains(&"Foo"), "Should include contract Foo");
}

#[test]
fn document_symbols_sorted_by_position() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ordered {
    uint256 public alpha;
    uint256 public beta;
    function gamma() public {}
    function delta() public {}
}
"#;
    let (st, path) = setup(source);
    let syms = document_symbols(&st, &path, source, &LineIndex::new(source));

    let children = syms[0].children.as_ref().unwrap();
    // Verify they are sorted by line number
    for i in 1..children.len() {
        assert!(
            children[i].range.start.line >= children[i - 1].range.start.line,
            "Children should be sorted by position"
        );
    }
}
