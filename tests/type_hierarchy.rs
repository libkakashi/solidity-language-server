use std::path::PathBuf;

use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::type_hierarchy;
use solidity_language_server::utils::LineIndex;
use tower_lsp::lsp_types::{Position, SymbolKind};

/// Set up a single-file test using an on-disk temp directory.
///
/// `type_hierarchy` internally calls `read_file_source` which may fall back to
/// `std::fs::read_to_string`, so the file must exist on disk.
fn setup_on_disk(source: &str) -> (SymbolTable, PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.sol");
    std::fs::write(&path, source).unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path, tmp)
}

/// Convert a byte offset inside `source` to an LSP `Position`.
fn offset_to_position(source: &str, offset: usize) -> Position {
    let line = source[..offset].matches('\n').count() as u32;
    let col = (offset - source[..offset].rfind('\n').map_or(0, |p| p + 1)) as u32;
    Position::new(line, col)
}

// ---------------------------------------------------------------------------
// 1. Prepare on contract
// ---------------------------------------------------------------------------

#[test]
fn prepare_on_contract() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    uint256 public balance;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Vault").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li);

    assert!(items.is_some(), "prepare should return Some for a contract");
    let items = items.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "Vault");
}

// ---------------------------------------------------------------------------
// 2. Prepare on interface
// ---------------------------------------------------------------------------

#[test]
fn prepare_on_interface() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function totalSupply() external view returns (uint256);
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("IERC20").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li);

    assert!(
        items.is_some(),
        "prepare should return Some for an interface"
    );
    let items = items.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "IERC20");
}

// ---------------------------------------------------------------------------
// 3. Prepare on library
// ---------------------------------------------------------------------------

#[test]
fn prepare_on_library() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("MathLib").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li);

    assert!(items.is_some(), "prepare should return Some for a library");
    let items = items.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "MathLib");
}

// ---------------------------------------------------------------------------
// 4. Prepare on struct
// ---------------------------------------------------------------------------

#[test]
fn prepare_on_struct() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct User {
        address wallet;
        uint256 balance;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("User").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li);

    assert!(items.is_some(), "prepare should return Some for a struct");
    let items = items.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "User");
}

// ---------------------------------------------------------------------------
// 5. Prepare on function returns None (not a type)
// ---------------------------------------------------------------------------

#[test]
fn prepare_on_function_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    function deposit(uint256 amount) public pure returns (bool) {
        return amount > 0;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("deposit").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li);

    assert!(
        items.is_none(),
        "prepare should return None for a function name"
    );
}

// ---------------------------------------------------------------------------
// 6. Prepare on variable returns None
// ---------------------------------------------------------------------------

#[test]
fn prepare_on_variable_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public totalSupply;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("totalSupply").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li);

    assert!(
        items.is_none(),
        "prepare should return None for a state variable"
    );
}

// ---------------------------------------------------------------------------
// 7. Supertypes - single base
// ---------------------------------------------------------------------------

#[test]
fn supertypes_single_base() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    uint256 public x;
}

contract Child is Base {
    uint256 public y;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Child").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let child_item = &items[0];

    let supers = type_hierarchy::supertypes(&st, child_item);
    assert_eq!(supers.len(), 1, "Child should have exactly one supertype");
    assert_eq!(supers[0].name, "Base");
}

// ---------------------------------------------------------------------------
// 8. Supertypes - multiple bases
// ---------------------------------------------------------------------------

#[test]
fn supertypes_multiple_bases() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 public a;
}

contract B {
    uint256 public b;
}

contract C {
    uint256 public c;
}

contract D is A, B, C {
    uint256 public d;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(
        source,
        source.find("contract D").unwrap() + "contract ".len(),
    );
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let d_item = &items[0];

    let supers = type_hierarchy::supertypes(&st, d_item);
    assert_eq!(supers.len(), 3, "D should have three supertypes");

    let names: Vec<&str> = supers.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"A"), "Supertypes should contain A");
    assert!(names.contains(&"B"), "Supertypes should contain B");
    assert!(names.contains(&"C"), "Supertypes should contain C");
}

