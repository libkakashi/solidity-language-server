use solidity_language_server::folding_ranges::folding_ranges;
use solidity_language_server::parser::TsParser;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::FoldingRangeKind;

fn get_ranges(source: &str) -> Vec<tower_lsp::lsp_types::FoldingRange> {
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let li = LineIndex::new(source);
    folding_ranges(source, &li, Some(&tree))
}

// ---------------------------------------------------------------------------
// 1. Contract body
// ---------------------------------------------------------------------------

#[test]
fn contract_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract MyContract {
    uint256 public value;
    address public owner;
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    assert!(
        !region_ranges.is_empty(),
        "contract body should produce a foldable region"
    );
    // The contract body starts on the line with `{` (line 3) and ends on `}` (line 6).
    let contract_fold = region_ranges
        .iter()
        .find(|r| r.start_line == 3)
        .expect("should have a fold starting at the contract opening brace line");
    assert_eq!(contract_fold.end_line, 6);
}

// ---------------------------------------------------------------------------
// 2. Interface body
// ---------------------------------------------------------------------------

#[test]
fn interface_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IToken {
    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    assert!(
        region_ranges.iter().any(|r| r.start_line == 3),
        "interface body should be foldable starting at its declaration line"
    );
}

// ---------------------------------------------------------------------------
// 3. Library body
// ---------------------------------------------------------------------------

#[test]
fn library_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Library body fold
    assert!(
        region_ranges.iter().any(|r| r.start_line == 3),
        "library body should be foldable"
    );
}

// ---------------------------------------------------------------------------
// 4. Function body
// ---------------------------------------------------------------------------

#[test]
fn function_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure returns (uint256) {
        uint256 x = 1;
        uint256 y = 2;
        return x + y;
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Should have at least 2 regions: contract body + function body
    assert!(
        region_ranges.len() >= 2,
        "expected at least 2 region folds (contract + function body), got {}",
        region_ranges.len()
    );
    // Function body starts at line 4, ends at line 8
    assert!(
        region_ranges.iter().any(|r| r.start_line == 4),
        "function body should be foldable"
    );
}

// ---------------------------------------------------------------------------
// 5. Constructor body
// ---------------------------------------------------------------------------

#[test]
fn constructor_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public value;

    constructor(uint256 _value) {
        value = _value;
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Constructor body fold should start on line 6
    assert!(
        region_ranges.iter().any(|r| r.start_line == 6),
        "constructor body should be foldable"
    );
}

// ---------------------------------------------------------------------------
// 6. Modifier body
// ---------------------------------------------------------------------------

#[test]
fn modifier_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Modifier body fold should start on line 6
    assert!(
        region_ranges.iter().any(|r| r.start_line == 6),
        "modifier body should be foldable"
    );
}

// ---------------------------------------------------------------------------
// 7. Struct body
// ---------------------------------------------------------------------------

#[test]
fn struct_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct User {
        address addr;
        uint256 balance;
        bool active;
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Struct body fold should start on line 4
    assert!(
        region_ranges.iter().any(|r| r.start_line == 4),
        "struct body should be foldable"
    );
}

// ---------------------------------------------------------------------------
// 8. Enum body
// ---------------------------------------------------------------------------

#[test]
fn enum_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    enum Status {
        Active,
        Inactive,
        Paused
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Enum body fold should start on line 4
    assert!(
        region_ranges.iter().any(|r| r.start_line == 4),
        "enum body should be foldable"
    );
}

// ---------------------------------------------------------------------------
// 9. If-else blocks
// ---------------------------------------------------------------------------

#[test]
fn if_else_blocks_are_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function check(uint256 x) public pure returns (uint256) {
        if (x > 10) {
            return x * 2;
        } else {
            return x;
        }
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Should have folds for: contract body, function body, if block, else block
    // The if block_statement and else block_statement are each foldable
    assert!(
        region_ranges.len() >= 4,
        "expected at least 4 region folds (contract + function + if + else blocks), got {}",
        region_ranges.len()
    );
}

// ---------------------------------------------------------------------------
// 10. For loop body
// ---------------------------------------------------------------------------

#[test]
fn for_loop_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function sum(uint256 n) public pure returns (uint256) {
        uint256 total = 0;
        for (uint256 i = 0; i < n; i++) {
            total += i;
        }
        return total;
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Contract body + function body + for loop block
    assert!(
        region_ranges.len() >= 3,
        "expected at least 3 region folds (contract + function + for loop), got {}",
        region_ranges.len()
    );
}

// ---------------------------------------------------------------------------
// 11. While loop body
// ---------------------------------------------------------------------------

