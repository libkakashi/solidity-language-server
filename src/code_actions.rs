use tower_lsp::lsp_types::*;

use crate::symbol_table::SymbolTable;
use crate::utils::LineIndex;
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Produce code actions for diagnostics overlapping the requested range.
pub fn code_actions(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    _range: Range,
    diagnostics: &[Diagnostic],
    line_index: &LineIndex,
    uri: &Url,
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();
    for diag in diagnostics {
        let code = match &diag.code {
            Some(NumberOrString::String(s)) => s.as_str(),
            _ => continue,
        };
        match code {
            "unused-import" => {
                if let Some(a) = action_remove_unused_import(source, diag, line_index, uri) {
                    actions.push(CodeActionOrCommand::CodeAction(a));
                }
            }
            "unaliased-plain-import" => {
                if let Some(a) =
                    action_convert_to_named_import(st, file, source, diag, line_index, uri)
                {
                    actions.push(CodeActionOrCommand::CodeAction(a));
                }
            }
            "pascal-case-struct" => {
                if let Some(a) =
                    action_rename_to_convention(source, diag, line_index, uri, to_pascal_case)
                {
                    actions.push(CodeActionOrCommand::CodeAction(a));
                }
            }
            "mixed-case-function" | "mixed-case-variable" => {
                if let Some(a) =
                    action_rename_to_convention(source, diag, line_index, uri, to_mixed_case)
                {
                    actions.push(CodeActionOrCommand::CodeAction(a));
                }
            }
            "screaming-snake-case-const" | "screaming-snake-case-immutable" => {
                if let Some(a) = action_rename_to_convention(
                    source,
                    diag,
                    line_index,
                    uri,
                    to_screaming_snake_case,
                ) {
                    actions.push(CodeActionOrCommand::CodeAction(a));
                }
            }
            "incorrect-shift" => {
                if let Some(a) = action_swap_shift_operands(source, diag, line_index, uri) {
                    actions.push(CodeActionOrCommand::CodeAction(a));
                }
            }
            "custom-errors" => {
                if let Some(a) = action_use_custom_error(source, diag, line_index, uri) {
                    actions.push(CodeActionOrCommand::CodeAction(a));
                }
            }
            _ => {}
        }
    }

    // Check solar diagnostics for undeclared identifiers (auto-import).
    for diag in diagnostics {
        if diag.source.as_deref() != Some("solar") {
            continue;
        }
        if let Some(a) = action_auto_import(st, file, source, diag, line_index, uri) {
            actions.push(CodeActionOrCommand::CodeAction(a));
        }
    }

    actions
}

// ---------------------------------------------------------------------------
// Quick fix: remove unused import
// ---------------------------------------------------------------------------

fn action_remove_unused_import(
    source: &str,
    diag: &Diagnostic,
    line_index: &LineIndex,
    uri: &Url,
) -> Option<CodeAction> {
    // The diagnostic range covers the entire import directive.
    // We delete the whole line (including trailing newline).
    let start_byte = line_index.position_to_byte_offset(
        source,
        diag.range.start.line,
        diag.range.start.character,
    );
    let end_byte =
        line_index.position_to_byte_offset(source, diag.range.end.line, diag.range.end.character);
    let import_text = &source[start_byte..end_byte];

    // Check if this is a named import with multiple symbols: `import {A, B} from "..."`
    // If so, only remove the unused symbol rather than the entire import.
    if let Some(brace_start) = import_text.find('{') {
        if let Some(brace_end) = import_text.find('}') {
            let symbols_text = &import_text[brace_start + 1..brace_end];
            let symbols: Vec<&str> = symbols_text.split(',').map(|s| s.trim()).collect();
            if symbols.len() > 1 {
                // Extract the unused name from the diagnostic message.
                let unused_name = extract_name_from_message(&diag.message, "import `", "`")?;
                return action_remove_single_named_import(
                    source,
                    diag,
                    start_byte,
                    import_text,
                    &unused_name,
                    line_index,
                    uri,
                );
            }
        }
    }

    // Remove the entire import line.
    let line_start = source[..start_byte].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end_byte = source[end_byte..]
        .find('\n')
        .map(|i| end_byte + i + 1)
        .unwrap_or(source.len());

    let delete_range = Range {
        start: line_index.byte_offset_to_lsp_position(source, line_start),
        end: line_index.byte_offset_to_lsp_position(source, line_end_byte),
    };

    let mut changes = HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range: delete_range,
            new_text: String::new(),
        }],
    );

    Some(CodeAction {
        title: "Remove unused import".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        is_preferred: Some(true),
        ..Default::default()
    })
}

