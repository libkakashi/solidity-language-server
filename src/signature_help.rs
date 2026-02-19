use std::path::Path;

use tower_lsp::lsp_types::*;
use tree_sitter::{Node, Tree};

use crate::hover::format_natspec;
use crate::symbol_table::{DeclKind, Declaration, SymbolTable};
use crate::utils::LineIndex;

/// Provide signature help at the given cursor position.
///
/// Triggers on `(` and `,` — determines which function/event/error/modifier
/// is being called, which parameter the cursor is on, and returns the full
/// signature with active-parameter highlighting.
pub fn signature_help(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    line_index: &LineIndex,
    tree: Option<&Tree>,
) -> Option<SignatureHelp> {
    let tree = tree?;
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);

    // Walk up from the cursor to find an enclosing call site.
    let call_site = find_call_site(tree.root_node(), byte_offset)?;

    // Count the active parameter (number of commas before cursor inside the arg list).
    let active_param = count_active_parameter(&call_site, byte_offset, source);

    // Resolve the callee to a declaration.
    let decl = resolve_callee(st, file, source, &call_site)?;
    let params = decl.parameters();
    if params.is_empty() && st.find_overloads(file, call_site.callee.start_byte()).len() <= 1 {
        return None;
    }

    // Collect all overloads for this function name.
    let overloads = st.find_overloads(file, call_site.callee.start_byte());

    let mut signatures: Vec<SignatureInformation> = Vec::new();
    let mut active_signature: u32 = 0;

    if overloads.len() > 1 {
        // Multiple overloads: build a signature for each, pick best match.
        for (i, overload) in overloads.iter().enumerate() {
            let sig = build_signature_info(overload, active_param);
            // Best match: the overload whose parameter count best fits.
            let param_count = overload.parameters().len() as u32;
            if param_count > active_param
                && (signatures.is_empty()
                    || param_count < overloads[active_signature as usize].parameters().len() as u32)
            {
                active_signature = i as u32;
            }
            signatures.push(sig);
        }
        // If no overload fits (active_param >= all param counts), pick the one
        // with the most parameters.
        if active_param > 0
            && overloads
                .get(active_signature as usize)
                .map_or(true, |d| (d.parameters().len() as u32) <= active_param)
        {
            if let Some((i, _)) = overloads
                .iter()
                .enumerate()
                .max_by_key(|(_, d)| d.parameters().len())
            {
                active_signature = i as u32;
            }
        }
    } else {
        // Single function (no overloads or only one match).
        if params.is_empty() {
            return None;
        }
        signatures.push(build_signature_info(decl, active_param));
    }

    Some(SignatureHelp {
        signatures,
        active_signature: Some(active_signature),
        active_parameter: Some(active_param),
    })
}

// ---------------------------------------------------------------------------
// Call-site detection
// ---------------------------------------------------------------------------

/// Information about a call site found at the cursor position.
struct CallSite<'a> {
    /// The node representing the callee (function name, member expression, etc.).
    callee: Node<'a>,
    /// The kind of call site (determines how to interpret the callee).
    _kind: CallSiteKind,
    /// The full call node (call_expression, emit_statement, etc.) for param counting.
    call_node: Node<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallSiteKind {
    /// Regular function/modifier call: `foo(...)`, `x.bar(...)`
    FunctionCall,
    /// Emit statement: `emit Transfer(...)`
    Emit,
    /// Revert statement: `revert CustomError(...)`
    Revert,
}

fn find_call_site<'a>(root: Node<'a>, byte_offset: usize) -> Option<CallSite<'a>> {
    // Find the deepest node at the cursor.
    let mut node = root.descendant_for_byte_range(byte_offset, byte_offset)?;

    // Walk up to find the enclosing call context.
    loop {
        match node.kind() {
            "call_expression" => {
                // Check that cursor is inside the argument list (after the opening paren).
                if is_inside_arg_list(node, byte_offset) {
                    if let Some(callee) = node.child_by_field_name("function") {
                        return Some(CallSite {
                            callee,
                            _kind: CallSiteKind::FunctionCall,
                            call_node: node,
                        });
                    }
                }
            }
            "emit_statement" => {
                if is_inside_arg_list(node, byte_offset) {
                    if let Some(callee) = find_emit_callee(node) {
                        return Some(CallSite {
                            callee,
                            _kind: CallSiteKind::Emit,
                            call_node: node,
                        });
                    }
                }
            }
            "revert_statement" => {
                if is_inside_arg_list(node, byte_offset) {
                    if let Some(callee) = find_revert_callee(node) {
                        return Some(CallSite {
                            callee,
                            _kind: CallSiteKind::Revert,
                            call_node: node,
                        });
                    }
                }
            }
            _ => {}
        }
        node = node.parent()?;
    }
}

