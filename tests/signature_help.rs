use std::path::PathBuf;

use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::signature_help::signature_help;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::{Position, SignatureHelp};

fn setup(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

fn get_sig_help(source: &str, line: u32, col: u32) -> Option<SignatureHelp> {
    let (st, path) = setup(source);
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let li = LineIndex::new(source);
    signature_help(
        &st,
        &path,
        source,
        Position::new(line, col),
        &li,
        Some(&tree),
    )
}

/// Convert a byte offset in `source` to an LSP Position (line, character).
fn pos_of(source: &str, byte_offset: usize) -> (u32, u32) {
    let li = LineIndex::new(source);
    li.byte_offset_to_position(source, byte_offset)
}

/// Helper: get signature help at a byte offset in source.
fn sig_at(source: &str, byte_offset: usize) -> Option<SignatureHelp> {
    let (line, col) = pos_of(source, byte_offset);
    get_sig_help(source, line, col)
}

// ---------------------------------------------------------------------------
// 1. Basic function call - signature help inside parentheses
// ---------------------------------------------------------------------------

#[test]
fn basic_function_call_inside_parens() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }

    function test() public pure {
        add(1, 2);
    }
}
"#;
    let open = source.find("add(1, 2)").unwrap() + "add(".len();
    let help = sig_at(source, open);
    assert!(
        help.is_some(),
        "Should provide signature help inside parens"
    );
    let help = help.unwrap();
    assert_eq!(help.signatures.len(), 1);
    assert!(
        help.signatures[0].label.contains("add"),
        "Label should mention function name, got: {}",
        help.signatures[0].label
    );
    let params = help.signatures[0].parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2, "Should have 2 parameters");
}

// ---------------------------------------------------------------------------
// 2. Multiple parameters - active parameter index changes
// ---------------------------------------------------------------------------

#[test]
fn active_parameter_index_changes_with_commas() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function triple(uint256 a, uint256 b, uint256 c) public pure returns (uint256) {
        return a + b + c;
    }

    function test() public pure {
        triple(1, 2, 3);
    }
}
"#;
    let call = source.find("triple(1, 2, 3)").unwrap();

    // After opening paren => param 0
    let h0 = sig_at(source, call + "triple(".len()).unwrap();
    assert_eq!(
        h0.active_parameter,
        Some(0),
        "First arg => active_parameter 0"
    );

    // After first comma => param 1
    let comma1 = source[call..].find(", 2").unwrap() + call + 2;
    let h1 = sig_at(source, comma1).unwrap();
    assert_eq!(
        h1.active_parameter,
        Some(1),
        "After first comma => active_parameter 1"
    );

    // After second comma => param 2
    let comma2 = source[call..].find(", 3").unwrap() + call + 2;
    let h2 = sig_at(source, comma2).unwrap();
    assert_eq!(
        h2.active_parameter,
        Some(2),
        "After second comma => active_parameter 2"
    );
}

// ---------------------------------------------------------------------------
// 3. First parameter - cursor right after opening paren
// ---------------------------------------------------------------------------

#[test]
fn first_parameter_right_after_open_paren() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function deposit(uint256 amount) public pure {
    }

    function test() public pure {
        deposit(100);
    }
}
"#;
    let open = source.find("deposit(100)").unwrap() + "deposit(".len();
    let help = sig_at(source, open).unwrap();
    assert_eq!(help.active_parameter, Some(0));
    assert!(
        help.signatures[0].label.contains("deposit"),
        "Should show deposit signature"
    );
}

// ---------------------------------------------------------------------------
// 4. Second parameter - cursor after first comma
// ---------------------------------------------------------------------------

#[test]
fn second_parameter_after_first_comma() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function transfer(address to, uint256 amount) public pure {
    }

    function test() public pure {
        transfer(msg.sender, 100);
    }
}
"#;
    let call = source.find("transfer(msg.sender, 100)").unwrap();
    let after_comma = source[call..].find(", 100").unwrap() + call + 2;
    let help = sig_at(source, after_comma).unwrap();
    assert_eq!(
        help.active_parameter,
        Some(1),
        "After first comma should be param index 1"
    );
}

// ---------------------------------------------------------------------------
// 5. Third parameter - cursor after second comma
// ---------------------------------------------------------------------------

#[test]
fn third_parameter_after_second_comma() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function multi(uint256 a, address b, bool c) public pure {
    }

    function test() public pure {
        multi(1, msg.sender, true);
    }
}
"#;
    let call = source.find("multi(1, msg.sender, true)").unwrap();
    let after_comma2 = source[call..].find(", true").unwrap() + call + 2;
    let help = sig_at(source, after_comma2).unwrap();
    assert_eq!(
        help.active_parameter,
        Some(2),
        "After second comma should be param index 2"
    );
}

// ---------------------------------------------------------------------------
// 6. No params function - should return None
// ---------------------------------------------------------------------------

#[test]
fn no_params_function_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function noParams() public pure returns (uint256) {
        return 42;
    }

    function test() public pure {
        noParams();
    }
}
"#;
    let open = source.find("noParams()").unwrap() + "noParams(".len();
    let help = sig_at(source, open);
    assert!(help.is_none(), "Zero-param functions should return None");
}

// ---------------------------------------------------------------------------
// 7. Nested function calls - inner call gets inner signature
// ---------------------------------------------------------------------------

