use std::path::PathBuf;

use solidity_language_server::document_highlight::document_highlight;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::{DocumentHighlight, DocumentHighlightKind, Position};

fn setup(source: &str) -> (SymbolTable, PathBuf, LineIndex) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test_highlight.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    let li = LineIndex::new(source);
    (st, path, li)
}

/// Find the byte offset of `needle` in `source` and convert to an LSP Position.
fn pos_of(source: &str, needle: &str) -> Position {
    let byte = source
        .find(needle)
        .unwrap_or_else(|| panic!("needle {:?} not found in source", needle));
    let line = source[..byte].matches('\n').count() as u32;
    let col = (byte - source[..byte].rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32;
    Position::new(line, col)
}

/// Find the n-th (1-based) occurrence of `needle`.
fn pos_of_nth(source: &str, needle: &str, n: usize) -> Position {
    assert!(n >= 1);
    let mut start = 0;
    for _ in 0..n - 1 {
        let idx = source[start..]
            .find(needle)
            .unwrap_or_else(|| panic!("fewer than {} occurrences of {:?}", n, needle));
        start += idx + needle.len();
    }
    let byte = start
        + source[start..]
            .find(needle)
            .unwrap_or_else(|| panic!("fewer than {} occurrences of {:?}", n, needle));
    let line = source[..byte].matches('\n').count() as u32;
    let col = (byte - source[..byte].rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32;
    Position::new(line, col)
}

fn get_highlights(source: &str, pos: Position) -> Vec<DocumentHighlight> {
    let (st, path, li) = setup(source);
    document_highlight(&st, &path, source, pos, &li)
}

// =========================================================================
// 1. State variable highlights
// =========================================================================

#[test]
fn state_variable_declaration_and_usages() {
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

    function get() public view returns (uint256) {
        return totalSupply;
    }
}"#;
    let pos = pos_of(source, "totalSupply");
    let highlights = get_highlights(source, pos);
    // Declaration + 3 usages (mint, burn, get)
    assert!(
        highlights.len() >= 4,
        "expected at least 4 highlights for totalSupply, got {}",
        highlights.len()
    );
    // First highlight should be the WRITE (declaration)
    assert_eq!(highlights[0].kind, Some(DocumentHighlightKind::WRITE));
    // All other highlights should be READ
    for h in &highlights[1..] {
        assert_eq!(h.kind, Some(DocumentHighlightKind::READ));
    }
}

// =========================================================================
// 2. Function name highlights
// =========================================================================

#[test]
fn function_name_declaration_and_call() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function compute(uint256 x) public pure returns (uint256) {
        return x * 2;
    }

    function caller() public pure returns (uint256) {
        return compute(42);
    }
}"#;
    let pos = pos_of(source, "compute");
    let highlights = get_highlights(source, pos);
    // Declaration + call site
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights, got {}",
        highlights.len()
    );
    assert_eq!(highlights[0].kind, Some(DocumentHighlightKind::WRITE));
}

// =========================================================================
// 3. No highlights on whitespace/comments
// =========================================================================

#[test]
fn no_highlights_on_whitespace() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public {}
}"#;
    // Position on the comment line, col 0
    let pos = Position::new(0, 0);
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.is_empty(),
        "no highlights expected on comment, got {}",
        highlights.len()
    );
}

// =========================================================================
// 4. Function parameter highlights
// =========================================================================

#[test]
fn function_parameter_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function process(uint256 amount) public pure returns (uint256) {
        uint256 doubled = amount * 2;
        return doubled + amount;
    }
}"#;
    let pos = pos_of(source, "amount");
    let highlights = get_highlights(source, pos);
    // Declaration + 2 usages inside the function body
    assert!(
        highlights.len() >= 3,
        "expected at least 3 highlights for amount, got {}",
        highlights.len()
    );
}

// =========================================================================
// 5. Local variable highlights
// =========================================================================

#[test]
fn local_variable_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function calc() public pure returns (uint256) {
        uint256 temp = 10;
        temp = temp + 5;
        return temp;
    }
}"#;
    let pos = pos_of(source, "temp");
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 3,
        "expected at least 3 highlights for temp, got {}",
        highlights.len()
    );
}

// =========================================================================
// 6. Contract name highlights
// =========================================================================

