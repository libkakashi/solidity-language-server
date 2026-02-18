use std::path::PathBuf;

use solidity_language_server::code_actions::code_actions;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::lint::LintEngine;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::*;

// ---------------------------------------------------------------------------
// Setup helpers
// ---------------------------------------------------------------------------

fn setup(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

/// Run the lint engine on `source`, then feed every diagnostic into `code_actions`.
fn get_actions(source: &str) -> Vec<CodeActionOrCommand> {
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let uri = Url::from_file_path(&path).unwrap();

    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).unwrap();
    let engine = LintEngine::new();
    let diagnostics = engine.run(&tree, source, &li);

    let range = Range {
        start: Position::new(0, 0),
        end: Position::new(u32::MAX, u32::MAX),
    };

    code_actions(&st, &path, source, range, &diagnostics, &li, &uri)
}

/// Feed an explicit set of diagnostics into `code_actions` (no lint engine).
fn get_actions_for_diags(source: &str, diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let uri = Url::from_file_path(&path).unwrap();
    let range = Range {
        start: Position::new(0, 0),
        end: Position::new(u32::MAX, u32::MAX),
    };
    code_actions(&st, &path, source, range, diagnostics, &li, &uri)
}

/// Build a synthetic diagnostic with the given code string.
fn make_diag(code: &str, range: Range, message: &str) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::String(code.to_string())),
        source: Some("solidity-language-server".to_string()),
        message: message.to_string(),
        ..Default::default()
    }
}

/// Extract the inner `CodeAction` from a `CodeActionOrCommand`, panicking otherwise.
fn unwrap_action(item: &CodeActionOrCommand) -> &CodeAction {
    match item {
        CodeActionOrCommand::CodeAction(a) => a,
        CodeActionOrCommand::Command(_) => panic!("expected CodeAction, got Command"),
    }
}

/// Collect action titles from a list of `CodeActionOrCommand`.
fn action_titles(actions: &[CodeActionOrCommand]) -> Vec<String> {
    actions
        .iter()
        .map(|a| unwrap_action(a).title.clone())
        .collect()
}

/// Filter actions whose associated diagnostic code matches `code`.
fn actions_for_code(actions: &[CodeActionOrCommand], code: &str) -> Vec<CodeAction> {
    actions
        .iter()
        .filter_map(|item| {
            let a = unwrap_action(item);
            let matches = a.diagnostics.as_ref().map_or(false, |diags| {
                diags
                    .iter()
                    .any(|d| d.code == Some(NumberOrString::String(code.to_string())))
            });
            if matches { Some(a.clone()) } else { None }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Mixed case function fix — `add_num` -> `addNum`
// ---------------------------------------------------------------------------

#[test]
fn mixed_case_function_fix() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function add_num(uint256 a) public pure returns (uint256) {
        return a + 4;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(
        !matched.is_empty(),
        "expected a mixed-case-function code action"
    );
    assert!(
        matched[0].title.contains("addNum"),
        "action title should suggest `addNum`, got: {}",
        matched[0].title
    );
}

// ---------------------------------------------------------------------------
// 2. Mixed case variable fix — `my_var` -> `myVar`
// ---------------------------------------------------------------------------

#[test]
fn mixed_case_variable_fix() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 public my_var;
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-variable");
    assert!(
        !matched.is_empty(),
        "expected a mixed-case-variable code action"
    );
    assert!(
        matched[0].title.contains("myVar"),
        "action title should suggest `myVar`, got: {}",
        matched[0].title
    );
}

// ---------------------------------------------------------------------------
// 3. Pascal case struct fix — `my_struct` -> `MyStruct`
// ---------------------------------------------------------------------------

#[test]
fn pascal_case_struct_fix() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    struct my_struct {
        uint256 x;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "pascal-case-struct");
    assert!(
        !matched.is_empty(),
        "expected a pascal-case-struct code action"
    );
    assert!(
        matched[0].title.contains("MyStruct"),
        "action title should suggest `MyStruct`, got: {}",
        matched[0].title
    );
}

// ---------------------------------------------------------------------------
// 4. Screaming snake const fix — `myConst` -> `MY_CONST`
// ---------------------------------------------------------------------------

#[test]
fn screaming_snake_const_fix() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 constant myConst = 42;
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "screaming-snake-case-const");
    assert!(
        !matched.is_empty(),
        "expected a screaming-snake-case-const code action"
    );
    assert!(
        matched[0].title.contains("MY_CONST"),
        "action title should suggest `MY_CONST`, got: {}",
        matched[0].title
    );
}

