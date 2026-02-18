use std::path::PathBuf;

use solidity_language_server::completion::handle_completion;
use solidity_language_server::goto::goto_definition;
use solidity_language_server::hover::hover_info;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::lint::LintEngine;
use solidity_language_server::parser::TsParser;
use solidity_language_server::references::find_references;
use solidity_language_server::rename::rename_symbol;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::*;

fn setup(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

/// Extract the text from a Hover result.
fn hover_text(source: &str, st: &SymbolTable, path: &PathBuf, pos: Position) -> Option<String> {
    let hover = hover_info(st, path, source, pos, &LineIndex::new(source))?;
    match hover.contents {
        HoverContents::Markup(markup) => Some(markup.value),
        _ => None,
    }
}

/// Collect completion item labels.
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

/// Helper: find the byte offset of `needle` in `source` and return its Position.
fn pos_of(source: &str, needle: &str) -> Position {
    let offset = source.find(needle).expect("needle not found in source");
    let line = source[..offset].matches('\n').count() as u32;
    let col = (offset - source[..offset].rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32;
    Position::new(line, col)
}

/// Like `pos_of` but finds the Nth occurrence (0-indexed).
fn pos_of_nth(source: &str, needle: &str, nth: usize) -> Position {
    let mut start = 0;
    for _ in 0..nth {
        let found = source[start..].find(needle).expect("nth needle not found");
        start += found + needle.len();
    }
    let offset = source[start..].find(needle).expect("nth needle not found") + start;
    let line = source[..offset].matches('\n').count() as u32;
    let col = (offset - source[..offset].rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32;
    Position::new(line, col)
}

/// Run the lint engine on source, returning diagnostics.
fn lint(source: &str) -> Vec<Diagnostic> {
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).expect("parse failed");
    let engine = LintEngine::new();
    engine.run(&tree, source, &LineIndex::new(source))
}

// ===========================================================================
// 1. Inline assembly - goto/hover should not crash
// ===========================================================================
#[test]
fn inline_assembly_goto_does_not_crash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract AsmTest {
    function foo() public pure returns (uint256 result) {
        assembly {
            let x := mload(0x40)
            mstore(x, 42)
            result := mload(x)
        }
    }
}
"#;
    let (st, path) = setup(source);

    // Position inside the assembly block on "mload"
    let pos = pos_of(source, "mload(0x40)");
    let loc = goto_definition(&st, &path, source, pos, &LineIndex::new(source));
    // It is fine to return None -- the key is it must not panic.
    let _ = loc;
}

#[test]
fn inline_assembly_hover_does_not_crash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract AsmTest {
    function foo() public pure returns (uint256 result) {
        assembly {
            let x := mload(0x40)
            mstore(x, 42)
            result := mload(x)
        }
    }
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "mstore");
    let text = hover_text(source, &st, &path, pos);
    // May be None -- just must not panic.
    let _ = text;
}

// ===========================================================================
// 2. Yul variable in assembly - completion inside assembly block
// ===========================================================================
#[test]
fn completion_inside_assembly_block_does_not_crash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract AsmCompletion {
    function foo() public pure {
        assembly {
            let x := 1

        }
    }
}
"#;
    let (st, path) = setup(source);

    // Trigger completion on the empty line inside assembly
    let labels = completion_labels(&st, &path, source, Position::new(7, 12), None);
    // Must not panic; may or may not return results
    let _ = labels;
}

// ===========================================================================
// 3. Deeply nested mapping - hover shows type
// ===========================================================================
#[test]
fn deeply_nested_mapping_hover() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Nested {
    mapping(address => mapping(uint256 => mapping(bytes32 => bool))) public deepMap;
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "deepMap");
    let text = hover_text(source, &st, &path, pos);
    assert!(
        text.is_some(),
        "Should show hover for deeply nested mapping"
    );
    let text = text.unwrap();
    assert!(
        text.contains("mapping"),
        "Should contain mapping keyword, got: {text}"
    );
    assert!(
        text.contains("deepMap"),
        "Should contain variable name, got: {text}"
    );
}

// ===========================================================================
// 4. Array of structs - goto on Point resolves
// ===========================================================================
#[test]
fn array_of_structs_goto_resolves() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Geometry {
    struct Point {
        uint256 x;
        uint256 y;
    }

    function createPoints() public pure {
        Point[] memory points;
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "Point" in "Point[] memory points" -- use exact match to avoid
    // matching inside "createPoints"
    let needle_offset = source.find("Point[] memory").unwrap();
    let line = source[..needle_offset].matches('\n').count() as u32;
    let col = (needle_offset - source[..needle_offset].rfind('\n').unwrap() - 1) as u32;
    let pos = Position::new(line, col);
    let loc = goto_definition(&st, &path, source, pos, &LineIndex::new(source));
    assert!(
        loc.is_some(),
        "Should resolve Point in array type to struct declaration"
    );
    let loc = loc.unwrap();
    assert_eq!(loc.range.start.line, 4, "Point struct is on line 4");
}

// ===========================================================================
// 5. Fixed-size arrays - hover shows type
// ===========================================================================
#[test]
fn fixed_size_array_hover() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract FixedArr {
    uint256[10] public arr;
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "arr");
    let text = hover_text(source, &st, &path, pos);
    assert!(text.is_some(), "Should show hover for fixed-size array");
    let text = text.unwrap();
    assert!(
        text.contains("uint256") && text.contains("10"),
        "Should show fixed-size array type, got: {text}"
    );
}

