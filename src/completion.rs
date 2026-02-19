use std::path::Path;
use std::sync::OnceLock;

use rustc_hash::FxHashSet;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, Position,
};

use crate::parser::TsParser;
use crate::symbol_table::{
    BUILTIN_GLOBALS, BUILTIN_TYPES, DeclKind, MemberInfo, SYNTHETIC_BASE, ScopeKind, SymbolTable,
};
use crate::utils::LineIndex;

/// Handle a completion request.
pub fn handle_completion(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    trigger_char: Option<&str>,
    line_index: &LineIndex,
    cached_tree: Option<&tree_sitter::Tree>,
) -> Option<CompletionResponse> {
    // Handle empty/whitespace-only files gracefully.
    if source.is_empty() {
        return Some(CompletionResponse::List(CompletionList {
            is_incomplete: false,
            items: static_completions().to_vec(),
        }));
    }

    let abs_byte = line_index.position_to_byte_offset(source, position.line, position.character);
    let line_start_byte: usize = source[..abs_byte].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col_byte = (abs_byte - line_start_byte) as u32;
    let line = &source[line_start_byte
        ..source[line_start_byte..]
            .find('\n')
            .map(|i| line_start_byte + i)
            .unwrap_or(source.len())];

    // Use cached tree if available, otherwise parse for comment/string detection.
    let mut fallback_parser;
    let fallback_tree;
    let tree: Option<&tree_sitter::Tree> = if let Some(t) = cached_tree {
        Some(t)
    } else {
        fallback_parser = TsParser::new();
        fallback_tree = fallback_parser.parse(source, None);
        fallback_tree.as_ref()
    };

    // Suppress completion inside comments and string literals — except NatSpec.
    if let Some(t) = tree {
        match classify_comment_context(t, source, abs_byte) {
            CommentContext::NatSpec => {
                let items = get_natspec_completions(t, source, abs_byte, line, col_byte);
                return Some(CompletionResponse::List(CompletionList {
                    is_incomplete: false,
                    items,
                }));
            }
            CommentContext::Other => {
                return Some(CompletionResponse::List(CompletionList {
                    is_incomplete: false,
                    items: vec![],
                }));
            }
            CommentContext::None => {}
        }
    }

    // Check for import completion context.
    if let Some(items) = get_import_completions(st, file, abs_byte) {
        return Some(CompletionResponse::List(CompletionList {
            is_incomplete: false,
            items,
        }));
    }

    // Check for override specifier completion context.
    if let Some(items) = get_override_completions(st, file, source, abs_byte) {
        return Some(CompletionResponse::List(CompletionList {
            is_incomplete: false,
            items,
        }));
    }

    // Check for emit/revert contextual completion.
    if trigger_char != Some(".") {
        if let Some(items) = get_emit_revert_completions(st, file, abs_byte, line, col_byte) {
            return Some(CompletionResponse::List(CompletionList {
                is_incomplete: false,
                items,
            }));
        }
    }

    let items = if trigger_char == Some(".") {
        get_dot_completions(st, file, source, line, col_byte, abs_byte)
    } else {
        get_general_completions(st, file, abs_byte)
    };

    Some(CompletionResponse::List(CompletionList {
        is_incomplete: false,
        items,
    }))
}

// ---------------------------------------------------------------------------
// Comment / string detection
// ---------------------------------------------------------------------------

enum CommentContext {
    /// Not inside a comment or string.
    None,
    /// Inside a NatSpec comment (`///` or `/** */`).
    NatSpec,
    /// Inside a regular comment or string literal.
    Other,
}

/// Classify whether the cursor is inside a comment, and if so, whether it's
/// a NatSpec comment that should receive tag completions.
fn classify_comment_context(
    tree: &tree_sitter::Tree,
    source: &str,
    byte_offset: usize,
) -> CommentContext {
    if byte_offset == 0 {
        return CommentContext::None;
    }
    let check = byte_offset - 1;
    let node = match tree.root_node().descendant_for_byte_range(check, check) {
        Some(n) => n,
        None => return CommentContext::None,
    };
    let mut current = Some(node);
    while let Some(n) = current {
        match n.kind() {
            "comment" => {
                let text = &source[n.start_byte()..n.end_byte()];
                if text.starts_with("///") || text.starts_with("/**") {
                    return CommentContext::NatSpec;
                }
                return CommentContext::Other;
            }
            "string" | "string_literal" | "hex_string_literal" | "unicode_string_literal" => {
                return CommentContext::Other;
            }
            _ => {}
        }
        current = n.parent();
    }
    CommentContext::None
}

// ---------------------------------------------------------------------------
// NatSpec completion
// ---------------------------------------------------------------------------