/// Check if the byte_offset is inside the parenthesized argument list
/// (i.e., after the `(` and before the `)`).
fn is_inside_arg_list(node: Node, byte_offset: usize) -> bool {
    // First try direct children (works for call_expression and emit_statement).
    if is_inside_arg_list_direct(node, byte_offset) {
        return true;
    }
    // For revert_statement, parentheses are inside a `revert_arguments` child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "revert_arguments" {
            if is_inside_arg_list_direct(child, byte_offset) {
                return true;
            }
        }
    }
    false
}

fn is_inside_arg_list_direct(node: Node, byte_offset: usize) -> bool {
    let mut cursor = node.walk();
    let mut found_open_paren = false;
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            let kind = child.kind();
            if kind == "(" && child.end_byte() <= byte_offset {
                found_open_paren = true;
            }
            if kind == ")" && child.start_byte() >= byte_offset {
                return found_open_paren;
            }
        }
    }
    // Cursor might be past the last token but before `)` (e.g., no closing paren yet).
    found_open_paren
}

/// Find the event name in an `emit_statement`.
fn find_emit_callee(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            match child.kind() {
                "identifier" | "member_expression" => return Some(child),
                // tree-sitter-solidity >=1.2 wraps the callee in an `expression` node.
                "expression" => {
                    if let Some(inner) = child.named_child(0) {
                        return Some(inner);
                    }
                }
                // In some grammars, emit uses a call_expression child.
                "call_expression" => {
                    return child.child_by_field_name("function");
                }
                _ => {}
            }
        }
    }
    None
}

/// Find the error name in a `revert_statement`.
fn find_revert_callee(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            match child.kind() {
                "identifier" | "member_expression" => return Some(child),
                // tree-sitter-solidity >=1.2 wraps the callee in an `expression` node.
                "expression" => {
                    if let Some(inner) = child.named_child(0) {
                        return Some(inner);
                    }
                }
                "call_expression" => {
                    return child.child_by_field_name("function");
                }
                _ => {}
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Active parameter counting
// ---------------------------------------------------------------------------

/// Count the number of top-level commas before the cursor in the argument list.
fn count_active_parameter(site: &CallSite, byte_offset: usize, _source: &str) -> u32 {
    // For revert_statement, the commas and parens are inside `revert_arguments`.
    // Find the right node to iterate.
    let arg_node = find_arg_container(site.call_node);
    count_commas_in_node(arg_node, byte_offset)
}

/// Find the node whose direct children contain the `(`, `,`, `)` tokens.
///
/// For `call_expression` and `emit_statement`, the tokens are direct children.
/// For `revert_statement`, they are inside a `revert_arguments` child.
fn find_arg_container(node: Node) -> Node {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "revert_arguments" {
            return child;
        }
    }
    node
}

fn count_commas_in_node(node: Node, byte_offset: usize) -> u32 {
    let mut count: u32 = 0;
    let mut depth: i32 = 0;
    let mut past_open_paren = false;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.start_byte() >= byte_offset {
            break;
        }
        if !child.is_named() {
            match child.kind() {
                "(" => {
                    if depth == 0 {
                        past_open_paren = true;
                    }
                    depth += 1;
                }
                ")" => {
                    depth -= 1;
                }
                "," if depth == 1 && past_open_paren => {
                    count += 1;
                }
                _ => {}
            }
        }
    }

    count
}

// ---------------------------------------------------------------------------
// Callee resolution
// ---------------------------------------------------------------------------

fn resolve_callee<'a>(
    st: &'a SymbolTable,
    file: &Path,
    source: &str,
    site: &CallSite,
) -> Option<&'a Declaration> {
    // Resolve the callee node via the symbol table.
    let callee_text = &source[site.callee.start_byte()..site.callee.end_byte()];

    match site.callee.kind() {
        "identifier" => {
            // Simple function call: `foo(...)`.
            // Resolve the identifier through the symbol table.
            let decl = st.resolve_at(file, site.callee.start_byte())?;
            // Follow import aliases to their target declaration.
            if decl.kind() == DeclKind::ImportAlias {
                if let Some(target) = decl.import_target() {
                    if let Some(target_decl_id) = target.decl {
                        return st.get_declaration(&target_decl_id);
                    }
                }
            }
            Some(decl)
        }
        "member_expression" => {
            // Member call: `x.bar(...)` — resolve the property.
            if let Some(prop) = site.callee.child_by_field_name("property") {
                st.resolve_at(file, prop.start_byte())
            } else {
                None
            }
        }
        _ => {
            // Fallback: try resolving the start of the callee text.
            // For qualified event names like `IFoo.Transfer`, the callee
            // might be a `user_defined_type`.
            let _ = callee_text; // suppress unused warning
            st.resolve_at(file, site.callee.start_byte())
        }
    }
}