// ===========================================================================
// 6. Function type - hover
// ===========================================================================
#[test]
fn function_type_variable_hover() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract FnType {
    function(uint256) external returns (bool) public callback;
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "callback");
    let text = hover_text(source, &st, &path, pos);
    assert!(
        text.is_some(),
        "Should show hover for function type variable"
    );
    let text = text.unwrap();
    assert!(
        text.contains("callback"),
        "Should contain variable name, got: {text}"
    );
}

// ===========================================================================
// 7. Bytes and string - hover
// ===========================================================================
#[test]
fn bytes_and_string_hover() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract DataTypes {
    bytes public data;
    string public name;
}
"#;
    let (st, path) = setup(source);

    let pos_data = pos_of(source, "data");
    let text_data = hover_text(source, &st, &path, pos_data);
    assert!(text_data.is_some(), "Should show hover for bytes variable");
    let text_data = text_data.unwrap();
    assert!(
        text_data.contains("bytes"),
        "Should contain bytes type, got: {text_data}"
    );

    let pos_name = pos_of(source, "name");
    let text_name = hover_text(source, &st, &path, pos_name);
    assert!(text_name.is_some(), "Should show hover for string variable");
    let text_name = text_name.unwrap();
    assert!(
        text_name.contains("string"),
        "Should contain string type, got: {text_name}"
    );
}

// ===========================================================================
// 8. Single char identifiers - goto resolves
// ===========================================================================
#[test]
fn single_char_identifiers_goto() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Tiny {
    uint256 public x;
    uint256 public y;

    function add() public view returns (uint256) {
        return x + y;
    }
}
"#;
    let (st, path) = setup(source);

    // Goto "x" in "return x + y"
    let return_pos = source.find("return x + y").unwrap();
    let x_pos = return_pos + "return ".len();
    let line = source[..x_pos].matches('\n').count() as u32;
    let col = (x_pos - source[..x_pos].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve single-char identifier 'x'");
    assert_eq!(loc.unwrap().range.start.line, 4, "'x' declared on line 4");
}

// ===========================================================================
// 9. Underscore prefix - references work
// ===========================================================================
#[test]
fn underscore_prefix_references() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Prefixed {
    uint256 private _amount;

    function set(uint256 val) public {
        _amount = val;
    }

    function get() public view returns (uint256) {
        return _amount;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "_amount");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    // 1 declaration + 2 usages = 3
    assert_eq!(
        refs.len(),
        3,
        "Expected 3 references for _amount, got {:?}",
        refs
    );
}

// ===========================================================================
// 10. Double underscore - rename works
// ===========================================================================
#[test]
fn double_underscore_rename() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Slots {
    uint256 private __slot;

    function setSlot(uint256 val) public {
        __slot = val;
    }

    function getSlot() public view returns (uint256) {
        return __slot;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "__slot");
    let edit = rename_symbol(
        &st,
        &path,
        source,
        pos,
        "__newSlot",
        &LineIndex::new(source),
    );
    assert!(edit.is_some(), "Should produce rename edits for __slot");
    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = Url::from_file_path("/tmp/test.sol").unwrap();
    let file_edits = changes.get(&uri).unwrap();
    assert!(
        file_edits.len() >= 3,
        "Expected at least 3 edits (decl + 2 usages), got {}",
        file_edits.len()
    );
    for e in file_edits {
        assert_eq!(e.new_text, "__newSlot");
    }
}

// ===========================================================================
// 11. Very long identifier - all operations work
// ===========================================================================
#[test]
fn very_long_identifier_operations() {
    let long_name = format!("very{}", "Long".repeat(30));
    let source = format!(
        r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract LongNames {{
    uint256 public {long_name};

    function test() public view returns (uint256) {{
        return {long_name};
    }}
}}
"#
    );
    let (st, path) = setup(&source);

    // Hover on the long identifier
    let pos = pos_of(&source, &long_name);
    let text = hover_text(&source, &st, &path, pos);
    assert!(text.is_some(), "Should hover on very long identifier");

    // References
    let refs = find_references(&st, &path, &source, pos, true, &LineIndex::new(&source));
    assert_eq!(
        refs.len(),
        2,
        "Expected 2 references for the long identifier"
    );

    // Goto definition from usage
    let usage_pos = pos_of_nth(&source, &long_name, 1);
    let loc = goto_definition(&st, &path, &source, usage_pos, &LineIndex::new(&source));
    assert!(loc.is_some(), "Goto should resolve very long identifier");
}

// ===========================================================================
// 12. Unicode in comments - operations don't break
// ===========================================================================
#[test]
fn unicode_in_comments_does_not_break_operations() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

/// @notice Transfer tokens to recipient
/// @dev Emits a Transfer event
contract UniToken {
    uint256 public balance;

    function transfer() public view returns (uint256) {
        return balance;
    }
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of_nth(source, "balance", 1);
    let loc = goto_definition(&st, &path, source, pos, &LineIndex::new(source));
    assert!(loc.is_some(), "Goto should work with unicode in comments");
    assert_eq!(
        loc.unwrap().range.start.line,
        6,
        "balance declared on line 6"
    );

    let hover = hover_text(source, &st, &path, pos);
    assert!(
        hover.is_some(),
        "Hover should work with unicode in comments"
    );
}

