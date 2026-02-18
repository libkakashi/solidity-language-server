use std::path::PathBuf;

use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::semantic_tokens::{legend, semantic_tokens_full};
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

fn get_tokens(source: &str) -> Vec<SemanticToken> {
    let (st, path) = setup(source);
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let li = LineIndex::new(source);
    match semantic_tokens_full(&st, &path, source, &li, Some(&tree)) {
        Some(SemanticTokensResult::Tokens(tokens)) => tokens.data,
        _ => vec![],
    }
}

/// Helper: find the first token whose absolute line and column match.
/// We reconstruct absolute positions from the delta-encoded stream.
fn find_token_at(
    tokens: &[SemanticToken],
    target_line: u32,
    target_col: u32,
) -> Option<&SemanticToken> {
    let mut line: u32 = 0;
    let mut col: u32 = 0;
    for tok in tokens {
        if tok.delta_line > 0 {
            line += tok.delta_line;
            col = tok.delta_start;
        } else {
            col += tok.delta_start;
        }
        if line == target_line && col == target_col {
            return Some(tok);
        }
    }
    None
}

/// Helper: collect all tokens with a given token_type.
fn all_with_type(tokens: &[SemanticToken], token_type: u32) -> Vec<&SemanticToken> {
    tokens
        .iter()
        .filter(|t| t.token_type == token_type)
        .collect()
}

// Token type constants matching the legend order.
const TT_NAMESPACE: u32 = 0;
const TT_TYPE: u32 = 1;
const TT_FUNCTION: u32 = 2;
const TT_VARIABLE: u32 = 3;
const TT_PROPERTY: u32 = 4;
const TT_EVENT: u32 = 5;
const _TT_KEYWORD: u32 = 6;
const TT_NUMBER: u32 = 7;
const TT_STRING: u32 = 8;
const TT_COMMENT: u32 = 9;
const TT_MACRO: u32 = 10;
const TT_PARAMETER: u32 = 11;
const TT_ENUM_MEMBER: u32 = 12;

// Modifier bit constants.
const TM_DECLARATION: u32 = 1 << 0;
const TM_READONLY: u32 = 1 << 1;
const TM_STATIC: u32 = 1 << 2;
const TM_DEFINITION: u32 = 1 << 3;

// ===== 1. Legend has correct types =====

#[test]
fn legend_has_correct_token_types_count() {
    let leg = legend();
    assert_eq!(
        leg.token_types.len(),
        13,
        "Legend should have exactly 13 token types"
    );
}

#[test]
fn legend_has_correct_modifier_count() {
    let leg = legend();
    assert_eq!(
        leg.token_modifiers.len(),
        4,
        "Legend should have exactly 4 token modifiers"
    );
}

#[test]
fn legend_token_types_in_expected_order() {
    let leg = legend();
    assert_eq!(leg.token_types[0], SemanticTokenType::NAMESPACE);
    assert_eq!(leg.token_types[1], SemanticTokenType::TYPE);
    assert_eq!(leg.token_types[2], SemanticTokenType::FUNCTION);
    assert_eq!(leg.token_types[3], SemanticTokenType::VARIABLE);
    assert_eq!(leg.token_types[4], SemanticTokenType::PROPERTY);
    assert_eq!(leg.token_types[5], SemanticTokenType::EVENT);
    assert_eq!(leg.token_types[6], SemanticTokenType::KEYWORD);
    assert_eq!(leg.token_types[7], SemanticTokenType::NUMBER);
    assert_eq!(leg.token_types[8], SemanticTokenType::STRING);
    assert_eq!(leg.token_types[9], SemanticTokenType::COMMENT);
    assert_eq!(leg.token_types[10], SemanticTokenType::MACRO);
    assert_eq!(leg.token_types[11], SemanticTokenType::PARAMETER);
    assert_eq!(leg.token_types[12], SemanticTokenType::ENUM_MEMBER);
}

#[test]
fn legend_modifiers_in_expected_order() {
    let leg = legend();
    assert_eq!(leg.token_modifiers[0], SemanticTokenModifier::DECLARATION);
    assert_eq!(leg.token_modifiers[1], SemanticTokenModifier::READONLY);
    assert_eq!(leg.token_modifiers[2], SemanticTokenModifier::STATIC);
    assert_eq!(leg.token_modifiers[3], SemanticTokenModifier::DEFINITION);
}

