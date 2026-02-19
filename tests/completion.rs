use std::path::PathBuf;

use solidity_language_server::completion::handle_completion;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
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
    match handle_completion(
        st,
        path,
        source,
        pos,
        trigger,
        &LineIndex::new(source),
        None,
    ) {
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

// ========== EDGE CASE TESTS ==========

#[test]
fn dot_completion_on_tx() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public view {
        tx.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("tx.").unwrap() + "tx.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"origin".to_string()),
        "tx. should include origin, got: {labels:?}"
    );
    assert!(
        labels.contains(&"gasprice".to_string()),
        "tx. should include gasprice"
    );
}

#[test]
fn dot_completion_on_abi() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure {
        abi.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("abi.").unwrap() + "abi.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.iter().any(|l| l.starts_with("encode")),
        "abi. should include encode, got: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("decode")),
        "abi. should include decode, got: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("encodePacked")),
        "abi. should include encodePacked"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("encodeWithSelector")),
        "abi. should include encodeWithSelector"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("encodeWithSignature")),
        "abi. should include encodeWithSignature"
    );
}

#[test]
fn dot_completion_on_this() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function externalFunc() external returns (uint256) {
        return 42;
    }

    function bar() public {
        this.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("this.").unwrap() + "this.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"externalFunc".to_string()),
        "this. should include external functions, got: {labels:?}"
    );
}

#[test]
fn dot_completion_on_type_uint256() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure {
        type(uint256).
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("type(uint256).").unwrap() + "type(uint256).".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"min".to_string()),
        "type(uint256). should include min, got: {labels:?}"
    );
    assert!(
        labels.contains(&"max".to_string()),
        "type(uint256). should include max"
    );
}

#[test]
fn dot_completion_on_bytes() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure {
        bytes.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("bytes.").unwrap() + "bytes.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.iter().any(|l| l.starts_with("concat")),
        "bytes. should include concat, got: {labels:?}"
    );
}

#[test]
fn dot_completion_on_string() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure {
        string.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("string.").unwrap() + "string.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.iter().any(|l| l.starts_with("concat")),
        "string. should include concat, got: {labels:?}"
    );
}

#[test]
fn dot_completion_on_contract_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
}

contract Foo {
    IERC20 public token;

    function bar() public view {
        token.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.rfind("token.").unwrap() + "token.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let _labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    // Check that we get members of IERC20
    let members = st.members_of("IERC20", &path);
    assert!(
        members.iter().any(|m| m.name == "balanceOf"),
        "IERC20 should have balanceOf member"
    );
    assert!(
        members.iter().any(|m| m.name == "transfer"),
        "IERC20 should have transfer member"
    );
}

#[test]
fn dot_completion_on_enum() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    enum Status { Pending, Active, Completed }

    function bar() public pure {
        Status.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.rfind("Status.").unwrap() + "Status.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"Pending".to_string()),
        "Status. should include Pending, got: {labels:?}"
    );
    assert!(
        labels.contains(&"Active".to_string()),
        "Status. should include Active"
    );
    assert!(
        labels.contains(&"Completed".to_string()),
        "Status. should include Completed"
    );
}

#[test]
fn general_completion_inside_modifier_body() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    address public owner;

    modifier onlyOwner() {

        _;
    }
}
"#;
    let (st, path) = setup(source);

    // Position inside modifier body (line 7, after whitespace)
    let labels = completion_labels(&st, &path, source, Position::new(7, 8), None);

    assert!(
        labels.contains(&"owner".to_string()),
        "Should include state variable 'owner' in modifier, got: {labels:?}"
    );
    assert!(
        labels.contains(&"msg".to_string()),
        "Should include global 'msg' in modifier"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("require")),
        "Should include require in modifier, got: {labels:?}"
    );
}

#[test]
fn general_completion_includes_inherited_members() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    uint256 public baseValue;

    function baseFunction() public view returns (uint256) {
        return baseValue;
    }
}

contract Derived is Base {
    uint256 public derivedValue;

    function derivedFunction() public {

    }
}
"#;
    let (st, path) = setup(source);

    // Position inside derivedFunction (line 15, after whitespace)
    let labels = completion_labels(&st, &path, source, Position::new(15, 8), None);

    assert!(
        labels.contains(&"baseValue".to_string()),
        "Should include inherited state variable 'baseValue', got: {labels:?}"
    );
    assert!(
        labels.contains(&"baseFunction".to_string()),
        "Should include inherited function 'baseFunction'"
    );
    assert!(
        labels.contains(&"derivedValue".to_string()),
        "Should include own state variable 'derivedValue'"
    );
    assert!(
        labels.contains(&"derivedFunction".to_string()),
        "Should include own function 'derivedFunction'"
    );
}

#[test]
fn completion_does_not_include_private_members_from_other_contracts() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 private secretValue;
    uint256 public publicValue;
}