/// Remove a single symbol from a multi-symbol named import.
fn action_remove_single_named_import(
    source: &str,
    diag: &Diagnostic,
    import_start_byte: usize,
    import_text: &str,
    unused_name: &str,
    line_index: &LineIndex,
    uri: &Url,
) -> Option<CodeAction> {
    let brace_start = import_text.find('{')?;
    let brace_end = import_text.find('}')?;
    let symbols_text = &import_text[brace_start + 1..brace_end];

    // Rebuild the symbol list without the unused name.
    let symbols: Vec<&str> = symbols_text
        .split(',')
        .map(|s| s.trim())
        .filter(|s| {
            // Handle `Name as Alias` — check both the original name and alias.
            let parts: Vec<&str> = s.split_whitespace().collect();
            let original = parts.first().copied().unwrap_or(s);
            let alias = if parts.len() >= 3 && parts[1] == "as" {
                parts[2]
            } else {
                original
            };
            alias != unused_name && original != unused_name
        })
        .collect();

    if symbols.is_empty() {
        // All symbols removed — remove entire import.
        return action_remove_unused_import(source, diag, line_index, uri);
    }

    let new_symbols = symbols.join(", ");
    let new_import_segment = format!("{{{new_symbols}}}");

    let abs_brace_start = import_start_byte + brace_start;
    let abs_brace_end = import_start_byte + brace_end + 1;

    let edit_range = Range {
        start: line_index.byte_offset_to_lsp_position(source, abs_brace_start),
        end: line_index.byte_offset_to_lsp_position(source, abs_brace_end),
    };

    let mut changes = HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range: edit_range,
            new_text: new_import_segment,
        }],
    );

    Some(CodeAction {
        title: format!("Remove unused import `{unused_name}`"),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        is_preferred: Some(true),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Quick fix: convert plain import to named import
// ---------------------------------------------------------------------------

fn action_convert_to_named_import(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    diag: &Diagnostic,
    line_index: &LineIndex,
    uri: &Url,
) -> Option<CodeAction> {
    let start_byte = line_index.position_to_byte_offset(
        source,
        diag.range.start.line,
        diag.range.start.character,
    );
    let end_byte =
        line_index.position_to_byte_offset(source, diag.range.end.line, diag.range.end.character);

    let import_text = &source[start_byte..end_byte];

    // Extract the path from the import statement.
    let path_start = import_text.find('"').or_else(|| import_text.find('\''))?;
    let quote_char = import_text.as_bytes()[path_start] as char;
    let path_end = import_text[path_start + 1..].find(quote_char)? + path_start + 1;
    let import_path = &import_text[path_start + 1..path_end];

    // Look up what the imported file exports.
    let fi = st.get_file_index(file)?;
    let import_info = fi
        .imports
        .iter()
        .find(|imp| imp.source_path == import_path)?;
    let resolved = import_info.resolved_path.as_ref()?;
    let target_fid = st.lookup_file_id(resolved)?;
    let target_fi = st.files.get(&target_fid)?;

    // Collect exported top-level names.
    let mut exports: Vec<&str> = target_fi.top_level_names().map(|s| s.as_str()).collect();
    exports.sort_unstable();

    if exports.is_empty() {
        return None;
    }

    let names = exports.join(", ");
    let new_text = format!("import {{{names}}} from {quote_char}{import_path}{quote_char};",);

    let edit_range = Range {
        start: line_index.byte_offset_to_lsp_position(source, start_byte),
        end: line_index.byte_offset_to_lsp_position(source, end_byte),
    };

    let mut changes = HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range: edit_range,
            new_text,
        }],
    );

    Some(CodeAction {
        title: "Convert to named import".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        is_preferred: Some(true),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Quick fix: rename to naming convention
// ---------------------------------------------------------------------------

fn action_rename_to_convention(
    source: &str,
    diag: &Diagnostic,
    line_index: &LineIndex,
    uri: &Url,
    convert: fn(&str) -> String,
) -> Option<CodeAction> {
    // The diagnostic range covers just the identifier name.
    let start_byte = line_index.position_to_byte_offset(
        source,
        diag.range.start.line,
        diag.range.start.character,
    );
    let end_byte =
        line_index.position_to_byte_offset(source, diag.range.end.line, diag.range.end.character);

    let current_name = &source[start_byte..end_byte];
    let new_name = convert(current_name);

    if new_name == current_name || new_name.is_empty() {
        return None;
    }

    let mut changes = HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range: diag.range,
            new_text: new_name.clone(),
        }],
    );

    Some(CodeAction {
        title: format!("Rename to `{new_name}`"),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        is_preferred: Some(false),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Quick fix: swap shift operands
// ---------------------------------------------------------------------------

fn action_swap_shift_operands(
    source: &str,
    diag: &Diagnostic,
    line_index: &LineIndex,
    uri: &Url,
) -> Option<CodeAction> {
    let start_byte = line_index.position_to_byte_offset(
        source,
        diag.range.start.line,
        diag.range.start.character,
    );
    let end_byte =
        line_index.position_to_byte_offset(source, diag.range.end.line, diag.range.end.character);

    let expr_text = source.get(start_byte..end_byte)?;

    // Find the shift operator (>> or <<).
    let (op, op_pos) = if let Some(pos) = expr_text.find("<<") {
        ("<<", pos)
    } else if let Some(pos) = expr_text.find(">>") {
        (">>", pos)
    } else {
        return None;
    };

    let lhs = expr_text[..op_pos].trim();
    let rhs = expr_text[op_pos + 2..].trim();

    let new_text = format!("{rhs} {op} {lhs}");

    let mut changes = HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range: diag.range,
            new_text,
        }],
    );

    Some(CodeAction {
        title: "Swap shift operands".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        is_preferred: Some(true),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Quick fix: replace require(cond, "msg") with custom error pattern
// ---------------------------------------------------------------------------

fn action_use_custom_error(
    source: &str,
    diag: &Diagnostic,
    line_index: &LineIndex,
    uri: &Url,
) -> Option<CodeAction> {
    let start_byte = line_index.position_to_byte_offset(
        source,
        diag.range.start.line,
        diag.range.start.character,
    );
    let end_byte =
        line_index.position_to_byte_offset(source, diag.range.end.line, diag.range.end.character);

    let call_text = source.get(start_byte..end_byte)?;

    // Parse: require(condition, "message") → if (!condition) revert CustomError();
    // or:   revert("message") → revert CustomError();
    if call_text.starts_with("require") {
        let inner = extract_parens_content(call_text)?;
        // Split on the first comma that's not inside nested parens.
        let comma_pos = find_top_level_comma(inner)?;
        let condition = inner[..comma_pos].trim();
        let new_text = format!("if (!({condition})) revert CustomError()");

        let mut changes = HashMap::new();
        changes.insert(
            uri.clone(),
            vec![TextEdit {
                range: diag.range,
                new_text,
            }],
        );

        Some(CodeAction {
            title: "Replace with custom error".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diag.clone()]),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            is_preferred: Some(false),
            ..Default::default()
        })
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Quick fix: auto-import for undeclared identifiers
// ---------------------------------------------------------------------------

fn action_auto_import(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    diag: &Diagnostic,
    line_index: &LineIndex,
    uri: &Url,
) -> Option<CodeAction> {
    // Match solar error messages about undeclared identifiers.
    let msg = &diag.message;
    if !msg.contains("undeclared identifier")
        && !msg.contains("not found")
        && !msg.contains("not declared")
    {
        return None;
    }

    // Extract the symbol name from the diagnostic range.
    let start_byte = line_index.position_to_byte_offset(
        source,
        diag.range.start.line,
        diag.range.start.character,
    );
    let end_byte =
        line_index.position_to_byte_offset(source, diag.range.end.line, diag.range.end.character);
    let symbol_name = source.get(start_byte..end_byte)?.trim();

    if symbol_name.is_empty()
        || !symbol_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }

    let current_file_id = st.lookup_file_id(file)?;

    // Search all indexed files for a top-level declaration with this name.
    for (&file_id, fi) in &st.files {
        if file_id == current_file_id {
            continue;
        }
        // Check if this file exports the symbol.
        if !fi.top_level_names().any(|n| n == symbol_name) {
            continue;
        }
        let target_path = st.resolve_path(file_id);

        // Compute relative import path.
        let import_path = compute_relative_import(file, target_path)?;

        // Find where to insert the import (after the last import or after pragma).
        let insert_pos = find_import_insert_position(source);
        let insert_lsp = line_index.byte_offset_to_lsp_position(source, insert_pos);

        let new_text = format!("import {{{symbol_name}}} from \"{import_path}\";\n");

        let mut changes = HashMap::new();
        changes.insert(
            uri.clone(),
            vec![TextEdit {
                range: Range {
                    start: insert_lsp,
                    end: insert_lsp,
                },
                new_text,
            }],
        );

        return Some(CodeAction {
            title: format!("Import `{symbol_name}` from \"{import_path}\""),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diag.clone()]),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            is_preferred: Some(false),
            ..Default::default()
        });
    }

    None
}