// ===== 2. Contract name token =====

#[test]
fn contract_name_gets_namespace_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract MyContract {
}
"#;
    let tokens = get_tokens(source);
    let ns_tokens = all_with_type(&tokens, TT_NAMESPACE);
    assert!(
        !ns_tokens.is_empty(),
        "Should produce at least one NAMESPACE token for a contract name"
    );
    // The contract name "MyContract" is 10 chars.
    let contract_tok = ns_tokens
        .iter()
        .find(|t| t.length == 10)
        .expect("Should have a NAMESPACE token with length 10 for 'MyContract'");
    assert_eq!(contract_tok.token_type, TT_NAMESPACE);
}

// ===== 3. Function name token =====

#[test]
fn function_name_gets_function_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function doSomething() public pure {
    }
}
"#;
    let tokens = get_tokens(source);
    let fn_tokens = all_with_type(&tokens, TT_FUNCTION);
    assert!(
        !fn_tokens.is_empty(),
        "Should produce at least one FUNCTION token"
    );
    // "doSomething" is 11 chars.
    let fn_tok = fn_tokens
        .iter()
        .find(|t| t.length == 11)
        .expect("Should have a FUNCTION token with length 11 for 'doSomething'");
    assert_eq!(fn_tok.token_type, TT_FUNCTION);
    assert!(
        fn_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Function declaration should have DECLARATION modifier"
    );
}

// ===== 4. State variable token =====

#[test]
fn state_variable_gets_variable_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public totalSupply;
}
"#;
    let tokens = get_tokens(source);
    let var_tokens = all_with_type(&tokens, TT_VARIABLE);
    // "totalSupply" is 11 chars.
    let var_tok = var_tokens
        .iter()
        .find(|t| t.length == 11)
        .expect("Should have a VARIABLE token with length 11 for 'totalSupply'");
    assert_eq!(var_tok.token_type, TT_VARIABLE);
    assert!(
        var_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "State variable declaration should have DECLARATION modifier"
    );
}

// ===== 5. Parameter token =====

#[test]
fn parameter_gets_parameter_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar(uint256 amount) public pure returns (uint256) {
        return amount;
    }
}
"#;
    let tokens = get_tokens(source);
    let param_tokens = all_with_type(&tokens, TT_PARAMETER);
    // "amount" is 6 chars.
    let param_tok = param_tokens
        .iter()
        .find(|t| t.length == 6)
        .expect("Should have a PARAMETER token with length 6 for 'amount'");
    assert_eq!(param_tok.token_type, TT_PARAMETER);
    assert!(
        param_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Parameter declaration should have DECLARATION modifier"
    );
}

// ===== 6. Struct name token =====

#[test]
fn struct_name_gets_type_token() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Position {
        uint256 x;
        uint256 y;
    }
}
"#;
    let tokens = get_tokens(source);
    let type_tokens = all_with_type(&tokens, TT_TYPE);
    // "Position" is 8 chars.
    let struct_tok = type_tokens
        .iter()
        .find(|t| t.length == 8 && t.token_modifiers_bitset & TM_DECLARATION != 0)
        .expect("Should have a TYPE token with length 8 for 'Position' with DECLARATION modifier");
    assert_eq!(struct_tok.token_type, TT_TYPE);
}

// ===== 7. Enum name token =====

#[test]
fn enum_name_gets_type_token() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    enum Color { Red, Green, Blue }
}
"#;
    let tokens = get_tokens(source);
    let type_tokens = all_with_type(&tokens, TT_TYPE);
    // "Color" is 5 chars.
    let enum_tok = type_tokens
        .iter()
        .find(|t| t.length == 5 && t.token_modifiers_bitset & TM_DECLARATION != 0)
        .expect("Should have a TYPE token with length 5 for 'Color' with DECLARATION modifier");
    assert_eq!(enum_tok.token_type, TT_TYPE);
}

// ===== 8. Event name token =====

