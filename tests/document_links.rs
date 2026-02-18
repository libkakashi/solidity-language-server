use std::path::PathBuf;

use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::links::document_links;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::Url;

fn setup_on_disk(files: &[(&str, &str)]) -> (SymbolTable, Vec<PathBuf>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    for (name, source) in files {
        let path = tmp.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, source).unwrap();
        paths.push(path);
    }

    // Index all files
    for (i, (_, source)) in files.iter().enumerate() {
        st.index_file(&paths[i], source, &mut parser);
    }
    for (i, (_, _source)) in files.iter().enumerate() {
        st.resolve_file_references(&paths[i], &mut parser);
    }

    (st, paths, tmp)
}

// =========================================================================
// 1. Single import link
// =========================================================================

#[test]
fn single_import_creates_one_link() {
    let helper_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Helper {
    function help() public pure returns (uint256) {
        return 1;
    }
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Helper.sol";

contract Main {
    Helper h;
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Helper.sol", helper_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(
        links.len(),
        1,
        "Expected exactly 1 document link for a single import"
    );
}

// =========================================================================
// 2. Multiple imports each get their own link
// =========================================================================

#[test]
fn multiple_imports_create_multiple_links() {
    let a_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {}
"#;
    let b_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract B {}
"#;
    let c_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract C {}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./A.sol";
import "./B.sol";
import "./C.sol";

contract Main is A, B, C {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[
        ("A.sol", a_src),
        ("B.sol", b_src),
        ("C.sol", c_src),
        ("Main.sol", main_src),
    ]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[3], main_src, &line_index);
    assert_eq!(links.len(), 3, "Expected 3 document links for 3 imports");
}

// =========================================================================
// 3. Named import link
// =========================================================================

#[test]
fn named_import_creates_link() {
    let foo_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    function bar() public pure returns (uint256) {
        return 42;
    }
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Foo} from "./Foo.sol";

contract Main {
    Foo f;
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Foo.sol", foo_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(links.len(), 1, "Named import should create exactly 1 link");
}

// =========================================================================
// 4. No imports returns empty links
// =========================================================================

#[test]
fn no_imports_returns_empty_links() {
    let src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Standalone {
    uint256 public value;

    function set(uint256 v) public {
        value = v;
    }
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Standalone.sol", src)]);
    let line_index = LineIndex::new(src);

    let links = document_links(&st, &paths[0], src, &line_index);
    assert!(
        links.is_empty(),
        "Contract with no imports should produce no links"
    );
}

// =========================================================================
// 5. Link target is correct URI
// =========================================================================

#[test]
fn link_target_is_correct_uri() {
    let helper_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Helper {}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Helper.sol";

contract Main {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Helper.sol", helper_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(links.len(), 1);

    let expected_uri = Url::from_file_path(&paths[0]).unwrap();
    assert_eq!(
        links[0].target.as_ref().unwrap(),
        &expected_uri,
        "Link target URI should point to the resolved Helper.sol file"
    );
}

// =========================================================================
// 6. Link range covers path string (inside quotes)
// =========================================================================

#[test]
fn link_range_covers_path_string() {
    let helper_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Helper {}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Helper.sol";

contract Main {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Helper.sol", helper_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(links.len(), 1);

    let range = links[0].range;
    // The import is on line 3: import "./Helper.sol";
    assert_eq!(range.start.line, 3, "Link range should start on line 3");

    // Extract the text covered by the range
    let lines: Vec<&str> = main_src.lines().collect();
    let line_text = lines[range.start.line as usize];
    let start_col = range.start.character as usize;
    let end_col = range.end.character as usize;
    let covered_text = &line_text[start_col..end_col];

    assert_eq!(
        covered_text, "./Helper.sol",
        "Link range should cover the import path inside the quotes, not the quotes themselves"
    );
}

// =========================================================================
// 7. Link tooltip shows the source path
// =========================================================================

#[test]
fn link_tooltip_shows_source_path() {
    let helper_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Helper {}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Helper.sol";

contract Main {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Helper.sol", helper_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(links.len(), 1);

    let tooltip = links[0]
        .tooltip
        .as_ref()
        .expect("Link should have a tooltip");
    assert_eq!(
        tooltip, "./Helper.sol",
        "Tooltip should show the original source path from the import statement"
    );
}

// =========================================================================
// 8. Links are sorted by position (line number)
// =========================================================================

#[test]
fn links_sorted_by_position() {
    let a_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {}
"#;
    let b_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract B {}
"#;
    let c_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract C {}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./C.sol";
import "./A.sol";
import "./B.sol";

contract Main is A, B, C {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[
        ("A.sol", a_src),
        ("B.sol", b_src),
        ("C.sol", c_src),
        ("Main.sol", main_src),
    ]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[3], main_src, &line_index);
    assert_eq!(links.len(), 3);

    // Links should be sorted by line number
    for i in 1..links.len() {
        assert!(
            links[i - 1].range.start.line <= links[i].range.start.line,
            "Links should be sorted by line number: line {} should come before line {}",
            links[i - 1].range.start.line,
            links[i].range.start.line
        );
    }

    // Verify the specific order: C.sol on line 3, A.sol on line 4, B.sol on line 5
    assert_eq!(links[0].tooltip.as_deref(), Some("./C.sol"));
    assert_eq!(links[1].tooltip.as_deref(), Some("./A.sol"));
    assert_eq!(links[2].tooltip.as_deref(), Some("./B.sol"));
}

// =========================================================================
// 9. Unresolved import does not create a link
// =========================================================================

#[test]
fn unresolved_import_no_link() {
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./NonExistent.sol";

contract Main {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[0], main_src, &line_index);
    assert!(
        links.is_empty(),
        "Import to a non-existent file should not produce a document link"
    );
}

// =========================================================================
// 10. Subdirectory import resolves correctly
// =========================================================================

#[test]
fn subdirectory_import_resolves_correctly() {
    let sub_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract SubFile {
    function subFunc() public pure returns (uint256) {
        return 99;
    }
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./sub/File.sol";

contract Main {
    SubFile s;
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("sub/File.sol", sub_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(links.len(), 1, "Subdirectory import should create a link");

    let expected_uri = Url::from_file_path(&paths[0]).unwrap();
    assert_eq!(
        links[0].target.as_ref().unwrap(),
        &expected_uri,
        "Link target should point to the file in the subdirectory"
    );

    assert_eq!(
        links[0].tooltip.as_deref(),
        Some("./sub/File.sol"),
        "Tooltip should show the relative subdirectory path"
    );
}

// =========================================================================
// 11. Multiple files with imports from different files
// =========================================================================

#[test]
fn multiple_files_with_imports() {
    let types_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

struct Token {
    address addr;
    uint256 amount;
}
"#;
    let utils_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library MathUtils {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Types.sol";
import "./Utils.sol";

contract Main {
    Token public t;
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[
        ("Types.sol", types_src),
        ("Utils.sol", utils_src),
        ("Main.sol", main_src),
    ]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[2], main_src, &line_index);
    assert_eq!(
        links.len(),
        2,
        "Should create links for both imports from different files"
    );

    let types_uri = Url::from_file_path(&paths[0]).unwrap();
    let utils_uri = Url::from_file_path(&paths[1]).unwrap();

    assert_eq!(links[0].target.as_ref().unwrap(), &types_uri);
    assert_eq!(links[1].target.as_ref().unwrap(), &utils_uri);
}

// =========================================================================
// 12. Aliased import creates link
// =========================================================================

#[test]
fn aliased_import_creates_link() {
    let lib_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library Lib {
    function compute(uint256 x) internal pure returns (uint256) {
        return x * 2;
    }
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Lib.sol" as LibAlias;

contract Main {
    function test() public pure returns (uint256) {
        return LibAlias.Lib.compute(5);
    }
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Lib.sol", lib_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(
        links.len(),
        1,
        "Aliased import should create exactly 1 link"
    );

    let expected_uri = Url::from_file_path(&paths[0]).unwrap();
    assert_eq!(
        links[0].target.as_ref().unwrap(),
        &expected_uri,
        "Aliased import link target should point to the resolved file"
    );
}

// =========================================================================
// 13. Double quotes vs single quotes
// =========================================================================

#[test]
fn double_quotes_import_creates_link() {
    let helper_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Helper {}
"#;
    // Double quotes
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Helper.sol";

contract Main {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Helper.sol", helper_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(links.len(), 1, "Double-quoted import should create a link");

    // Verify range does not include the quotes
    let range = links[0].range;
    let lines: Vec<&str> = main_src.lines().collect();
    let line_text = lines[range.start.line as usize];
    let covered = &line_text[range.start.character as usize..range.end.character as usize];
    assert_eq!(
        covered, "./Helper.sol",
        "Range should cover just the path, not the quotes"
    );
}

#[test]
fn single_quotes_import_creates_link() {
    let helper_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Helper {}
"#;
    // Single quotes
    let main_src = "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.29;\n\nimport './Helper.sol';\n\ncontract Main {}\n";
    let (st, paths, _tmp) = setup_on_disk(&[("Helper.sol", helper_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(links.len(), 1, "Single-quoted import should create a link");

    // Verify range does not include the quotes
    let range = links[0].range;
    let lines: Vec<&str> = main_src.lines().collect();
    let line_text = lines[range.start.line as usize];
    let covered = &line_text[range.start.character as usize..range.end.character as usize];
    assert_eq!(
        covered, "./Helper.sol",
        "Range should cover just the path, not the single quotes"
    );
}

// =========================================================================
// 14. Glob import link (wildcard/star import)
// =========================================================================

#[test]
fn glob_import_creates_link() {
    let base_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    uint256 public baseValue;
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Base.sol";

contract Child is Base {
    function getBase() public view returns (uint256) {
        return baseValue;
    }
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Base.sol", base_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(links.len(), 1, "Glob/wildcard import should create a link");

    let expected_uri = Url::from_file_path(&paths[0]).unwrap();
    assert_eq!(links[0].target.as_ref().unwrap(), &expected_uri);
}

// =========================================================================
// 15. Import with multiple named symbols creates single link
// =========================================================================

#[test]
fn import_with_multiple_named_symbols_creates_single_link() {
    let types_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

struct TypeA {
    uint256 x;
}

struct TypeB {
    uint256 y;
}

enum Status {
    Active,
    Inactive
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {TypeA, TypeB, Status} from "./Types.sol";

contract Main {
    TypeA public a;
    TypeB public b;
    Status public s;
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Types.sol", types_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(
        links.len(),
        1,
        "Import with multiple named symbols should create a single link to the file"
    );

    let expected_uri = Url::from_file_path(&paths[0]).unwrap();
    assert_eq!(links[0].target.as_ref().unwrap(), &expected_uri);
}

// =========================================================================
// 16. Unindexed file returns empty links
// =========================================================================

#[test]
fn unindexed_file_returns_empty_links() {
    let src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ghost {}
"#;
    let tmp = tempfile::tempdir().unwrap();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let st = SymbolTable::new(resolver);
    let unknown_path = tmp.path().join("Unknown.sol");
    let line_index = LineIndex::new(src);

    let links = document_links(&st, &unknown_path, src, &line_index);
    assert!(
        links.is_empty(),
        "A file not in the symbol table should return no links"
    );
}

// =========================================================================
// 17. Link range start and end lines match the import line
// =========================================================================

#[test]
fn link_range_on_correct_line_for_each_import() {
    let a_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Alpha {}
"#;
    let b_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Beta {}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Alpha.sol";
import "./Beta.sol";

contract Main is Alpha, Beta {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[
        ("Alpha.sol", a_src),
        ("Beta.sol", b_src),
        ("Main.sol", main_src),
    ]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[2], main_src, &line_index);
    assert_eq!(links.len(), 2);

    // First import on line 3
    assert_eq!(
        links[0].range.start.line, 3,
        "First import link should be on line 3"
    );
    assert_eq!(
        links[0].range.end.line, 3,
        "First import link end should be on line 3"
    );

    // Second import on line 4
    assert_eq!(
        links[1].range.start.line, 4,
        "Second import link should be on line 4"
    );
    assert_eq!(
        links[1].range.end.line, 4,
        "Second import link end should be on line 4"
    );
}

// =========================================================================
// 18. Named import with alias creates link
// =========================================================================

#[test]
fn named_import_with_alias_creates_link() {
    let token_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public supply;
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Token as MyToken} from "./Token.sol";

contract Main {
    MyToken public t;
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Token.sol", token_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(
        links.len(),
        1,
        "Named import with alias should create exactly 1 link"
    );

    let expected_uri = Url::from_file_path(&paths[0]).unwrap();
    assert_eq!(links[0].target.as_ref().unwrap(), &expected_uri);
    assert_eq!(links[0].tooltip.as_deref(), Some("./Token.sol"));
}

// =========================================================================
// 19. Deeply nested subdirectory import
// =========================================================================

#[test]
fn deeply_nested_subdirectory_import() {
    let deep_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract DeepContract {
    function deepFunc() public pure returns (uint256) {
        return 7;
    }
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./a/b/c/Deep.sol";

contract Main {
    DeepContract d;
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("a/b/c/Deep.sol", deep_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(
        links.len(),
        1,
        "Deeply nested subdirectory import should create a link"
    );

    let expected_uri = Url::from_file_path(&paths[0]).unwrap();
    assert_eq!(links[0].target.as_ref().unwrap(), &expected_uri);
    assert_eq!(links[0].tooltip.as_deref(), Some("./a/b/c/Deep.sol"));
}

// =========================================================================
// 20. Multiple unresolved imports produce no links
// =========================================================================

#[test]
fn multiple_unresolved_imports_produce_no_links() {
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Missing1.sol";
import "./Missing2.sol";
import "./Missing3.sol";

contract Main {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[0], main_src, &line_index);
    assert!(
        links.is_empty(),
        "Multiple imports to non-existent files should produce no links"
    );
}

// =========================================================================
// 21. Mix of resolved and unresolved imports
// =========================================================================

#[test]
fn mix_of_resolved_and_unresolved_imports() {
    let helper_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Helper {}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Helper.sol";
import "./NonExistent.sol";

contract Main {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Helper.sol", helper_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(
        links.len(),
        1,
        "Only the resolved import should produce a link; unresolved imports are skipped"
    );

    let expected_uri = Url::from_file_path(&paths[0]).unwrap();
    assert_eq!(links[0].target.as_ref().unwrap(), &expected_uri);
}

// =========================================================================
// 22. Link data field is None
// =========================================================================

#[test]
fn link_data_field_is_none() {
    let helper_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Helper {}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "./Helper.sol";

contract Main {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Helper.sol", helper_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(links.len(), 1);
    assert!(
        links[0].data.is_none(),
        "The data field of the document link should be None"
    );
}

// =========================================================================
// 23. Import from sibling directory
// =========================================================================

#[test]
fn import_from_sibling_directory() {
    let lib_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library Util {
    function double(uint256 x) internal pure returns (uint256) {
        return x * 2;
    }
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "../lib/Util.sol";

contract Main {}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("lib/Util.sol", lib_src), ("src/Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(
        links.len(),
        1,
        "Import from sibling directory should create a link"
    );

    let expected_uri = Url::from_file_path(&paths[0]).unwrap();
    assert_eq!(links[0].target.as_ref().unwrap(), &expected_uri);
}

// =========================================================================
// 24. Named import with multiple symbols -- range and tooltip
// =========================================================================

#[test]
fn named_import_multiple_symbols_range_and_tooltip() {
    let types_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

struct Coord {
    uint256 x;
    uint256 y;
}

enum Direction {
    Up,
    Down
}
"#;
    let main_src = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Coord, Direction} from "./Types.sol";

contract Main {
    Coord public origin;
    Direction public dir;
}
"#;
    let (st, paths, _tmp) = setup_on_disk(&[("Types.sol", types_src), ("Main.sol", main_src)]);
    let line_index = LineIndex::new(main_src);

    let links = document_links(&st, &paths[1], main_src, &line_index);
    assert_eq!(
        links.len(),
        1,
        "Named import with multiple symbols yields a single link"
    );

    // The range should cover the path "./Types.sol"
    let range = links[0].range;
    let lines: Vec<&str> = main_src.lines().collect();
    let line_text = lines[range.start.line as usize];
    let covered = &line_text[range.start.character as usize..range.end.character as usize];
    assert_eq!(covered, "./Types.sol");

    assert_eq!(links[0].tooltip.as_deref(), Some("./Types.sol"));
}

// =========================================================================
// 25. Empty source file returns empty links
// =========================================================================

#[test]
fn empty_source_file_returns_empty_links() {
    let src = "";
    let (st, paths, _tmp) = setup_on_disk(&[("Empty.sol", src)]);
    let line_index = LineIndex::new(src);

    let links = document_links(&st, &paths[0], src, &line_index);
    assert!(
        links.is_empty(),
        "Empty source file should produce no links"
    );
}
