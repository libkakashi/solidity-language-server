use std::path::Path;

use tower_lsp::lsp_types::*;
use tree_sitter::{Node, Tree};

use crate::symbol_table::{DeclKind, SymbolTable};
use crate::utils::LineIndex;

// ---------------------------------------------------------------------------
// Token type and modifier indices (must match LEGEND order)
// ---------------------------------------------------------------------------

// Token types — indices into SemanticTokensLegend::token_types
const TT_NAMESPACE: u32 = 0; // contract, interface, library
const TT_TYPE: u32 = 1; // struct, enum, user-defined type
const TT_FUNCTION: u32 = 2; // function, modifier names
const TT_VARIABLE: u32 = 3; // state variables, locals, params
const TT_PROPERTY: u32 = 4; // struct fields, enum values
const TT_EVENT: u32 = 5; // event names
const _TT_KEYWORD: u32 = 6; // Solidity keywords (reserved for future use)
const TT_NUMBER: u32 = 7; // number literals
const TT_STRING: u32 = 8; // string literals
const TT_COMMENT: u32 = 9; // comments
const TT_MACRO: u32 = 10; // custom error names
const TT_PARAMETER: u32 = 11; // function/event parameters
const TT_ENUM_MEMBER: u32 = 12; // enum values

// Token modifiers — bit flags
const TM_DECLARATION: u32 = 1 << 0;
const TM_READONLY: u32 = 1 << 1;
const TM_STATIC: u32 = 1 << 2;
const TM_DEFINITION: u32 = 1 << 3;

/// Build the legend that the server advertises to the client.
pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::NAMESPACE,   // 0
            SemanticTokenType::TYPE,        // 1
            SemanticTokenType::FUNCTION,    // 2
            SemanticTokenType::VARIABLE,    // 3
            SemanticTokenType::PROPERTY,    // 4
            SemanticTokenType::EVENT,       // 5
            SemanticTokenType::KEYWORD,     // 6
            SemanticTokenType::NUMBER,      // 7
            SemanticTokenType::STRING,      // 8
            SemanticTokenType::COMMENT,     // 9
            SemanticTokenType::MACRO,       // 10 (used for error types)
            SemanticTokenType::PARAMETER,   // 11
            SemanticTokenType::ENUM_MEMBER, // 12
        ],
        token_modifiers: vec![
            SemanticTokenModifier::DECLARATION, // bit 0
            SemanticTokenModifier::READONLY,    // bit 1
            SemanticTokenModifier::STATIC,      // bit 2
            SemanticTokenModifier::DEFINITION,  // bit 3
        ],
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Compute full-document semantic tokens.
pub fn semantic_tokens_full(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    line_index: &LineIndex,
    tree: Option<&Tree>,
) -> Option<SemanticTokensResult> {
    let tree = tree?;
    let mut tokens = Vec::new();
    walk_for_tokens(tree.root_node(), source, st, file, &mut tokens);

    // Sort by byte offset (tokens are mostly in order from the walk, but
    // ensure stability).
    tokens.sort_by_key(|t| t.0);

    // Convert absolute positions to LSP delta-encoded format.
    let mut result = Vec::with_capacity(tokens.len());
    let mut prev_line: u32 = 0;
    let mut prev_start: u32 = 0;

    for (byte_start, byte_end, token_type, modifiers) in &tokens {
        let pos = line_index.byte_offset_to_lsp_position(source, *byte_start);
        // Compute length in the negotiated encoding (UTF-16 or UTF-8).
        let length = compute_token_length(source, *byte_start, *byte_end);
        if length == 0 {
            continue;
        }

        let delta_line = pos.line - prev_line;
        let delta_start = if delta_line == 0 {
            pos.character - prev_start
        } else {
            pos.character
        };

        result.push(SemanticToken {
            delta_line,
            delta_start,
            length: length as u32,
            token_type: *token_type,
            token_modifiers_bitset: *modifiers,
        });

        prev_line = pos.line;
        prev_start = pos.character;
    }

    Some(SemanticTokensResult::Tokens(SemanticTokens {
        result_id: None,
        data: result,
    }))
}

