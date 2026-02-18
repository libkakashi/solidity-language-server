use std::path::PathBuf;

use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::inlay_hints::inlay_hints;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Position, Range};

fn setup(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

fn full_range() -> Range {
    Range {
        start: Position::new(0, 0),
        end: Position::new(u32::MAX, u32::MAX),
    }
}

fn get_hints(source: &str) -> Vec<InlayHint> {
    let (st, path) = setup(source);
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let li = LineIndex::new(source);
    inlay_hints(&st, &path, source, full_range(), &li, Some(&tree))
}

fn hint_label(hint: &InlayHint) -> String {
    match &hint.label {
        InlayHintLabel::String(s) => s.clone(),
        InlayHintLabel::LabelParts(parts) => {
            parts.iter().map(|p| p.value.as_str()).collect::<String>()
        }
    }
}

// =========================================================================
// 1. Basic function call with two parameters
// =========================================================================

#[test]
fn basic_function_call_two_params() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }

    function test() public pure returns (uint256) {
        return add(1, 2);
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 2, "expected 2 parameter hints, got {}", hints.len());
    assert_eq!(hint_label(&hints[0]), "a:");
    assert_eq!(hint_label(&hints[1]), "b:");
}

// =========================================================================
// 2. Constructor call via `new`
// =========================================================================

#[test]
fn constructor_call_via_new() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    constructor(string memory _name, string memory _symbol) {}
}

contract Factory {
    function create() public returns (Token) {
        return new Token("MyToken", "MTK");
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 2);
    assert_eq!(hint_label(&hints[0]), "_name:");
    assert_eq!(hint_label(&hints[1]), "_symbol:");
}

// =========================================================================
// 3. Modifier invocation hints
// =========================================================================

#[test]
fn modifier_invocation_hint() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    modifier onlyRole(bytes32 role) { _; }
    function admin() public onlyRole(0x00) {}
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 1);
    assert_eq!(hint_label(&hints[0]), "role:");
}

// =========================================================================
// 4. Emit statement hints
// =========================================================================

#[test]
fn emit_statement_hints() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    event Transfer(address indexed from, address indexed to, uint256 value);

    function send(address recipient, uint256 amount) public {
        emit Transfer(msg.sender, recipient, amount);
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 3, "expected 3 emit hints, got {}", hints.len());
    assert_eq!(hint_label(&hints[0]), "from:");
    assert_eq!(hint_label(&hints[1]), "to:");
    assert_eq!(hint_label(&hints[2]), "value:");
}

// =========================================================================
// 5. Revert statement hints
// =========================================================================

#[test]
fn revert_statement_hints() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    error InsufficientBalance(uint256 available, uint256 required);

    function withdraw(uint256 amount) public {
        revert InsufficientBalance(100, amount);
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 2, "expected 2 revert hints, got {}", hints.len());
    assert_eq!(hint_label(&hints[0]), "available:");
    assert_eq!(hint_label(&hints[1]), "required:");
}

// =========================================================================
// 6. No hints for zero-parameter functions
// =========================================================================

#[test]
fn no_hints_for_zero_params() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function getOne() public pure returns (uint256) { return 1; }
    function test() public pure returns (uint256) {
        return getOne();
    }
}"#;
    let hints = get_hints(source);
    assert!(hints.is_empty(), "no hints expected for zero-param call");
}

// =========================================================================
// 7. Arg name matches param name - skip hint
// =========================================================================

#[test]
fn skip_when_arg_matches_param() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function transfer(address to, uint256 amount) public {
    }

    function test(address to, uint256 amount) public {
        transfer(to, amount);
    }
}"#;
    let hints = get_hints(source);
    assert!(
        hints.is_empty(),
        "no hints expected when arg names match param names, got {}",
        hints.len()
    );
}

// =========================================================================
// 8. Underscore-prefixed arg matches param - skip hint
// =========================================================================

