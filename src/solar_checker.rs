use solar::{
    config::{ImportRemapping, Opts},
    interface::{
        Session, SourceMap, Span,
        diagnostics::{Diag, DiagCtxt, InMemoryEmitter, Level},
    },
    sema::Compiler,
};
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};

use crate::utils;

/// Configuration for solar import resolution.
#[derive(Clone, Default)]
pub struct SolarConfig {
    pub remappings: Vec<ImportRemapping>,
    pub include_paths: Vec<PathBuf>,
    pub base_path: Option<PathBuf>,
}

/// Run solar type checking on a saved file, returning LSP diagnostics.
///
/// This is synchronous — designed to run inside `tokio::task::spawn_blocking`.
/// Wrapped in `catch_unwind` so a solar panic does not crash the LSP.
pub fn check_file(file_path: &Path, config: &SolarConfig) -> Vec<Diagnostic> {
    let config = config.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_file_inner(file_path, &config)
    }));
    match result {
        Ok(diags) => diags,
        Err(_) => {
            tracing::warn!("solar panicked while checking {}", file_path.display());
            vec![]
        }
    }
}

fn check_file_inner(file_path: &Path, config: &SolarConfig) -> Vec<Diagnostic> {
    let (emitter, diag_buffer) = InMemoryEmitter::new();
    let opts = Opts {
        import_remappings: config.remappings.clone(),
        include_paths: config.include_paths.clone(),
        base_path: config.base_path.clone(),
        ..Default::default()
    };
    let sess = Session::builder()
        .dcx(DiagCtxt::new(Box::new(emitter)))
        .opts(opts)
        .build();
    let mut compiler = Compiler::new(sess);

    let parse_ok = compiler.enter_mut(|compiler| -> solar::interface::Result<()> {
        let mut pcx = compiler.parse();
        pcx.load_file(file_path)?;
        pcx.parse();
        Ok(())
    });

    if parse_ok.is_ok() {
        let _ = compiler.enter_mut(|compiler| -> solar::interface::Result<()> {
            let _ = compiler.lower_asts();
            let _ = compiler.analysis();
            Ok(())
        });
    }

    let source_map = compiler.sess().source_map();
    let buffer = diag_buffer.read();
    buffer
        .iter()
        .filter_map(|diag| solar_diag_to_lsp(diag, source_map, file_path))
        .collect()
}

fn solar_diag_to_lsp(
    diag: &Diag,
    source_map: &SourceMap,
    target_file: &Path,
) -> Option<Diagnostic> {
    let span = diag.span.primary_span()?;
    let lo = source_map.lookup_char_pos(span.lo());

    // Only include diagnostics that originate from the target file.
    if lo.file.name != *target_file {
        return None;
    }

    let range = span_to_range(source_map, span);
    let code: Option<NumberOrString> = diag
        .code
        .as_ref()
        .map(|id| NumberOrString::String(id.as_string()));
    Some(Diagnostic {
        range,
        severity: Some(map_severity(diag.level())),
        code,
        source: Some("solar".into()),
        message: diag.label().into_owned(),
        ..Default::default()
    })
}

/// Convert a solar span to an LSP range, respecting position encoding. (Fix #26)
fn span_to_range(source_map: &SourceMap, span: Span) -> Range {
    let lo = source_map.lookup_char_pos(span.lo());
    let hi = source_map.lookup_char_pos(span.hi());

    // Solar gives us 1-based line numbers and byte-offset columns.
    // We need to apply position encoding negotiation for correct columns.
    let lo_line = lo.data.line.saturating_sub(1) as u32;
    let hi_line = hi.data.line.saturating_sub(1) as u32;

    let enc = utils::encoding();
    let (lo_col, hi_col) = match enc {
        utils::PositionEncoding::Utf8 => {
            // Solar's col.0 is a byte offset within the line, which matches UTF-8.
            (lo.data.col.0 as u32, hi.data.col.0 as u32)
        }
        utils::PositionEncoding::Utf16 => {
            // Use the source already loaded by solar (avoids redundant disk read).
            let source = &*lo.file.src;
            let line_index = utils::LineIndex::new(source);
            // Compute byte offset of lo position, then convert.
            let lo_byte = line_index.position_to_byte_offset(source, lo_line, lo.data.col.0 as u32);
            let hi_byte = line_index.position_to_byte_offset(source, hi_line, hi.data.col.0 as u32);
            let (_, lo_col) = line_index.byte_offset_to_position(source, lo_byte);
            let (_, hi_col) = line_index.byte_offset_to_position(source, hi_byte);
            (lo_col, hi_col)
        }
    };

    Range {
        start: Position {
            line: lo_line,
            character: lo_col,
        },
        end: Position {
            line: hi_line,
            character: hi_col,
        },
    }
}

fn map_severity(level: Level) -> DiagnosticSeverity {
    use Level::*;
    match level {
        Error | Fatal | Bug => DiagnosticSeverity::ERROR,
        Warning => DiagnosticSeverity::WARNING,
        Note | OnceNote => DiagnosticSeverity::INFORMATION,
        Help | OnceHelp => DiagnosticSeverity::HINT,
        _ => DiagnosticSeverity::INFORMATION,
    }
}