#[test]
fn event_name_gets_event_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    event Transfer(address indexed from, address indexed to, uint256 amount);
}
"#;
    let tokens = get_tokens(source);
    let event_tokens = all_with_type(&tokens, TT_EVENT);
    // "Transfer" is 8 chars.
    let event_tok = event_tokens
        .iter()
        .find(|t| t.length == 8)
        .expect("Should have an EVENT token with length 8 for 'Transfer'");
    assert_eq!(event_tok.token_type, TT_EVENT);
    assert!(
        event_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Event declaration should have DECLARATION modifier"
    );
}

// ===== 9. Error name token =====

#[test]
fn error_name_gets_macro_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    error InsufficientBalance(uint256 available, uint256 required);
}
"#;
    let tokens = get_tokens(source);
    let macro_tokens = all_with_type(&tokens, TT_MACRO);
    // "InsufficientBalance" is 19 chars.
    let error_tok = macro_tokens
        .iter()
        .find(|t| t.length == 19)
        .expect("Should have a MACRO token with length 19 for 'InsufficientBalance'");
    assert_eq!(error_tok.token_type, TT_MACRO);
    assert!(
        error_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Error declaration should have DECLARATION modifier"
    );
}

// ===== 10. Number literal token =====

#[test]
fn number_literal_gets_number_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public value = 42;
}
"#;
    let tokens = get_tokens(source);
    let num_tokens = all_with_type(&tokens, TT_NUMBER);
    // "42" is 2 chars.
    let num_tok = num_tokens
        .iter()
        .find(|t| t.length == 2)
        .expect("Should have a NUMBER token with length 2 for '42'");
    assert_eq!(num_tok.token_type, TT_NUMBER);
    assert_eq!(
        num_tok.token_modifiers_bitset, 0,
        "Number literal should have no modifiers"
    );
}

// ===== 11. String literal token =====

#[test]
fn string_literal_gets_string_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    string public name = "hello";
}
"#;
    let tokens = get_tokens(source);
    let str_tokens = all_with_type(&tokens, TT_STRING);
    // "hello" with quotes is 7 chars.
    let str_tok = str_tokens
        .iter()
        .find(|t| t.length == 7)
        .expect("Should have a STRING token with length 7 for '\"hello\"'");
    assert_eq!(str_tok.token_type, TT_STRING);
    assert_eq!(
        str_tok.token_modifiers_bitset, 0,
        "String literal should have no modifiers"
    );
}

// ===== 12. Comment token =====

#[test]
fn comment_gets_comment_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

// This is a comment
contract Foo {
}
"#;
    let tokens = get_tokens(source);
    let comment_tokens = all_with_type(&tokens, TT_COMMENT);
    assert!(
        !comment_tokens.is_empty(),
        "Should produce at least one COMMENT token"
    );
    // The SPDX comment itself is a comment.
    // "// This is a comment" is 20 chars.
    let user_comment = comment_tokens
        .iter()
        .find(|t| t.length == 20)
        .expect("Should have a COMMENT token with length 20 for '// This is a comment'");
    assert_eq!(user_comment.token_type, TT_COMMENT);
}

// ===== 13. Struct field token =====

#[test]
fn struct_field_gets_property_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }
}
"#;
    let tokens = get_tokens(source);
    let prop_tokens = all_with_type(&tokens, TT_PROPERTY);
    // "x" is 1 char, "y" is 1 char.
    assert!(
        prop_tokens.len() >= 2,
        "Should have at least 2 PROPERTY tokens for struct fields x and y, got {}",
        prop_tokens.len()
    );
    let x_tok = prop_tokens
        .iter()
        .find(|t| t.length == 1)
        .expect("Should have a PROPERTY token with length 1 for 'x'");
    assert_eq!(x_tok.token_type, TT_PROPERTY);
    assert!(
        x_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Struct field declaration should have DECLARATION modifier"
    );
}

// ===== 14. Enum member token =====

#[test]
fn enum_member_gets_enum_member_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    enum Status { Active, Inactive, Pending }
}
"#;
    let tokens = get_tokens(source);
    let em_tokens = all_with_type(&tokens, TT_ENUM_MEMBER);
    assert!(
        em_tokens.len() >= 3,
        "Should have at least 3 ENUM_MEMBER tokens for Active, Inactive, Pending, got {}",
        em_tokens.len()
    );
    // "Active" is 6 chars.
    let active_tok = em_tokens
        .iter()
        .find(|t| t.length == 6)
        .expect("Should have an ENUM_MEMBER token with length 6 for 'Active'");
    assert_eq!(active_tok.token_type, TT_ENUM_MEMBER);
    assert!(
        active_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Enum member should have DECLARATION modifier"
    );
}

