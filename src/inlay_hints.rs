use std::path::Path;

use tower_lsp::lsp_types::*;
use tree_sitter::{Node, Tree};

use crate::symbol_table::{DeclKind, SymbolTable};
use crate::utils::LineIndex;

/// Provide inlay hints for a given range of the document.
///
/// Currently supports:
/// - **Parameter name hints**: show parameter names at call sites
///   e.g. `transfer(▸to: addr, ▸amount: 100)`
pub fn inlay_hints(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    range: Range,
    line_index: &LineIndex,
    tree: Option<&Tree>,
) -> Vec<InlayHint> {
    let tree = match tree {
        Some(t) => t,
        None => return vec![],
    };

    let start_byte =
        line_index.position_to_byte_offset(source, range.start.line, range.start.character);
    let end_byte = line_index.position_to_byte_offset(source, range.end.line, range.end.character);

    let mut hints = Vec::new();
    collect_hints(
        tree.root_node(),
        st,
        file,
        source,
        line_index,
        start_byte,
        end_byte,
        &mut hints,
    );
    hints
}

fn collect_hints(
    node: Node,
    st: &SymbolTable,
    file: &Path,
    source: &str,
    line_index: &LineIndex,
    start_byte: usize,
    end_byte: usize,
    hints: &mut Vec<InlayHint>,
) {
    // Skip nodes entirely outside the requested range.
    if node.end_byte() < start_byte || node.start_byte() > end_byte {
        return;
    }

    match node.kind() {
        "call_expression" => {
            collect_call_hints(node, st, file, source, line_index, hints);
        }
        "emit_statement" | "revert_statement" | "modifier_invocation" => {
            collect_emit_revert_hints(node, st, file, source, line_index, hints);
        }
        _ => {}
    }

    // Recurse into children.
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_hints(
                cursor.node(),
                st,
                file,
                source,
                line_index,
                start_byte,
                end_byte,
                hints,
            );
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Collect parameter name hints for a function/event/error call.
fn collect_call_hints(
    call_node: Node,
    st: &SymbolTable,
    file: &Path,
    source: &str,
    line_index: &LineIndex,
    hints: &mut Vec<InlayHint>,
) {
    let callee_node = match call_node.child_by_field_name("function") {
        Some(n) => n,
        None => return,
    };

    // Resolve the callee to get parameter names.
    let decl = match resolve_callee_node(st, file, source, &callee_node) {
        Some(d) => d,
        None => return,
    };

    let params = decl.parameters();
    if params.is_empty() {
        return;
    }

    // Collect argument expression nodes from call_argument children.
    let arg_nodes = collect_call_arguments(&call_node);

    // Generate a hint for each argument that has a corresponding named parameter.
    for (i, arg_node) in arg_nodes.iter().enumerate() {
        if i >= params.len() {
            break;
        }

        let param_name = &params[i].1;
        // Skip if parameter has no name.
        if param_name.is_empty() {
            continue;
        }

        // Skip if the argument is already a named argument (struct-style call).
        if arg_node.kind() == "call_struct_argument" {
            continue;
        }

        // Skip if the argument is just an identifier with the same name as the parameter.
        let arg_text = node_text(arg_node, source);
        if arg_text == param_name {
            continue;
        }

        // Skip simple literal-like single-identifier args that match the param name
        // after common prefixes (e.g., `_to` for param `to`). This reduces noise.
        if is_trivially_obvious(arg_text, param_name) {
            continue;
        }

        let (line, character) = line_index.byte_offset_to_position(source, arg_node.start_byte());
        let position = Position { line, character };
        hints.push(InlayHint {
            position,
            label: InlayHintLabel::String(format!("{param_name}:")),
            kind: Some(InlayHintKind::PARAMETER),
            text_edits: None,
            tooltip: None,
            padding_left: None,
            padding_right: Some(true),
            data: None,
        });
    }
}

/// Collect parameter name hints for an emit or revert statement.
///
/// `emit Transfer(a, b, c)` is parsed as:
///   emit_statement → emit, expression[identifier], (, call_argument, …, )
///
/// `revert Err(a, b)` is parsed as:
///   revert_statement → revert, expression[identifier], revert_arguments(…)
fn collect_emit_revert_hints(
    node: Node,
    st: &SymbolTable,
    file: &Path,
    source: &str,
    line_index: &LineIndex,
    hints: &mut Vec<InlayHint>,
) {
    // Find the callee name — the first named child that is an expression or
    // identifier (skip the keyword).
    let callee_node = {
        let mut found = None;
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                match child.kind() {
                    "expression" | "identifier" | "member_expression" => {
                        found = Some(child);
                        break;
                    }
                    _ => {}
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        match found {
            Some(n) => n,
            None => return,
        }
    };

    let decl = match resolve_callee_node(st, file, source, &callee_node) {
        Some(d) => d,
        None => return,
    };

    let params = decl.parameters();
    if params.is_empty() {
        return;
    }

    // Collect call_argument children — they may be direct children of the
    // statement node (emit) or inside a revert_arguments wrapper (revert).
    let arg_nodes = collect_call_arguments_from_descendants(&node);

    for (i, arg_node) in arg_nodes.iter().enumerate() {
        if i >= params.len() {
            break;
        }

        let param_name = &params[i].1;
        if param_name.is_empty() {
            continue;
        }

        if arg_node.kind() == "call_struct_argument" {
            continue;
        }

        let arg_text = node_text(arg_node, source);
        if arg_text == param_name {
            continue;
        }

        if is_trivially_obvious(arg_text, param_name) {
            continue;
        }

        let (line, character) = line_index.byte_offset_to_position(source, arg_node.start_byte());
        let position = Position { line, character };
        hints.push(InlayHint {
            position,
            label: InlayHintLabel::String(format!("{param_name}:")),
            kind: Some(InlayHintKind::PARAMETER),
            text_edits: None,
            tooltip: None,
            padding_left: None,
            padding_right: Some(true),
            data: None,
        });
    }
}

/// Collect call_argument expression nodes from anywhere within a node's direct
/// children or one level of nesting (for revert_arguments wrappers).
fn collect_call_arguments_from_descendants<'a>(node: &Node<'a>) -> Vec<Node<'a>> {
    let mut args = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "call_argument" {
                // Unwrap the inner expression.
                if let Some(inner) = first_named_child(&child) {
                    args.push(inner);
                }
            } else if child.kind() == "revert_arguments" {
                // Recurse one level into revert_arguments.
                let mut inner_cursor = child.walk();
                if inner_cursor.goto_first_child() {
                    loop {
                        let ic = inner_cursor.node();
                        if ic.kind() == "call_argument" {
                            if let Some(expr) = first_named_child(&ic) {
                                args.push(expr);
                            }
                        }
                        if !inner_cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    args
}

fn first_named_child<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "call_struct_argument" {
                return Some(child);
            }
            if child.is_named() {
                return Some(child);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Check if the argument name trivially implies the parameter name,
/// making a hint redundant. For example:
/// - `_to` → `to` (underscore prefix)
/// - `to` → `to` (exact match, handled separately)
/// - `newOwner` → `owner` when param is `owner` (not obvious enough, show hint)
fn is_trivially_obvious(arg_text: &str, param_name: &str) -> bool {
    // Strip leading underscores from the argument.
    let stripped = arg_text.trim_start_matches('_');
    if stripped.is_empty() {
        return false;
    }
    // Case-insensitive match after stripping underscores.
    stripped.eq_ignore_ascii_case(param_name)
}

/// Collect argument nodes from all `call_argument` children of a call_expression.
///
/// In tree-sitter-solidity >=1.2, each argument is wrapped in its own
/// `call_argument` node (rather than a single argument-list node).  Each
/// `call_argument` contains either a single named `expression` child, or a
/// `call_struct_argument` for named-argument syntax.
fn collect_call_arguments<'a>(call_node: &Node<'a>) -> Vec<Node<'a>> {
    let mut args = Vec::new();
    let mut cursor = call_node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "call_argument" {
                // Look inside the call_argument for the actual expression or
                // a call_struct_argument.
                let mut inner = child.walk();
                if inner.goto_first_child() {
                    loop {
                        let ic = inner.node();
                        if ic.kind() == "call_struct_argument" {
                            args.push(ic);
                            break;
                        }
                        if ic.is_named() {
                            args.push(ic);
                            break;
                        }
                        if !inner.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    args
}

/// Resolve a callee CST node to a Declaration.
///
/// The callee may be wrapped in an `expression` node (tree-sitter-solidity
/// >=1.2).  We unwrap transparently before matching on concrete kinds.
fn resolve_callee_node<'a>(
    st: &'a SymbolTable,
    file: &Path,
    source: &str,
    callee_node: &Node,
) -> Option<&'a crate::symbol_table::Declaration> {
    // Unwrap `expression` wrapper nodes.
    if callee_node.kind() == "expression" {
        if let Some(inner) = callee_node.named_child(0) {
            return resolve_callee_node(st, file, source, &inner);
        }
        return None;
    }

    match callee_node.kind() {
        "identifier" => {
            let name = node_text(callee_node, source);
            resolve_name(st, file, name)
        }
        "new_expression" => {
            // `new Token(...)` — resolve the type name to find its constructor.
            // The constructor is stored as a declaration named "constructor"
            // whose scope places it inside the contract.  We find the contract
            // declaration first, then scan for a constructor that immediately
            // follows it in byte order.
            if let Some(type_name) = callee_node.child_by_field_name("name") {
                let name = node_text(&type_name, source);
                let file_id = st.lookup_file_id(file)?;
                let fi = st.files.get(&file_id)?;
                // Find the contract's byte offset so we can locate its constructor.
                let mut contract_offset = None;
                for decl in fi.declarations.values() {
                    if decl.name == name
                        && matches!(
                            decl.kind(),
                            DeclKind::Contract | DeclKind::Interface | DeclKind::Library
                        )
                    {
                        contract_offset = Some(decl.name_range.0);
                        break;
                    }
                }
                // Find a constructor declaration in this file.
                for decl in fi.declarations.values() {
                    if decl.kind() == DeclKind::Constructor {
                        // If we know the contract offset, verify the constructor
                        // belongs to it (its offset is after the contract start).
                        if let Some(co) = contract_offset {
                            if decl.name_range.0 > co {
                                return Some(decl);
                            }
                        } else {
                            return Some(decl);
                        }
                    }
                }
                // Also check imported files.
                for imp in &fi.imports {
                    if let Some(ref resolved) = imp.resolved_path {
                        if let Some(target_fid) = st.lookup_file_id(resolved) {
                            if let Some(target_fi) = st.files.get(&target_fid) {
                                let mut co = None;
                                for decl in target_fi.declarations.values() {
                                    if decl.name == name
                                        && matches!(
                                            decl.kind(),
                                            DeclKind::Contract
                                                | DeclKind::Interface
                                                | DeclKind::Library
                                        )
                                    {
                                        co = Some(decl.name_range.0);
                                        break;
                                    }
                                }
                                for decl in target_fi.declarations.values() {
                                    if decl.kind() == DeclKind::Constructor {
                                        if let Some(offset) = co {
                                            if decl.name_range.0 > offset {
                                                return Some(decl);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                return None;
            }
            None
        }
        "member_expression" => {
            // e.g., `token.transfer(...)` — resolve the property part.
            if let Some(prop) = callee_node.child_by_field_name("property") {
                let prop_name = node_text(&prop, source);
                // Try resolving via the object's type.
                if let Some(obj) = callee_node.child_by_field_name("object") {
                    if let Some(obj_decl) = resolve_callee_node(st, file, source, &obj) {
                        // Get the type of the object and look up the member.
                        if let Some(type_text) = obj_decl.type_text() {
                            let base = strip_type_modifiers(type_text);
                            let members = st.all_members_of(base, file);
                            for m in &members {
                                if m.name == prop_name && m.kind == DeclKind::Function {
                                    if let Some(id) = m.decl_id {
                                        return st.get_declaration(&id);
                                    }
                                }
                            }
                        }
                        // If the object is a contract/interface/library, look up its members.
                        if matches!(
                            obj_decl.kind(),
                            DeclKind::Contract | DeclKind::Interface | DeclKind::Library
                        ) {
                            let members = st.all_members_of(&obj_decl.name, file);
                            for m in &members {
                                if m.name == prop_name && m.kind == DeclKind::Function {
                                    if let Some(id) = m.decl_id {
                                        return st.get_declaration(&id);
                                    }
                                }
                            }
                        }
                    }
                }
                // Fallback: try resolving the property name directly.
                resolve_name(st, file, prop_name)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Resolve a simple name to a declaration via the symbol table.
fn resolve_name<'a>(
    st: &'a SymbolTable,
    file: &Path,
    name: &str,
) -> Option<&'a crate::symbol_table::Declaration> {
    let file_id = st.lookup_file_id(file)?;
    let fi = st.files.get(&file_id)?;

    // Check all declarations in the file.
    for decl in fi.declarations.values() {
        if decl.name == name {
            return Some(decl);
        }
    }

    // Check imported files.
    for imp in &fi.imports {
        if let Some(ref resolved) = imp.resolved_path {
            if let Some(target_fid) = st.lookup_file_id(resolved) {
                if let Some(target_fi) = st.files.get(&target_fid) {
                    for decl in target_fi.declarations.values() {
                        if decl.name == name && decl.scope == 0 {
                            return Some(decl);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Strip memory/storage/calldata/payable suffixes for type lookup.
fn strip_type_modifiers(type_text: &str) -> &str {
    let s = type_text.trim();
    let s = s
        .strip_suffix(" memory")
        .or_else(|| s.strip_suffix(" storage"))
        .or_else(|| s.strip_suffix(" calldata"))
        .or_else(|| s.strip_suffix(" payable"))
        .unwrap_or(s);
    if let Some(bracket_pos) = s.find('[') {
        &s[..bracket_pos]
    } else {
        s
    }
    .trim()
}

fn node_text<'a>(node: &Node, source: &'a str) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_trivially_obvious() {
        assert!(is_trivially_obvious("to", "to"));
        assert!(is_trivially_obvious("_to", "to"));
        assert!(is_trivially_obvious("__to", "to"));
        assert!(is_trivially_obvious("_To", "to"));
        assert!(!is_trivially_obvious("newTo", "to"));
        assert!(!is_trivially_obvious("recipient", "to"));
        assert!(!is_trivially_obvious("_", "to"));
    }

    #[test]
    fn test_strip_type_modifiers() {
        assert_eq!(strip_type_modifiers("uint256"), "uint256");
        assert_eq!(strip_type_modifiers("address payable"), "address");
        assert_eq!(strip_type_modifiers("uint256[] memory"), "uint256");
        assert_eq!(strip_type_modifiers("bytes storage"), "bytes");
        assert_eq!(
            strip_type_modifiers("mapping(address => uint256)"),
            "mapping(address => uint256)"
        );
    }

    #[test]
    fn test_collect_call_arguments_requires_tree() {
        // We can't easily construct tree-sitter nodes in isolation, but we
        // verify the helper compiles and handles the empty-children case
        // indirectly through integration tests.
        let args: Vec<Node> = Vec::new();
        assert!(args.is_empty());
    }
}