// ---------------------------------------------------------------------------
// 5. Screaming snake immutable fix — `myImmutable` -> `MY_IMMUTABLE`
// ---------------------------------------------------------------------------

#[test]
fn screaming_snake_immutable_fix() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 immutable myImmutable;

    constructor(uint256 v) {
        myImmutable = v;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "screaming-snake-case-immutable");
    assert!(
        !matched.is_empty(),
        "expected a screaming-snake-case-immutable code action"
    );
    assert!(
        matched[0].title.contains("MY_IMMUTABLE"),
        "action title should suggest `MY_IMMUTABLE`, got: {}",
        matched[0].title
    );
}

// ---------------------------------------------------------------------------
// 6. Custom errors fix — require(cond, "msg") -> revert
// ---------------------------------------------------------------------------

#[test]
fn custom_errors_fix() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function foo() public {
        require(msg.sender != address(0), "not allowed");
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "custom-errors");
    assert!(!matched.is_empty(), "expected a custom-errors code action");
    let action = &matched[0];
    assert!(
        action.title.contains("custom error"),
        "action title should mention custom error, got: {}",
        action.title
    );
    // Verify the replacement text contains `revert`
    let edit = action
        .edit
        .as_ref()
        .expect("action should have a workspace edit");
    let changes = edit
        .changes
        .as_ref()
        .expect("workspace edit should have changes");
    let edits: Vec<&TextEdit> = changes.values().flatten().collect();
    assert!(
        edits.iter().any(|e| e.new_text.contains("revert")),
        "replacement should contain `revert`"
    );
}

// ---------------------------------------------------------------------------
// 7. No actions for clean code
// ---------------------------------------------------------------------------

#[test]
fn no_actions_for_clean_code() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Clean {
    uint256 constant MAX_SUPPLY = 1000;
    uint256 public totalSupply;

    struct Token {
        address owner;
        uint256 amount;
    }

    function mint(uint256 amount) public {
        totalSupply = totalSupply + amount;
    }
}
"#;
    let actions = get_actions(source);
    assert!(
        actions.is_empty(),
        "clean code should produce no code actions, got {} action(s): {:?}",
        actions.len(),
        action_titles(&actions)
    );
}

// ---------------------------------------------------------------------------
// 8. Multiple actions — code with multiple issues gets multiple fixes
// ---------------------------------------------------------------------------

#[test]
fn multiple_actions_for_multiple_issues() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Bad {
    uint256 constant badConst = 100;

    struct bad_struct {
        uint256 x;
    }

    function bad_func() public {}
}
"#;
    let actions = get_actions(source);
    assert!(
        actions.len() >= 3,
        "expected at least 3 code actions for 3 violations, got {}",
        actions.len()
    );
}

// ---------------------------------------------------------------------------
// 9. Action has correct title — verify title describes the fix
// ---------------------------------------------------------------------------

#[test]
fn action_has_correct_title() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function add_num(uint256 a) public pure returns (uint256) {
        return a + 4;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(!matched.is_empty());
    assert_eq!(
        matched[0].title, "Rename to `addNum`",
        "expected exact title"
    );
}

// ---------------------------------------------------------------------------
// 10. Action produces workspace edit
// ---------------------------------------------------------------------------