// ===== 15. Declaration modifier =====

#[test]
fn declarations_have_declaration_modifier_set() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    uint256 public balance;
    function deposit() public {}
    event Deposited(uint256 amount);
    error NotAllowed();
    struct Info { uint256 id; }
    enum State { Open, Closed }
}
"#;
    let tokens = get_tokens(source);

    // Contract name "Vault" should have DECLARATION | DEFINITION.
    let vault_tok = all_with_type(&tokens, TT_NAMESPACE)
        .into_iter()
        .find(|t| t.length == 5)
        .expect("Should find NAMESPACE token for 'Vault'");
    assert!(
        vault_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Contract declaration should have DECLARATION modifier"
    );
    assert!(
        vault_tok.token_modifiers_bitset & TM_DEFINITION != 0,
        "Contract declaration should have DEFINITION modifier"
    );

    // Function name "deposit" (7 chars) should have DECLARATION | DEFINITION.
    let deposit_tok = all_with_type(&tokens, TT_FUNCTION)
        .into_iter()
        .find(|t| t.length == 7)
        .expect("Should find FUNCTION token for 'deposit'");
    assert!(
        deposit_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Function declaration should have DECLARATION modifier"
    );
    assert!(
        deposit_tok.token_modifiers_bitset & TM_DEFINITION != 0,
        "Function declaration should have DEFINITION modifier"
    );

    // Event name "Deposited" (9 chars) should have DECLARATION | DEFINITION.
    let event_tok = all_with_type(&tokens, TT_EVENT)
        .into_iter()
        .find(|t| t.length == 9)
        .expect("Should find EVENT token for 'Deposited'");
    assert!(
        event_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Event declaration should have DECLARATION modifier"
    );

    // Error name "NotAllowed" (10 chars) should have DECLARATION | DEFINITION.
    let error_tok = all_with_type(&tokens, TT_MACRO)
        .into_iter()
        .find(|t| t.length == 10)
        .expect("Should find MACRO token for 'NotAllowed'");
    assert!(
        error_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Error declaration should have DECLARATION modifier"
    );
}

// ===== 16. Constant variable readonly =====

#[test]
fn constant_variable_has_readonly_modifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public constant MAX_SUPPLY = 1000000;
}
"#;
    let tokens = get_tokens(source);
    // "MAX_SUPPLY" is 10 chars.
    let var_tokens = all_with_type(&tokens, TT_VARIABLE);
    let const_tok = var_tokens
        .iter()
        .find(|t| t.length == 10)
        .expect("Should find VARIABLE token with length 10 for 'MAX_SUPPLY'");
    assert!(
        const_tok.token_modifiers_bitset & TM_READONLY != 0,
        "Constant variable should have READONLY modifier, got bitset: {}",
        const_tok.token_modifiers_bitset
    );
    assert!(
        const_tok.token_modifiers_bitset & TM_STATIC != 0,
        "Constant variable should have STATIC modifier, got bitset: {}",
        const_tok.token_modifiers_bitset
    );
    assert!(
        const_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "Constant variable should have DECLARATION modifier"
    );
}

#[test]
fn immutable_variable_has_readonly_modifier() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public immutable deployTime;
    constructor() {
        deployTime = block.timestamp;
    }
}
"#;
    let tokens = get_tokens(source);
    // "deployTime" is 10 chars.
    let var_tokens = all_with_type(&tokens, TT_VARIABLE);
    let imm_tok = var_tokens
        .iter()
        .find(|t| t.length == 10 && t.token_modifiers_bitset & TM_READONLY != 0)
        .expect("Should find VARIABLE token with READONLY modifier for 'deployTime'");
    assert!(
        imm_tok.token_modifiers_bitset & TM_STATIC != 0,
        "Immutable variable should have STATIC modifier"
    );
}

// ===== 17. No tokens for empty file =====