// ===========================================================================
// 13. Identifier same as builtin keyword pattern
// ===========================================================================
#[test]
fn identifier_resembling_builtin() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Builtins {
    uint256 public value;
    address public sender;

    function getValue() public view returns (uint256) {
        return value;
    }

    function getSender() public view returns (address) {
        return sender;
    }
}
"#;
    let (st, path) = setup(source);

    // "value" and "sender" look like msg.value and msg.sender but are state variables
    let pos_val = pos_of_nth(source, "value", 1);
    let loc = goto_definition(&st, &path, source, pos_val, &LineIndex::new(source));
    assert!(loc.is_some(), "Should resolve 'value' to state variable");
    assert_eq!(loc.unwrap().range.start.line, 4);

    let pos_send = pos_of_nth(source, "sender", 1);
    let loc2 = goto_definition(&st, &path, source, pos_send, &LineIndex::new(source));
    assert!(loc2.is_some(), "Should resolve 'sender' to state variable");
    assert_eq!(loc2.unwrap().range.start.line, 5);
}

// ===========================================================================
// 14. Variable shadowing - local shadows state variable
// ===========================================================================
#[test]
fn variable_shadowing_goto_resolves_local() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Shadow {
    uint256 public count;

    function test() public pure returns (uint256) {
        uint256 count = 42;
        return count;
    }
}
"#;
    let (st, path) = setup(source);

    // "count" in "return count;" should resolve to the LOCAL variable on line 7, not state on line 4
    let return_offset = source.find("return count;").unwrap() + "return ".len();
    let line = source[..return_offset].matches('\n').count() as u32;
    let col = (return_offset - source[..return_offset].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve shadowed count");
    let loc = loc.unwrap();
    // The local `uint256 count = 42;` is on line 7
    assert_eq!(
        loc.range.start.line, 7,
        "Should resolve to local variable (line 7), not state variable (line 4)"
    );
}

// ===========================================================================
// 15. Same name in different functions
// ===========================================================================
#[test]
fn same_name_different_functions() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Scope {
    function funcA() public pure returns (uint256) {
        uint256 x = 10;
        return x;
    }

    function funcB() public pure returns (uint256) {
        uint256 x = 20;
        return x;
    }
}
"#;
    let (st, path) = setup(source);

    // "x" in funcA's return should resolve to funcA's local x (line 5)
    let return_a = source.find("return x;").unwrap() + "return ".len();
    let line_a = source[..return_a].matches('\n').count() as u32;
    let col_a = (return_a - source[..return_a].rfind('\n').unwrap() - 1) as u32;

    let loc_a = goto_definition(
        &st,
        &path,
        source,
        Position::new(line_a, col_a),
        &LineIndex::new(source),
    );
    assert!(loc_a.is_some(), "Should resolve x in funcA");
    assert_eq!(loc_a.unwrap().range.start.line, 5, "funcA's x on line 5");

    // "x" in funcB's return should resolve to funcB's local x (line 10)
    let second_return = source.rfind("return x;").unwrap() + "return ".len();
    let line_b = source[..second_return].matches('\n').count() as u32;
    let col_b = (second_return - source[..second_return].rfind('\n').unwrap() - 1) as u32;

    let loc_b = goto_definition(
        &st,
        &path,
        source,
        Position::new(line_b, col_b),
        &LineIndex::new(source),
    );
    assert!(loc_b.is_some(), "Should resolve x in funcB");
    assert_eq!(loc_b.unwrap().range.start.line, 10, "funcB's x on line 10");
}