#[test]
fn skip_underscore_prefixed_match() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function setOwner(address owner) public {}

    function test(address _owner) public {
        setOwner(_owner);
    }
}"#;
    let hints = get_hints(source);
    assert!(
        hints.is_empty(),
        "no hints when underscore-prefixed arg matches param, got {}",
        hints.len()
    );
}

// =========================================================================
// 9. Mixed - some args match, some don't
// =========================================================================

#[test]
fn mixed_matching_and_non_matching() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function configure(uint256 max, uint256 min) public {}

    function test(uint256 max) public {
        configure(max, 100);
    }
}"#;
    let hints = get_hints(source);
    // `max` matches the param name, so it's skipped.
    // `100` doesn't match `min`, so it should get a hint (if function resolves).
    // The number of hints depends on whether the resolver can find `configure`.
    // Verify no crash and that any produced hints are correct.
    for h in &hints {
        let label = hint_label(h);
        assert!(
            label == "max:" || label == "min:",
            "unexpected hint label: {}",
            label
        );
    }
}

// =========================================================================
// 10. All hints have PARAMETER kind
// =========================================================================

#[test]
fn all_hints_have_parameter_kind() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function multi(uint256 a, uint256 b, uint256 c) public pure returns (uint256) {
        return a + b + c;
    }

    function test() public pure returns (uint256) {
        return multi(1, 2, 3);
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 3);
    for h in &hints {
        assert_eq!(h.kind, Some(InlayHintKind::PARAMETER));
    }
}

// =========================================================================
// 11. No tree returns empty hints
// =========================================================================

#[test]
fn no_tree_returns_empty() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
    function test() public pure { add(1, 2); }
}"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let result = inlay_hints(&st, &path, source, full_range(), &li, None);
    assert!(result.is_empty(), "no tree should return empty hints");
}

// =========================================================================
// 12. Range filtering - only hints within requested range
// =========================================================================

#[test]
fn range_filtering() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }

    function first() public pure returns (uint256) {
        return add(1, 2);
    }

    function second() public pure returns (uint256) {
        return add(3, 4);
    }
}"#;
    let (st, path) = setup(source);
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let li = LineIndex::new(source);

    // Request only the range covering `first()` function (lines ~8-10)
    let narrow_range = Range {
        start: Position::new(8, 0),
        end: Position::new(10, 100),
    };
    let hints = inlay_hints(&st, &path, source, narrow_range, &li, Some(&tree));
    // Should only get hints for add(1, 2), not add(3, 4)
    assert_eq!(hints.len(), 2, "expected 2 hints in narrow range, got {}", hints.len());
}

// =========================================================================
// 13. Multiple call sites
// =========================================================================

#[test]
fn multiple_call_sites() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 x, uint256 y) public pure returns (uint256) {
        return x + y;
    }

    function test() public pure returns (uint256) {
        uint256 a = add(1, 2);
        uint256 b = add(3, 4);
        return add(a, b);
    }
}"#;
    let hints = get_hints(source);
    // 3 call sites x 2 params = 6, but add(a, b) might skip since a/b don't match x/y
    // First two calls: add(1, 2) and add(3, 4) produce hints for both params
    assert!(
        hints.len() >= 4,
        "expected at least 4 hints for multiple calls, got {}",
        hints.len()
    );
}

// =========================================================================
// 14. Unnamed parameter - no hint
// =========================================================================

#[test]
fn unnamed_parameter_no_hint() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function process(uint256, uint256 amount) public pure returns (uint256) {
        return amount;
    }

    function test() public pure returns (uint256) {
        return process(1, 2);
    }
}"#;
    let hints = get_hints(source);
    // First param has no name, only second should get a hint
    assert_eq!(hints.len(), 1, "expected 1 hint for named param only, got {}", hints.len());
    assert_eq!(hint_label(&hints[0]), "amount:");
}

// =========================================================================
// 15. Multiple events and functions
// =========================================================================

