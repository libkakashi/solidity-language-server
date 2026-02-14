use solidity_language_server::parser::{TsParser, collect_parse_errors};

#[test]
fn parse_valid_contract_no_errors() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Counter {
    uint256 public count;

    function increment() public {
        count += 1;
    }

    function decrement() public {
        count -= 1;
    }

    function reset() public {
        count = 0;
    }
}
"#;
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None);
    assert!(tree.is_some(), "Should parse valid Solidity");
    let tree = tree.unwrap();
    let errors = collect_parse_errors(&tree, source);
    assert!(
        errors.is_empty(),
        "Valid code should have no parse errors, got: {:?}",
        errors
    );
}

#[test]
fn parse_invalid_syntax_reports_errors() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Bad {
    function foo( {
        return;
    }
}
"#;
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None);
    assert!(
        tree.is_some(),
        "Tree-sitter should still produce a tree for invalid code"
    );
    let tree = tree.unwrap();
    let errors = collect_parse_errors(&tree, source);
    assert!(
        !errors.is_empty(),
        "Should report parse errors for invalid syntax"
    );
}

#[test]
fn parse_empty_source() {
    let mut parser = TsParser::new();
    let tree = parser.parse("", None);
    assert!(tree.is_some(), "Should parse empty source");
    let tree = tree.unwrap();
    let errors = collect_parse_errors(&tree, "");
    assert!(errors.is_empty(), "Empty source should have no errors");
}

#[test]
fn parse_complex_contract() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    event Transfer(address indexed from, address indexed to, uint256 value);
}

abstract contract Ownable {
    address private _owner;

    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);

    modifier onlyOwner() {
        require(msg.sender == _owner);
        _;
    }

    constructor() {
        _owner = msg.sender;
    }

    function owner() public view returns (address) {
        return _owner;
    }
}

contract MyToken is IERC20, Ownable {
    string public name;
    string public symbol;
    uint8 public decimals;
    uint256 public override totalSupply;
    mapping(address => uint256) public override balanceOf;

    constructor(string memory _name, string memory _symbol) {
        name = _name;
        symbol = _symbol;
        decimals = 18;
    }

    function transfer(address to, uint256 amount) external override returns (bool) {
        require(balanceOf[msg.sender] >= amount);
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        emit Transfer(msg.sender, to, amount);
        return true;
    }

    function mint(address to, uint256 amount) external onlyOwner {
        totalSupply += amount;
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }
}
"#;
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None);
    assert!(tree.is_some());
    let tree = tree.unwrap();
    let errors = collect_parse_errors(&tree, source);
    assert!(
        errors.is_empty(),
        "Complex valid contract should have no parse errors, got: {:?}",
        errors
    );
}

#[test]
fn parse_errors_have_correct_source() {
    let source = r#"contract Bad {
    function foo() {
        uint256 x =;
    }
}
"#;
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let errors = collect_parse_errors(&tree, source);
    assert!(!errors.is_empty(), "Should detect parse error");
    // All errors should have source "ts-parse"
    for err in &errors {
        assert_eq!(
            err.source.as_deref(),
            Some("ts-parse"),
            "Error source should be ts-parse"
        );
    }
}