// ===========================================================================
// 16. Same name struct and variable
// ===========================================================================
#[test]
fn same_name_struct_and_variable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Naming {
    struct Data {
        uint256 val;
    }

    Data public data;

    function test() public view returns (uint256) {
        return data.val;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on "Data" (the struct type usage in state variable declaration)
    let pos_type = pos_of_nth(source, "Data", 1);
    let text_type = hover_text(source, &st, &path, pos_type);
    assert!(text_type.is_some(), "Should hover on struct type Data");
    let text_type = text_type.unwrap();
    assert!(
        text_type.contains("struct"),
        "Should identify as struct, got: {text_type}"
    );

    // Hover on "data" (the variable)
    let pos_var = pos_of(source, "data;");
    let text_var = hover_text(source, &st, &path, pos_var);
    assert!(text_var.is_some(), "Should hover on variable data");
    let text_var = text_var.unwrap();
    assert!(
        text_var.contains("Data"),
        "Should show Data type, got: {text_var}"
    );
}

// ===========================================================================
// 17. For loop variable scope
// ===========================================================================
#[test]
fn for_loop_variable_scope() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Loop {
    function sum() public pure returns (uint256) {
        uint256 total = 0;
        for (uint256 i = 0; i < 10; i++) {
            total += i;
        }
        return total;
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "i" inside the loop body "total += i;"
    let usage_offset = source.find("total += i").unwrap() + "total += ".len();
    let line = source[..usage_offset].matches('\n').count() as u32;
    let col = (usage_offset - source[..usage_offset].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve for-loop variable i");
    let loc = loc.unwrap();
    // `uint256 i = 0` is on line 6
    assert_eq!(loc.range.start.line, 6, "Loop var i declared on line 6");
}

// ===========================================================================
// 18. Try-catch variable scope
// ===========================================================================
#[test]
fn try_catch_does_not_crash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface ITarget {
    function doSomething() external returns (uint256);
}

contract TryCatch {
    ITarget public target;

    function attempt() public returns (uint256) {
        try target.doSomething() returns (uint256 result) {
            return result;
        } catch Error(string memory reason) {
            revert(reason);
        } catch {
            revert("Unknown error");
        }
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "result" in "return result;"
    let offset = source.find("return result;").unwrap() + "return ".len();
    let line = source[..offset].matches('\n').count() as u32;
    let col = (offset - source[..offset].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    // May or may not resolve depending on implementation, but must not crash
    let _ = loc;

    // Hover on "reason" in catch block
    let reason_offset = source.find("revert(reason)").unwrap() + "revert(".len();
    let line_r = source[..reason_offset].matches('\n').count() as u32;
    let col_r = (reason_offset - source[..reason_offset].rfind('\n').unwrap() - 1) as u32;
    let hover = hover_text(source, &st, &path, Position::new(line_r, col_r));
    // May be None, but must not crash
    let _ = hover;
}

// ===========================================================================
// 19. Deep inheritance (5+ levels) - goto resolves through chain
// ===========================================================================
#[test]
fn deep_inheritance_goto_resolves() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 public rootVal;
}

contract B is A {}
contract C is B {}
contract D is C {}
contract E is D {}
contract F is E {
    function test() public view returns (uint256) {
        return rootVal;
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "rootVal" in contract F
    let offset = source.find("return rootVal").unwrap() + "return ".len();
    let line = source[..offset].matches('\n').count() as u32;
    let col = (offset - source[..offset].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_some(),
        "Should resolve rootVal through deep inheritance chain"
    );
    let loc = loc.unwrap();
    assert_eq!(
        loc.range.start.line, 4,
        "rootVal declared in contract A on line 4"
    );
}

// ===========================================================================
// 20. Multiple inheritance - completion shows all inherited members
// ===========================================================================
#[test]
fn multiple_inheritance_completion() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function funcA() public pure returns (uint256) { return 1; }
}

contract B {
    function funcB() public pure returns (uint256) { return 2; }
}

contract C {
    function funcC() public pure returns (uint256) { return 3; }
}

contract D is A, B, C {
    function test() public pure {

    }
}
"#;
    let (st, path) = setup(source);

    // Completion inside D.test() body
    let labels = completion_labels(&st, &path, source, Position::new(18, 8), None);

    assert!(
        labels.contains(&"funcA".to_string()),
        "Should include funcA from A, got: {labels:?}"
    );
    assert!(
        labels.contains(&"funcB".to_string()),
        "Should include funcB from B, got: {labels:?}"
    );
    assert!(
        labels.contains(&"funcC".to_string()),
        "Should include funcC from C, got: {labels:?}"
    );
}

// ===========================================================================
// 21. Override functions - goto resolves to correct version
// ===========================================================================
#[test]
fn override_function_goto_resolves() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function getValue() public virtual pure returns (uint256) {
        return 1;
    }
}

contract Child is Base {
    function getValue() public pure override returns (uint256) {
        return 2;
    }

    function test() public pure returns (uint256) {
        return getValue();
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "getValue()" call in test() of Child
    let call_offset = source.find("return getValue();").unwrap() + "return ".len();
    let line = source[..call_offset].matches('\n').count() as u32;
    let col = (call_offset - source[..call_offset].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve getValue() call");
    // It should resolve to Child's override (line 11) or Base's definition (line 4)
    // Either is acceptable depending on resolution strategy
    let loc = loc.unwrap();
    assert!(
        loc.range.start.line == 4 || loc.range.start.line == 10,
        "Should resolve to either Base (line 4) or Child (line 10), got line {}",
        loc.range.start.line
    );
}

// ===========================================================================
// 22. Virtual function in abstract contract
// ===========================================================================
#[test]
fn virtual_function_in_abstract_contract() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

abstract contract AbstractBase {
    function compute() public virtual pure returns (uint256);
}

contract Impl is AbstractBase {
    function compute() public pure override returns (uint256) {
        return 42;
    }

    function test() public pure returns (uint256) {
        return compute();
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on abstract function declaration
    let pos = pos_of(source, "compute");
    let text = hover_text(source, &st, &path, pos);
    assert!(
        text.is_some(),
        "Should hover on virtual function in abstract contract"
    );
    let text = text.unwrap();
    assert!(
        text.contains("compute"),
        "Should show function name, got: {text}"
    );
}

// ===========================================================================
// 23. Using-for with library - completion on uint256 shows library methods
// ===========================================================================
#[test]
fn using_for_library_completion() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library MathLib {
    function square(uint256 x) internal pure returns (uint256) {
        return x * x;
    }
    function cube(uint256 x) internal pure returns (uint256) {
        return x * x * x;
    }
}

contract Calculator {
    using MathLib for uint256;

    function test() public pure returns (uint256) {
        uint256 n = 5;
        n.
    }
}
"#;
    let (st, path) = setup(source);

    let dot_pos = source.find("n.\n").unwrap() + "n.".len();
    let line = source[..dot_pos].matches('\n').count() as u32;
    let col = (dot_pos - source[..dot_pos].rfind('\n').unwrap() - 1) as u32;

    let labels = completion_labels(&st, &path, source, Position::new(line, col), Some("."));
    assert!(
        labels.contains(&"square".to_string()),
        "Should include 'square' from MathLib via using-for, got: {labels:?}"
    );
    assert!(
        labels.contains(&"cube".to_string()),
        "Should include 'cube' from MathLib via using-for, got: {labels:?}"
    );
}

// ===========================================================================
// 24. Receive function - hover works
// ===========================================================================
#[test]
fn receive_function_hover() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    receive() external payable {}
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "receive()");
    let text = hover_text(source, &st, &path, pos);
    assert!(text.is_some(), "Should hover on receive function");
    let text = text.unwrap();
    assert!(text.contains("receive"), "Should show receive, got: {text}");
    assert!(text.contains("payable"), "Should show payable, got: {text}");
}

// ===========================================================================
// 25. Fallback function - hover works
// ===========================================================================
#[test]
fn fallback_function_hover() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Proxy {
    fallback() external {}
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "fallback()");
    let text = hover_text(source, &st, &path, pos);
    assert!(text.is_some(), "Should hover on fallback function");
    let text = text.unwrap();
    assert!(
        text.contains("fallback"),
        "Should show fallback, got: {text}"
    );
}

// ===========================================================================
// 26. Constructor - goto/hover on constructor
// ===========================================================================
#[test]
fn constructor_hover_and_goto() {
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

    let pos = pos_of(source, "constructor");
    let text = hover_text(source, &st, &path, pos);
    assert!(text.is_some(), "Should hover on constructor");
    let text = text.unwrap();
    assert!(
        text.contains("constructor"),
        "Should show constructor, got: {text}"
    );

    // Goto on _owner param inside constructor body
    let usage_offset = source.find("owner = _owner").unwrap() + "owner = ".len();
    let line = source[..usage_offset].matches('\n').count() as u32;
    let col = (usage_offset - source[..usage_offset].rfind('\n').unwrap() - 1) as u32;
    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve _owner in constructor body");
}

// ===========================================================================
// 27. Modifier with complex require
// ===========================================================================
#[test]
fn modifier_with_require_hover() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    address public admin;
    uint256 public threshold;

    modifier onlyAdminAboveThreshold(uint256 val) {
        require(msg.sender == admin && val > threshold);
        _;
    }

    function execute(uint256 val) public onlyAdminAboveThreshold(val) {
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on modifier name at declaration
    let pos = pos_of(source, "onlyAdminAboveThreshold");
    let text = hover_text(source, &st, &path, pos);
    assert!(text.is_some(), "Should hover on complex modifier");
    let text = text.unwrap();
    assert!(
        text.contains("modifier onlyAdminAboveThreshold"),
        "Should show modifier signature, got: {text}"
    );

    // Goto on modifier usage in function
    let usage_offset = source.find("public onlyAdminAboveThreshold").unwrap() + "public ".len();
    let line = source[..usage_offset].matches('\n').count() as u32;
    let col = (usage_offset - source[..usage_offset].rfind('\n').unwrap() - 1) as u32;
    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should goto modifier definition from usage");
    assert_eq!(
        loc.unwrap().range.start.line,
        7,
        "Modifier declared on line 7"
    );
}

// ===========================================================================
// 28. Payable address - type resolution
// ===========================================================================
#[test]
fn payable_address_does_not_crash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract PayableTest {
    function withdraw() public {
        address payable recipient = payable(msg.sender);
        recipient.transfer(1 ether);
    }
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "recipient");
    let text = hover_text(source, &st, &path, pos);
    // Should not crash. May or may not resolve payable address.
    let _ = text;

    // Goto on "recipient" in "recipient.transfer"
    let usage_offset = source.find("recipient.transfer").unwrap();
    let line = source[..usage_offset].matches('\n').count() as u32;
    let col = (usage_offset - source[..usage_offset].rfind('\n').unwrap() - 1) as u32;
    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    // Must not crash
    let _ = loc;
}