#[test]
fn empty_source_returns_empty_tokens() {
    let source = "";
    let tokens = get_tokens(source);
    assert!(
        tokens.is_empty(),
        "Empty source should produce no tokens, got {} tokens",
        tokens.len()
    );
}

// ===== 18. None tree returns None =====

#[test]
fn none_tree_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let result = semantic_tokens_full(&st, &path, source, &li, None);
    assert!(result.is_none(), "Passing None for tree should return None");
}

// ===== 19. Multiple declarations =====

#[test]
fn multiple_declarations_produce_correct_token_count() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Multi {
    uint256 public alpha;
    uint256 public beta;
    uint256 public gamma;

    function funcA() public pure {}
    function funcB() public pure {}

    event EventA(uint256 value);
    event EventB(uint256 value);

    error ErrorA();
    error ErrorB();
}
"#;
    let tokens = get_tokens(source);

    // Count declaration tokens by type.
    let ns_decls: Vec<_> = all_with_type(&tokens, TT_NAMESPACE)
        .into_iter()
        .filter(|t| t.token_modifiers_bitset & TM_DECLARATION != 0)
        .collect();
    assert_eq!(
        ns_decls.len(),
        1,
        "Should have 1 NAMESPACE declaration (Multi)"
    );

    let fn_decls: Vec<_> = all_with_type(&tokens, TT_FUNCTION)
        .into_iter()
        .filter(|t| t.token_modifiers_bitset & TM_DECLARATION != 0)
        .collect();
    assert_eq!(
        fn_decls.len(),
        2,
        "Should have 2 FUNCTION declarations (funcA, funcB)"
    );

    let event_decls: Vec<_> = all_with_type(&tokens, TT_EVENT)
        .into_iter()
        .filter(|t| t.token_modifiers_bitset & TM_DECLARATION != 0)
        .collect();
    assert_eq!(
        event_decls.len(),
        2,
        "Should have 2 EVENT declarations (EventA, EventB)"
    );

    let error_decls: Vec<_> = all_with_type(&tokens, TT_MACRO)
        .into_iter()
        .filter(|t| t.token_modifiers_bitset & TM_DECLARATION != 0)
        .collect();
    assert_eq!(
        error_decls.len(),
        2,
        "Should have 2 MACRO declarations (ErrorA, ErrorB)"
    );

    let var_decls: Vec<_> = all_with_type(&tokens, TT_VARIABLE)
        .into_iter()
        .filter(|t| t.token_modifiers_bitset & TM_DECLARATION != 0)
        .collect();
    assert_eq!(
        var_decls.len(),
        3,
        "Should have 3 VARIABLE declarations (alpha, beta, gamma)"
    );
}

// ===== 20. Interface declarations =====

#[test]
fn interface_name_gets_namespace_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
}
"#;
    let tokens = get_tokens(source);
    let ns_tokens = all_with_type(&tokens, TT_NAMESPACE);
    // "IERC20" is 6 chars.
    let iface_tok = ns_tokens
        .iter()
        .find(|t| t.length == 6 && t.token_modifiers_bitset & TM_DECLARATION != 0)
        .expect("Should have a NAMESPACE token with length 6 for 'IERC20'");
    assert_eq!(iface_tok.token_type, TT_NAMESPACE);
    assert!(
        iface_tok.token_modifiers_bitset & TM_DEFINITION != 0,
        "Interface declaration should have DEFINITION modifier"
    );
}

// ===== 21. Token positions are correct =====

#[test]
fn token_positions_encode_valid_deltas() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Pos {
    uint256 public x;
    uint256 public y;
}
"#;
    let tokens = get_tokens(source);
    assert!(
        !tokens.is_empty(),
        "Should produce tokens for this contract"
    );

    // Reconstruct absolute positions and verify they are monotonically ordered.
    let mut line: u32 = 0;
    let mut col: u32 = 0;
    let mut prev_line: u32 = 0;
    let mut prev_col: u32 = 0;
    let mut first = true;

    for tok in &tokens {
        if tok.delta_line > 0 {
            line += tok.delta_line;
            col = tok.delta_start;
        } else {
            col += tok.delta_start;
        }

        if !first {
            assert!(
                line > prev_line || (line == prev_line && col >= prev_col),
                "Tokens should be in monotonically non-decreasing position order: \
                 prev=({}, {}), current=({}, {})",
                prev_line,
                prev_col,
                line,
                col
            );
        }

        // Token length should be positive.
        assert!(tok.length > 0, "Token length should be positive");

        prev_line = line;
        prev_col = col;
        first = false;
    }
}