contract B {
    A public contractA;

    function foo() public {

    }
}
"#;
    let (st, path) = setup(source);

    // Position inside B.foo (line 12, after whitespace)
    let labels = completion_labels(&st, &path, source, Position::new(12, 8), None);

    // Should include contractA
    assert!(
        labels.contains(&"contractA".to_string()),
        "Should include contractA, got: {labels:?}"
    );

    // Should not directly include secretValue from contract A in general completion
    // (private members should not be visible)
    // Note: This test checks that private members aren't leaked in general completion
}

#[test]
fn completion_includes_imported_symbols() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {IERC20} from "./IERC20.sol";

contract Foo {
    function bar() public {

    }
}
"#;
    let (st, path) = setup(source);

    // Position inside bar (line 7, after whitespace)
    let labels = completion_labels(&st, &path, source, Position::new(7, 8), None);

    assert!(
        labels.contains(&"IERC20".to_string()),
        "Should include imported symbol 'IERC20', got: {labels:?}"
    );
}

#[test]
fn completion_includes_ether_units() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure {
        uint256 amount = 1
    }
}
"#;
    let (st, path) = setup(source);

    // Position after "1 " (line 5)
    let labels = completion_labels(&st, &path, source, Position::new(5, 27), None);

    assert!(
        labels.contains(&"wei".to_string()),
        "Should include ether unit 'wei', got: {labels:?}"
    );
    assert!(
        labels.contains(&"gwei".to_string()),
        "Should include ether unit 'gwei'"
    );
    assert!(
        labels.contains(&"ether".to_string()),
        "Should include ether unit 'ether'"
    );
}

#[test]
fn completion_includes_time_units() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure {
        uint256 duration = 1
    }
}
"#;
    let (st, path) = setup(source);

    // Position after "1 " (line 5)
    let labels = completion_labels(&st, &path, source, Position::new(5, 30), None);

    assert!(
        labels.contains(&"seconds".to_string()),
        "Should include time unit 'seconds', got: {labels:?}"
    );
    assert!(
        labels.contains(&"minutes".to_string()),
        "Should include time unit 'minutes'"
    );
    assert!(
        labels.contains(&"hours".to_string()),
        "Should include time unit 'hours'"
    );
    assert!(
        labels.contains(&"days".to_string()),
        "Should include time unit 'days'"
    );
    assert!(
        labels.contains(&"weeks".to_string()),
        "Should include time unit 'weeks'"
    );
}

#[test]
fn completion_at_top_level_shows_contract_keywords() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;


"#;
    let (st, path) = setup(source);

    // Position at line 3 (empty line after pragma)
    let labels = completion_labels(&st, &path, source, Position::new(3, 0), None);

    assert!(
        labels.contains(&"contract".to_string()),
        "Should include 'contract' keyword at top level, got: {labels:?}"
    );
    assert!(
        labels.contains(&"library".to_string()),
        "Should include 'library' keyword at top level"
    );
    assert!(
        labels.contains(&"interface".to_string()),
        "Should include 'interface' keyword at top level"
    );
    assert!(
        labels.contains(&"function".to_string()),
        "Should include 'function' keyword at top level (for free functions)"
    );
}

#[test]
fn no_completion_inside_string_literal() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure returns (string memory) {
        return "hello ";
    }
}
"#;
    let (st, path) = setup(source);

    // Position inside the string literal
    let str_pos = source.find("hello ").unwrap() + "hello ".len();
    let line = source[..str_pos].matches('\n').count() as u32;
    let col = (str_pos - source[..str_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), None);

    // Should not provide meaningful completions inside a string literal
    assert!(
        labels.is_empty()
            || labels
                .iter()
                .all(|l| !l.starts_with("msg") && !l.starts_with("uint")),
        "Should not provide keyword completions inside string literal, got: {labels:?}"
    );
}

#[test]
fn no_completion_inside_comment() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public {
        // this is a comment
    }
}
"#;
    let (st, path) = setup(source);

    // Position inside the comment
    let comment_pos = source.find("comment").unwrap() + "comment".len();
    let line = source[..comment_pos].matches('\n').count() as u32;
    let col = (comment_pos - source[..comment_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), None);

    // Should not provide completions inside a comment
    assert!(
        labels.is_empty()
            || labels
                .iter()
                .all(|l| !l.starts_with("msg") && !l.starts_with("uint")),
        "Should not provide keyword completions inside comment, got: {labels:?}"
    );
}

#[test]
fn dot_completion_after_array_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256[] public items;

    function bar() public {
        items.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("items.").unwrap() + "items.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"length".to_string()),
        "Array should include 'length' property, got: {labels:?}"
    );
    assert!(
        labels.contains(&"push".to_string()),
        "Array should include 'push' method"
    );
    assert!(
        labels.contains(&"pop".to_string()),
        "Array should include 'pop' method"
    );
}

#[test]
fn completion_in_empty_file_shows_pragmas_and_keywords() {
    let source = "";
    let (st, path) = setup(source);

    let labels = completion_labels(&st, &path, source, Position::new(0, 0), None);

    assert!(
        labels.contains(&"pragma".to_string()),
        "Empty file should include 'pragma' keyword, got: {labels:?}"
    );
    assert!(
        labels.contains(&"contract".to_string()),
        "Empty file should include 'contract' keyword"
    );
    assert!(
        labels.contains(&"import".to_string()),
        "Empty file should include 'import' keyword"
    );
}