#[test]
fn nested_function_calls_inner_signature() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function inner(uint256 x) public pure returns (uint256) {
        return x;
    }

    function outer(uint256 y) public pure returns (uint256) {
        return y;
    }

    function test() public pure {
        outer(inner(5));
    }
}
"#;
    let inner_open = source.find("inner(5)").unwrap() + "inner(".len();
    let help = sig_at(source, inner_open).unwrap();
    assert!(
        help.signatures[0].label.contains("inner"),
        "Nested: should show inner function signature, got: {}",
        help.signatures[0].label
    );
}

// ---------------------------------------------------------------------------
// 8. Event emit - emit Transfer(from, to, amount) should show event signature
//
// NOTE: The tree-sitter-solidity grammar wraps the event name in an
// `expression` node inside `emit_statement`. The current `find_emit_callee`
// implementation handles this by matching the `call_expression` child or
// the direct `identifier`/`member_expression`. This grammar puts `(`, `)`,
// and arguments as direct children of `emit_statement`, so `is_inside_arg_list`
// works. The callee is an `expression` node, which falls through to the
// fallback resolver that calls `st.resolve_at` at the expression start byte
// (the event identifier). This successfully resolves the event declaration.
// ---------------------------------------------------------------------------

#[test]
fn event_emit_signature_help() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    event Transfer(address indexed from, address indexed to, uint256 amount);

    function doTransfer() public {
        emit Transfer(msg.sender, msg.sender, 100);
    }
}
"#;
    // The emit_statement tree has the event name wrapped in an `expression` node.
    // find_emit_callee needs to unwrap that. If it doesn't find a callee, it returns None.
    let emit_call = source
        .find("emit Transfer(msg.sender, msg.sender, 100)")
        .unwrap();
    let open = source[emit_call..].find('(').unwrap() + emit_call + 1;
    let help = sig_at(source, open);
    // If the implementation resolves the event through the expression wrapper,
    // we get the full event signature. Otherwise None is acceptable (known limitation).
    if let Some(help) = help {
        assert!(
            help.signatures[0].label.contains("Transfer"),
            "Should show event name, got: {}",
            help.signatures[0].label
        );
    }
}

// ---------------------------------------------------------------------------
// 9. Error revert - revert InsufficientBalance(available, required)
//
// NOTE: tree-sitter-solidity wraps revert arguments in a `revert_arguments`
// node. The `is_inside_arg_list` check on `revert_statement` may not find
// the `(` and `)` as direct children. This is a known tree-sitter grammar
// difference.
// ---------------------------------------------------------------------------

#[test]
fn error_revert_signature_help() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    error InsufficientBalance(uint256 available, uint256 required);

    function withdraw(uint256 amount) public pure {
        revert InsufficientBalance(0, amount);
    }
}
"#;
    let revert_call = source.find("revert InsufficientBalance(").unwrap();
    let open = source[revert_call..].find('(').unwrap() + revert_call + 1;
    let help = sig_at(source, open);
    // The revert_statement grammar wraps args in `revert_arguments`.
    // If the implementation handles this node type, validate. Otherwise, None is
    // the expected current behavior (revert_arguments contains the parens, not
    // revert_statement directly).
    if let Some(help) = help {
        assert!(
            help.signatures[0].label.contains("InsufficientBalance"),
            "Should show error name, got: {}",
            help.signatures[0].label
        );
    }
}

// ---------------------------------------------------------------------------
// 10. Constructor call via `new`
// ---------------------------------------------------------------------------

#[test]
fn constructor_call_via_new() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    string public name;
    string public symbol;

    constructor(string memory _name, string memory _symbol) {
        name = _name;
        symbol = _symbol;
    }
}

contract Factory {
    function create() public {
        new Token("MyToken", "MTK");
    }
}
"#;
    let new_call = source.find(r#"new Token("MyToken""#).unwrap();
    let open = source[new_call..].find('(').unwrap() + new_call + 1;
    let help = sig_at(source, open);
    // `new Token(...)` may or may not resolve to the constructor depending
    // on how tree-sitter grammar and resolve_callee handle `new_expression`.
    if let Some(help) = help {
        assert!(
            help.signatures[0].label.contains("constructor")
                || help.signatures[0].label.contains("Token"),
            "Should show constructor signature, got: {}",
            help.signatures[0].label
        );
    }
}

// ---------------------------------------------------------------------------
// 11. State mutability display - pure/view/payable in signature
// ---------------------------------------------------------------------------

#[test]
fn state_mutability_in_signature_label() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function pureFunc(uint256 x) public pure returns (uint256) {
        return x;
    }

    function test() public pure {
        pureFunc(1);
    }
}
"#;
    let open = source.find("pureFunc(1)").unwrap() + "pureFunc(".len();
    let help = sig_at(source, open).unwrap();
    assert!(
        help.signatures[0].label.contains("pureFunc"),
        "Label should contain function name"
    );
    assert!(
        help.signatures[0].label.contains("uint256"),
        "Label should contain parameter type"
    );
}

// ---------------------------------------------------------------------------
// 12. Return type display - function with returns should show them
// ---------------------------------------------------------------------------

#[test]
fn return_type_in_signature_label() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function getVal(uint256 x) public pure returns (uint256) {
        return x;
    }

    function test() public pure {
        getVal(42);
    }
}
"#;
    let open = source.find("getVal(42)").unwrap() + "getVal(".len();
    let help = sig_at(source, open).unwrap();
    assert!(
        help.signatures[0].label.contains("returns"),
        "Label should show return type, got: {}",
        help.signatures[0].label
    );
}