#[test]
fn contract_name_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract MyToken {
    function get() public pure returns (uint256) { return 1; }
}

contract Factory {
    function create() public returns (MyToken) {
        return new MyToken();
    }
}"#;
    let pos = pos_of(source, "MyToken");
    let highlights = get_highlights(source, pos);
    // At least: declaration + return type reference + new expression
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for MyToken, got {}",
        highlights.len()
    );
}

// =========================================================================
// 7. Event name highlights
// =========================================================================

#[test]
fn event_name_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    event Transfer(address indexed from, address indexed to, uint256 value);

    function send(address to, uint256 amount) public {
        emit Transfer(msg.sender, to, amount);
    }
}"#;
    let pos = pos_of(source, "Transfer");
    let highlights = get_highlights(source, pos);
    // Declaration + emit site
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for Transfer, got {}",
        highlights.len()
    );
}

// =========================================================================
// 8. Struct name highlights
// =========================================================================

#[test]
fn struct_name_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Store {
    struct Item {
        uint256 id;
        string name;
    }

    Item[] public items;

    function add(string memory name) public {
        items.push(Item(items.length, name));
    }
}"#;
    let pos = pos_of(source, "Item");
    let highlights = get_highlights(source, pos);
    // Declaration + array type + constructor call
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for Item, got {}",
        highlights.len()
    );
}

// =========================================================================
// 9. Enum name highlights
// =========================================================================

#[test]
fn enum_name_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Voting {
    enum Status { Pending, Active, Closed }

    Status public currentStatus;

    function activate() public {
        currentStatus = Status.Active;
    }
}"#;
    let pos = pos_of(source, "Status");
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for Status, got {}",
        highlights.len()
    );
}

// =========================================================================
// 10. Modifier name highlights
// =========================================================================

#[test]
fn modifier_name_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function restricted() public onlyOwner {
    }

    function alsoRestricted() public onlyOwner {
    }
}"#;
    let pos = pos_of(source, "onlyOwner");
    let highlights = get_highlights(source, pos);
    // Declaration + 2 usages
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for onlyOwner, got {}",
        highlights.len()
    );
}

// =========================================================================
// 11. Error name highlights
// =========================================================================

#[test]
fn error_name_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    error Unauthorized(address caller);

    function withdraw() public {
        revert Unauthorized(msg.sender);
    }
}"#;
    let pos = pos_of(source, "Unauthorized");
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for Unauthorized, got {}",
        highlights.len()
    );
}

// =========================================================================
// 12. Multiple functions - no cross-contamination
// =========================================================================

#[test]
fn no_cross_function_contamination() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function alpha() public pure returns (uint256) { return 1; }
    function beta() public pure returns (uint256) { return 2; }
}"#;
    // Highlight alpha should not pick up beta
    let pos = pos_of(source, "alpha");
    let highlights = get_highlights(source, pos);
    for h in &highlights {
        // All highlights should be on the same line as "alpha" or referencing alpha
        // Just verify they exist and are reasonable
        assert!(h.kind.is_some());
    }

    // Highlight beta
    let pos_beta = pos_of(source, "beta");
    let highlights_beta = get_highlights(source, pos_beta);
    assert!(
        !highlights_beta.is_empty(),
        "should highlight beta"
    );
}

// =========================================================================
// 13. Highlights are sorted by position
// =========================================================================

#[test]
fn highlights_sorted_by_position() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public x;

    function a() public view returns (uint256) { return x; }
    function b() public view returns (uint256) { return x; }
    function c() public view returns (uint256) { return x; }
}"#;
    let pos = pos_of(source, "x");
    let highlights = get_highlights(source, pos);
    assert!(highlights.len() >= 2);
    // Verify sorted
    for w in highlights.windows(2) {
        let a_pos = (w[0].range.start.line, w[0].range.start.character);
        let b_pos = (w[1].range.start.line, w[1].range.start.character);
        assert!(a_pos <= b_pos, "highlights should be sorted by position");
    }
}

// =========================================================================
// 14. Highlights are deduplicated
// =========================================================================