/// Provide NatSpec tag completions inside `///` or `/** */` comments.
fn get_natspec_completions(
    tree: &tree_sitter::Tree,
    source: &str,
    byte_offset: usize,
    line: &str,
    col_byte: u32,
) -> Vec<CompletionItem> {
    let col = col_byte as usize;
    let before_cursor = &line[..col.min(line.len())];

    // Check if the user just typed '@' or is typing a tag name.
    let tag_prefix = if let Some(at_pos) = before_cursor.rfind('@') {
        // Only complete if there's no space between '@' and cursor (still typing the tag).
        let after_at = &before_cursor[at_pos + 1..];
        if after_at.chars().all(|c| c.is_ascii_alphanumeric()) {
            Some(after_at)
        } else {
            // Cursor is past the tag — check if we should complete param names.
            return get_natspec_param_name_completions(tree, source, byte_offset, before_cursor);
        }
    } else {
        // No '@' on this line — no NatSpec tag completion.
        return vec![];
    };

    let prefix = tag_prefix.unwrap_or("");

    // Collect parameter names for @param tag detail.
    let param_names = get_next_function_params(tree, source, byte_offset);

    let mut items = Vec::new();
    let tags: &[(&str, &str, &str)] = &[
        (
            "@notice",
            "notice",
            "Explains to an end user what this does",
        ),
        ("@dev", "dev", "Explains to a developer extra details"),
        ("@param", "param", "Documents a parameter"),
        ("@return", "return", "Documents the return value(s)"),
        (
            "@inheritdoc",
            "inheritdoc",
            "Inherits documentation from a base contract",
        ),
        ("@custom", "custom", "Custom tag (application-defined)"),
        (
            "@title",
            "title",
            "A title for the contract/interface/library",
        ),
        ("@author", "author", "The author of the contract"),
    ];

    for &(tag, tag_name, description) in tags {
        if !tag_name.starts_with(prefix) {
            continue;
        }
        let mut detail = description.to_string();
        if tag == "@param" && !param_names.is_empty() {
            detail = format!("{detail} — params: {}", param_names.join(", "));
        }
        items.push(CompletionItem {
            label: tag.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(detail),
            ..Default::default()
        });
    }

    items
}

/// After `@param `, suggest parameter names from the next function declaration.
fn get_natspec_param_name_completions(
    tree: &tree_sitter::Tree,
    source: &str,
    byte_offset: usize,
    before_cursor: &str,
) -> Vec<CompletionItem> {
    // Check if line contains `@param` followed by a partial identifier at cursor.
    let trimmed = before_cursor.trim_start_matches(|c: char| c == '/' || c == '*' || c == ' ');
    if !trimmed.starts_with("@param") {
        return vec![];
    }
    let after_param = &trimmed["@param".len()..];
    // Must have at least one space after @param.
    if !after_param.starts_with(' ') {
        return vec![];
    }
    let name_prefix = after_param.trim_start();

    let param_names = get_next_function_params(tree, source, byte_offset);
    param_names
        .into_iter()
        .filter(|name| name.starts_with(name_prefix))
        .map(|name| CompletionItem {
            label: name,
            kind: Some(CompletionItemKind::VARIABLE),
            detail: Some("parameter".to_string()),
            ..Default::default()
        })
        .collect()
}

/// Find the next function/event/error declaration after the current comment
/// and extract its parameter names.
fn get_next_function_params(
    tree: &tree_sitter::Tree,
    source: &str,
    byte_offset: usize,
) -> Vec<String> {
    // Find the comment node we're inside.
    if byte_offset == 0 {
        return vec![];
    }
    let comment_node = match tree
        .root_node()
        .descendant_for_byte_range(byte_offset - 1, byte_offset - 1)
    {
        Some(n) => {
            let mut node = n;
            while node.kind() != "comment" {
                match node.parent() {
                    Some(p) => node = p,
                    None => return vec![],
                }
            }
            node
        }
        None => return vec![],
    };

    // Walk forward from the comment to find the next non-comment sibling.
    let mut next = comment_node.next_named_sibling();
    while let Some(n) = next {
        if n.kind() == "comment" {
            next = n.next_named_sibling();
            continue;
        }
        // Found a non-comment node. Extract parameter names.
        return extract_param_names_from_node(n, source);
    }
    vec![]
}

/// Extract parameter names from a function/event/error/modifier declaration node.
fn extract_param_names_from_node(node: tree_sitter::Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut cursor = node.walk();

    // Look for parameter nodes within the declaration.
    for child in node.children(&mut cursor) {
        if child.kind() == "parameter" {
            // Parameter has a "name" field.
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = &source[name_node.start_byte()..name_node.end_byte()];
                if !name.is_empty() {
                    params.push(name.to_string());
                }
            }
        }
        // Also check inside parameter_list nodes.
        if child.kind().contains("parameter") && child.kind() != "parameter" {
            let mut inner = child.walk();
            for p in child.children(&mut inner) {
                if p.kind() == "parameter" {
                    if let Some(name_node) = p.child_by_field_name("name") {
                        let name = &source[name_node.start_byte()..name_node.end_byte()];
                        if !name.is_empty() {
                            params.push(name.to_string());
                        }
                    }
                }
            }
        }
    }
    params
}

// ---------------------------------------------------------------------------
// Emit / revert contextual completion
// ---------------------------------------------------------------------------