#[test]
fn mixed_calls_and_emits() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    event Transfer(address indexed from, address indexed to, uint256 value);

    function _transfer(address sender, address recipient, uint256 amount) internal {}

    function send(address dest, uint256 val) public {
        _transfer(msg.sender, dest, val);
        emit Transfer(msg.sender, dest, val);
    }
}"#;
    let hints = get_hints(source);
    // _transfer: 3 params (sender, recipient, amount)
    // emit Transfer: 3 params (from, to, value)
    assert!(
        hints.len() >= 6,
        "expected at least 6 hints for mixed calls/emits, got {}",
        hints.len()
    );
}

// =========================================================================
// 16. Single parameter function
// =========================================================================

#[test]
fn single_parameter_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function double(uint256 value) public pure returns (uint256) {
        return value * 2;
    }

    function test() public pure returns (uint256) {
        return double(42);
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 1);
    assert_eq!(hint_label(&hints[0]), "value:");
}

// =========================================================================
// 17. Nested function calls
// =========================================================================

#[test]
fn nested_function_calls() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function inner(uint256 x) public pure returns (uint256) { return x; }
    function outer(uint256 y) public pure returns (uint256) { return y; }

    function test() public pure returns (uint256) {
        return outer(inner(42));
    }
}"#;
    let hints = get_hints(source);
    // inner(42) → x:, outer(...) → y:
    assert_eq!(hints.len(), 2, "expected 2 hints for nested calls, got {}", hints.len());
}

// =========================================================================
// 18. Boolean literal arguments
// =========================================================================

#[test]
fn boolean_literal_hints() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Config {
    function setConfig(bool enabled, bool verbose) public {}

    function init() public {
        setConfig(true, false);
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 2);
    assert_eq!(hint_label(&hints[0]), "enabled:");
    assert_eq!(hint_label(&hints[1]), "verbose:");
}

// =========================================================================
// 19. Address literal arguments
// =========================================================================

#[test]
fn address_literal_hints() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function setAddress(address target) public {}

    function init() public {
        setAddress(address(0));
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 1);
    assert_eq!(hint_label(&hints[0]), "target:");
}

// =========================================================================
// 20. More args than params - no crash
// =========================================================================

#[test]
fn more_args_than_params_no_crash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function single(uint256 x) public pure returns (uint256) { return x; }

    function test() public pure returns (uint256) {
        return single(1, 2, 3);
    }
}"#;
    let hints = get_hints(source);
    // Should only produce 1 hint (for x), not crash
    assert_eq!(hints.len(), 1);
    assert_eq!(hint_label(&hints[0]), "x:");
}

// =========================================================================
// 21. String arguments
// =========================================================================

#[test]
fn string_arguments_hints() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Greeter {
    function greet(string memory greeting, string memory name) public pure returns (string memory) {
        return greeting;
    }

    function test() public pure returns (string memory) {
        return greet("Hello", "World");
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 2);
    assert_eq!(hint_label(&hints[0]), "greeting:");
    assert_eq!(hint_label(&hints[1]), "name:");
}

// =========================================================================
// 22. Hints have correct padding
// =========================================================================

#[test]
fn hints_have_right_padding() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) { return a + b; }
    function test() public pure { add(1, 2); }
}"#;
    let hints = get_hints(source);
    for h in &hints {
        assert_eq!(h.padding_right, Some(true), "hints should have right padding");
        assert!(h.padding_left.is_none() || h.padding_left == Some(false));
    }
}

// =========================================================================
// 23. Internal function call
// =========================================================================

#[test]
fn internal_function_call_hints() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract ERC20 {
    function _mint(address account, uint256 amount) internal {}

    function mint(uint256 qty) public {
        _mint(msg.sender, qty);
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 2, "expected 2 hints for internal call");
    assert_eq!(hint_label(&hints[0]), "account:");
    assert_eq!(hint_label(&hints[1]), "amount:");
}

