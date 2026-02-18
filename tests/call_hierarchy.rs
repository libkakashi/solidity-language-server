use std::path::PathBuf;

use solidity_language_server::call_hierarchy;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::{Position, SymbolKind};

/// Setup with on-disk files so that `incoming_calls` / `outgoing_calls` can
/// read source from disk when they need to.
fn setup_on_disk(source: &str) -> (SymbolTable, PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.sol");
    std::fs::write(&path, source).unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path, tmp)
}

/// Helper: find the byte offset of `needle` in `source` and convert to an LSP
/// `Position` (0-based line, 0-based character).
fn pos_of(source: &str, needle: &str) -> Position {
    let byte = source.find(needle).unwrap_or_else(|| {
        panic!("needle {:?} not found in source", needle);
    });
    let line = source[..byte].matches('\n').count() as u32;
    let col = (byte - source[..byte].rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32;
    Position::new(line, col)
}

/// Helper: find the byte offset of the *n*-th (1-based) occurrence of `needle`.
fn pos_of_nth(source: &str, needle: &str, n: usize) -> Position {
    assert!(n >= 1, "n must be >= 1");
    let mut start = 0;
    for _ in 0..n - 1 {
        let idx = source[start..].find(needle).unwrap_or_else(|| {
            panic!("fewer than {} occurrences of {:?}", n, needle);
        });
        start += idx + needle.len();
    }
    let byte = start
        + source[start..].find(needle).unwrap_or_else(|| {
            panic!("fewer than {} occurrences of {:?}", n, needle);
        });
    let line = source[..byte].matches('\n').count() as u32;
    let col = (byte - source[..byte].rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32;
    Position::new(line, col)
}

// =========================================================================
// 1. Prepare on function
// =========================================================================

#[test]
fn prepare_on_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure returns (uint256) {
        return 42;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "bar");

    let result = call_hierarchy::prepare(&st, &path, source, pos, &li);
    assert!(
        result.is_some(),
        "prepare should return Some for a function"
    );
    let items = result.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "bar");
    assert_eq!(items[0].kind, SymbolKind::FUNCTION);
}

// =========================================================================
// 2. Prepare on modifier
// =========================================================================

#[test]
fn prepare_on_modifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Auth {
    modifier onlyOwner() {
        _;
    }

    function withdraw() public onlyOwner {}
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "onlyOwner");

    let result = call_hierarchy::prepare(&st, &path, source, pos, &li);
    assert!(
        result.is_some(),
        "prepare should return Some for a modifier"
    );
    let items = result.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "onlyOwner");
}

// =========================================================================
// 3. Prepare on constructor
// =========================================================================

#[test]
fn prepare_on_constructor() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public supply;
    constructor(uint256 _supply) {
        supply = _supply;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    // "constructor" keyword is the name for the constructor declaration
    let pos = pos_of(source, "constructor");

    let result = call_hierarchy::prepare(&st, &path, source, pos, &li);
    assert!(
        result.is_some(),
        "prepare should return Some for a constructor"
    );
    let items = result.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, SymbolKind::CONSTRUCTOR);
}

// =========================================================================
// 4. Prepare on non-callable (state variable) -> None
// =========================================================================

#[test]
fn prepare_on_state_variable_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public value;
    function set(uint256 v) public {
        value = v;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "value");

    let result = call_hierarchy::prepare(&st, &path, source, pos, &li);
    assert!(
        result.is_none(),
        "prepare should return None for a state variable"
    );
}

// =========================================================================
// 5. Prepare on whitespace -> None
// =========================================================================

#[test]
fn prepare_on_whitespace_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public {}
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    // Line 0, char 0 is the comment — but let's pick a position in a blank line
    // Line 2 is the blank line between pragma and contract
    let pos = Position::new(2, 0);

    let result = call_hierarchy::prepare(&st, &path, source, pos, &li);
    assert!(
        result.is_none(),
        "prepare should return None for whitespace"
    );
}

// =========================================================================
// 6. Incoming calls - single caller
// =========================================================================

#[test]
fn incoming_calls_single_caller() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function helper() internal pure returns (uint256) {
        return 1;
    }

    function caller() public pure returns (uint256) {
        return helper();
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "helper");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let incoming = call_hierarchy::incoming_calls(&st, &items[0]);

    assert_eq!(
        incoming.len(),
        1,
        "helper should have exactly 1 incoming call"
    );
    assert_eq!(incoming[0].from.name, "caller");
    assert_eq!(incoming[0].from_ranges.len(), 1);
}

