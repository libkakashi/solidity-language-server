use solidity_language_server::fmt_config::{
    FmtConfig, IndentStyle, IntTypes, NumberUnderscore, QuoteStyle,
};
use solidity_language_server::formatter;
use solidity_language_server::parser::TsParser;

/// Helper: parse source and format with given config.
fn fmt_with(source: &str, config: &FmtConfig) -> String {
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).expect("parse failed");
    formatter::format(source, &tree, config).into_owned()
}

/// Helper: parse source and format with default config.
fn fmt(source: &str) -> String {
    fmt_with(source, &FmtConfig::default())
}

// ---------------------------------------------------------------------------
// Basic formatting
// ---------------------------------------------------------------------------

#[test]
fn basic_indentation() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
function bar() public pure returns (uint256) {
return 42;
}
}"#;
    let result = fmt(source);
    // Function body should be indented.
    assert!(
        result.contains("    function bar()"),
        "Function should be indented:\n{result}"
    );
    assert!(
        result.contains("        return 42;"),
        "Return should be double-indented:\n{result}"
    );
}

#[test]
fn basic_spacing() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}"#;
    let result = fmt(source);
    assert!(result.contains("a + b"), "Operators should have spaces:\n{result}");
}

#[test]
fn semicolons_preserved() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    uint256 public x;
}"#;
    let result = fmt(source);
    assert!(
        result.contains("uint256 public x;"),
        "Semicolon should be preserved:\n{result}"
    );
}

#[test]
fn blank_lines_between_sections() {
    let source = r#"pragma solidity ^0.8.29;
import {Foo} from "foo.sol";
contract Bar {}"#;
    let result = fmt(source);
    // There should be blank lines between pragma, import, and contract sections.
    let lines: Vec<&str> = result.lines().collect();
    let pragma_idx = lines.iter().position(|l| l.starts_with("pragma")).unwrap();
    let import_idx = lines.iter().position(|l| l.starts_with("import")).unwrap();
    assert!(
        import_idx > pragma_idx + 1,
        "Should have blank line between pragma and import:\n{result}"
    );
}

// ---------------------------------------------------------------------------
// Config option tests
// ---------------------------------------------------------------------------

#[test]
fn config_tab_indentation() {
    let mut config = FmtConfig::default();
    config.indent_style = IndentStyle::Tabs;

    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    uint256 public x;
}"#;
    let result = fmt_with(source, &config);
    assert!(
        result.contains("\tuint256"),
        "Should use tab indentation:\n{result}"
    );
}

#[test]
fn config_quote_style_double() {
    let source = r#"import {Foo} from './Foo.sol';"#;
    let result = fmt(source);
    assert!(
        result.contains("\"./Foo.sol\""),
        "Should convert to double quotes:\n{result}"
    );
}

#[test]
fn config_quote_style_single() {
    let mut config = FmtConfig::default();
    config.quote_style = QuoteStyle::Single;

    let source = r#"import {Foo} from "./Foo.sol";"#;
    let result = fmt_with(source, &config);
    assert!(
        result.contains("'./Foo.sol'"),
        "Should convert to single quotes:\n{result}"
    );
}

#[test]
fn config_quote_style_preserve() {
    let mut config = FmtConfig::default();
    config.quote_style = QuoteStyle::Preserve;

    let source = r#"import {Foo} from './Foo.sol';"#;
    let result = fmt_with(source, &config);
    assert!(
        result.contains("'./Foo.sol'"),
        "Should preserve single quotes:\n{result}"
    );
}

#[test]
fn config_int_types_long() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    uint public x;
    int public y;
}"#;
    let result = fmt(source);
    assert!(result.contains("uint256"), "uint should become uint256:\n{result}");
    assert!(result.contains("int256"), "int should become int256:\n{result}");
}

#[test]
fn config_int_types_short() {
    let mut config = FmtConfig::default();
    config.int_types = IntTypes::Short;

    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    uint256 public x;
    int256 public y;
}"#;
    let result = fmt_with(source, &config);
    // "uint " (with space) should appear, but not "uint256"
    assert!(
        result.contains("uint ") || result.contains("uint\t"),
        "uint256 should become uint:\n{result}"
    );
}