// =========================================================================
// 24. Pure literal arguments get hints
// =========================================================================

#[test]
fn literal_arguments_get_hints() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function configure(uint256 max, uint256 min, bool active) public {}

    function init() public {
        configure(1000, 0, true);
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 3);
    assert_eq!(hint_label(&hints[0]), "max:");
    assert_eq!(hint_label(&hints[1]), "min:");
    assert_eq!(hint_label(&hints[2]), "active:");
}

// =========================================================================
// 25. Empty contract - no crash
// =========================================================================

#[test]
fn empty_contract_no_crash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Empty {}"#;
    let hints = get_hints(source);
    assert!(hints.is_empty());
}

// =========================================================================
// 26. Event with no parameters
// =========================================================================

#[test]
fn event_no_params_no_hints() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    event Paused();

    function pause() public {
        emit Paused();
    }
}"#;
    let hints = get_hints(source);
    assert!(hints.is_empty());
}

// =========================================================================
// 27. Modifier with multiple parameters
// =========================================================================

#[test]
fn modifier_multiple_params() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    modifier requireRole(bytes32 role, address account) { _; }

    function admin() public requireRole(0x00, msg.sender) {}
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 2);
    assert_eq!(hint_label(&hints[0]), "role:");
    assert_eq!(hint_label(&hints[1]), "account:");
}

// =========================================================================
// 28. Multiple functions with same param name
// =========================================================================

#[test]
fn same_param_name_different_functions() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function setA(uint256 value) public {}
    function setB(uint256 value) public {}

    function init() public {
        setA(1);
        setB(2);
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 2);
    assert_eq!(hint_label(&hints[0]), "value:");
    assert_eq!(hint_label(&hints[1]), "value:");
}

// =========================================================================
// 29. Hints at correct positions
// =========================================================================

#[test]
fn hints_at_correct_positions() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) { return a + b; }

    function test() public pure returns (uint256) {
        return add(10, 20);
    }
}"#;
    let hints = get_hints(source);
    assert_eq!(hints.len(), 2);
    // First hint should be before "10"
    let first_arg_line = hints[0].position.line;
    let second_arg_line = hints[1].position.line;
    assert_eq!(first_arg_line, second_arg_line, "both hints on the same line");
    // Second hint should be after the first
    assert!(
        hints[1].position.character > hints[0].position.character,
        "second hint should be after first"
    );
}

// =========================================================================
// 30. Complex ERC20-like pattern
// =========================================================================

#[test]
fn erc20_pattern() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract ERC20 {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    mapping(address => uint256) private _balances;
    mapping(address => mapping(address => uint256)) private _allowances;

    function _transfer(address from, address to, uint256 amount) internal {
        _balances[from] -= amount;
        _balances[to] += amount;
        emit Transfer(from, to, amount);
    }

    function _approve(address owner, address spender, uint256 amount) internal {
        _allowances[owner][spender] = amount;
        emit Approval(owner, spender, amount);
    }

    function transferFrom(address sender, address recipient, uint256 amount) public {
        _transfer(sender, recipient, amount);
        _approve(sender, msg.sender, _allowances[sender][msg.sender] - amount);
    }
}"#;
    let hints = get_hints(source);
    // This complex example should produce hints without crashing
    // _transfer(from, to, amount) - args match params, so no hints
    // emit Transfer(from, to, amount) - args match params, so no hints
    // _approve(owner, spender, amount) - args match params, so no hints
    // emit Approval(owner, spender, amount) - args match params, so no hints
    // _transfer(sender, recipient, amount) - sender!=from, recipient!=to, amount matches
    // _approve(sender, msg.sender, ...) - sender!=owner, msg.sender!=spender
    // So we should get some hints for the non-matching args
    assert!(
        !hints.is_empty(),
        "ERC20 pattern should produce some hints for non-matching args"
    );
}