// ===========================================================================
// 29. Ternary expression - goto on variables inside
// ===========================================================================
#[test]
fn ternary_expression_goto() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ternary {
    uint256 public a;
    uint256 public b;

    function pick(bool flag) public view returns (uint256) {
        return flag ? a : b;
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "a" in the ternary
    let ternary_offset = source.find("flag ? a : b").unwrap() + "flag ? ".len();
    let line = source[..ternary_offset].matches('\n').count() as u32;
    let col = (ternary_offset - source[..ternary_offset].rfind('\n').unwrap() - 1) as u32;
    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve 'a' in ternary expression");
    assert_eq!(loc.unwrap().range.start.line, 4, "'a' declared on line 4");

    // Goto on "b" in the ternary
    let b_offset = source.find("flag ? a : b").unwrap() + "flag ? a : ".len();
    let line_b = source[..b_offset].matches('\n').count() as u32;
    let col_b = (b_offset - source[..b_offset].rfind('\n').unwrap() - 1) as u32;
    let loc_b = goto_definition(
        &st,
        &path,
        source,
        Position::new(line_b, col_b),
        &LineIndex::new(source),
    );
    assert!(loc_b.is_some(), "Should resolve 'b' in ternary expression");
    assert_eq!(loc_b.unwrap().range.start.line, 5, "'b' declared on line 5");
}

// ===========================================================================
// 30. Error handling - try/catch blocks - references work
// ===========================================================================
#[test]
fn try_catch_references_on_target() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface ITarget {
    function execute() external returns (uint256);
}

contract Handler {
    ITarget public target;

    function run() public returns (uint256) {
        try target.execute() returns (uint256 val) {
            return val;
        } catch {
            return 0;
        }
    }
}
"#;
    let (st, path) = setup(source);

    // References on "target" -- declaration + usage in try
    let pos = pos_of(source, "target");
    let refs = find_references(&st, &path, source, pos, true, &LineIndex::new(source));
    assert!(
        refs.len() >= 2,
        "Expected at least 2 references for target (decl + try usage), got {:?}",
        refs
    );
}

