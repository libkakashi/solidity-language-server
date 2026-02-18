use std::path::Path;

use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};

use crate::completion::extract_type_call_before_dot;
use crate::symbol_table::{DeclKind, SymbolTable};
use crate::utils::LineIndex;

/// Produce hover information for the symbol at the given position.
pub fn hover_info(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    position: Position,
    line_index: &LineIndex,
) -> Option<Hover> {
    let byte_offset = line_index.position_to_byte_offset(source, position.line, position.character);

    if let Some(decl) = st.resolve_at(file, byte_offset) {
        let mut parts: Vec<String> = Vec::new();

        let sig = build_signature(decl);
        parts.push(format!("```solidity\n{sig}\n```"));

        if let Some(natspec) = decl.natspec() {
            let formatted = format_natspec(natspec);
            if !formatted.is_empty() {
                parts.push(format!("---\n{formatted}"));
            }
        }

        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: parts.join("\n\n"),
            }),
            range: None,
        });
    }

    // Fallback for magic expressions: type(X).member, string.concat, bytes.concat
    magic_hover(st, file, source, byte_offset, line_index, position)
}

/// Fallback hover for expressions not in the symbol table.
fn magic_hover(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    byte_offset: usize,
    line_index: &LineIndex,
    position: Position,
) -> Option<Hover> {
    let line_start = line_index.line_start(position.line);
    let line_text = source[line_start..]
        .lines()
        .next()
        .unwrap_or("");
    let col = byte_offset - line_start;

    // Extract the token (identifier) at cursor position.
    let bytes = line_text.as_bytes();
    let mut tok_start = col;
    while tok_start > 0
        && (bytes[tok_start - 1].is_ascii_alphanumeric() || bytes[tok_start - 1] == b'_')
    {
        tok_start -= 1;
    }
    let mut tok_end = col;
    while tok_end < bytes.len()
        && (bytes[tok_end].is_ascii_alphanumeric() || bytes[tok_end] == b'_')
    {
        tok_end += 1;
    }
    if tok_start == tok_end {
        return None;
    }
    let member = &line_text[tok_start..tok_end];

    // Require a dot before the token.
    if tok_start == 0 || bytes[tok_start - 1] != b'.' {
        return None;
    }
    let dot_pos = tok_start - 1;

    // Try type(X).member
    // extract_type_call_before_dot expects col_byte pointing at the character
    // right after the dot. The member token starts at tok_start (after the dot),
    // so pass tok_start as col_byte (dot is at tok_start-1, the function skips
    // the dot internally when it is at pos-1). Actually, the function expects
    // the column of the cursor AFTER the dot, meaning it looks for '.' at pos-1.
    // tok_start is right after the dot, so that works.
    if let Some(type_name) = extract_type_call_before_dot(line_text, tok_start as u32) {
        if let Some((sig, doc)) = type_member_signature(&type_name, member, st, file) {
            return Some(make_hover(&sig, doc.as_deref()));
        }
    }

    // Try string.member / bytes.member
    if let Some((sig, doc)) = static_type_member_signature(line_text, dot_pos, member) {
        return Some(make_hover(&sig, Some(&doc)));
    }

    None
}

fn make_hover(sig: &str, doc: Option<&str>) -> Hover {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("```solidity\n{sig}\n```"));
    if let Some(d) = doc {
        if !d.is_empty() {
            parts.push(format!("---\n{d}"));
        }
    }
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: parts.join("\n\n"),
        }),
        range: None,
    }
}