// ---------------------------------------------------------------------------
// Signature label building
// ---------------------------------------------------------------------------

/// Build a complete `SignatureInformation` for a single declaration.
fn build_signature_info(decl: &Declaration, active_param: u32) -> SignatureInformation {
    let (label, param_labels) = build_label(decl);

    let parameters: Vec<ParameterInformation> = param_labels
        .into_iter()
        .map(|(start, end)| {
            let enc_start = encoding_offset(&label, start);
            let enc_end = encoding_offset(&label, end);
            ParameterInformation {
                label: ParameterLabel::LabelOffsets([enc_start, enc_end]),
                documentation: None,
            }
        })
        .collect();

    let parameters = attach_param_docs(parameters, decl);

    let documentation = decl.natspec().map(|ns| {
        Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format_natspec(ns),
        })
    });

    SignatureInformation {
        label,
        documentation,
        parameters: Some(parameters),
        active_parameter: Some(active_param),
    }
}

/// Build the full signature label string and byte offset ranges for each parameter.
/// Returns (label, Vec<(start_offset, end_offset)>) where offsets are in the label string.
fn build_label(decl: &Declaration) -> (String, Vec<(usize, usize)>) {
    let kind_prefix = match decl.kind() {
        DeclKind::Function => "function",
        DeclKind::Constructor => "constructor",
        DeclKind::Modifier => "modifier",
        DeclKind::Event => "event",
        DeclKind::Error => "error",
        DeclKind::FallbackReceive if decl.name == "fallback" => "fallback",
        DeclKind::FallbackReceive => "receive",
        _ => "function",
    };

    let name_part = match decl.kind() {
        DeclKind::Constructor | DeclKind::FallbackReceive => String::new(),
        _ => format!(" {}", decl.name),
    };

    let mut label = format!("{kind_prefix}{name_part}(");
    let mut param_labels = Vec::new();
    let params = decl.parameters();

    for (i, (ptype, pname)) in params.iter().enumerate() {
        if i > 0 {
            label.push_str(", ");
        }
        let start = label.len();
        if pname.is_empty() {
            label.push_str(ptype);
        } else {
            label.push_str(&format!("{ptype} {pname}"));
        }
        let end = label.len();
        param_labels.push((start, end));
    }

    label.push(')');

    // Add return types for functions.
    let ret_params = decl.return_parameters();
    if !ret_params.is_empty() {
        let returns: Vec<String> = ret_params
            .iter()
            .map(|(ty, name)| {
                if name.is_empty() {
                    ty.clone()
                } else {
                    format!("{ty} {name}")
                }
            })
            .collect();
        label.push_str(&format!(" returns ({})", returns.join(", ")));
    }

    (label, param_labels)
}

/// Convert a byte offset within a label string to the negotiated encoding offset.
fn encoding_offset(label: &str, byte_offset: usize) -> u32 {
    match crate::utils::encoding() {
        crate::utils::PositionEncoding::Utf8 => byte_offset as u32,
        crate::utils::PositionEncoding::Utf16 => {
            let segment = &label[..byte_offset];
            segment.chars().map(|c| c.len_utf16() as u32).sum()
        }
    }
}

