use std::path::PathBuf;

use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::references::find_references;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
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

    let refs = find_references(
        &st,
        &path,
        source,
        Position::new(line, col),
        true,
        &LineIndex::new(source),
    );
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

    let refs = find_references(
        &st,
        &path,
        source,
        Position::new(line, col),
        false,
        &LineIndex::new(source),
    );
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

    let refs = find_references(
        &st,
        &path,
        source,
        Position::new(line, col),
        true,
        &LineIndex::new(source),
    );
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
    let refs = find_references(
        &st,
        &path,
        source,
        Position::new(0, 0),
        true,
        &LineIndex::new(source),
    );
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
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
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
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
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
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
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
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
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
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
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
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
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
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
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
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 1 param type + 2 return types = 4
    assert_eq!(refs.len(), 4, "Expected 4 references, got {:?}", refs);
}

#[test]
fn find_references_distinct_types_in_multiple_returns() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Exchange {
    struct Price {
        uint256 value;
    }

    struct Volume {
        uint256 amount;
    }

    function quote() external pure returns (Price memory, Volume memory) {
        return (Price(0), Volume(0));
    }
}
"#;
    let (st, path) = setup(source);

    // Check references to Price: 1 decl + 1 return type + 1 constructor call = 3
    let price_pos = first_position(source, "Price");
    let price_refs = find_references(&st, &path, source, price_pos, true, &LineIndex::new(source));
    assert_eq!(
        price_refs.len(),
        3,
        "Expected 3 references for Price, got {:?}",
        price_refs
    );

    // Check references to Volume: 1 decl + 1 return type + 1 constructor call = 3
    let volume_pos = first_position(source, "Volume");
    let volume_refs = find_references(
        &st,
        &path,
        source,
        volume_pos,
        true,
        &LineIndex::new(source),
    );
    assert_eq!(
        volume_refs.len(),
        3,
        "Expected 3 references for Volume, got {:?}",
        volume_refs
    );
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
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 1 return type + 1 constructor call + 1 param type = 4
    assert_eq!(refs.len(), 4, "Expected 4 references, got {:?}", refs);
}

#[test]
fn find_references_to_event_via_qualified_name() {
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

    // Position on "FeeUpdated" declaration inside IFees
    let pos = first_position(source, "FeeUpdated");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 1 qualified usage in emit = 2
    assert_eq!(
        refs.len(),
        2,
        "Expected 2 references for FeeUpdated (decl + qualified usage), got {:?}",
        refs
    );
}

#[test]
fn find_references_to_struct_field_via_member_access() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }

    function bar() public {
        Point memory p;
        uint256 a = p.x;
        uint256 b = p.x;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on "x" in struct declaration
    let pos = first_position(source, "x;");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 2 member accesses = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for struct field x, got {:?}",
        refs
    );
}

#[test]
fn find_references_to_enum_value_via_qualified_access() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    enum Status { Active, Paused }

    function bar() public {
        Status s = Status.Active;
        Status t = Status.Active;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on first "Active" (the declaration inside enum)
    let pos = first_position(source, "Active");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 2 qualified usages = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for Active, got {:?}",
        refs
    );
}

// ===================================================================
// Edge-case reference tests
// ===================================================================

#[test]
fn find_references_to_modifier_definition_and_usages() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    modifier onlyOwner() {
        _;
    }

    function withdraw() public onlyOwner {
    }

    function pause() public onlyOwner {
    }

    function resume() external onlyOwner {
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "onlyOwner");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 3 modifier invocations = 4
    assert_eq!(
        refs.len(),
        4,
        "Expected 4 references for onlyOwner (1 decl + 3 usages), got {:?}",
        refs
    );
}

#[test]
fn find_references_to_contract_via_new_constructor_calls() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public supply;
}

contract Factory {
    function create() public returns (Token) {
        Token t1 = new Token();
        Token t2 = new Token();
        return t1;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Token");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 1 return type + 2 local var types + 2 new calls = 6
    assert!(
        refs.len() >= 5,
        "Expected at least 5 references for Token (decl + return type + var types + new calls), got {:?}",
        refs
    );
}

#[test]
fn find_references_across_inheritance() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseFn() internal pure returns (uint256) {
        return 42;
    }
}