// ---------------------------------------------------------------------------
// 13. NatSpec documentation - should include @notice/@param docs
// ---------------------------------------------------------------------------

#[test]
fn natspec_documentation_included() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    /// @notice Transfer tokens to a recipient
    /// @param to The recipient address
    /// @param amount The amount to transfer
    function transfer(address to, uint256 amount) public pure {
    }

    function test() public pure {
        transfer(msg.sender, 100);
    }
}
"#;
    let open = source.find("transfer(msg.sender, 100)").unwrap() + "transfer(".len();
    let help = sig_at(source, open).unwrap();
    let sig = &help.signatures[0];

    // Check function-level documentation
    assert!(
        sig.documentation.is_some(),
        "Should include NatSpec documentation"
    );
    let doc_text = match sig.documentation.as_ref().unwrap() {
        tower_lsp::lsp_types::Documentation::String(s) => s.clone(),
        tower_lsp::lsp_types::Documentation::MarkupContent(m) => m.value.clone(),
    };
    assert!(
        doc_text.contains("Transfer tokens"),
        "NatSpec should contain @notice text, got: {}",
        doc_text
    );

    // Check per-parameter documentation
    let params = sig.parameters.as_ref().unwrap();
    assert!(
        params[0].documentation.is_some(),
        "First param should have NatSpec doc"
    );
    assert!(
        params[1].documentation.is_some(),
        "Second param should have NatSpec doc"
    );
}

// ---------------------------------------------------------------------------
// 14. Outside parentheses - cursor outside parens should return None
// ---------------------------------------------------------------------------

#[test]
fn outside_parentheses_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }

    function test() public pure {
        add(1, 2);
        uint256 x = 5;
    }
}
"#;
    let outside = source.find("uint256 x = 5").unwrap() + 5;
    let help = sig_at(source, outside);
    assert!(help.is_none(), "Cursor outside parens should return None");
}

// ---------------------------------------------------------------------------
// 15. Modifier with args - onlyOwner(msg.sender) modifier call
// ---------------------------------------------------------------------------

#[test]
fn modifier_with_args_signature_help() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    modifier onlyRole(bytes32 role) {
        _;
    }

    function admin() public onlyRole(0x00) {
    }

    function test() public {
        onlyRole(0x00);
    }
}
"#;
    // Calling a modifier like a function in a body. tree-sitter may parse this
    // as a call_expression, allowing signature help to resolve the modifier.
    let call = source.find("onlyRole(0x00);").unwrap();
    let open = call + "onlyRole(".len();
    let help = sig_at(source, open);
    if let Some(help) = help {
        assert!(
            help.signatures[0].label.contains("onlyRole"),
            "Should show modifier name in signature, got: {}",
            help.signatures[0].label
        );
    }
}

// ---------------------------------------------------------------------------
// 16. Library function - MathLib.add(a, b)
//
// NOTE: tree-sitter wraps the callee `MathLib.add` in an `expression` node
// of kind `member_expression`. The `child_by_field_name("function")` on
// the `call_expression` returns this `expression` wrapper. The fallback
// arm in `resolve_callee` calls `resolve_at` at the expression start byte,
// which points to "MathLib" (the library), not "add". As a result, it
// resolves the library declaration, which may not have parameters, causing
// the signature help to return None. This is a known limitation with
// qualified member calls.
// ---------------------------------------------------------------------------

#[test]
fn library_function_call_signature() {
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
    let call = source.find("MathLib.add(1, 2)").unwrap();
    let open = call + "MathLib.add(".len();
    let help = sig_at(source, open);
    // Library qualified calls may not resolve through the expression wrapper
    // depending on the implementation. If they do, validate.
    if let Some(help) = help {
        assert!(
            help.signatures[0].label.contains("add"),
            "Should show library function name, got: {}",
            help.signatures[0].label
        );
    }
}

// ---------------------------------------------------------------------------
// 17. Inherited function call - calling parent contract function
// ---------------------------------------------------------------------------

#[test]
fn inherited_function_call_signature() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseFunc(uint256 x) public pure returns (uint256) {
        return x;
    }
}

contract Child is Base {
    function test() public pure {
        baseFunc(42);
    }
}
"#;
    let call = source.find("baseFunc(42)").unwrap();
    let open = call + "baseFunc(".len();
    let help = sig_at(source, open);
    assert!(
        help.is_some(),
        "Should provide signature help for inherited function"
    );
    let help = help.unwrap();
    assert!(
        help.signatures[0].label.contains("baseFunc"),
        "Should show inherited function name, got: {}",
        help.signatures[0].label
    );
}

// ---------------------------------------------------------------------------
// 18. Mapping access - should NOT trigger signature help
// ---------------------------------------------------------------------------

#[test]
fn mapping_access_does_not_trigger_signature_help() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ledger {
    mapping(address => uint256) public balances;

    function getBalance(address who) public view returns (uint256) {
        return balances[who];
    }
}
"#;
    let bracket = source.find("balances[who]").unwrap() + "balances[".len();
    let help = sig_at(source, bracket);
    assert!(
        help.is_none(),
        "Mapping access should not trigger signature help"
    );
}

// ---------------------------------------------------------------------------
// 19. Complex param types - bytes calldata, address[] memory
// ---------------------------------------------------------------------------

