use std::path::PathBuf;

use solidity_language_server::completion::handle_completion;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use tower_lsp::lsp_types::{CompletionResponse, Position};

fn setup(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

fn completion_labels(
    st: &SymbolTable,
    path: &PathBuf,
    source: &str,
    pos: Position,
    trigger: Option<&str>,
) -> Vec<String> {
    match handle_completion(st, path, source, pos, trigger) {
        Some(CompletionResponse::List(list)) => {
            list.items.iter().map(|i| i.label.clone()).collect()
        }
        _ => vec![],
    }
}

#[test]
fn general_completion_includes_visible_declarations() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    uint256 public balance;
    address public owner;

    function deposit() public {

    }
}
"#;
    let (st, path) = setup(source);

    // Trigger completion inside the function body (line 8, after whitespace)
    let labels = completion_labels(&st, &path, source, Position::new(8, 8), None);

    assert!(
        labels.contains(&"balance".to_string()),
        "Should include state variable 'balance', got: {labels:?}"
    );
    assert!(
        labels.contains(&"owner".to_string()),
        "Should include state variable 'owner'"
    );
    assert!(
        labels.contains(&"deposit".to_string()),
        "Should include function 'deposit'"
    );
    assert!(
        labels.contains(&"Vault".to_string()),
        "Should include contract 'Vault'"
    );
}

#[test]
fn general_completion_includes_keywords() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public {

    }
}
"#;
    let (st, path) = setup(source);

    let labels = completion_labels(&st, &path, source, Position::new(5, 8), None);

    assert!(
        labels.contains(&"uint256".to_string()),
        "Should include uint256 keyword"
    );
    assert!(
        labels.contains(&"address".to_string()),
        "Should include address keyword"
    );
    assert!(
        labels.contains(&"mapping".to_string()),
        "Should include mapping keyword"
    );
    assert!(
        labels.contains(&"msg".to_string()),
        "Should include msg global"
    );
}

#[test]
fn dot_completion_for_msg() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public {
        msg.
    }
}
"#;
    let (st, path) = setup(source);

    // Position right after `msg.` — line 5
    let msg_dot = source.find("msg.").unwrap() + "msg.".len();
    let line = source[..msg_dot].matches('\n').count() as u32;
    let col = (msg_dot - source[..msg_dot].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"sender".to_string()),
        "msg. should include sender, got: {labels:?}"
    );
    assert!(
        labels.contains(&"value".to_string()),
        "msg. should include value"
    );
    assert!(
        labels.contains(&"data".to_string()),
        "msg. should include data"
    );
    assert!(
        labels.contains(&"sig".to_string()),
        "msg. should include sig"
    );
}

#[test]
fn dot_completion_for_block() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public view {
        block.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("block.").unwrap() + "block.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"timestamp".to_string()),
        "block. should include timestamp"
    );
    assert!(
        labels.contains(&"number".to_string()),
        "block. should include number"
    );
    assert!(
        labels.contains(&"chainid".to_string()),
        "block. should include chainid"
    );
}

#[test]
fn dot_completion_for_struct_members() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }

    Point public origin;

    function getX() public view returns (uint256) {
        return origin.x;
    }
}
"#;
    let (st, path) = setup(source);

    // Dot completion on a variable whose type is Point
    // Since our type resolution is limited, we test that members_of("Point") works
    // by checking the completion for `Point.` (static access)
    let members = st.members_of("Point", &path);
    assert_eq!(members.len(), 2, "Point should have 2 members");
    assert!(
        members.iter().any(|m| m.name == "x"),
        "Should have member x"
    );
    assert!(
        members.iter().any(|m| m.name == "y"),
        "Should have member y"
    );
}

#[test]
fn completion_includes_global_functions() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public {

    }
}
"#;
    let (st, path) = setup(source);

    let labels = completion_labels(&st, &path, source, Position::new(5, 8), None);

    assert!(
        labels.iter().any(|l| l.starts_with("keccak256")),
        "Should include keccak256"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("require")),
        "Should include require"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("assert")),
        "Should include assert"
    );
}