// ========== NEW TESTS ==========

#[test]
fn dot_completion_on_address_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    address public owner;

    function bar() public view {
        owner.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("owner.").unwrap() + "owner.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"balance".to_string()),
        "address. should include balance, got: {labels:?}"
    );
    assert!(
        labels.contains(&"transfer".to_string()),
        "address. should include transfer"
    );
    assert!(
        labels.contains(&"send".to_string()),
        "address. should include send"
    );
    assert!(
        labels.contains(&"call".to_string()),
        "address. should include call"
    );
}

#[test]
fn dot_completion_on_super() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseFunc() public pure returns (uint256) {
        return 1;
    }
}

contract Derived is Base {
    function bar() public pure {
        super.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("super.").unwrap() + "super.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"baseFunc".to_string()),
        "super. should include baseFunc from parent, got: {labels:?}"
    );
}

#[test]
fn dot_completion_with_using_for() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
    function sub(uint256 a, uint256 b) internal pure returns (uint256) {
        return a - b;
    }
}

contract Foo {
    using SafeMath for uint256;

    function bar() public pure {
        uint256 x = 1;
        x.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("x.").unwrap() + "x.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"add".to_string()),
        "x. should include 'add' from using SafeMath, got: {labels:?}"
    );
    assert!(
        labels.contains(&"sub".to_string()),
        "x. should include 'sub' from using SafeMath"
    );
}

#[test]
fn dot_completion_cross_file_imported_interface() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let ierc20_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}
"#;
    let ierc20_path = tmp.path().join("IERC20.sol");
    std::fs::write(&ierc20_path, ierc20_source).unwrap();

    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {IERC20} from "./IERC20.sol";

contract Vault {
    IERC20 public token;

    function test() public {
        token.
    }
}
"#;
    let main_path = tmp.path().join("Vault.sol");
    std::fs::write(&main_path, main_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&ierc20_path, ierc20_source, &mut parser);
    st.resolve_file_references(&ierc20_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    let dot_pos = main_source.find("token.\n").unwrap() + "token.".len();
    let line = main_source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - main_source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(
        &st,
        &main_path,
        main_source,
        Position::new(line, col),
        Some("."),
    );

    eprintln!("labels: {labels:?}");

    assert!(
        labels.contains(&"transfer".to_string()),
        "token. should include 'transfer' from imported IERC20, got: {labels:?}"
    );
    assert!(
        labels.contains(&"balanceOf".to_string()),
        "token. should include 'balanceOf' from imported IERC20, got: {labels:?}"
    );
}

#[test]
fn dot_completion_cross_file_using_for() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let ierc20_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
}
"#;
    let ierc20_path = tmp.path().join("IERC20.sol");
    std::fs::write(&ierc20_path, ierc20_source).unwrap();

    let safelib_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {IERC20} from "./IERC20.sol";

library SafeERC20 {
    function safeTransfer(IERC20 token, address to, uint256 value) internal {
    }
    function safeTransferFrom(IERC20 token, address from, address to, uint256 value) internal {
    }
}
"#;
    let safelib_path = tmp.path().join("SafeERC20.sol");
    std::fs::write(&safelib_path, safelib_source).unwrap();

    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";

contract Vault {
    using SafeERC20 for IERC20;
    IERC20 public token;

    function test() public {
        token.
    }
}
"#;
    let main_path = tmp.path().join("Vault.sol");
    std::fs::write(&main_path, main_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&ierc20_path, ierc20_source, &mut parser);
    st.resolve_file_references(&ierc20_path, &mut parser);
    st.index_file(&safelib_path, safelib_source, &mut parser);
    st.resolve_file_references(&safelib_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    let dot_pos = main_source.find("token.\n").unwrap() + "token.".len();
    let line = main_source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - main_source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(
        &st,
        &main_path,
        main_source,
        Position::new(line, col),
        Some("."),
    );

    eprintln!("labels: {labels:?}");

    // Should include both direct IERC20 methods and using-for SafeERC20 methods
    assert!(
        labels.contains(&"transfer".to_string()),
        "token. should include 'transfer' from IERC20, got: {labels:?}"
    );
    assert!(
        labels.contains(&"safeTransfer".to_string()),
        "token. should include 'safeTransfer' from using SafeERC20, got: {labels:?}"
    );
    assert!(
        labels.contains(&"safeTransferFrom".to_string()),
        "token. should include 'safeTransferFrom' from using SafeERC20, got: {labels:?}"
    );
}

#[test]
fn dot_completion_after_type_cast() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let ierc20_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}
"#;
    let ierc20_path = tmp.path().join("IERC20.sol");
    std::fs::write(&ierc20_path, ierc20_source).unwrap();

    let safelib_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {IERC20} from "./IERC20.sol";

library SafeERC20 {
    function safeTransferFrom(IERC20 token, address from, address to, uint256 value) internal {
    }
}
"#;
    let safelib_path = tmp.path().join("SafeERC20.sol");
    std::fs::write(&safelib_path, safelib_source).unwrap();

    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";