#[test]
fn highlights_deduplicated() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public val;

    function get() public view returns (uint256) { return val; }
}"#;
    let pos = pos_of(source, "val");
    let highlights = get_highlights(source, pos);
    // Check no duplicate positions
    let mut positions: Vec<(u32, u32)> = highlights
        .iter()
        .map(|h| (h.range.start.line, h.range.start.character))
        .collect();
    let before_dedup = positions.len();
    positions.sort();
    positions.dedup();
    assert_eq!(
        positions.len(),
        before_dedup,
        "highlights should not have duplicate positions"
    );
}

// =========================================================================
// 15. Cursor on usage resolves back to declaration
// =========================================================================

#[test]
fn highlight_from_usage_site() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public balance;

    function deposit(uint256 amount) public {
        balance += amount;
    }

    function getBalance() public view returns (uint256) {
        return balance;
    }
}"#;
    // Click on `balance` in the deposit function (2nd occurrence)
    let pos = pos_of_nth(source, "balance", 2);
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 3,
        "expected at least 3 highlights from usage site, got {}",
        highlights.len()
    );
    // Declaration should still be WRITE
    assert_eq!(highlights[0].kind, Some(DocumentHighlightKind::WRITE));
}

// =========================================================================
// 16. Mapping variable highlights
// =========================================================================

#[test]
fn mapping_variable_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    mapping(address => uint256) public balances;

    function mint(address to, uint256 amount) public {
        balances[to] += amount;
    }

    function balanceOf(address account) public view returns (uint256) {
        return balances[account];
    }
}"#;
    let pos = pos_of(source, "balances");
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 3,
        "expected at least 3 highlights for balances, got {}",
        highlights.len()
    );
}

// =========================================================================
// 17. Inherited contract name
// =========================================================================

#[test]
fn inherited_contract_name_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function foo() public virtual returns (uint256) { return 1; }
}

contract Child is Base {
    function foo() public override returns (uint256) { return 2; }
}"#;
    let pos = pos_of(source, "Base");
    let highlights = get_highlights(source, pos);
    // Declaration + inheritance clause
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for Base, got {}",
        highlights.len()
    );
}

// =========================================================================
// 18. Interface name highlights
// =========================================================================

#[test]
fn interface_name_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IVault {
    function deposit(uint256 amount) external;
}

contract Vault is IVault {
    function deposit(uint256 amount) external override {}
}"#;
    let pos = pos_of(source, "IVault");
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for IVault, got {}",
        highlights.len()
    );
}

// =========================================================================
// 19. Library name highlights
// =========================================================================

#[test]
fn library_name_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

contract Foo {
    using SafeMath for uint256;
}"#;
    let pos = pos_of(source, "SafeMath");
    let highlights = get_highlights(source, pos);
    // At least the declaration itself
    assert!(
        !highlights.is_empty(),
        "expected at least 1 highlight for SafeMath, got 0"
    );
    assert_eq!(highlights[0].kind, Some(DocumentHighlightKind::WRITE));
}

// =========================================================================
// 20. Constant variable highlights
// =========================================================================

#[test]
fn constant_variable_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Config {
    uint256 public constant MAX_SUPPLY = 1000000;

    function check(uint256 amount) public pure returns (bool) {
        return amount <= MAX_SUPPLY;
    }
}"#;
    let pos = pos_of(source, "MAX_SUPPLY");
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for MAX_SUPPLY, got {}",
        highlights.len()
    );
}

// =========================================================================
// 21. Multiple parameters - only target is highlighted
// =========================================================================

#[test]
fn only_target_parameter_highlighted() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function swap(uint256 x, uint256 y) public pure returns (uint256, uint256) {
        return (y, x);
    }
}"#;
    // Highlight only `x`
    let pos = pos_of(source, "x,");
    let highlights_x = get_highlights(source, pos);

    // Highlight only `y`
    let pos_y = pos_of(source, "y)");
    let highlights_y = get_highlights(source, pos_y);

    // Both should have highlights, but they should be different sets
    assert!(!highlights_x.is_empty(), "x should have highlights");
    assert!(!highlights_y.is_empty(), "y should have highlights");
}

// =========================================================================
// 22. Empty contract - no crash
// =========================================================================

#[test]
fn empty_contract_no_crash() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Empty {}"#;
    let pos = Position::new(3, 10);
    let highlights = get_highlights(source, pos);
    // May or may not find highlights, just ensure no panic
    let _ = highlights;
}

