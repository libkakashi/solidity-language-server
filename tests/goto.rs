use std::path::PathBuf;

use solidity_language_server::goto::goto_definition;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
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
fn goto_state_variable_from_function_body() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Counter {
    uint256 public count;

    function increment() public {
        count += 1;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `count` in `count += 1;`
    let count_usage = source.find("count += 1").unwrap();
    let line = source[..count_usage].matches('\n').count() as u32;
    let col = (count_usage - source[..count_usage].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve count to its declaration");
    let loc = loc.unwrap();
    // Declaration of `count` is on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_function_parameter() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Math {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `a` in `return a + b;`
    let return_pos = source.find("return a + b").unwrap();
    let a_pos = return_pos + "return ".len();
    let line = source[..a_pos].matches('\n').count() as u32;
    let col = (a_pos - source[..a_pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve parameter 'a'");
    let loc = loc.unwrap();
    // Parameter `a` is declared on the function signature line (line 4)
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_local_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Test {
    function foo() public pure returns (uint256) {
        uint256 result = 42;
        return result;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `result` in `return result;`
    let return_pos = source.find("return result;").unwrap();
    let result_pos = return_pos + "return ".len();
    let line = source[..result_pos].matches('\n').count() as u32;
    let col = (result_pos - source[..result_pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve local variable 'result'");
    let loc = loc.unwrap();
    // `result` declared on line 5 (`uint256 result = 42;`)
    assert_eq!(loc.range.start.line, 5);
}

#[test]
fn goto_struct_type_reference() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct Entry {
        address addr;
        uint256 value;
    }

    Entry[] public entries;

    function addEntry(address addr, uint256 value) public {
        entries.push(Entry(addr, value));
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `Entry` in `Entry[] public entries;`
    let entry_usage = source.find("Entry[] public").unwrap();
    let line = source[..entry_usage].matches('\n').count() as u32;
    let col = (entry_usage - source[..entry_usage].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve struct Entry");
    let loc = loc.unwrap();
    // `Entry` struct is declared on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_on_declaration_returns_itself() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public bar;
}
"#;
    let (st, path) = setup(source);

    // Position on `bar` in its declaration
    let bar_pos = source.find("bar").unwrap();
    let line = source[..bar_pos].matches('\n').count() as u32;
    let col = (bar_pos - source[..bar_pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve even on the declaration itself"
    );
}

#[test]
fn goto_qualified_event_definition() {
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

    // Position on "FeeUpdated" in "IFees.FeeUpdated"
    let pos = source.rfind("FeeUpdated").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve FeeUpdated to its declaration in IFees"
    );
    let loc = loc.unwrap();
    // FeeUpdated is declared on line 4 (inside interface IFees)
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_struct_field_from_member_access() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }

    function bar() public {
        Point memory p;
        uint256 val = p.x;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on "x" in "p.x"
    let pos = source.find("p.x").unwrap() + 2; // skip "p."
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve x to struct field declaration"
    );
    let loc = loc.unwrap();
    // "uint256 x;" is on line 5
    assert_eq!(loc.range.start.line, 5);
}

#[test]
fn goto_enum_value_from_qualified_access() {
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

    // Position on "Active" in "Status.Active"
    let pos = source.find("Status.Active").unwrap() + "Status.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve Active to its enum value declaration"
    );
    let loc = loc.unwrap();
    // enum Status { Active, ... } is on line 4
    assert_eq!(loc.range.start.line, 4);
}

// ---------------------------------------------------------------------------
// Edge case tests
// ---------------------------------------------------------------------------

#[test]
fn goto_function_call_to_definition() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Calculator {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }

    function compute() public pure returns (uint256) {
        return add(1, 2);
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `add` in `add(1, 2)`
    let pos = source.find("add(1, 2)").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve function call to definition");
    let loc = loc.unwrap();
    // function add(...) is declared on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_event_emit_to_definition() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    event Transfer(address indexed from, address indexed to, uint256 amount);

    function send(address to, uint256 amount) public {
        emit Transfer(msg.sender, to, amount);
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `Transfer` in `emit Transfer(...)`
    let pos = source.find("emit Transfer").unwrap() + "emit ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve event emit to event definition"
    );
    let loc = loc.unwrap();
    // event Transfer is declared on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_error_revert_to_definition() {
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

    // Position on `InsufficientBalance` in `revert InsufficientBalance(...)`
    let pos = source.find("revert InsufficientBalance").unwrap() + "revert ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve error revert to error definition"
    );
    let loc = loc.unwrap();
    // error InsufficientBalance is declared on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_modifier_usage_to_definition() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    modifier onlyOwner() {
        _;
    }

    function withdraw() public onlyOwner {
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `onlyOwner` in `function withdraw() public onlyOwner`
    let pos = source.find("public onlyOwner").unwrap() + "public ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve modifier usage to definition");
    let loc = loc.unwrap();
    // modifier onlyOwner is declared on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_inherited_function_call_to_base() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseFunc() public pure returns (uint256) {
        return 42;
    }
}

contract Child is Base {
    function test() public pure returns (uint256) {
        return baseFunc();
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `baseFunc` in `return baseFunc();`
    let pos = source.find("baseFunc();").unwrap();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve inherited function call to base contract"
    );
    let loc = loc.unwrap();
    // function baseFunc() in Base is on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_type_in_mapping_to_struct() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct User {
        address wallet;
        uint256 balance;
    }

    mapping(address => User) public users;
}
"#;
    let (st, path) = setup(source);

    // Position on `User` in `mapping(address => User)`
    let pos = source.find("=> User)").unwrap() + "=> ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve type in mapping to struct definition"
    );
    let loc = loc.unwrap();
    // struct User is declared on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_constructor_call_new_to_contract() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public supply;
}