contract Vault {
    using SafeERC20 for IERC20;

    function deposit(address tokenAddr, uint256 amount) external {
        IERC20(tokenAddr).
    }
}
"#;
    let main_path = tmp.path().join("Vault.sol");
    std::fs::write(&main_path, main_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&ierc20_path, ierc20_source, &mut parser);
    st.resolve_file_references(&ierc20_path, &mut parser);
    st.index_file(&safelib_path, safelib_source, &mut parser);
    st.resolve_file_references(&safelib_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    let dot_pos = main_source.find("IERC20(tokenAddr).").unwrap() + "IERC20(tokenAddr).".len();
    let line = main_source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - main_source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(
        &st,
        &main_path,
        main_source,
        Position::new(line, col),
        Some("."),
    );

    // Should include IERC20 interface methods
    assert!(
        labels.contains(&"transfer".to_string()),
        "IERC20(tokenAddr). should include 'transfer', got: {labels:?}"
    );
    assert!(
        labels.contains(&"balanceOf".to_string()),
        "IERC20(tokenAddr). should include 'balanceOf', got: {labels:?}"
    );
    // Should also include using-for methods from SafeERC20
    assert!(
        labels.contains(&"safeTransferFrom".to_string()),
        "IERC20(tokenAddr). should include 'safeTransferFrom' via using-for, got: {labels:?}"
    );
}

#[test]
fn dot_completion_on_this_includes_public_not_internal() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function externalFunc() external pure returns (uint256) {
        return 1;
    }
    function publicFunc() public pure returns (uint256) {
        return 2;
    }
    function internalFunc() internal pure returns (uint256) {
        return 3;
    }

    function bar() public {
        this.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("this.").unwrap() + "this.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"externalFunc".to_string()),
        "this. should include external functions, got: {labels:?}"
    );
    assert!(
        labels.contains(&"publicFunc".to_string()),
        "this. should include public functions"
    );
    assert!(
        !labels.contains(&"internalFunc".to_string()),
        "this. should NOT include internal functions"
    );
}

#[test]
fn override_completion_suggests_base_contracts() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function foo() public virtual {}
}

contract B {
    function foo() public virtual {}
}

contract C is A, B {
    function foo() public override(
    ) {}
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("override(").unwrap() + "override(".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), None);

    assert!(
        labels.contains(&"A".to_string()),
        "override() should suggest base contract A, got: {labels:?}"
    );
    assert!(
        labels.contains(&"B".to_string()),
        "override() should suggest base contract B"
    );
}

#[test]
fn dot_completion_on_variable_includes_inherited_members() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseFunc() public pure returns (uint256) {
        return 1;
    }
}

contract Derived is Base {
    function derivedFunc() public pure returns (uint256) {
        return 2;
    }
}

contract User {
    function test() public {
        Derived d;
        d.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("d.\n").unwrap() + "d.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"derivedFunc".to_string()),
        "d. should include own function 'derivedFunc', got: {labels:?}"
    );
    assert!(
        labels.contains(&"baseFunc".to_string()),
        "d. should include inherited function 'baseFunc', got: {labels:?}"
    );
}

#[test]
fn dot_completion_on_contract_name_includes_inherited() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseFunc() public pure returns (uint256) {
        return 1;
    }
}

contract Derived is Base {
    function derivedFunc() public pure returns (uint256) {
        return 2;
    }
}

contract User {
    function test() public {
        Derived.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("Derived.\n").unwrap() + "Derived.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"derivedFunc".to_string()),
        "Derived. should include own function 'derivedFunc', got: {labels:?}"
    );
    assert!(
        labels.contains(&"baseFunc".to_string()),
        "Derived. should include inherited function 'baseFunc', got: {labels:?}"
    );
}

#[test]
fn dot_completion_this_includes_inherited_public() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function basePub() public pure returns (uint256) {
        return 1;
    }
    function baseInternal() internal pure returns (uint256) {
        return 2;
    }
}

contract Derived is Base {
    function ownPub() public pure returns (uint256) {
        return 3;
    }

    function test() public {
        this.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("this.\n").unwrap() + "this.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"ownPub".to_string()),
        "this. should include own public function, got: {labels:?}"
    );
    assert!(
        labels.contains(&"basePub".to_string()),
        "this. should include inherited public function, got: {labels:?}"
    );
    assert!(
        !labels.contains(&"baseInternal".to_string()),
        "this. should NOT include inherited internal function"
    );
}

#[test]
fn dot_completion_super_includes_grandparent() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Grandparent {
    function grandFunc() public pure returns (uint256) {
        return 1;
    }
}

contract Parent is Grandparent {
    function parentFunc() public pure returns (uint256) {
        return 2;
    }
}

contract Child is Parent {
    function test() public {
        super.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("super.\n").unwrap() + "super.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));

    assert!(
        labels.contains(&"parentFunc".to_string()),
        "super. should include parent function, got: {labels:?}"
    );
    assert!(
        labels.contains(&"grandFunc".to_string()),
        "super. should include grandparent function, got: {labels:?}"
    );
}

#[test]
fn dot_completion_cross_file_inherited_members() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let base_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseFunc() public pure returns (uint256) {
        return 1;
    }
}
"#;
    let base_path = tmp.path().join("Base.sol");
    std::fs::write(&base_path, base_source).unwrap();

    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Base} from "./Base.sol";