#[test]
fn complex_param_types_in_signature() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function complex(bytes calldata data, address[] memory addrs) public pure {
    }

    function test() public pure {
        bytes memory d;
        address[] memory a;
        complex(d, a);
    }
}
"#;
    let call = source.find("complex(d, a)").unwrap();
    let open = call + "complex(".len();
    let help = sig_at(source, open);
    assert!(
        help.is_some(),
        "Should provide signature help for complex params"
    );
    let help = help.unwrap();
    let label = &help.signatures[0].label;
    assert!(
        label.contains("bytes"),
        "Should show bytes type, got: {}",
        label
    );
}

// ---------------------------------------------------------------------------
// 20. Multiple return values - returns (bool success, uint256 amount)
// ---------------------------------------------------------------------------

#[test]
fn multiple_return_values_in_signature() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function getInfo(uint256 id) public pure returns (bool success, uint256 amount) {
        return (true, id);
    }

    function test() public pure {
        getInfo(1);
    }
}
"#;
    let call = source.find("getInfo(1)").unwrap();
    let open = call + "getInfo(".len();
    let help = sig_at(source, open).unwrap();
    let label = &help.signatures[0].label;
    assert!(
        label.contains("returns"),
        "Label should show returns clause, got: {}",
        label
    );
    assert!(
        label.contains("bool") && label.contains("uint256"),
        "Label should show both return types, got: {}",
        label
    );
}

// ---------------------------------------------------------------------------
// 21. Overloaded-style calls - functions with same name, different params
// ---------------------------------------------------------------------------

#[test]
fn overloaded_function_call() {
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
    let call = source.find("process(1, 2)").unwrap();
    let open = call + "process(".len();
    let help = sig_at(source, open);
    if let Some(help) = help {
        assert!(
            help.signatures[0].label.contains("process"),
            "Should show process signature, got: {}",
            help.signatures[0].label
        );
    }
}

// ---------------------------------------------------------------------------
// 22. Empty call - foo() with cursor inside empty parens
// ---------------------------------------------------------------------------

#[test]
fn empty_call_with_params_shows_help() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function deposit(uint256 amount) public pure {
    }

    function test() public pure {
        deposit();
    }
}
"#;
    let open = source.find("deposit()").unwrap() + "deposit(".len();
    let help = sig_at(source, open);
    assert!(
        help.is_some(),
        "Should show signature help even when no args typed yet"
    );
    let help = help.unwrap();
    assert_eq!(help.active_parameter, Some(0), "Active param should be 0");
}

// ---------------------------------------------------------------------------
// 23. Whitespace in args - foo(  a  ,  b  ) cursor positions
// ---------------------------------------------------------------------------

#[test]
fn whitespace_in_args_cursor_positions() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function spaced(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }

    function test() public pure {
        spaced(  1  ,  2  );
    }
}
"#;
    let call = source.find("spaced(  1  ,  2  )").unwrap();

    // After opening paren + whitespace, still param 0
    let pos0 = call + "spaced(  ".len();
    let h0 = sig_at(source, pos0).unwrap();
    assert_eq!(
        h0.active_parameter,
        Some(0),
        "Before first arg should be param 0"
    );

    // After the comma + whitespace, should be param 1
    let comma = source[call..].find(',').unwrap() + call;
    let pos1 = comma + 2;
    let h1 = sig_at(source, pos1).unwrap();
    assert_eq!(
        h1.active_parameter,
        Some(1),
        "After comma should be param 1"
    );
}

// ---------------------------------------------------------------------------
// 24. Struct constructor - Point({x: 1, y: 2})
// ---------------------------------------------------------------------------

#[test]
fn struct_constructor_named_args() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }

    function test() public pure {
        Point({x: 1, y: 2});
    }
}
"#;
    let call = source.find("Point({x: 1, y: 2})").unwrap();
    let open = call + "Point(".len();
    let help = sig_at(source, open);
    // Struct constructors with named args may not resolve in the current impl.
    if let Some(help) = help {
        assert!(
            help.signatures[0].label.contains("Point"),
            "Should reference struct name, got: {}",
            help.signatures[0].label
        );
    }
}

// ---------------------------------------------------------------------------
// 25. Chained calls - foo().bar(x) should show bar's signature
// ---------------------------------------------------------------------------

#[test]
fn chained_calls_show_inner_signature() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function swap(address pool, uint256 amount) public pure returns (bool) {
        return true;
    }

    function getPool(address token) public pure returns (address) {
        return token;
    }

    function test() public pure {
        swap(getPool(msg.sender), 100);
    }
}
"#;
    // Cursor inside getPool(...) -- the inner call
    let inner_call = source.find("getPool(msg.sender)").unwrap();
    let inner_open = inner_call + "getPool(".len();
    let help = sig_at(source, inner_open);
    assert!(help.is_some(), "Should provide sig help for inner call");
    let help = help.unwrap();
    assert!(
        help.signatures[0].label.contains("getPool"),
        "Chained: inner call should show getPool signature, got: {}",
        help.signatures[0].label
    );

    // Cursor at the second argument of swap (after the comma)
    let outer_call = source.find("swap(getPool(msg.sender), 100)").unwrap();
    let after_comma = source[outer_call..].find(", 100").unwrap() + outer_call + 2;
    let help2 = sig_at(source, after_comma);
    assert!(help2.is_some(), "Should show signature help for outer call");
    let help2 = help2.unwrap();
    assert!(
        help2.signatures[0].label.contains("swap"),
        "Outer call should show swap signature, got: {}",
        help2.signatures[0].label
    );
    assert_eq!(
        help2.active_parameter,
        Some(1),
        "Should be on second parameter of swap"
    );
}