#[test]
fn while_loop_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function countdown(uint256 n) public pure returns (uint256) {
        uint256 count = n;
        while (count > 0) {
            count--;
        }
        return count;
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Contract body + function body + while loop block
    assert!(
        region_ranges.len() >= 3,
        "expected at least 3 region folds (contract + function + while loop), got {}",
        region_ranges.len()
    );
}

// ---------------------------------------------------------------------------
// 12. Multi-line comment
// ---------------------------------------------------------------------------

#[test]
fn multiline_comment_is_foldable_with_comment_kind() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

/*
 * This is a multi-line
 * block comment that
 * should be foldable.
 */
contract Foo {}"#;
    let ranges = get_ranges(source);
    let comment_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Comment))
        .collect();
    assert_eq!(
        comment_ranges.len(),
        1,
        "expected exactly 1 comment folding range, got {}",
        comment_ranges.len()
    );
    assert_eq!(comment_ranges[0].start_line, 3);
    assert_eq!(comment_ranges[0].end_line, 7);
}

// ---------------------------------------------------------------------------
// 13. NatSpec comment
// ---------------------------------------------------------------------------

#[test]
fn natspec_comment_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

/**
 * @title MyContract
 * @notice This is a NatSpec comment
 * @dev Developer documentation
 */
contract Foo {
    uint256 public x;
}"#;
    let ranges = get_ranges(source);
    let comment_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Comment))
        .collect();
    assert!(
        !comment_ranges.is_empty(),
        "NatSpec comment should produce a foldable comment region"
    );
    assert_eq!(comment_ranges[0].start_line, 3);
    assert_eq!(comment_ranges[0].end_line, 7);
}

// ---------------------------------------------------------------------------
// 14. Import groups
// ---------------------------------------------------------------------------

#[test]
fn consecutive_imports_form_foldable_region() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Alpha.sol";
import "./Beta.sol";
import "./Gamma.sol";
import "./Delta.sol";

contract Foo {}"#;
    let ranges = get_ranges(source);
    let import_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Imports))
        .collect();
    assert_eq!(
        import_ranges.len(),
        1,
        "expected exactly 1 imports folding range, got {}",
        import_ranges.len()
    );
    assert_eq!(import_ranges[0].start_line, 3);
    assert_eq!(import_ranges[0].end_line, 6);
}

#[test]
fn single_import_does_not_form_foldable_region() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Alpha.sol";

contract Foo {}"#;
    let ranges = get_ranges(source);
    let import_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Imports))
        .collect();
    assert!(
        import_ranges.is_empty(),
        "a single import should NOT form a foldable imports region"
    );
}

// ---------------------------------------------------------------------------
// 15. Nested functions in contract
// ---------------------------------------------------------------------------

#[test]
fn multiple_functions_each_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function alpha() public pure returns (uint256) {
        return 1;
    }

    function beta() public pure returns (uint256) {
        return 2;
    }

    function gamma() public pure returns (uint256) {
        return 3;
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // 1 contract body + 3 function bodies = at least 4 region folds
    assert!(
        region_ranges.len() >= 4,
        "expected at least 4 region folds (1 contract + 3 functions), got {}",
        region_ranges.len()
    );
}

// ---------------------------------------------------------------------------
// 16. Nested blocks
// ---------------------------------------------------------------------------

#[test]
fn nested_blocks_are_each_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function nested(uint256 x) public pure returns (uint256) {
        if (x > 100) {
            for (uint256 i = 0; i < x; i++) {
                if (i > 50) {
                    x = x + i;
                }
            }
        }
        return x;
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // contract body + function body + outer if block + for block + inner if block = at least 5
    assert!(
        region_ranges.len() >= 5,
        "expected at least 5 region folds for nested blocks, got {}",
        region_ranges.len()
    );
}

// ---------------------------------------------------------------------------
// 17. Empty contract (single-line)
// ---------------------------------------------------------------------------

#[test]
fn empty_contract_no_folding() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Empty {}"#;
    let ranges = get_ranges(source);
    // A single-line contract body `{}` should not produce a fold since start_line == end_line
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    assert!(
        region_ranges.is_empty(),
        "single-line empty contract should NOT produce a folding range"
    );
}

// ---------------------------------------------------------------------------
// 18. Single-line function
// ---------------------------------------------------------------------------

#[test]
fn single_line_function_not_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public x;
    function get() public view returns (uint256) { return x; }
}"#;
    let ranges = get_ranges(source);
    // The contract body IS multi-line so it folds, but the single-line function body should not
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Only the contract body should fold (line 3..6), not the single-line function body
    assert_eq!(
        region_ranges.len(),
        1,
        "only the contract body should be foldable, not the single-line function; got {} folds",
        region_ranges.len()
    );
    assert_eq!(region_ranges[0].start_line, 3);
}

