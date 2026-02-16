use std::path::Path;
use std::sync::OnceLock;

use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, Position,
};

use crate::symbol_table::{DeclKind, MemberInfo, SymbolTable};
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

    let items = if trigger_char == Some(".") {
        get_dot_completions(st, file, line, col_byte, abs_byte)
    } else {
        get_general_completions(st, file, abs_byte)
    };

    Some(CompletionResponse::List(CompletionList {
        is_incomplete: false,
        items,
    }))
}

fn get_dot_completions(
    st: &SymbolTable,
    file: &Path,
    line: &str,
    col_byte: u32,
    cursor_byte: usize,
) -> Vec<CompletionItem> {
    let identifier = extract_identifier_before_dot(line, col_byte);
    let identifier = match identifier {
        Some(id) => id,
        None => return vec![],
    };

    // Check magic globals first.
    if let Some(items) = magic_members(&identifier) {
        return items;
    }

    // Try to find the type by looking up the identifier in the symbol table.
    let scope = st.scope_at(file, cursor_byte).unwrap_or(0);

    let visible = st.visible_declarations(file, scope);
    for decl in &visible {
        if decl.name == identifier {
            let members = decl.members();
            if !members.is_empty() {
                return members.iter().map(member_to_completion).collect();
            }
            if let Some(ref type_text) = decl.type_text {
                let members = st.members_of(type_text, file);
                if !members.is_empty() {
                    return members.iter().map(member_to_completion).collect();
                }
            }
        }
    }

    // Try members_of directly (e.g. "ContractName.")
    let members = st.members_of(&identifier, file);
    if !members.is_empty() {
        return members.iter().map(member_to_completion).collect();
    }

    vec![]
}

fn get_general_completions(
    st: &SymbolTable,
    file: &Path,
    byte_offset: usize,
) -> Vec<CompletionItem> {
    let scope_id = st.scope_at(file, byte_offset).unwrap_or(0);
    let visible = st.visible_declarations(file, scope_id);

    let mut items: Vec<CompletionItem> = visible
        .iter()
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
        "type" => vec![
            ("name", "string"),
            ("creationCode", "bytes memory"),
            ("runtimeCode", "bytes memory"),
            ("interfaceId", "bytes4"),
            ("min", "T"),
            ("max", "T"),
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