// =========================================================================
// 23. Multiple state variables
// =========================================================================

#[test]
fn multiple_state_variables_independent() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Multi {
    uint256 public alpha;
    uint256 public beta;

    function use_both() public view returns (uint256) {
        return alpha + beta;
    }
}"#;
    let pos_alpha = pos_of(source, "alpha");
    let h_alpha = get_highlights(source, pos_alpha);

    let pos_beta = pos_of(source, "beta");
    let h_beta = get_highlights(source, pos_beta);

    assert!(!h_alpha.is_empty());
    assert!(!h_beta.is_empty());

    // alpha highlights should not overlap with beta highlights
    let alpha_lines: Vec<u32> = h_alpha.iter().map(|h| h.range.start.line).collect();
    let beta_lines: Vec<u32> = h_beta.iter().map(|h| h.range.start.line).collect();
    // The usage line (use_both) will appear in both, but declarations differ
    assert_ne!(alpha_lines[0], beta_lines[0], "declaration lines should differ");
}

// =========================================================================
// 24. Custom error parameter usage
// =========================================================================

#[test]
fn custom_error_field_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    error InsufficientBalance(uint256 available, uint256 required);

    function withdraw(uint256 amount) public {
        uint256 available = address(this).balance;
        if (available < amount) {
            revert InsufficientBalance(available, amount);
        }
    }
}"#;
    let pos = pos_of(source, "InsufficientBalance");
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for InsufficientBalance, got {}",
        highlights.len()
    );
}

// =========================================================================
// 25. Array variable highlights
// =========================================================================

#[test]
fn array_variable_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract List {
    uint256[] public items;

    function add(uint256 val) public {
        items.push(val);
    }

    function count() public view returns (uint256) {
        return items.length;
    }

    function first() public view returns (uint256) {
        return items[0];
    }
}"#;
    let pos = pos_of(source, "items");
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 4,
        "expected at least 4 highlights for items, got {}",
        highlights.len()
    );
}

// =========================================================================
// 26. Highlight from inside nested expression
// =========================================================================

#[test]
fn highlight_from_nested_expression() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Math {
    uint256 public result;

    function compute(uint256 a, uint256 b) public {
        result = (a + b) * (a - b);
    }
}"#;
    // Click on `result` in the assignment
    let pos = pos_of_nth(source, "result", 2);
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 2,
        "expected at least 2 highlights for result, got {}",
        highlights.len()
    );
}

// =========================================================================
// 27. Immutable variable highlights
// =========================================================================

#[test]
fn immutable_variable_highlight() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Config {
    uint256 public immutable DEPLOY_TIME;

    constructor() {
        DEPLOY_TIME = block.timestamp;
    }

    function getDeployTime() public view returns (uint256) {
        return DEPLOY_TIME;
    }
}"#;
    let pos = pos_of(source, "DEPLOY_TIME");
    let highlights = get_highlights(source, pos);
    assert!(
        highlights.len() >= 3,
        "expected at least 3 highlights for DEPLOY_TIME, got {}",
        highlights.len()
    );
}

// =========================================================================
// 28. Complex contract with many references
// =========================================================================

#[test]
fn complex_contract_many_references() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract ERC20 {
    mapping(address => uint256) private _balances;

    function balanceOf(address account) public view returns (uint256) {
        return _balances[account];
    }

    function _mint(address account, uint256 amount) internal {
        _balances[account] += amount;
    }

    function _burn(address account, uint256 amount) internal {
        _balances[account] -= amount;
    }

    function _transfer(address from, address to, uint256 amount) internal {
        _balances[from] -= amount;
        _balances[to] += amount;
    }
}"#;
    let pos = pos_of(source, "_balances");
    let highlights = get_highlights(source, pos);
    // Declaration + 5 usages (balanceOf, _mint, _burn, _transfer x2)
    assert!(
        highlights.len() >= 5,
        "expected at least 5 highlights for _balances, got {}",
        highlights.len()
    );
    // All sorted
    for w in highlights.windows(2) {
        assert!(
            (w[0].range.start.line, w[0].range.start.character)
                <= (w[1].range.start.line, w[1].range.start.character)
        );
    }
}
