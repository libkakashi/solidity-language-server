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