// ---------------------------------------------------------------------------
// 9. Supertypes - no bases returns empty
// ---------------------------------------------------------------------------

#[test]
fn supertypes_no_bases() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Standalone {
    uint256 public x;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Standalone").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let item = &items[0];

    let supers = type_hierarchy::supertypes(&st, item);
    assert!(
        supers.is_empty(),
        "Contract with no inheritance should have no supertypes"
    );
}

// ---------------------------------------------------------------------------
// 10. Subtypes - single derived
// ---------------------------------------------------------------------------

#[test]
fn subtypes_single_derived() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    uint256 public x;
}

contract Derived is Base {
    uint256 public y;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Base").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let base_item = &items[0];

    let subs = type_hierarchy::subtypes(&st, base_item);
    assert_eq!(subs.len(), 1, "Base should have exactly one subtype");
    assert_eq!(subs[0].name, "Derived");
}

// ---------------------------------------------------------------------------
// 11. Subtypes - multiple derived
// ---------------------------------------------------------------------------

#[test]
fn subtypes_multiple_derived() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    uint256 public x;
}

contract DerivedA is Base {
    uint256 public a;
}

contract DerivedB is Base {
    uint256 public b;
}

contract DerivedC is Base {
    uint256 public c;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Base").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let base_item = &items[0];

    let subs = type_hierarchy::subtypes(&st, base_item);
    assert_eq!(subs.len(), 3, "Base should have three subtypes");

    let names: Vec<&str> = subs.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"DerivedA"),
        "Subtypes should contain DerivedA"
    );
    assert!(
        names.contains(&"DerivedB"),
        "Subtypes should contain DerivedB"
    );
    assert!(
        names.contains(&"DerivedC"),
        "Subtypes should contain DerivedC"
    );
}

// ---------------------------------------------------------------------------
// 12. Subtypes - no derived returns empty
// ---------------------------------------------------------------------------

#[test]
fn subtypes_no_derived() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Leaf {
    uint256 public x;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Leaf").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let leaf_item = &items[0];

    let subs = type_hierarchy::subtypes(&st, leaf_item);
    assert!(
        subs.is_empty(),
        "Contract with no derived contracts should have no subtypes"
    );
}

// ---------------------------------------------------------------------------
// 13. Diamond inheritance
// ---------------------------------------------------------------------------

#[test]
fn diamond_inheritance() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    uint256 public a;
}

contract B is A {
    uint256 public b;
}

contract C is A {
    uint256 public c;
}

contract D is B, C {
    uint256 public d;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // D's supertypes should be B and C (direct parents only)
    let pos_d = offset_to_position(
        source,
        source.find("contract D").unwrap() + "contract ".len(),
    );
    let d_items = type_hierarchy::prepare(&st, &path, source, pos_d, &li).unwrap();
    let d_item = &d_items[0];

    let d_supers = type_hierarchy::supertypes(&st, d_item);
    assert_eq!(d_supers.len(), 2, "D should have two direct supertypes");
    let d_super_names: Vec<&str> = d_supers.iter().map(|s| s.name.as_str()).collect();
    assert!(
        d_super_names.contains(&"B"),
        "D supertypes should contain B"
    );
    assert!(
        d_super_names.contains(&"C"),
        "D supertypes should contain C"
    );

    // A's subtypes should be B and C (direct children only)
    let pos_a = offset_to_position(
        source,
        source.find("contract A").unwrap() + "contract ".len(),
    );
    let a_items = type_hierarchy::prepare(&st, &path, source, pos_a, &li).unwrap();
    let a_item = &a_items[0];

    let a_subs = type_hierarchy::subtypes(&st, a_item);
    let a_sub_names: Vec<&str> = a_subs.iter().map(|s| s.name.as_str()).collect();
    assert!(a_sub_names.contains(&"B"), "A subtypes should contain B");
    assert!(a_sub_names.contains(&"C"), "A subtypes should contain C");
    // D does NOT directly inherit A, so it should not appear
    assert!(
        !a_sub_names.contains(&"D"),
        "A subtypes should not contain D (indirect)"
    );

    // B's supertypes should be A
    let pos_b = offset_to_position(
        source,
        source.find("contract B").unwrap() + "contract ".len(),
    );
    let b_items = type_hierarchy::prepare(&st, &path, source, pos_b, &li).unwrap();
    let b_item = &b_items[0];

    let b_supers = type_hierarchy::supertypes(&st, b_item);
    assert_eq!(b_supers.len(), 1, "B should have one supertype");
    assert_eq!(b_supers[0].name, "A");
}