/// Signature for `type(X).member` expressions.
fn type_member_signature(
    type_name: &str,
    member: &str,
    st: &SymbolTable,
    file: &Path,
) -> Option<(String, Option<String>)> {
    let is_int = type_name.starts_with("uint") || type_name.starts_with("int");
    if is_int {
        return match member {
            "min" => Some((format!("{type_name} type({type_name}).min"), Some("The smallest value representable by type T.".to_string()))),
            "max" => Some((format!("{type_name} type({type_name}).max"), Some("The largest value representable by type T.".to_string()))),
            _ => None,
        };
    }

    // Look up user-defined type to determine kind.
    if let Some(decl_id) = st.find_type_decl(file, type_name) {
        if let Some(decl) = st.get_declaration(&decl_id) {
            return match decl.kind() {
                DeclKind::Enum => match member {
                    "min" => Some((format!("{type_name} type({type_name}).min"), Some("The smallest value representable by type T.".to_string()))),
                    "max" => Some((format!("{type_name} type({type_name}).max"), Some("The largest value representable by type T.".to_string()))),
                    _ => None,
                },
                DeclKind::Interface => match member {
                    "name" => Some((format!("string type({type_name}).name"), Some("The name of the contract.".to_string()))),
                    "interfaceId" => Some((format!("bytes4 type({type_name}).interfaceId"), Some("A `bytes4` value containing the EIP-165 interface identifier of the given interface.".to_string()))),
                    _ => None,
                },
                _ => type_member_signature_contract(type_name, member),
            };
        }
    }

    // Fallback for unknown types: assume contract-like members.
    type_member_signature_contract(type_name, member)
}

fn type_member_signature_contract(type_name: &str, member: &str) -> Option<(String, Option<String>)> {
    match member {
        "name" => Some((format!("string type({type_name}).name"), Some("The name of the contract.".to_string()))),
        "creationCode" => Some((format!("bytes memory type({type_name}).creationCode"), Some("Memory byte array that contains the creation bytecode of the contract.".to_string()))),
        "runtimeCode" => Some((format!("bytes memory type({type_name}).runtimeCode"), Some("Memory byte array that contains the runtime bytecode of the contract.".to_string()))),
        "interfaceId" => Some((format!("bytes4 type({type_name}).interfaceId"), Some("A `bytes4` value containing the EIP-165 interface identifier of the given interface.".to_string()))),
        "min" => Some((format!("{type_name} type({type_name}).min"), Some("The smallest value representable by type T.".to_string()))),
        "max" => Some((format!("{type_name} type({type_name}).max"), Some("The largest value representable by type T.".to_string()))),
        _ => None,
    }
}

/// Signature for `string.concat(...)` and `bytes.concat(...)`.
fn static_type_member_signature(line: &str, dot_pos: usize, member: &str) -> Option<(String, String)> {
    // Extract the identifier before the dot.
    let bytes = line.as_bytes();
    let mut pos = dot_pos;
    while pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_') {
        pos -= 1;
    }
    if pos == dot_pos {
        return None;
    }
    let prefix = &line[pos..dot_pos];

    match (prefix, member) {
        ("string", "concat") => {
            Some(("function string.concat(...) returns (string memory)".to_string(),
                  "Concatenates variable number of `string` arguments to one string array without padding.".to_string()))
        }
        ("bytes", "concat") => {
            Some(("function bytes.concat(...) returns (bytes memory)".to_string(),
                  "Concatenates variable number of `bytes` and `bytes1`, ..., `bytes32` arguments to one byte array without padding.".to_string()))
        }
        _ => None,
    }
}