/// Compute a relative import path from `from_file` to `to_file`.
fn compute_relative_import(from_file: &Path, to_file: &Path) -> Option<String> {
    let from_dir = from_file.parent()?;
    let to_dir = to_file.parent()?;
    let to_name = to_file.file_name()?.to_str()?;

    // Find common prefix.
    let from_components: Vec<_> = from_dir.components().collect();
    let to_components: Vec<_> = to_dir.components().collect();

    let common = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let ups = from_components.len() - common;
    let mut parts = Vec::new();
    if ups == 0 {
        parts.push(".".to_string());
    } else {
        for _ in 0..ups {
            parts.push("..".to_string());
        }
    }

    for comp in &to_components[common..] {
        parts.push(comp.as_os_str().to_str()?.to_string());
    }

    parts.push(to_name.to_string());
    Some(parts.join("/"))
}

/// Find the byte position where a new import should be inserted.
/// Prefers after the last existing import, or after the pragma line.
fn find_import_insert_position(source: &str) -> usize {
    let mut last_import_end = None;
    let mut pragma_end = None;

    for (i, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("import ") || trimmed.starts_with("import{") {
            // Find end of this line in bytes.
            let line_start: usize = source.lines().take(i).map(|l| l.len() + 1).sum();
            last_import_end = Some(line_start + line.len() + 1);
        }
        if trimmed.starts_with("pragma ") {
            let line_start: usize = source.lines().take(i).map(|l| l.len() + 1).sum();
            pragma_end = Some(line_start + line.len() + 1);
        }
    }

    last_import_end
        .or(pragma_end)
        .unwrap_or(0)
        .min(source.len())
}