// ---------------------------------------------------------------------------
// 19. Multiple contracts
// ---------------------------------------------------------------------------

#[test]
fn multiple_contracts_each_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Alpha {
    uint256 public a;
}

contract Beta {
    uint256 public b;
}

contract Gamma {
    uint256 public c;
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    assert_eq!(
        region_ranges.len(),
        3,
        "expected exactly 3 contract body folds, got {}",
        region_ranges.len()
    );
    assert_eq!(region_ranges[0].start_line, 3);
    assert_eq!(region_ranges[1].start_line, 7);
    assert_eq!(region_ranges[2].start_line, 11);
}

// ---------------------------------------------------------------------------
// 20. Fallback and receive functions
// ---------------------------------------------------------------------------

#[test]
fn fallback_function_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    fallback() external payable {
        revert();
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Contract body + fallback body
    assert!(
        region_ranges.len() >= 2,
        "expected at least 2 region folds (contract + fallback), got {}",
        region_ranges.len()
    );
}

#[test]
fn receive_function_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    receive() external payable {
        // accept ether
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Contract body + receive body
    assert!(
        region_ranges.len() >= 2,
        "expected at least 2 region folds (contract + receive), got {}",
        region_ranges.len()
    );
}

// ---------------------------------------------------------------------------
// 21. Unchecked blocks
// ---------------------------------------------------------------------------

#[test]
fn unchecked_block_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function unsafeAdd(uint256 a, uint256 b) public pure returns (uint256) {
        unchecked {
            return a + b;
        }
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Contract body + function body + unchecked block
    assert!(
        region_ranges.len() >= 3,
        "expected at least 3 region folds (contract + function + unchecked), got {}",
        region_ranges.len()
    );
}

// ---------------------------------------------------------------------------
// 22. Complex contract with everything
// ---------------------------------------------------------------------------

#[test]
fn complex_contract_with_all_foldable_regions() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./IERC20.sol";
import "./SafeMath.sol";

/**
 * @title CompleteToken
 * @notice A token with all types of foldable regions
 */
contract CompleteToken {
    struct Holder {
        address addr;
        uint256 balance;
    }

    enum Status {
        Active,
        Paused,
        Stopped
    }

    uint256 public totalSupply;
    address public owner;

    event Transfer(
        address indexed from,
        address indexed to,
        uint256 amount
    );

    error InsufficientBalance(
        uint256 available,
        uint256 required
    );

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    constructor(uint256 _supply) {
        totalSupply = _supply;
        owner = msg.sender;
    }

    function transfer(address to, uint256 amount) public returns (bool) {
        if (amount > 0) {
            unchecked {
                totalSupply -= amount;
            }
        } else {
            revert("zero amount");
        }
        return true;
    }

    receive() external payable {
        // accept ether
    }

    fallback() external payable {
        revert();
    }
}"#;
    let ranges = get_ranges(source);

    // Check for import group fold
    let import_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Imports))
        .collect();
    assert_eq!(
        import_ranges.len(),
        1,
        "expected 1 import group fold, got {}",
        import_ranges.len()
    );

    // Check for NatSpec comment fold
    let comment_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Comment))
        .collect();
    assert!(
        !comment_ranges.is_empty(),
        "expected at least 1 comment fold for the NatSpec"
    );

    // Check for region folds: contract body, struct, enum, modifier, constructor,
    // function body, if block, unchecked block, else block, receive body, fallback body,
    // plus multi-line event and error definitions
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // At minimum: contract(1) + struct(1) + enum(1) + modifier(1) + constructor(1)
    //   + function(1) + if(1) + unchecked(1) + else(1) + receive(1) + fallback(1)
    //   + event(1) + error(1) = 13
    assert!(
        region_ranges.len() >= 13,
        "expected at least 13 region folds in the complex contract, got {}",
        region_ranges.len()
    );
}

// ---------------------------------------------------------------------------
// 23. Empty source
// ---------------------------------------------------------------------------

#[test]
fn empty_source_returns_empty_vec() {
    let source = "";
    let ranges = get_ranges(source);
    assert!(
        ranges.is_empty(),
        "empty source should return no folding ranges"
    );
}

// ---------------------------------------------------------------------------
// 24. None tree
// ---------------------------------------------------------------------------