contract Derived is Base {
    function useFn() public pure returns (uint256) {
        return baseFn();
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "baseFn");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 1 call from Derived = 2
    assert_eq!(
        refs.len(),
        2,
        "Expected 2 references for baseFn (decl + inherited call), got {:?}",
        refs
    );
}

#[test]
fn find_references_to_free_function_used_in_multiple_contracts() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

function helper() pure returns (uint256) {
    return 1;
}

contract A {
    function use1() public pure returns (uint256) {
        return helper();
    }
}

contract B {
    function use2() public pure returns (uint256) {
        return helper();
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "helper");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 2 calls = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for free function helper, got {:?}",
        refs
    );
}

#[test]
fn find_references_to_library_function_via_qualified_calls() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

contract Calculator {
    function compute() public pure returns (uint256) {
        return MathLib.add(1, 2) + MathLib.add(3, 4);
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "add");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 2 qualified calls = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for MathLib.add (1 decl + 2 calls), got {:?}",
        refs
    );
}

#[test]
fn find_references_to_constant_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Config {
    uint256 constant MAX_SUPPLY = 1000000;

    function check(uint256 amount) public pure returns (bool) {
        return amount <= MAX_SUPPLY;
    }

    function getMax() public pure returns (uint256) {
        return MAX_SUPPLY;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "MAX_SUPPLY");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 2 usages = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for MAX_SUPPLY (1 decl + 2 usages), got {:?}",
        refs
    );
}

#[test]
fn find_references_to_enum_type_in_params_returns_state_vars() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Workflow {
    enum Phase { Init, Running, Done }

    Phase public currentPhase;

    function setPhase(Phase p) external {
        currentPhase = p;
    }

    function getPhase() external view returns (Phase) {
        return currentPhase;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Phase");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 1 state var type + 1 param type + 1 return type = 4
    assert_eq!(
        refs.len(),
        4,
        "Expected 4 references for Phase enum, got {:?}",
        refs
    );
}

#[test]
fn find_references_cross_file_glob_import() {
    let tmp = tempfile::tempdir().unwrap();
    let helper_path = tmp.path().join("Helper.sol");
    let main_path = tmp.path().join("Main.sol");

    let helper_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

struct Point {
    uint256 x;
    uint256 y;
}
"#;
    let main_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Helper.sol";

contract Canvas {
    Point public origin;

    function setOrigin(Point memory p) external {
        origin = p;
    }
}
"#;
    std::fs::write(&helper_path, helper_source).unwrap();
    std::fs::write(&main_path, main_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&helper_path, helper_source, &mut parser);
    st.resolve_file_references(&helper_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // Find references to Point from the helper file
    let pos = first_position(helper_source, "Point");
    let refs = find_references(
        &st,
        &helper_path,
        helper_source,
        pos,
        true,
        &LineIndex::new(helper_source),
    );
    // 1 declaration (in Helper.sol) + 2 usages (in Main.sol: state var type + param type) = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for Point via glob import (1 decl + 2 cross-file usages), got {:?}",
        refs
    );
}

#[test]
fn find_references_cross_file_named_import() {
    let tmp = tempfile::tempdir().unwrap();
    let types_path = tmp.path().join("Types.sol");
    let consumer_path = tmp.path().join("Consumer.sol");

    let types_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

struct Coord {
    uint256 x;
    uint256 y;
}
"#;
    let consumer_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Coord} from "./Types.sol";

contract Map {
    Coord public origin;

    function setOrigin(Coord memory c) external {
        origin = c;
    }
}
"#;
    std::fs::write(&types_path, types_source).unwrap();
    std::fs::write(&consumer_path, consumer_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&types_path, types_source, &mut parser);
    st.resolve_file_references(&types_path, &mut parser);
    st.index_file(&consumer_path, consumer_source, &mut parser);
    st.resolve_file_references(&consumer_path, &mut parser);

    // Find references to Coord from the consumer file (click on usage in consumer)
    // Named imports create a local ImportAlias — searching from the consumer
    // side finds the alias declaration + local usages.
    let pos = first_position(consumer_source, "Coord");
    let refs = find_references(
        &st,
        &consumer_path,
        consumer_source,
        pos,
        true,
        &LineIndex::new(consumer_source),
    );
    // 1 import alias decl + 2 usages (state var type + param type) = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for Coord via named import in consumer (1 alias decl + 2 usages), got {:?}",
        refs
    );
}