// ---------------------------------------------------------------------------
// 14. Interface hierarchy - interface extending another interface
// ---------------------------------------------------------------------------

#[test]
fn interface_hierarchy() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IBase {
    function baseFunc() external;
}

interface IExtended is IBase {
    function extFunc() external;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // IExtended supertypes should include IBase
    let pos = offset_to_position(source, source.find("IExtended").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let ext_item = &items[0];

    let supers = type_hierarchy::supertypes(&st, ext_item);
    assert_eq!(supers.len(), 1, "IExtended should have one supertype");
    assert_eq!(supers[0].name, "IBase");

    // IBase subtypes should include IExtended
    let pos_base = offset_to_position(source, source.find("IBase").unwrap());
    let base_items = type_hierarchy::prepare(&st, &path, source, pos_base, &li).unwrap();
    let base_item = &base_items[0];

    let subs = type_hierarchy::subtypes(&st, base_item);
    assert_eq!(subs.len(), 1, "IBase should have one subtype");
    assert_eq!(subs[0].name, "IExtended");
}

// ---------------------------------------------------------------------------
// 15. Prepare returns correct name
// ---------------------------------------------------------------------------

#[test]
fn prepare_returns_correct_name() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract MySpecialContract {
    uint256 public x;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("MySpecialContract").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();

    assert_eq!(
        items[0].name, "MySpecialContract",
        "Item name should match the contract name exactly"
    );
}

// ---------------------------------------------------------------------------
// 16. Prepare returns correct kind
// ---------------------------------------------------------------------------

#[test]
fn prepare_returns_correct_kind_for_contract() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public supply;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Token").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();

    assert_eq!(
        items[0].kind,
        SymbolKind::CLASS,
        "Contract should have SymbolKind::CLASS"
    );
}

#[test]
fn prepare_returns_correct_kind_for_interface() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC721 {
    function ownerOf(uint256 tokenId) external view returns (address);
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("IERC721").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();

    assert_eq!(
        items[0].kind,
        SymbolKind::INTERFACE,
        "Interface should have SymbolKind::INTERFACE"
    );
}

#[test]
fn prepare_returns_correct_kind_for_library() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("SafeMath").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();

    assert_eq!(
        items[0].kind,
        SymbolKind::NAMESPACE,
        "Library should have SymbolKind::NAMESPACE"
    );
}

#[test]
fn prepare_returns_correct_kind_for_struct() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Point").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();

    assert_eq!(
        items[0].kind,
        SymbolKind::STRUCT,
        "Struct should have SymbolKind::STRUCT"
    );
}

// ---------------------------------------------------------------------------
// 17. Prepare detail shows bases
// ---------------------------------------------------------------------------

#[test]
fn prepare_detail_shows_bases() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract BaseA {
    uint256 public a;
}

contract BaseB {
    uint256 public b;
}

contract Child is BaseA, BaseB {
    uint256 public c;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Child").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let child_item = &items[0];

    let detail = child_item
        .detail
        .as_ref()
        .expect("Child should have detail showing bases");
    assert!(
        detail.contains("BaseA"),
        "Detail should mention BaseA, got: {detail}"
    );
    assert!(
        detail.contains("BaseB"),
        "Detail should mention BaseB, got: {detail}"
    );
    assert!(
        detail.starts_with("is "),
        "Detail should start with 'is ', got: {detail}"
    );
}

#[test]
fn prepare_detail_none_for_no_bases() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Standalone {
    uint256 public x;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Standalone").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let item = &items[0];

    assert!(
        item.detail.is_none(),
        "Contract with no bases should have None detail"
    );
}