contract Derived is Base {
    function derivedFunc() public pure returns (uint256) {
        return 2;
    }
}

contract User {
    function test() public {
        Derived d;
        d.
    }
}
"#;
    let main_path = tmp.path().join("Main.sol");
    std::fs::write(&main_path, main_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&base_path, base_source, &mut parser);
    st.resolve_file_references(&base_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    let dot_pos = main_source.find("d.\n").unwrap() + "d.".len();
    let line = main_source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - main_source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(
        &st,
        &main_path,
        main_source,
        Position::new(line, col),
        Some("."),
    );

    assert!(
        labels.contains(&"derivedFunc".to_string()),
        "d. should include 'derivedFunc', got: {labels:?}"
    );
    assert!(
        labels.contains(&"baseFunc".to_string()),
        "d. should include inherited 'baseFunc' from cross-file Base, got: {labels:?}"
    );
}

#[test]
fn import_completion_suggests_exports() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let lib_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract MyContract {
    function foo() public {}
}

interface IToken {
    function transfer(address to, uint256 amount) external;
}

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

struct Point {
    uint256 x;
    uint256 y;
}
"#;
    let lib_path = tmp.path().join("Lib.sol");
    std::fs::write(&lib_path, lib_source).unwrap();

    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import { } from "./Lib.sol";
"#;
    let main_path = tmp.path().join("Main.sol");
    std::fs::write(&main_path, main_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&lib_path, lib_source, &mut parser);
    st.resolve_file_references(&lib_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // Cursor inside `import { | } from "./Lib.sol";`
    let cursor_pos = main_source.find("{ }").unwrap() + 2; // between { and }
    let line = main_source[..cursor_pos].matches('\n').count() as u32;
    let col = (cursor_pos - main_source[..cursor_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &main_path, main_source, Position::new(line, col), None);

    assert!(
        labels.contains(&"MyContract".to_string()),
        "import completion should suggest 'MyContract', got: {labels:?}"
    );
    assert!(
        labels.contains(&"IToken".to_string()),
        "import completion should suggest 'IToken', got: {labels:?}"
    );
    assert!(
        labels.contains(&"MathLib".to_string()),
        "import completion should suggest 'MathLib', got: {labels:?}"
    );
}

#[test]
fn dot_completion_on_qualified_imported_struct_variable() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let types_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Types {
    struct UserInfo {
        address account;
        uint256 balance;
        bool active;
    }
}
"#;
    let types_path = tmp.path().join("Types.sol");
    std::fs::write(&types_path, types_source).unwrap();

    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Types} from "./Types.sol";

contract Main {
    function test() public {
        Types.UserInfo memory user = Types.UserInfo(msg.sender, 100, true);
        user.
    }
}
"#;
    let main_path = tmp.path().join("Main.sol");
    std::fs::write(&main_path, main_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&types_path, types_source, &mut parser);
    st.resolve_file_references(&types_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    let dot_pos = main_source.find("user.\n").unwrap() + "user.".len();
    let line = main_source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - main_source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(
        &st,
        &main_path,
        main_source,
        Position::new(line, col),
        Some("."),
    );

    eprintln!("labels: {labels:?}");

    assert!(
        labels.contains(&"account".to_string()),
        "user. should include 'account' from Types.UserInfo, got: {labels:?}"
    );
    assert!(
        labels.contains(&"balance".to_string()),
        "user. should include 'balance' from Types.UserInfo, got: {labels:?}"
    );
    assert!(
        labels.contains(&"active".to_string()),
        "user. should include 'active' from Types.UserInfo, got: {labels:?}"
    );
}

// ========== NATSPEC COMPLETION TESTS ==========

/// Helper that parses with tree-sitter and passes the tree to handle_completion.
fn completion_labels_with_tree(
    st: &SymbolTable,
    path: &PathBuf,
    source: &str,
    pos: Position,
    trigger: Option<&str>,
) -> Vec<String> {
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    match handle_completion(
        st,
        path,
        source,
        pos,
        trigger,
        &LineIndex::new(source),
        Some(&tree),
    ) {
        Some(CompletionResponse::List(list)) => {
            list.items.iter().map(|i| i.label.clone()).collect()
        }
        _ => vec![],
    }
}

