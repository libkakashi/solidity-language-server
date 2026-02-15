use std::path::PathBuf;

use solidity_language_server::hover::hover_info;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use tower_lsp::lsp_types::{HoverContents, Position};

fn setup(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

fn hover_text(source: &str, st: &SymbolTable, path: &PathBuf, pos: Position) -> Option<String> {
    let hover = hover_info(st, path, source, pos)?;
    match hover.contents {
        HoverContents::Markup(markup) => Some(markup.value),
        _ => None,
    }
}

#[test]
fn hover_on_function_shows_signature() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    /// @notice Deposits tokens into the vault
    /// @param amount The amount to deposit
    /// @return success Whether the deposit succeeded
    function deposit(uint256 amount) public pure returns (bool success) {
        return amount > 0;
    }

    function test() public pure {
        deposit(100);
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on `deposit` in `deposit(100);`
    let call_pos = source.find("deposit(100)").unwrap();
    let line = source[..call_pos].matches('\n').count() as u32;
    let col = (call_pos - source[..call_pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for deposit");
    let text = text.unwrap();
    assert!(
        text.contains("function deposit"),
        "Should show function signature, got: {text}"
    );
    assert!(
        text.contains("uint256 amount"),
        "Should show parameter type"
    );
    assert!(text.contains("returns"), "Should show return type");
    assert!(text.contains("Deposits tokens"), "Should show NatSpec");
}

#[test]
fn hover_on_contract_shows_kind() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
}

contract Token is IERC20 {
    function transfer(address to, uint256 amount) external returns (bool) {
        return true;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on `IERC20` in `contract Token is IERC20`
    let usage = source.find("is IERC20").unwrap() + "is ".len();
    let line = source[..usage].matches('\n').count() as u32;
    let col = (usage - source[..usage].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for IERC20");
    let text = text.unwrap();
    assert!(
        text.contains("interface IERC20"),
        "Should show interface keyword, got: {text}"
    );
}

#[test]
fn hover_on_struct_shows_members() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct User {
        address wallet;
        uint256 balance;
        bool active;
    }

    User public admin;
}
"#;
    let (st, path) = setup(source);

    // Hover on `User` in `User public admin;`
    let usage = source.find("User public").unwrap();
    let line = source[..usage].matches('\n').count() as u32;
    let col = (usage - source[..usage].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for User struct");
    let text = text.unwrap();
    assert!(
        text.contains("struct User"),
        "Should show struct keyword, got: {text}"
    );
    assert!(text.contains("wallet"), "Should show member 'wallet'");
    assert!(text.contains("balance"), "Should show member 'balance'");
}

#[test]
fn hover_on_enum_shows_values() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract StateMachine {
    enum State { Idle, Running, Paused, Stopped }

    State public currentState;
}
"#;
    let (st, path) = setup(source);

    // Hover on `State` in `State public currentState;`
    let usage = source.find("State public").unwrap();
    let line = source[..usage].matches('\n').count() as u32;
    let col = (usage - source[..usage].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for State enum");
    let text = text.unwrap();
    assert!(
        text.contains("enum State"),
        "Should show enum keyword, got: {text}"
    );
    assert!(text.contains("Idle"), "Should show enum value Idle");
    assert!(text.contains("Stopped"), "Should show enum value Stopped");
}

#[test]
fn hover_on_state_variable_shows_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    /// @notice The total supply of tokens
    uint256 public constant MAX_SUPPLY = 1000000;
}
"#;
    let (st, path) = setup(source);

    // Hover on MAX_SUPPLY
    let pos = source.find("MAX_SUPPLY").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for MAX_SUPPLY");
    let text = text.unwrap();
    assert!(text.contains("uint256"), "Should show type");
    assert!(text.contains("constant"), "Should show constant modifier");
    assert!(text.contains("total supply"), "Should show NatSpec");
}

#[test]
fn hover_on_qualified_event_name() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IFees {
    event FeeUpdated(uint256 fee);
}
contract Pool {
    function emitFee() public {
        emit IFees.FeeUpdated(100);
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "FeeUpdated" in "IFees.FeeUpdated" (second occurrence)
    let pos = source.rfind("FeeUpdated").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for qualified FeeUpdated");
    let text = text.unwrap();
    assert!(
        text.contains("event FeeUpdated"),
        "Should show event signature, got: {text}"
    );
}

#[test]
fn hover_on_struct_field_via_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct User {
        address wallet;
        uint256 balance;
    }

    function test() public {
        User memory u;
        address w = u.wallet;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "wallet" in "u.wallet"
    let pos = source.find("u.wallet").unwrap() + 2; // skip "u."
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for struct field via variable"
    );
    let text = text.unwrap();
    assert!(
        text.contains("address"),
        "Should show field type, got: {text}"
    );
}

#[test]
fn hover_on_return_parameter() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    function getBalance() public pure returns (uint256 balance) {
        balance = 42;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "balance" in "balance = 42;"
    let pos = source.find("balance = 42").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for return parameter");
    let text = text.unwrap();
    assert!(
        text.contains("uint256"),
        "Should show return parameter type, got: {text}"
    );
}

#[test]
fn hover_on_enum_value_via_qualified_name() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    enum Status { Active, Paused }

    function bar() public {
        Status s = Status.Active;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "Active" in "Status.Active"
    let pos = source.find("Status.Active").unwrap() + "Status.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for enum value via qualified name"
    );
    let text = text.unwrap();
    assert!(
        text.contains("Active"),
        "Should show enum value name, got: {text}"
    );
}