// ---------------------------------------------------------------------------
// 18. Deep inheritance chain
// ---------------------------------------------------------------------------

#[test]
fn deep_inheritance_chain() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract GrandParent {
    uint256 public gp;
}

contract Parent is GrandParent {
    uint256 public p;
}

contract Child is Parent {
    uint256 public c;
}

contract GrandChild is Child {
    uint256 public gc;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // GrandChild -> Child (direct supertype)
    let pos_gc = offset_to_position(source, source.find("GrandChild").unwrap());
    let gc_items = type_hierarchy::prepare(&st, &path, source, pos_gc, &li).unwrap();
    let gc_item = &gc_items[0];

    let gc_supers = type_hierarchy::supertypes(&st, gc_item);
    assert_eq!(
        gc_supers.len(),
        1,
        "GrandChild should have one direct supertype"
    );
    assert_eq!(gc_supers[0].name, "Child");

    // Child -> Parent (direct supertype)
    let child_supers = type_hierarchy::supertypes(&st, &gc_supers[0]);
    assert_eq!(
        child_supers.len(),
        1,
        "Child should have one direct supertype"
    );
    assert_eq!(child_supers[0].name, "Parent");

    // Parent -> GrandParent (direct supertype)
    let parent_supers = type_hierarchy::supertypes(&st, &child_supers[0]);
    assert_eq!(
        parent_supers.len(),
        1,
        "Parent should have one direct supertype"
    );
    assert_eq!(parent_supers[0].name, "GrandParent");

    // GrandParent has no supertypes
    let gp_supers = type_hierarchy::supertypes(&st, &parent_supers[0]);
    assert!(
        gp_supers.is_empty(),
        "GrandParent should have no supertypes"
    );

    // Verify subtypes chain going down: GrandParent -> Parent
    let pos_gp = offset_to_position(source, source.find("GrandParent").unwrap());
    let gp_items = type_hierarchy::prepare(&st, &path, source, pos_gp, &li).unwrap();
    let gp_item = &gp_items[0];

    let gp_subs = type_hierarchy::subtypes(&st, gp_item);
    assert_eq!(
        gp_subs.len(),
        1,
        "GrandParent should have one direct subtype"
    );
    assert_eq!(gp_subs[0].name, "Parent");
}

// ---------------------------------------------------------------------------
// 19. Abstract contract hierarchy
// ---------------------------------------------------------------------------

#[test]
fn abstract_contract_hierarchy() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

abstract contract AbstractBase {
    function doSomething() public virtual returns (uint256);
}

contract Concrete is AbstractBase {
    function doSomething() public pure override returns (uint256) {
        return 42;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // Prepare on AbstractBase
    let pos_abs = offset_to_position(source, source.find("AbstractBase").unwrap());
    let abs_items = type_hierarchy::prepare(&st, &path, source, pos_abs, &li);
    assert!(
        abs_items.is_some(),
        "prepare should return Some for abstract contract"
    );
    let abs_items = abs_items.unwrap();
    assert_eq!(abs_items[0].name, "AbstractBase");

    // AbstractBase subtypes should include Concrete
    let abs_subs = type_hierarchy::subtypes(&st, &abs_items[0]);
    assert_eq!(abs_subs.len(), 1, "AbstractBase should have one subtype");
    assert_eq!(abs_subs[0].name, "Concrete");

    // Concrete supertypes should include AbstractBase
    let pos_concrete = offset_to_position(source, source.find("Concrete").unwrap());
    let concrete_items = type_hierarchy::prepare(&st, &path, source, pos_concrete, &li).unwrap();
    let concrete_supers = type_hierarchy::supertypes(&st, &concrete_items[0]);
    assert_eq!(
        concrete_supers.len(),
        1,
        "Concrete should have one supertype"
    );
    assert_eq!(concrete_supers[0].name, "AbstractBase");
}

// ---------------------------------------------------------------------------
// 20. Mixed contract/interface - contract implementing interface
// ---------------------------------------------------------------------------

#[test]
fn mixed_contract_interface() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
}

