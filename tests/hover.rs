use std::path::PathBuf;

use solidity_language_server::hover::hover_info;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
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
    let hover = hover_info(st, path, source, pos, &LineIndex::new(source))?;
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

// ---------------------------------------------------------------------------
// Edge case tests
// ---------------------------------------------------------------------------

#[test]
fn hover_on_constructor() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    address public owner;

    constructor(address _owner) {
        owner = _owner;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on `constructor` keyword
    let pos = source.find("constructor(address").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for constructor");
    let text = text.unwrap();
    assert!(
        text.contains("constructor("),
        "Should show constructor signature, got: {text}"
    );
    assert!(
        text.contains("address"),
        "Should show parameter type, got: {text}"
    );
}

#[test]
fn hover_on_fallback_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Proxy {
    fallback() external payable {}
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("fallback()").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for fallback");
    let text = text.unwrap();
    assert!(
        text.contains("fallback"),
        "Should show fallback signature, got: {text}"
    );
}

#[test]
fn hover_on_receive_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    receive() external payable {}
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("receive()").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for receive");
    let text = text.unwrap();
    assert!(
        text.contains("receive()"),
        "Should show receive signature, got: {text}"
    );
    assert!(
        text.contains("payable"),
        "Should show payable keyword, got: {text}"
    );
}

#[test]
fn hover_on_modifier_declaration() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    modifier onlyAdmin(address caller) {
        _;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("onlyAdmin").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for modifier");
    let text = text.unwrap();
    assert!(
        text.contains("modifier onlyAdmin"),
        "Should show modifier signature, got: {text}"
    );
    assert!(
        text.contains("address caller"),
        "Should show modifier parameters, got: {text}"
    );
}

#[test]
fn hover_on_event_declaration() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    /// @notice Emitted when tokens are transferred
    event Transfer(address indexed from, address indexed to, uint256 amount);
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("Transfer").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for event");
    let text = text.unwrap();
    assert!(
        text.contains("event Transfer"),
        "Should show event signature, got: {text}"
    );
}

#[test]
fn hover_on_error_definition() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    error InsufficientBalance(uint256 available, uint256 required);

    function transfer(uint256 amount) public {
        revert InsufficientBalance(0, amount);
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on error usage in revert
    let pos = source.find("revert InsufficientBalance").unwrap() + "revert ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for error usage");
    let text = text.unwrap();
    assert!(
        text.contains("error InsufficientBalance"),
        "Should show error signature, got: {text}"
    );
    assert!(
        text.contains("uint256 available"),
        "Should show error params, got: {text}"
    );
}

#[test]
fn hover_on_enum_value_qualified() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Game {
    enum Status { Active, Paused, Ended }

    function start() public pure returns (Status) {
        return Status.Active;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "Active" in "Status.Active" inside return statement
    let pos = source.find("return Status.Active").unwrap() + "return Status.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for enum value Active");
    let text = text.unwrap();
    assert!(
        text.contains("Active"),
        "Should show enum value name, got: {text}"
    );
}

#[test]
fn hover_on_loop_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Iter {
    function loop_test() public pure {
        for (uint256 i = 0; i < 10; i++) {
            uint256 x = i;
        }
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on `i` in `uint256 x = i;`
    let pos = source.find("uint256 x = i").unwrap() + "uint256 x = ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for loop variable i");
    let text = text.unwrap();
    assert!(
        text.contains("uint256"),
        "Should show loop variable type, got: {text}"
    );
}

#[test]
fn hover_on_mapping_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ledger {
    mapping(address => uint256) public balances;
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("balances").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for mapping variable");
    let text = text.unwrap();
    assert!(
        text.contains("mapping"),
        "Should show mapping type, got: {text}"
    );
    assert!(
        text.contains("balances"),
        "Should show variable name, got: {text}"
    );
}

#[test]
fn hover_on_array_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Store {
    uint256[] public items;
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("items").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for array variable");
    let text = text.unwrap();
    assert!(
        text.contains("uint256[]"),
        "Should show array type, got: {text}"
    );
}