contract Factory {
    function create() public returns (Token) {
        Token t = new Token();
        return t;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `Token` in `new Token()` (the type reference after `new`)
    let pos = source.find("new Token()").unwrap() + "new ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve constructor call to contract definition"
    );
    let loc = loc.unwrap();
    // contract Token is declared on line 3
    assert_eq!(loc.range.start.line, 3);
}

// ========== OVERLOAD RESOLUTION TESTS ==========

#[test]
fn goto_overloaded_function_one_arg() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function process(uint256 x) public pure returns (uint256) {
        return x;
    }

    function process(uint256 x, uint256 y) public pure returns (uint256) {
        return x + y;
    }

    function test() public pure {
        process(1);
    }
}
"#;
    let (st, path) = setup(source);
    let call_pos = source.find("process(1);").unwrap();
    let line = source[..call_pos].matches('\n').count() as u32;
    let col = (call_pos - source[..call_pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve overloaded function call");
    let loc = loc.unwrap();
    // The 1-arg overload is on line 4
    assert_eq!(
        loc.range.start.line, 4,
        "Should jump to 1-arg overload (line 4), got line {}",
        loc.range.start.line
    );
}

#[test]
fn goto_overloaded_function_two_args() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function process(uint256 x) public pure returns (uint256) {
        return x;
    }

    function process(uint256 x, uint256 y) public pure returns (uint256) {
        return x + y;
    }

    function test() public pure {
        process(1, 2);
    }
}
"#;
    let (st, path) = setup(source);
    let call_pos = source.find("process(1, 2);").unwrap();
    let line = source[..call_pos].matches('\n').count() as u32;
    let col = (call_pos - source[..call_pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve 2-arg overloaded function call"
    );
    let loc = loc.unwrap();
    // The 2-arg overload is on line 8
    assert_eq!(
        loc.range.start.line, 8,
        "Should jump to 2-arg overload (line 8), got line {}",
        loc.range.start.line
    );
}