#[test]
fn find_references_to_mapping_value_type_struct() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct Record {
        address owner;
        uint256 balance;
    }

    mapping(address => Record) public records;

    function getRecord(address user) external view returns (Record memory) {
        return records[user];
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Record");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 1 mapping value type + 1 return type = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for Record struct (decl + mapping type + return type), got {:?}",
        refs
    );
}

#[test]
fn find_references_exclude_declaration_for_state_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Counter {
    uint256 public count;

    function increment() public {
        count += 1;
    }

    function decrement() public {
        count -= 1;
    }

    function reset() public {
        count = 0;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "count");
    let refs_with = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    let refs_without = find_references(&st, &path, source, pos, false, &LineIndex::new(source));
    // With declaration: 1 decl + 3 usages = 4
    assert_eq!(
        refs_with.len(),
        4,
        "Expected 4 references with declaration, got {:?}",
        refs_with
    );
    // Without declaration: 3 usages only
    assert_eq!(
        refs_without.len(),
        3,
        "Expected 3 references without declaration, got {:?}",
        refs_without
    );
    assert_eq!(
        refs_with.len() - refs_without.len(),
        1,
        "Difference should be exactly 1 (the declaration)"
    );
}

#[test]
fn find_references_exclude_declaration_for_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Math {
    function double(uint256 x) internal pure returns (uint256) {
        return x * 2;
    }

    function quadruple(uint256 x) public pure returns (uint256) {
        return double(double(x));
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "double");
    let refs_with = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    let refs_without = find_references(&st, &path, source, pos, false, &LineIndex::new(source));
    // With declaration: 1 decl + 2 calls = 3
    assert_eq!(
        refs_with.len(),
        3,
        "Expected 3 references with declaration, got {:?}",
        refs_with
    );
    // Without declaration: 2 calls only
    assert_eq!(
        refs_without.len(),
        2,
        "Expected 2 references without declaration, got {:?}",
        refs_without
    );
}

#[test]
fn find_references_to_interface_from_inheritance_clauses() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function totalSupply() external view returns (uint256);
}

contract TokenA is IERC20 {
    function totalSupply() external pure returns (uint256) {
        return 100;
    }
}

contract TokenB is IERC20 {
    function totalSupply() external pure returns (uint256) {
        return 200;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "IERC20");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 2 inheritance clauses = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for IERC20 (1 decl + 2 inheritance), got {:?}",
        refs
    );
}

#[test]
fn find_references_to_immutable_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    address public immutable owner;

    constructor(address _owner) {
        owner = _owner;
    }

    function getOwner() public view returns (address) {
        return owner;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "owner");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 1 constructor assignment + 1 getter usage = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for immutable owner (1 decl + 1 constructor + 1 getter), got {:?}",
        refs
    );
}

#[test]
fn find_references_to_event_from_emit_statements() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ledger {
    event Transfer(address indexed from, address indexed to, uint256 amount);

    function send(address to, uint256 amount) external {
        emit Transfer(msg.sender, to, amount);
    }

    function batchSend(address to, uint256 a1, uint256 a2) external {
        emit Transfer(msg.sender, to, a1);
        emit Transfer(msg.sender, to, a2);
    }
}
"#;
    let (st, path) = setup(source);

    let pos = first_position(source, "Transfer");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 3 emit usages = 4
    assert_eq!(
        refs.len(),
        4,
        "Expected 4 references for Transfer event (1 decl + 3 emits), got {:?}",
        refs
    );
}