#[test]
fn contract_name_is_on_expected_line() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Located {
}
"#;
    let tokens = get_tokens(source);
    // "Located" is on line 3 (0-indexed). Lines: 0=SPDX, 1=pragma, 2=blank, 3=contract.
    let tok = find_token_at(&tokens, 3, 9);
    assert!(
        tok.is_some(),
        "Should find a token at line 3, column 9 (the position of 'Located')"
    );
    let tok = tok.unwrap();
    assert_eq!(tok.token_type, TT_NAMESPACE);
    assert_eq!(tok.length, 7, "Token length should be 7 for 'Located'");
}

// ===== 22. Complex contract =====

#[test]
fn complex_contract_produces_all_element_types() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

/// A complex contract for testing
contract Complex {
    struct Info {
        uint256 id;
        string label;
    }

    enum State { Created, Active, Closed }

    event StateChanged(State newState);
    error Unauthorized(address caller);

    uint256 public constant VERSION = 1;
    uint256 public totalCount;

    function process(uint256 amount) public pure returns (uint256) {
        uint256 result = amount * 2;
        return result;
    }

    function getName() public pure returns (string memory) {
        return "complex";
    }
}
"#;
    let tokens = get_tokens(source);

    // Verify we get tokens for all major token types present in this source.
    let has_comment = tokens.iter().any(|t| t.token_type == TT_COMMENT);
    let has_namespace = tokens.iter().any(|t| t.token_type == TT_NAMESPACE);
    let has_type = tokens.iter().any(|t| t.token_type == TT_TYPE);
    let has_function = tokens.iter().any(|t| t.token_type == TT_FUNCTION);
    let has_variable = tokens.iter().any(|t| t.token_type == TT_VARIABLE);
    let has_property = tokens.iter().any(|t| t.token_type == TT_PROPERTY);
    let has_event = tokens.iter().any(|t| t.token_type == TT_EVENT);
    let has_number = tokens.iter().any(|t| t.token_type == TT_NUMBER);
    let has_string = tokens.iter().any(|t| t.token_type == TT_STRING);
    let has_macro = tokens.iter().any(|t| t.token_type == TT_MACRO);
    let has_parameter = tokens.iter().any(|t| t.token_type == TT_PARAMETER);
    let has_enum_member = tokens.iter().any(|t| t.token_type == TT_ENUM_MEMBER);

    assert!(
        has_comment,
        "Complex contract should produce COMMENT tokens"
    );
    assert!(
        has_namespace,
        "Complex contract should produce NAMESPACE tokens (contract name)"
    );
    assert!(
        has_type,
        "Complex contract should produce TYPE tokens (struct/enum names)"
    );
    assert!(
        has_function,
        "Complex contract should produce FUNCTION tokens"
    );
    assert!(
        has_variable,
        "Complex contract should produce VARIABLE tokens"
    );
    assert!(
        has_property,
        "Complex contract should produce PROPERTY tokens (struct fields)"
    );
    assert!(has_event, "Complex contract should produce EVENT tokens");
    assert!(has_number, "Complex contract should produce NUMBER tokens");
    assert!(has_string, "Complex contract should produce STRING tokens");
    assert!(
        has_macro,
        "Complex contract should produce MACRO tokens (error names)"
    );
    assert!(
        has_parameter,
        "Complex contract should produce PARAMETER tokens"
    );
    assert!(
        has_enum_member,
        "Complex contract should produce ENUM_MEMBER tokens"
    );
}

// ===== Additional tests for completeness =====

#[test]
fn library_name_gets_namespace_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#;
    let tokens = get_tokens(source);
    let ns_tokens = all_with_type(&tokens, TT_NAMESPACE);
    // "MathLib" is 7 chars.
    let lib_tok = ns_tokens
        .iter()
        .find(|t| t.length == 7 && t.token_modifiers_bitset & TM_DECLARATION != 0)
        .expect("Should have a NAMESPACE token with length 7 for 'MathLib'");
    assert_eq!(lib_tok.token_type, TT_NAMESPACE);
    assert!(
        lib_tok.token_modifiers_bitset & TM_DEFINITION != 0,
        "Library declaration should have DEFINITION modifier"
    );
}

