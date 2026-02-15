use std::path::PathBuf;

use solidity_language_server::goto::goto_definition;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
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

    let loc = goto_definition(&st, &path, source, Position::new(line, col));
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

    let loc = goto_definition(&st, &path, source, Position::new(line, col));
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

    let loc = goto_definition(&st, &path, source, Position::new(line, col));
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

    let loc = goto_definition(&st, &path, source, Position::new(line, col));
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

    let loc = goto_definition(&st, &path, source, Position::new(line, col));
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

    let loc = goto_definition(&st, &path, source, Position::new(line, col));
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

    let loc = goto_definition(&st, &path, source, Position::new(line, col));
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

    let loc = goto_definition(&st, &path, source, Position::new(line, col));
    assert!(
        loc.is_some(),
        "Should resolve Active to its enum value declaration"
    );
    let loc = loc.unwrap();
    // enum Status { Active, ... } is on line 4
    assert_eq!(loc.range.start.line, 4);
}
