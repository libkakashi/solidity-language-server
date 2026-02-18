use std::path::Path;

use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};

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
    let decl = st.resolve_at(file, byte_offset)?;

    let mut parts: Vec<String> = Vec::new();

    let sig = build_signature(decl);
    parts.push(format!("```solidity\n{sig}\n```"));

    if let Some(natspec) = decl.natspec() {
        let formatted = format_natspec(natspec);
        if !formatted.is_empty() {
            parts.push(format!("---\n{formatted}"));
        }
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: parts.join("\n\n"),
        }),
        range: None,
    })
}

fn build_signature(decl: &crate::symbol_table::Declaration) -> String {
    match decl.kind {
        DeclKind::Function => {
            let params = format_params(decl.parameters());
            let returns = format_params(decl.return_parameters());
            let mut sig = format!("function {}({params})", decl.name);
            if let Some(ref vis) = decl.visibility {
                sig.push_str(&format!(" {vis}"));
            }
            if let Some(ref sm) = decl.state_mutability {
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
            let keyword = match decl.kind {
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
            let type_str = decl.type_text.as_deref().unwrap_or("unknown");
            let mut sig = type_str.to_string();
            if let Some(ref vis) = decl.visibility {
                sig.push_str(&format!(" {vis}"));
            }
            if decl.is_constant {
                sig.push_str(" constant");
            }
            if decl.is_immutable {
                sig.push_str(" immutable");
            }
            sig.push_str(&format!(" {}", decl.name));
            sig
        }
        DeclKind::EnumValue => decl.name.clone(),
        DeclKind::UserDefinedType => format!("type {}", decl.name),
        DeclKind::ImportAlias => format!("import alias {}", decl.name),
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