#[test]
fn action_produces_workspace_edit() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function add_num(uint256 a) public pure returns (uint256) {
        return a + 4;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(!matched.is_empty());
    let action = &matched[0];
    let edit = action
        .edit
        .as_ref()
        .expect("code action must have a workspace edit");
    let changes = edit
        .changes
        .as_ref()
        .expect("workspace edit must have changes map");
    assert!(!changes.is_empty(), "changes map should not be empty");

    let text_edits: Vec<&TextEdit> = changes.values().flatten().collect();
    assert!(
        !text_edits.is_empty(),
        "there should be at least one text edit"
    );
    assert_eq!(
        text_edits[0].new_text, "addNum",
        "text edit should replace with `addNum`"
    );
}

// ---------------------------------------------------------------------------
// 11. Unused import removal
// ---------------------------------------------------------------------------

#[test]
fn unused_import_removal() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Unused} from "foo.sol";

contract A {
    uint256 public x;
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "unused-import");
    assert!(!matched.is_empty(), "expected an unused-import code action");
    assert!(
        matched[0].title.contains("Remove unused import"),
        "action title should mention removing unused import, got: {}",
        matched[0].title
    );
}

// ---------------------------------------------------------------------------
// 12. Action is QuickFix kind
// ---------------------------------------------------------------------------

#[test]
fn action_is_quickfix_kind() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function add_num(uint256 a) public pure returns (uint256) {
        return a + 4;
    }
}
"#;
    let actions = get_actions(source);
    for item in &actions {
        let action = unwrap_action(item);
        assert_eq!(
            action.kind,
            Some(CodeActionKind::QUICKFIX),
            "all code actions should be QuickFix kind, got {:?} for '{}'",
            action.kind,
            action.title
        );
    }
}

// ---------------------------------------------------------------------------
// 13. Mixed case with multiple underscores — `get_all_items` -> `getAllItems`
// ---------------------------------------------------------------------------

#[test]
fn mixed_case_multiple_underscores() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function get_all_items() public pure returns (uint256) {
        return 0;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(
        !matched.is_empty(),
        "expected a mixed-case-function code action"
    );
    assert!(
        matched[0].title.contains("getAllItems"),
        "action title should suggest `getAllItems`, got: {}",
        matched[0].title
    );
}

// ---------------------------------------------------------------------------
// 14. Multiple lint violations — each gets its own action
// ---------------------------------------------------------------------------

#[test]
fn each_violation_gets_own_action() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function bad_one() public {}
    function bad_two() public {}
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(
        matched.len() >= 2,
        "expected at least 2 mixed-case-function actions (one per violation), got {}",
        matched.len()
    );
    let titles: Vec<&str> = matched.iter().map(|a| a.title.as_str()).collect();
    assert!(
        titles.iter().any(|t| t.contains("badOne")),
        "should have action for badOne: {:?}",
        titles
    );
    assert!(
        titles.iter().any(|t| t.contains("badTwo")),
        "should have action for badTwo: {:?}",
        titles
    );
}

// ---------------------------------------------------------------------------
// 15. Screaming snake with mixed case — `myConstValue` -> `MY_CONST_VALUE`
// ---------------------------------------------------------------------------

#[test]
fn screaming_snake_mixed_case_const_value() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 constant myConstValue = 99;
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "screaming-snake-case-const");
    assert!(
        !matched.is_empty(),
        "expected a screaming-snake-case-const code action"
    );
    assert!(
        matched[0].title.contains("MY_CONST_VALUE"),
        "action title should suggest `MY_CONST_VALUE`, got: {}",
        matched[0].title
    );
}

// ---------------------------------------------------------------------------
// 16. No action for valid names — `addNum` gets no action
// ---------------------------------------------------------------------------

#[test]
fn no_action_for_valid_mixed_case() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function addNum(uint256 a) public pure returns (uint256) {
        return a + 1;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(
        matched.is_empty(),
        "correctly named `addNum` should not trigger mixed-case-function action"
    );
}

// ---------------------------------------------------------------------------
// 17. Function with test prefix excluded
// ---------------------------------------------------------------------------

