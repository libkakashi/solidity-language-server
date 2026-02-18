#![allow(deprecated)]

use std::path::Path;

use tower_lsp::lsp_types::{
    DocumentSymbol, Location, Position, Range, SymbolInformation, SymbolKind, Url,
};

use crate::symbol_table::{DeclKind, Declaration, FileIndex, ScopeKind, SymbolTable, SYNTHETIC_BASE};
use crate::utils::LineIndex;

/// Extract document symbols for a single file (hierarchical tree).
pub fn document_symbols(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    line_index: &LineIndex,
) -> Vec<DocumentSymbol> {
    let fi = match st.get_file_index(file) {
        Some(fi) => fi,
        None => return vec![],
    };

    let mut top_level = Vec::new();

    for decl in fi.declarations.values() {
        if decl.scope != 0 || decl.id.byte_offset >= SYNTHETIC_BASE {
            continue;
        }
        match decl.kind {
            DeclKind::Contract | DeclKind::Interface | DeclKind::Library => {
                let children = collect_children(fi, source, decl, line_index);
                top_level.push(make_document_symbol(decl, source, children, line_index));
            }
            DeclKind::ImportAlias => {
                top_level.push(make_document_symbol(decl, source, vec![], line_index));
            }
            _ => {
                top_level.push(make_document_symbol(decl, source, vec![], line_index));
            }
        }
    }

    top_level.sort_by_key(|s| (s.range.start.line, s.range.start.character));
    top_level
}

/// Extract workspace symbols (flat list across all files).
/// Uses cached source from symbol table when available. (Fix #8)
pub fn workspace_symbols(st: &SymbolTable, query: &str) -> Vec<SymbolInformation> {
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for (&file_id, fi) in &st.files {
        let path = st.resolve_path(file_id);

        // Use cached source from symbol table, fall back to disk. (Fix #8)
        let source: String;
        let source_ref = if let Some(cached) = st.get_source(file_id) {
            cached
        } else {
            match std::fs::read_to_string(path) {
                Ok(s) => {
                    source = s;
                    source.as_str()
                }
                Err(_) => continue,
            }
        };

        let uri = match Url::from_file_path(path) {
            Ok(u) => u,
            Err(_) => continue,
        };

        let ws_line_index = LineIndex::new(source_ref);

        for decl in fi.declarations.values() {
            if !query_lower.is_empty() && !decl.name.to_lowercase().contains(&query_lower) {
                continue;
            }
            if matches!(decl.kind, DeclKind::Parameter | DeclKind::LocalVariable)
                || decl.id.byte_offset >= SYNTHETIC_BASE
            {
                continue;
            }

            let container_name = find_container_name(fi, decl);
            let (sl, sc) = ws_line_index.byte_offset_to_position(source_ref, decl.full_range.0);
            let (el, ec) = ws_line_index.byte_offset_to_position(source_ref, decl.full_range.1);

            results.push(SymbolInformation {
                name: decl.name.clone(),
                kind: decl_kind_to_symbol_kind(decl.kind),
                tags: None,
                deprecated: None,
                location: Location {
                    uri: uri.clone(),
                    range: Range {
                        start: Position {
                            line: sl,
                            character: sc,
                        },
                        end: Position {
                            line: el,
                            character: ec,
                        },
                    },
                },
                container_name,
            });
        }
    }

    results
}

fn collect_children(
    fi: &FileIndex,
    source: &str,
    parent: &Declaration,
    line_index: &LineIndex,
) -> Vec<DocumentSymbol> {
    let contract_scope = fi.scopes.iter().find(|s| {
        matches!(
            s.kind,
            ScopeKind::Contract | ScopeKind::Interface | ScopeKind::Library
        ) && s.range == parent.full_range
            || (s.range.0 >= parent.full_range.0
                && s.range.1 <= parent.full_range.1
                && matches!(
                    s.kind,
                    ScopeKind::Contract | ScopeKind::Interface | ScopeKind::Library
                )
                && s.parent == Some(parent.scope))
    });

    let scope_id = match contract_scope {
        Some(s) => s.id,
        None => return vec![],
    };

    let mut children: Vec<DocumentSymbol> = fi
        .declarations
        .values()
        .filter(|d| d.scope == scope_id && d.id != parent.id)
        .map(|d| {
            let grandchildren = if matches!(d.kind, DeclKind::Struct | DeclKind::Enum) {
                d.members()
                    .iter()
                    .map(|m| {
                        let (sl, sc) = line_index.byte_offset_to_position(source, m.name_range.0);
                        let (el, ec) = line_index.byte_offset_to_position(source, m.name_range.1);
                        DocumentSymbol {
                            name: m.name.clone(),
                            detail: Some(m.type_text.clone()),
                            kind: match m.kind {
                                DeclKind::EnumValue => SymbolKind::ENUM_MEMBER,
                                _ => SymbolKind::FIELD,
                            },
                            range: Range {
                                start: Position {
                                    line: sl,
                                    character: sc,
                                },
                                end: Position {
                                    line: el,
                                    character: ec,
                                },
                            },
                            selection_range: Range {
                                start: Position {
                                    line: sl,
                                    character: sc,
                                },
                                end: Position {
                                    line: el,
                                    character: ec,
                                },
                            },
                            children: None,
                            tags: None,
                            deprecated: None,
                        }
                    })
                    .collect()
            } else {
                vec![]
            };
            make_document_symbol(d, source, grandchildren, line_index)
        })
        .collect();

    children.sort_by_key(|s| (s.range.start.line, s.range.start.character));
    children
}

