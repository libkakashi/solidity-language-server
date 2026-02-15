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

/// Helper: find the first occurrence of `needle` and return its Position.
fn first_position(source: &str, needle: &str) -> Position {
    let pos = source.find(needle).unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32;
    Position::new(line, col)
}

#[test]
fn find_references_struct_in_function_return_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    struct Position {
        uint256 size;
        uint256 collateral;
    }

    function getPosition() external view returns (Position memory) {
        return Position(0, 0);
    }
}
"#;
    let (st, path) = setup(source);

    // Cursor on the struct declaration name
    let pos = first_position(source, "Position");
    let refs = find_references(&st, &path, source, pos, true);
    // 1 declaration + 1 return type + 1 constructor call = 3
    assert_eq!(refs.len(), 3, "Expected 3 references, got {:?}", refs);
}

#[test]
fn find_references_struct_in_function_parameter_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct Entry {
        address addr;
        uint256 value;
    }

    function register(Entry memory entry) external {
    }

    function registerBatch(Entry[] memory entries) external {
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Entry");
    let refs = find_references(&st, &path, source, pos, true);
    // 1 declaration + 2 parameter type usages = 3
    assert_eq!(refs.len(), 3, "Expected 3 references, got {:?}", refs);
}

#[test]
fn find_references_struct_in_both_param_and_return() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Transform {
    struct Data {
        uint256 x;
    }

    function process(Data memory input) external pure returns (Data memory) {
        return input;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Data");
    let refs = find_references(&st, &path, source, pos, true);
    // 1 declaration + 1 param type + 1 return type = 3
    assert_eq!(refs.len(), 3, "Expected 3 references, got {:?}", refs);
}

#[test]
fn find_references_enum_in_return_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract StateMachine {
    enum Status { Active, Paused, Stopped }

    Status public current;

    function getStatus() external view returns (Status) {
        return current;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Status");
    let refs = find_references(&st, &path, source, pos, true);
    // 1 declaration + 1 state var type + 1 return type = 3
    assert_eq!(refs.len(), 3, "Expected 3 references, got {:?}", refs);
}

#[test]
fn find_references_in_event_parameter_types() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Marketplace {
    struct Order {
        address buyer;
        uint256 price;
    }

    event OrderPlaced(Order order);

    function placeOrder(Order memory order) external {
        emit OrderPlaced(order);
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Order");
    let refs = find_references(&st, &path, source, pos, true);
    // 1 declaration + 1 event param type + 1 function param type = 3
    assert_eq!(refs.len(), 3, "Expected 3 references, got {:?}", refs);
}

#[test]
fn find_references_in_error_parameter_types() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Lending {
    struct Loan {
        uint256 amount;
        uint256 due;
    }

    error InvalidLoan(Loan loan);

    function validate(Loan memory loan) internal pure {
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Loan");
    let refs = find_references(&st, &path, source, pos, true);
    // 1 declaration + 1 error param type + 1 function param type = 3
    assert_eq!(refs.len(), 3, "Expected 3 references, got {:?}", refs);
}

#[test]
fn find_references_struct_used_across_multiple_contexts() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract MultiRef {
    struct Token {
        address addr;
        uint256 amount;
    }

    Token public stored;

    event TokenUpdated(Token token);

    error InvalidToken(Token token);

    function update(Token memory t) external returns (Token memory) {
        stored = t;
        emit TokenUpdated(t);
        return t;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Token");
    let refs = find_references(&st, &path, source, pos, true);
    // 1 declaration
    // + 1 state variable type
    // + 1 event param type
    // + 1 error param type
    // + 1 function param type
    // + 1 function return type
    // = 6
    assert_eq!(refs.len(), 6, "Expected 6 references, got {:?}", refs);
}

#[test]
fn find_references_multiple_return_parameters() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Pair {
    struct Info {
        uint256 x;
    }

    function split(Info memory input) external pure returns (Info memory a, Info memory b) {
        a = input;
        b = input;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Info");
    let refs = find_references(&st, &path, source, pos, true);
    // 1 declaration + 1 param type + 2 return types = 4
    assert_eq!(refs.len(), 4, "Expected 4 references, got {:?}", refs);
}

#[test]
fn find_references_contract_type_in_function_signature() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract IERC20 {
    function balanceOf(address) external view returns (uint256) {
        return 0;
    }
}

contract Vault {
    function getToken() external pure returns (IERC20) {
        return IERC20(address(0));
    }

    function deposit(IERC20 token) external {
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "IERC20");
    let refs = find_references(&st, &path, source, pos, true);
    // 1 declaration + 1 return type + 1 constructor call + 1 param type = 4
    assert_eq!(refs.len(), 4, "Expected 4 references, got {:?}", refs);
}