/// After `emit `, show only events. After `revert `, show only custom errors.
fn get_emit_revert_completions(
    st: &SymbolTable,
    file: &Path,
    byte_offset: usize,
    line: &str,
    col_byte: u32,
) -> Option<Vec<CompletionItem>> {
    let col = col_byte as usize;
    let before = line[..col.min(line.len())].trim_start();

    let (keyword, target_kind) = if let Some(rest) = before.strip_prefix("emit ") {
        // Allow partial identifier after "emit " (e.g. "emit Tr")
        if rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            ("emit", DeclKind::Event)
        } else {
            return None;
        }
    } else if let Some(rest) = before.strip_prefix("revert ") {
        if rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            ("revert", DeclKind::Error)
        } else {
            return None;
        }
    } else {
        return None;
    };

    let scope_id = st.scope_at(file, byte_offset).unwrap_or(0);
    let visible = st.visible_declarations(file, scope_id);

    let items: Vec<CompletionItem> = visible
        .iter()
        .filter(|decl| decl.kind() == target_kind)
        .map(|decl| CompletionItem {
            label: decl.name.clone(),
            kind: Some(decl_kind_to_completion_kind(decl.kind())),
            detail: decl.type_text().map(|s| s.to_string()),
            ..Default::default()
        })
        .collect();

    let _ = keyword; // used for clarity in the match above
    Some(items)
}

// ---------------------------------------------------------------------------
// Dot completion
// ---------------------------------------------------------------------------

fn get_dot_completions(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    line: &str,
    col_byte: u32,
    cursor_byte: usize,
) -> Vec<CompletionItem> {
    // Check for `type(X).` before anything else — extract_identifier_before_dot
    // would only extract "type" and miss the inner type argument.
    if let Some(inner_type) = extract_type_call_before_dot(line, col_byte) {
        return type_members_for(&inner_type, st, file);
    }

    // Check for `SomeType(args).` — type casts like `IERC20(token).`
    if let Some(type_name) = extract_call_before_dot(line, col_byte) {
        let scope = st.scope_at(file, cursor_byte).unwrap_or(0);
        return call_result_completions(st, file, &type_name, scope);
    }

    let identifier = match extract_identifier_before_dot(line, col_byte) {
        Some(id) => id,
        None => return vec![],
    };

    // Check magic globals first (msg, block, tx, abi, bytes, string).
    if let Some(items) = magic_members(&identifier) {
        return items;
    }

    let scope = st.scope_at(file, cursor_byte).unwrap_or(0);

    // Handle `this.` — show external/public functions of enclosing contract.
    if identifier == "this" {
        return this_completions(st, file, scope, source, cursor_byte);
    }

    // Handle `super.` — show members from parent contracts.
    if identifier == "super" {
        return super_completions(st, file, scope, source, cursor_byte);
    }

    // Look up the identifier in visible declarations.
    let visible = st.visible_declarations(file, scope);
    for decl in &visible {
        if decl.name != identifier {
            continue;
        }

        // Direct members (struct fields, contract functions, etc.)
        // For contracts/interfaces/libraries, also include inherited members.
        if matches!(
            decl.kind(),
            DeclKind::Contract | DeclKind::Interface | DeclKind::Library
        ) {
            let all = st.all_members_of(&decl.name, file);
            if !all.is_empty() {
                let mut items: Vec<CompletionItem> = all.iter().map(member_to_completion).collect();
                if let Some(tt) = decl.type_text() {
                    append_using_for(st, file, tt, &mut items);
                }
                return items;
            }
        }
        let members = decl.members();
        if !members.is_empty() {
            let mut items: Vec<CompletionItem> = members.iter().map(member_to_completion).collect();
            // Also include using-for methods if the decl has a type.
            if let Some(tt) = decl.type_text() {
                append_using_for(st, file, tt, &mut items);
            }
            return items;
        }

        // Enum values as fallback (enum_values might be populated even if
        // members isn't, depending on the indexing path).
        if decl.kind() == DeclKind::Enum {
            let vals = decl.enum_values();
            if !vals.is_empty() {
                return vals
                    .iter()
                    .map(|v| CompletionItem {
                        label: v.clone(),
                        kind: Some(CompletionItemKind::ENUM_MEMBER),
                        detail: Some(identifier.clone()),
                        ..Default::default()
                    })
                    .collect();
            }
        }

        // Type-based member lookup.
        if let Some(type_text) = decl.type_text() {
            // Built-in type members (arrays, address).
            if let Some(mut items) = builtin_type_members(type_text) {
                append_using_for(st, file, type_text, &mut items);
                return items;
            }
            // User-defined type members (including inherited).
            let members = st.all_members_of(type_text, file);
            if !members.is_empty() {
                let mut items: Vec<CompletionItem> =
                    members.iter().map(member_to_completion).collect();
                append_using_for(st, file, type_text, &mut items);
                return items;
            }
            // Only using-for methods.
            let mut items = Vec::new();
            append_using_for(st, file, type_text, &mut items);
            if !items.is_empty() {
                return items;
            }
        }
    }

    // Try members_of directly (e.g. "ContractName." or "EnumName.")
    let members = st.all_members_of(&identifier, file);
    if !members.is_empty() {
        return members.iter().map(member_to_completion).collect();
    }

    vec![]
}

/// Append using-for library methods that apply to `type_text`.
fn append_using_for(
    st: &SymbolTable,
    file: &Path,
    type_text: &str,
    items: &mut Vec<CompletionItem>,
) {
    let using_members = st.using_for_members(type_text, file);
    items.extend(using_members.iter().map(member_to_completion));
}