/// Compute the length of a token in the negotiated encoding (UTF-16 or UTF-8).
fn compute_token_length(source: &str, byte_start: usize, byte_end: usize) -> u32 {
    match crate::utils::encoding() {
        crate::utils::PositionEncoding::Utf8 => (byte_end - byte_start) as u32,
        crate::utils::PositionEncoding::Utf16 => {
            let segment = &source[byte_start..byte_end];
            segment.chars().map(|c| c.len_utf16() as u32).sum()
        }
    }
}

// ---------------------------------------------------------------------------
// Token: (byte_start, byte_end, token_type, modifiers)
// ---------------------------------------------------------------------------
type Token = (usize, usize, u32, u32);

// ---------------------------------------------------------------------------
// Tree walker
// ---------------------------------------------------------------------------

fn walk_for_tokens(
    node: Node,
    source: &str,
    st: &SymbolTable,
    file: &Path,
    tokens: &mut Vec<Token>,
) {
    match node.kind() {
        // Comments
        "comment" => {
            push_single_line_tokens(node, source, TT_COMMENT, 0, tokens);
        }

        // String literals
        "string_literal" | "hex_string_literal" | "unicode_string_literal" => {
            tokens.push((node.start_byte(), node.end_byte(), TT_STRING, 0));
        }

        // Number literals
        "number_literal" => {
            tokens.push((node.start_byte(), node.end_byte(), TT_NUMBER, 0));
        }

        // Contract/interface/library declarations — highlight the name
        "contract_declaration" | "interface_declaration" | "library_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_NAMESPACE,
                    TM_DECLARATION | TM_DEFINITION,
                ));
            }
            // Highlight base contract names in inheritance
            highlight_inheritance(node, source, tokens);
            walk_children(node, source, st, file, tokens);
        }

        // Function definitions — highlight the name
        "function_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_FUNCTION,
                    TM_DECLARATION | TM_DEFINITION,
                ));
            }
            walk_children(node, source, st, file, tokens);
        }

        // Constructor, fallback, receive
        "constructor_definition" | "fallback_receive_definition" => {
            walk_children(node, source, st, file, tokens);
        }

        // Modifier definitions
        "modifier_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_FUNCTION,
                    TM_DECLARATION | TM_DEFINITION,
                ));
            }
            walk_children(node, source, st, file, tokens);
        }

        // Event definitions
        "event_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_EVENT,
                    TM_DECLARATION | TM_DEFINITION,
                ));
            }
            walk_children(node, source, st, file, tokens);
        }

        // Error declarations
        "error_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_MACRO,
                    TM_DECLARATION | TM_DEFINITION,
                ));
            }
            walk_children(node, source, st, file, tokens);
        }

        // Struct declarations
        "struct_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_TYPE,
                    TM_DECLARATION | TM_DEFINITION,
                ));
            }
            walk_children(node, source, st, file, tokens);
        }

        // Enum declarations
        "enum_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_TYPE,
                    TM_DECLARATION | TM_DEFINITION,
                ));
            }
            walk_children(node, source, st, file, tokens);
        }

        // Enum values
        "enum_value" => {
            tokens.push((
                node.start_byte(),
                node.end_byte(),
                TT_ENUM_MEMBER,
                TM_DECLARATION,
            ));
        }

        // User-defined type definitions
        "user_defined_type_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_TYPE,
                    TM_DECLARATION | TM_DEFINITION,
                ));
            }
            walk_children(node, source, st, file, tokens);
        }

        // State variable declarations
        "state_variable_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                let is_const = has_child_kind(&node, "constant");
                let is_immutable = has_child_kind(&node, "immutable");
                let mods = TM_DECLARATION
                    | if is_const || is_immutable {
                        TM_READONLY | TM_STATIC
                    } else {
                        0
                    };
                tokens.push((name.start_byte(), name.end_byte(), TT_VARIABLE, mods));
            }
            walk_children(node, source, st, file, tokens);
        }

        // Constant variable declarations (file-level)
        "constant_variable_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_VARIABLE,
                    TM_DECLARATION | TM_READONLY | TM_STATIC,
                ));
            }
            walk_children(node, source, st, file, tokens);
        }

        // Parameters
        "parameter" | "event_parameter" | "error_parameter" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_PARAMETER,
                    TM_DECLARATION,
                ));
            }
            // Walk type children for user-defined type references
            if let Some(type_node) = node.child_by_field_name("type") {
                walk_for_tokens(type_node, source, st, file, tokens);
            }
        }

        // Variable declarations in statements
        "variable_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_VARIABLE,
                    TM_DECLARATION,
                ));
            }
            if let Some(type_node) = node.child_by_field_name("type") {
                walk_for_tokens(type_node, source, st, file, tokens);
            }
        }

        // Struct members
        "struct_member" => {
            if let Some(name) = node.child_by_field_name("name") {
                tokens.push((
                    name.start_byte(),
                    name.end_byte(),
                    TT_PROPERTY,
                    TM_DECLARATION,
                ));
            }
            if let Some(type_node) = node.child_by_field_name("type") {
                walk_for_tokens(type_node, source, st, file, tokens);
            }
        }

        // Emit statement — highlight the event name
        "emit_statement" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() && child.kind() == "identifier" {
                    tokens.push((child.start_byte(), child.end_byte(), TT_EVENT, 0));
                } else {
                    walk_for_tokens(child, source, st, file, tokens);
                }
            }
        }

        // Modifier invocations — highlight the modifier name
        "modifier_invocation" => {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                if cursor.node().kind() == "identifier" {
                    tokens.push((
                        cursor.node().start_byte(),
                        cursor.node().end_byte(),
                        TT_FUNCTION,
                        0,
                    ));
                }
            }
            walk_children(node, source, st, file, tokens);
        }

        // Call expressions — resolve the callee to determine token type
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                highlight_callee(func, source, st, file, tokens);
            }
            // Walk arguments but skip the function child (already handled).
            let func_id = node.child_by_field_name("function").map(|f| f.id());
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if Some(child.id()) == func_id {
                    continue;
                }
                walk_for_tokens(child, source, st, file, tokens);
            }
        }

        // Member expressions — highlight property based on resolved type
        "member_expression" => {
            if let Some(obj) = node.child_by_field_name("object") {
                walk_for_tokens(obj, source, st, file, tokens);
            }
            if let Some(prop) = node.child_by_field_name("property") {
                let token_type = resolve_member_token_type(st, file, prop.start_byte());
                tokens.push((prop.start_byte(), prop.end_byte(), token_type, 0));
            }
        }

        // User-defined type references (in type positions)
        "user_defined_type" => {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "identifier" {
                        let token_type =
                            resolve_identifier_token_type(st, file, child.start_byte());
                        tokens.push((child.start_byte(), child.end_byte(), token_type, 0));
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }

        // Bare identifiers (references)
        "identifier" => {
            // Skip if this is a declaration name (handled by parent).
            if !is_declaration_name_ctx(&node) {
                let token_type = resolve_identifier_token_type(st, file, node.start_byte());
                tokens.push((node.start_byte(), node.end_byte(), token_type, 0));
            }
        }

        // Primitive types — highlight as type keyword
        "primitive_type" => {
            tokens.push((node.start_byte(), node.end_byte(), TT_TYPE, 0));
        }

        // Skip recursing into string children (already captured above)
        "string" => {}

        // Everything else: recurse into children
        _ => {
            walk_children(node, source, st, file, tokens);
        }
    }
}