#[test]
fn test_prefix_excluded_from_mixed_case() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract TestContract {
    function test_something() public {}
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(
        matched.is_empty(),
        "test_ prefixed functions should be excluded from mixed-case lint"
    );
}

// ---------------------------------------------------------------------------
// 18. Function with invariant prefix excluded
// ---------------------------------------------------------------------------

#[test]
fn invariant_prefix_excluded_from_mixed_case() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract InvariantTests {
    function invariant_something() public {}
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(
        matched.is_empty(),
        "invariant_ prefixed functions should be excluded from mixed-case lint"
    );
}

// ---------------------------------------------------------------------------
// 19. Incorrect shift fix — `1 << x` -> `x << 1`
// ---------------------------------------------------------------------------

#[test]
fn incorrect_shift_fix() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function shift(uint256 x) public pure returns (uint256) {
        return 1 << x;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "incorrect-shift");
    assert!(
        !matched.is_empty(),
        "expected an incorrect-shift code action"
    );
    assert_eq!(
        matched[0].title, "Swap shift operands",
        "action title should be 'Swap shift operands'"
    );

    // Verify the edit swaps the operands
    let edit = matched[0]
        .edit
        .as_ref()
        .expect("action should have workspace edit");
    let changes = edit.changes.as_ref().expect("should have changes");
    let text_edits: Vec<&TextEdit> = changes.values().flatten().collect();
    assert!(
        text_edits.iter().any(|e| e.new_text.contains("x << 1")),
        "replacement should swap operands to `x << 1`, got: {:?}",
        text_edits.iter().map(|e| &e.new_text).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 20. Empty diagnostics — returns empty actions
// ---------------------------------------------------------------------------

#[test]
fn empty_diagnostics_returns_empty_actions() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function foo() public {}
}
"#;
    let actions = get_actions_for_diags(source, &[]);
    assert!(
        actions.is_empty(),
        "empty diagnostics should yield no code actions"
    );
}

// ---------------------------------------------------------------------------
// 21. Manual diagnostic triggers correct code action
// ---------------------------------------------------------------------------

#[test]
fn manual_diagnostic_triggers_action() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function add_num(uint256 a) public pure returns (uint256) {
        return a + 4;
    }
}
"#;
    // Locate "add_num" in the source to build accurate range
    let fn_offset = source.find("add_num").unwrap();
    let _li = LineIndex::new(source);
    let start_line = source[..fn_offset].matches('\n').count() as u32;
    let start_col = (fn_offset - source[..fn_offset].rfind('\n').unwrap() - 1) as u32;
    let end_col = start_col + "add_num".len() as u32;

    let diag = make_diag(
        "mixed-case-function",
        Range {
            start: Position::new(start_line, start_col),
            end: Position::new(start_line, end_col),
        },
        "[lint] function name `add_num` should be mixedCase",
    );

    let actions = get_actions_for_diags(source, &[diag]);
    assert!(
        !actions.is_empty(),
        "manual diagnostic should produce an action"
    );
    let action = unwrap_action(&actions[0]);
    assert!(
        action.title.contains("addNum"),
        "action from manual diag should suggest `addNum`, got: {}",
        action.title
    );
}

// ---------------------------------------------------------------------------
// 22. Unknown diagnostic code produces no action
// ---------------------------------------------------------------------------

#[test]
fn unknown_diagnostic_code_produces_no_action() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {}
"#;
    let diag = make_diag(
        "nonexistent-code",
        Range {
            start: Position::new(3, 0),
            end: Position::new(3, 14),
        },
        "some unknown lint",
    );
    let actions = get_actions_for_diags(source, &[diag]);
    assert!(
        actions.is_empty(),
        "unknown diagnostic code should not produce any code action"
    );
}

// ---------------------------------------------------------------------------
// 23. Diagnostic without string code is ignored
// ---------------------------------------------------------------------------

