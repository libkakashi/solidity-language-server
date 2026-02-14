use std::path::PathBuf;

use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::references::find_references;
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
fn find_all_references_to_state_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public totalSupply;

    function mint(uint256 amount) public {
        totalSupply += amount;
    }

    function burn(uint256 amount) public {
        totalSupply -= amount;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `totalSupply` declaration
    let ts_pos = source.find("totalSupply").unwrap();
    let line = source[..ts_pos].matches('\n').count() as u32;
    let col = (ts_pos - source[..ts_pos].rfind('\n').unwrap() - 1) as u32;

    let refs = find_references(&st, &path, source, Position::new(line, col), true);
    // Declaration + 2 usages = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references (1 decl + 2 uses), got {:?}",
        refs
    );
}

#[test]
fn find_references_excludes_declaration() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public totalSupply;

    function mint(uint256 amount) public {
        totalSupply += amount;
    }
}
"#;
    let (st, path) = setup(source);

    let ts_pos = source.find("totalSupply").unwrap();
    let line = source[..ts_pos].matches('\n').count() as u32;
    let col = (ts_pos - source[..ts_pos].rfind('\n').unwrap() - 1) as u32;

    let refs = find_references(&st, &path, source, Position::new(line, col), false);
    // Only usages, not declaration
    assert_eq!(
        refs.len(),
        1,
        "Expected 1 reference (usage only), got {:?}",
        refs
    );
}

#[test]
fn find_references_to_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Calculator {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }

    function compute() public pure returns (uint256) {
        return add(1, 2);
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `add` in `return add(1, 2);`
    let add_call = source.find("add(1, 2)").unwrap();
    let line = source[..add_call].matches('\n').count() as u32;
    let col = (add_call - source[..add_call].rfind('\n').unwrap() - 1) as u32;

    let refs = find_references(&st, &path, source, Position::new(line, col), true);
    // Declaration + 1 call = 2
    assert_eq!(refs.len(), 2, "Expected 2 references, got {:?}", refs);
}

#[test]
fn find_references_no_results_for_unknown() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public {}
}
"#;
    let (st, path) = setup(source);

    // Position in whitespace — should find nothing
    let refs = find_references(&st, &path, source, Position::new(0, 0), true);
    assert!(
        refs.is_empty(),
        "Expected no references for non-identifier position"
    );
}
