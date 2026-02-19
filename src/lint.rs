use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag, NumberOrString};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

// ---------------------------------------------------------------------------
// Lint rule definition
// ---------------------------------------------------------------------------

type FilterFn = fn(rule: &LintRule, source: &str, captures: &[(&str, Node)]) -> Option<LintHit>;

struct LintRule {
    id: &'static str,
    description: &'static str,
    severity: DiagnosticSeverity,
    query: Query,
    range_capture: &'static str,
    filter: Option<FilterFn>,
    /// If true, skip in main loop — handled by a dedicated pass. (Fix #6)
    skip_main_loop: bool,
}

struct LintHit {
    start_byte: usize,
    end_byte: usize,
    message: String,
}

// ---------------------------------------------------------------------------
// Lint engine
// ---------------------------------------------------------------------------

pub struct LintEngine {
    rules: Vec<LintRule>,
}

impl LintEngine {
    pub fn new() -> Self {
        let lang: tree_sitter::Language = tree_sitter_solidity::LANGUAGE.into();
        let rules = build_rules(&lang);
        Self { rules }
    }

    pub fn run(
        &self,
        tree: &Tree,
        source: &str,
        line_index: &crate::utils::LineIndex,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        // Reuse a single QueryCursor across rules. (Fix #20)
        let mut cursor = QueryCursor::new();

        for rule in &self.rules {
            // Skip rules handled by dedicated passes. (Fix #6)
            if rule.skip_main_loop {
                continue;
            }

            let mut matches = cursor.matches(&rule.query, tree.root_node(), source.as_bytes());

            while let Some(m) = matches.next() {
                let captures: Vec<(&str, Node)> = m
                    .captures
                    .iter()
                    .map(|c| {
                        let name = rule.query.capture_names()[c.index as usize];
                        (name, c.node)
                    })
                    .collect();

                if let Some(filter) = rule.filter {
                    if let Some(hit) = filter(rule, source, &captures) {
                        diagnostics.push(make_diagnostic(
                            rule,
                            hit.start_byte,
                            hit.end_byte,
                            source,
                            line_index,
                            &hit.message,
                        ));
                    }
                } else {
                    if let Some(node) = find_capture(&captures, rule.range_capture) {
                        diagnostics.push(make_diagnostic(
                            rule,
                            node.start_byte(),
                            node.end_byte(),
                            source,
                            line_index,
                            rule.description,
                        ));
                    }
                }
            }
        }

        // Run the two-pass unused-import check separately.
        diagnostics.extend(check_unused_imports(
            tree,
            source,
            &self.rules,
            &mut cursor,
            line_index,
        ));

        diagnostics
    }
}

fn find_capture<'a>(captures: &'a [(&str, Node<'a>)], name: &str) -> Option<Node<'a>> {
    captures
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, node)| *node)
}

fn node_text<'a>(node: Node<'a>, source: &'a str) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