#[test]
fn config_int_types_preserve() {
    let mut config = FmtConfig::default();
    config.int_types = IntTypes::Preserve;

    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    uint public x;
}"#;
    let result = fmt_with(source, &config);
    // Should keep "uint" as-is, not convert to "uint256".
    assert!(
        result.contains("uint public"),
        "Should preserve uint:\n{result}"
    );
}

#[test]
fn config_number_underscore_remove() {
    let mut config = FmtConfig::default();
    config.number_literal_underscore = NumberUnderscore::Remove;

    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    uint256 constant X = 1_000_000;
}"#;
    let result = fmt_with(source, &config);
    assert!(
        result.contains("1000000"),
        "Should remove underscores:\n{result}"
    );
}

#[test]
fn config_number_underscore_thousands() {
    let mut config = FmtConfig::default();
    config.number_literal_underscore = NumberUnderscore::Thousands;

    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    uint256 constant X = 1000000;
}"#;
    let result = fmt_with(source, &config);
    assert!(
        result.contains("1_000_000"),
        "Should add thousands separators:\n{result}"
    );
}

#[test]
fn config_sort_imports() {
    let mut config = FmtConfig::default();
    config.sort_imports = true;

    let source = r#"pragma solidity ^0.8.29;
import {B} from "b.sol";
import {A} from "a.sol";"#;
    let result = fmt_with(source, &config);
    let a_pos = result.find("\"a.sol\"").unwrap();
    let b_pos = result.find("\"b.sol\"").unwrap();
    assert!(a_pos < b_pos, "Imports should be sorted:\n{result}");
}

// ---------------------------------------------------------------------------
// Idempotency tests
// ---------------------------------------------------------------------------

#[test]
fn idempotency_simple_contract() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {IERC20} from "./IERC20.sol";

contract Token {
    uint256 public totalSupply;

    mapping(address => uint256) public balanceOf;

    function transfer(address to, uint256 amount) public returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}
"#;
    let first = fmt(source);
    let second = fmt(&first);
    assert_eq!(first, second, "Formatting should be idempotent");
}

#[test]
fn idempotency_with_inheritance() {
    let source = r#"pragma solidity ^0.8.29;

contract Base {
    uint256 public x;
}

contract Child is Base {
    function setX(uint256 _x) public {
        x = _x;
    }
}
"#;
    let first = fmt(source);
    let second = fmt(&first);
    assert_eq!(first, second, "Formatting should be idempotent");
}

// ---------------------------------------------------------------------------
// Disable comment tests
// ---------------------------------------------------------------------------

#[test]
fn disable_next_line() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    // forgefmt: disable-next-line
    uint256   public   x ;
    uint256 public y;
}"#;
    let result = fmt(source);
    assert!(
        result.contains("uint256   public   x ;"),
        "Disabled line should be preserved:\n{result}"
    );
}

#[test]
fn disable_start_end_block() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    // forgefmt: disable-start
    uint256   public   x ;
    uint256   public   y ;
    // forgefmt: disable-end
    uint256 public z;
}"#;
    let result = fmt(source);
    assert!(
        result.contains("uint256   public   x ;"),
        "Disabled block should be preserved:\n{result}"
    );
    assert!(
        result.contains("uint256   public   y ;"),
        "Disabled block should be preserved:\n{result}"
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_file() {
    let result = fmt("");
    assert_eq!(result, "\n");
}

#[test]
fn syntax_errors_return_original() {
    let source = "contract { invalid }}}";
    let result = fmt(source);
    // With parse errors, should return original source unchanged.
    assert_eq!(result, source.to_string());
}

#[test]
fn empty_contract() {
    let source = "pragma solidity ^0.8.29;\ncontract Foo {}";
    let result = fmt(source);
    assert!(result.contains("contract Foo {}"), "Empty contract:\n{result}");
}

#[test]
fn multi_contract_file() {
    let source = r#"pragma solidity ^0.8.29;
contract A {
    uint256 public x;
}
contract B {
    uint256 public y;
}"#;
    let result = fmt(source);
    assert!(result.contains("contract A"), "Should have contract A:\n{result}");
    assert!(result.contains("contract B"), "Should have contract B:\n{result}");
}

#[test]
fn interface_formatting() {
    let source = r#"pragma solidity ^0.8.29;
interface IFoo {
    function bar() external returns (uint256);
}"#;
    let result = fmt(source);
    assert!(
        result.contains("interface IFoo"),
        "Should format interface:\n{result}"
    );
}

#[test]
fn library_formatting() {
    let source = r#"pragma solidity ^0.8.29;
library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}"#;
    let result = fmt(source);
    assert!(
        result.contains("library MathLib"),
        "Should format library:\n{result}"
    );
}

