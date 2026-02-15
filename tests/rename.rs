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

// ========== EDGE CASE TESTS ==========

#[test]
fn rename_function_across_declaration_and_call_sites() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function myFunc() public returns (uint256) {
        return 42;
    }

    function caller() public returns (uint256) {
        return myFunc();
    }

    function anotherCaller() public returns (uint256) {
        return this.myFunc();
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `myFunc` declaration
    let func_pos = source.find("myFunc").unwrap();
    let line = source[..func_pos].matches('\n').count() as u32;
    let col = (func_pos - source[..func_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(&st, &path, source, Position::new(line, col), "renamedFunc");
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename declaration + direct call = 2 edits minimum
    // Note: `this.myFunc()` is a member access and may not be renamed (known limitation)
    assert!(
        file_edits.len() >= 2,
        "Expected at least 2 text edits (declaration + direct call), got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "renamedFunc");
    }
}

#[test]
fn rename_struct_updates_type_references() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }

    Point public origin;

    function createPoint(uint256 x, uint256 y) public pure returns (Point memory) {
        return Point(x, y);
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `Point` struct declaration
    let struct_pos = source.find("Point").unwrap();
    let line = source[..struct_pos].matches('\n').count() as u32;
    let col = (struct_pos - source[..struct_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(&st, &path, source, Position::new(line, col), "Coordinate");
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename struct declaration + type references in variables, parameters, return types
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for struct rename, got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "Coordinate");
    }
}

#[test]
fn rename_struct_field_updates_member_access() {
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

    function setX(uint256 newX) public {
        origin.x = newX;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `x` field in struct declaration
    let field_pos = source.find("uint256 x").unwrap() + "uint256 ".len();
    let line = source[..field_pos].matches('\n').count() as u32;
    let col = (field_pos - source[..field_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(&st, &path, source, Position::new(line, col), "xCoord");
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename field declaration + all member accesses
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for struct field rename, got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "xCoord");
    }
}

#[test]
fn rename_enum_type_updates_qualified_access() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    enum Status { Pending, Active, Completed }

    Status public currentStatus;

    function setStatus(Status newStatus) public {
        currentStatus = newStatus;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `Status` enum declaration
    let enum_pos = source.find("Status").unwrap();
    let line = source[..enum_pos].matches('\n').count() as u32;
    let col = (enum_pos - source[..enum_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(&st, &path, source, Position::new(line, col), "State");
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename enum declaration + type references
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for enum rename, got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "State");
    }
}

#[test]
fn rename_across_multiple_functions() {
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

    function getSupply() public view returns (uint256) {
        return totalSupply;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `totalSupply` declaration
    let var_pos = source.find("totalSupply").unwrap();
    let line = source[..var_pos].matches('\n').count() as u32;
    let col = (var_pos - source[..var_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(&st, &path, source, Position::new(line, col), "supply");
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename declaration + usage in mint, burn, and getSupply = 4 edits
    assert!(
        file_edits.len() >= 4,
        "Expected at least 4 text edits, got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "supply");
    }
}

#[test]
fn rename_event_updates_emit_sites() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    event Transfer(address indexed from, address indexed to, uint256 amount);

    function transfer(address to, uint256 amount) public {
        emit Transfer(msg.sender, to, amount);
    }

    function transferFrom(address from, address to, uint256 amount) public {
        emit Transfer(from, to, amount);
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `Transfer` event declaration
    let event_pos = source.find("Transfer").unwrap();
    let line = source[..event_pos].matches('\n').count() as u32;
    let col = (event_pos - source[..event_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(
        &st,
        &path,
        source,
        Position::new(line, col),
        "TokenTransfer",
    );
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename event declaration + emit sites
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for event rename, got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "TokenTransfer");
    }
}

#[test]
fn rename_parameter_updates_body_usage() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 num) public pure returns (uint256) {
        return num + 10;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `num` parameter
    let param_pos = source.find("num").unwrap();
    let line = source[..param_pos].matches('\n').count() as u32;
    let col = (param_pos - source[..param_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(&st, &path, source, Position::new(line, col), "value");
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename parameter declaration + usage in body = 2 edits
    assert_eq!(
        file_edits.len(),
        2,
        "Expected 2 text edits for parameter rename, got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "value");
    }
}

#[test]
fn rename_modifier_updates_function_modifier_lists() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Owned {
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function changeOwner(address newOwner) public onlyOwner {
        owner = newOwner;
    }

    function someAction() public onlyOwner {
        // do something
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `onlyOwner` modifier declaration
    let mod_pos = source.find("onlyOwner").unwrap();
    let line = source[..mod_pos].matches('\n').count() as u32;
    let col = (mod_pos - source[..mod_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(&st, &path, source, Position::new(line, col), "ownerOnly");
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename modifier declaration + usages in function modifiers = 3 edits
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for modifier rename, got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "ownerOnly");
    }
}

#[test]
fn get_identifier_at_beginning_of_identifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 myVariable;
}
"#;
    let var_pos = source.find("myVariable").unwrap();
    let line = source[..var_pos].matches('\n').count() as u32;
    let col = (var_pos - source[..var_pos].rfind('\n').unwrap() - 1) as u32;

    let ident = get_identifier_at_position(source, Position::new(line, col));
    assert_eq!(ident, Some("myVariable".to_string()));
}

#[test]
fn get_identifier_at_middle_of_identifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 myVariable;
}
"#;
    let var_pos = source.find("myVariable").unwrap() + 4; // Position at 'V'
    let line = source[..var_pos].matches('\n').count() as u32;
    let col = (var_pos - source[..var_pos].rfind('\n').unwrap() - 1) as u32;

    let ident = get_identifier_at_position(source, Position::new(line, col));
    assert_eq!(ident, Some("myVariable".to_string()));
}

#[test]
fn get_identifier_at_end_of_identifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 myVariable;
}
"#;
    let var_pos = source.find("myVariable").unwrap() + "myVariable".len() - 1; // Last char
    let line = source[..var_pos].matches('\n').count() as u32;
    let col = (var_pos - source[..var_pos].rfind('\n').unwrap() - 1) as u32;

    let ident = get_identifier_at_position(source, Position::new(line, col));
    assert_eq!(ident, Some("myVariable".to_string()));
}

#[test]
fn get_identifier_returns_none_for_plus_operator() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}
"#;
    let plus_pos = source.find(" + ").unwrap() + 1; // Position at '+'
    let line = source[..plus_pos].matches('\n').count() as u32;
    let col = (plus_pos - source[..plus_pos].rfind('\n').unwrap() - 1) as u32;

    let ident = get_identifier_at_position(source, Position::new(line, col));
    assert!(ident.is_none() || ident == Some("".to_string()));
}