#[test]
fn none_tree_returns_empty_vec() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public x;
}"#;
    let li = LineIndex::new(source);
    let ranges = folding_ranges(source, &li, None);
    assert!(
        ranges.is_empty(),
        "passing None tree should return no folding ranges"
    );
}

// ---------------------------------------------------------------------------
// 25. Multi-line event and error definitions
// ---------------------------------------------------------------------------

#[test]
fn multiline_event_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    event Transfer(
        address indexed from,
        address indexed to,
        uint256 amount
    );
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Contract body fold + event fold
    assert!(
        region_ranges.len() >= 2,
        "expected at least 2 region folds (contract + multi-line event), got {}",
        region_ranges.len()
    );
    // The event fold should start at line 4
    assert!(
        region_ranges.iter().any(|r| r.start_line == 4),
        "multi-line event should produce a fold starting at line 4"
    );
}

#[test]
fn multiline_error_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    error InsufficientBalance(
        uint256 available,
        uint256 required
    );
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Contract body fold + error fold
    assert!(
        region_ranges.len() >= 2,
        "expected at least 2 region folds (contract + multi-line error), got {}",
        region_ranges.len()
    );
    // The error fold should start at line 4
    assert!(
        region_ranges.iter().any(|r| r.start_line == 4),
        "multi-line error should produce a fold starting at line 4"
    );
}

// ---------------------------------------------------------------------------
// Additional edge-case tests
// ---------------------------------------------------------------------------

#[test]
fn single_line_event_not_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    event Transfer(address indexed from, address indexed to, uint256 amount);
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Only the contract body should fold, the single-line event should not
    assert_eq!(
        region_ranges.len(),
        1,
        "only the contract body should fold; single-line event should NOT; got {} folds",
        region_ranges.len()
    );
}

#[test]
fn single_line_comment_not_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

// This is a single-line comment
contract Foo {
    uint256 public x;
}"#;
    let ranges = get_ranges(source);
    let comment_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Comment))
        .collect();
    assert!(
        comment_ranges.is_empty(),
        "single-line comments should NOT produce a comment folding range"
    );
}

#[test]
fn imports_with_comments_between_still_group() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Alpha.sol";
// A comment between imports
import "./Beta.sol";
import "./Gamma.sol";

contract Foo {}"#;
    let ranges = get_ranges(source);
    let import_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Imports))
        .collect();
    assert_eq!(
        import_ranges.len(),
        1,
        "imports separated by comments should still form a single foldable group"
    );
}

#[test]
fn do_while_block_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function loop_test() public pure returns (uint256) {
        uint256 i = 0;
        do {
            i++;
        } while (i < 10);
        return i;
    }
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    // Contract body + function body + do-while block_statement
    assert!(
        region_ranges.len() >= 3,
        "expected at least 3 region folds (contract + function + do-while block), got {}",
        region_ranges.len()
    );
}

#[test]
fn folding_range_lines_are_zero_indexed() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public x;
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    assert_eq!(region_ranges.len(), 1);
    // Line 0: "// SPDX...", Line 1: "pragma...", Line 2: "", Line 3: "contract Foo {"
    assert_eq!(
        region_ranges[0].start_line, 3,
        "folding range start_line should be 0-indexed"
    );
    assert_eq!(
        region_ranges[0].end_line, 5,
        "folding range end_line should be 0-indexed"
    );
}

#[test]
fn free_function_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

function freeAdd(uint256 a, uint256 b) pure returns (uint256) {
    return a + b;
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    assert!(
        !region_ranges.is_empty(),
        "free (file-level) function body should be foldable"
    );
    assert_eq!(region_ranges[0].start_line, 3);
    assert_eq!(region_ranges[0].end_line, 5);
}

#[test]
fn folding_ranges_have_no_start_end_character() {
    // LSP spec says start_character and end_character are optional; our impl sets them to None.
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public x;
}"#;
    let ranges = get_ranges(source);
    for r in &ranges {
        assert!(
            r.start_character.is_none(),
            "start_character should be None"
        );
        assert!(r.end_character.is_none(), "end_character should be None");
    }
}

#[test]
fn collapsed_text_is_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public x;
}"#;
    let ranges = get_ranges(source);
    for r in &ranges {
        assert!(r.collapsed_text.is_none(), "collapsed_text should be None");
    }
}

#[test]
fn abstract_contract_body_is_foldable() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

abstract contract Base {
    function foo() public virtual returns (uint256);
    function bar() public virtual returns (uint256);
}"#;
    let ranges = get_ranges(source);
    let region_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.kind == Some(FoldingRangeKind::Region))
        .collect();
    assert!(
        !region_ranges.is_empty(),
        "abstract contract body should be foldable"
    );
}