#[test]
fn struct_formatting() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }
}"#;
    let result = fmt(source);
    assert!(
        result.contains("struct Point"),
        "Should format struct:\n{result}"
    );
}

#[test]
fn enum_formatting() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    enum Status {
        Active,
        Inactive
    }
}"#;
    let result = fmt(source);
    assert!(result.contains("enum Status"), "Should format enum:\n{result}");
    assert!(result.contains("Active"), "Should have enum value:\n{result}");
}

#[test]
fn event_formatting() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    event Transfer(address indexed from, address indexed to, uint256 amount);
}"#;
    let result = fmt(source);
    assert!(
        result.contains("event Transfer"),
        "Should format event:\n{result}"
    );
}

#[test]
fn error_formatting() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    error InsufficientBalance(uint256 available, uint256 required);
}"#;
    let result = fmt(source);
    assert!(
        result.contains("error InsufficientBalance"),
        "Should format error:\n{result}"
    );
}

#[test]
fn constructor_formatting() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    uint256 public x;
    constructor(uint256 _x) {
        x = _x;
    }
}"#;
    let result = fmt(source);
    assert!(
        result.contains("constructor(uint256 _x)"),
        "Should format constructor:\n{result}"
    );
}

#[test]
fn modifier_formatting() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    address public owner;
    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }
}"#;
    let result = fmt(source);
    assert!(
        result.contains("modifier onlyOwner()"),
        "Should format modifier:\n{result}"
    );
}

#[test]
fn if_else_formatting() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    function check(uint256 x) public pure returns (bool) {
        if (x > 10) {
            return true;
        } else {
            return false;
        }
    }
}"#;
    let result = fmt(source);
    assert!(result.contains("if (x > 10)"), "Should format if:\n{result}");
    assert!(result.contains("} else {"), "Should format else:\n{result}");
}

#[test]
fn for_loop_formatting() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    function sum(uint256 n) public pure returns (uint256) {
        uint256 total = 0;
        for (uint256 i = 0; i < n; i++) {
            total += i;
        }
        return total;
    }
}"#;
    let result = fmt(source);
    assert!(result.contains("for ("), "Should format for loop:\n{result}");
}

#[test]
fn while_loop_formatting() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    function count() public pure returns (uint256) {
        uint256 i = 0;
        while (i < 10) {
            i++;
        }
        return i;
    }
}"#;
    let result = fmt(source);
    assert!(result.contains("while (i < 10)"), "Should format while:\n{result}");
}

#[test]
fn return_statement() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    function get() public pure returns (uint256) {
        return 42;
    }
}"#;
    let result = fmt(source);
    assert!(result.contains("return 42;"), "Should format return:\n{result}");
}

#[test]
fn binary_expression_spacing() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    function calc(uint256 a, uint256 b) public pure returns (uint256) {
        return a+b*2-1;
    }
}"#;
    let result = fmt(source);
    // Operators should have spaces around them.
    // The expression tree should produce: a + b * 2 - 1 (or similar with precedence)
    assert!(
        !result.contains("a+b"),
        "Should add spaces around operators:\n{result}"
    );
}

#[test]
fn member_expression() {
    let source = r#"pragma solidity ^0.8.29;
contract Foo {
    function getBalance() public view returns (uint256) {
        return msg.sender.balance;
    }
}"#;
    let result = fmt(source);
    assert!(
        result.contains("msg.sender"),
        "Should preserve member access:\n{result}"
    );
}

#[test]
fn inheritance_formatting() {
    let source = r#"pragma solidity ^0.8.29;
contract Child is Parent, Ownable {
    uint256 public x;
}"#;
    let result = fmt(source);
    assert!(
        result.contains("contract Child is Parent, Ownable"),
        "Should format inheritance:\n{result}"
    );
}