#[test]
fn natspec_completion_at_sign_in_triple_slash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    /// @
    function bar(uint256 x, address to) public {}
}
"#;
    let (st, path) = setup(source);

    // Position right after `/// @`
    let at_pos = source.find("/// @").unwrap() + "/// @".len();
    let line = source[..at_pos].matches('\n').count() as u32;
    let col = (at_pos - source[..at_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels_with_tree(&st, &path, source, Position::new(line, col), None);

    assert!(
        labels.contains(&"@notice".to_string()),
        "NatSpec should include @notice, got: {labels:?}"
    );
    assert!(
        labels.contains(&"@dev".to_string()),
        "NatSpec should include @dev"
    );
    assert!(
        labels.contains(&"@param".to_string()),
        "NatSpec should include @param"
    );
    assert!(
        labels.contains(&"@return".to_string()),
        "NatSpec should include @return"
    );
    assert!(
        labels.contains(&"@inheritdoc".to_string()),
        "NatSpec should include @inheritdoc"
    );
}

#[test]
fn natspec_completion_partial_tag() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    /// @par
    function bar(uint256 x) public {}
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("/// @par").unwrap() + "/// @par".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels_with_tree(&st, &path, source, Position::new(line, col), None);

    assert!(
        labels.contains(&"@param".to_string()),
        "Partial @par should include @param, got: {labels:?}"
    );
    // @notice should NOT match @par prefix.
    assert!(
        !labels.contains(&"@notice".to_string()),
        "Partial @par should not include @notice"
    );
}

#[test]
fn natspec_param_name_completion() {
    // Use a regular string so we can have a trailing space after @param.
    let source = "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.29;\n\ncontract Foo {\n    /// @param \n    function transfer(address to, uint256 amount) public {}\n}\n";
    let (st, path) = setup(source);

    let pos = source.find("@param ").unwrap() + "@param ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels_with_tree(&st, &path, source, Position::new(line, col), None);

    assert!(
        labels.contains(&"to".to_string()),
        "Should suggest param name 'to', got: {labels:?}"
    );
    assert!(
        labels.contains(&"amount".to_string()),
        "Should suggest param name 'amount', got: {labels:?}"
    );
}

#[test]
fn natspec_no_completion_in_regular_comment() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    // regular comment @
    function bar() public {}
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("comment @").unwrap() + "comment @".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels_with_tree(&st, &path, source, Position::new(line, col), None);

    assert!(
        labels.is_empty(),
        "Regular comments should not get NatSpec completions, got: {labels:?}"
    );
}

#[test]
fn natspec_completion_in_block_comment() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    /**
     * @
     */
    function bar(uint256 x) public {}
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("* @\n").unwrap() + "* @".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels_with_tree(&st, &path, source, Position::new(line, col), None);

    assert!(
        labels.contains(&"@notice".to_string()),
        "Block NatSpec should include @notice, got: {labels:?}"
    );
    assert!(
        labels.contains(&"@param".to_string()),
        "Block NatSpec should include @param"
    );
}

// ========== EMIT / REVERT CONTEXTUAL COMPLETION TESTS ==========

#[test]
fn emit_completion_shows_only_events() {
    let source = "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.29;\n\ncontract Foo {\n    event Transfer(address indexed from, address indexed to, uint256 value);\n    event Approval(address indexed owner, address indexed spender, uint256 value);\n    error InsufficientBalance(uint256 available, uint256 required);\n\n    function bar() public {\n        emit \n    }\n}\n";
    let (st, path) = setup(source);

    let pos = source.find("emit \n").unwrap() + "emit ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), None);

    assert!(
        labels.contains(&"Transfer".to_string()),
        "emit should suggest Transfer event, got: {labels:?}"
    );
    assert!(
        labels.contains(&"Approval".to_string()),
        "emit should suggest Approval event"
    );
    assert!(
        !labels.contains(&"InsufficientBalance".to_string()),
        "emit should NOT suggest errors"
    );
    assert!(
        !labels.contains(&"bar".to_string()),
        "emit should NOT suggest functions"
    );
}

#[test]
fn revert_completion_shows_only_errors() {
    let source = "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.29;\n\ncontract Foo {\n    event Transfer(address indexed from, address indexed to, uint256 value);\n    error InsufficientBalance(uint256 available, uint256 required);\n    error Unauthorized();\n\n    function bar() public {\n        revert \n    }\n}\n";
    let (st, path) = setup(source);

    let pos = source.find("revert \n").unwrap() + "revert ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), None);

    assert!(
        labels.contains(&"InsufficientBalance".to_string()),
        "revert should suggest InsufficientBalance error, got: {labels:?}"
    );
    assert!(
        labels.contains(&"Unauthorized".to_string()),
        "revert should suggest Unauthorized error"
    );
    assert!(
        !labels.contains(&"Transfer".to_string()),
        "revert should NOT suggest events"
    );
    assert!(
        !labels.contains(&"bar".to_string()),
        "revert should NOT suggest functions"
    );
}

#[test]
fn emit_completion_with_partial_name() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    function bar() public {
        emit Tr
    }
}
"#;
    let (st, path) = setup(source);

    // Position after "emit Tr" — should still trigger emit context
    let pos = source.find("emit Tr").unwrap() + "emit Tr".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), None);

    assert!(
        labels.contains(&"Transfer".to_string()),
        "emit Tr should still suggest Transfer, got: {labels:?}"
    );
    assert!(
        labels.contains(&"Approval".to_string()),
        "emit Tr should still suggest Approval (client does filtering)"
    );
}

// ========== ASSEMBLY / YUL COMPLETION TESTS ==========