#[test]
fn diagnostic_without_string_code_is_ignored() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {}
"#;
    let diag = Diagnostic {
        range: Range {
            start: Position::new(3, 0),
            end: Position::new(3, 14),
        },
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::Number(42)),
        message: "numeric code diagnostic".to_string(),
        ..Default::default()
    };
    let actions = get_actions_for_diags(source, &[diag]);
    assert!(
        actions.is_empty(),
        "diagnostic with numeric code should be ignored"
    );
}

// ---------------------------------------------------------------------------
// 24. Diagnostic with no code at all is ignored
// ---------------------------------------------------------------------------

#[test]
fn diagnostic_with_no_code_is_ignored() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {}
"#;
    let diag = Diagnostic {
        range: Range {
            start: Position::new(3, 0),
            end: Position::new(3, 14),
        },
        severity: Some(DiagnosticSeverity::WARNING),
        code: None,
        message: "no code diagnostic".to_string(),
        ..Default::default()
    };
    let actions = get_actions_for_diags(source, &[diag]);
    assert!(
        actions.is_empty(),
        "diagnostic with no code should be ignored"
    );
}

// ---------------------------------------------------------------------------
// 25. Custom errors fix preserves condition in revert
// ---------------------------------------------------------------------------

#[test]
fn custom_errors_fix_preserves_condition() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function foo(uint256 x) public {
        require(x > 0, "must be positive");
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "custom-errors");
    assert!(!matched.is_empty(), "expected a custom-errors code action");
    let edit = matched[0].edit.as_ref().unwrap();
    let changes = edit.changes.as_ref().unwrap();
    let text_edits: Vec<&TextEdit> = changes.values().flatten().collect();
    // The replacement should negate the condition and use revert
    assert!(
        text_edits
            .iter()
            .any(|e| e.new_text.contains("x > 0") && e.new_text.contains("revert")),
        "replacement should preserve condition `x > 0` and use `revert`, got: {:?}",
        text_edits.iter().map(|e| &e.new_text).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 26. Pascal case struct — `myStruct` (no underscore) -> `MyStruct`
// ---------------------------------------------------------------------------

#[test]
fn pascal_case_struct_no_underscore() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    struct myStruct {
        uint256 x;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "pascal-case-struct");
    assert!(
        !matched.is_empty(),
        "expected a pascal-case-struct code action"
    );
    assert!(
        matched[0].title.contains("MyStruct"),
        "action title should suggest `MyStruct`, got: {}",
        matched[0].title
    );
}

// ---------------------------------------------------------------------------
// 27. No action for valid PascalCase struct
// ---------------------------------------------------------------------------

#[test]
fn no_action_for_valid_pascal_case_struct() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    struct MyToken {
        uint256 id;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "pascal-case-struct");
    assert!(
        matched.is_empty(),
        "correctly named PascalCase struct should not trigger action"
    );
}

// ---------------------------------------------------------------------------
// 28. No action for valid SCREAMING_SNAKE_CASE constant
// ---------------------------------------------------------------------------

#[test]
fn no_action_for_valid_screaming_snake_const() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 constant MAX_SUPPLY = 1000;
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "screaming-snake-case-const");
    assert!(
        matched.is_empty(),
        "correctly named SCREAMING_SNAKE_CASE constant should not trigger action"
    );
}

// ---------------------------------------------------------------------------
// 29. Rename action has associated diagnostics
// ---------------------------------------------------------------------------

#[test]
fn rename_action_has_associated_diagnostics() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function add_num(uint256 a) public pure returns (uint256) {
        return a + 4;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(!matched.is_empty());
    let action = &matched[0];
    let diags = action
        .diagnostics
        .as_ref()
        .expect("code action should carry its triggering diagnostic(s)");
    assert!(
        !diags.is_empty(),
        "diagnostics list on action should not be empty"
    );
    assert_eq!(
        diags[0].code,
        Some(NumberOrString::String("mixed-case-function".to_string())),
        "attached diagnostic should have the same code"
    );
}

// ---------------------------------------------------------------------------
// 30. Unused import action removes the correct line
// ---------------------------------------------------------------------------