/// `this.` — show external/public functions of the enclosing contract
/// (including inherited ones).
fn this_completions(
    st: &SymbolTable,
    file: &Path,
    scope: usize,
    source: &str,
    cursor_byte: usize,
) -> Vec<CompletionItem> {
    let fi = match st.get_file_index(file) {
        Some(fi) => fi,
        None => return vec![],
    };

    // Try scope-based approach first (uses fi.declarations which have visibility).
    if let Some(contract) = find_enclosing_contract(st, file, scope) {
        let mut items: Vec<CompletionItem> = Vec::new();
        let mut seen = FxHashSet::default();

        // Collect own external/public functions from the contract's scope.
        for s in &fi.scopes {
            if s.owner == Some(contract.id) {
                for (name, did) in &s.declarations {
                    if let Some(d) = fi.declarations.get(did) {
                        if is_public_function(d) && seen.insert(name.clone()) {
                            items.push(decl_to_function_completion(d));
                        }
                    }
                }
                break;
            }
        }

        // Collect inherited external/public functions from base contracts.
        append_inherited_public_functions(st, fi, contract.base_contracts(), &mut items, &mut seen);

        if !items.is_empty() {
            return items;
        }
    }

    // Fallback for parse-error cases: find contract body range from text,
    // collect function declarations within it that are external/public.
    let mut items = this_completions_fallback(fi, source, cursor_byte);

    // Also add inherited members in the fallback path.
    if let Some(base_names) = extract_base_contracts_from_text(&source[..cursor_byte]) {
        let mut seen: FxHashSet<String> = items.iter().map(|i| i.label.clone()).collect();
        append_inherited_public_functions(st, fi, &base_names, &mut items, &mut seen);
    }

    items
}

fn is_public_function(d: &crate::symbol_table::Declaration) -> bool {
    d.kind() == DeclKind::Function
        && d.visibility()
            .map_or(false, |v| v == "external" || v == "public")
}

fn decl_to_function_completion(d: &crate::symbol_table::Declaration) -> CompletionItem {
    CompletionItem {
        label: d.name.clone(),
        kind: Some(CompletionItemKind::FUNCTION),
        detail: d.type_text().map(|s| s.to_string()),
        ..Default::default()
    }
}

/// Append inherited external/public function completions from base contracts.
fn append_inherited_public_functions(
    st: &SymbolTable,
    fi: &crate::symbol_table::FileIndex,
    base_names: &[String],
    items: &mut Vec<CompletionItem>,
    seen: &mut FxHashSet<String>,
) {
    let mut base_decls = Vec::new();
    let mut base_seen = FxHashSet::default();
    for base_name in base_names {
        st.collect_base_declarations(fi.file_id, base_name, &mut base_decls, &mut base_seen);
    }
    for d in &base_decls {
        if is_public_function(d) && seen.insert(d.name.clone()) {
            items.push(decl_to_function_completion(d));
        }
    }
}

/// `super.` — show members from parent contracts (including grandparents).
fn super_completions(
    st: &SymbolTable,
    file: &Path,
    scope: usize,
    source: &str,
    cursor_byte: usize,
) -> Vec<CompletionItem> {
    // Try scope-based approach first, fall back to text-based extraction.
    let base_names: Vec<String> = if let Some(contract) = find_enclosing_contract(st, file, scope) {
        contract
            .base_contracts()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        extract_base_contracts_from_text(&source[..cursor_byte]).unwrap_or_default()
    };

    collect_base_member_completions(st, file, &base_names)
}

/// Collect deduplicated completions for all members of the given base contracts.
fn collect_base_member_completions(
    st: &SymbolTable,
    file: &Path,
    base_names: &[String],
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    let mut seen = FxHashSet::default();
    for base_name in base_names {
        for m in &st.all_members_of(base_name, file) {
            if seen.insert(m.name.clone()) {
                items.push(member_to_completion(m));
            }
        }
    }
    items
}

/// Walk the scope chain to find the enclosing contract/interface declaration.
fn find_enclosing_contract<'a>(
    st: &'a SymbolTable,
    file: &Path,
    scope_id: usize,
) -> Option<&'a crate::symbol_table::Declaration> {
    let fi = st.get_file_index(file)?;
    let mut current = Some(scope_id);
    while let Some(sid) = current {
        let scope = fi.scopes.get(sid)?;
        if matches!(
            scope.kind,
            ScopeKind::Contract | ScopeKind::Interface | ScopeKind::Library
        ) {
            if let Some(decl) = scope.owner.and_then(|id| fi.declarations.get(&id)) {
                return Some(decl);
            }
        }
        current = scope.parent;
    }
    None
}

/// Fallback for `this.` when the symbol table lacks proper scopes due to
/// parse errors. Finds the enclosing contract from source text, then
/// collects external/public functions from the symbol table declarations
/// whose ranges fall within the contract body.
fn this_completions_fallback(
    fi: &crate::symbol_table::FileIndex,
    source: &str,
    cursor_byte: usize,
) -> Vec<CompletionItem> {
    let (body_start, body_end) = match find_enclosing_contract_braces(source, cursor_byte) {
        Some(range) => range,
        None => return vec![],
    };
    fi.declarations
        .values()
        .filter(|d| {
            is_public_function(d) && d.full_range.0 >= body_start && d.full_range.1 <= body_end
        })
        .map(|d| decl_to_function_completion(d))
        .collect()
}