// =========================================================================
// 7. Incoming calls - multiple callers
// =========================================================================

#[test]
fn incoming_calls_multiple_callers() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function helper() internal pure returns (uint256) {
        return 1;
    }

    function callerA() public pure returns (uint256) {
        return helper();
    }

    function callerB() public pure returns (uint256) {
        return helper();
    }

    function callerC() public pure returns (uint256) {
        return helper();
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "helper");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let incoming = call_hierarchy::incoming_calls(&st, &items[0]);

    assert_eq!(
        incoming.len(),
        3,
        "helper should have exactly 3 incoming callers"
    );
    let caller_names: Vec<&str> = incoming.iter().map(|c| c.from.name.as_str()).collect();
    assert!(caller_names.contains(&"callerA"));
    assert!(caller_names.contains(&"callerB"));
    assert!(caller_names.contains(&"callerC"));
}

// =========================================================================
// 8. Incoming calls - no callers
// =========================================================================

#[test]
fn incoming_calls_no_callers() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function unused() internal pure returns (uint256) {
        return 42;
    }

    function other() public pure returns (uint256) {
        return 99;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "unused");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let incoming = call_hierarchy::incoming_calls(&st, &items[0]);

    assert!(
        incoming.is_empty(),
        "unused function should have no incoming calls"
    );
}

// =========================================================================
// 9. Outgoing calls - single callee
// =========================================================================

#[test]
fn outgoing_calls_single_callee() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function helper() internal pure returns (uint256) {
        return 1;
    }

    function main() public pure returns (uint256) {
        return helper();
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    // Prepare on "main" (the second occurrence of "main" is in the function name)
    let pos = pos_of_nth(source, "main", 1);

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let outgoing = call_hierarchy::outgoing_calls(&st, &items[0]);

    assert_eq!(
        outgoing.len(),
        1,
        "main should have exactly 1 outgoing call"
    );
    assert_eq!(outgoing[0].to.name, "helper");
    assert_eq!(outgoing[0].from_ranges.len(), 1);
}

// =========================================================================
// 10. Outgoing calls - multiple callees
// =========================================================================

#[test]
fn outgoing_calls_multiple_callees() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }

    function mul(uint256 a, uint256 b) internal pure returns (uint256) {
        return a * b;
    }

    function sub(uint256 a, uint256 b) internal pure returns (uint256) {
        return a - b;
    }

    function compute(uint256 x, uint256 y) public pure returns (uint256) {
        uint256 s = add(x, y);
        uint256 p = mul(x, y);
        uint256 d = sub(x, y);
        return s + p + d;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "compute");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let outgoing = call_hierarchy::outgoing_calls(&st, &items[0]);

    assert_eq!(
        outgoing.len(),
        3,
        "compute should have 3 outgoing calls (add, mul, sub)"
    );
    let callee_names: Vec<&str> = outgoing.iter().map(|c| c.to.name.as_str()).collect();
    assert!(callee_names.contains(&"add"));
    assert!(callee_names.contains(&"mul"));
    assert!(callee_names.contains(&"sub"));
}

// =========================================================================
// 11. Outgoing calls - no callees
// =========================================================================

#[test]
fn outgoing_calls_no_callees() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function leaf() public pure returns (uint256) {
        return 42;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "leaf");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let outgoing = call_hierarchy::outgoing_calls(&st, &items[0]);

    assert!(
        outgoing.is_empty(),
        "leaf function should have no outgoing calls"
    );
}

// =========================================================================
// 12. Recursive function - function calls itself
// =========================================================================

#[test]
fn recursive_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function factorial(uint256 n) internal pure returns (uint256) {
        if (n <= 1) return 1;
        return n * factorial(n - 1);
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "factorial");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();

    // Outgoing: factorial calls itself
    let outgoing = call_hierarchy::outgoing_calls(&st, &items[0]);
    assert_eq!(
        outgoing.len(),
        1,
        "recursive factorial should have 1 outgoing call (itself)"
    );
    assert_eq!(outgoing[0].to.name, "factorial");

    // Incoming: factorial is called by itself
    let incoming = call_hierarchy::incoming_calls(&st, &items[0]);
    assert_eq!(
        incoming.len(),
        1,
        "recursive factorial should have 1 incoming caller (itself)"
    );
    assert_eq!(incoming[0].from.name, "factorial");
}

// =========================================================================
// 13. Mutual recursion - two functions calling each other
// =========================================================================