fn make_document_symbol(
    decl: &Declaration,
    source: &str,
    children: Vec<DocumentSymbol>,
    line_index: &LineIndex,
) -> DocumentSymbol {
    let (sl, sc) = line_index.byte_offset_to_position(source, decl.full_range.0);
    let (el, ec) = line_index.byte_offset_to_position(source, decl.full_range.1);
    let (nsl, nsc) = line_index.byte_offset_to_position(source, decl.name_range.0);
    let (nel, nec) = line_index.byte_offset_to_position(source, decl.name_range.1);

    DocumentSymbol {
        name: decl.name.clone(),
        detail: decl.type_text.clone(),
        kind: decl_kind_to_symbol_kind(decl.kind),
        range: Range {
            start: Position {
                line: sl,
                character: sc,
            },
            end: Position {
                line: el,
                character: ec,
            },
        },
        selection_range: Range {
            start: Position {
                line: nsl,
                character: nsc,
            },
            end: Position {
                line: nel,
                character: nec,
            },
        },
        children: if children.is_empty() {
            None
        } else {
            Some(children)
        },
        tags: None,
        deprecated: None,
    }
}

fn decl_kind_to_symbol_kind(kind: DeclKind) -> SymbolKind {
    match kind {
        DeclKind::Contract | DeclKind::Interface | DeclKind::Library => SymbolKind::CLASS,
        DeclKind::Function => SymbolKind::FUNCTION,
        DeclKind::Constructor => SymbolKind::CONSTRUCTOR,
        DeclKind::FallbackReceive => SymbolKind::FUNCTION,
        DeclKind::Modifier => SymbolKind::METHOD,
        DeclKind::Event => SymbolKind::EVENT,
        DeclKind::Error => SymbolKind::EVENT,
        DeclKind::Struct => SymbolKind::STRUCT,
        DeclKind::Enum => SymbolKind::ENUM,
        DeclKind::EnumValue => SymbolKind::ENUM_MEMBER,
        DeclKind::StateVariable | DeclKind::Constant => SymbolKind::FIELD,
        DeclKind::LocalVariable | DeclKind::Parameter => SymbolKind::VARIABLE,
        DeclKind::UserDefinedType => SymbolKind::CLASS,
        DeclKind::ImportAlias => SymbolKind::MODULE,
    }
}

fn find_container_name(fi: &FileIndex, decl: &Declaration) -> Option<String> {
    let scope = fi.scopes.get(decl.scope)?;
    if matches!(
        scope.kind,
        ScopeKind::Contract | ScopeKind::Interface | ScopeKind::Library
    ) {
        for d in fi.declarations.values() {
            if matches!(
                d.kind,
                DeclKind::Contract | DeclKind::Interface | DeclKind::Library
            ) && d.full_range.0 <= scope.range.0
                && d.full_range.1 >= scope.range.1
                && d.id != decl.id
            {
                return Some(d.name.clone());
            }
        }
    }
    if let Some(parent_id) = scope.parent {
        if let Some(parent_scope) = fi.scopes.get(parent_id) {
            if matches!(
                parent_scope.kind,
                ScopeKind::Contract | ScopeKind::Interface | ScopeKind::Library
            ) {
                for d in fi.declarations.values() {
                    if matches!(
                        d.kind,
                        DeclKind::Contract | DeclKind::Interface | DeclKind::Library
                    ) && d.full_range.0 <= parent_scope.range.0
                        && d.full_range.1 >= parent_scope.range.1
                    {
                        return Some(d.name.clone());
                    }
                }
            }
        }
    }
    None
}