#[test]
fn goto_overloaded_function_three_overloads() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function calc() public pure returns (uint256) {
        return 0;
    }

    function calc(uint256 x) public pure returns (uint256) {
        return x;
    }

    function calc(uint256 x, uint256 y) public pure returns (uint256) {
        return x + y;
    }

    function test() public pure {
        calc(10, 20);
    }
}
"#;
    let (st, path) = setup(source);
    let call_pos = source.find("calc(10, 20);").unwrap();
    let line = source[..call_pos].matches('\n').count() as u32;
    let col = (call_pos - source[..call_pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve 2-arg overloaded calc call");
    let loc = loc.unwrap();
    // The 2-arg overload is on line 12
    assert_eq!(
        loc.range.start.line, 12,
        "Should jump to 2-arg calc overload (line 12), got line {}",
        loc.range.start.line
    );
}

#[test]
fn goto_for_loop_variable_to_declaration() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Iter {
    function loop_test() public pure returns (uint256) {
        uint256 sum = 0;
        for (uint256 idx = 0; idx < 10; idx++) {
            sum += idx;
        }
        return sum;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `idx` in `sum += idx;`
    let pos = source.find("sum += idx").unwrap() + "sum += ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve for-loop variable to its declaration"
    );
    let loc = loc.unwrap();
    // `uint256 idx = 0` is on line 6
    assert_eq!(loc.range.start.line, 6);
}

#[test]
fn goto_on_whitespace_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Empty {
    uint256 public x;
}
"#;
    let (st, path) = setup(source);

    // Position on an empty line (line 2 is blank)
    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(2, 0),
        &LineIndex::new(source),
    );
    assert!(loc.is_none(), "Goto on whitespace should return None");
}

#[test]
fn goto_library_qualified_function_call() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

contract Calculator {
    function compute() public pure returns (uint256) {
        return MathLib.add(1, 2);
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `add` in `MathLib.add(1, 2)`
    let pos = source.find("MathLib.add(1").unwrap() + "MathLib.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve library qualified function call"
    );
    let loc = loc.unwrap();
    // function add in MathLib is on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_base_contract_name_in_inheritance() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    uint256 public x;
}

contract Child is Base {
    function test() public view returns (uint256) {
        return x;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `Base` in `contract Child is Base`
    let pos = source.find("is Base").unwrap() + "is ".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve base contract name in inheritance"
    );
    let loc = loc.unwrap();
    // contract Base is declared on line 3
    assert_eq!(loc.range.start.line, 3);
}

#[test]
fn goto_return_type_to_struct() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct Info {
        address owner;
        uint256 value;
    }

    function getInfo() public pure returns (Info memory) {
        Info memory info;
        return info;
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `Info` in `returns (Info memory)`
    let pos = source.find("returns (Info").unwrap() + "returns (".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve return type to struct definition"
    );
    let loc = loc.unwrap();
    // struct Info is declared on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_cross_file_imported_struct_field() {
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

    // Position on `buyer` in `o.buyer`
    let pos = main_source.find("o.buyer").unwrap() + "o.".len();
    let line = main_source[..pos].matches('\n').count() as u32;
    let col = (pos - main_source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &main_path,
        main_source,
        Position::new(line, col),
        &LineIndex::new(main_source),
    );
    assert!(
        loc.is_some(),
        "Should resolve cross-file struct field access"
    );
    let loc = loc.unwrap();
    // `address buyer;` is on line 5 in Types.sol
    assert_eq!(loc.range.start.line, 5);
}

#[test]
fn goto_enum_value_to_enum_definition() {
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

    // Position on `Active` in `Status.Active`
    let pos = source.find("return Status.Active").unwrap() + "return Status.".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve enum value to enum definition"
    );
    let loc = loc.unwrap();
    // enum Status { Active, ... } is on line 4
    assert_eq!(loc.range.start.line, 4);
}

#[test]
fn goto_parameter_type_to_contract() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public supply;
}

contract Exchange {
    function deposit(Token token) public {
    }
}
"#;
    let (st, path) = setup(source);

    // Position on `Token` in `function deposit(Token token)`
    let pos = source.find("deposit(Token").unwrap() + "deposit(".len();
    let line = source[..pos].matches('\n').count() as u32;
    let col = (pos - source[..pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve parameter type to contract definition"
    );
    let loc = loc.unwrap();
    // contract Token is declared on line 3
    assert_eq!(loc.range.start.line, 3);
}
