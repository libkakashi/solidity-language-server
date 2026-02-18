use solidity_language_server::parser::TsParser;
use solidity_language_server::selection_ranges::selection_ranges;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::{Position, SelectionRange};

fn get_selection(source: &str, line: u32, character: u32) -> SelectionRange {
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let li = LineIndex::new(source);
    let pos = Position { line, character };
    let results = selection_ranges(source, &[pos], &li, Some(&tree));
    results.into_iter().next().unwrap()
}

fn get_selections(source: &str, positions: &[(u32, u32)]) -> Vec<SelectionRange> {
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let li = LineIndex::new(source);
    let pos_vec: Vec<Position> = positions
        .iter()
        .map(|(l, c)| Position {
            line: *l,
            character: *c,
        })
        .collect();
    selection_ranges(source, &pos_vec, &li, Some(&tree))
}

/// Find the byte offset of `needle` and convert to (line, character).
fn lc_of(source: &str, needle: &str) -> (u32, u32) {
    let byte = source
        .find(needle)
        .unwrap_or_else(|| panic!("needle {:?} not found in source", needle));
    let line = source[..byte].matches('\n').count() as u32;
    let col = (byte - source[..byte].rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32;
    (line, col)
}

/// Walk the parent chain and return the depth (number of levels including the root).
fn chain_depth(sel: &SelectionRange) -> usize {
    let mut depth = 1;
    let mut current = sel;
    while let Some(ref parent) = current.parent {
        depth += 1;
        current = parent;
    }
    depth
}

/// Collect all ranges in the chain from innermost to outermost.
fn collect_chain(sel: &SelectionRange) -> Vec<tower_lsp::lsp_types::Range> {
    let mut ranges = vec![sel.range];
    let mut current = sel;
    while let Some(ref parent) = current.parent {
        ranges.push(parent.range);
        current = parent;
    }
    ranges
}

// =========================================================================
// 1. Basic parent chain - variable inside function inside contract
// =========================================================================

#[test]
fn variable_inside_function_has_deep_chain() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    function bar() public {
        uint256 x = 1;
    }
}"#;
    let (line, col) = lc_of(source, "x =");
    let sel = get_selection(source, line, col);
    let depth = chain_depth(&sel);
    // x -> declaration -> block -> function_body -> function -> contract_body -> contract -> source_file
    assert!(
        depth >= 4,
        "expected at least 4 levels for variable inside function, got {}",
        depth
    );
}

// =========================================================================
// 2. Innermost range covers the cursor position
// =========================================================================

#[test]
fn innermost_range_covers_cursor() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    uint256 value;
}"#;
    let (line, col) = lc_of(source, "value");
    let sel = get_selection(source, line, col);
    assert_eq!(sel.range.start.line, line);
    assert!(sel.range.start.character <= col);
    assert!(sel.range.end.character >= col);
}

// =========================================================================
// 3. No duplicate ranges in the chain
// =========================================================================

#[test]
fn no_duplicate_ranges_in_chain() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    function bar() public {
        uint256 x = 1;
    }
}"#;
    let (line, col) = lc_of(source, "x =");
    let sel = get_selection(source, line, col);
    let chain = collect_chain(&sel);
    for w in chain.windows(2) {
        assert_ne!(w[0], w[1], "duplicate range in selection chain");
    }
}

// =========================================================================
// 4. Each parent range contains the child range
// =========================================================================

#[test]
fn parent_range_contains_child_range() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    function baz(uint256 a, uint256 b) public pure returns (uint256) {
        if (a > b) {
            return a - b;
        }
        return b - a;
    }
}"#;
    let (line, col) = lc_of(source, "a - b");
    let sel = get_selection(source, line, col);
    let chain = collect_chain(&sel);
    for w in chain.windows(2) {
        let child = &w[0];
        let parent = &w[1];
        assert!(
            parent.start.line < child.start.line
                || (parent.start.line == child.start.line
                    && parent.start.character <= child.start.character),
            "parent start should be <= child start: parent={:?} child={:?}",
            parent,
            child
        );
        assert!(
            parent.end.line > child.end.line
                || (parent.end.line == child.end.line
                    && parent.end.character >= child.end.character),
            "parent end should be >= child end: parent={:?} child={:?}",
            parent,
            child
        );
    }
}