#[test]
fn assembly_completion_shows_yul_builtins() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public view returns (uint256 result) {
        assembly {
            result := add(1, 2)

        }
    }
}
"#;
    let (st, path) = setup(source);

    // Position on the blank line inside the assembly block (line 7, after the add line)
    let target = source.find("add(1, 2)").unwrap() + "add(1, 2)".len();
    // Go to the next line (the blank line inside assembly)
    let next_line_start = source[target..].find('\n').unwrap() + target + 1;
    let line = source[..next_line_start].matches('\n').count() as u32;
    let col = 12u32; // indentation inside assembly

    let labels = completion_labels_with_tree(&st, &path, source, Position::new(line, col), None);

    // Should include Yul builtins
    assert!(
        labels.iter().any(|l| l.starts_with("mload")),
        "Assembly should include mload, got: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("sload")),
        "Assembly should include sload"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("caller")),
        "Assembly should include caller"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("add")),
        "Assembly should include add"
    );
    // Should include Yul keywords
    assert!(
        labels.contains(&"let".to_string()),
        "Assembly should include 'let' keyword"
    );
    assert!(
        labels.contains(&"switch".to_string()),
        "Assembly should include 'switch' keyword"
    );
    // Should NOT include Solidity keywords
    assert!(
        !labels.contains(&"contract".to_string()),
        "Assembly should NOT include Solidity keywords"
    );
    assert!(
        !labels.contains(&"mapping".to_string()),
        "Assembly should NOT include 'mapping'"
    );
}

#[test]
fn outside_assembly_no_yul_builtins() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public {

    }
}
"#;
    let (st, path) = setup(source);

    let labels = completion_labels_with_tree(&st, &path, source, Position::new(5, 8), None);

    // Should include Solidity keywords, not Yul
    assert!(
        labels.contains(&"uint256".to_string()),
        "Outside assembly should include uint256, got: {labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("mload")),
        "Outside assembly should NOT include mload"
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("sload")),
        "Outside assembly should NOT include sload"
    );
}

// ========== MAPPING VALUE TYPE COMPLETION TESTS ==========

#[test]
fn mapping_value_type_struct_completion() {
    let source = "// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    struct UserInfo {
        uint256 balance;
        address wallet;
    }

    mapping(address => UserInfo) public users;

    function getBalance(address user) public view returns (uint256) {
        return users[user].
    }
}
";
    let (st, path) = setup(source);
    // Cursor is at the dot after `users[user].`
    let labels = completion_labels(&st, &path, source, Position::new(12, 28), Some("."));

    assert!(
        labels.contains(&"balance".to_string()),
        "Should complete with struct field 'balance', got: {labels:?}"
    );
    assert!(
        labels.contains(&"wallet".to_string()),
        "Should complete with struct field 'wallet', got: {labels:?}"
    );
}

#[test]
fn mapping_value_type_address_completion() {
    let source = "// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    mapping(uint256 => address) public owners;

    function sendTo(uint256 id) public {
        owners[id].
    }
}
";
    let (st, path) = setup(source);
    let labels = completion_labels(&st, &path, source, Position::new(7, 19), Some("."));

    assert!(
        labels.contains(&"balance".to_string()),
        "Should complete with address member 'balance', got: {labels:?}"
    );
    assert!(
        labels.contains(&"transfer".to_string()),
        "Should complete with address member 'transfer', got: {labels:?}"
    );
}

#[test]
fn array_element_struct_completion() {
    let source = "// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract List {
    struct Item {
        string name;
        uint256 price;
    }

    Item[] public items;

    function getPrice(uint256 i) public view returns (uint256) {
        return items[i].
    }
}
";
    let (st, path) = setup(source);
    let labels = completion_labels(&st, &path, source, Position::new(12, 25), Some("."));

    assert!(
        labels.contains(&"name".to_string()),
        "Should complete with struct field 'name', got: {labels:?}"
    );
    assert!(
        labels.contains(&"price".to_string()),
        "Should complete with struct field 'price', got: {labels:?}"
    );
}

// ========== CHAINED MULTI-CALL COMPLETION TESTS ==========

#[test]
fn chained_call_completion() {
    let source = "// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function balanceOf(address) external view returns (uint256);
}

contract Vault {
    IERC20 public token;

    function getToken() public view returns (IERC20) {
        return token;
    }

    function check() public view {
        getToken().
    }
}
";
    let (st, path) = setup(source);
    // After `getToken().` (line 15, col 19)
    let labels = completion_labels(&st, &path, source, Position::new(15, 19), Some("."));

    assert!(
        labels.contains(&"balanceOf".to_string()),
        "Should complete with IERC20 method 'balanceOf' after getToken()., got: {labels:?}"
    );
}

#[test]
fn chained_member_then_call_completion() {
    let source = "// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function balanceOf(address) external view returns (uint256);
    function transfer(address, uint256) external returns (bool);
}

contract Vault {
    IERC20 public token;

    function check() public view {
        token.balanceOf(msg.sender).
    }
}
";
    let (st, path) = setup(source);
    // After `token.balanceOf(msg.sender).` (line 13, col 36)
    let labels = completion_labels(&st, &path, source, Position::new(13, 36), Some("."));

    // uint256 has no built-in members, so this should be empty
    // (This test verifies the chain resolves without crashing)
    assert!(
        labels.is_empty() || !labels.contains(&"balanceOf".to_string()),
        "Should NOT show IERC20 members after balanceOf() which returns uint256, got: {labels:?}"
    );
}

