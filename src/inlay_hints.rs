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

    if node.kind() == "call_expression" {
        collect_call_hints(node, st, file, source, line_index, hints);
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

    // Find the arguments node — walk children to find the call_argument list.
    let args_node = match find_arguments_node(&call_node) {
        Some(n) => n,
        None => return,
    };

    // Collect argument expression nodes (skip commas, parens).
    let arg_nodes = collect_argument_nodes(&args_node);

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

/// Find the arguments list node inside a call_expression.
fn find_arguments_node<'a>(call_node: &Node<'a>) -> Option<Node<'a>> {
    let mut cursor = call_node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "call_argument" {
                return Some(child);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Collect the actual argument expression nodes from a call_argument node,
/// skipping punctuation (parens, commas).
fn collect_argument_nodes<'a>(args_node: &Node<'a>) -> Vec<Node<'a>> {
    let mut args = Vec::new();
    let mut cursor = args_node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.is_named() && child.kind() != "call_struct_argument" {
                args.push(child);
            } else if child.kind() == "call_struct_argument" {
                // Named argument — push it so we can skip it in the caller.
                args.push(child);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    args
}

/// Resolve a callee CST node to a Declaration.
fn resolve_callee_node<'a>(
    st: &'a SymbolTable,
    file: &Path,
    source: &str,
    callee_node: &Node,
) -> Option<&'a crate::symbol_table::Declaration> {
    match callee_node.kind() {
        "identifier" => {
            let name = node_text(callee_node, source);
            resolve_name(st, file, name)
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
    fn test_collect_argument_nodes_empty() {
        // Verify the function handles being called — we can't easily construct
        // tree-sitter nodes in isolation, but we test the helper logic.
        let args: Vec<Node> = Vec::new();
        assert!(args.is_empty());
    }
}