#[test]
fn hover_on_immutable_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    address public immutable deployer;

    constructor() {
        deployer = msg.sender;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("deployer").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for immutable variable");
    let text = text.unwrap();
    assert!(
        text.contains("immutable"),
        "Should show immutable modifier, got: {text}"
    );
    assert!(text.contains("address"), "Should show type, got: {text}");
}

#[test]
fn hover_on_function_with_multiple_returns() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Multi {
    function getInfo() public pure returns (uint256 id, address owner, bool active) {
        return (1, address(0), true);
    }

    function test() public pure {
        getInfo();
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on `getInfo` in the call
    let pos = source.find("getInfo();").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for multi-return function"
    );
    let text = text.unwrap();
    assert!(
        text.contains("function getInfo"),
        "Should show function name, got: {text}"
    );
    assert!(
        text.contains("returns"),
        "Should show returns keyword, got: {text}"
    );
    assert!(
        text.contains("uint256 id"),
        "Should show first return param, got: {text}"
    );
    assert!(
        text.contains("bool active"),
        "Should show last return param, got: {text}"
    );
}

#[test]
fn hover_on_interface_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
}
"#;
    let (st, path) = setup(source);

    let pos = source.find("balanceOf").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for interface function");
    let text = text.unwrap();
    assert!(
        text.contains("function balanceOf"),
        "Should show function signature, got: {text}"
    );
    assert!(
        text.contains("address account"),
        "Should show parameter, got: {text}"
    );
    assert!(
        text.contains("view"),
        "Should show state mutability, got: {text}"
    );
}

#[test]
fn hover_on_inherited_state_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    uint256 public value;
}

contract Child is Base {
    function getValue() public view returns (uint256) {
        return value;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on `value` in `return value;`
    let pos = source.find("return value").unwrap() + "return ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for inherited state variable"
    );
    let text = text.unwrap();
    assert!(
        text.contains("uint256"),
        "Should show inherited variable type, got: {text}"
    );
    assert!(
        text.contains("value"),
        "Should show inherited variable name, got: {text}"
    );
}

#[test]
fn hover_on_whitespace_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Empty {
    uint256 public x;
}
"#;
    let (st, path) = setup(source);

    // Position on an empty line (line 2 is blank)
    let text = hover_text(source, &st, &path, Position::new(2, 0));
    assert!(text.is_none(), "Hover on whitespace should return None");
}

#[test]
fn hover_on_comment_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

// This is a comment
contract Foo {
    uint256 public x;
}
"#;
    let (st, path) = setup(source);

    // Position on the comment line (line 3: "// This is a comment")
    let text = hover_text(source, &st, &path, Position::new(3, 5));
    assert!(text.is_none(), "Hover on comment should return None");
}

#[test]
fn hover_on_free_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

function freeAdd(uint256 a, uint256 b) pure returns (uint256) {
    return a + b;
}

contract Calc {
    function compute() public pure returns (uint256) {
        return freeAdd(1, 2);
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on `freeAdd` in `return freeAdd(1, 2);`
    let pos = source.find("freeAdd(1, 2)").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for free function");
    let text = text.unwrap();
    assert!(
        text.contains("function freeAdd"),
        "Should show free function signature, got: {text}"
    );
    assert!(
        text.contains("uint256 a"),
        "Should show parameter, got: {text}"
    );
}

#[test]
fn hover_on_struct_field_declaration() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct Record {
        bytes32 id;
        address owner;
        uint256 timestamp;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "owner" field in the struct declaration
    let pos = source.find("address owner").unwrap() + "address ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for struct field declaration"
    );
    let text = text.unwrap();
    assert!(
        text.contains("address"),
        "Should show struct field type, got: {text}"
    );
    assert!(
        text.contains("owner"),
        "Should show struct field name, got: {text}"
    );
}

#[test]
fn hover_on_library_function_qualified_access() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library SafeMath {
    function mul(uint256 a, uint256 b) internal pure returns (uint256) {
        return a * b;
    }
}