fn make_diagnostic(
    rule: &LintRule,
    start_byte: usize,
    end_byte: usize,
    source: &str,
    line_index: &crate::utils::LineIndex,
    message: &str,
) -> Diagnostic {
    let range = line_index.byte_range_to_lsp_range(source, start_byte, end_byte);
    Diagnostic {
        range,
        severity: Some(rule.severity),
        code: Some(NumberOrString::String(rule.id.to_string())),
        source: Some("ts-lint".into()),
        message: format!("[lint] {message}"),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Naming convention helpers
// ---------------------------------------------------------------------------

fn is_pascal_case(name: &str) -> bool {
    if name.is_empty() || name.len() == 1 {
        return true;
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_uppercase() {
        return false;
    }
    !name.contains('_')
}

fn is_mixed_case(name: &str) -> bool {
    if name.is_empty() || name.len() == 1 {
        return true;
    }
    let trimmed = name.trim_start_matches('_');
    if trimmed.is_empty() {
        return true;
    }
    let first = trimmed.chars().next().unwrap();
    if !first.is_ascii_lowercase() {
        return false;
    }
    !trimmed.contains('_')
}

fn is_screaming_snake_case(name: &str) -> bool {
    if name.is_empty() || name.len() == 1 {
        return true;
    }
    let trimmed = name.trim_matches('_');
    if trimmed.is_empty() {
        return true;
    }
    trimmed
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !trimmed.contains("__")
}

fn decl_has_constant(node: Node, source: &str) -> bool {
    let text = node_text(node, source);
    text.contains(" constant ") || text.contains("\tconstant ")
}

fn decl_has_immutable(node: Node) -> bool {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.node().kind() == "immutable" {
                return true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Filter functions for each rule
// ---------------------------------------------------------------------------

fn filter_incorrect_shift(
    _rule: &LintRule,
    source: &str,
    captures: &[(&str, Node)],
) -> Option<LintHit> {
    let expr = find_capture(captures, "expr")?;
    let literal = find_capture(captures, "literal")?;
    let variable = find_capture(captures, "variable")?;

    let between = &source[literal.end_byte()..variable.start_byte()];
    if !between.contains("<<") && !between.contains(">>") {
        return None;
    }

    Some(LintHit {
        start_byte: expr.start_byte(),
        end_byte: expr.end_byte(),
        message: "shift operands may be reversed: literal shifted by variable".into(),
    })
}

fn filter_divide_before_multiply(
    _rule: &LintRule,
    source: &str,
    captures: &[(&str, Node)],
) -> Option<LintHit> {
    let outer = find_capture(captures, "outer")?;
    let left_subtree = find_capture(captures, "left_subtree")?;

    let outer_text = node_text(outer, source);
    if !has_operator(outer, source, "*") {
        return None;
    }
    if !contains_division(left_subtree, source) {
        return None;
    }

    Some(LintHit {
        start_byte: outer.start_byte(),
        end_byte: outer.end_byte(),
        message: format!(
            "division before multiplication may cause precision loss: `{}`",
            outer_text.replace('\n', " ")
        ),
    })
}

fn contains_division(node: Node, source: &str) -> bool {
    if node.kind() == "binary_expression" && has_operator(node, source, "/") {
        return true;
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if contains_division(cursor.node(), source) {
                return true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

fn has_operator(binary_expr: Node, source: &str, op: &str) -> bool {
    let mut cursor = binary_expr.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.field_name() == Some("operator") {
                let node = cursor.node();
                let text = &source[node.start_byte()..node.end_byte()];
                return text.trim() == op;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

fn filter_pascal_case_struct(
    _rule: &LintRule,
    source: &str,
    captures: &[(&str, Node)],
) -> Option<LintHit> {
    let name_node = find_capture(captures, "name")?;
    let name = node_text(name_node, source);
    if is_pascal_case(name) {
        return None;
    }
    Some(LintHit {
        start_byte: name_node.start_byte(),
        end_byte: name_node.end_byte(),
        message: format!("struct name `{name}` should be PascalCase"),
    })
}

fn filter_mixed_case_function(
    _rule: &LintRule,
    source: &str,
    captures: &[(&str, Node)],
) -> Option<LintHit> {
    let name_node = find_capture(captures, "name")?;
    let name = node_text(name_node, source);

    if name.starts_with("test")
        || name.starts_with("invariant_")
        || name.starts_with("statefulFuzz")
    {
        return None;
    }

    if is_mixed_case(name) {
        return None;
    }
    Some(LintHit {
        start_byte: name_node.start_byte(),
        end_byte: name_node.end_byte(),
        message: format!("function name `{name}` should be mixedCase"),
    })
}

fn filter_mixed_case_variable(
    _rule: &LintRule,
    source: &str,
    captures: &[(&str, Node)],
) -> Option<LintHit> {
    let decl = find_capture(captures, "decl")?;
    let name_node = find_capture(captures, "name")?;

    if decl_has_constant(decl, source) || decl_has_immutable(decl) {
        return None;
    }

    let name = node_text(name_node, source);
    if is_mixed_case(name) {
        return None;
    }
    Some(LintHit {
        start_byte: name_node.start_byte(),
        end_byte: name_node.end_byte(),
        message: format!("mutable variable name `{name}` should be mixedCase"),
    })
}

fn filter_screaming_snake_const(
    _rule: &LintRule,
    source: &str,
    captures: &[(&str, Node)],
) -> Option<LintHit> {
    let decl = find_capture(captures, "decl")?;
    let name_node = find_capture(captures, "name")?;

    if !decl_has_constant(decl, source) {
        return None;
    }

    let name = node_text(name_node, source);
    if is_screaming_snake_case(name) {
        return None;
    }
    Some(LintHit {
        start_byte: name_node.start_byte(),
        end_byte: name_node.end_byte(),
        message: format!("constant `{name}` should be SCREAMING_SNAKE_CASE"),
    })
}

fn filter_screaming_snake_immutable(
    _rule: &LintRule,
    source: &str,
    captures: &[(&str, Node)],
) -> Option<LintHit> {
    let name_node = find_capture(captures, "name")?;
    let name = node_text(name_node, source);
    if is_screaming_snake_case(name) {
        return None;
    }
    Some(LintHit {
        start_byte: name_node.start_byte(),
        end_byte: name_node.end_byte(),
        message: format!("immutable `{name}` should be SCREAMING_SNAKE_CASE"),
    })
}

fn filter_unaliased_plain_import(
    _rule: &LintRule,
    source: &str,
    captures: &[(&str, Node)],
) -> Option<LintHit> {
    let import_node = find_capture(captures, "import")?;
    let path_node = find_capture(captures, "path")?;

    let mut cursor = import_node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "identifier" || cursor.field_name() == Some("import_name") {
                return None;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    let path = node_text(path_node, source);
    Some(LintHit {
        start_byte: import_node.start_byte(),
        end_byte: import_node.end_byte(),
        message: format!("use named imports instead of plain `import {path}`"),
    })
}

fn filter_unsafe_cheatcode(
    _rule: &LintRule,
    source: &str,
    captures: &[(&str, Node)],
) -> Option<LintHit> {
    let call = find_capture(captures, "call")?;
    let method = find_capture(captures, "method")?;
    let method_name = node_text(method, source);

    Some(LintHit {
        start_byte: call.start_byte(),
        end_byte: call.end_byte(),
        message: format!("potentially unsafe cheatcode `{method_name}`"),
    })
}

fn filter_custom_errors(
    _rule: &LintRule,
    _source: &str,
    captures: &[(&str, Node)],
) -> Option<LintHit> {
    let call = find_capture(captures, "call")?;

    let mut has_string_arg = false;
    let mut cursor = call.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "call_argument" {
                let mut inner = child.walk();
                if inner.goto_first_child() {
                    loop {
                        if contains_string_literal(inner.node()) {
                            has_string_arg = true;
                            break;
                        }
                        if !inner.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if has_string_arg {
                break;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    if !has_string_arg {
        return None;
    }

    Some(LintHit {
        start_byte: call.start_byte(),
        end_byte: call.end_byte(),
        message: "use custom errors instead of `require` with string message".into(),
    })
}

fn contains_string_literal(node: Node) -> bool {
    if node.kind() == "string_literal" || node.kind() == "string" {
        return true;
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if contains_string_literal(cursor.node()) {
                return true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Two-pass: unused imports (reuses the provided cursor)
// ---------------------------------------------------------------------------

fn check_unused_imports(
    tree: &Tree,
    source: &str,
    rules: &[LintRule],
    cursor: &mut QueryCursor,
    line_index: &crate::utils::LineIndex,
) -> Vec<Diagnostic> {
    let rule = match rules.iter().find(|r| r.id == "unused-import") {
        Some(r) => r,
        None => return vec![],
    };

    let mut diagnostics = Vec::new();

    // Pass 1: collect all named imports.
    let mut matches = cursor.matches(&rule.query, tree.root_node(), source.as_bytes());
    let mut imports: Vec<(String, usize, usize)> = Vec::new();
    while let Some(m) = matches.next() {
        let captures: Vec<(&str, Node)> = m
            .captures
            .iter()
            .map(|c| {
                let name = rule.query.capture_names()[c.index as usize];
                (name, c.node)
            })
            .collect();
        if let (Some(name_node), Some(import_node)) = (
            find_capture(&captures, "imported_name"),
            find_capture(&captures, "import"),
        ) {
            let name = node_text(name_node, source).to_string();
            imports.push((name, import_node.start_byte(), import_node.end_byte()));
        }
    }

    if imports.is_empty() {
        return diagnostics;
    }

    // Pass 2: collect all identifier usages (outside import directives).
    let mut used_names = rustc_hash::FxHashSet::default();
    collect_identifiers(tree.root_node(), source, &mut used_names);

    for (name, start_byte, end_byte) in &imports {
        if !used_names.contains(name.as_str()) {
            diagnostics.push(Diagnostic {
                range: line_index.byte_range_to_lsp_range(source, *start_byte, *end_byte),
                severity: Some(rule.severity),
                code: Some(NumberOrString::String(rule.id.to_string())),
                source: Some("ts-lint".into()),
                message: format!("[lint] import `{name}` is never used"),
                ..Default::default()
            });
        }
    }

    diagnostics
}

fn collect_identifiers<'a>(
    node: Node<'a>,
    source: &'a str,
    used: &mut rustc_hash::FxHashSet<&'a str>,
) {
    if node.kind() == "import_directive" {
        return;
    }

    if node.kind() == "identifier" {
        let text = &source[node.start_byte()..node.end_byte()];
        used.insert(text);
    }

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_identifiers(cursor.node(), source, used);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dead code detection (symbol-table-aware)
// ---------------------------------------------------------------------------

/// Detect unused internal/private functions and private state variables.
/// Must be called after the symbol table has been populated and references resolved.
pub fn check_dead_code(
    st: &crate::symbol_table::SymbolTable,
    file: &std::path::Path,
    source: &str,
    line_index: &crate::utils::LineIndex,
) -> Vec<Diagnostic> {
    use crate::symbol_table::{DeclKind, SYNTHETIC_BASE, ScopeKind};

    let file_id = match st.lookup_file_id(file) {
        Some(id) => id,
        None => return vec![],
    };
    let fi = match st.files.get(&file_id) {
        Some(fi) => fi,
        None => return vec![],
    };

    let mut diagnostics = Vec::new();

    for decl in fi.declarations.values() {
        // Skip synthetic built-in declarations.
        if decl.id.byte_offset >= SYNTHETIC_BASE {
            continue;
        }

        match decl.kind() {
            DeclKind::Function => {
                // Only flag internal/private functions (not external/public).
                let vis = decl.visibility().unwrap_or("internal");
                if vis == "external" || vis == "public" {
                    continue;
                }
                // Skip constructors, fallbacks, etc. (they have DeclKind::Constructor etc.)
                // Skip functions in interfaces (all are implicitly external).
                let scope = match fi.scopes.get(decl.scope) {
                    Some(s) => s,
                    None => continue,
                };
                if scope.kind == ScopeKind::Interface {
                    continue;
                }

                // Check if this function has any references.
                let refs = st.find_references(&decl.id);
                if refs.is_empty() {
                    diagnostics.push(Diagnostic {
                        range: line_index.byte_range_to_lsp_range(
                            source,
                            decl.name_range.0,
                            decl.name_range.1,
                        ),
                        severity: Some(DiagnosticSeverity::HINT),
                        code: Some(NumberOrString::String("dead-code".to_string())),
                        source: Some("ts-lint".into()),
                        message: format!(
                            "[lint] function `{}` is declared but never used",
                            decl.name
                        ),
                        tags: Some(vec![DiagnosticTag::UNNECESSARY]),
                        ..Default::default()
                    });
                }
            }
            DeclKind::StateVariable => {
                // Only flag private state variables (not public ones which
                // generate getters and may be used externally).
                let vis = decl.visibility().unwrap_or("internal");
                if vis == "public" {
                    continue;
                }
                // Skip constants and immutables — they are often used as
                // configuration values and are cheap, so flagging them is noisy.
                if decl.is_constant() || decl.is_immutable() {
                    continue;
                }

                let refs = st.find_references(&decl.id);
                if refs.is_empty() {
                    diagnostics.push(Diagnostic {
                        range: line_index.byte_range_to_lsp_range(
                            source,
                            decl.name_range.0,
                            decl.name_range.1,
                        ),
                        severity: Some(DiagnosticSeverity::HINT),
                        code: Some(NumberOrString::String("dead-code".to_string())),
                        source: Some("ts-lint".into()),
                        message: format!(
                            "[lint] state variable `{}` is declared but never used",
                            decl.name
                        ),
                        tags: Some(vec![DiagnosticTag::UNNECESSARY]),
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
    }

    diagnostics
}

// ---------------------------------------------------------------------------
// Rule registration
// ---------------------------------------------------------------------------

fn build_rules(lang: &tree_sitter::Language) -> Vec<LintRule> {
    let mut rules = Vec::new();

    macro_rules! add_rule {
        ($id:expr, $desc:expr, $sev:expr, $query_str:expr, $range_cap:expr, $filter:expr) => {
            add_rule!($id, $desc, $sev, $query_str, $range_cap, $filter, false)
        };
        ($id:expr, $desc:expr, $sev:expr, $query_str:expr, $range_cap:expr, $filter:expr, $skip:expr) => {
            match Query::new(lang, $query_str) {
                Ok(query) => {
                    rules.push(LintRule {
                        id: $id,
                        description: $desc,
                        severity: $sev,
                        query,
                        range_capture: $range_cap,
                        filter: $filter,
                        skip_main_loop: $skip,
                    });
                }
                Err(e) => {
                    tracing::warn!("Failed to compile lint query for {}: {e}", $id);
                }
            }
        };
    }

    // HIGH
    add_rule!(
        "incorrect-shift",
        "shift operands may be reversed",
        DiagnosticSeverity::WARNING,
        include_str!("queries/incorrect_shift.scm"),
        "expr",
        Some(filter_incorrect_shift)
    );

    add_rule!(
        "unchecked-call",
        "low-level call without checking success return value",
        DiagnosticSeverity::WARNING,
        include_str!("queries/unchecked_call.scm"),
        "stmt",
        None
    );

    // MEDIUM
    add_rule!(
        "divide-before-multiply",
        "division before multiplication may cause precision loss",
        DiagnosticSeverity::WARNING,
        include_str!("queries/divide_before_multiply.scm"),
        "outer",
        Some(filter_divide_before_multiply)
    );

    // INFO - naming conventions
    add_rule!(
        "pascal-case-struct",
        "struct names should be PascalCase",
        DiagnosticSeverity::INFORMATION,
        include_str!("queries/pascal_case_struct.scm"),
        "name",
        Some(filter_pascal_case_struct)
    );

    add_rule!(
        "mixed-case-function",
        "function names should be mixedCase",
        DiagnosticSeverity::INFORMATION,
        include_str!("queries/mixed_case_function.scm"),
        "name",
        Some(filter_mixed_case_function)
    );

    add_rule!(
        "mixed-case-variable",
        "mutable variable names should be mixedCase",
        DiagnosticSeverity::INFORMATION,
        include_str!("queries/mixed_case_variable.scm"),
        "name",
        Some(filter_mixed_case_variable)
    );

    add_rule!(
        "screaming-snake-case-const",
        "constants should be SCREAMING_SNAKE_CASE",
        DiagnosticSeverity::INFORMATION,
        include_str!("queries/screaming_snake_const.scm"),
        "name",
        Some(filter_screaming_snake_const)
    );

    add_rule!(
        "screaming-snake-case-immutable",
        "immutables should be SCREAMING_SNAKE_CASE",
        DiagnosticSeverity::INFORMATION,
        include_str!("queries/screaming_snake_immutable.scm"),
        "name",
        Some(filter_screaming_snake_immutable)
    );

    // unused-import: handled by dedicated two-pass checker, skip main loop. (Fix #6)
    add_rule!(
        "unused-import",
        "import is never used",
        DiagnosticSeverity::INFORMATION,
        include_str!("queries/unused_import.scm"),
        "import",
        None,
        true // skip_main_loop
    );

    add_rule!(
        "unaliased-plain-import",
        "use named imports instead of plain import",
        DiagnosticSeverity::INFORMATION,
        include_str!("queries/unaliased_plain_import.scm"),
        "import",
        Some(filter_unaliased_plain_import)
    );

    add_rule!(
        "unsafe-cheatcode",
        "potentially unsafe cheatcode usage",
        DiagnosticSeverity::INFORMATION,
        include_str!("queries/unsafe_cheatcode.scm"),
        "call",
        Some(filter_unsafe_cheatcode)
    );

    // GAS
    add_rule!(
        "custom-errors",
        "use custom errors instead of require/revert with strings",
        DiagnosticSeverity::HINT,
        include_str!("queries/custom_errors.scm"),
        "call",
        Some(filter_custom_errors)
    );

    rules
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::TsParser;

    fn lint(source: &str) -> Vec<Diagnostic> {
        let mut parser = TsParser::new();
        let tree = parser.parse(source, None).unwrap();
        let engine = LintEngine::new();
        let line_index = crate::utils::LineIndex::new(source);
        engine.run(&tree, source, &line_index)
    }

    fn lint_ids(source: &str) -> Vec<String> {
        lint(source)
            .into_iter()
            .filter_map(|d| {
                d.code.map(|c| match c {
                    NumberOrString::String(s) => s,
                    NumberOrString::Number(n) => n.to_string(),
                })
            })
            .collect()
    }

    #[test]
    fn test_incorrect_shift() {
        let source = r#"
contract Foo {
    function bar() public {
        uint256 x = 256 << y;
    }
}
"#;
        let ids = lint_ids(source);
        assert!(ids.contains(&"incorrect-shift".to_string()), "ids: {ids:?}");
    }

    #[test]
    fn test_correct_shift_no_flag() {
        let source = r#"
contract Foo {
    function bar() public {
        uint256 x = y << 256;
    }
}
"#;
        let ids = lint_ids(source);
        assert!(
            !ids.contains(&"incorrect-shift".to_string()),
            "ids: {ids:?}"
        );
    }

    #[test]
    fn test_unchecked_call() {
        let source = r#"
contract Foo {
    function bar(address a) external {
        a.call("");
    }
}
"#;
        let ids = lint_ids(source);
        assert!(ids.contains(&"unchecked-call".to_string()), "ids: {ids:?}");
    }

    #[test]
    fn test_checked_call_no_flag() {
        let source = r#"
contract Foo {
    function bar(address a) external {
        (bool success, ) = a.call("");
        require(success);
    }
}
"#;
        let ids = lint_ids(source);
        assert!(!ids.contains(&"unchecked-call".to_string()), "ids: {ids:?}");
    }

    #[test]
    fn test_divide_before_multiply() {
        let source = r#"
contract Foo {
    function bar() public {
        uint256 x = (10 / 5) * 3;
    }
}
"#;
        let ids = lint_ids(source);
        assert!(
            ids.contains(&"divide-before-multiply".to_string()),
            "ids: {ids:?}"
        );
    }

    #[test]
    fn test_pascal_case_struct() {
        let source = r#"
contract Foo {
    struct myStruct { uint256 x; }
    struct MyStruct { uint256 x; }
}
"#;
        let ids = lint_ids(source);
        assert!(
            ids.contains(&"pascal-case-struct".to_string()),
            "ids: {ids:?}"
        );
        assert_eq!(
            ids.iter().filter(|id| *id == "pascal-case-struct").count(),
            1
        );
    }

    #[test]
    fn test_mixed_case_function() {
        let source = r#"
contract Foo {
    function BadName() public {}
    function goodName() public {}
}
"#;
        let ids = lint_ids(source);
        assert!(
            ids.contains(&"mixed-case-function".to_string()),
            "ids: {ids:?}"
        );
        assert_eq!(
            ids.iter().filter(|id| *id == "mixed-case-function").count(),
            1
        );
    }

    #[test]
    fn test_mixed_case_function_test_excluded() {
        let source = r#"
contract Foo {
    function testSomething() public {}
    function invariant_check() public {}
}
"#;
        let ids = lint_ids(source);
        assert!(
            !ids.contains(&"mixed-case-function".to_string()),
            "ids: {ids:?}"
        );
    }

    #[test]
    fn test_screaming_snake_const() {
        let source = r#"
contract Foo {
    uint256 public constant badConst = 1;
    uint256 public constant GOOD_CONST = 1;
}
"#;
        let ids = lint_ids(source);
        assert!(
            ids.contains(&"screaming-snake-case-const".to_string()),
            "ids: {ids:?}"
        );
        assert_eq!(
            ids.iter()
                .filter(|id| *id == "screaming-snake-case-const")
                .count(),
            1
        );
    }

    #[test]
    fn test_screaming_snake_immutable() {
        let source = r#"
contract Foo {
    uint256 public immutable badImm;
    uint256 public immutable GOOD_IMM;
}
"#;
        let ids = lint_ids(source);
        assert!(
            ids.contains(&"screaming-snake-case-immutable".to_string()),
            "ids: {ids:?}"
        );
        assert_eq!(
            ids.iter()
                .filter(|id| *id == "screaming-snake-case-immutable")
                .count(),
            1
        );
    }

    #[test]
    fn test_mixed_case_variable_skips_const() {
        let source = r#"
contract Foo {
    uint256 public constant MAX_VALUE = 100;
    uint256 public immutable MIN_VALUE;
    uint256 public mutableVar;
}
"#;
        let ids = lint_ids(source);
        assert!(
            !ids.iter().any(|id| id == "mixed-case-variable"),
            "ids: {ids:?}"
        );
    }

    #[test]
    fn test_unaliased_plain_import() {
        let source = r#"
import "foo.sol";
import {Bar} from "bar.sol";
"#;
        let ids = lint_ids(source);
        assert!(
            ids.contains(&"unaliased-plain-import".to_string()),
            "ids: {ids:?}"
        );
        assert_eq!(
            ids.iter()
                .filter(|id| *id == "unaliased-plain-import")
                .count(),
            1
        );
    }

    #[test]
    fn test_custom_errors_require() {
        let source = r#"
contract Foo {
    function bar() public {
        require(true, "bad");
    }
}
"#;
        let ids = lint_ids(source);
        assert!(ids.contains(&"custom-errors".to_string()), "ids: {ids:?}");
    }

    #[test]
    fn test_custom_errors_require_no_string() {
        let source = r#"
contract Foo {
    function bar() public {
        require(true);
    }
}
"#;
        let ids = lint_ids(source);
        assert!(!ids.contains(&"custom-errors".to_string()), "ids: {ids:?}");
    }

    #[test]
    fn test_unused_import() {
        let source = r#"
import {Unused} from "foo.sol";
import {Used} from "bar.sol";

contract Foo {
    Used public x;
}
"#;
        let ids = lint_ids(source);
        assert!(ids.contains(&"unused-import".to_string()), "ids: {ids:?}");
        assert_eq!(ids.iter().filter(|id| *id == "unused-import").count(), 1);
    }

    // Dead code detection tests (require symbol table)

    fn dead_code_ids(source: &str) -> Vec<String> {
        let mut parser = TsParser::new();
        let path = std::path::PathBuf::from("/tmp/test.sol");
        let resolver =
            crate::import_resolver::ImportResolver::with_root(std::path::PathBuf::from("/tmp"));
        let mut st = crate::symbol_table::SymbolTable::new(resolver);
        st.index_file(&path, source, &mut parser);
        st.resolve_file_references(&path, &mut parser);
        let line_index = crate::utils::LineIndex::new(source);
        super::check_dead_code(&st, &path, source, &line_index)
            .into_iter()
            .filter_map(|d| {
                d.code.map(|c| match c {
                    NumberOrString::String(s) => s,
                    NumberOrString::Number(n) => n.to_string(),
                })
            })
            .collect()
    }

    fn dead_code_messages(source: &str) -> Vec<String> {
        let mut parser = TsParser::new();
        let path = std::path::PathBuf::from("/tmp/test.sol");
        let resolver =
            crate::import_resolver::ImportResolver::with_root(std::path::PathBuf::from("/tmp"));
        let mut st = crate::symbol_table::SymbolTable::new(resolver);
        st.index_file(&path, source, &mut parser);
        st.resolve_file_references(&path, &mut parser);
        let line_index = crate::utils::LineIndex::new(source);
        super::check_dead_code(&st, &path, source, &line_index)
            .into_iter()
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn test_dead_code_unused_internal_function() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function _unused() internal pure returns (uint256) {
        return 42;
    }

    function bar() public pure returns (uint256) {
        return 1;
    }
}
"#;
        let ids = dead_code_ids(source);
        assert!(
            ids.contains(&"dead-code".to_string()),
            "should flag unused internal function, got: {ids:?}"
        );
        let msgs = dead_code_messages(source);
        assert!(
            msgs.iter().any(|m| m.contains("_unused")),
            "should mention _unused, got: {msgs:?}"
        );
    }

    #[test]
    fn test_dead_code_used_internal_function_not_flagged() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function _helper() internal pure returns (uint256) {
        return 42;
    }

    function bar() public pure returns (uint256) {
        return _helper();
    }
}
"#;
        let msgs = dead_code_messages(source);
        assert!(
            !msgs.iter().any(|m| m.contains("_helper")),
            "used internal function should not be flagged, got: {msgs:?}"
        );
    }

    #[test]
    fn test_dead_code_public_function_not_flagged() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure returns (uint256) {
        return 1;
    }
}
"#;
        let ids = dead_code_ids(source);
        assert!(
            ids.is_empty(),
            "public functions should not be flagged as dead code, got: {ids:?}"
        );
    }

    #[test]
    fn test_dead_code_unused_private_state_variable() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 private _unused;

    function bar() public pure returns (uint256) {
        return 1;
    }
}
"#;
        let msgs = dead_code_messages(source);
        assert!(
            msgs.iter().any(|m| m.contains("_unused")),
            "should flag unused private state variable, got: {msgs:?}"
        );
    }

    #[test]
    fn test_dead_code_public_state_variable_not_flagged() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public totalSupply;
}
"#;
        let ids = dead_code_ids(source);
        assert!(
            !ids.iter().any(|id| id == "dead-code"),
            "public state variables should not be flagged, got: {ids:?}"
        );
    }

    #[test]
    fn test_dead_code_used_state_variable_not_flagged() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 private _count;

    function increment() public {
        _count += 1;
    }
}
"#;
        let msgs = dead_code_messages(source);
        assert!(
            !msgs.iter().any(|m| m.contains("_count")),
            "used state variable should not be flagged, got: {msgs:?}"
        );
    }
}