// ---------------------------------------------------------------------------
// Naming convention converters
// ---------------------------------------------------------------------------

fn to_pascal_case(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut capitalize_next = true;
    for ch in name.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

fn to_mixed_case(name: &str) -> String {
    // Preserve leading underscores.
    let leading = name.len() - name.trim_start_matches('_').len();
    let prefix = &name[..leading];
    let rest = &name[leading..];

    if rest.is_empty() {
        return name.to_string();
    }

    // If already camelCase-ish but starts with uppercase, just lowercase the first char.
    if !rest.contains('_') {
        let mut result = String::with_capacity(name.len());
        result.push_str(prefix);
        let mut chars = rest.chars();
        if let Some(first) = chars.next() {
            result.extend(first.to_lowercase());
            result.extend(chars);
        }
        return result;
    }

    // SCREAMING_SNAKE or snake_case → camelCase.
    // Split on underscores, lowercase each segment, capitalize first letter of
    // all segments except the first.
    let mut result = String::with_capacity(name.len());
    result.push_str(prefix);
    for (i, segment) in rest.split('_').filter(|s| !s.is_empty()).enumerate() {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            if i == 0 {
                result.extend(first.to_lowercase());
            } else {
                result.extend(first.to_uppercase());
            }
            for ch in chars {
                result.extend(ch.to_lowercase());
            }
        }
    }
    result
}