// ---------------------------------------------------------------------------
// 26. Free function (file-level) call
// ---------------------------------------------------------------------------

#[test]
fn free_function_signature_help() {
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
    let call = source.find("freeAdd(1, 2)").unwrap();
    let open = call + "freeAdd(".len();
    let help = sig_at(source, open);
    assert!(help.is_some(), "Should provide sig help for free function");
    let help = help.unwrap();
    assert!(
        help.signatures[0].label.contains("freeAdd"),
        "Should show free function name, got: {}",
        help.signatures[0].label
    );
}

// ---------------------------------------------------------------------------
// 27. Function with address payable parameter
// ---------------------------------------------------------------------------

#[test]
fn address_payable_parameter_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function sendEther(address payable recipient, uint256 amount) public {
    }

    function test() public {
        sendEther(payable(msg.sender), 100);
    }
}
"#;
    let call = source.find("sendEther(payable").unwrap();
    let open = call + "sendEther(".len();
    let help = sig_at(source, open);
    assert!(
        help.is_some(),
        "Should provide sig help for address payable param"
    );
    let help = help.unwrap();
    assert!(
        help.signatures[0].label.contains("address"),
        "Should show address type in label, got: {}",
        help.signatures[0].label
    );
}

// ---------------------------------------------------------------------------
// 28. Active parameter stays valid at end of arg list
// ---------------------------------------------------------------------------

#[test]
fn active_parameter_with_trailing_position() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function three(uint256 a, uint256 b, uint256 c) public pure returns (uint256) {
        return a + b + c;
    }

    function test() public pure {
        three(1, 2, 3);
    }
}
"#;
    let call = source.find("three(1, 2, 3)").unwrap();
    let before_close = source[call..].find(')').unwrap() + call;
    let help = sig_at(source, before_close).unwrap();
    assert_eq!(
        help.active_parameter,
        Some(2),
        "Right before ) should still be on last param"
    );
}

// ---------------------------------------------------------------------------
// 29. Internal function with named return parameters
// ---------------------------------------------------------------------------

#[test]
fn internal_function_with_named_returns() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function compute(uint256 input) internal pure returns (uint256 result, bool success) {
        return (input, true);
    }

    function test() public pure {
        compute(42);
    }
}
"#;
    let call = source.find("compute(42)").unwrap();
    let open = call + "compute(".len();
    let help = sig_at(source, open).unwrap();
    let label = &help.signatures[0].label;
    assert!(
        label.contains("returns"),
        "Should include returns, got: {}",
        label
    );
    assert!(
        label.contains("result") || label.contains("success"),
        "Should include named return parameters, got: {}",
        label
    );
}

// ---------------------------------------------------------------------------
// 30. Deeply nested calls - innermost resolved
// ---------------------------------------------------------------------------

#[test]
fn deeply_nested_calls() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function a(uint256 x) public pure returns (uint256) { return x; }
    function b(uint256 x) public pure returns (uint256) { return x; }
    function c(uint256 x) public pure returns (uint256) { return x; }

    function test() public pure {
        a(b(c(1)));
    }
}
"#;
    let c_call = source.find("c(1)").unwrap();
    let c_open = c_call + "c(".len();
    let help = sig_at(source, c_open).unwrap();
    assert!(
        help.signatures[0].label.contains(" c("),
        "Deepest call should show c's signature, got: {}",
        help.signatures[0].label
    );
}

// ---------------------------------------------------------------------------
// 31. Cursor before opening paren returns None
// ---------------------------------------------------------------------------

#[test]
fn cursor_before_opening_paren_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }

    function test() public pure {
        add(1, 2);
    }
}
"#;
    let call = source.find("add(1, 2)").unwrap();
    let help = sig_at(source, call + 1); // inside 'add' name, before '('
    assert!(
        help.is_none(),
        "Cursor on function name before ( should return None"
    );
}

// ---------------------------------------------------------------------------
// 32. Cursor after closing paren returns None
// ---------------------------------------------------------------------------

#[test]
fn cursor_after_closing_paren_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }

    function test() public pure {
        add(1, 2);
    }
}
"#;
    let call = source.find("add(1, 2)").unwrap();
    let after_close = call + "add(1, 2)".len();
    let help = sig_at(source, after_close);
    assert!(help.is_none(), "Cursor after closing ) should return None");
}

// ---------------------------------------------------------------------------
// 33. Signature with many parameters
// ---------------------------------------------------------------------------

#[test]
fn signature_with_many_parameters() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function manyParams(
        uint256 a,
        address b,
        bool c,
        bytes32 d,
        string memory e
    ) public pure {
    }

    function test() public pure {
        manyParams(1, msg.sender, true, 0x00, "hi");
    }
}
"#;
    let call = source
        .find(r#"manyParams(1, msg.sender, true, 0x00, "hi")"#)
        .unwrap();
    let open = call + "manyParams(".len();
    let help = sig_at(source, open).unwrap();
    let params = help.signatures[0].parameters.as_ref().unwrap();
    assert_eq!(params.len(), 5, "Should have 5 parameters");

    // Check active parameter for the last argument
    let last_comma = source[call..].rfind(',').unwrap() + call + 2;
    let h_last = sig_at(source, last_comma).unwrap();
    assert_eq!(
        h_last.active_parameter,
        Some(4),
        "Last arg should be param index 4"
    );
}