#[test]
fn multiline_comment_produces_multiple_tokens() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

/* This is
   a multi-line
   comment */
contract Foo {
}
"#;
    let tokens = get_tokens(source);
    let comment_tokens = all_with_type(&tokens, TT_COMMENT);
    // The SPDX comment is one, plus the multiline comment should be split.
    // The multiline comment "/* This is\n   a multi-line\n   comment */" spans 3 lines,
    // producing 3 separate comment tokens.
    assert!(
        comment_tokens.len() >= 4,
        "Should have at least 4 COMMENT tokens (1 SPDX + 3 from multiline), got {}",
        comment_tokens.len()
    );
}

#[test]
fn modifier_definition_gets_function_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function restricted() public onlyOwner {}
}
"#;
    let tokens = get_tokens(source);
    let fn_tokens = all_with_type(&tokens, TT_FUNCTION);
    // "onlyOwner" is 9 chars.
    let mod_tok = fn_tokens
        .iter()
        .find(|t| t.length == 9 && t.token_modifiers_bitset & TM_DECLARATION != 0)
        .expect("Should have a FUNCTION token with length 9 for 'onlyOwner' modifier definition");
    assert_eq!(mod_tok.token_type, TT_FUNCTION);
    assert!(
        mod_tok.token_modifiers_bitset & TM_DEFINITION != 0,
        "Modifier definition should have DEFINITION modifier"
    );
}

#[test]
fn event_parameters_get_parameter_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    event Transfer(address indexed sender, address indexed receiver, uint256 value);
}
"#;
    let tokens = get_tokens(source);
    let param_tokens = all_with_type(&tokens, TT_PARAMETER);
    // "sender" (6), "receiver" (8), "value" (5).
    let sender_tok = param_tokens.iter().find(|t| t.length == 6);
    let receiver_tok = param_tokens.iter().find(|t| t.length == 8);
    let value_tok = param_tokens.iter().find(|t| t.length == 5);
    assert!(
        sender_tok.is_some(),
        "Should have PARAMETER token for 'sender'"
    );
    assert!(
        receiver_tok.is_some(),
        "Should have PARAMETER token for 'receiver'"
    );
    assert!(
        value_tok.is_some(),
        "Should have PARAMETER token for 'value'"
    );
}

#[test]
fn local_variable_gets_variable_type_with_declaration() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure returns (uint256) {
        uint256 localVal = 10;
        return localVal;
    }
}
"#;
    let tokens = get_tokens(source);
    let var_tokens = all_with_type(&tokens, TT_VARIABLE);
    // "localVal" is 8 chars.
    let local_tok = var_tokens
        .iter()
        .find(|t| t.length == 8 && t.token_modifiers_bitset & TM_DECLARATION != 0)
        .expect("Should have a VARIABLE token with DECLARATION for 'localVal'");
    assert_eq!(local_tok.token_type, TT_VARIABLE);
}

#[test]
fn inheritance_base_contract_gets_namespace_type() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseFunc() public pure returns (uint256) {
        return 1;
    }
}

contract Child is Base {
    function childFunc() public pure returns (uint256) {
        return 2;
    }
}
"#;
    let tokens = get_tokens(source);
    let ns_tokens = all_with_type(&tokens, TT_NAMESPACE);
    // Should have at least 3 NAMESPACE tokens: "Base" decl, "Child" decl, "Base" in inheritance.
    assert!(
        ns_tokens.len() >= 3,
        "Should have at least 3 NAMESPACE tokens (Base decl, Child decl, Base in inheritance), got {}",
        ns_tokens.len()
    );
    // "Base" tokens (4 chars) - should appear at least twice (declaration + inheritance ref).
    let base_tokens: Vec<_> = ns_tokens.iter().filter(|t| t.length == 4).collect();
    assert!(
        base_tokens.len() >= 2,
        "Should have at least 2 NAMESPACE tokens for 'Base' (decl + inheritance ref), got {}",
        base_tokens.len()
    );
}