// =========================================================================
// 5. Outermost range covers the whole file
// =========================================================================

#[test]
fn outermost_range_covers_file() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    uint256 x;
}"#;
    let (line, col) = lc_of(source, "x;");
    let sel = get_selection(source, line, col);
    let chain = collect_chain(&sel);
    let outermost = chain.last().unwrap();
    assert_eq!(outermost.start.line, 0);
    assert_eq!(outermost.start.character, 0);
}

// =========================================================================
// 6. Multiple positions return independent results
// =========================================================================

#[test]
fn multiple_positions_independent() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    uint256 alpha;
    uint256 beta;
}"#;
    let (l1, c1) = lc_of(source, "alpha");
    let (l2, c2) = lc_of(source, "beta");
    let results = get_selections(source, &[(l1, c1), (l2, c2)]);
    assert_eq!(results.len(), 2);
    // Each should have different innermost ranges
    assert_ne!(
        results[0].range, results[1].range,
        "different positions should produce different innermost ranges"
    );
}

// =========================================================================
// 7. Nested if/else blocks
// =========================================================================

#[test]
fn nested_if_else_blocks() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    function check(uint256 x) public pure returns (string memory) {
        if (x > 100) {
            if (x > 200) {
                return "very high";
            } else {
                return "high";
            }
        } else {
            return "low";
        }
    }
}"#;
    let (line, col) = lc_of(source, "very high");
    let sel = get_selection(source, line, col);
    let depth = chain_depth(&sel);
    // "very high" -> string -> return -> block -> if body -> if -> block -> if -> block -> function body -> function -> contract body -> contract -> source
    assert!(
        depth >= 6,
        "deeply nested should have at least 6 levels, got {}",
        depth
    );
}

// =========================================================================
// 8. For loop body
// =========================================================================

#[test]
fn for_loop_body() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    function sum(uint256[] memory arr) public pure returns (uint256) {
        uint256 total = 0;
        for (uint256 i = 0; i < arr.length; i++) {
            total += arr[i];
        }
        return total;
    }
}"#;
    let (line, col) = lc_of(source, "total += arr");
    let sel = get_selection(source, line, col);
    let depth = chain_depth(&sel);
    assert!(depth >= 4, "for loop body should have at least 4 levels, got {}", depth);
    // Should have parent chain
    assert!(sel.parent.is_some());
}

// =========================================================================
// 9. Struct definition
// =========================================================================

#[test]
fn struct_field_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Store {
    struct Item {
        uint256 id;
        string name;
        uint256 price;
    }
}"#;
    let (line, col) = lc_of(source, "price");
    let sel = get_selection(source, line, col);
    let chain = collect_chain(&sel);
    // Should go from field -> struct body -> struct -> contract body -> contract -> source
    assert!(chain.len() >= 4, "struct field should have at least 4 levels");
}

// =========================================================================
// 10. Event definition
// =========================================================================

#[test]
fn event_parameter_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Token {
    event Transfer(address indexed from, address indexed to, uint256 value);
}"#;
    let (line, col) = lc_of(source, "value");
    let sel = get_selection(source, line, col);
    assert!(sel.parent.is_some(), "event parameter should have parent");
}

// =========================================================================
// 11. Empty function body
// =========================================================================

#[test]
fn empty_function_body() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    function noop() public {}
}"#;
    // Position inside the empty body braces
    let open_brace = source.find("{}").unwrap();
    let li = LineIndex::new(source);
    let (line, col) = li.byte_offset_to_position(source, open_brace + 1);
    let sel = get_selection(source, line, col);
    // Should not panic, should have some chain
    assert!(chain_depth(&sel) >= 2);
}

// =========================================================================
// 12. Modifier body
// =========================================================================

#[test]
fn modifier_body_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Access {
    address owner;
    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }
}"#;
    let (line, col) = lc_of(source, "require");
    let sel = get_selection(source, line, col);
    let depth = chain_depth(&sel);
    assert!(depth >= 3, "modifier body should have at least 3 levels");
}

// =========================================================================
// 13. Mapping type
// =========================================================================