fn walk_children(node: Node, source: &str, st: &SymbolTable, file: &Path, tokens: &mut Vec<Token>) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk_for_tokens(cursor.node(), source, st, file, tokens);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Push tokens for a comment that might span multiple lines.
/// Semantic tokens must not span multiple lines in the LSP protocol.
fn push_single_line_tokens(
    node: Node,
    source: &str,
    token_type: u32,
    modifiers: u32,
    tokens: &mut Vec<Token>,
) {
    let text = &source[node.start_byte()..node.end_byte()];
    if !text.contains('\n') {
        tokens.push((node.start_byte(), node.end_byte(), token_type, modifiers));
        return;
    }
    // Split multi-line token into one token per line.
    let mut offset = node.start_byte();
    for line in text.split('\n') {
        let line_bytes = line.len();
        if line_bytes > 0 {
            // Trim trailing \r for Windows line endings.
            let trimmed_len = line.trim_end_matches('\r').len();
            if trimmed_len > 0 {
                tokens.push((offset, offset + trimmed_len, token_type, modifiers));
            }
        }
        offset += line_bytes + 1; // +1 for the \n
    }
}

fn has_child_kind(node: &Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.node().kind() == kind {
                return true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

/// Check if this identifier node is a declaration name (handled by parent node).
fn is_declaration_name_ctx(node: &Node) -> bool {
    if let Some(parent) = node.parent() {
        match parent.kind() {
            "contract_declaration"
            | "interface_declaration"
            | "library_declaration"
            | "function_definition"
            | "modifier_definition"
            | "struct_declaration"
            | "enum_declaration"
            | "event_definition"
            | "error_declaration"
            | "state_variable_declaration"
            | "constant_variable_declaration"
            | "variable_declaration"
            | "parameter"
            | "event_parameter"
            | "error_parameter"
            | "struct_member"
            | "user_defined_type_definition" => {
                if let Some(name) = parent.child_by_field_name("name") {
                    return name.id() == node.id();
                }
            }
            "import_directive" => {
                // Import names are handled separately
                return true;
            }
            "enum_body" => {
                // Enum values handled by enum_value
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Highlight base contract names in inheritance specifiers.
fn highlight_inheritance(node: Node, _source: &str, tokens: &mut Vec<Token>) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.node().kind() == "inheritance_specifier" {
                if let Some(ancestor) = cursor.node().child_by_field_name("ancestor") {
                    let mut inner = ancestor.walk();
                    if inner.goto_first_child() {
                        loop {
                            if inner.node().kind() == "identifier" {
                                tokens.push((
                                    inner.node().start_byte(),
                                    inner.node().end_byte(),
                                    TT_NAMESPACE,
                                    0,
                                ));
                            }
                            if !inner.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Highlight the callee of a call expression.
fn highlight_callee(
    func: Node,
    source: &str,
    st: &SymbolTable,
    file: &Path,
    tokens: &mut Vec<Token>,
) {
    match func.kind() {
        "identifier" => {
            let token_type = resolve_call_token_type(st, file, func.start_byte());
            tokens.push((func.start_byte(), func.end_byte(), token_type, 0));
        }
        "member_expression" => {
            if let Some(obj) = func.child_by_field_name("object") {
                walk_for_tokens(obj, source, st, file, tokens);
            }
            if let Some(prop) = func.child_by_field_name("property") {
                let token_type = resolve_call_token_type(st, file, prop.start_byte());
                tokens.push((prop.start_byte(), prop.end_byte(), token_type, 0));
            }
        }
        _ => {
            walk_for_tokens(func, source, st, file, tokens);
        }
    }
}

/// Resolve the semantic token type for an identifier at a byte offset.
fn resolve_identifier_token_type(st: &SymbolTable, file: &Path, byte_offset: usize) -> u32 {
    if let Some(decl) = st.resolve_at(file, byte_offset) {
        decl_kind_to_token_type(decl.kind())
    } else {
        TT_VARIABLE // fallback
    }
}

/// Resolve the token type for a member access property.
fn resolve_member_token_type(st: &SymbolTable, file: &Path, byte_offset: usize) -> u32 {
    if let Some(decl) = st.resolve_at(file, byte_offset) {
        decl_kind_to_token_type(decl.kind())
    } else {
        TT_PROPERTY // fallback for unresolved members
    }
}

/// Resolve the token type for a function call.
fn resolve_call_token_type(st: &SymbolTable, file: &Path, byte_offset: usize) -> u32 {
    if let Some(decl) = st.resolve_at(file, byte_offset) {
        match decl.kind() {
            DeclKind::Event => TT_EVENT,
            DeclKind::Error => TT_MACRO,
            DeclKind::Contract | DeclKind::Interface | DeclKind::Library => TT_NAMESPACE,
            DeclKind::Struct => TT_TYPE,
            _ => TT_FUNCTION,
        }
    } else {
        TT_FUNCTION // fallback
    }
}

fn decl_kind_to_token_type(kind: DeclKind) -> u32 {
    match kind {
        DeclKind::Contract | DeclKind::Interface | DeclKind::Library | DeclKind::ImportAlias => {
            TT_NAMESPACE
        }
        DeclKind::Struct | DeclKind::Enum | DeclKind::UserDefinedType => TT_TYPE,
        DeclKind::Function
        | DeclKind::Constructor
        | DeclKind::FallbackReceive
        | DeclKind::Modifier => TT_FUNCTION,
        DeclKind::Event => TT_EVENT,
        DeclKind::Error => TT_MACRO,
        DeclKind::EnumValue => TT_ENUM_MEMBER,
        DeclKind::Parameter => TT_PARAMETER,
        DeclKind::StateVariable | DeclKind::LocalVariable | DeclKind::Constant => TT_VARIABLE,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_legend_sizes() {
        let leg = legend();
        // Ensure indices in constants are within bounds.
        assert!(TT_ENUM_MEMBER < leg.token_types.len() as u32);
        assert_eq!(leg.token_modifiers.len(), 4);
    }

    #[test]
    fn test_push_single_line_tokens_no_newline() {
        let mut tokens = Vec::new();
        let source = "// hello";
        // Create a minimal node-like scenario.
        push_single_line_tokens_raw(0, source.len(), source, TT_COMMENT, 0, &mut tokens);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], (0, 8, TT_COMMENT, 0));
    }

    #[test]
    fn test_push_single_line_tokens_multiline() {
        let mut tokens = Vec::new();
        let source = "/* line1\nline2\nline3 */";
        push_single_line_tokens_raw(0, source.len(), source, TT_COMMENT, 0, &mut tokens);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], (0, 8, TT_COMMENT, 0)); // "/* line1"
        assert_eq!(tokens[1], (9, 14, TT_COMMENT, 0)); // "line2"
        assert_eq!(tokens[2], (15, 23, TT_COMMENT, 0)); // "line3 */"
    }

    /// Helper that mimics push_single_line_tokens but works with byte ranges.
    fn push_single_line_tokens_raw(
        start: usize,
        end: usize,
        source: &str,
        token_type: u32,
        modifiers: u32,
        tokens: &mut Vec<Token>,
    ) {
        let text = &source[start..end];
        if !text.contains('\n') {
            tokens.push((start, end, token_type, modifiers));
            return;
        }
        let mut offset = start;
        for line in text.split('\n') {
            let line_bytes = line.len();
            if line_bytes > 0 {
                let trimmed_len = line.trim_end_matches('\r').len();
                if trimmed_len > 0 {
                    tokens.push((offset, offset + trimmed_len, token_type, modifiers));
                }
            }
            offset += line_bytes + 1;
        }
    }

    #[test]
    fn test_decl_kind_to_token_type() {
        assert_eq!(decl_kind_to_token_type(DeclKind::Contract), TT_NAMESPACE);
        assert_eq!(decl_kind_to_token_type(DeclKind::Function), TT_FUNCTION);
        assert_eq!(decl_kind_to_token_type(DeclKind::Struct), TT_TYPE);
        assert_eq!(decl_kind_to_token_type(DeclKind::Event), TT_EVENT);
        assert_eq!(decl_kind_to_token_type(DeclKind::Error), TT_MACRO);
        assert_eq!(decl_kind_to_token_type(DeclKind::EnumValue), TT_ENUM_MEMBER);
        assert_eq!(decl_kind_to_token_type(DeclKind::Parameter), TT_PARAMETER);
        assert_eq!(
            decl_kind_to_token_type(DeclKind::StateVariable),
            TT_VARIABLE
        );
    }
}
