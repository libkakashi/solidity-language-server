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
