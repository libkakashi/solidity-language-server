use solidity_language_server::utils::{LineIndex, is_valid_solidity_identifier};

#[test]
fn test_byte_offset_to_position_unix_newlines() {
    let source = "line1\nline2\nline3\n";
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 0),
        (0, 0)
    ); // 'l' in line1
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 5),
        (0, 5)
    ); // '\n'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 6),
        (1, 0)
    ); // 'l' in line2
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 11),
        (1, 5)
    ); // '\n'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 12),
        (2, 0)
    ); // 'l' in line3
}

#[test]
fn test_byte_offset_to_position_windows_newlines() {
    let source = "line1\r\nline2\r\nline3\r\n";
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 0),
        (0, 0)
    );
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 5),
        (0, 5)
    );
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 7),
        (1, 0)
    ); // skips \r\n
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 12),
        (1, 5)
    );
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 14),
        (2, 0)
    );
}

#[test]
fn test_byte_offset_to_position_no_newlines() {
    let source = "justoneline";
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 0),
        (0, 0)
    );
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 5),
        (0, 5)
    );
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 11),
        (0, 11)
    );
}

#[test]
fn test_byte_offset_to_position_offset_out_of_bounds() {
    let source = "short\nfile";
    let offset = source.len() + 10;
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, offset),
        (1, 4)
    );
}

#[test]
fn test_byte_offset_to_position_empty_source() {
    let source = "";
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 0),
        (0, 0)
    );
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 10),
        (0, 0)
    );
}

#[test]
fn test_position_to_byte_offset_basic() {
    let source = "line1\nline2\nline3\n";
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 0),
        0
    ); // 'l'
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 5),
        5
    ); // '\n'
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 1, 0),
        6
    ); // 'l' in line2
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 1, 3),
        9
    ); // 'e' in line2
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 2, 0),
        12
    ); // 'l' in line3
}

#[test]
fn test_position_to_byte_offset_out_of_bounds() {
    let source = "line1\nline2\n";
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 10, 10),
        source.len()
    );
}

#[test]
fn test_position_to_byte_offset_empty() {
    let source = "";
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 0),
        0
    );
}

#[test]
fn test_is_valid_solidity_identifier() {
    assert!(is_valid_solidity_identifier("validName"));
    assert!(is_valid_solidity_identifier("_valid"));
    assert!(is_valid_solidity_identifier("a"));
    assert!(is_valid_solidity_identifier("name123"));
    assert!(is_valid_solidity_identifier("_"));
    assert!(is_valid_solidity_identifier("a_b_c"));

    assert!(!is_valid_solidity_identifier(""));
    assert!(!is_valid_solidity_identifier("123invalid"));
    assert!(!is_valid_solidity_identifier("invalid-name"));
    assert!(!is_valid_solidity_identifier("invalid name"));
    assert!(!is_valid_solidity_identifier("invalid.name"));
}

// ---------------------------------------------------------------------------
// UTF-16 encoding-aware tests (default encoding = UTF-16)
//
// Characters used:
//   '█' = U+2588  -> 3 bytes UTF-8, 1 UTF-16 code unit
//   '░' = U+2591  -> 3 bytes UTF-8, 1 UTF-16 code unit
//   '😀' = U+1F600 -> 4 bytes UTF-8, 2 UTF-16 code units (surrogate pair)
//   'é' = U+00E9  -> 2 bytes UTF-8, 1 UTF-16 code unit
// ---------------------------------------------------------------------------

#[test]
fn test_byte_offset_to_position_utf16_bmp_chars() {
    // "a█b" — 'a' is 1 byte, '█' is 3 bytes (U+2588), 'b' is 1 byte
    // Total: 5 bytes
    // UTF-16 columns: a=0, █=1, b=2
    let source = "a█b";
    assert_eq!(source.len(), 5);

    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 0),
        (0, 0)
    ); // before 'a'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 1),
        (0, 1)
    ); // before '█' (byte 1)
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 4),
        (0, 2)
    ); // before 'b' (byte 4)
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 5),
        (0, 3)
    ); // end
}

