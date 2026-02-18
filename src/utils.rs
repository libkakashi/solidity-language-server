use std::sync::OnceLock;
use tower_lsp::lsp_types::{Position, PositionEncodingKind, Range};

/// How the LSP client counts column offsets within a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
}

impl PositionEncoding {
    pub const DEFAULT: Self = PositionEncoding::Utf16;

    pub fn negotiate(client_encodings: Option<&[PositionEncodingKind]>) -> Self {
        let Some(encodings) = client_encodings else {
            return Self::DEFAULT;
        };
        if encodings.contains(&PositionEncodingKind::UTF8) {
            PositionEncoding::Utf8
        } else {
            PositionEncoding::Utf16
        }
    }

    pub fn to_encoding_kind(self) -> PositionEncodingKind {
        match self {
            PositionEncoding::Utf8 => PositionEncodingKind::UTF8,
            PositionEncoding::Utf16 => PositionEncodingKind::UTF16,
        }
    }
}

static ENCODING: OnceLock<PositionEncoding> = OnceLock::new();

pub fn set_encoding(enc: PositionEncoding) {
    let _ = ENCODING.set(enc);
}

pub fn encoding() -> PositionEncoding {
    ENCODING.get().copied().unwrap_or(PositionEncoding::DEFAULT)
}

// ---------------------------------------------------------------------------
// Line index — pre-computed line-start offsets for O(log n) conversion (Fix #16)
// ---------------------------------------------------------------------------

/// Pre-computed index of line-start byte offsets for a source file.
/// Build once, then use for all byte<->position conversions on that file.
pub struct LineIndex {
    /// Byte offset of the start of each line. line_starts[0] == 0 always.
    line_starts: Vec<usize>,
}

impl LineIndex {
    /// Build a line index from source text. O(n) once.
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0usize];
        let bytes = source.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\n' {
                line_starts.push(i + 1);
            } else if bytes[i] == b'\r' {
                // \r\n counts as one line ending (Fix #4)
                if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                    line_starts.push(i + 2);
                    i += 1; // skip the \n
                } else {
                    // bare \r also counts as a line ending
                    line_starts.push(i + 1);
                }
            }
            i += 1;
        }
        Self { line_starts }
    }

    /// Return the byte offset of the start of the given line.
    pub fn line_start(&self, line: u32) -> usize {
        let idx = line as usize;
        if idx < self.line_starts.len() {
            self.line_starts[idx]
        } else {
            *self.line_starts.last().unwrap_or(&0)
        }
    }

    /// Convert a byte offset to (line, column). O(log n) via binary search.
    pub fn byte_offset_to_position(&self, source: &str, byte_offset: usize) -> (u32, u32) {
        let byte_offset = byte_offset.min(source.len());
        let line = match self.line_starts.binary_search(&byte_offset) {
            Ok(exact) => exact,
            Err(insert) => insert.saturating_sub(1),
        };
        let line_start = self.line_starts[line];
        let col = compute_column(source, line_start, byte_offset);
        (line as u32, col)
    }

    /// Convert a byte offset to an LSP `Position`.
    pub fn byte_offset_to_lsp_position(&self, source: &str, byte_offset: usize) -> Position {
        let (line, character) = self.byte_offset_to_position(source, byte_offset);
        Position { line, character }
    }

    /// Convert a byte range to an LSP `Range`.
    pub fn byte_range_to_lsp_range(
        &self,
        source: &str,
        start_byte: usize,
        end_byte: usize,
    ) -> Range {
        Range {
            start: self.byte_offset_to_lsp_position(source, start_byte),
            end: self.byte_offset_to_lsp_position(source, end_byte),
        }
    }

    /// Convert an LSP (line, character) position to a byte offset. O(line_length).
    pub fn position_to_byte_offset(&self, source: &str, line: u32, character: u32) -> usize {
        let line = line as usize;
        if line >= self.line_starts.len() {
            return source.len();
        }
        let line_start = self.line_starts[line];
        let line_end = if line + 1 < self.line_starts.len() {
            self.line_starts[line + 1]
        } else {
            source.len()
        };
        // Walk the line to find the byte offset for the given column.
        let enc = encoding();
        let mut col: u32 = 0;
        let line_slice = &source[line_start..line_end];
        for (i, ch) in line_slice.char_indices() {
            if col == character {
                return line_start + i;
            }
            // Don't count line endings as columns
            if ch == '\n' || ch == '\r' {
                return line_start + i;
            }
            col += match enc {
                PositionEncoding::Utf8 => ch.len_utf8() as u32,
                PositionEncoding::Utf16 => ch.len_utf16() as u32,
            };
        }
        // If character is past the end of the line, clamp.
        line_end.min(source.len())
    }
}

/// Compute column offset from line_start to byte_offset.
fn compute_column(source: &str, line_start: usize, byte_offset: usize) -> u32 {
    let enc = encoding();
    match enc {
        PositionEncoding::Utf8 => {
            // UTF-8: column = number of bytes from line start.
            (byte_offset - line_start) as u32
        }
        PositionEncoding::Utf16 => {
            // UTF-16: count UTF-16 code units.
            let segment = &source[line_start..byte_offset];
            segment.chars().map(|c| c.len_utf16() as u32).sum()
        }
    }
}

// ---------------------------------------------------------------------------
// Source cache — avoids redundant fs::read + LineIndex construction (Fix #19)
// ---------------------------------------------------------------------------

use std::path::Path;

use rustc_hash::FxHashMap;

/// Caches `(source, LineIndex)` for files that are not the "current" open file.
/// The current file's source and line-index are passed in at construction and
/// returned without any I/O.
pub struct SourceCache<'a> {
    current_file: &'a Path,
    current_source: &'a str,
    current_line_index: &'a LineIndex,
    cache: FxHashMap<&'a Path, (String, LineIndex)>,
}

impl<'a> SourceCache<'a> {
    pub fn new(
        current_file: &'a Path,
        current_source: &'a str,
        current_line_index: &'a LineIndex,
    ) -> Self {
        Self {
            current_file,
            current_source,
            current_line_index,
            cache: FxHashMap::default(),
        }
    }

    /// Return `(source, LineIndex)` for `path`, reading from disk at most once.
    /// Returns `None` only when the file cannot be read.
    pub fn get(&mut self, path: &'a Path) -> Option<(&str, &LineIndex)> {
        if path == self.current_file {
            return Some((self.current_source, self.current_line_index));
        }
        if !self.cache.contains_key(path) {
            let s = std::fs::read_to_string(path).ok()?;
            let li = LineIndex::new(&s);
            self.cache.insert(path, (s, li));
        }
        self.cache.get(path).map(|(s, li)| (s.as_str(), li))
    }
}

pub fn is_valid_solidity_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