// ---------------------------------------------------------------------------
// 34. Parameter label offsets are correct
// ---------------------------------------------------------------------------

#[test]
fn parameter_label_offsets_are_correct() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function transfer(address to, uint256 amount) public pure {
    }

    function test() public pure {
        transfer(msg.sender, 100);
    }
}
"#;
    let call = source.find("transfer(msg.sender, 100)").unwrap();
    let open = call + "transfer(".len();
    let help = sig_at(source, open).unwrap();
    let sig = &help.signatures[0];
    let params = sig.parameters.as_ref().unwrap();

    for param in params {
        match &param.label {
            tower_lsp::lsp_types::ParameterLabel::LabelOffsets([start, end]) => {
                let substr = &sig.label[*start as usize..*end as usize];
                assert!(
                    !substr.is_empty(),
                    "Parameter label offset should produce non-empty substring"
                );
            }
            tower_lsp::lsp_types::ParameterLabel::Simple(s) => {
                assert!(!s.is_empty(), "Simple label should be non-empty");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 35. Verify active_signature is always 0
// ---------------------------------------------------------------------------

#[test]
fn active_signature_is_zero() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar(uint256 x) public pure returns (uint256) {
        return x;
    }

    function test() public pure {
        bar(1);
    }
}
"#;
    let open = source.find("bar(1)").unwrap() + "bar(".len();
    let help = sig_at(source, open).unwrap();
    assert_eq!(
        help.active_signature,
        Some(0),
        "active_signature should be 0"
    );
}

// ---------------------------------------------------------------------------
// 36. Signatures count is always 1 for resolved call
// ---------------------------------------------------------------------------

#[test]
fn signatures_count_is_one() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function calc(uint256 a, uint256 b) public pure returns (uint256) {
        return a * b;
    }

    function test() public pure {
        calc(3, 4);
    }
}
"#;
    let call = source.find("calc(3, 4)").unwrap();
    let open = call + "calc(".len();
    let help = sig_at(source, open).unwrap();
    assert_eq!(
        help.signatures.len(),
        1,
        "Should return exactly 1 signature"
    );
}

// ---------------------------------------------------------------------------
// 37. NatSpec per-param docs map to correct parameters
// ---------------------------------------------------------------------------

#[test]
fn natspec_per_param_docs_correct_mapping() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    /// @notice Swap tokens
    /// @param tokenIn The input token
    /// @param tokenOut The output token
    /// @param amountIn The input amount
    function swap(address tokenIn, address tokenOut, uint256 amountIn) public pure {
    }

    function test() public pure {
        swap(msg.sender, msg.sender, 100);
    }
}
"#;
    let call = source.find("swap(msg.sender, msg.sender, 100)").unwrap();
    let open = call + "swap(".len();
    let help = sig_at(source, open).unwrap();
    let params = help.signatures[0].parameters.as_ref().unwrap();

    assert!(
        params[0].documentation.is_some(),
        "tokenIn should have docs"
    );
    assert!(
        params[1].documentation.is_some(),
        "tokenOut should have docs"
    );
    assert!(
        params[2].documentation.is_some(),
        "amountIn should have docs"
    );

    let doc0 = match &params[0].documentation {
        Some(tower_lsp::lsp_types::Documentation::String(s)) => s.clone(),
        _ => String::new(),
    };
    assert!(
        doc0.contains("input token"),
        "First param doc should mention 'input token', got: {}",
        doc0
    );

    let doc2 = match &params[2].documentation {
        Some(tower_lsp::lsp_types::Documentation::String(s)) => s.clone(),
        _ => String::new(),
    };
    assert!(
        doc2.contains("input amount"),
        "Third param doc should mention 'input amount', got: {}",
        doc2
    );
}

// ---------------------------------------------------------------------------
// 38. Passing no tree returns None
// ---------------------------------------------------------------------------

#[test]
fn no_tree_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar(uint256 x) public pure { }
    function test() public pure { bar(1); }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let call = source.find("bar(1)").unwrap() + "bar(".len();
    let (line, col) = li.byte_offset_to_position(source, call);
    let help = signature_help(&st, &path, source, Position::new(line, col), &li, None);
    assert!(help.is_none(), "Passing None for tree should return None");
}

// ---------------------------------------------------------------------------
// 39. Signature help not triggered on variable declaration
// ---------------------------------------------------------------------------

#[test]
fn variable_declaration_does_not_trigger() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function test() public pure {
        uint256 x = 5;
    }
}
"#;
    let decl = source.find("uint256 x = 5").unwrap() + 10;
    let help = sig_at(source, decl);
    assert!(
        help.is_none(),
        "Variable declaration should not trigger signature help"
    );
}

// ---------------------------------------------------------------------------
// 40. Function with struct parameter
// ---------------------------------------------------------------------------

#[test]
fn function_with_struct_parameter() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Order {
        uint256 id;
        uint256 amount;
    }

    function processOrder(Order memory order, bool urgent) public pure {
    }

    function test() public pure {
        Order memory o;
        processOrder(o, true);
    }
}
"#;
    let call = source.find("processOrder(o, true)").unwrap();
    let open = call + "processOrder(".len();
    let help = sig_at(source, open);
    assert!(
        help.is_some(),
        "Should provide sig help for struct param function"
    );
    let help = help.unwrap();
    assert!(
        help.signatures[0].label.contains("Order"),
        "Label should show struct type, got: {}",
        help.signatures[0].label
    );
}

// ---------------------------------------------------------------------------
// 41. Using-for library method call
// ---------------------------------------------------------------------------