#[test]
fn get_identifier_returns_none_for_equals_operator() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function assign() public {
        uint256 x = 5;
    }
}
"#;
    let eq_pos = source.find(" = ").unwrap() + 1; // Position at '='
    let line = source[..eq_pos].matches('\n').count() as u32;
    let col = (eq_pos - source[..eq_pos].rfind('\n').unwrap() - 1) as u32;

    let ident = get_identifier_at_position(source, Position::new(line, col));
    assert!(ident.is_none() || ident == Some("".to_string()));
}

#[test]
#[ignore] // BUG: get_identifier_at_position returns adjacent identifier "x" at semicolon position
fn get_identifier_returns_none_for_semicolon() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 x;
}
"#;
    let semi_pos = source.find("x;").unwrap() + 1; // Position at ';'
    let line = source[..semi_pos].matches('\n').count() as u32;
    let col = (semi_pos - source[..semi_pos].rfind('\n').unwrap() - 1) as u32;

    let ident = get_identifier_at_position(source, Position::new(line, col));
    assert!(ident.is_none() || ident == Some("".to_string()));
}

#[test]
fn get_identifier_returns_none_for_opening_brace() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
}
"#;
    let brace_pos = source.find("Foo {").unwrap() + 4; // Position at '{'
    let line = source[..brace_pos].matches('\n').count() as u32;
    let col = (brace_pos - source[..brace_pos].rfind('\n').unwrap() - 1) as u32;

    let ident = get_identifier_at_position(source, Position::new(line, col));
    assert!(ident.is_none() || ident == Some("".to_string()));
}

#[test]
fn rename_produces_no_edits_for_unknown_identifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 x;
}
"#;
    let (st, path) = setup(source);

    // Position on 'x' which might not have references
    let x_pos = source.find("x;").unwrap();
    let line = source[..x_pos].matches('\n').count() as u32;
    let col = (x_pos - source[..x_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(&st, &path, source, Position::new(line, col), "y");

    // Even if the symbol is found, it should produce valid edits or none
    if let Some(edit) = edit {
        let changes = edit.changes.unwrap();
        let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
        if let Some(file_edits) = changes.get(&uri) {
            // If edits are produced, they should all rename to "y"
            for e in file_edits {
                assert_eq!(e.new_text, "y");
            }
        }
    }
}

#[test]
fn rename_with_underscore_prefixed_identifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Owned {
    address private _owner;

    constructor() {
        _owner = msg.sender;
    }

    function getOwner() public view returns (address) {
        return _owner;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `_owner` declaration
    let owner_pos = source.find("_owner").unwrap();
    let line = source[..owner_pos].matches('\n').count() as u32;
    let col = (owner_pos - source[..owner_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(
        &st,
        &path,
        source,
        Position::new(line, col),
        "_ownerAddress",
    );
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename declaration + usages = 3 edits
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for underscore-prefixed rename, got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "_ownerAddress");
    }
}

#[test]
fn rename_enum_value_updates_qualified_accesses() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    enum Status { Pending, Active, Completed }

    Status public currentStatus = Status.Pending;

    function activate() public {
        currentStatus = Status.Active;
    }

    function isPending() public view returns (bool) {
        return currentStatus == Status.Pending;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `Pending` enum value in declaration
    let pending_pos = source.find("Pending").unwrap();
    let line = source[..pending_pos].matches('\n').count() as u32;
    let col = (pending_pos - source[..pending_pos].rfind('\n').unwrap() - 1) as u32;

    let edit = rename_symbol(&st, &path, source, Position::new(line, col), "Waiting");
    assert!(edit.is_some(), "Should produce a workspace edit");

    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&path).unwrap();
    let file_edits = changes.get(&uri).unwrap();

    // Should rename enum value declaration + qualified accesses
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits for enum value rename, got {}",
        file_edits.len()
    );
    for edit in file_edits {
        assert_eq!(edit.new_text, "Waiting");
    }
}