#[test]
fn unused_import_action_removes_line() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Unused} from "foo.sol";

contract A {
    uint256 public x;
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "unused-import");
    assert!(!matched.is_empty(), "expected unused-import action");
    let edit = matched[0]
        .edit
        .as_ref()
        .expect("should have workspace edit");
    let changes = edit.changes.as_ref().expect("should have changes");
    let text_edits: Vec<&TextEdit> = changes.values().flatten().collect();
    // The removal replaces the import line with empty string
    assert!(
        text_edits.iter().any(|e| e.new_text.is_empty()),
        "unused import removal should produce an empty replacement (deletion)"
    );
}

// ---------------------------------------------------------------------------
// 31. Swap shift operands title is correct
// ---------------------------------------------------------------------------

#[test]
fn swap_shift_operands_title() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function shift(uint256 x) public pure returns (uint256) {
        return 256 << x;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "incorrect-shift");
    assert!(!matched.is_empty(), "expected an incorrect-shift action");
    assert_eq!(matched[0].title, "Swap shift operands");
}

// ---------------------------------------------------------------------------
// 32. statefulFuzz prefix excluded from mixed case lint
// ---------------------------------------------------------------------------

#[test]
fn stateful_fuzz_prefix_excluded() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract FuzzTests {
    function statefulFuzz_something() public {}
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(
        matched.is_empty(),
        "statefulFuzz_ prefixed functions should be excluded from mixed-case lint"
    );
}

// ---------------------------------------------------------------------------
// 33. Mixed actions — struct + const violations each get their own
// ---------------------------------------------------------------------------

#[test]
fn mixed_struct_and_const_violations() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 constant badConst = 10;

    struct bad_struct {
        uint256 x;
    }
}
"#;
    let actions = get_actions(source);
    let struct_actions = actions_for_code(&actions, "pascal-case-struct");
    let const_actions = actions_for_code(&actions, "screaming-snake-case-const");
    assert!(
        !struct_actions.is_empty(),
        "should have a pascal-case-struct action"
    );
    assert!(
        !const_actions.is_empty(),
        "should have a screaming-snake-case-const action"
    );
}

// ---------------------------------------------------------------------------
// 34. Workspace edit targets correct URI
// ---------------------------------------------------------------------------

#[test]
fn workspace_edit_targets_correct_uri() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function add_num(uint256 a) public pure returns (uint256) {
        return a + 4;
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "mixed-case-function");
    assert!(!matched.is_empty());
    let edit = matched[0].edit.as_ref().unwrap();
    let changes = edit.changes.as_ref().unwrap();
    let expected_uri = Url::from_file_path("/tmp/test.sol").unwrap();
    assert!(
        changes.contains_key(&expected_uri),
        "workspace edit should target the file URI, keys: {:?}",
        changes.keys().collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 35. Custom errors — require without string message gets no action
// ---------------------------------------------------------------------------

#[test]
fn custom_errors_no_string_message_no_action() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function foo() public {
        require(msg.sender != address(0));
    }
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "custom-errors");
    assert!(
        matched.is_empty(),
        "require without string message should not trigger custom-errors action"
    );
}

// ---------------------------------------------------------------------------
// BUG TEST: to_screaming_snake_case should preserve leading underscores
// ---------------------------------------------------------------------------

#[test]
fn screaming_snake_preserves_leading_underscore() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 constant _myConst = 42;
}
"#;
    let actions = get_actions(source);
    let matched = actions_for_code(&actions, "screaming-snake-case-const");
    assert!(
        !matched.is_empty(),
        "expected a screaming-snake-case-const code action for _myConst"
    );
    // The fix should preserve the leading underscore: _myConst -> _MY_CONST
    // Bug: to_screaming_snake_case strips leading underscores, producing MY_CONST
    assert!(
        matched[0].title.contains("_MY_CONST"),
        "action should suggest `_MY_CONST` (preserving leading underscore), got: {}",
        matched[0].title
    );
}