contract Calculator {
    function compute() public pure returns (uint256) {
        return SafeMath.mul(3, 4);
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "mul" in "SafeMath.mul"
    let pos = source.find("SafeMath.mul(3").unwrap() + "SafeMath.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for library qualified function"
    );
    let text = text.unwrap();
    assert!(
        text.contains("function mul"),
        "Should show function signature, got: {text}"
    );
    assert!(
        text.contains("uint256 a"),
        "Should show parameter, got: {text}"
    );
}

#[test]
fn hover_on_cross_file_imported_struct() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let types_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

struct Order {
    uint256 id;
    address buyer;
    uint256 price;
}
"#;
    let types_path = tmp.path().join("Types.sol");
    std::fs::write(&types_path, types_source).unwrap();

    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Order} from "./Types.sol";

contract Exchange {
    function test() public pure {
        Order memory o;
        address b = o.buyer;
    }
}
"#;
    let main_path = tmp.path().join("Exchange.sol");
    std::fs::write(&main_path, main_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&types_path, types_source, &mut parser);
    st.resolve_file_references(&types_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // Hover on `buyer` in `o.buyer` -- resolves to the struct field from
    // the cross-file imported struct.
    let pos = main_source.find("o.buyer").unwrap() + "o.".len();
    let line = main_source[..pos].matches('\n').count() as u32;
    let col = (pos - main_source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(main_source, &st, &main_path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for cross-file struct field"
    );
    let text = text.unwrap();
    assert!(
        text.contains("address"),
        "Should show field type from cross-file struct, got: {text}"
    );
    assert!(
        text.contains("buyer"),
        "Should show field name, got: {text}"
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

// ---------------------------------------------------------------------------
// Built-in globals (msg, block, tx) hover tests
// ---------------------------------------------------------------------------

#[test]
fn hover_on_msg_sender() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    function getOwner() public view returns (address) {
        return msg.sender;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "sender" in "msg.sender"
    let pos = source.find("msg.sender").unwrap() + "msg.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for msg.sender");
    let text = text.unwrap();
    assert!(
        text.contains("address"),
        "Should show address type, got: {text}"
    );
    assert!(
        text.contains("sender"),
        "Should show sender name, got: {text}"
    );
}

#[test]
fn hover_on_block_timestamp() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Timer {
    function getTime() public view returns (uint256) {
        return block.timestamp;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "timestamp" in "block.timestamp"
    let pos = source.find("block.timestamp").unwrap() + "block.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for block.timestamp");
    let text = text.unwrap();
    assert!(
        text.contains("uint256"),
        "Should show uint256 type, got: {text}"
    );
    assert!(
        text.contains("timestamp"),
        "Should show timestamp name, got: {text}"
    );
}

#[test]
fn hover_on_tx_gasprice() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract GasInfo {
    function getGasPrice() public view returns (uint256) {
        return tx.gasprice;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "gasprice" in "tx.gasprice"
    let pos = source.find("tx.gasprice").unwrap() + "tx.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for tx.gasprice");
    let text = text.unwrap();
    assert!(
        text.contains("uint256"),
        "Should show uint256 type, got: {text}"
    );
    assert!(
        text.contains("gasprice"),
        "Should show gasprice name, got: {text}"
    );
}

// ---------------------------------------------------------------------------
// Built-in type member hover tests (address, array, super)
// ---------------------------------------------------------------------------

#[test]
fn hover_on_address_balance() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Wallet {
    address public owner;

    function getBalance() public view returns (uint256) {
        return owner.balance;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "balance" in "owner.balance"
    let pos = source.find("owner.balance").unwrap() + "owner.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for address.balance");
    let text = text.unwrap();
    assert!(
        text.contains("uint256"),
        "Should show uint256 type, got: {text}"
    );
    assert!(
        text.contains("balance"),
        "Should show balance name, got: {text}"
    );
}

#[test]
fn hover_on_array_length() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Store {
    uint256[] public items;

    function count() public view returns (uint256) {
        return items.length;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "length" in "items.length"
    let pos = source.find("items.length").unwrap() + "items.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for array.length");
    let text = text.unwrap();
    assert!(
        text.contains("uint256"),
        "Should show uint256 type, got: {text}"
    );
    assert!(
        text.contains("length"),
        "Should show length name, got: {text}"
    );
}

#[test]
fn hover_on_super_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function foo() public pure returns (uint256) {
        return 42;
    }
}

contract Child is Base {
    function foo() public pure override returns (uint256) {
        return super.foo();
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "foo" in "super.foo()"
    let pos = source.find("super.foo").unwrap() + "super.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Should show hover for super.foo");
    let text = text.unwrap();
    assert!(
        text.contains("function foo"),
        "Should show function signature, got: {text}"
    );
}

#[test]
fn hover_on_enum_member_value() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Game {
    enum Status { Active, Paused, Ended }

    function start() public pure returns (Status) {
        return Status.Active;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "Active" in "Status.Active" in return statement
    let pos = source.find("return Status.Active").unwrap() + "return Status.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for enum member via Status.Active"
    );
    let text = text.unwrap();
    assert!(
        text.contains("Active"),
        "Should show enum value name, got: {text}"
    );
}

#[test]
fn hover_on_using_for_primitive_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

contract Foo {
    using SafeMath for uint256;

    function bar() public pure returns (uint256) {
        uint256 x = 1;
        return x.add(2);
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "add" in "x.add(2)"
    let pos = source.find("x.add(2)").unwrap() + "x.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for using-for method on primitive type"
    );
    let text = text.unwrap();
    assert!(
        text.contains("add") && text.contains("uint256"),
        "Should show function signature, got: {text}"
    );
}

#[test]
fn hover_on_using_for_struct_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

struct Counter {
    uint256 value;
}

library CounterLib {
    function increment(Counter storage c) internal {
        c.value += 1;
    }
}

contract Foo {
    using CounterLib for Counter;
    Counter private counter;

    function inc() public {
        counter.increment();
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "increment" in "counter.increment()"
    let pos = source.find("counter.increment()").unwrap() + "counter.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for using-for method on struct type"
    );
    let text = text.unwrap();
    assert!(
        text.contains("increment") && text.contains("Counter"),
        "Should show function signature, got: {text}"
    );
}

#[test]
fn hover_on_using_for_wildcard() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library Ops {
    function double(uint256 x) internal pure returns (uint256) {
        return x * 2;
    }
}

contract Foo {
    using Ops for *;

    function bar() public pure returns (uint256) {
        uint256 x = 5;
        return x.double();
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "double" in "x.double()"
    let pos = source.find("x.double()").unwrap() + "x.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for using-for wildcard method"
    );
    let text = text.unwrap();
    assert!(
        text.contains("double"),
        "Should show function signature, got: {text}"
    );
}

#[test]
fn hover_on_using_for_via_type_cast() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
}