contract Token is IERC20 {
    function transfer(address to, uint256 amount) external pure returns (bool) {
        return true;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // Token supertypes should include IERC20
    let pos_token = offset_to_position(source, source.find("Token").unwrap());
    let token_items = type_hierarchy::prepare(&st, &path, source, pos_token, &li).unwrap();
    let token_item = &token_items[0];

    let supers = type_hierarchy::supertypes(&st, token_item);
    assert_eq!(supers.len(), 1, "Token should have one supertype (IERC20)");
    assert_eq!(supers[0].name, "IERC20");
    assert_eq!(
        supers[0].kind,
        SymbolKind::INTERFACE,
        "IERC20 should have SymbolKind::INTERFACE"
    );

    // IERC20 subtypes should include Token
    let pos_ierc = offset_to_position(source, source.find("IERC20").unwrap());
    let ierc_items = type_hierarchy::prepare(&st, &path, source, pos_ierc, &li).unwrap();
    let ierc_item = &ierc_items[0];

    let subs = type_hierarchy::subtypes(&st, ierc_item);
    assert_eq!(subs.len(), 1, "IERC20 should have one subtype (Token)");
    assert_eq!(subs[0].name, "Token");
    assert_eq!(
        subs[0].kind,
        SymbolKind::CLASS,
        "Token should have SymbolKind::CLASS"
    );
}

// ---------------------------------------------------------------------------
// 21. Prepare on whitespace returns None
// ---------------------------------------------------------------------------

#[test]
fn prepare_on_whitespace_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public x;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // Line 2 is blank
    let items = type_hierarchy::prepare(&st, &path, source, Position::new(2, 0), &li);
    assert!(items.is_none(), "prepare on whitespace should return None");
}

// ---------------------------------------------------------------------------
// 22. Prepare on event returns None
// ---------------------------------------------------------------------------

#[test]
fn prepare_on_event_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    event Transfer(address indexed from, address indexed to, uint256 amount);
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Transfer").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li);

    assert!(
        items.is_none(),
        "prepare should return None for an event declaration"
    );
}

// ---------------------------------------------------------------------------
// 23. Prepare on modifier returns None
// ---------------------------------------------------------------------------

#[test]
fn prepare_on_modifier_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    modifier onlyOwner() {
        _;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("onlyOwner").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li);

    assert!(items.is_none(), "prepare should return None for a modifier");
}

// ---------------------------------------------------------------------------
// 24. Prepare on enum returns None (enum is not a type hierarchy type)
// ---------------------------------------------------------------------------

#[test]
fn prepare_on_enum_returns_none() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Game {
    enum Status { Active, Paused, Ended }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Status").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li);

    assert!(
        items.is_none(),
        "prepare should return None for an enum (not in is_type_kind)"
    );
}

// ---------------------------------------------------------------------------
// 25. Multiple interfaces inherited by contract
// ---------------------------------------------------------------------------

#[test]
fn contract_inheriting_multiple_interfaces() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
}

interface IERC20Metadata {
    function name() external view returns (string memory);
}

contract Token is IERC20, IERC20Metadata {
    function transfer(address to, uint256 amount) external pure returns (bool) {
        return true;
    }
    function name() external pure returns (string memory) {
        return "Token";
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(
        source,
        source.find("contract Token").unwrap() + "contract ".len(),
    );
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let token_item = &items[0];

    let supers = type_hierarchy::supertypes(&st, token_item);
    assert_eq!(supers.len(), 2, "Token should have two supertypes");

    let names: Vec<&str> = supers.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"IERC20"), "Should have IERC20 as supertype");
    assert!(
        names.contains(&"IERC20Metadata"),
        "Should have IERC20Metadata as supertype"
    );

    // Both supertypes should be INTERFACE kind
    for s in &supers {
        assert_eq!(
            s.kind,
            SymbolKind::INTERFACE,
            "{} should have INTERFACE kind",
            s.name
        );
    }
}