#[test]
fn mapping_type_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Token {
    mapping(address => mapping(address => uint256)) public allowances;
}"#;
    let (line, col) = lc_of(source, "allowances");
    let sel = get_selection(source, line, col);
    assert!(sel.parent.is_some());
}

// =========================================================================
// 14. Assembly block
// =========================================================================

#[test]
fn assembly_block_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Util {
    function getSize(address addr) public view returns (uint256 size) {
        assembly {
            size := extcodesize(addr)
        }
    }
}"#;
    let (line, col) = lc_of(source, "extcodesize");
    let sel = get_selection(source, line, col);
    let depth = chain_depth(&sel);
    assert!(depth >= 3, "assembly should have at least 3 levels, got {}", depth);
}

// =========================================================================
// 15. No tree fallback
// =========================================================================

#[test]
fn no_tree_returns_whole_file() {
    let source = "pragma solidity ^0.8.0;\ncontract Foo {}";
    let li = LineIndex::new(source);
    let pos = Position {
        line: 1,
        character: 9,
    };
    let results = selection_ranges(source, &[pos], &li, None);
    assert_eq!(results.len(), 1);
    // Without a tree, should return a default covering the whole file
    assert_eq!(results[0].range.start.line, 0);
    assert_eq!(results[0].range.start.character, 0);
    assert!(results[0].parent.is_none());
}

// =========================================================================
// 16. Multiple cursors in same function
// =========================================================================

#[test]
fn multiple_cursors_same_function() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    function calc() public pure returns (uint256) {
        uint256 a = 1;
        uint256 b = 2;
        return a + b;
    }
}"#;
    let (l1, c1) = lc_of(source, "a = 1");
    let (l2, c2) = lc_of(source, "b = 2");
    let results = get_selections(source, &[(l1, c1), (l2, c2)]);
    assert_eq!(results.len(), 2);
    // Both should have comparable depths since they're at the same nesting level
    let d1 = chain_depth(&results[0]);
    let d2 = chain_depth(&results[1]);
    assert!(
        (d1 as i32 - d2 as i32).abs() <= 1,
        "depths should be similar: {} vs {}",
        d1,
        d2
    );
}

// =========================================================================
// 17. Enum definition
// =========================================================================

#[test]
fn enum_member_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Voting {
    enum Status { Pending, Active, Closed }
}"#;
    let (line, col) = lc_of(source, "Active");
    let sel = get_selection(source, line, col);
    assert!(sel.parent.is_some(), "enum member should have parent");
    let chain = collect_chain(&sel);
    assert!(chain.len() >= 3, "enum member should have at least 3 levels");
}

// =========================================================================
// 18. Try/catch block
// =========================================================================

#[test]
fn try_catch_selection() {
    let source = r#"pragma solidity ^0.8.0;

interface IFoo {
    function bar() external returns (uint256);
}

contract Foo {
    IFoo target;

    function safe() public returns (uint256) {
        try target.bar() returns (uint256 val) {
            return val;
        } catch {
            return 0;
        }
    }
}"#;
    let (line, col) = lc_of(source, "return val");
    let sel = get_selection(source, line, col);
    let depth = chain_depth(&sel);
    assert!(depth >= 4, "try body should have deep chain, got {}", depth);
}

// =========================================================================
// 19. Constructor body
// =========================================================================

#[test]
fn constructor_body_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Token {
    string public name;
    constructor(string memory _name) {
        name = _name;
    }
}"#;
    let (line, col) = lc_of(source, "name = _name");
    let sel = get_selection(source, line, col);
    assert!(sel.parent.is_some());
    let depth = chain_depth(&sel);
    assert!(depth >= 3, "constructor body should have at least 3 levels");
}

// =========================================================================
// 20. Pragma line
// =========================================================================

#[test]
fn pragma_line_selection() {
    let source = "pragma solidity ^0.8.0;\ncontract Foo {}";
    let sel = get_selection(source, 0, 7); // on "solidity"
    let chain = collect_chain(&sel);
    // Should still have a chain (token -> pragma -> source_file)
    assert!(chain.len() >= 2, "pragma should have at least 2 levels");
}

// =========================================================================
// 21. Ternary expression
// =========================================================================