#[test]
fn mutual_recursion() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function isEven(uint256 n) internal pure returns (bool) {
        if (n == 0) return true;
        return isOdd(n - 1);
    }

    function isOdd(uint256 n) internal pure returns (bool) {
        if (n == 0) return false;
        return isEven(n - 1);
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // Check isEven
    let pos_even = pos_of(source, "isEven");
    let even_items = call_hierarchy::prepare(&st, &path, source, pos_even, &li).unwrap();

    let even_outgoing = call_hierarchy::outgoing_calls(&st, &even_items[0]);
    assert_eq!(even_outgoing.len(), 1);
    assert_eq!(even_outgoing[0].to.name, "isOdd");

    let even_incoming = call_hierarchy::incoming_calls(&st, &even_items[0]);
    assert_eq!(even_incoming.len(), 1);
    assert_eq!(even_incoming[0].from.name, "isOdd");

    // Check isOdd
    let pos_odd = pos_of(source, "isOdd");
    let odd_items = call_hierarchy::prepare(&st, &path, source, pos_odd, &li).unwrap();

    let odd_outgoing = call_hierarchy::outgoing_calls(&st, &odd_items[0]);
    assert_eq!(odd_outgoing.len(), 1);
    assert_eq!(odd_outgoing[0].to.name, "isEven");

    let odd_incoming = call_hierarchy::incoming_calls(&st, &odd_items[0]);
    assert_eq!(odd_incoming.len(), 1);
    assert_eq!(odd_incoming[0].from.name, "isEven");
}

// =========================================================================
// 14. Cross-function call chain: A calls B calls C
// =========================================================================

#[test]
fn cross_function_call_chain() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function fnC() internal pure returns (uint256) {
        return 1;
    }

    function fnB() internal pure returns (uint256) {
        return fnC();
    }

    function fnA() public pure returns (uint256) {
        return fnB();
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // fnB: should have incoming from fnA and outgoing to fnC
    let pos_b = pos_of(source, "fnB");
    let items_b = call_hierarchy::prepare(&st, &path, source, pos_b, &li).unwrap();

    let b_incoming = call_hierarchy::incoming_calls(&st, &items_b[0]);
    assert_eq!(b_incoming.len(), 1, "fnB should have 1 incoming caller");
    assert_eq!(b_incoming[0].from.name, "fnA");

    let b_outgoing = call_hierarchy::outgoing_calls(&st, &items_b[0]);
    assert_eq!(b_outgoing.len(), 1, "fnB should have 1 outgoing call");
    assert_eq!(b_outgoing[0].to.name, "fnC");

    // fnC: incoming from fnB only, no outgoing
    let pos_c = pos_of(source, "fnC");
    let items_c = call_hierarchy::prepare(&st, &path, source, pos_c, &li).unwrap();

    let c_incoming = call_hierarchy::incoming_calls(&st, &items_c[0]);
    assert_eq!(c_incoming.len(), 1);
    assert_eq!(c_incoming[0].from.name, "fnB");

    let c_outgoing = call_hierarchy::outgoing_calls(&st, &items_c[0]);
    assert!(c_outgoing.is_empty(), "fnC should have no outgoing calls");

    // fnA: no incoming, outgoing to fnB
    let pos_a = pos_of(source, "fnA");
    let items_a = call_hierarchy::prepare(&st, &path, source, pos_a, &li).unwrap();

    let a_incoming = call_hierarchy::incoming_calls(&st, &items_a[0]);
    assert!(a_incoming.is_empty(), "fnA should have no incoming calls");

    let a_outgoing = call_hierarchy::outgoing_calls(&st, &items_a[0]);
    assert_eq!(a_outgoing.len(), 1);
    assert_eq!(a_outgoing[0].to.name, "fnB");
}

// =========================================================================
// 15. Modifier calls - function using a modifier
// =========================================================================

#[test]
fn modifier_in_call_hierarchy() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Auth {
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function withdraw() public onlyOwner {
    }

    function pause() public onlyOwner {
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "onlyOwner");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    assert_eq!(items[0].name, "onlyOwner");

    let incoming = call_hierarchy::incoming_calls(&st, &items[0]);
    // withdraw and pause both use onlyOwner
    assert_eq!(
        incoming.len(),
        2,
        "onlyOwner should have 2 incoming callers (withdraw, pause)"
    );
    let caller_names: Vec<&str> = incoming.iter().map(|c| c.from.name.as_str()).collect();
    assert!(caller_names.contains(&"withdraw"));
    assert!(caller_names.contains(&"pause"));
}

// =========================================================================
// 16. Event emit as reference
// =========================================================================