/// Find the byte range of the enclosing contract/interface/library body braces
/// by scanning the source text around the cursor position. Skips inner scopes
/// (function bodies, etc.) to find the contract-level braces.
fn find_enclosing_contract_braces(source: &str, cursor_byte: usize) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut depth: i32 = 0;
    let mut pos = cursor_byte;

    // Scan backwards, looking for each unmatched `{` and checking if it
    // belongs to a contract/interface/library.
    while pos > 0 {
        pos -= 1;
        match bytes[pos] {
            b'}' => depth += 1,
            b'{' => {
                if depth == 0 {
                    // Found an unmatched `{`. Check if a contract keyword precedes it.
                    let prefix = source[..pos].trim_end();
                    if is_contract_header(prefix) {
                        // Found the contract opening brace. Now find the matching close.
                        let mut fwd_depth: i32 = 0;
                        let mut close_brace = source.len();
                        for i in cursor_byte..source.len() {
                            match bytes[i] {
                                b'{' => fwd_depth += 1,
                                b'}' => {
                                    if fwd_depth == 0 {
                                        close_brace = i;
                                        break;
                                    }
                                    fwd_depth -= 1;
                                }
                                _ => {}
                            }
                        }
                        return Some((pos, close_brace));
                    }
                    // Not a contract brace — this is a function/block scope.
                    // Keep scanning upward (treat as if we entered a scope).
                } else {
                    depth -= 1;
                }
            }
            _ => {}
        }
    }
    None
}

/// Check if the text (trimmed) before an opening brace looks like a
/// contract/interface/library header.
fn is_contract_header(prefix: &str) -> bool {
    // The prefix ends with something like "contract Foo" or "contract Foo is Base".
    // A function header would end with ")" or "returns (...)".
    // Simple heuristic: check if any contract keyword appears AND the last
    // non-whitespace char is NOT ')'.
    let trimmed = prefix.trim_end();
    if trimmed.ends_with(')') {
        return false;
    }
    // Check for contract/interface/library keyword in the line before the brace.
    let last_line = trimmed.rsplit('\n').next().unwrap_or(trimmed);
    ["contract ", "interface ", "library "]
        .iter()
        .any(|kw| last_line.contains(kw))
}

