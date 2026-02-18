use std::path::Path;
use std::sync::OnceLock;

use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, Position,
};

use crate::parser::TsParser;
use crate::symbol_table::{DeclKind, MemberInfo, ScopeKind, SymbolTable, SYNTHETIC_BASE};
use crate::utils::LineIndex;

/// Handle a completion request.
pub fn handle_completion(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    trigger_char: Option<&str>,
    line_index: &LineIndex,
) -> Option<CompletionResponse> {
    // Handle empty/whitespace-only files gracefully.
    if source.is_empty() {
        return Some(CompletionResponse::List(CompletionList {
            is_incomplete: false,
            items: static_completions().to_vec(),
        }));
    }

    let lines: Vec<&str> = source.lines().collect();
    let _line = lines.get(position.line as usize)?;

    let abs_byte = line_index.position_to_byte_offset(source, position.line, position.character);
    let line_start_byte: usize = source[..abs_byte].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col_byte = (abs_byte - line_start_byte) as u32;
    let line = &source[line_start_byte
        ..source[line_start_byte..]
            .find('\n')
            .map(|i| line_start_byte + i)
            .unwrap_or(source.len())];

    // Suppress completion inside comments and string literals.
    if is_in_comment_or_string(source, abs_byte) {
        return Some(CompletionResponse::List(CompletionList {
            is_incomplete: false,
            items: vec![],
        }));
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

/// Check if a byte offset falls inside a comment or string literal using
/// tree-sitter. Creating a parser and parsing is sub-millisecond for typical
/// Solidity files, so this is negligible overhead for a completion request.
fn is_in_comment_or_string(source: &str, byte_offset: usize) -> bool {
    if byte_offset == 0 {
        return false;
    }
    let mut parser = TsParser::new();
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return false,
    };
    // Check the position just before the cursor. Tree-sitter uses half-open
    // ranges [start, end), so checking at `byte_offset` exactly can miss the
    // end boundary of comment/string nodes.
    let check = byte_offset - 1;
    let node = match tree
        .root_node()
        .descendant_for_byte_range(check, check)
    {
        Some(n) => n,
        None => return false,
    };
    // Walk up to check if we're inside a comment or string node.
    let mut current = Some(node);
    while let Some(n) = current {
        match n.kind() {
            "comment" | "string" | "string_literal" | "hex_string_literal"
            | "unicode_string_literal" => return true,
            _ => {}
        }
        current = n.parent();
    }
    false
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
        let members = decl.members();
        if !members.is_empty() {
            let mut items: Vec<CompletionItem> =
                members.iter().map(member_to_completion).collect();
            // Also include using-for methods if the decl has a type.
            if let Some(ref tt) = decl.type_text {
                append_using_for(st, file, scope, tt, &mut items);
            }
            return items;
        }

        // Enum values as fallback (enum_values might be populated even if
        // members isn't, depending on the indexing path).
        if decl.kind == DeclKind::Enum {
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
        if let Some(ref type_text) = decl.type_text {
            // Built-in type members (arrays, address).
            if let Some(mut items) = builtin_type_members(type_text) {
                append_using_for(st, file, scope, type_text, &mut items);
                return items;
            }
            // User-defined type members.
            let members = st.members_of(type_text, file);
            if !members.is_empty() {
                let mut items: Vec<CompletionItem> =
                    members.iter().map(member_to_completion).collect();
                append_using_for(st, file, scope, type_text, &mut items);
                return items;
            }
            // Only using-for methods.
            let mut items = Vec::new();
            append_using_for(st, file, scope, type_text, &mut items);
            if !items.is_empty() {
                return items;
            }
        }
    }

    // Try members_of directly (e.g. "ContractName." or "EnumName.")
    let members = st.members_of(&identifier, file);
    if !members.is_empty() {
        return members.iter().map(member_to_completion).collect();
    }

    vec![]
}

/// Append using-for library methods that apply to `type_text` in the given scope.
fn append_using_for(
    st: &SymbolTable,
    file: &Path,
    scope: usize,
    type_text: &str,
    items: &mut Vec<CompletionItem>,
) {
    let using_members = st.using_for_members(type_text, file, scope);
    items.extend(using_members.iter().map(member_to_completion));
}

/// `this.` — show external/public functions of the enclosing contract.
fn this_completions(
    st: &SymbolTable,
    file: &Path,
    scope: usize,
    source: &str,
    cursor_byte: usize,
) -> Vec<CompletionItem> {
    if let Some(contract) = find_enclosing_contract(st, file, scope) {
        let fi = match st.get_file_index(file) {
            Some(fi) => fi,
            None => return vec![],
        };
        return contract
            .members()
            .iter()
            .filter(|m| {
                m.kind == DeclKind::Function
                    && m.decl_id
                        .and_then(|did| fi.declarations.get(&did))
                        .and_then(|d| d.visibility.as_deref())
                        .map_or(false, |v| v == "external" || v == "public")
            })
            .map(member_to_completion)
            .collect();
    }

    // Fallback for parse-error cases: find contract body range from text,
    // collect function declarations within it that are external/public.
    this_completions_fallback(st, file, source, cursor_byte)
}

/// `super.` — show members from parent contracts.
fn super_completions(
    st: &SymbolTable,
    file: &Path,
    scope: usize,
    source: &str,
    cursor_byte: usize,
) -> Vec<CompletionItem> {
    if let Some(contract) = find_enclosing_contract(st, file, scope) {
        let mut items = Vec::new();
        for base_name in contract.base_contracts() {
            for m in st.members_of(base_name, file) {
                items.push(member_to_completion(m));
            }
        }
        return items;
    }

    // Fallback: find base contract names from text, look up their members.
    super_completions_fallback(st, file, source, cursor_byte)
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
            // Find the contract/interface declaration that owns this scope.
            for decl in fi.declarations.values() {
                if matches!(
                    decl.kind,
                    DeclKind::Contract | DeclKind::Interface | DeclKind::Library
                ) && scope.range.0 >= decl.full_range.0
                    && scope.range.1 <= decl.full_range.1
                {
                    return Some(decl);
                }
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
    st: &SymbolTable,
    file: &Path,
    source: &str,
    cursor_byte: usize,
) -> Vec<CompletionItem> {
    let fi = match st.get_file_index(file) {
        Some(fi) => fi,
        None => return vec![],
    };
    let (body_start, body_end) = match find_enclosing_contract_braces(source, cursor_byte) {
        Some(range) => range,
        None => return vec![],
    };
    fi.declarations
        .values()
        .filter(|d| {
            d.kind == DeclKind::Function
                && d.full_range.0 >= body_start
                && d.full_range.1 <= body_end
                && d.visibility
                    .as_deref()
                    .map_or(false, |v| v == "external" || v == "public")
        })
        .map(|d| CompletionItem {
            label: d.name.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: d.type_text.clone(),
            ..Default::default()
        })
        .collect()
}

/// Fallback for `super.` when parse errors prevent scope-based lookup.
/// Extracts base contract names from the source text and looks up their
/// members in the symbol table.
fn super_completions_fallback(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    cursor_byte: usize,
) -> Vec<CompletionItem> {
    let before = &source[..cursor_byte];
    // Find the innermost "contract Name is Base1, Base2" before cursor.
    let base_names = match extract_base_contracts_from_text(before) {
        Some(names) => names,
        None => return vec![],
    };
    let mut items = Vec::new();
    for base_name in &base_names {
        for m in st.members_of(base_name, file) {
            items.push(member_to_completion(m));
        }
    }
    items
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
fn extract_type_call_before_dot(line: &str, col_byte: u32) -> Option<String> {
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
            return match decl.kind {
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

    // Array types: anything containing `[`.
    if stripped.contains('[') {
        return Some(vec![
            CompletionItem {
                label: "length".to_string(),
                kind: Some(CompletionItemKind::PROPERTY),
                detail: Some("uint256".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "push".to_string(),
                kind: Some(CompletionItemKind::METHOD),
                detail: Some("function".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "pop".to_string(),
                kind: Some(CompletionItemKind::METHOD),
                detail: Some("function".to_string()),
                ..Default::default()
            },
        ]);
    }

    // Address types.
    let base = stripped
        .strip_suffix(" payable")
        .unwrap_or(stripped);
    if base == "address" {
        return Some(vec![
            make_property("balance", "uint256"),
            make_property("code", "bytes memory"),
            make_property("codehash", "bytes32"),
            make_method("transfer", "function(uint256)"),
            make_method("send", "function(uint256) returns (bool)"),
            make_method("call", "function(bytes memory) returns (bool, bytes memory)"),
            make_method(
                "delegatecall",
                "function(bytes memory) returns (bool, bytes memory)",
            ),
            make_method(
                "staticcall",
                "function(bytes memory) returns (bool, bytes memory)",
            ),
        ]);
    }

    None
}

fn make_property(label: &str, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::PROPERTY),
        detail: Some(detail.to_string()),
        ..Default::default()
    }
}

fn make_method(label: &str, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::METHOD),
        detail: Some(detail.to_string()),
        ..Default::default()
    }
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
                        kind: Some(decl_kind_to_completion_kind(decl.kind)),
                        detail: decl.type_text.clone(),
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
            kind: Some(decl_kind_to_completion_kind(decl.kind)),
            detail: decl.type_text.clone(),
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

/// Magic type member definitions (msg, block, tx, abi).
fn magic_members(name: &str) -> Option<Vec<CompletionItem>> {
    let items = match name {
        "msg" => vec![
            ("data", "bytes calldata"),
            ("sender", "address"),
            ("sig", "bytes4"),
            ("value", "uint256"),
        ],
        "block" => vec![
            ("basefee", "uint256"),
            ("blobbasefee", "uint256"),
            ("chainid", "uint256"),
            ("coinbase", "address payable"),
            ("difficulty", "uint256"),
            ("gaslimit", "uint256"),
            ("number", "uint256"),
            ("prevrandao", "uint256"),
            ("timestamp", "uint256"),
        ],
        "tx" => vec![("gasprice", "uint256"), ("origin", "address")],
        "abi" => vec![
            ("decode(bytes memory, (...))", "..."),
            ("encode(...)", "bytes memory"),
            ("encodePacked(...)", "bytes memory"),
            ("encodeWithSelector(bytes4, ...)", "bytes memory"),
            ("encodeWithSignature(string memory, ...)", "bytes memory"),
            ("encodeCall(function, (...))", "bytes memory"),
        ],
        "bytes" => vec![("concat(...)", "bytes memory")],
        "string" => vec![("concat(...)", "string memory")],
        _ => return None,
    };

    Some(
        items
            .into_iter()
            .map(|(label, detail)| CompletionItem {
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
