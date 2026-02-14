use solidity_language_server::lint::LintEngine;
use solidity_language_server::parser::TsParser;

/// Helper: parse source with tree-sitter and run lint engine, return diagnostics.
fn lint(source: &str) -> Vec<tower_lsp::lsp_types::Diagnostic> {
    let mut parser = TsParser::new();
    let tree = parser.parse(source, None).expect("parse failed");
    let engine = LintEngine::new();
    engine.run(&tree, source)
}

#[test]
fn test_mixed_case_function_lint() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function add_num(uint256 a) public pure returns (uint256) {
        return a + 4;
    }
}
"#;
    let diags = lint(source);
    let mixed_case: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "mixed-case-function".to_string(),
                ))
        })
        .collect();
    assert!(
        !mixed_case.is_empty(),
        "Expected mixed-case-function lint diagnostic"
    );
    assert!(
        mixed_case[0].message.contains("add_num"),
        "Diagnostic should mention the function name"
    );
}

#[test]
fn test_pascal_case_function_no_false_positive() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function addNum(uint256 a) public pure returns (uint256) {
        return a + 4;
    }
}
"#;
    let diags = lint(source);
    let mixed_case: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "mixed-case-function".to_string(),
                ))
        })
        .collect();
    assert!(
        mixed_case.is_empty(),
        "Should not fire mixed-case-function for properly named function"
    );
}

#[test]
fn test_custom_errors_lint() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function foo() public {
        require(msg.sender != address(0), "not allowed");
    }
}
"#;
    let diags = lint(source);
    let custom_errors: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "custom-errors".to_string(),
                ))
        })
        .collect();
    assert!(
        !custom_errors.is_empty(),
        "Expected custom-errors lint diagnostic for require with string"
    );
}

#[test]
fn test_no_lint_on_clean_code() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Clean {
    uint256 public value;

    function setValue(uint256 newValue) public {
        value = newValue;
    }
}
"#;
    let diags = lint(source);
    // Clean code should have no linting issues (or very few)
    let lint_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.source == Some("ts-lint".to_string()))
        .collect();
    assert!(
        lint_diags.is_empty(),
        "Clean code should not trigger lint rules, got: {:?}",
        lint_diags
    );
}