// ---------------------------------------------------------------------------
// 26. Supertype detail propagates correctly
// ---------------------------------------------------------------------------

#[test]
fn supertype_detail_propagates() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract GrandBase {
    uint256 public gp;
}

contract Middle is GrandBase {
    uint256 public m;
}

contract Leaf is Middle {
    uint256 public l;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    // Leaf's detail should say "is Middle"
    let pos = offset_to_position(source, source.find("Leaf").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let leaf_item = &items[0];
    let detail = leaf_item.detail.as_ref().unwrap();
    assert_eq!(
        detail, "is Middle",
        "Leaf detail should be 'is Middle', got: {detail}"
    );

    // Middle's detail should say "is GrandBase"
    let supers = type_hierarchy::supertypes(&st, leaf_item);
    assert_eq!(supers.len(), 1);
    let middle_item = &supers[0];
    let middle_detail = middle_item.detail.as_ref().unwrap();
    assert_eq!(
        middle_detail, "is GrandBase",
        "Middle detail should be 'is GrandBase', got: {middle_detail}"
    );
}

// ---------------------------------------------------------------------------
// 27. URI correctness in type hierarchy items
// ---------------------------------------------------------------------------

#[test]
fn prepare_returns_correct_uri() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract MyContract {
    uint256 public x;
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("MyContract").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let item = &items[0];

    let item_path = item.uri.to_file_path().unwrap();
    assert_eq!(
        item_path, path,
        "Item URI should point to the correct file path"
    );
}

// ---------------------------------------------------------------------------
// 28. Cross-file inheritance hierarchy
// ---------------------------------------------------------------------------

#[test]
fn cross_file_supertypes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let base_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    uint256 public x;
}
"#;
    let base_path = tmp.path().join("Base.sol");
    std::fs::write(&base_path, base_source).unwrap();

    let child_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Base} from "./Base.sol";

contract Child is Base {
    uint256 public y;
}
"#;
    let child_path = tmp.path().join("Child.sol");
    std::fs::write(&child_path, child_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&base_path, base_source, &mut parser);
    st.resolve_file_references(&base_path, &mut parser);
    st.index_file(&child_path, child_source, &mut parser);
    st.resolve_file_references(&child_path, &mut parser);

    let li = LineIndex::new(child_source);
    let pos = offset_to_position(child_source, child_source.find("Child").unwrap());
    let items = type_hierarchy::prepare(&st, &child_path, child_source, pos, &li).unwrap();
    let child_item = &items[0];

    let supers = type_hierarchy::supertypes(&st, child_item);
    assert_eq!(
        supers.len(),
        1,
        "Child should have one supertype across files"
    );
    assert_eq!(supers[0].name, "Base");

    // Verify the supertype points to the correct file
    let super_path = supers[0].uri.to_file_path().unwrap();
    assert_eq!(
        super_path, base_path,
        "Supertype URI should point to Base.sol"
    );
}

// ---------------------------------------------------------------------------
// 29. Cross-file subtypes
// ---------------------------------------------------------------------------

#[test]
fn cross_file_subtypes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());

    let base_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    uint256 public x;
}
"#;
    let base_path = tmp.path().join("Base.sol");
    std::fs::write(&base_path, base_source).unwrap();

    let child_source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {Base} from "./Base.sol";

contract Child is Base {
    uint256 public y;
}
"#;
    let child_path = tmp.path().join("Child.sol");
    std::fs::write(&child_path, child_source).unwrap();

    let mut st = SymbolTable::new(resolver);
    st.index_file(&base_path, base_source, &mut parser);
    st.resolve_file_references(&base_path, &mut parser);
    st.index_file(&child_path, child_source, &mut parser);
    st.resolve_file_references(&child_path, &mut parser);

    let li = LineIndex::new(base_source);
    let pos = offset_to_position(base_source, base_source.find("Base").unwrap());
    let items = type_hierarchy::prepare(&st, &base_path, base_source, pos, &li).unwrap();
    let base_item = &items[0];

    let subs = type_hierarchy::subtypes(&st, base_item);
    assert_eq!(
        subs.len(),
        1,
        "Base should have one subtype from another file"
    );
    assert_eq!(subs[0].name, "Child");

    // Verify subtype URI points to Child.sol
    let sub_path = subs[0].uri.to_file_path().unwrap();
    assert_eq!(
        sub_path, child_path,
        "Subtype URI should point to Child.sol"
    );
}