library SafeERC20 {
    function safeTransferFrom(IERC20 token, address from, address to, uint256 value) internal {
    }
}

contract Vault {
    using SafeERC20 for IERC20;

    function deposit(address token, uint256 amount) external {
        IERC20(token).safeTransferFrom(msg.sender, address(this), amount);
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "safeTransferFrom" in "IERC20(token).safeTransferFrom(...)"
    let pos = source.find("IERC20(token).safeTransferFrom(msg").unwrap() + "IERC20(token).".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for using-for method via type cast"
    );
    let text = text.unwrap();
    assert!(
        text.contains("safeTransferFrom") && text.contains("IERC20"),
        "Should show function signature, got: {text}"
    );
}

#[test]
fn hover_on_using_for_cross_file_import() {
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

    function deposit(address token, uint256 amount) external {
        IERC20(token).safeTransferFrom(msg.sender, address(this), amount);
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

    // Hover on "safeTransferFrom" in "IERC20(token).safeTransferFrom(...)"
    let pos = main_source
        .find("IERC20(token).safeTransferFrom(msg")
        .unwrap()
        + "IERC20(token).".len();
    let line = main_source[..pos].matches('\n').count() as u32;
    let col = (pos - main_source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(main_source, &st, &main_path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for cross-file using-for method via type cast"
    );
    let text = text.unwrap();
    assert!(
        text.contains("safeTransferFrom"),
        "Should show function name, got: {text}"
    );
}

#[test]
fn hover_on_using_for_cross_file_struct() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let types_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

struct Counter {
    uint256 value;
}
"#;
    let types_path = tmp.path().join("Types.sol");
    std::fs::write(&types_path, types_source).unwrap();

    let lib_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Counter} from "./Types.sol";

library CounterLib {
    function increment(Counter storage c) internal {
        c.value += 1;
    }
}
"#;
    let lib_path = tmp.path().join("CounterLib.sol");
    std::fs::write(&lib_path, lib_source).unwrap();

    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Counter} from "./Types.sol";