/// Attach NatSpec @param documentation to each parameter.
fn attach_param_docs(
    mut parameters: Vec<ParameterInformation>,
    decl: &Declaration,
) -> Vec<ParameterInformation> {
    let natspec = match decl.natspec() {
        Some(ns) => ns,
        None => return parameters,
    };

    let params = decl.parameters();
    for (i, (_, pname)) in params.iter().enumerate() {
        if pname.is_empty() || i >= parameters.len() {
            continue;
        }
        // Search NatSpec for `@param <pname> <description>`.
        for line in natspec.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("@param ") {
                let rest = rest.trim_start();
                if let Some(desc) = rest.strip_prefix(pname.as_str()) {
                    // The param name must be followed by whitespace or end-of-string.
                    if desc.is_empty() || desc.starts_with(' ') {
                        let doc_text = desc.trim();
                        if !doc_text.is_empty() {
                            parameters[i].documentation =
                                Some(Documentation::String(doc_text.to_string()));
                        }
                        break;
                    }
                }
            }
        }
    }

    parameters
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol_table::*;

    fn make_test_decl(name: &str, params: Vec<(String, String)>) -> Declaration {
        Declaration {
            id: DeclId {
                file: 0,
                byte_offset: 0,
            },
            name: name.to_string(),
            full_range: (0, 0),
            name_range: (0, 0),
            scope: 0,
            detail: DeclDetail::Function(Box::new(CallableDetail {
                visibility: Some("public".to_string()),
                state_mutability: None,
                parameters: params,
                return_parameters: vec![],
            })),
            natspec: None,
        }
    }

    #[test]
    fn test_build_label_simple() {
        let decl = make_test_decl(
            "transfer",
            vec![
                ("address".to_string(), "to".to_string()),
                ("uint256".to_string(), "amount".to_string()),
            ],
        );
        let (label, offsets) = build_label(&decl);
        assert_eq!(label, "function transfer(address to, uint256 amount)");
        assert_eq!(offsets.len(), 2);
        assert_eq!(&label[offsets[0].0..offsets[0].1], "address to");
        assert_eq!(&label[offsets[1].0..offsets[1].1], "uint256 amount");
    }

    #[test]
    fn test_build_label_no_param_names() {
        let decl = make_test_decl(
            "foo",
            vec![
                ("uint256".to_string(), String::new()),
                ("bool".to_string(), String::new()),
            ],
        );
        let (label, offsets) = build_label(&decl);
        assert_eq!(label, "function foo(uint256, bool)");
        assert_eq!(&label[offsets[0].0..offsets[0].1], "uint256");
        assert_eq!(&label[offsets[1].0..offsets[1].1], "bool");
    }

    #[test]
    fn test_build_label_event() {
        let decl = Declaration {
            id: DeclId {
                file: 0,
                byte_offset: 0,
            },
            name: "Transfer".to_string(),
            full_range: (0, 0),
            name_range: (0, 0),
            scope: 0,
            detail: DeclDetail::Event(Box::new(EventDetail {
                parameters: vec![
                    ("address".to_string(), "from".to_string()),
                    ("address".to_string(), "to".to_string()),
                    ("uint256".to_string(), "amount".to_string()),
                ],
            })),
            natspec: None,
        };
        let (label, offsets) = build_label(&decl);
        assert_eq!(
            label,
            "event Transfer(address from, address to, uint256 amount)"
        );
        assert_eq!(offsets.len(), 3);
    }

    #[test]
    fn test_build_label_constructor() {
        let decl = Declaration {
            id: DeclId {
                file: 0,
                byte_offset: 0,
            },
            name: "constructor".to_string(),
            full_range: (0, 0),
            name_range: (0, 0),
            scope: 0,
            detail: DeclDetail::Constructor(Box::new(CallableDetail {
                visibility: None,
                state_mutability: None,
                parameters: vec![("uint256".to_string(), "x".to_string())],
                return_parameters: vec![],
            })),
            natspec: None,
        };
        let (label, _) = build_label(&decl);
        assert_eq!(label, "constructor(uint256 x)");
    }

    #[test]
    fn test_build_label_with_returns() {
        let decl = Declaration {
            id: DeclId {
                file: 0,
                byte_offset: 0,
            },
            name: "balanceOf".to_string(),
            full_range: (0, 0),
            name_range: (0, 0),
            scope: 0,
            detail: DeclDetail::Function(Box::new(CallableDetail {
                visibility: Some("external".to_string()),
                state_mutability: Some("view".to_string()),
                parameters: vec![("address".to_string(), "account".to_string())],
                return_parameters: vec![("uint256".to_string(), String::new())],
            })),
            natspec: None,
        };
        let (label, offsets) = build_label(&decl);
        assert_eq!(
            label,
            "function balanceOf(address account) returns (uint256)"
        );
        assert_eq!(offsets.len(), 1);
        assert_eq!(&label[offsets[0].0..offsets[0].1], "address account");
    }

    #[test]
    fn test_attach_param_docs() {
        let decl = Declaration {
            id: DeclId {
                file: 0,
                byte_offset: 0,
            },
            name: "transfer".to_string(),
            full_range: (0, 0),
            name_range: (0, 0),
            scope: 0,
            detail: DeclDetail::Function(Box::new(CallableDetail {
                visibility: None,
                state_mutability: None,
                parameters: vec![
                    ("address".to_string(), "to".to_string()),
                    ("uint256".to_string(), "amount".to_string()),
                ],
                return_parameters: vec![],
            })),
            natspec: Some("@notice Transfer tokens\n@param to The recipient\n@param amount The amount to send".to_string()),
        };

        let params = vec![
            ParameterInformation {
                label: ParameterLabel::LabelOffsets([0, 10]),
                documentation: None,
            },
            ParameterInformation {
                label: ParameterLabel::LabelOffsets([12, 26]),
                documentation: None,
            },
        ];

        let result = attach_param_docs(params, &decl);
        assert!(result[0].documentation.is_some());
        assert!(result[1].documentation.is_some());
        match &result[0].documentation {
            Some(Documentation::String(s)) => assert_eq!(s, "The recipient"),
            _ => panic!("expected string doc"),
        }
        match &result[1].documentation {
            Some(Documentation::String(s)) => assert_eq!(s, "The amount to send"),
            _ => panic!("expected string doc"),
        }
    }
}