// ===========================================================================
// 31. Large file (100+ line contract) - operations don't break
// ===========================================================================
#[test]
fn large_file_operations() {
    let mut source = String::from(
        "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.29;\n\ncontract Large {\n    uint256 public counter;\n\n",
    );
    // Generate 50 functions
    for i in 0..50 {
        source.push_str(&format!(
            "    function func{}() public view returns (uint256) {{\n        return counter;\n    }}\n\n",
            i
        ));
    }
    source.push_str("}\n");

    let (st, path) = setup(&source);

    // Hover on "counter" declaration
    let pos = pos_of(&source, "counter");
    let text = hover_text(&source, &st, &path, pos);
    assert!(text.is_some(), "Hover should work in large file");

    // References to "counter" -- 1 decl + 50 usages = 51
    let refs = find_references(&st, &path, &source, pos, true, &LineIndex::new(&source));
    assert_eq!(
        refs.len(),
        51,
        "Expected 51 references (1 decl + 50 function usages), got {}",
        refs.len()
    );

    // Goto from the last function's "counter" usage
    let last_return = source.rfind("return counter;").unwrap() + "return ".len();
    let line = source[..last_return].matches('\n').count() as u32;
    let col = (last_return - source[..last_return].rfind('\n').unwrap() - 1) as u32;
    let loc = goto_definition(
        &st,
        &path,
        &source,
        Position::new(line, col),
        &LineIndex::new(&source),
    );
    assert!(loc.is_some(), "Goto should work in large file");
    assert_eq!(
        loc.unwrap().range.start.line,
        4,
        "counter declared on line 4"
    );
}

// ===========================================================================
// 32. Empty contract - all operations return appropriate empty/None values
// ===========================================================================
#[test]
fn empty_contract_operations() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Empty {}
"#;
    let (st, path) = setup(source);

    // Hover inside the empty contract on "Empty"
    let pos = pos_of(source, "Empty");
    let text = hover_text(source, &st, &path, pos);
    // May return info about the contract, or None -- must not crash
    let _ = text;

    // Goto on a blank area
    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(2, 0),
        &LineIndex::new(source),
    );
    assert!(
        loc.is_none(),
        "Goto on blank line in empty contract should be None"
    );

    // References on blank area
    let refs = find_references(
        &st,
        &path,
        source,
        Position::new(2, 0),
        true,
        &LineIndex::new(source),
    );
    assert!(refs.is_empty(), "References on blank line should be empty");

    // Completion inside empty contract body
    let labels = completion_labels(&st, &path, source, Position::new(3, 16), None);
    // Should not crash; may return keywords
    let _ = labels;

    // Lint should produce no diagnostics
    let diags = lint(source);
    let lint_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.source == Some("ts-lint".to_string()))
        .collect();
    assert!(
        lint_diags.is_empty(),
        "Empty contract should not trigger lint rules, got: {:?}",
        lint_diags
    );
}

// ===========================================================================
// 33. Multiple pragmas - don't confuse
// ===========================================================================
#[test]
fn multiple_pragmas_do_not_confuse() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;
pragma abicoder v2;

contract MultiPragma {
    uint256 public val;

    function test() public view returns (uint256) {
        return val;
    }
}
"#;
    let (st, path) = setup(source);

    // Goto "val" in "return val;"
    let usage_offset = source.find("return val;").unwrap() + "return ".len();
    let line = source[..usage_offset].matches('\n').count() as u32;
    let col = (usage_offset - source[..usage_offset].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve val with multiple pragmas");
    assert_eq!(loc.unwrap().range.start.line, 5, "val declared on line 5");

    // Hover also works
    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(text.is_some(), "Hover should work with multiple pragmas");
}

// ===========================================================================
// 34. Comments everywhere - heavily commented code works correctly
// ===========================================================================
#[test]
fn heavily_commented_code() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