#[test]
fn event_emit_in_outgoing_calls() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ledger {
    event Transfer(address indexed from, address indexed to, uint256 amount);

    function send(address to, uint256 amount) external {
        emit Transfer(msg.sender, to, amount);
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "send");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();

    // Events are not callable (DeclKind::Event is not in is_callable), so they
    // should NOT appear in outgoing calls. This is a design-verification test.
    let outgoing = call_hierarchy::outgoing_calls(&st, &items[0]);
    let has_event = outgoing.iter().any(|c| c.to.name == "Transfer");
    // Events are intentionally excluded from call hierarchy since they are not callable.
    assert!(
        !has_event,
        "Events should not appear as outgoing callable targets"
    );
}

// =========================================================================
// 17. Constructor calls (new Contract())
// =========================================================================

#[test]
fn constructor_calls_via_new() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Child {
    uint256 public val;
    constructor(uint256 v) {
        val = v;
    }
}

contract Factory {
    function create() public returns (Child) {
        Child c = new Child(42);
        return c;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "constructor");

    let result = call_hierarchy::prepare(&st, &path, source, pos, &li);
    assert!(result.is_some(), "prepare should work on constructor");
    let items = result.unwrap();
    assert_eq!(items[0].kind, SymbolKind::CONSTRUCTOR);
}

// =========================================================================
// 18. Inherited function - calling inherited function
// =========================================================================

#[test]
fn inherited_function_call() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseFn() internal pure returns (uint256) {
        return 10;
    }
}

contract Derived is Base {
    function derivedFn() public pure returns (uint256) {
        return baseFn();
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // Prepare on baseFn
    let pos = pos_of(source, "baseFn");
    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    assert_eq!(items[0].name, "baseFn");

    // baseFn should have an incoming call from derivedFn
    let incoming = call_hierarchy::incoming_calls(&st, &items[0]);
    assert_eq!(
        incoming.len(),
        1,
        "baseFn should have 1 incoming caller from Derived"
    );
    assert_eq!(incoming[0].from.name, "derivedFn");

    // derivedFn should have an outgoing call to baseFn
    let pos_derived = pos_of(source, "derivedFn");
    let derived_items = call_hierarchy::prepare(&st, &path, source, pos_derived, &li).unwrap();
    let outgoing = call_hierarchy::outgoing_calls(&st, &derived_items[0]);
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].to.name, "baseFn");
}

// =========================================================================
// 19. Multiple calls to same function - fromRanges multiple entries
// =========================================================================

#[test]
fn multiple_calls_to_same_function() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function helper() internal pure returns (uint256) {
        return 1;
    }

    function caller() public pure returns (uint256) {
        uint256 a = helper();
        uint256 b = helper();
        uint256 c = helper();
        return a + b + c;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // Check outgoing from caller: should be 1 callee (helper) with 3 from_ranges
    let pos_caller = pos_of(source, "caller");
    let caller_items = call_hierarchy::prepare(&st, &path, source, pos_caller, &li).unwrap();
    let outgoing = call_hierarchy::outgoing_calls(&st, &caller_items[0]);
    assert_eq!(
        outgoing.len(),
        1,
        "caller should have 1 unique outgoing callee"
    );
    assert_eq!(outgoing[0].to.name, "helper");
    assert_eq!(
        outgoing[0].from_ranges.len(),
        3,
        "from_ranges should have 3 entries for 3 calls to helper"
    );

    // Check incoming to helper: should be 1 caller with 3 from_ranges
    let pos_helper = pos_of(source, "helper");
    let helper_items = call_hierarchy::prepare(&st, &path, source, pos_helper, &li).unwrap();
    let incoming = call_hierarchy::incoming_calls(&st, &helper_items[0]);
    assert_eq!(incoming.len(), 1, "helper should have 1 unique caller");
    assert_eq!(incoming[0].from.name, "caller");
    assert_eq!(
        incoming[0].from_ranges.len(),
        3,
        "incoming from_ranges should have 3 entries"
    );
}

// =========================================================================
// 20. Prepare item has correct name and kind
// =========================================================================

