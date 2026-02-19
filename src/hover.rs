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

        // Add ABI information for functions, events, errors, and interfaces.
        if let Some(abi_info) = build_abi_info(decl, st, file) {
            parts.push(abi_info);
        }

        // Show computed value for constants.
        if decl.is_constant() || decl.is_immutable() {
            if let Some(value) = evaluate_constant_initializer(decl, source) {
                parts.push(format!("Value: `{value}`"));
            }
        }

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
    let line_text = source[line_start..].lines().next().unwrap_or("");
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

    // If no dot before the token, try type(X) hover (cursor on `type` keyword
    // or on the type name inside the parentheses).
    if tok_start == 0 || bytes[tok_start - 1] != b'.' {
        return type_expr_hover(line_text, tok_start, tok_end, member);
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

/// Hover when the cursor is on the `type` keyword or the type name inside `type(X)`.
fn type_expr_hover(line: &str, tok_start: usize, tok_end: usize, token: &str) -> Option<Hover> {
    let bytes = line.as_bytes();

    if token == "type" {
        // Cursor on `type` keyword — look for `(X)` after it.
        let mut pos = tok_end;
        // Skip optional whitespace.
        while pos < bytes.len() && bytes[pos] == b' ' {
            pos += 1;
        }
        if pos >= bytes.len() || bytes[pos] != b'(' {
            return None;
        }
        pos += 1;
        let name_start = pos;
        // Find matching ')'.
        let mut depth: u32 = 1;
        while pos < bytes.len() && depth > 0 {
            match bytes[pos] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            pos += 1;
        }
        if depth != 0 {
            return None;
        }
        let type_name = line[name_start..pos - 1].trim();
        if type_name.is_empty() {
            return None;
        }
        return Some(make_hover(
            &format!("type({type_name})"),
            Some(&format!(
                "Returns meta type information for `{type_name}`.\n\nMembers provide compile-time constants such as `.min`, `.max`, `.interfaceId`, `.name`, `.creationCode`, and `.runtimeCode`."
            )),
        ));
    }

    // Cursor on a type name inside `type(X)` — look backwards for `type(`.
    // Check if '(' precedes the token (possibly with whitespace).
    let mut pos = tok_start;
    while pos > 0 && bytes[pos - 1] == b' ' {
        pos -= 1;
    }
    if pos == 0 || bytes[pos - 1] != b'(' {
        return None;
    }
    let paren_pos = pos - 1;
    // Check that "type" precedes the '('.
    if paren_pos < 4 {
        return None;
    }
    if &line[paren_pos - 4..paren_pos] != "type" {
        return None;
    }
    // Make sure "type" isn't part of a larger identifier.
    if paren_pos > 4 {
        let prev = bytes[paren_pos - 5];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return None;
        }
    }
    // Verify closing ')' after the type name.
    let mut end = tok_end;
    while end < bytes.len() && bytes[end] == b' ' {
        end += 1;
    }
    if end >= bytes.len() || bytes[end] != b')' {
        return None;
    }

    Some(make_hover(
        &format!("type({token})"),
        Some(&format!(
            "Returns meta type information for `{token}`.\n\nMembers provide compile-time constants such as `.min`, `.max`, `.interfaceId`, `.name`, `.creationCode`, and `.runtimeCode`."
        )),
    ))
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
            "min" => Some((
                format!("{type_name} type({type_name}).min"),
                Some("The smallest value representable by type T.".to_string()),
            )),
            "max" => Some((
                format!("{type_name} type({type_name}).max"),
                Some("The largest value representable by type T.".to_string()),
            )),
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

fn type_member_signature_contract(
    type_name: &str,
    member: &str,
) -> Option<(String, Option<String>)> {
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
fn static_type_member_signature(
    line: &str,
    dot_pos: usize,
    member: &str,
) -> Option<(String, String)> {
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

// ---------------------------------------------------------------------------
// ABI information: canonical signature, selector, topic hash, interface ID
// ---------------------------------------------------------------------------

/// Compute keccak256 hash of a byte slice.
fn keccak256(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut output);
    output
}

/// Build the ABI canonical signature for a function/event/error.
/// E.g. `transfer(address,uint256)` — types only, no names, no spaces after commas.
fn abi_signature(name: &str, params: &[(String, String)]) -> String {
    let types: Vec<&str> = params.iter().map(|(ty, _)| canonicalize_type(ty)).collect();
    format!("{name}({})", types.join(","))
}

/// Canonicalize a Solidity type for ABI encoding.
/// Strips `memory`, `storage`, `calldata`; converts `uint`→`uint256`, `int`→`int256`, etc.
fn canonicalize_type(ty: &str) -> &str {
    let s = ty
        .strip_suffix(" memory")
        .or_else(|| ty.strip_suffix(" storage"))
        .or_else(|| ty.strip_suffix(" calldata"))
        .unwrap_or(ty)
        .trim();
    // Solidity ABI canonical forms.
    match s {
        "uint" => "uint256",
        "int" => "int256",
        "byte" => "bytes1",
        other => other,
    }
}

/// Build ABI information string for hover display.
/// Returns None for declaration kinds that don't have ABI info.
fn build_abi_info(
    decl: &crate::symbol_table::Declaration,
    st: &SymbolTable,
    file: &Path,
) -> Option<String> {
    match decl.kind() {
        DeclKind::Function => {
            let params = decl.parameters();
            let sig = abi_signature(&decl.name, params);
            let hash = keccak256(sig.as_bytes());
            let selector = format!("0x{}", hex(&hash[..4]));
            Some(format!("Selector: `{selector}` | Signature: `{sig}`"))
        }
        DeclKind::Event => {
            let params = decl.parameters();
            let sig = abi_signature(&decl.name, params);
            let hash = keccak256(sig.as_bytes());
            let topic = format!("0x{}", hex(&hash));
            Some(format!("Topic: `{topic}`\n\nSignature: `{sig}`"))
        }
        DeclKind::Error => {
            let params = decl.parameters();
            let sig = abi_signature(&decl.name, params);
            let hash = keccak256(sig.as_bytes());
            let selector = format!("0x{}", hex(&hash[..4]));
            Some(format!("Selector: `{selector}` | Signature: `{sig}`"))
        }
        DeclKind::Interface => {
            // ERC-165 interface ID: XOR of all function selectors.
            let members = st.members_of(&decl.name, file);
            let mut interface_id: u32 = 0;
            let mut has_functions = false;
            for member in members {
                if member.kind == DeclKind::Function {
                    // Look up function declaration for its parameters.
                    if let Some(func_decl) = member
                        .decl_id
                        .as_ref()
                        .and_then(|id| st.get_declaration(id))
                        .or_else(|| {
                            // Fall back to scope lookup.
                            let decl_id = st.find_type_decl(file, &decl.name)?;
                            let fi = st.files.get(&decl_id.file)?;
                            let scope = fi.scopes.iter().find(|s| s.owner == Some(decl_id))?;
                            let func_id = scope.get_decl(&member.name)?;
                            fi.declarations.get(func_id)
                        })
                    {
                        let sig = abi_signature(&func_decl.name, func_decl.parameters());
                        let hash = keccak256(sig.as_bytes());
                        let selector = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]);
                        interface_id ^= selector;
                        has_functions = true;
                    }
                }
            }
            if has_functions {
                Some(format!("ERC-165 Interface ID: `0x{interface_id:08x}`"))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Format bytes as hex string.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Constant expression evaluation
// ---------------------------------------------------------------------------

/// Extract and evaluate a constant's initializer expression.
/// Returns the computed value as a string for simple expressions.
fn evaluate_constant_initializer(
    decl: &crate::symbol_table::Declaration,
    source: &str,
) -> Option<String> {
    // Extract the initializer from the source text.
    let decl_text = source.get(decl.full_range.0..decl.full_range.1)?;
    let eq_pos = decl_text.find('=')?;
    let after_eq = decl_text[eq_pos + 1..].trim();
    // Strip trailing semicolon.
    let expr = after_eq.strip_suffix(';').unwrap_or(after_eq).trim();

    if expr.is_empty() {
        return None;
    }

    // Try to evaluate as a simple constant expression.
    eval_expr(expr)
}

/// Evaluate a simple constant expression.
/// Supports: integer literals (decimal, hex), basic arithmetic (+, -, *, /, %, **),
/// bitwise operations (&, |, ^, <<, >>), and parenthesized sub-expressions.
fn eval_expr(expr: &str) -> Option<String> {
    let expr = expr.trim();

    // String literal — return as-is.
    if (expr.starts_with('"') && expr.ends_with('"'))
        || (expr.starts_with('\'') && expr.ends_with('\''))
    {
        return Some(expr.to_string());
    }

    // Boolean literal.
    if expr == "true" || expr == "false" {
        return Some(expr.to_string());
    }

    // Try numeric evaluation.
    if let Some(val) = eval_numeric(expr) {
        if val >= 0 {
            return Some(format!("{val} (0x{val:x})"));
        } else {
            return Some(format!("{val}"));
        }
    }

    // If it's a simple literal that doesn't need evaluation, return it.
    if expr.starts_with("0x") || expr.starts_with("0X") {
        // Already a hex literal — show as decimal too.
        let hex_str = &expr[2..];
        if let Ok(val) = i128::from_str_radix(hex_str, 16) {
            return Some(format!("{val} ({expr})"));
        }
    }

    None
}

/// Evaluate a numeric expression, returning the computed i128 value.
fn eval_numeric(expr: &str) -> Option<i128> {
    let expr = expr.trim();

    // Parenthesized expression.
    if expr.starts_with('(') && expr.ends_with(')') {
        let inner = &expr[1..expr.len() - 1];
        // Verify parens are balanced.
        let mut depth = 0i32;
        for ch in inner.chars() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth == 0 {
            return eval_numeric(inner);
        }
    }

    // Try binary operators (lowest precedence first).
    // Order: |, ^, &, <<, >>, +, -, *, /, %, **
    for &op in &["|", "^", "&", "<<", ">>", "+", "-", "*", "/", "%", "**"] {
        if let Some((left, right)) = split_binary_op(expr, op) {
            let lhs = eval_numeric(left)?;
            let rhs = eval_numeric(right)?;
            return match op {
                "+" => Some(lhs.checked_add(rhs)?),
                "-" => Some(lhs.checked_sub(rhs)?),
                "*" => Some(lhs.checked_mul(rhs)?),
                "/" => {
                    if rhs == 0 {
                        None
                    } else {
                        Some(lhs / rhs)
                    }
                }
                "%" => {
                    if rhs == 0 {
                        None
                    } else {
                        Some(lhs % rhs)
                    }
                }
                "**" => {
                    if rhs < 0 || rhs > 128 {
                        None
                    } else {
                        Some(lhs.checked_pow(rhs as u32)?)
                    }
                }
                "<<" => Some(lhs.checked_shl(rhs as u32)?),
                ">>" => Some(lhs.checked_shr(rhs as u32)?),
                "|" => Some(lhs | rhs),
                "^" => Some(lhs ^ rhs),
                "&" => Some(lhs & rhs),
                _ => None,
            };
        }
    }

    // Integer literal.
    if expr.starts_with("0x") || expr.starts_with("0X") {
        let hex_str = &expr[2..].replace('_', "");
        return i128::from_str_radix(hex_str, 16).ok();
    }

    // Decimal literal (may contain underscores).
    let clean = expr.replace('_', "");
    clean.parse::<i128>().ok()
}

/// Split an expression on a binary operator at the top level (not inside parens).
/// Returns (left, right) or None if the operator isn't found at the top level.
fn split_binary_op<'a>(expr: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    let bytes = expr.as_bytes();
    let op_bytes = op.as_bytes();
    let op_len = op_bytes.len();
    let mut depth = 0i32;

    // Scan from right to left for lowest-precedence operators,
    // from left to right for highest-precedence (** is right-assoc).
    let is_right_assoc = op == "**";
    let indices: Box<dyn Iterator<Item = usize>> = if is_right_assoc {
        Box::new(0..expr.len())
    } else {
        Box::new((0..expr.len()).rev())
    };

    for i in indices {
        match bytes[i] {
            b'(' => {
                if is_right_assoc {
                    depth += 1;
                } else {
                    depth -= 1;
                }
            }
            b')' => {
                if is_right_assoc {
                    depth -= 1;
                } else {
                    depth += 1;
                }
            }
            _ => {}
        }
        if depth != 0 {
            continue;
        }
        if i + op_len <= expr.len() && &bytes[i..i + op_len] == op_bytes {
            // Avoid matching `-` in a negative number at position 0.
            if op == "-" && i == 0 {
                continue;
            }
            // Avoid matching `*` when it's part of `**`.
            if op == "*" && i + 1 < expr.len() && bytes[i + 1] == b'*' {
                continue;
            }
            if op == "*" && i > 0 && bytes[i - 1] == b'*' {
                continue;
            }
            // Avoid matching `>` or `<` when it's part of `>>` or `<<`.
            if op == ">" && i + 1 < expr.len() && bytes[i + 1] == b'>' {
                continue;
            }
            if op == "<" && i + 1 < expr.len() && bytes[i + 1] == b'<' {
                continue;
            }
            let left = expr[..i].trim();
            let right = expr[i + op_len..].trim();
            if !left.is_empty() && !right.is_empty() {
                return Some((left, right));
            }
        }
    }
    None
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