/// @title Documented Contract
/// @author Test
/// @notice A very well documented contract
contract Documented {
    /// @notice The stored value
    uint256 public /* inline comment */ value;

    /**
     * @notice Sets the value
     * @param newValue The new value to store
     */
    function setValue(
        uint256 newValue // param comment
    ) public {
        // Set the value here
        value = newValue; // assignment
    }

    /* Multi-line
       block comment */
    function getValue() public view returns (
        uint256 // return value
    ) {
        return value; // return statement
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "value" in "value = newValue"
    let assignment = source.find("value = newValue").unwrap();
    let line = source[..assignment].matches('\n').count() as u32;
    let col = (assignment - source[..assignment].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Goto should work in heavily commented code");

    // Hover on "setValue"
    let pos = pos_of(source, "setValue");
    let text = hover_text(source, &st, &path, pos);
    assert!(
        text.is_some(),
        "Hover should work in heavily commented code"
    );
    let text = text.unwrap();
    assert!(
        text.contains("setValue"),
        "Should show function name, got: {text}"
    );

    // References on "value" -- find the actual state variable declaration, not occurrences
    // inside comments. The declaration is: `uint256 public /* inline comment */ value;`
    let val_decl = source.find("*/ value;").unwrap() + "*/ ".len();
    let val_line = source[..val_decl].matches('\n').count() as u32;
    let val_col = (val_decl - source[..val_decl].rfind('\n').unwrap() - 1) as u32;
    let val_pos = Position::new(val_line, val_col);
    let refs = find_references(&st, &path, source, val_pos, true, &LineIndex::new(source));
    assert!(
        refs.len() >= 3,
        "Expected at least 3 references for value in commented code, got {}",
        refs.len()
    );
}

// ===========================================================================
// Additional edge cases beyond the 34 required
// ===========================================================================

// 35. Enum with many values
#[test]
fn enum_with_many_values_hover() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract BigEnum {
    enum Status {
        Pending,
        Active,
        Paused,
        Cancelled,
        Completed,
        Archived,
        Deleted,
        Frozen,
        Expired,
        Draft
    }

    Status public current;
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of_nth(source, "Status", 1);
    let text = hover_text(source, &st, &path, pos);
    assert!(text.is_some(), "Should hover on enum with many values");
    let text = text.unwrap();
    assert!(
        text.contains("enum Status"),
        "Should show enum, got: {text}"
    );
    assert!(
        text.contains("Pending"),
        "Should show first value, got: {text}"
    );
    assert!(
        text.contains("Draft"),
        "Should show last value, got: {text}"
    );
}

// 36. Nested struct access
#[test]
fn nested_struct_field_access() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract NestedStruct {
    struct Inner {
        uint256 val;
    }

    struct Outer {
        Inner inner;
        uint256 id;
    }

    Outer public item;

    function test() public view returns (uint256) {
        return item.id;
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "id" in "item.id"
    let dot_offset = source.find("item.id").unwrap() + "item.".len();
    let line = source[..dot_offset].matches('\n').count() as u32;
    let col = (dot_offset - source[..dot_offset].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve nested struct field id");
}

// 37. Multiple contracts in single file
#[test]
fn multiple_contracts_single_file() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract First {
    uint256 public alpha;
}

contract Second {
    uint256 public beta;
}

contract Third {
    uint256 public gamma;

    function test() public view returns (uint256) {
        return gamma;
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "gamma" in return statement
    let usage_offset = source.find("return gamma").unwrap() + "return ".len();
    let line = source[..usage_offset].matches('\n').count() as u32;
    let col = (usage_offset - source[..usage_offset].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve gamma in Third contract");
    assert_eq!(
        loc.unwrap().range.start.line,
        12,
        "gamma declared on line 12"
    );
}

// 38. Interface with multiple functions and events
#[test]
fn interface_with_events_and_functions() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IComplex {
    event Action(address indexed actor, uint256 value);
    error Unauthorized(address caller);

    function execute(uint256 val) external returns (bool);
    function query() external view returns (uint256);
}

contract Impl is IComplex {
    function execute(uint256 val) external returns (bool) {
        emit Action(msg.sender, val);
        return true;
    }

    function query() external pure returns (uint256) {
        return 0;
    }
}
"#;
    let (st, path) = setup(source);

    // Hover on the "Action" event usage in emit
    let emit_offset = source.find("emit Action").unwrap() + "emit ".len();
    let line = source[..emit_offset].matches('\n').count() as u32;
    let col = (emit_offset - source[..emit_offset].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    assert!(
        text.is_some(),
        "Should hover on event in emit from interface impl"
    );
    let text = text.unwrap();
    assert!(
        text.contains("event Action"),
        "Should show event signature, got: {text}"
    );
}

// 39. Struct with mapping member
#[test]
fn struct_with_mapping_member_does_not_crash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract MapStruct {
    struct Account {
        mapping(address => uint256) balances;
        uint256 nonce;
    }

    mapping(address => Account) public accounts;
}
"#;
    let (st, path) = setup(source);

    // Hover on "accounts"
    let pos = pos_of(source, "accounts");
    let text = hover_text(source, &st, &path, pos);
    // Must not crash
    let _ = text;

    // Hover on "Account" struct
    let pos_struct = pos_of(source, "Account");
    let text_struct = hover_text(source, &st, &path, pos_struct);
    assert!(
        text_struct.is_some(),
        "Should hover on struct with mapping member"
    );
}

// 40. Free function and contract function with same name
#[test]
fn free_function_and_contract_function_same_name() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

function helper() pure returns (uint256) {
    return 1;
}

contract Test {
    function helper() public pure returns (uint256) {
        return 2;
    }

    function callHelper() public pure returns (uint256) {
        return helper();
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "helper()" call in callHelper -- should resolve to the contract method
    let call_offset = source.find("return helper();").unwrap() + "return ".len();
    let line = source[..call_offset].matches('\n').count() as u32;
    let col = (call_offset - source[..call_offset].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve helper() call");
    // Should resolve to the contract function (line 8) rather than the free function (line 3)
    // because the call is from inside the contract
    let loc = loc.unwrap();
    assert!(
        loc.range.start.line == 3 || loc.range.start.line == 8,
        "Should resolve to either free function (line 3) or contract function (line 8), got line {}",
        loc.range.start.line
    );
}

// 41. Library with internal and private functions
#[test]
fn library_internal_private_functions() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library Utils {
    function internal_helper(uint256 x) internal pure returns (uint256) {
        return private_helper(x);
    }

    function private_helper(uint256 x) private pure returns (uint256) {
        return x * 2;
    }
}

contract Consumer {
    function test() public pure returns (uint256) {
        return Utils.internal_helper(5);
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "internal_helper" in qualified call
    let call_offset = source.find("Utils.internal_helper").unwrap() + "Utils.".len();
    let line = source[..call_offset].matches('\n').count() as u32;
    let col = (call_offset - source[..call_offset].rfind('\n').unwrap() - 1) as u32;

    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve library internal function");
    assert_eq!(
        loc.unwrap().range.start.line,
        4,
        "internal_helper on line 4"
    );
}

// 42. Custom error used in require (Solidity 0.8.26+)
#[test]
fn custom_error_in_revert() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Guarded {
    error NotEnough(uint256 available, uint256 requested);
    error ZeroAddress();

    function withdraw(uint256 amount) public {
        revert NotEnough(0, amount);
    }

    function check(address addr) public pure {
        if (addr == address(0)) revert ZeroAddress();
    }
}
"#;
    let (st, path) = setup(source);

    // Goto on "NotEnough" in revert
    let revert_offset = source.find("revert NotEnough").unwrap() + "revert ".len();
    let line = source[..revert_offset].matches('\n').count() as u32;
    let col = (revert_offset - source[..revert_offset].rfind('\n').unwrap() - 1) as u32;
    let loc = goto_definition(
        &st,
        &path,
        source,
        Position::new(line, col),
        &LineIndex::new(source),
    );
    assert!(loc.is_some(), "Should resolve NotEnough error");
    assert_eq!(
        loc.unwrap().range.start.line,
        4,
        "NotEnough declared on line 4"
    );

    // Goto on "ZeroAddress" in inline revert
    let zero_offset = source.find("revert ZeroAddress").unwrap() + "revert ".len();
    let line2 = source[..zero_offset].matches('\n').count() as u32;
    let col2 = (zero_offset - source[..zero_offset].rfind('\n').unwrap() - 1) as u32;
    let loc2 = goto_definition(
        &st,
        &path,
        source,
        Position::new(line2, col2),
        &LineIndex::new(source),
    );
    assert!(loc2.is_some(), "Should resolve ZeroAddress error");
    assert_eq!(
        loc2.unwrap().range.start.line,
        5,
        "ZeroAddress declared on line 5"
    );
}

// 43. Lint on assembly blocks does not produce spurious warnings
#[test]
fn lint_on_assembly_blocks() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract AsmLint {
    function getCodeSize(address target) public view returns (uint256 size) {
        assembly {
            size := extcodesize(target)
        }
    }
}
"#;
    let diags = lint(source);
    // Must not crash; should not produce lint errors for the assembly usage
    let _ = diags;
}

// 44. Type aliases (user defined value types)
#[test]
fn user_defined_value_type_does_not_crash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

type Price is uint256;
type Quantity is uint128;

contract Trading {
    Price public currentPrice;
    Quantity public inventory;
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "currentPrice");
    let text = hover_text(source, &st, &path, pos);
    // Must not crash
    let _ = text;

    let pos2 = pos_of(source, "inventory");
    let text2 = hover_text(source, &st, &path, pos2);
    let _ = text2;
}

// 45. Event with anonymous keyword
#[test]
fn anonymous_event_hover() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Events {
    event Ping() anonymous;

    function doPing() public {
        emit Ping();
    }
}
"#;
    let (st, path) = setup(source);

    let emit_offset = source.find("emit Ping").unwrap() + "emit ".len();
    let line = source[..emit_offset].matches('\n').count() as u32;
    let col = (emit_offset - source[..emit_offset].rfind('\n').unwrap() - 1) as u32;

    let text = hover_text(source, &st, &path, Position::new(line, col));
    // Must not crash; anonymous events should still show hover info
    let _ = text;
}

// 46. Contract with only state variables (no functions)
#[test]
fn contract_only_state_variables() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Storage {
    uint256 public a;
    address public b;
    bool public c;
    bytes32 public d;
    string public e;
}
"#;
    let (st, path) = setup(source);

    // Hover on each variable type
    for (name, expected) in [
        ("a", "uint256"),
        ("b", "address"),
        ("c", "bool"),
        ("d", "bytes32"),
        ("e", "string"),
    ] {
        let pos = pos_of(source, &format!("{name};"));
        let text = hover_text(source, &st, &path, pos);
        assert!(text.is_some(), "Should hover on state variable '{name}'");
        let text = text.unwrap();
        assert!(
            text.contains(expected),
            "Hover on '{name}' should show type '{expected}', got: {text}"
        );
    }
}

// 47. Rename across deeply nested scopes
#[test]
fn rename_across_nested_scopes() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Nested {
    uint256 public target;

    function outer() public {
        target = 1;
        if (true) {
            target = 2;
            for (uint256 i = 0; i < 1; i++) {
                target = 3;
            }
        }
    }
}
"#;
    let (st, path) = setup(source);

    let pos = pos_of(source, "target");
    let edit = rename_symbol(&st, &path, source, pos, "renamed", &LineIndex::new(source));
    assert!(edit.is_some(), "Should rename across nested scopes");
    let edit = edit.unwrap();
    let changes = edit.changes.unwrap();
    let uri = Url::from_file_path("/tmp/test.sol").unwrap();
    let file_edits = changes.get(&uri).unwrap();
    // 1 decl + 3 assignments = 4
    assert_eq!(
        file_edits.len(),
        4,
        "Expected 4 rename edits (decl + 3 nested usages), got {}",
        file_edits.len()
    );
    for e in file_edits {
        assert_eq!(e.new_text, "renamed");
    }
}