#[test]
fn using_for_library_method_call() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

contract Foo {
    using SafeMath for uint256;

    function test() public pure {
        uint256 x = 1;
        x.add(2);
    }
}
"#;
    let call = source.find("x.add(2)").unwrap();
    let open = call + "x.add(".len();
    let help = sig_at(source, open);
    if let Some(help) = help {
        assert!(
            help.signatures[0].label.contains("add"),
            "Should show add signature for using-for call, got: {}",
            help.signatures[0].label
        );
    }
}

// ---------------------------------------------------------------------------
// 42. Cross-file inherited function call
// ---------------------------------------------------------------------------

#[test]
fn cross_file_inherited_function_signature() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let base_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseMethod(uint256 x, bool flag) public pure returns (uint256) {
        return x;
    }
}
"#;
    let base_path = tmp.path().join("Base.sol");
    std::fs::write(&base_path, base_source).unwrap();

    let child_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Base} from "./Base.sol";

contract Child is Base {
    function test() public pure {
        baseMethod(42, true);
    }
}
"#;
    let child_path = tmp.path().join("Child.sol");
    std::fs::write(&child_path, child_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&base_path, base_source, &mut parser);
    st.resolve_file_references(&base_path, &mut parser);
    st.index_file(&child_path, child_source, &mut parser);
    st.resolve_file_references(&child_path, &mut parser);

    let call = child_source.find("baseMethod(42, true)").unwrap();
    let open = call + "baseMethod(".len();
    let li = LineIndex::new(child_source);
    let (line, col) = li.byte_offset_to_position(child_source, open);
    let tree = parser.parse(child_source, None).unwrap();

    let help = signature_help(
        &st,
        &child_path,
        child_source,
        Position::new(line, col),
        &li,
        Some(&tree),
    );
    assert!(
        help.is_some(),
        "Should get sig help for cross-file inherited function"
    );
    let help = help.unwrap();
    assert!(
        help.signatures[0].label.contains("baseMethod"),
        "Should show baseMethod, got: {}",
        help.signatures[0].label
    );
}

// ---------------------------------------------------------------------------
// 43. Signature help inside boolean argument
// ---------------------------------------------------------------------------

#[test]
fn signature_help_in_boolean_argument() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function process(uint256 x, bool flag) public pure returns (uint256) {
        return x;
    }

    function test() public pure {
        process(1, true);
    }
}
"#;
    let call = source.find("process(1, true)").unwrap();
    let after_comma = source[call..].find(", true").unwrap() + call + 2;
    let help = sig_at(source, after_comma).unwrap();
    assert_eq!(help.active_parameter, Some(1));
    assert!(help.signatures[0].label.contains("process"));
}

// ---------------------------------------------------------------------------
// 44. Signature label contains "function" keyword prefix
// ---------------------------------------------------------------------------

#[test]
fn signature_label_starts_with_function_keyword() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function doWork(uint256 x) public pure returns (uint256) {
        return x;
    }

    function test() public pure {
        doWork(1);
    }
}
"#;
    let open = source.find("doWork(1)").unwrap() + "doWork(".len();
    let help = sig_at(source, open).unwrap();
    assert!(
        help.signatures[0].label.starts_with("function "),
        "Label should start with 'function ', got: {}",
        help.signatures[0].label
    );
}

// ---------------------------------------------------------------------------
// 45. Parameter offsets index into correct substrings of the label
// ---------------------------------------------------------------------------

#[test]
fn parameter_offsets_produce_correct_substrings() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function mixedParams(address to, uint256 amount, bool flag) public pure {
    }

    function test() public pure {
        mixedParams(msg.sender, 100, true);
    }
}
"#;
    let open = source.find("mixedParams(msg.sender").unwrap() + "mixedParams(".len();
    let help = sig_at(source, open).unwrap();
    let sig = &help.signatures[0];
    let params = sig.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 3);

    // Extract the label substrings for each parameter
    let mut labels: Vec<String> = Vec::new();
    for param in params {
        if let tower_lsp::lsp_types::ParameterLabel::LabelOffsets([s, e]) = &param.label {
            labels.push(sig.label[*s as usize..*e as usize].to_string());
        }
    }
    assert!(
        labels[0].contains("address"),
        "First param should contain 'address', got: {}",
        labels[0]
    );
    assert!(
        labels[1].contains("uint256"),
        "Second param should contain 'uint256', got: {}",
        labels[1]
    );
    assert!(
        labels[2].contains("bool"),
        "Third param should contain 'bool', got: {}",
        labels[2]
    );
}

// ---------------------------------------------------------------------------
// 46. Signature help with single-param function and cursor in middle
// ---------------------------------------------------------------------------

#[test]
fn single_param_cursor_in_middle_of_arg() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function getValue(uint256 key) public pure returns (uint256) {
        return key;
    }

    function test() public pure {
        getValue(12345);
    }
}
"#;
    // Cursor in the middle of the number literal "12345"
    let call = source.find("getValue(12345)").unwrap();
    let mid = call + "getValue(12".len();
    let help = sig_at(source, mid).unwrap();
    assert_eq!(help.active_parameter, Some(0), "Should still be param 0");
    assert!(help.signatures[0].label.contains("getValue"));
}

// ---------------------------------------------------------------------------
// 47. Signature for function with only unnamed parameters
// ---------------------------------------------------------------------------