#[test]
fn ternary_expression_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {
    function max(uint256 a, uint256 b) public pure returns (uint256) {
        return a > b ? a : b;
    }
}"#;
    let (line, col) = lc_of(source, "a > b ?");
    let sel = get_selection(source, line, col);
    let depth = chain_depth(&sel);
    assert!(depth >= 3, "ternary should have at least 3 levels, got {}", depth);
}

// =========================================================================
// 22. Import statement
// =========================================================================

#[test]
fn import_statement_selection() {
    let source = r#"pragma solidity ^0.8.0;
import "./other.sol";
contract Foo {}"#;
    let (line, col) = lc_of(source, "other.sol");
    let sel = get_selection(source, line, col);
    assert!(sel.parent.is_some());
}

// =========================================================================
// 23. Error definition
// =========================================================================

#[test]
fn error_definition_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Vault {
    error Unauthorized(address caller);
}"#;
    let (line, col) = lc_of(source, "Unauthorized");
    let sel = get_selection(source, line, col);
    assert!(sel.parent.is_some());
    let chain = collect_chain(&sel);
    assert!(chain.len() >= 3);
}

// =========================================================================
// 24. Receive/fallback functions
// =========================================================================

#[test]
fn receive_function_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Vault {
    receive() external payable {
        // accept ETH
    }
    fallback() external payable {
        // fallback logic
    }
}"#;
    let (line, col) = lc_of(source, "receive");
    let sel = get_selection(source, line, col);
    assert!(sel.parent.is_some());
}

// =========================================================================
// 25. Using-for directive
// =========================================================================

#[test]
fn using_for_selection() {
    let source = r#"pragma solidity ^0.8.0;

library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

contract Token {
    using SafeMath for uint256;
}"#;
    let (line, col) = lc_of(source, "using SafeMath");
    let sel = get_selection(source, line, col);
    assert!(sel.parent.is_some());
}

// =========================================================================
// 26. Deeply nested contract hierarchy
// =========================================================================

#[test]
fn deeply_nested_expressions() {
    let source = r#"pragma solidity ^0.8.0;
contract Math {
    function complex(uint256 a, uint256 b, uint256 c) public pure returns (uint256) {
        return ((a + b) * (c - a)) / ((b + c) * 2);
    }
}"#;
    let (line, col) = lc_of(source, "c - a");
    let sel = get_selection(source, line, col);
    let depth = chain_depth(&sel);
    assert!(
        depth >= 5,
        "deeply nested expression should have at least 5 levels, got {}",
        depth
    );
}

// =========================================================================
// 27. Empty source
// =========================================================================

#[test]
fn empty_source_no_crash() {
    let source = "";
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let li = LineIndex::new(source);
    let results = selection_ranges(
        source,
        &[Position {
            line: 0,
            character: 0,
        }],
        &li,
        Some(&tree),
    );
    assert_eq!(results.len(), 1);
}

// =========================================================================
// 28. Multiple empty positions
// =========================================================================

#[test]
fn multiple_positions_on_same_token() {
    let source = r#"pragma solidity ^0.8.0;
contract Foo {}"#;
    // Two positions on the same word "contract"
    let results = get_selections(source, &[(1, 0), (1, 3)]);
    assert_eq!(results.len(), 2);
    // Both innermost ranges should cover "contract" (or at least overlap)
}

// =========================================================================
// 29. Unchecked block
// =========================================================================

#[test]
fn unchecked_block_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Counter {
    uint256 count;
    function increment() public {
        unchecked {
            count++;
        }
    }
}"#;
    let (line, col) = lc_of(source, "count++");
    let sel = get_selection(source, line, col);
    let depth = chain_depth(&sel);
    assert!(depth >= 4, "unchecked block should add nesting, got {}", depth);
}

// =========================================================================
// 30. While loop body
// =========================================================================

#[test]
fn while_loop_selection() {
    let source = r#"pragma solidity ^0.8.0;
contract Iter {
    function countdown(uint256 n) public pure returns (uint256) {
        uint256 sum = 0;
        while (n > 0) {
            sum += n;
            n--;
        }
        return sum;
    }
}"#;
    let (line, col) = lc_of(source, "sum += n");
    let sel = get_selection(source, line, col);
    let depth = chain_depth(&sel);
    assert!(depth >= 4, "while loop body should have at least 4 levels, got {}", depth);
}