fn to_screaming_snake_case(name: &str) -> String {
    // Preserve leading underscores.
    let leading = name.len() - name.trim_start_matches('_').len();
    let prefix = &name[..leading];
    let rest = &name[leading..];

    let trimmed = rest.trim_end_matches('_');

    let mut result = String::with_capacity(name.len() + 4);
    result.push_str(prefix);
    let mut prev_lower = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_uppercase() && prev_lower {
            result.push('_');
        }
        result.extend(ch.to_uppercase());
        prev_lower = ch.is_ascii_lowercase();
    }
    result
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_name_from_message<'a>(msg: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let start = msg.find(prefix)? + prefix.len();
    let rest = &msg[start..];
    let end = rest.find(suffix)?;
    Some(&rest[..end])
}

fn extract_parens_content(text: &str) -> Option<&str> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close > open + 1 {
        Some(&text[open + 1..close])
    } else {
        None
    }
}

fn find_top_level_comma(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in text.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("my_struct"), "MyStruct");
        assert_eq!(to_pascal_case("myStruct"), "MyStruct");
        assert_eq!(to_pascal_case("already_Correct"), "AlreadyCorrect");
    }

    #[test]
    fn test_to_mixed_case() {
        assert_eq!(to_mixed_case("BadName"), "badName");
        assert_eq!(to_mixed_case("snake_case"), "snakeCase");
        assert_eq!(to_mixed_case("_private"), "_private");
        assert_eq!(to_mixed_case("__double"), "__double");
        assert_eq!(to_mixed_case("UPPER_CASE"), "upperCase");
    }

    #[test]
    fn test_to_screaming_snake_case() {
        assert_eq!(to_screaming_snake_case("badConst"), "BAD_CONST");
        assert_eq!(to_screaming_snake_case("myValue"), "MY_VALUE");
        assert_eq!(to_screaming_snake_case("already_GOOD"), "ALREADY_GOOD");
        assert_eq!(to_screaming_snake_case("x"), "X");
    }

    #[test]
    fn test_extract_name_from_message() {
        let msg = "[lint] import `Unused` is never used";
        assert_eq!(
            extract_name_from_message(msg, "import `", "`"),
            Some("Unused")
        );
    }

    #[test]
    fn test_extract_parens_content() {
        assert_eq!(
            extract_parens_content("require(x > 0, \"bad\")"),
            Some("x > 0, \"bad\"")
        );
        assert_eq!(extract_parens_content("foo()"), None);
    }

    #[test]
    fn test_find_top_level_comma() {
        assert_eq!(find_top_level_comma("a, b"), Some(1));
        assert_eq!(find_top_level_comma("foo(a, b), c"), Some(9));
        assert_eq!(find_top_level_comma("no_comma"), None);
    }

    #[test]
    fn test_action_swap_shift_builds_correct_text() {
        // Test the swap logic directly.
        let expr = "256 << y";
        let op_pos = expr.find("<<").unwrap();
        let lhs = expr[..op_pos].trim();
        let rhs = expr[op_pos + 2..].trim();
        assert_eq!(format!("{rhs} << {lhs}"), "y << 256");
    }
}