/// Extract base contract names from source text, looking for the pattern
/// `contract Name is Base1, Base2 {`.
fn extract_base_contracts_from_text(before_cursor: &str) -> Option<Vec<String>> {
    // Find the last "contract ... is ... {" or "interface ... is ... {" pattern.
    for keyword in &["contract", "interface"] {
        if let Some(kw_pos) = before_cursor.rfind(keyword) {
            let after_kw = &before_cursor[kw_pos + keyword.len()..];
            // Find " is " after the contract name.
            if let Some(is_pos) = after_kw.find(" is ") {
                let after_is = &after_kw[is_pos + 4..];
                // Find the opening brace.
                if let Some(brace_pos) = after_is.find('{') {
                    let bases_str = &after_is[..brace_pos].trim();
                    let names: Vec<String> = bases_str
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !names.is_empty() {
                        return Some(names);
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// type(X). completion
// ---------------------------------------------------------------------------

/// Extract `type(X)` expression before a dot, returning X (the inner type).
pub(crate) fn extract_type_call_before_dot(line: &str, col_byte: u32) -> Option<String> {
    let col = col_byte as usize;
    if col < 7 {
        // Minimum: "type(X)." = 8 chars, dot is at col so need at least 7 before it.
        return None;
    }
    let bytes = line.as_bytes();

    let mut pos = col;
    // Skip the dot.
    if pos > 0 && pos <= bytes.len() && bytes[pos - 1] == b'.' {
        pos -= 1;
    }
    // Expect ')'.
    if pos == 0 || bytes[pos - 1] != b')' {
        return None;
    }
    pos -= 1;

    // Scan back to find matching '('.
    let mut depth: u32 = 1;
    let paren_end = pos;
    while pos > 0 && depth > 0 {
        pos -= 1;
        match bytes[pos] {
            b')' => depth += 1,
            b'(' => depth -= 1,
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    let paren_start = pos;

    // Check that "type" precedes the '('.
    if paren_start < 4 {
        return None;
    }
    let prefix = &line[paren_start - 4..paren_start];
    if prefix != "type" {
        return None;
    }

    // Make sure "type" isn't part of a larger identifier.
    if paren_start > 4 {
        let prev = bytes[paren_start - 5];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return None;
        }
    }

    Some(line[paren_start + 1..paren_end].trim().to_string())
}

/// Return type-specific members for `type(X).` expressions.
fn type_members_for(type_name: &str, st: &SymbolTable, file: &Path) -> Vec<CompletionItem> {
    let is_int = type_name.starts_with("uint") || type_name.starts_with("int");
    if is_int {
        return make_type_items(&[("min", type_name), ("max", type_name)]);
    }

    // Look up the type declaration to determine its kind.
    if let Some(decl_id) = st.find_type_decl(file, type_name) {
        if let Some(decl) = st.get_declaration(&decl_id) {
            return match decl.kind() {
                DeclKind::Enum => make_type_items(&[("min", type_name), ("max", type_name)]),
                DeclKind::Interface => {
                    make_type_items(&[("name", "string"), ("interfaceId", "bytes4")])
                }
                _ => make_type_items(&[
                    ("name", "string"),
                    ("creationCode", "bytes memory"),
                    ("runtimeCode", "bytes memory"),
                    ("interfaceId", "bytes4"),
                ]),
            };
        }
    }

    // Fallback: contract-like type.
    make_type_items(&[
        ("name", "string"),
        ("creationCode", "bytes memory"),
        ("runtimeCode", "bytes memory"),
        ("interfaceId", "bytes4"),
    ])
}

fn make_type_items(pairs: &[(&str, &str)]) -> Vec<CompletionItem> {
    pairs
        .iter()
        .map(|(label, detail)| CompletionItem {
            label: label.to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some(detail.to_string()),
            ..Default::default()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Built-in type members
// ---------------------------------------------------------------------------

/// Returns completion items for built-in type members (arrays, address).
fn builtin_type_members(type_text: &str) -> Option<Vec<CompletionItem>> {
    let stripped = type_text.trim();

    // Map the type text to the builtin name used in BUILTIN_TYPES.
    let builtin_name = if stripped.contains('[') {
        "__builtin_array"
    } else if stripped.strip_suffix(" payable").unwrap_or(stripped) == "address" {
        "address"
    } else {
        return None;
    };

    for &(name, _, members) in BUILTIN_TYPES {
        if name == builtin_name {
            return Some(
                members
                    .iter()
                    .map(|&(label, detail, _, _)| {
                        let kind = if detail.starts_with("function") {
                            CompletionItemKind::METHOD
                        } else {
                            CompletionItemKind::PROPERTY
                        };
                        CompletionItem {
                            label: label.to_string(),
                            kind: Some(kind),
                            detail: Some(detail.to_string()),
                            ..Default::default()
                        }
                    })
                    .collect(),
            );
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Import completion
// ---------------------------------------------------------------------------

/// If the cursor is inside an import directive (but not inside the path string),
/// suggest exports from the resolved target file.
fn get_import_completions(
    st: &SymbolTable,
    file: &Path,
    byte_offset: usize,
) -> Option<Vec<CompletionItem>> {
    let fi = st.get_file_index(file)?;

    for imp in &fi.imports {
        // Check if cursor is within the import directive's range.
        if byte_offset < imp.range.0 || byte_offset > imp.range.1 {
            continue;
        }
        // Skip if cursor is inside the path string itself.
        if byte_offset >= imp.path_range.0 && byte_offset <= imp.path_range.1 {
            continue;
        }
        // Get the resolved file's top-level exports.
        let resolved_path = imp.resolved_path.as_ref()?;
        let target_fid = st.lookup_file_id(resolved_path)?;
        let target_fi = st.files.get(&target_fid)?;

        let mut items = Vec::new();
        // The file-level scope is always index 0.
        if let Some(scope) = target_fi.scopes.first() {
            for (name, decl_id) in &scope.declarations {
                if let Some(decl) = target_fi.declarations.get(decl_id) {
                    items.push(CompletionItem {
                        label: name.clone(),
                        kind: Some(decl_kind_to_completion_kind(decl.kind())),
                        detail: decl.type_text().map(|s| s.to_string()),
                        ..Default::default()
                    });
                }
            }
        }
        return Some(items);
    }
    None
}

// ---------------------------------------------------------------------------
// Override specifier completion
// ---------------------------------------------------------------------------

/// If the cursor is inside `override(...)`, suggest base contract names.
fn get_override_completions(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    byte_offset: usize,
) -> Option<Vec<CompletionItem>> {
    let before = &source[..byte_offset];

    // Find the last unmatched '(' before cursor and check for "override" prefix.
    let mut depth: i32 = 0;
    for (i, ch) in before.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    // Check if "override" precedes this paren.
                    let prefix = before[..i].trim_end();
                    if prefix.ends_with("override") {
                        let scope_id = st.scope_at(file, byte_offset)?;
                        let contract = find_enclosing_contract(st, file, scope_id)?;
                        return Some(
                            contract
                                .base_contracts()
                                .iter()
                                .map(|name| CompletionItem {
                                    label: name.clone(),
                                    kind: Some(CompletionItemKind::CLASS),
                                    detail: Some("base contract".to_string()),
                                    ..Default::default()
                                })
                                .collect(),
                        );
                    }
                    return None;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// General completion
// ---------------------------------------------------------------------------

fn get_general_completions(
    st: &SymbolTable,
    file: &Path,
    byte_offset: usize,
) -> Vec<CompletionItem> {
    let scope_id = st.scope_at(file, byte_offset).unwrap_or(0);
    let visible = st.visible_declarations(file, scope_id);

    let mut items: Vec<CompletionItem> = visible
        .iter()
        .filter(|decl| decl.id.byte_offset < SYNTHETIC_BASE)
        .map(|decl| CompletionItem {
            label: decl.name.clone(),
            kind: Some(decl_kind_to_completion_kind(decl.kind())),
            detail: decl.type_text().map(|s| s.to_string()),
            ..Default::default()
        })
        .collect();

    // Use cached static completions. (Fix #17)
    items.extend_from_slice(static_completions());
    items
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn member_to_completion(m: &MemberInfo) -> CompletionItem {
    CompletionItem {
        label: m.name.clone(),
        kind: Some(decl_kind_to_completion_kind(m.kind)),
        detail: if m.type_text.is_empty() {
            None
        } else {
            Some(m.type_text.clone())
        },
        ..Default::default()
    }
}

fn decl_kind_to_completion_kind(kind: DeclKind) -> CompletionItemKind {
    match kind {
        DeclKind::Function | DeclKind::Constructor | DeclKind::FallbackReceive => {
            CompletionItemKind::FUNCTION
        }
        DeclKind::Modifier => CompletionItemKind::METHOD,
        DeclKind::Event => CompletionItemKind::EVENT,
        DeclKind::Error => CompletionItemKind::EVENT,
        DeclKind::Contract | DeclKind::Interface | DeclKind::Library => CompletionItemKind::CLASS,
        DeclKind::Struct => CompletionItemKind::STRUCT,
        DeclKind::Enum => CompletionItemKind::ENUM,
        DeclKind::EnumValue => CompletionItemKind::ENUM_MEMBER,
        DeclKind::StateVariable
        | DeclKind::LocalVariable
        | DeclKind::Parameter
        | DeclKind::Constant => CompletionItemKind::VARIABLE,
        DeclKind::UserDefinedType => CompletionItemKind::CLASS,
        DeclKind::ImportAlias => CompletionItemKind::MODULE,
    }
}

/// Extract the identifier before `(args).` — handles type casts like `IERC20(token).`
/// and function calls like `getToken().`.
fn extract_call_before_dot(line: &str, col_byte: u32) -> Option<String> {
    let col = col_byte as usize;
    if col < 3 {
        // Minimum: `f().` = 4 chars, dot at col so need at least 3 before it.
        return None;
    }
    let bytes = line.as_bytes();

    let mut pos = col;
    // Skip the dot.
    if pos > 0 && pos <= bytes.len() && bytes[pos - 1] == b'.' {
        pos -= 1;
    }
    // Expect ')'.
    if pos == 0 || bytes[pos - 1] != b')' {
        return None;
    }
    pos -= 1;

    // Scan back to find matching '('.
    let mut depth: u32 = 1;
    while pos > 0 && depth > 0 {
        pos -= 1;
        match bytes[pos] {
            b')' => depth += 1,
            b'(' => depth -= 1,
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    // `pos` now points to the opening '('. Extract the identifier before it.
    let paren_start = pos;
    let end = paren_start;
    while pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_') {
        pos -= 1;
    }
    if pos == end {
        return None;
    }

    Some(String::from_utf8_lossy(&bytes[pos..end]).to_string())
}

/// Provide completions for the result of a call expression `SomeType(args).`
/// This handles type casts (IERC20(token).) and function calls (getToken().).
fn call_result_completions(
    st: &SymbolTable,
    file: &Path,
    name: &str,
    scope: usize,
) -> Vec<CompletionItem> {
    // First, check if `name` is a known type (contract, interface, struct, etc.)
    // and show its instance members (including inherited) + using-for methods.
    let members = st.all_members_of(name, file);
    let mut items: Vec<CompletionItem> = members.iter().map(member_to_completion).collect();
    append_using_for(st, file, name, &mut items);

    if !items.is_empty() {
        return items;
    }

    // Check if it's a function — use its return type for completions.
    let visible = st.visible_declarations(file, scope);
    for decl in &visible {
        if decl.name == name && decl.kind() == DeclKind::Function {
            let ret_params = decl.return_parameters();
            if ret_params.len() == 1 {
                let ret_type = &ret_params[0].0;
                // Try builtin type members.
                if let Some(mut bi) = builtin_type_members(ret_type) {
                    append_using_for(st, file, ret_type, &mut bi);
                    return bi;
                }
                // Try user-defined type members (including inherited).
                let base_type = ret_type
                    .replace(" memory", "")
                    .replace(" storage", "")
                    .replace(" calldata", "");
                let ret_members = st.all_members_of(&base_type, file);
                if !ret_members.is_empty() {
                    let mut ret_items: Vec<CompletionItem> =
                        ret_members.iter().map(member_to_completion).collect();
                    append_using_for(st, file, &base_type, &mut ret_items);
                    return ret_items;
                }
                // Only using-for.
                let mut ret_items = Vec::new();
                append_using_for(st, file, &base_type, &mut ret_items);
                if !ret_items.is_empty() {
                    return ret_items;
                }
            }
        }
    }

    vec![]
}

fn extract_identifier_before_dot(line: &str, col_byte: u32) -> Option<String> {
    let col = col_byte as usize;
    if col == 0 {
        return None;
    }
    let bytes = line.as_bytes();

    let mut pos = col;
    if pos > 0 && pos <= bytes.len() && bytes[pos - 1] == b'.' {
        pos -= 1;
    }

    let end = pos;
    while pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_') {
        pos -= 1;
    }

    if pos == end {
        return None;
    }

    Some(String::from_utf8_lossy(&bytes[pos..end]).to_string())
}

/// Magic type member definitions (msg, block, tx, abi, bytes, string).
fn magic_members(name: &str) -> Option<Vec<CompletionItem>> {
    // Check shared builtin globals first (msg, block, tx).
    for &(global_name, _, members) in BUILTIN_GLOBALS {
        if global_name == name {
            return Some(
                members
                    .iter()
                    .map(|&(label, detail, _, _)| CompletionItem {
                        label: label.to_string(),
                        kind: Some(CompletionItemKind::PROPERTY),
                        detail: Some(detail.to_string()),
                        ..Default::default()
                    })
                    .collect(),
            );
        }
    }

    // Completion-only globals not in the symbol table.
    let items: &[(&str, &str)] = match name {
        "abi" => &[
            ("decode(bytes memory, (...))", "..."),
            ("encode(...)", "bytes memory"),
            ("encodePacked(...)", "bytes memory"),
            ("encodeWithSelector(bytes4, ...)", "bytes memory"),
            ("encodeWithSignature(string memory, ...)", "bytes memory"),
            ("encodeCall(function, (...))", "bytes memory"),
        ],
        "bytes" => &[("concat(...)", "bytes memory")],
        "string" => &[("concat(...)", "string memory")],
        _ => return None,
    };

    Some(
        items
            .iter()
            .map(|&(label, detail)| CompletionItem {
                label: label.to_string(),
                kind: Some(CompletionItemKind::PROPERTY),
                detail: Some(detail.to_string()),
                ..Default::default()
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Cached static completions (Fix #17)
// ---------------------------------------------------------------------------

/// Returns a reference to the cached static completions (built once).
fn static_completions() -> &'static [CompletionItem] {
    static CACHE: OnceLock<Vec<CompletionItem>> = OnceLock::new();
    CACHE.get_or_init(build_static_completions)
}

fn build_static_completions() -> Vec<CompletionItem> {
    let mut items = Vec::new();

    for kw in SOLIDITY_KEYWORDS {
        items.push(CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        });
    }

    for (name, detail) in MAGIC_GLOBALS {
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::VARIABLE),
            detail: Some(detail.to_string()),
            ..Default::default()
        });
    }

    for (name, detail) in GLOBAL_FUNCTIONS {
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(detail.to_string()),
            ..Default::default()
        });
    }

    for (name, detail) in ETHER_UNITS {
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::UNIT),
            detail: Some(detail.to_string()),
            ..Default::default()
        });
    }

    for (name, detail) in TIME_UNITS {
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::UNIT),
            detail: Some(detail.to_string()),
            ..Default::default()
        });
    }

    items
}

const SOLIDITY_KEYWORDS: &[&str] = &[
    "abstract",
    "address",
    "assembly",
    "bool",
    "break",
    "bytes",
    "bytes1",
    "bytes4",
    "bytes32",
    "calldata",
    "constant",
    "constructor",
    "continue",
    "contract",
    "delete",
    "do",
    "else",
    "emit",
    "enum",
    "error",
    "event",
    "external",
    "fallback",
    "false",
    "for",
    "function",
    "if",
    "immutable",
    "import",
    "indexed",
    "int8",
    "int24",
    "int128",
    "int256",
    "interface",
    "internal",
    "library",
    "mapping",
    "memory",
    "modifier",
    "new",
    "override",
    "payable",
    "pragma",
    "private",
    "public",
    "pure",
    "receive",
    "return",
    "returns",
    "revert",
    "storage",
    "string",
    "struct",
    "true",
    "type",
    "uint8",
    "uint24",
    "uint128",
    "uint160",
    "uint256",
    "unchecked",
    "using",
    "view",
    "virtual",
    "while",
];

const ETHER_UNITS: &[(&str, &str)] = &[("wei", "1"), ("gwei", "1e9"), ("ether", "1e18")];

const TIME_UNITS: &[(&str, &str)] = &[
    ("seconds", "1"),
    ("minutes", "60 seconds"),
    ("hours", "3600 seconds"),
    ("days", "86400 seconds"),
    ("weeks", "604800 seconds"),
];

const MAGIC_GLOBALS: &[(&str, &str)] = &[
    ("msg", "msg"),
    ("block", "block"),
    ("tx", "tx"),
    ("abi", "abi"),
    ("this", "address"),
    ("super", "contract"),
    ("type", "type information"),
];

const GLOBAL_FUNCTIONS: &[(&str, &str)] = &[
    ("addmod(uint256, uint256, uint256)", "uint256"),
    ("mulmod(uint256, uint256, uint256)", "uint256"),
    ("keccak256(bytes memory)", "bytes32"),
    ("sha256(bytes memory)", "bytes32"),
    ("ripemd160(bytes memory)", "bytes20"),
    (
        "ecrecover(bytes32 hash, uint8 v, bytes32 r, bytes32 s)",
        "address",
    ),
    ("blockhash(uint256 blockNumber)", "bytes32"),
    ("blobhash(uint256 index)", "bytes32"),
    ("gasleft()", "uint256"),
    ("assert(bool condition)", ""),
    ("require(bool condition)", ""),
    ("require(bool condition, string memory message)", ""),
    ("revert()", ""),
    ("revert(string memory reason)", ""),
    ("selfdestruct(address payable recipient)", ""),
];