#[test]
fn file_level_constant_has_readonly_and_static() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

uint256 constant FILE_CONST = 999;

contract Foo {
}
"#;
    let tokens = get_tokens(source);
    let var_tokens = all_with_type(&tokens, TT_VARIABLE);
    // "FILE_CONST" is 10 chars.
    let const_tok = var_tokens
        .iter()
        .find(|t| t.length == 10)
        .expect("Should have a VARIABLE token with length 10 for 'FILE_CONST'");
    assert!(
        const_tok.token_modifiers_bitset & TM_DECLARATION != 0,
        "File-level constant should have DECLARATION modifier"
    );
    assert!(
        const_tok.token_modifiers_bitset & TM_READONLY != 0,
        "File-level constant should have READONLY modifier"
    );
    assert!(
        const_tok.token_modifiers_bitset & TM_STATIC != 0,
        "File-level constant should have STATIC modifier"
    );
}

#[test]
fn primitive_type_in_function_gets_type_token() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar(uint256 val) public pure returns (bool) {
        return val > 0;
    }
}
"#;
    let tokens = get_tokens(source);
    let type_tokens = all_with_type(&tokens, TT_TYPE);
    // "uint256" (7 chars) and "bool" (4 chars) are primitive types.
    let uint_tok = type_tokens.iter().find(|t| t.length == 7);
    let bool_tok = type_tokens.iter().find(|t| t.length == 4);
    assert!(
        uint_tok.is_some(),
        "Should have a TYPE token for 'uint256' primitive type"
    );
    assert!(
        bool_tok.is_some(),
        "Should have a TYPE token for 'bool' primitive type"
    );
}

#[test]
fn token_types_are_within_legend_bounds() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Bounds {
    struct Data { uint256 v; }
    enum Mode { Fast, Slow }
    event Ping(uint256 seq);
    error Boom();
    uint256 public counter = 0;
    function increment(uint256 step) public {
        counter += step;
    }
}
"#;
    let tokens = get_tokens(source);
    let leg = legend();
    let max_type = leg.token_types.len() as u32;
    let max_mod_bits = (1u32 << leg.token_modifiers.len()) - 1;

    for (i, tok) in tokens.iter().enumerate() {
        assert!(
            tok.token_type < max_type,
            "Token {} has token_type {} which exceeds legend size {}",
            i,
            tok.token_type,
            max_type
        );
        assert!(
            tok.token_modifiers_bitset <= max_mod_bits,
            "Token {} has modifier bitset {} which exceeds max valid {}",
            i,
            tok.token_modifiers_bitset,
            max_mod_bits
        );
    }
}

#[test]
fn user_defined_type_definition_gets_type_token() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

type FixedPoint is uint256;
"#;
    let tokens = get_tokens(source);
    let type_tokens = all_with_type(&tokens, TT_TYPE);
    // "FixedPoint" is 10 chars.
    let udt_tok = type_tokens
        .iter()
        .find(|t| t.length == 10 && t.token_modifiers_bitset & TM_DECLARATION != 0)
        .expect("Should have a TYPE token with DECLARATION for 'FixedPoint'");
    assert_eq!(udt_tok.token_type, TT_TYPE);
    assert!(
        udt_tok.token_modifiers_bitset & TM_DEFINITION != 0,
        "User-defined type definition should have DEFINITION modifier"
    );
}

#[test]
fn only_pragma_produces_minimal_tokens() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;
"#;
    let tokens = get_tokens(source);
    // Should have at least the SPDX comment token.
    let comment_tokens = all_with_type(&tokens, TT_COMMENT);
    assert!(
        !comment_tokens.is_empty(),
        "Source with only SPDX and pragma should still produce comment tokens"
    );
    // Should NOT have any contract/function/variable tokens.
    let ns_tokens = all_with_type(&tokens, TT_NAMESPACE);
    let fn_tokens = all_with_type(&tokens, TT_FUNCTION);
    assert!(
        ns_tokens.is_empty(),
        "Source with no contract should not produce NAMESPACE tokens"
    );
    assert!(
        fn_tokens.is_empty(),
        "Source with no functions should not produce FUNCTION tokens"
    );
}