#[test]
fn prepare_item_has_correct_name_and_kind() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function myFunction(uint256 x) public pure returns (uint256) {
        return x;
    }

    modifier myModifier() {
        _;
    }

    constructor() {}
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // Function
    let pos_fn = pos_of(source, "myFunction");
    let fn_items = call_hierarchy::prepare(&st, &path, source, pos_fn, &li).unwrap();
    assert_eq!(fn_items[0].name, "myFunction");
    assert_eq!(fn_items[0].kind, SymbolKind::FUNCTION);

    // Modifier
    let pos_mod = pos_of(source, "myModifier");
    let mod_items = call_hierarchy::prepare(&st, &path, source, pos_mod, &li).unwrap();
    assert_eq!(mod_items[0].name, "myModifier");
    // Modifiers are mapped to SymbolKind::FUNCTION in call_hierarchy.rs
    assert_eq!(mod_items[0].kind, SymbolKind::FUNCTION);

    // Constructor
    let pos_ctor = pos_of(source, "constructor");
    let ctor_items = call_hierarchy::prepare(&st, &path, source, pos_ctor, &li).unwrap();
    assert_eq!(ctor_items[0].kind, SymbolKind::CONSTRUCTOR);
}

// =========================================================================
// 21. Prepare on event returns None (events are not callable)
// =========================================================================

#[test]
fn prepare_on_event_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    event Deposited(address user, uint256 amount);

    function deposit() public {
        emit Deposited(msg.sender, 100);
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "Deposited");

    let result = call_hierarchy::prepare(&st, &path, source, pos, &li);
    assert!(
        result.is_none(),
        "prepare should return None for an event (not callable)"
    );
}

// =========================================================================
// 22. Prepare on error definition returns None
// =========================================================================

#[test]
fn prepare_on_error_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    error Unauthorized(address caller);

    function restricted() public view {
        if (msg.sender != address(0)) revert Unauthorized(msg.sender);
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "Unauthorized");

    let result = call_hierarchy::prepare(&st, &path, source, pos, &li);
    assert!(
        result.is_none(),
        "prepare should return None for a custom error (not callable)"
    );
}

// =========================================================================
// 23. Free function calls across contracts
// =========================================================================

#[test]
fn free_function_call_hierarchy() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

function freeHelper() pure returns (uint256) {
    return 42;
}

contract A {
    function useHelper() public pure returns (uint256) {
        return freeHelper();
    }
}

contract B {
    function alsoUseHelper() public pure returns (uint256) {
        return freeHelper();
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "freeHelper");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    assert_eq!(items[0].name, "freeHelper");

    let incoming = call_hierarchy::incoming_calls(&st, &items[0]);
    assert_eq!(
        incoming.len(),
        2,
        "freeHelper should be called by 2 functions"
    );
    let caller_names: Vec<&str> = incoming.iter().map(|c| c.from.name.as_str()).collect();
    assert!(caller_names.contains(&"useHelper"));
    assert!(caller_names.contains(&"alsoUseHelper"));
}

// =========================================================================
// 24. Library function call hierarchy via qualified calls
// =========================================================================

#[test]
fn library_function_call_hierarchy() {
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
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "add");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    assert_eq!(items[0].name, "add");

    let incoming = call_hierarchy::incoming_calls(&st, &items[0]);
    assert_eq!(
        incoming.len(),
        1,
        "add should have 1 incoming caller (compute)"
    );
    assert_eq!(incoming[0].from.name, "compute");
}

// =========================================================================
// 25. Prepare item URI matches file path
// =========================================================================

#[test]
fn prepare_item_uri_matches_file() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure returns (uint256) {
        return 1;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "bar");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let item_path = items[0].uri.to_file_path().unwrap();
    assert_eq!(
        item_path, path,
        "Item URI should resolve to the original file path"
    );
}

// =========================================================================
// 26. Incoming and outgoing calls on a function with mixed callables
// =========================================================================

#[test]
fn mixed_callable_outgoing() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function helperA() internal pure returns (uint256) {
        return 1;
    }

    function helperB() internal pure returns (uint256) {
        return 2;
    }

    function orchestrate() public pure returns (uint256) {
        return helperA() + helperB();
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "orchestrate");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let outgoing = call_hierarchy::outgoing_calls(&st, &items[0]);

    assert_eq!(outgoing.len(), 2, "orchestrate should call 2 functions");
    let callee_names: Vec<&str> = outgoing.iter().map(|c| c.to.name.as_str()).collect();
    assert!(callee_names.contains(&"helperA"));
    assert!(callee_names.contains(&"helperB"));
}

// =========================================================================
// 27. Prepare on struct returns None
// =========================================================================

#[test]
fn prepare_on_struct_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }

    function bar() public {}
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "Point");

    let result = call_hierarchy::prepare(&st, &path, source, pos, &li);
    assert!(result.is_none(), "prepare should return None for a struct");
}