fn build_signature(decl: &crate::symbol_table::Declaration) -> String {
    match decl.kind() {
        DeclKind::Function => {
            let params = format_params(decl.parameters());
            let returns = format_params(decl.return_parameters());
            let mut sig = format!("function {}({params})", decl.name);
            if let Some(vis) = decl.visibility() {
                sig.push_str(&format!(" {vis}"));
            }
            if let Some(sm) = decl.state_mutability() {
                if sm != "nonpayable" {
                    sig.push_str(&format!(" {sm}"));
                }
            }
            if !returns.is_empty() {
                sig.push_str(&format!(" returns ({returns})"));
            }
            sig
        }
        DeclKind::Constructor => {
            let params = format_params(decl.parameters());
            format!("constructor({params})")
        }
        DeclKind::FallbackReceive => {
            let params = format_params(decl.parameters());
            if decl.name == "receive" {
                "receive() external payable".to_string()
            } else {
                format!("fallback({params})")
            }
        }
        DeclKind::Modifier => {
            let params = format_params(decl.parameters());
            format!("modifier {}({params})", decl.name)
        }
        DeclKind::Event => {
            let params = format_params(decl.parameters());
            format!("event {}({params})", decl.name)
        }
        DeclKind::Error => {
            let params = format_params(decl.parameters());
            format!("error {}({params})", decl.name)
        }
        DeclKind::Contract | DeclKind::Interface | DeclKind::Library => {
            let keyword = match decl.kind() {
                DeclKind::Interface => "interface",
                DeclKind::Library => "library",
                _ => "contract",
            };
            let mut sig = format!("{keyword} {}", decl.name);
            let base = decl.base_contracts();
            if !base.is_empty() {
                sig.push_str(&format!(" is {}", base.join(", ")));
            }
            sig
        }
        DeclKind::Struct => {
            let mut sig = format!("struct {} {{\n", decl.name);
            for member in decl.members() {
                sig.push_str(&format!("    {} {};\n", member.type_text, member.name));
            }
            sig.push('}');
            sig
        }
        DeclKind::Enum => {
            let mut sig = format!("enum {} {{\n", decl.name);
            for val in decl.enum_values() {
                sig.push_str(&format!("    {val},\n"));
            }
            sig.push('}');
            sig
        }
        DeclKind::StateVariable
        | DeclKind::LocalVariable
        | DeclKind::Parameter
        | DeclKind::Constant => {
            let type_str = decl.type_text().unwrap_or("unknown");
            let mut sig = type_str.to_string();
            if let Some(vis) = decl.visibility() {
                sig.push_str(&format!(" {vis}"));
            }
            if decl.is_constant() {
                sig.push_str(" constant");
            }
            if decl.is_immutable() {
                sig.push_str(" immutable");
            }
            sig.push_str(&format!(" {}", decl.name));
            sig
        }
        DeclKind::EnumValue => decl.name.clone(),
        DeclKind::UserDefinedType => format!("type {}", decl.name),
        DeclKind::ImportAlias => {
            if let Some(target) = decl.import_target() {
                match target.kind {
                    Some(DeclKind::Contract) => format!("contract {}", decl.name),
                    Some(DeclKind::Interface) => format!("interface {}", decl.name),
                    Some(DeclKind::Library) => format!("library {}", decl.name),
                    Some(DeclKind::Struct) => format!("struct {}", decl.name),
                    Some(DeclKind::Enum) => format!("enum {}", decl.name),
                    _ => format!("import {}", decl.name),
                }
            } else {
                format!("import {}", decl.name)
            }
        }
    }
}

fn format_params(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(ty, name)| {
            if name.is_empty() {
                ty.clone()
            } else {
                format!("{ty} {name}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Format NatSpec documentation as markdown.
pub fn format_natspec(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut in_params = false;
    let mut in_returns = false;

    for raw_line in text.lines() {
        let line = raw_line.trim().trim_start_matches('*').trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("@notice ") {
            in_params = false;
            in_returns = false;
            lines.push(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("@dev ") {
            in_params = false;
            in_returns = false;
            lines.push(String::new());
            lines.push(format!("*{rest}*"));
        } else if let Some(rest) = line.strip_prefix("@param ") {
            if !in_params {
                in_params = true;
                in_returns = false;
                lines.push(String::new());
                lines.push("**Parameters:**".to_string());
            }
            if let Some((name, desc)) = rest.split_once(' ') {
                lines.push(format!("- `{name}` — {desc}"));
            } else {
                lines.push(format!("- `{rest}`"));
            }
        } else if let Some(rest) = line.strip_prefix("@return ") {
            if !in_returns {
                in_returns = true;
                in_params = false;
                lines.push(String::new());
                lines.push("**Returns:**".to_string());
            }
            if let Some((name, desc)) = rest.split_once(' ') {
                lines.push(format!("- `{name}` — {desc}"));
            } else {
                lines.push(format!("- `{rest}`"));
            }
        } else if line.starts_with("@author ") {
            // skip
        } else if line.starts_with("@inheritdoc ") {
            let parent = line.strip_prefix("@inheritdoc ").unwrap_or("");
            lines.push(format!("*Inherits documentation from `{parent}`*"));
        } else {
            lines.push(line.to_string());
        }
    }

    lines.join("\n")
}