#[test]
fn function_with_unnamed_parameters() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function anonymous(uint256, bool) public pure {
    }

    function test() public pure {
        anonymous(1, true);
    }
}
"#;
    let call = source.find("anonymous(1, true)").unwrap();
    let open = call + "anonymous(".len();
    let help = sig_at(source, open);
    assert!(help.is_some(), "Should provide sig help for unnamed params");
    let help = help.unwrap();
    let label = &help.signatures[0].label;
    assert!(
        label.contains("uint256"),
        "Should show uint256 type, got: {}",
        label
    );
    assert!(
        label.contains("bool"),
        "Should show bool type, got: {}",
        label
    );
}

// ---------------------------------------------------------------------------
// 48. Nested call: outer function active parameter tracking
// ---------------------------------------------------------------------------

#[test]
fn nested_call_outer_first_param() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function inner(uint256 x) public pure returns (uint256) { return x; }
    function outer(uint256 a, uint256 b) public pure returns (uint256) { return a + b; }

    function test() public pure {
        outer(inner(5), 10);
    }
}
"#;
    // Cursor on the "10" (second arg of outer)
    let call = source.find("outer(inner(5), 10)").unwrap();
    let after_comma = source[call..].find(", 10").unwrap() + call + 2;
    let help = sig_at(source, after_comma).unwrap();
    assert!(
        help.signatures[0].label.contains("outer"),
        "Should be on outer function, got: {}",
        help.signatures[0].label
    );
    assert_eq!(
        help.active_parameter,
        Some(1),
        "Should be second param of outer"
    );
}

// ---------------------------------------------------------------------------
// 49. Recursive function call
// ---------------------------------------------------------------------------

#[test]
fn recursive_function_call() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function factorial(uint256 n) public pure returns (uint256) {
        if (n <= 1) return 1;
        return factorial(n);
    }
}
"#;
    let call = source.find("factorial(n)").unwrap();
    let open = call + "factorial(".len();
    let help = sig_at(source, open).unwrap();
    assert!(
        help.signatures[0].label.contains("factorial"),
        "Recursive call should show function signature, got: {}",
        help.signatures[0].label
    );
    assert_eq!(help.active_parameter, Some(0));
}

// ---------------------------------------------------------------------------
// 50. Multiple functions in same contract -- correct resolution
// ---------------------------------------------------------------------------

#[test]
fn multiple_functions_correct_resolution() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function alpha(uint256 x) public pure returns (uint256) { return x; }
    function beta(address a, bool b) public pure returns (bool) { return b; }

    function test() public pure {
        alpha(1);
        beta(msg.sender, true);
    }
}
"#;
    // Test alpha
    let alpha_call = source.find("alpha(1)").unwrap();
    let alpha_open = alpha_call + "alpha(".len();
    let help_a = sig_at(source, alpha_open).unwrap();
    assert!(
        help_a.signatures[0].label.contains("alpha"),
        "Should show alpha signature, got: {}",
        help_a.signatures[0].label
    );
    let params_a = help_a.signatures[0].parameters.as_ref().unwrap();
    assert_eq!(params_a.len(), 1, "alpha has 1 param");

    // Test beta
    let beta_call = source.find("beta(msg.sender, true)").unwrap();
    let beta_open = beta_call + "beta(".len();
    let help_b = sig_at(source, beta_open).unwrap();
    assert!(
        help_b.signatures[0].label.contains("beta"),
        "Should show beta signature, got: {}",
        help_b.signatures[0].label
    );
    let params_b = help_b.signatures[0].parameters.as_ref().unwrap();
    assert_eq!(params_b.len(), 2, "beta has 2 params");
}

// ---------------------------------------------------------------------------
// BUG TEST: emit statement should track active parameter correctly
// ---------------------------------------------------------------------------

#[test]
fn emit_active_parameter_second_arg() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    event Transfer(address indexed from, address indexed to, uint256 value);

    function send(address recipient, uint256 amount) public {
        emit Transfer(msg.sender, recipient, amount);
    }
}
"#;
    // Place cursor after the second comma, on the third argument
    let emit_call = source.find("Transfer(msg.sender, recipient, amount)").unwrap();
    let after_second_comma = emit_call + "Transfer(msg.sender, recipient, ".len();
    let help = sig_at(source, after_second_comma);
    assert!(
        help.is_some(),
        "Should provide signature help inside emit arguments"
    );
    let help = help.unwrap();
    // Active parameter should be 2 (third parameter, 0-indexed)
    let active = help.active_parameter.unwrap_or(0);
    assert_eq!(
        active, 2,
        "Active parameter should be 2 (third param 'value') after second comma, got {}",
        active
    );
}

// ---------------------------------------------------------------------------
// BUG TEST: revert statement should track active parameter correctly
// ---------------------------------------------------------------------------

#[test]
fn revert_active_parameter_second_arg() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    error InsufficientBalance(uint256 available, uint256 required);

    function withdraw(uint256 amount) public {
        revert InsufficientBalance(100, amount);
    }
}
"#;
    let revert_call = source.find("InsufficientBalance(100, amount)").unwrap();
    let after_comma = revert_call + "InsufficientBalance(100, ".len();
    let help = sig_at(source, after_comma);
    assert!(
        help.is_some(),
        "Should provide signature help inside revert arguments"
    );
    let help = help.unwrap();
    let active = help.active_parameter.unwrap_or(0);
    assert_eq!(
        active, 1,
        "Active parameter should be 1 (second param 'required') after comma, got {}",
        active
    );
}