import {CounterLib} from "./CounterLib.sol";

contract Foo {
    using CounterLib for Counter;
    Counter private counter;

    function inc() public {
        counter.increment();
    }
}
"#;
    let main_path = tmp.path().join("Foo.sol");
    std::fs::write(&main_path, main_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&types_path, types_source, &mut parser);
    st.resolve_file_references(&types_path, &mut parser);
    st.index_file(&lib_path, lib_source, &mut parser);
    st.resolve_file_references(&lib_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // Hover on "increment" in "counter.increment()"
    let pos = main_source.find("counter.increment()").unwrap() + "counter.".len();
    let line = main_source[..pos].matches('\n').count() as u32;
    let col = (pos - main_source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(main_source, &st, &main_path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for cross-file using-for method on imported struct"
    );
    let text = text.unwrap();
    assert!(
        text.contains("increment"),
        "Should show function name, got: {text}"
    );
}

#[test]
fn hover_on_inherited_member_via_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function foo() public pure returns (uint256) {
        return 42;
    }
}

contract Child is Base {
    function bar() public pure {
        Child c = Child(address(0));
        c.foo();
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on `foo` in `c.foo()`
    let pos = source.find("c.foo()").unwrap() + "c.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for inherited member via variable"
    );
    let text = text.unwrap();
    assert!(
        text.contains("function foo"),
        "Should show function signature, got: {text}"
    );
}

#[test]
fn hover_on_inherited_member_via_contract_name() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function foo() public pure returns (uint256) {
        return 42;
    }
}

contract Child is Base {}

contract User {
    function test() public pure {
        Child.foo();
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on `foo` in `Child.foo()`
    let pos = source.find("Child.foo()").unwrap() + "Child.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for inherited member via contract name"
    );
    let text = text.unwrap();
    assert!(
        text.contains("function foo"),
        "Should show function signature, got: {text}"
    );
}

#[test]
fn hover_on_grandparent_inherited_member() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract GrandParent {
    function ancient() public pure returns (uint256) {
        return 1;
    }
}

contract Parent is GrandParent {}

contract Child is Parent {
    function test() public pure {
        Child c = Child(address(0));
        c.ancient();
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on `ancient` in `c.ancient()`
    let pos = source.find("c.ancient()").unwrap() + "c.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for grandparent inherited member"
    );
    let text = text.unwrap();
    assert!(
        text.contains("function ancient"),
        "Should show function signature, got: {text}"
    );
}

#[test]
fn hover_on_qualified_imported_struct_field() {
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
        address a = user.account;
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

    // Hover on `account` in `user.account`
    let pos = main_source.find("user.account").unwrap() + "user.".len();
    let line = main_source[..pos].matches('\n').count() as u32;
    let col = (pos - main_source[..pos].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(main_source, &st, &main_path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should show hover for field of qualified imported struct"
    );
    let text = text.unwrap();
    assert!(
        text.contains("address"),
        "Should show field type, got: {text}"
    );
    assert!(
        text.contains("account"),
        "Should show field name, got: {text}"
    );
}