#[test]
fn test_byte_offset_to_position_utf16_surrogate_pair() {
    // "a😀b" — 'a' is 1 byte, '😀' is 4 bytes (U+1F600), 'b' is 1 byte
    // Total: 6 bytes
    // UTF-16: a=col 0, 😀=col 1 (takes 2 UTF-16 code units), b=col 3
    let source = "a😀b";
    assert_eq!(source.len(), 6);

    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 0),
        (0, 0)
    ); // before 'a'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 1),
        (0, 1)
    ); // before '😀'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 5),
        (0, 3)
    ); // before 'b' (col 1+2=3)
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 6),
        (0, 4)
    ); // end
}

#[test]
fn test_byte_offset_to_position_utf16_mixed_multibyte() {
    // "é█😀x" — é=2B, █=3B, 😀=4B, x=1B -> 10 bytes total
    // UTF-16 columns: é=0(1 unit), █=1(1 unit), 😀=2(2 units), x=4
    let source = "é█😀x";
    assert_eq!(source.len(), 10);

    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 0),
        (0, 0)
    ); // before 'é'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 2),
        (0, 1)
    ); // before '█'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 5),
        (0, 2)
    ); // before '😀'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 9),
        (0, 4)
    ); // before 'x'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 10),
        (0, 5)
    ); // end
}

#[test]
fn test_byte_offset_to_position_utf16_multiline() {
    // Line 0: "█░\n" — 3+3+1 = 7 bytes; UTF-16 cols: █=0, ░=1
    // Line 1: "a😀\n" — 1+4+1 = 6 bytes; UTF-16 cols: a=0, 😀=1
    // Line 2: "z"
    let source = "█░\na😀\nz";
    assert_eq!(source.len(), 14);

    // Line 0
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 0),
        (0, 0)
    ); // '█'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 3),
        (0, 1)
    ); // '░'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 6),
        (0, 2)
    ); // '\n'
    // Line 1
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 7),
        (1, 0)
    ); // 'a'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 8),
        (1, 1)
    ); // '😀'
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 12),
        (1, 3)
    ); // '\n'
    // Line 2
    assert_eq!(
        LineIndex::new(source).byte_offset_to_position(source, 13),
        (2, 0)
    ); // 'z'
}

#[test]
fn test_position_to_byte_offset_utf16_bmp_chars() {
    let source = "a█b";
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 0),
        0
    ); // 'a'
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 1),
        1
    ); // '█' starts at byte 1
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 2),
        4
    ); // 'b' starts at byte 4
}

#[test]
fn test_position_to_byte_offset_utf16_surrogate_pair() {
    // "a😀b" — UTF-16 cols: a=0, 😀=1(2 units), b=3
    let source = "a😀b";
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 0),
        0
    ); // 'a'
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 1),
        1
    ); // '😀' at byte 1
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 3),
        5
    ); // 'b' at byte 5
}

#[test]
fn test_position_to_byte_offset_utf16_multiline() {
    let source = "█░\na😀\nz";
    // Line 0: █=col0, ░=col1
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 0),
        0
    );
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 0, 1),
        3
    );
    // Line 1: a=col0, 😀=col1, end=col3
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 1, 0),
        7
    );
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 1, 1),
        8
    );
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 1, 3),
        12
    );
    // Line 2: z=col0
    assert_eq!(
        LineIndex::new(source).position_to_byte_offset(source, 2, 0),
        13
    );
}

#[test]
fn test_roundtrip_utf16_byte_to_pos_to_byte() {
    // Verify byte_offset -> position -> byte_offset roundtrips for every char boundary
    let source = "ab😀é█cd\nef";
    let char_boundaries: Vec<usize> = source.char_indices().map(|(i, _)| i).collect();
    for &byte_off in &char_boundaries {
        let (line, col) = LineIndex::new(source).byte_offset_to_position(source, byte_off);
        let recovered = LineIndex::new(source).position_to_byte_offset(source, line, col);
        assert_eq!(
            recovered, byte_off,
            "roundtrip failed for byte_off={byte_off}: pos=({line},{col}) -> {recovered}"
        );
    }
}