#[test]
fn hover_on_library_function_via_dot() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

contract Calculator {
    function calc() public pure returns (uint256) {
        return MathLib.add(1, 2);
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "add" in "MathLib.add"
    let pos = source.find("MathLib.add(1").unwrap() + "MathLib.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for library function via dot"
    );
    let text = text.unwrap();
    assert!(
        text.contains("function add"),
        "Should show function signature, got: {text}"
    );
}

#[test]
fn hover_struct_literal_field_not_local_var() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }

    function test() public {
        uint256 x = 10;
        uint256 y = 20;
        Point memory p = Point({x: x, y: y});
        uint256 a = p.x;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on the value "x" (the second x in "x: x") should show the local var
    let struct_call = source.find("Point({x: x").unwrap();
    let value_x = source[struct_call..].find("x: x").unwrap() + struct_call + "x: ".len();
    let line = source[..value_x].matches('\n').count() as u32;
    let col = (value_x - source[..value_x].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should hover on value x in struct literal");
    let text = text.unwrap();
    assert!(
        text.contains("uint256"),
        "Value x should show local var type, got: {text}"
    );

    // Hover on the field name "x" (the first x in "x: x") should NOT resolve
    // to the local variable — currently it shows nothing, which is correct
    // (better than showing wrong info from the local var).
    let field_x = source[struct_call..].find("x: x").unwrap() + struct_call;
    let line2 = source[..field_x].matches('\n').count() as u32;
    let col2 = (field_x - source[..field_x].rfind('\n').unwrap() - 1) as u32;

    let text2 = hover_text(source, &st, &path, Position::new(line2, col2));
    // Field name should NOT resolve to the local variable (uint256 x = 10)
    if let Some(ref t) = text2 {
        assert!(
            !t.contains("uint256") || t.contains("struct"),
            "Struct field name should not show local var type, got: {t}"
        );
    }
}

#[test]
fn hover_on_inherited_modifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ownable {
    modifier onlyOwner() {
        _;
    }
}

contract MyContract is Ownable {
    function withdraw() public onlyOwner {
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "onlyOwner" in "function withdraw() public onlyOwner"
    let pos = source.find("public onlyOwner").unwrap() + "public ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for inherited modifier");
    let text = text.unwrap();
    assert!(
        text.contains("modifier onlyOwner"),
        "Should show modifier signature, got: {text}"
    );
}

#[test]
fn hover_on_imported_inherited_modifier() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let ownable_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ownable {
    modifier onlyOwner() {
        _;
    }
}
"#;
    let ownable_path = tmp.path().join("Ownable.sol");
    std::fs::write(&ownable_path, ownable_source).unwrap();

    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Ownable} from "./Ownable.sol";

contract MyContract is Ownable {
    function withdraw() public onlyOwner {
    }
}
"#;
    let main_path = tmp.path().join("MyContract.sol");
    std::fs::write(&main_path, main_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&ownable_path, ownable_source, &mut parser);
    st.resolve_file_references(&ownable_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // Hover on "onlyOwner" in "function withdraw() public onlyOwner"
    let pos = main_source.find("public onlyOwner").unwrap() + "public ".len();
    let line = main_source[..pos].matches('\n').count() as u32;
    let col = (pos - main_source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(main_source, &st, &main_path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for imported inherited modifier"
    );
    let text = text.unwrap();
    assert!(
        text.contains("modifier onlyOwner"),
        "Should show modifier signature, got: {text}"
    );
}

#[test]
fn hover_on_imported_contract_function() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let lib_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#;
    let lib_path = tmp.path().join("MathLib.sol");
    std::fs::write(&lib_path, lib_source).unwrap();

    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {MathLib} from "./MathLib.sol";

contract Calculator {
    function calc() public pure returns (uint256) {
        return MathLib.add(1, 2);
    }
}
"#;
    let main_path = tmp.path().join("Calculator.sol");
    std::fs::write(&main_path, main_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&lib_path, lib_source, &mut parser);
    st.resolve_file_references(&lib_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // Hover on "add" in "MathLib.add"
    let pos = main_source.find("MathLib.add(1").unwrap() + "MathLib.".len();
    let line = main_source[..pos].matches('\n').count() as u32;
    let col = (pos - main_source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(main_source, &st, &main_path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for imported library function"
    );
    let text = text.unwrap();
    assert!(
        text.contains("function add"),
        "Should show function signature, got: {text}"
    );
}