// ---------------------------------------------------------------------------
// 30. Selection range and range are valid for prepare items
// ---------------------------------------------------------------------------

#[test]
fn prepare_returns_valid_ranges() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    uint256 public balance;

    function deposit() public {}
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("Vault").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let item = &items[0];

    // selection_range should cover the name "Vault"
    assert_eq!(
        item.selection_range.start.line, item.selection_range.end.line,
        "Selection range for name should be on one line"
    );

    // The full range should be >= selection_range (it contains the whole contract)
    assert!(
        item.range.start.line <= item.selection_range.start.line,
        "Full range start should be at or before selection range start"
    );
    assert!(
        item.range.end.line >= item.selection_range.end.line,
        "Full range end should be at or after selection range end"
    );

    // The contract body should span multiple lines
    assert!(
        item.range.end.line > item.range.start.line,
        "Contract full range should span multiple lines"
    );
}

// ---------------------------------------------------------------------------
// 31. Subtypes with mixed contract and interface
// ---------------------------------------------------------------------------

#[test]
fn subtypes_mixed_contracts_and_interfaces() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IToken {
    function transfer(address to, uint256 amount) external returns (bool);
}

interface IExtendedToken is IToken {
    function mint(address to, uint256 amount) external;
}

contract Token is IToken {
    function transfer(address to, uint256 amount) external pure returns (bool) {
        return true;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("IToken").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let itoken_item = &items[0];

    let subs = type_hierarchy::subtypes(&st, itoken_item);
    assert_eq!(
        subs.len(),
        2,
        "IToken should have two subtypes (IExtendedToken and Token)"
    );

    let names: Vec<&str> = subs.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"IExtendedToken"),
        "IToken subtypes should contain IExtendedToken"
    );
    assert!(
        names.contains(&"Token"),
        "IToken subtypes should contain Token"
    );

    // Verify kinds are correct
    for s in &subs {
        match s.name.as_str() {
            "IExtendedToken" => assert_eq!(s.kind, SymbolKind::INTERFACE),
            "Token" => assert_eq!(s.kind, SymbolKind::CLASS),
            _ => panic!("Unexpected subtype: {}", s.name),
        }
    }
}

// ---------------------------------------------------------------------------
// 32. Struct has no supertypes or subtypes
// ---------------------------------------------------------------------------

#[test]
fn struct_has_no_supertypes_or_subtypes() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Registry {
    struct User {
        address wallet;
        uint256 balance;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("User").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let user_item = &items[0];

    let supers = type_hierarchy::supertypes(&st, user_item);
    assert!(
        supers.is_empty(),
        "Struct should have no supertypes (structs cannot inherit)"
    );

    let subs = type_hierarchy::subtypes(&st, user_item);
    assert!(
        subs.is_empty(),
        "Struct should have no subtypes (structs cannot be inherited)"
    );
}

// ---------------------------------------------------------------------------
// 33. Library has no supertypes or subtypes
// ---------------------------------------------------------------------------

#[test]
fn library_has_no_supertypes_or_subtypes() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#;
    let (st, path, _tmp) = setup_on_disk(source);
    let li = LineIndex::new(source);

    let pos = offset_to_position(source, source.find("MathLib").unwrap());
    let items = type_hierarchy::prepare(&st, &path, source, pos, &li).unwrap();
    let lib_item = &items[0];

    let supers = type_hierarchy::supertypes(&st, lib_item);
    assert!(
        supers.is_empty(),
        "Library should have no supertypes (libraries cannot inherit)"
    );

    let subs = type_hierarchy::subtypes(&st, lib_item);
    assert!(
        subs.is_empty(),
        "Library should have no subtypes (libraries cannot be inherited)"
    );
}
