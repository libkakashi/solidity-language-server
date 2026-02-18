use std::path::Path;

use tower_lsp::lsp_types::*;

use crate::symbol_table::{DeclKind, SymbolTable};
use crate::utils::LineIndex;

/// Compute code lenses for a document.
///
/// Shows reference counts above functions, state variables, events, errors,
/// contracts, interfaces, libraries, structs, and enums.
pub fn code_lens(
    st: &SymbolTable,
    file: &Path,
    source: &str,
    line_index: &LineIndex,
) -> Vec<CodeLens> {
    let file_id = match st.lookup_file_id(file) {
        Some(id) => id,
        None => return vec![],
    };
    let fi = match st.files.get(&file_id) {
        Some(fi) => fi,
        None => return vec![],
    };

    let mut lenses = Vec::new();

    for decl in fi.declarations.values() {
        // Only show lenses for "important" declarations.
        if !should_show_lens(decl.kind()) {
            continue;
        }

        let ref_count = st
            .ref_index
            .get(&decl.id)
            .map(|refs| refs.len())
            .unwrap_or(0);

        let range =
            line_index.byte_range_to_lsp_range(source, decl.name_range.0, decl.name_range.1);

        let title = match ref_count {
            0 => "0 references".to_string(),
            1 => "1 reference".to_string(),
            n => format!("{n} references"),
        };

        lenses.push(CodeLens {
            range,
            command: Some(Command {
                title,
                command: "solidity.showReferences".to_string(),
                arguments: None,
            }),
            data: None,
        });

        // For contracts/interfaces, also show implementation count.
        if matches!(
            decl.kind(),
            DeclKind::Contract | DeclKind::Interface | DeclKind::Library
        ) {
            let impl_count = count_implementations(st, &decl.name);
            if impl_count > 0 {
                let impl_title = match impl_count {
                    1 => "1 implementation".to_string(),
                    n => format!("{n} implementations"),
                };
                lenses.push(CodeLens {
                    range,
                    command: Some(Command {
                        title: impl_title,
                        command: "solidity.showImplementations".to_string(),
                        arguments: None,
                    }),
                    data: None,
                });
            }
        }
    }

    // Sort by position for consistent output.
    lenses.sort_by_key(|l| (l.range.start.line, l.range.start.character));
    lenses
}

fn should_show_lens(kind: DeclKind) -> bool {
    matches!(
        kind,
        DeclKind::Function
            | DeclKind::Constructor
            | DeclKind::Modifier
            | DeclKind::Event
            | DeclKind::Error
            | DeclKind::Contract
            | DeclKind::Interface
            | DeclKind::Library
            | DeclKind::Struct
            | DeclKind::Enum
            | DeclKind::StateVariable
    )
}

/// Count how many contracts inherit from the given name.
fn count_implementations(st: &SymbolTable, name: &str) -> usize {
    let mut count = 0;
    for (_, fi) in &st.files {
        for decl in fi.declarations.values() {
            if matches!(
                decl.kind(),
                DeclKind::Contract | DeclKind::Interface | DeclKind::Library
            ) && decl.base_contracts().iter().any(|b| b == name)
            {
                count += 1;
            }
        }
    }
    count
}