// =========================================================================
// 28. Prepare on enum returns None
// =========================================================================

#[test]
fn prepare_on_enum_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    enum Status { Active, Paused }

    function bar() public {}
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "Status");

    let result = call_hierarchy::prepare(&st, &path, source, pos, &li);
    assert!(result.is_none(), "prepare should return None for an enum");
}

// =========================================================================
// 29. Deeply nested calls - outgoing from function calling many nested levels
// =========================================================================

#[test]
fn deeply_nested_outgoing_calls() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function innerA() internal pure returns (uint256) {
        return 1;
    }

    function innerB() internal pure returns (uint256) {
        return 2;
    }

    function outer() public pure returns (uint256) {
        return innerA() + innerB() + innerA();
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "outer");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let outgoing = call_hierarchy::outgoing_calls(&st, &items[0]);

    // Should be 2 unique callees (innerA and innerB)
    assert_eq!(
        outgoing.len(),
        2,
        "outer should have 2 unique outgoing callees"
    );

    let inner_a = outgoing.iter().find(|c| c.to.name == "innerA").unwrap();
    assert_eq!(
        inner_a.from_ranges.len(),
        2,
        "innerA should be called 2 times from outer"
    );

    let inner_b = outgoing.iter().find(|c| c.to.name == "innerB").unwrap();
    assert_eq!(
        inner_b.from_ranges.len(),
        1,
        "innerB should be called 1 time from outer"
    );
}

// =========================================================================
// 30. Selection range matches the function name position
// =========================================================================

#[test]
fn selection_range_matches_function_name() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function myFunc() public pure returns (uint256) {
        return 1;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "myFunc");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let item = &items[0];

    // The selection_range should cover exactly the function name "myFunc"
    let name_start = source.find("myFunc").unwrap();
    let name_end = name_start + "myFunc".len();
    let expected_start_line = source[..name_start].matches('\n').count() as u32;
    let expected_start_col =
        (name_start - source[..name_start].rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32;
    let expected_end_line = source[..name_end].matches('\n').count() as u32;
    let expected_end_col =
        (name_end - source[..name_end].rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32;

    assert_eq!(item.selection_range.start.line, expected_start_line);
    assert_eq!(item.selection_range.start.character, expected_start_col);
    assert_eq!(item.selection_range.end.line, expected_end_line);
    assert_eq!(item.selection_range.end.character, expected_end_col);
}

// =========================================================================
// 31. Prepare with function parameters includes detail
// =========================================================================

#[test]
fn prepare_item_detail_with_parameters() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function transfer(address to, uint256 amount) public {
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "transfer");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let item = &items[0];

    // detail should contain the parameters
    assert!(
        item.detail.is_some(),
        "detail should be present for a function with parameters"
    );
    let detail = item.detail.as_ref().unwrap();
    assert!(
        detail.contains("address"),
        "detail should contain 'address' type, got: {}",
        detail
    );
    assert!(
        detail.contains("uint256"),
        "detail should contain 'uint256' type, got: {}",
        detail
    );
}

// =========================================================================
// 32. Prepare returns single-element vector
// =========================================================================

#[test]
fn prepare_returns_single_element_vector() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function alpha() public {}
    function beta() public {}
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = pos_of(source, "alpha");
    let result = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    assert_eq!(
        result.len(),
        1,
        "prepare should always return exactly 1 item"
    );

    let pos2 = pos_of(source, "beta");
    let result2 = call_hierarchy::prepare(&st, &path, source, pos2, &li).unwrap();
    assert_eq!(result2.len(), 1);
}

// =========================================================================
// BUG TEST: from_ranges should cover the full callee name, not just 1 char
// =========================================================================

#[test]
fn incoming_call_from_ranges_cover_full_name() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function helperFunction() internal pure returns (uint256) {
        return 1;
    }

    function caller() public pure returns (uint256) {
        return helperFunction();
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);
    let pos = pos_of(source, "helperFunction");

    let items = call_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let incoming = call_hierarchy::incoming_calls(&st, &items[0]);

    assert_eq!(incoming.len(), 1);
    let range = &incoming[0].from_ranges[0];
    // The range should cover "helperFunction" (14 characters), not just 1
    let range_len = range.end.character - range.start.character;
    assert!(
        range_len >= "helperFunction".len() as u32,
        "from_ranges should cover the full function name ({} chars), but range only covers {} chars ({}:{} to {}:{})",
        "helperFunction".len(),
        range_len,
        range.start.line, range.start.character,
        range.end.line, range.end.character
    );
}