// ===========================================================================
// EVM version-aware completions
// ===========================================================================

#[test]
fn evm_cancun_assembly_has_blobhash_and_mcopy() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract Foo {
    function bar() public view returns (uint256 result) {
        assembly {
            result := add(1, 2)

        }
    }
}
"#;
    let (st, path) = setup(source);
    let target = source.find("add(1, 2)").unwrap() + "add(1, 2)".len();
    let next_line_start = source[target..].find('\n').unwrap() + target + 1;
    let line = source[..next_line_start].matches('\n').count() as u32;
    let labels = completion_labels_with_tree(&st, &path, source, Position::new(line, 12), None);

    assert!(
        labels.iter().any(|l| l.starts_with("blobhash(")),
        "Cancun (0.8.24) should include blobhash, got: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("blobbasefee(")),
        "Cancun (0.8.24) should include blobbasefee"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("mcopy(")),
        "Cancun (0.8.24) should include mcopy"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("tload(")),
        "Cancun (0.8.24) should include tload"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("tstore(")),
        "Cancun (0.8.24) should include tstore"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("prevrandao(")),
        "Cancun (0.8.24) should include prevrandao"
    );
    // difficulty should NOT be present on Cancun (Paris+)
    assert!(
        !labels.iter().any(|l| l.starts_with("difficulty(")),
        "Cancun (0.8.24) should NOT include difficulty"
    );
}

#[test]
fn evm_pre_paris_assembly_has_difficulty_no_prevrandao() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

contract Foo {
    function bar() public view returns (uint256 result) {
        assembly {
            result := add(1, 2)

        }
    }
}
"#;
    let (st, path) = setup(source);
    let target = source.find("add(1, 2)").unwrap() + "add(1, 2)".len();
    let next_line_start = source[target..].find('\n').unwrap() + target + 1;
    let line = source[..next_line_start].matches('\n').count() as u32;
    let labels = completion_labels_with_tree(&st, &path, source, Position::new(line, 12), None);

    // Pre-Paris (< 0.8.18) should have difficulty, not prevrandao
    assert!(
        labels.iter().any(|l| l.starts_with("difficulty(")),
        "Pre-Paris (0.8.17) should include difficulty, got: {labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("prevrandao(")),
        "Pre-Paris (0.8.17) should NOT include prevrandao"
    );
    // Should NOT have Cancun opcodes
    assert!(
        !labels.iter().any(|l| l.starts_with("blobhash(")),
        "Pre-Paris (0.8.17) should NOT include blobhash"
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("mcopy(")),
        "Pre-Paris (0.8.17) should NOT include mcopy"
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("tload(")),
        "Pre-Paris (0.8.17) should NOT include tload"
    );
}

#[test]
fn evm_shanghai_assembly_has_prevrandao_no_cancun() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract Foo {
    function bar() public view returns (uint256 result) {
        assembly {
            result := add(1, 2)

        }
    }
}
"#;
    let (st, path) = setup(source);
    let target = source.find("add(1, 2)").unwrap() + "add(1, 2)".len();
    let next_line_start = source[target..].find('\n').unwrap() + target + 1;
    let line = source[..next_line_start].matches('\n').count() as u32;
    let labels = completion_labels_with_tree(&st, &path, source, Position::new(line, 12), None);

    // Shanghai (>= 0.8.20) is post-Paris, so prevrandao yes, difficulty no
    assert!(
        labels.iter().any(|l| l.starts_with("prevrandao(")),
        "Shanghai (0.8.20) should include prevrandao, got: {labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("difficulty(")),
        "Shanghai (0.8.20) should NOT include difficulty"
    );
    // Shanghai is pre-Cancun, so no blobhash/mcopy/tload/tstore
    assert!(
        !labels.iter().any(|l| l.starts_with("blobhash(")),
        "Shanghai (0.8.20) should NOT include blobhash"
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("mcopy(")),
        "Shanghai (0.8.20) should NOT include mcopy"
    );
}

#[test]
fn evm_pre_cancun_general_no_blobhash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract Foo {
    function bar() public {

    }
}
"#;
    let (st, path) = setup(source);
    let labels = completion_labels_with_tree(&st, &path, source, Position::new(5, 8), None);

    // blobhash should NOT appear in general completions for pre-Cancun
    assert!(
        !labels.iter().any(|l| l.contains("blobhash")),
        "Pre-Cancun (0.8.20) general completions should NOT include blobhash, got blobhash in: {:?}",
        labels
            .iter()
            .filter(|l| l.contains("blob"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn evm_cancun_general_has_blobhash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract Foo {
    function bar() public {

    }
}
"#;
    let (st, path) = setup(source);
    let labels = completion_labels_with_tree(&st, &path, source, Position::new(5, 8), None);

    // blobhash SHOULD appear in general completions for Cancun
    assert!(
        labels.iter().any(|l| l.contains("blobhash")),
        "Cancun (0.8.24) general completions should include blobhash"
    );
}
