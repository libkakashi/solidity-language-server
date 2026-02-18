use std::path::{Path, PathBuf};

use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::*;

fn index(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("."));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

fn get_fi<'a>(st: &'a SymbolTable, path: &Path) -> &'a FileIndex {
    st.get_file_index(path).unwrap()
}

#[test]
fn test_contract_declaration() {
    let source = r#"
contract Foo {
    uint256 public x;
    function bar() public returns (uint256) {
        return x;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let names: Vec<&str> = fi.declarations.values().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"Foo"), "names: {names:?}");
    assert!(names.contains(&"x"), "names: {names:?}");
    assert!(names.contains(&"bar"), "names: {names:?}");

    let foo = fi.declarations.values().find(|d| d.name == "Foo").unwrap();
    assert_eq!(foo.kind, DeclKind::Contract);
    assert_eq!(foo.members().len(), 2);
}

#[test]
fn test_struct_members() {
    let source = r#"
contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    let point = fi
        .declarations
        .values()
        .find(|d| d.name == "Point")
        .unwrap();
    assert_eq!(point.kind, DeclKind::Struct);
    assert_eq!(point.members().len(), 2);
    assert_eq!(point.members()[0].name, "x");
    assert_eq!(point.members()[1].name, "y");
}

#[test]
fn test_function_parameters() {
    let source = r#"
contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    let add = fi.declarations.values().find(|d| d.name == "add").unwrap();
    let params = add.parameters();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0], ("uint256".to_string(), "a".to_string()));
    assert_eq!(params[1], ("uint256".to_string(), "b".to_string()));
    let returns = add.return_parameters();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].0, "uint256");
}

#[test]
fn test_local_variable_resolution() {
    let source = r#"
contract Foo {
    function bar() public {
        uint256 x = 42;
        uint256 y = x;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let x_refs: Vec<&Reference> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "x" && r.resolved.is_some())
        .collect();
    assert!(!x_refs.is_empty(), "x should be resolved.");

    let x_decl = fi
        .declarations
        .values()
        .find(|d| d.name == "x" && d.kind == DeclKind::LocalVariable)
        .unwrap();
    assert_eq!(x_refs[0].resolved.as_ref().unwrap(), &x_decl.id);
}

#[test]
fn test_enum_declaration() {
    let source = r#"
contract Foo {
    enum Status { Active, Inactive, Paused }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    let status = fi
        .declarations
        .values()
        .find(|d| d.name == "Status")
        .unwrap();
    assert_eq!(status.kind, DeclKind::Enum);
    assert_eq!(status.enum_values(), &["Active", "Inactive", "Paused"]);
    assert_eq!(status.members().len(), 3);
}

#[test]
fn test_import_parsing() {
    let source = r#"
import "./Foo.sol";
import {Bar, Baz as B} from "./Bar.sol";
import "./Lib.sol" as Lib;
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    assert_eq!(fi.imports.len(), 3);
    assert!(matches!(fi.imports[0].kind, ImportKind::Glob));
    if let ImportKind::Named(ref names) = fi.imports[1].kind {
        assert_eq!(names.len(), 2);
        assert_eq!(names[0].0, "Bar");
        assert_eq!(names[0].1, None);
        assert_eq!(names[1].0, "Baz");
        assert_eq!(names[1].1, Some("B".to_string()));
    } else {
        panic!("Expected Named import");
    }
    if let ImportKind::Alias(ref alias) = fi.imports[2].kind {
        assert_eq!(alias, "Lib");
    } else {
        panic!("Expected Alias import");
    }
}

#[test]
fn test_natspec_extraction() {
    let source = r#"
/// @notice This is a test function
/// @param x The value
function foo(uint256 x) public pure returns (uint256) {
    return x;
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    let foo = fi.declarations.values().find(|d| d.name == "foo").unwrap();
    let natspec = foo.natspec().unwrap();
    assert!(natspec.contains("@notice This is a test function"));
    assert!(natspec.contains("@param x The value"));
}

#[test]
fn test_resolve_at() {
    let source = r#"
contract Foo {
    uint256 public x;
    function bar() public returns (uint256) {
        return x;
    }
}
"#;
    let (st, path) = index(source);

    let return_x_pos = source.find("return x;").unwrap() + "return ".len();
    let decl = st.resolve_at(&path, return_x_pos);
    assert!(decl.is_some(), "Should resolve x");
    let decl = decl.unwrap();
    assert_eq!(decl.name, "x");
    assert_eq!(decl.kind, DeclKind::StateVariable);
}

#[test]
fn test_inheritance() {
    let source = r#"
contract Base {
    function foo() public virtual {}
}
contract Child is Base {
    function foo() public override {}
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    let child = fi
        .declarations
        .values()
        .find(|d| d.name == "Child")
        .unwrap();
    assert_eq!(child.base_contracts(), &["Base"]);
}

#[test]
fn test_scope_nesting() {
    let source = r#"
contract Foo {
    function bar() public {
        uint256 a = 1;
        {
            uint256 b = 2;
        }
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let a = fi.declarations.values().find(|d| d.name == "a").unwrap();
    let b = fi.declarations.values().find(|d| d.name == "b").unwrap();
    assert_ne!(a.scope, b.scope, "a and b should be in different scopes");

    let b_scope = &fi.scopes[b.scope];
    assert_eq!(b_scope.parent, Some(a.scope));
}

#[test]
fn test_slim_declaration_no_extras_for_variables() {
    let source = r#"
contract Foo {
    function bar() public {
        uint256 x = 1;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    let x = fi.declarations.values().find(|d| d.name == "x").unwrap();
    // Local variables should have no extras allocated.
    assert!(x.extras.is_none(), "local var should not allocate extras");
}

#[test]
fn test_qualified_type_resolution() {
    let source = r#"
interface IFees {
    event FeeUpdated(uint256 fee);
}
contract Pool {
    function emitFee() public {
        emit IFees.FeeUpdated(100);
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    let source_text = source;

    // "FeeUpdated" in "IFees.FeeUpdated" should be resolved.
    let fee_ref = fi
        .references
        .iter()
        .find(|r| r.name(source_text) == "FeeUpdated" && r.member_of.is_some())
        .expect("Should have a member reference for FeeUpdated");
    assert!(fee_ref.resolved.is_some(), "FeeUpdated should be resolved");

    let fee_decl = st
        .get_declaration(fee_ref.resolved.as_ref().unwrap())
        .unwrap();
    assert_eq!(fee_decl.name, "FeeUpdated");
    assert_eq!(fee_decl.kind, DeclKind::Event);
}

#[test]
fn test_struct_field_resolution() {
    let source = r#"
contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }
    function bar() public {
        Point memory p;
        uint256 val = p.x;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // "x" in "p.x" should be resolved to the struct field declaration.
    let x_ref = fi
        .references
        .iter()
        .find(|r| r.name(source) == "x" && r.member_of.is_some())
        .expect("Should have a member reference for x");
    assert!(
        x_ref.resolved.is_some(),
        "x should be resolved via p's type"
    );

    let x_decl = st
        .get_declaration(x_ref.resolved.as_ref().unwrap())
        .unwrap();
    assert_eq!(x_decl.name, "x");
    assert_eq!(x_decl.type_text.as_deref(), Some("uint256"));
}

#[test]
fn test_enum_value_resolution() {
    let source = r#"
contract Foo {
    enum Status { Active, Paused }
    function bar() public {
        Status s = Status.Active;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let active_ref = fi
        .references
        .iter()
        .find(|r| r.name(source) == "Active" && r.member_of.is_some())
        .expect("Should have a member reference for Active");
    assert!(active_ref.resolved.is_some(), "Active should be resolved");

    let active_decl = st
        .get_declaration(active_ref.resolved.as_ref().unwrap())
        .unwrap();
    assert_eq!(active_decl.name, "Active");
    assert_eq!(active_decl.kind, DeclKind::EnumValue);
}

#[test]
fn test_qualified_type_in_type_position() {
    let source = r#"
interface IFees {
    struct Fee {
        uint256 amount;
    }
}
contract Pool {
    function getFee() external returns (IFees.Fee memory) {}
    function setFee(IFees.Fee memory f) external {}
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // All "Fee" references with member_of should be resolved.
    let fee_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "Fee" && r.member_of.is_some())
        .collect();
    assert!(
        fee_refs.len() >= 2,
        "Should have at least 2 qualified Fee refs, got {}",
        fee_refs.len()
    );
    for r in &fee_refs {
        assert!(
            r.resolved.is_some(),
            "Fee in qualified type position should be resolved"
        );
    }
}

#[test]
fn test_contract_member_function_resolution() {
    let source = r#"
library Math {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
contract Foo {
    function bar() public pure returns (uint256) {
        return Math.add(1, 2);
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let add_ref = fi
        .references
        .iter()
        .find(|r| r.name(source) == "add" && r.member_of.is_some())
        .expect("Should have a member reference for add");
    assert!(add_ref.resolved.is_some(), "add should resolve to Math.add");

    let add_decl = st
        .get_declaration(add_ref.resolved.as_ref().unwrap())
        .unwrap();
    assert_eq!(add_decl.name, "add");
    assert_eq!(add_decl.kind, DeclKind::Function);
}

// ---- Diagnostic tests to find resolution bugs ----

#[test]
fn test_bug_emit_event_resolution() {
    let source = r#"
contract Foo {
    event Transfer(address from, address to, uint256 amount);
    function transfer() public {
        emit Transfer(msg.sender, address(0), 100);
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let transfer_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "Transfer")
        .collect();
    assert!(
        !transfer_refs.is_empty(),
        "Should have a reference for Transfer in emit"
    );
    let resolved = transfer_refs.iter().find(|r| r.resolved.is_some());
    assert!(
        resolved.is_some(),
        "Transfer in emit should be resolved to the event declaration"
    );
}

#[test]
fn test_bug_function_call_resolution() {
    let source = r#"
contract Foo {
    function helper() internal pure returns (uint256) {
        return 42;
    }
    function bar() public view returns (uint256) {
        return helper();
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let helper_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "helper")
        .collect();
    assert!(
        !helper_refs.is_empty(),
        "Should have a reference for helper() call"
    );
    let resolved = helper_refs.iter().find(|r| r.resolved.is_some());
    assert!(
        resolved.is_some(),
        "helper in function call should be resolved"
    );
}

#[test]
fn test_bug_resolve_at_function_call() {
    let source = r#"
contract Foo {
    function helper() internal pure returns (uint256) {
        return 42;
    }
    function bar() public view returns (uint256) {
        return helper();
    }
}
"#;
    let (st, path) = index(source);
    let pos = source.find("return helper()").unwrap() + "return ".len();
    let decl = st.resolve_at(&path, pos);
    assert!(decl.is_some(), "resolve_at on helper() call should work");
    assert_eq!(decl.unwrap().name, "helper");
}

#[test]
fn test_bug_state_var_reference_in_expression() {
    let source = r#"
contract Foo {
    uint256 public count;
    function increment() public {
        count = count + 1;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let count_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "count" && r.resolved.is_some())
        .collect();
    assert!(
        count_refs.len() >= 2,
        "Should have at least 2 resolved references for count, got {}",
        count_refs.len()
    );
}

#[test]
fn test_bug_struct_property_hover() {
    let source = r#"
contract Foo {
    struct Config {
        uint256 fee;
        address admin;
    }
    Config public config;
    function getFee() public view returns (uint256) {
        return config.fee;
    }
}
"#;
    let (st, path) = index(source);

    // resolve_at on "fee" in "config.fee"
    let pos = source.find("config.fee").unwrap() + "config.".len();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "resolve_at on struct property 'fee' should work"
    );
    assert_eq!(decl.unwrap().name, "fee");
}

#[test]
fn test_bug_inherited_function_resolution() {
    let source = r#"
contract Base {
    function baseFn() public virtual returns (uint256) {
        return 1;
    }
}
contract Child is Base {
    function callBase() public returns (uint256) {
        return baseFn();
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let base_fn_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "baseFn" && r.resolved.is_some())
        .collect();
    assert!(
        !base_fn_refs.is_empty(),
        "baseFn() called from Child should be resolved via inheritance"
    );
}

#[test]
fn test_bug_library_using_for() {
    let source = r#"
library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
contract Foo {
    function bar() public pure returns (uint256) {
        return SafeMath.add(1, 2);
    }
}
"#;
    let (st, path) = index(source);

    let pos = source.find("SafeMath.add(1").unwrap() + "SafeMath.".len();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "resolve_at on SafeMath.add should resolve to the library function"
    );
    assert_eq!(decl.unwrap().name, "add");
}

#[test]
fn test_bug_chained_member_access() {
    let source = r#"
contract Foo {
    struct Inner {
        uint256 value;
    }
    struct Outer {
        Inner inner;
    }
    Outer public data;
    function getValue() public view returns (uint256) {
        return data.inner.value;
    }
}
"#;
    let (st, path) = index(source);

    // resolve_at on "inner" in "data.inner.value"
    let inner_pos = source.find("data.inner.value").unwrap() + "data.".len();
    let decl = st.resolve_at(&path, inner_pos);
    assert!(
        decl.is_some(),
        "resolve_at on 'inner' in chained access should work"
    );
    assert_eq!(decl.unwrap().name, "inner");

    // resolve_at on "value" in "data.inner.value"
    let value_pos = source.find("data.inner.value").unwrap() + "data.inner.".len();
    let value_decl = st.resolve_at(&path, value_pos);
    assert!(
        value_decl.is_some(),
        "resolve_at on 'value' in chained access should work"
    );
    assert_eq!(value_decl.unwrap().name, "value");
}

#[test]
fn test_bug_return_type_resolution() {
    let source = r#"
contract Foo {
    struct Point { uint256 x; uint256 y; }
    function getPoint() public view returns (Point memory) {
        Point memory p;
        return p;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // "Point" in returns type should be resolved
    let point_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "Point" && r.resolved.is_some())
        .collect();
    assert!(
        !point_refs.is_empty(),
        "Point in return type should have resolved references"
    );
}

#[test]
fn test_bug_parameter_type_resolution() {
    let source = r#"
contract Foo {
    struct Point { uint256 x; uint256 y; }
    function setPoint(Point memory p) public {}
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let point_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "Point" && r.resolved.is_some())
        .collect();
    assert!(
        !point_refs.is_empty(),
        "Point in parameter type should have resolved references"
    );
}

#[test]
fn test_bug_mapping_value_type_resolution() {
    // Mapping types that use user-defined types
    let source = r#"
contract Foo {
    struct Info { uint256 val; }
    mapping(address => Info) public infos;
    function get(address a) public view returns (uint256) {
        return infos[a].val;
    }
}
"#;
    let (st, path) = index(source);

    // "val" in "infos[a].val" should resolve via mapping value type
    let val_pos = source.find(".val").unwrap() + 1;
    let decl = st.resolve_at(&path, val_pos);
    assert!(
        decl.is_some(),
        "val in infos[a].val should resolve via mapping value type"
    );
    assert_eq!(decl.unwrap().name, "val");
}

#[test]
fn test_bug_assignment_lhs_member() {
    let source = r#"
contract Foo {
    struct Config { uint256 fee; }
    Config config;
    function setFee(uint256 f) public {
        config.fee = f;
    }
}
"#;
    let (st, path) = index(source);

    let pos = source.find("config.fee = f").unwrap() + "config.".len();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "resolve_at on struct field in assignment LHS should work"
    );
    assert_eq!(decl.unwrap().name, "fee");
}

#[test]
fn test_bug_for_loop_var_resolution() {
    let source = r#"
contract Foo {
    uint256[] public items;
    function sum() public view returns (uint256) {
        uint256 total = 0;
        for (uint256 i = 0; i < items.length; i++) {
            total = total + items[i];
        }
        return total;
    }
}
"#;
    let (st, path) = index(source);
    let _fi = get_fi(&st, &path);

    // "total" reference inside for loop body should resolve
    let total_in_loop = source.find("total = total + items").unwrap() + "total = ".len();
    let decl = st.resolve_at(&path, total_in_loop);
    assert!(
        decl.is_some(),
        "resolve_at on 'total' inside for loop should work"
    );
    assert_eq!(decl.unwrap().name, "total");
}

#[test]
fn test_bug_enum_type_in_variable_decl() {
    let source = r#"
contract Foo {
    enum Status { Active, Paused }
    Status public currentStatus;
    function pause() public {
        currentStatus = Status.Paused;
    }
}
"#;
    let (st, path) = index(source);

    // "Status" in the state variable type should resolve
    let fi = get_fi(&st, &path);
    let status_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "Status" && r.resolved.is_some())
        .collect();
    assert!(
        !status_refs.is_empty(),
        "Status used as a type in state var should be resolved"
    );

    // "Paused" in "Status.Paused" should resolve
    let paused_pos = source.find("Status.Paused").unwrap() + "Status.".len();
    let decl = st.resolve_at(&path, paused_pos);
    assert!(decl.is_some(), "resolve_at on Status.Paused should work");
    assert_eq!(decl.unwrap().name, "Paused");
}

#[test]
fn test_bug_constructor_call_resolution() {
    // new ContractName(...) — ContractName should resolve
    let source = r#"
contract Token {
    constructor() {}
}
contract Factory {
    function create() public returns (Token) {
        Token t = new Token();
        return t;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // "Token" references in Factory should resolve
    let token_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "Token" && r.resolved.is_some())
        .collect();
    assert!(
        !token_refs.is_empty(),
        "Token references in Factory should be resolved, got {} unresolved",
        fi.references
            .iter()
            .filter(|r| r.name(source) == "Token" && r.resolved.is_none())
            .count()
    );
}

#[test]
fn test_bug_if_condition_resolution() {
    let source = r#"
contract Foo {
    bool public paused;
    function doSomething() public view {
        if (paused) {
            revert();
        }
    }
}
"#;
    let (st, path) = index(source);

    let pos = source.find("if (paused)").unwrap() + "if (".len();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "resolve_at on 'paused' in if condition should work"
    );
    assert_eq!(decl.unwrap().name, "paused");
}

#[test]
fn test_bug_event_emit_with_qualified_name() {
    // emit Interface.Event(...)
    let source = r#"
interface IToken {
    event Transfer(address from, address to, uint256 amount);
}
contract Token is IToken {
    function transfer(address to, uint256 amount) public {
        emit IToken.Transfer(msg.sender, to, amount);
    }
}
"#;
    let (st, path) = index(source);

    // "Transfer" in "IToken.Transfer" should resolve
    let pos = source.find("IToken.Transfer(msg").unwrap() + "IToken.".len();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "resolve_at on IToken.Transfer in emit should work"
    );
    assert_eq!(decl.unwrap().name, "Transfer");
}

#[test]
fn test_bug_multiple_contracts_cross_ref() {
    let source = r#"
contract A {
    function foo() public pure returns (uint256) { return 1; }
}
contract B {
    A public a;
    function callFoo() public view returns (uint256) {
        return a.foo();
    }
}
"#;
    let (st, path) = index(source);

    // "foo" in "a.foo()" where a is of type A
    let pos = source.find("a.foo()").unwrap() + "a.".len();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "resolve_at on 'foo' in a.foo() should resolve to A.foo"
    );
    assert_eq!(decl.unwrap().name, "foo");
}

#[test]
fn test_bug_error_usage_resolution() {
    let source = r#"
contract Foo {
    error Unauthorized(address caller);
    function restricted() public view {
        if (msg.sender != address(0)) {
            revert Unauthorized(msg.sender);
        }
    }
}
"#;
    let (st, path) = index(source);

    let pos = source.find("revert Unauthorized").unwrap() + "revert ".len();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "resolve_at on 'Unauthorized' in revert should work"
    );
    assert_eq!(decl.unwrap().name, "Unauthorized");
}

#[test]
fn test_bug_modifier_usage_resolution() {
    let source = r#"
contract Foo {
    address public owner;
    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }
    function restricted() public onlyOwner {
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let owner_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "onlyOwner" && r.resolved.is_some())
        .collect();
    assert!(
        !owner_refs.is_empty(),
        "onlyOwner modifier usage should be resolved"
    );
}

#[test]
fn test_bug_function_param_resolution_in_body() {
    let source = r#"
contract Foo {
    function bar(uint256 amount) public pure returns (uint256) {
        return amount * 2;
    }
}
"#;
    let (st, path) = index(source);

    let pos = source.find("return amount").unwrap() + "return ".len();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "resolve_at on function parameter 'amount' in body should work"
    );
    assert_eq!(decl.unwrap().name, "amount");
    assert_eq!(decl.unwrap().kind, DeclKind::Parameter);
}

#[test]
fn test_bug_struct_literal_field_no_false_resolve() {
    // In struct literals like Point({x: 1, y: 2}), x and y should NOT
    // resolve to unrelated variables named x/y.
    let source = r#"
contract Foo {
    struct Point { uint256 x; uint256 y; }
    uint256 public x;
    function make() public view returns (Point memory) {
        return Point({x: 1, y: 2});
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // The "x" inside {x: 1, y: 2} should NOT resolve to the state variable x
    // (the call_struct_argument handler should skip it)
    let point_call_pos = source.find("{x: 1,").unwrap();
    let x_in_struct = fi.references.iter().find(|r| {
        r.name(source) == "x" && r.range.0 > point_call_pos && r.range.0 < point_call_pos + 10
    });
    // Either there's no reference for the struct field name, or it should not
    // be resolved to the state variable
    if let Some(r) = x_in_struct {
        if let Some(ref decl_id) = r.resolved {
            let decl = st.get_declaration(decl_id).unwrap();
            assert_ne!(
                decl.kind,
                DeclKind::StateVariable,
                "x in struct literal should not resolve to state variable x"
            );
        }
    }
}

#[test]
fn test_bug_resolve_at_on_declaration_name() {
    // Hovering on a declaration name itself should return that declaration
    let source = r#"
contract Foo {
    function myFunc() public {}
}
"#;
    let (st, path) = index(source);

    let pos = source.find("myFunc").unwrap();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "resolve_at on a declaration name should return the declaration itself"
    );
    assert_eq!(decl.unwrap().name, "myFunc");
}

#[test]
fn test_bug_comprehensive_resolution() {
    // Verify all non-builtin references resolve in a realistic contract
    let source = r#"
contract Test {
    struct Config {
        uint256 fee;
        address admin;
    }
    event FeeUpdated(uint256 fee);
    error Unauthorized();
    Config public config;

    function setFee(uint256 newFee) external {
        config.fee = newFee;
        emit FeeUpdated(newFee);
    }

    function restricted() external view {
        if (msg.sender != config.admin) {
            revert Unauthorized();
        }
    }

    function callHelper() external view returns (uint256) {
        return helper();
    }

    function helper() internal pure returns (uint256) {
        return 42;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // All non-builtin references should be resolved
    let unresolved: Vec<_> = fi
        .references
        .iter()
        .filter(|r| {
            let name = r.name(source);
            r.resolved.is_none() && name != "msg" && name != "sender" && name != "address"
        })
        .collect();
    assert!(
        unresolved.is_empty(),
        "All non-builtin references should be resolved, but these are not: {:?}",
        unresolved
            .iter()
            .map(|r| r.name(source))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_bug_variable_decl_type_not_walked_in_expression() {
    // variable_declaration_statement: the value expression should generate refs
    let source = r#"
contract Foo {
    uint256 public x;
    function bar() public view returns (uint256) {
        uint256 y = x + 1;
        return y;
    }
}
"#;
    let (st, path) = index(source);
    // "x" in "uint256 y = x + 1" should resolve
    let x_pos = source.find("= x + 1").unwrap() + 2;
    let decl = st.resolve_at(&path, x_pos);
    assert!(
        decl.is_some(),
        "x in variable init expression should resolve"
    );
    assert_eq!(decl.unwrap().name, "x");
}

#[test]
fn test_bug_tuple_decl_init_expression() {
    let source = r#"
contract Foo {
    function helper() internal pure returns (uint256, uint256) {
        return (1, 2);
    }
    function bar() public pure {
        (uint256 a, uint256 b) = helper();
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    // "helper" in the call should resolve
    let helper_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "helper" && r.resolved.is_some())
        .collect();
    assert!(
        !helper_refs.is_empty(),
        "helper() in tuple assignment should be resolved"
    );
}

#[test]
fn test_bug_nested_call_expression() {
    // require(balanceOf(msg.sender) > 0) — nested call inside call
    let source = r#"
contract Foo {
    mapping(address => uint256) balances;
    function balanceOf(address a) public view returns (uint256) {
        return balances[a];
    }
    function check() public view {
        require(balanceOf(msg.sender) > 0);
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    let bof_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "balanceOf" && r.resolved.is_some())
        .collect();
    assert!(
        !bof_refs.is_empty(),
        "balanceOf in nested call should be resolved"
    );
}

#[test]
fn test_bug_return_expression_member() {
    // return someStruct.field — member in return expression
    let source = r#"
contract Foo {
    struct Data { uint256 val; }
    Data public data;
    function getVal() public view returns (uint256) {
        return data.val;
    }
}
"#;
    let (st, path) = index(source);
    let pos = source.find("data.val").unwrap() + "data.".len();
    let decl = st.resolve_at(&path, pos);
    assert!(decl.is_some(), "val in return data.val should resolve");
    assert_eq!(decl.unwrap().name, "val");
}

#[test]
fn test_bug_array_element_member() {
    // arr[i].field — member access on array element
    let source = r#"
contract Foo {
    struct Item { uint256 price; }
    Item[] public items;
    function getPrice(uint256 i) public view returns (uint256) {
        return items[i].price;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    // "price" member ref should exist and be resolved
    let price_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "price" && r.member_of.is_some())
        .collect();
    assert!(!price_refs.is_empty(), "Should have a member ref for price");
    assert!(
        price_refs[0].resolved.is_some(),
        "price in items[i].price should be resolved"
    );

    // resolve_at on "price"
    let price_pos = source.find(".price").unwrap() + 1;
    let decl = st.resolve_at(&path, price_pos);
    assert!(decl.is_some(), "resolve_at on price should work");
    assert_eq!(decl.unwrap().name, "price");

    // "i" should resolve to the parameter
    let i_pos = source.find("items[i]").unwrap() + "items[".len();
    let decl = st.resolve_at(&path, i_pos);
    assert!(decl.is_some(), "i in items[i] should resolve to parameter");
    assert_eq!(decl.unwrap().name, "i");
}

#[test]
fn test_bug_ternary_expression() {
    let source = r#"
contract Foo {
    uint256 public x;
    uint256 public y;
    function max() public view returns (uint256) {
        return x > y ? x : y;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    let x_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "x" && r.resolved.is_some())
        .collect();
    // x appears in "x > y" and "? x :" — should have at least 2 resolved refs
    assert!(
        x_refs.len() >= 2,
        "x should be resolved at least twice in ternary, got {}",
        x_refs.len()
    );
}

#[test]
fn test_bug_interface_function_param_type() {
    // Interface function with struct param type
    let source = r#"
interface IPool {
    struct Config { uint256 fee; }
    function setConfig(Config calldata c) external;
}
contract Pool is IPool {
    Config public config;
    function setConfig(Config calldata c) external override {
        config = c;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);
    // All "Config" refs should be resolved
    let config_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "Config")
        .collect();
    let resolved_count = config_refs.iter().filter(|r| r.resolved.is_some()).count();
    // config_refs: total vs resolved
    assert!(
        resolved_count >= 2,
        "At least 2 Config type refs should be resolved, got {}",
        resolved_count
    );
}

#[test]
fn test_bug_multiple_return_values() {
    let source = r#"
contract Foo {
    function pair() internal pure returns (uint256, uint256) {
        return (1, 2);
    }
    function use_pair() public pure returns (uint256) {
        (uint256 a, uint256 b) = pair();
        return a + b;
    }
}
"#;
    let (st, path) = index(source);
    // "a" and "b" in "return a + b" should resolve
    let a_pos = source.find("return a + b").unwrap() + "return ".len();
    let decl = st.resolve_at(&path, a_pos);
    assert!(decl.is_some(), "a should resolve");
    assert_eq!(decl.unwrap().name, "a");

    let b_pos = source.find("return a + b").unwrap() + "return a + ".len();
    let decl = st.resolve_at(&path, b_pos);
    assert!(decl.is_some(), "b should resolve");
    assert_eq!(decl.unwrap().name, "b");
}

#[test]
fn test_bug_cross_file_struct_member_resolution() {
    let tmp = tempfile::tempdir().unwrap();
    let a_path = tmp.path().join("A.sol");
    let b_path = tmp.path().join("B.sol");

    let a_source = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;
struct Test { uint256 foo; }
"#;
    let b_source = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;
import {Test} from "./A.sol";
contract Bar {
    Test test;
    function getFoo() public view returns (uint256) {
        return test.foo;
    }
}
"#;
    std::fs::write(&a_path, a_source).unwrap();
    std::fs::write(&b_path, b_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&a_path, a_source, &mut parser);
    st.resolve_file_references(&a_path, &mut parser);
    st.index_file(&b_path, b_source, &mut parser);
    st.resolve_file_references(&b_path, &mut parser);

    // "foo" in "test.foo" should resolve to the struct field
    let foo_pos = b_source.find("test.foo").unwrap() + "test.".len();
    let decl = st.resolve_at(&b_path, foo_pos);
    assert!(
        decl.is_some(),
        "resolve_at on 'foo' in test.foo should resolve to struct field in A.sol"
    );
    assert_eq!(decl.unwrap().name, "foo");
}

#[test]
fn test_bug_cross_file_named_import_contract_member() {
    let tmp = tempfile::tempdir().unwrap();
    let lib_path = tmp.path().join("Lib.sol");
    let main_path = tmp.path().join("Main.sol");

    let lib_source = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;
library MyLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#;
    let main_source = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;
import {MyLib} from "./Lib.sol";
contract Main {
    function calc() public pure returns (uint256) {
        return MyLib.add(1, 2);
    }
}
"#;
    std::fs::write(&lib_path, lib_source).unwrap();
    std::fs::write(&main_path, main_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&lib_path, lib_source, &mut parser);
    st.resolve_file_references(&lib_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // "add" in "MyLib.add" should resolve
    let add_pos = main_source.find("MyLib.add(1").unwrap() + "MyLib.".len();
    let decl = st.resolve_at(&main_path, add_pos);
    assert!(
        decl.is_some(),
        "resolve_at on 'add' in MyLib.add should resolve to the library function"
    );
    assert_eq!(decl.unwrap().name, "add");
}

#[test]
fn test_bug_function_return_member_access() {
    // getConfig().fee — member access on function return value
    let source = r#"
contract Foo {
    struct Config { uint256 fee; address admin; }
    Config config;
    function getConfig() internal view returns (Config memory) {
        return config;
    }
    function readFee() public view returns (uint256) {
        return getConfig().fee;
    }
}
"#;
    let (st, path) = index(source);

    // "fee" in "getConfig().fee" should resolve via function return type
    let fee_pos = source.find("getConfig().fee").unwrap() + "getConfig().".len();
    let decl = st.resolve_at(&path, fee_pos);
    assert!(
        decl.is_some(),
        "fee in getConfig().fee should resolve via function return type"
    );
    assert_eq!(decl.unwrap().name, "fee");
}

#[test]
fn test_bug_mapping_member_access() {
    // mapping(address => Config) configs; configs[addr].fee
    let source = r#"
contract Foo {
    struct Config { uint256 fee; }
    mapping(address => Config) public configs;
    function getFee(address a) public view returns (uint256) {
        return configs[a].fee;
    }
}
"#;
    let (st, path) = index(source);

    // "fee" in "configs[a].fee" should resolve via mapping value type
    let fee_pos = source.find(".fee").unwrap() + 1;
    let decl = st.resolve_at(&path, fee_pos);
    assert!(
        decl.is_some(),
        "fee in configs[a].fee should resolve via mapping value type"
    );
    assert_eq!(decl.unwrap().name, "fee");
}

#[test]
fn test_bug_chained_call_member() {
    // a.getInner().value — chained: member access on member call result
    let source = r#"
contract Foo {
    struct Inner { uint256 value; }
    struct Outer { Inner inner; }
    Outer public data;
    function getInner() internal view returns (Inner memory) {
        return data.inner;
    }
    function getValue() public view returns (uint256) {
        return data.inner.value;
    }
}
"#;
    let (st, path) = index(source);

    // "value" in "data.inner.value"
    let value_pos = source.find("data.inner.value").unwrap() + "data.inner.".len();
    let decl = st.resolve_at(&path, value_pos);
    assert!(
        decl.is_some(),
        "value in data.inner.value should resolve (chained member access)"
    );
    assert_eq!(decl.unwrap().name, "value");
}

#[test]
fn test_bug_cross_file_named_import_struct_field() {
    // import {Test} from "A.sol"; — variable of type Test, access .foo
    // This requires find_type_declaration to follow ImportAlias → actual struct
    let tmp = tempfile::tempdir().unwrap();
    let a_path = tmp.path().join("A.sol");
    let b_path = tmp.path().join("B.sol");

    let a_source = r#"
pragma solidity ^0.8.0;
struct Test { uint256 foo; }
"#;
    let b_source = r#"
pragma solidity ^0.8.0;
import {Test} from "./A.sol";
contract Bar {
    Test test;
    function setFoo(uint256 v) public {
        test.foo = v;
    }
}
"#;
    std::fs::write(&a_path, a_source).unwrap();
    std::fs::write(&b_path, b_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&a_path, a_source, &mut parser);
    st.resolve_file_references(&a_path, &mut parser);
    st.index_file(&b_path, b_source, &mut parser);
    st.resolve_file_references(&b_path, &mut parser);

    // "test" should resolve to the state variable
    let test_pos = b_source.find("test.foo = v").unwrap();
    let decl = st.resolve_at(&b_path, test_pos);
    assert!(decl.is_some(), "test should resolve");
    assert_eq!(decl.unwrap().name, "test");

    // "foo" in "test.foo" should resolve to the struct field in A.sol
    let foo_pos = b_source.find("test.foo = v").unwrap() + "test.".len();
    let decl = st.resolve_at(&b_path, foo_pos);
    assert!(
        decl.is_some(),
        "foo in test.foo should resolve to struct field via named import. \
         This requires find_type_declaration to follow through the imported struct."
    );
    assert_eq!(decl.unwrap().name, "foo");
}

#[test]
fn test_bug_cross_file_function_call_on_imported_var() {
    // import {Token} from "./Token.sol"; Token t; t.transfer(...)
    let tmp = tempfile::tempdir().unwrap();
    let token_path = tmp.path().join("Token.sol");
    let main_path = tmp.path().join("Main.sol");

    let token_source = r#"
pragma solidity ^0.8.0;
contract Token {
    function transfer(address to, uint256 amount) public returns (bool) {
        return true;
    }
}
"#;
    let main_source = r#"
pragma solidity ^0.8.0;
import {Token} from "./Token.sol";
contract Main {
    Token public token;
    function doTransfer(address to) public {
        token.transfer(to, 100);
    }
}
"#;
    std::fs::write(&token_path, token_source).unwrap();
    std::fs::write(&main_path, main_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&token_path, token_source, &mut parser);
    st.resolve_file_references(&token_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // "transfer" in "token.transfer" should resolve
    let transfer_pos = main_source.find("token.transfer(to").unwrap() + "token.".len();
    let decl = st.resolve_at(&main_path, transfer_pos);
    assert!(
        decl.is_some(),
        "transfer in token.transfer should resolve to Token.transfer via named import"
    );
    assert_eq!(decl.unwrap().name, "transfer");
}

#[test]
fn test_bug_cross_file_glob_import() {
    let tmp = tempfile::tempdir().unwrap();
    let a_path = tmp.path().join("A.sol");
    let b_path = tmp.path().join("B.sol");

    let a_source = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;
struct Test { uint256 foo; }
"#;
    let b_source = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;
import "./A.sol";
contract Bar {
    Test test;
    function getFoo() public view returns (uint256) {
        return test.foo;
    }
}
"#;
    std::fs::write(&a_path, a_source).unwrap();
    std::fs::write(&b_path, b_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&a_path, a_source, &mut parser);
    st.resolve_file_references(&a_path, &mut parser);
    st.index_file(&b_path, b_source, &mut parser);
    st.resolve_file_references(&b_path, &mut parser);

    let foo_pos = b_source.find("test.foo").unwrap() + "test.".len();
    let decl = st.resolve_at(&b_path, foo_pos);
    assert!(
        decl.is_some(),
        "resolve_at on 'foo' in test.foo should resolve via glob import"
    );
    assert_eq!(decl.unwrap().name, "foo");
}

#[test]
fn test_bug_cross_file_interface_event() {
    let tmp = tempfile::tempdir().unwrap();
    let iface_path = tmp.path().join("IFees.sol");
    let impl_path = tmp.path().join("Pool.sol");

    let iface_source = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;
interface IFees {
    event FeeUpdated(uint256 fee);
    struct Fee { uint256 amount; }
}
"#;
    let impl_source = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;
import {IFees} from "./IFees.sol";
contract Pool {
    IFees.Fee public currentFee;
    function updateFee(uint256 newFee) external {
        emit IFees.FeeUpdated(newFee);
    }
}
"#;
    std::fs::write(&iface_path, iface_source).unwrap();
    std::fs::write(&impl_path, impl_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&iface_path, iface_source, &mut parser);
    st.resolve_file_references(&iface_path, &mut parser);
    st.index_file(&impl_path, impl_source, &mut parser);
    st.resolve_file_references(&impl_path, &mut parser);

    // "FeeUpdated" in "IFees.FeeUpdated" should resolve
    let fee_pos = impl_source.find("IFees.FeeUpdated(newFee)").unwrap() + "IFees.".len();
    let decl = st.resolve_at(&impl_path, fee_pos);
    assert!(
        decl.is_some(),
        "resolve_at on FeeUpdated in IFees.FeeUpdated should work cross-file"
    );
    assert_eq!(decl.unwrap().name, "FeeUpdated");

    // "Fee" in "IFees.Fee" should resolve
    let fee_type_pos = impl_source.find("IFees.Fee public").unwrap() + "IFees.".len();
    let decl = st.resolve_at(&impl_path, fee_type_pos);
    assert!(
        decl.is_some(),
        "resolve_at on Fee in IFees.Fee type should work cross-file"
    );
    assert_eq!(decl.unwrap().name, "Fee");
}

// ===========================================================================
// Edge-case tests
// ===========================================================================

// ---------------------------------------------------------------------------
// 1. Using-for directives (UNIMPLEMENTED)
// ---------------------------------------------------------------------------
#[test]
fn test_edge_using_for_directive() {
    let source = r#"
library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
contract Foo {
    using SafeMath for uint256;
    function bar() public pure returns (uint256) {
        uint256 x = 1;
        return x.add(2);
    }
}
"#;
    let (st, path) = index(source);

    // "add" in "x.add(2)" should resolve to SafeMath.add via using-for
    let pos = source.find("x.add(2)").unwrap() + "x.".len();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "add in x.add(2) should resolve via using-for directive"
    );
    assert_eq!(decl.unwrap().name, "add");
}

// ---------------------------------------------------------------------------
// 2. Try/catch blocks (UNIMPLEMENTED — catch variable scoping not handled)
// ---------------------------------------------------------------------------
#[test]
fn test_edge_try_catch_variable_scoping() {
    let source = r#"
interface IExternal {
    function doSomething() external returns (uint256);
}
contract Foo {
    IExternal ext;
    function bar() public returns (uint256) {
        try ext.doSomething() returns (uint256 result) {
            return result;
        } catch Error(string memory reason) {
            return 0;
        }
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // "result" should be declared as a local variable inside the try returns
    let result_decl = fi
        .declarations
        .values()
        .find(|d| d.name == "result" && d.kind == DeclKind::LocalVariable);
    assert!(
        result_decl.is_some(),
        "result in try returns should be declared"
    );

    // "reason" should be declared in the catch block scope
    let reason_decl = fi
        .declarations
        .values()
        .find(|d| d.name == "reason" && d.kind == DeclKind::LocalVariable);
    assert!(
        reason_decl.is_some(),
        "reason in catch clause should be declared"
    );
}

// ---------------------------------------------------------------------------
// 3. Unchecked blocks — variables inside should be scoped
// ---------------------------------------------------------------------------
#[test]
fn test_edge_unchecked_block_variables() {
    let source = r#"
contract Foo {
    function bar() public pure returns (uint256) {
        uint256 a = 1;
        unchecked {
            uint256 b = a + 1;
            return b;
        }
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let a = fi
        .declarations
        .values()
        .find(|d| d.name == "a" && d.kind == DeclKind::LocalVariable)
        .expect("a should be declared");
    let b = fi
        .declarations
        .values()
        .find(|d| d.name == "b" && d.kind == DeclKind::LocalVariable)
        .expect("b should be declared inside unchecked block");

    // b should be in a deeper scope than a
    assert_ne!(a.scope, b.scope, "a and b should be in different scopes");

    // "a" referenced inside unchecked should resolve
    let pos = source.find("= a + 1").unwrap() + 2;
    let decl = st.resolve_at(&path, pos);
    assert!(decl.is_some(), "a inside unchecked block should resolve");
    assert_eq!(decl.unwrap().name, "a");
}

// ---------------------------------------------------------------------------
// 4. Free functions (top-level, no contract)
// ---------------------------------------------------------------------------
#[test]
fn test_edge_free_function_declaration() {
    let source = r#"
function helper(uint256 x) pure returns (uint256) {
    return x * 2;
}
contract Foo {
    function bar() public pure returns (uint256) {
        return helper(21);
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let helper = fi
        .declarations
        .values()
        .find(|d| d.name == "helper" && d.kind == DeclKind::Function)
        .expect("free function helper should be declared");
    let params = helper.parameters();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], ("uint256".to_string(), "x".to_string()));

    // "helper" call in contract should resolve to the free function
    let pos = source.find("return helper(21)").unwrap() + "return ".len();
    let decl = st.resolve_at(&path, pos);
    assert!(
        decl.is_some(),
        "free function call should resolve from inside a contract"
    );
    assert_eq!(decl.unwrap().name, "helper");
}

// ---------------------------------------------------------------------------
// 5. Multi-level inheritance (A is B, B is C) — member resolution
//    Only direct bases are searched, so transitive resolution is UNIMPLEMENTED.
// ---------------------------------------------------------------------------
#[test]
fn test_edge_multi_level_inheritance() {
    let source = r#"
contract GrandParent {
    function ancestorFn() public pure returns (uint256) {
        return 1;
    }
}
contract Parent is GrandParent {
}
contract Child is Parent {
    function callAncestor() public pure returns (uint256) {
        return ancestorFn();
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "ancestorFn" && r.resolved.is_some())
        .collect();
    assert!(
        !refs.is_empty(),
        "ancestorFn() should resolve via transitive inheritance (Child -> Parent -> GrandParent)"
    );
}

// ---------------------------------------------------------------------------
// 6. Function overloading — same name different params (UNIMPLEMENTED)
// ---------------------------------------------------------------------------
#[test]
fn test_edge_function_overloading() {
    let source = r#"
contract Foo {
    function process(uint256 x) public pure returns (uint256) {
        return x;
    }
    function process(uint256 x, uint256 y) public pure returns (uint256) {
        return x + y;
    }
    function bar() public pure returns (uint256) {
        return process(1, 2);
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // Both overloads should be declared
    let process_decls: Vec<_> = fi
        .declarations
        .values()
        .filter(|d| d.name == "process" && d.kind == DeclKind::Function)
        .collect();
    assert_eq!(
        process_decls.len(),
        2,
        "Both overloaded process functions should be declared"
    );
}

// ---------------------------------------------------------------------------
// 7. Variable shadowing — inner scope shadows outer
// ---------------------------------------------------------------------------
#[test]
fn test_edge_variable_shadowing() {
    let source = r#"
contract Foo {
    function bar() public pure returns (uint256) {
        uint256 x = 1;
        {
            uint256 x = 2;
            return x;
        }
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // There should be two declarations named "x"
    let x_decls: Vec<_> = fi
        .declarations
        .values()
        .filter(|d| d.name == "x" && d.kind == DeclKind::LocalVariable)
        .collect();
    assert_eq!(
        x_decls.len(),
        2,
        "There should be two 'x' declarations (outer and inner)"
    );

    // The two x declarations should be in different scopes
    assert_ne!(
        x_decls[0].scope, x_decls[1].scope,
        "The two x declarations should be in different scopes"
    );

    // "return x" should resolve to the inner x (the one in the block scope)
    let pos = source.find("return x;").unwrap() + "return ".len();
    let decl = st.resolve_at(&path, pos);
    assert!(decl.is_some(), "x in return should resolve");
    // The resolved x should be the one declared at "uint256 x = 2"
    let inner_x_pos = source.find("uint256 x = 2").unwrap() + "uint256 ".len();
    assert!(
        decl.unwrap().name_range.0 == inner_x_pos,
        "return x should resolve to the inner shadowing x"
    );
}

// ---------------------------------------------------------------------------
// 8. Complex type resolution (nested mappings, multi-dim arrays)
// ---------------------------------------------------------------------------
#[test]
fn test_edge_nested_mapping_type() {
    let source = r#"
contract Foo {
    mapping(address => mapping(address => uint256)) public allowances;
    function getAllowance(address owner, address spender) public view returns (uint256) {
        return allowances[owner][spender];
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let allowances = fi
        .declarations
        .values()
        .find(|d| d.name == "allowances" && d.kind == DeclKind::StateVariable)
        .expect("allowances should be declared");
    assert!(
        allowances.type_text.is_some(),
        "allowances should have type_text"
    );

    // "allowances" reference in the function body should resolve
    let pos = source.find("return allowances[").unwrap() + "return ".len();
    let decl = st.resolve_at(&path, pos);
    assert!(decl.is_some(), "allowances should resolve");
    assert_eq!(decl.unwrap().name, "allowances");
}

#[test]
fn test_edge_multi_dim_array_type() {
    let source = r#"
contract Foo {
    uint256[][] public matrix;
    function get(uint256 i, uint256 j) public view returns (uint256) {
        return matrix[i][j];
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let matrix = fi
        .declarations
        .values()
        .find(|d| d.name == "matrix" && d.kind == DeclKind::StateVariable)
        .expect("matrix should be declared");
    assert!(matrix.type_text.is_some(), "matrix should have type_text");

    // "matrix" reference should resolve
    let pos = source.find("return matrix[").unwrap() + "return ".len();
    let decl = st.resolve_at(&path, pos);
    assert!(decl.is_some(), "matrix should resolve");
    assert_eq!(decl.unwrap().name, "matrix");
}

// ---------------------------------------------------------------------------
// 9. Receive/fallback functions — should be declared
// ---------------------------------------------------------------------------
#[test]
fn test_edge_receive_fallback_functions() {
    let source = r#"
contract Foo {
    receive() external payable {}
    fallback() external payable {}
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let receive = fi
        .declarations
        .values()
        .find(|d| d.name == "receive" && d.kind == DeclKind::FallbackReceive);
    assert!(receive.is_some(), "receive function should be declared");

    let fallback = fi
        .declarations
        .values()
        .find(|d| d.name == "fallback" && d.kind == DeclKind::FallbackReceive);
    assert!(fallback.is_some(), "fallback function should be declared");
}

// ---------------------------------------------------------------------------
// 10. Constructor with inheritance args
// ---------------------------------------------------------------------------
#[test]
fn test_edge_constructor_with_inheritance_args() {
    let source = r#"
contract Base {
    uint256 public val;
    constructor(uint256 v) {
        val = v;
    }
}
contract Child is Base {
    constructor(uint256 x) Base(x) {
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // There should be two constructors
    let constructors: Vec<_> = fi
        .declarations
        .values()
        .filter(|d| d.kind == DeclKind::Constructor)
        .collect();
    assert_eq!(
        constructors.len(),
        2,
        "Both Base and Child should have constructors"
    );

    // Child constructor should have parameter x
    let child_ctor = constructors
        .iter()
        .find(|d| {
            let params = d.parameters();
            params.len() == 1 && params[0].1 == "x"
        })
        .expect("Child constructor should have param x");
    assert_eq!(child_ctor.parameters()[0].0, "uint256");
}

// ---------------------------------------------------------------------------
// 11. Modifier with parameters
// ---------------------------------------------------------------------------
#[test]
fn test_edge_modifier_with_parameters() {
    let source = r#"
contract Foo {
    mapping(address => bool) public authorized;
    modifier onlyAuthorized(address caller) {
        require(authorized[caller]);
        _;
    }
    function restricted(address a) public onlyAuthorized(a) {
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let modifier = fi
        .declarations
        .values()
        .find(|d| d.name == "onlyAuthorized" && d.kind == DeclKind::Modifier)
        .expect("onlyAuthorized modifier should be declared");
    let params = modifier.parameters();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], ("address".to_string(), "caller".to_string()));

    // modifier usage in function should resolve
    let mod_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "onlyAuthorized" && r.resolved.is_some())
        .collect();
    assert!(
        !mod_refs.is_empty(),
        "onlyAuthorized modifier usage should be resolved"
    );
}

// ---------------------------------------------------------------------------
// 12. User-defined value types
// ---------------------------------------------------------------------------
#[test]
fn test_edge_user_defined_value_type() {
    let source = r#"
type Price is uint256;
type Quantity is uint256;
contract Shop {
    Price public price;
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let price_type = fi
        .declarations
        .values()
        .find(|d| d.name == "Price" && d.kind == DeclKind::UserDefinedType);
    assert!(
        price_type.is_some(),
        "User-defined value type Price should be declared"
    );

    let quantity_type = fi
        .declarations
        .values()
        .find(|d| d.name == "Quantity" && d.kind == DeclKind::UserDefinedType);
    assert!(
        quantity_type.is_some(),
        "User-defined value type Quantity should be declared"
    );

    // "Price" reference in the state variable should resolve
    let price_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "Price" && r.resolved.is_some())
        .collect();
    assert!(
        !price_refs.is_empty(),
        "Price type reference in state variable should resolve"
    );
}

// ---------------------------------------------------------------------------
// 13. Tuple assignments with skipped elements
// ---------------------------------------------------------------------------
#[test]
fn test_edge_tuple_with_skipped_elements() {
    let source = r#"
contract Foo {
    function getPair() internal pure returns (uint256, uint256, uint256) {
        return (1, 2, 3);
    }
    function bar() public pure returns (uint256) {
        (uint256 a, , uint256 c) = getPair();
        return a + c;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let a_decl = fi
        .declarations
        .values()
        .find(|d| d.name == "a" && d.kind == DeclKind::LocalVariable);
    assert!(
        a_decl.is_some(),
        "a from tuple assignment should be declared"
    );

    let c_decl = fi
        .declarations
        .values()
        .find(|d| d.name == "c" && d.kind == DeclKind::LocalVariable);
    assert!(
        c_decl.is_some(),
        "c from tuple assignment should be declared"
    );

    // "a" and "c" in "return a + c" should resolve
    let a_pos = source.find("return a + c").unwrap() + "return ".len();
    let decl = st.resolve_at(&path, a_pos);
    assert!(decl.is_some(), "a should resolve in return statement");
    assert_eq!(decl.unwrap().name, "a");

    let c_pos = source.find("return a + c").unwrap() + "return a + ".len();
    let decl = st.resolve_at(&path, c_pos);
    assert!(decl.is_some(), "c should resolve in return statement");
    assert_eq!(decl.unwrap().name, "c");
}

// ---------------------------------------------------------------------------
// 14. Multiple contracts in same file referencing each other
// ---------------------------------------------------------------------------
#[test]
fn test_edge_multiple_contracts_same_file() {
    let source = r#"
contract Token {
    function totalSupply() public pure returns (uint256) {
        return 1000;
    }
}
contract Registry {
    Token public token;
    function getSupply() public view returns (uint256) {
        return token.totalSupply();
    }
}
contract Factory {
    function createToken() public returns (Token) {
        Token t = new Token();
        return t;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // Three contracts should be declared
    let contracts: Vec<_> = fi
        .declarations
        .values()
        .filter(|d| d.kind == DeclKind::Contract)
        .collect();
    assert_eq!(contracts.len(), 3, "Should have 3 contracts");

    // "totalSupply" in token.totalSupply() should resolve
    let pos = source.find("token.totalSupply()").unwrap() + "token.".len();
    let decl = st.resolve_at(&path, pos);
    assert!(decl.is_some(), "totalSupply should resolve");
    assert_eq!(decl.unwrap().name, "totalSupply");

    // "Token" type reference in Factory should resolve
    let token_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "Token" && r.resolved.is_some())
        .collect();
    assert!(
        !token_refs.is_empty(),
        "Token references in other contracts should resolve"
    );
}

// ---------------------------------------------------------------------------
// 15. Interface with events and errors
// ---------------------------------------------------------------------------
#[test]
fn test_edge_interface_with_events_and_errors() {
    let source = r#"
interface IVault {
    event Deposit(address indexed user, uint256 amount);
    event Withdrawal(address indexed user, uint256 amount);
    error InsufficientBalance(uint256 available, uint256 required);

    function deposit() external payable;
    function withdraw(uint256 amount) external;
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let iface = fi
        .declarations
        .values()
        .find(|d| d.name == "IVault" && d.kind == DeclKind::Interface)
        .expect("IVault interface should be declared");
    // Members should include events, errors, and functions
    let members = iface.members();
    let member_names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
    assert!(
        member_names.contains(&"Deposit"),
        "members: {member_names:?}"
    );
    assert!(
        member_names.contains(&"Withdrawal"),
        "members: {member_names:?}"
    );
    assert!(
        member_names.contains(&"InsufficientBalance"),
        "members: {member_names:?}"
    );
    assert!(
        member_names.contains(&"deposit"),
        "members: {member_names:?}"
    );
    assert!(
        member_names.contains(&"withdraw"),
        "members: {member_names:?}"
    );
}

// ---------------------------------------------------------------------------
// 16. Library with struct parameters
// ---------------------------------------------------------------------------
#[test]
fn test_edge_library_with_struct_param() {
    let source = r#"
library PointLib {
    struct Point { uint256 x; uint256 y; }
    function add(Point memory a, Point memory b) internal pure returns (Point memory) {
        return Point({x: a.x + b.x, y: a.y + b.y});
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let lib = fi
        .declarations
        .values()
        .find(|d| d.name == "PointLib" && d.kind == DeclKind::Library)
        .expect("PointLib library should be declared");
    assert!(!lib.members().is_empty(), "Library should have members");

    let add_fn = fi
        .declarations
        .values()
        .find(|d| d.name == "add" && d.kind == DeclKind::Function)
        .expect("add function should be declared");
    let params = add_fn.parameters();
    assert_eq!(params.len(), 2, "add should have 2 parameters");
}

// ---------------------------------------------------------------------------
// 17. Constant and immutable state variables
// ---------------------------------------------------------------------------
#[test]
fn test_edge_constant_and_immutable_state_vars() {
    let source = r#"
contract Foo {
    uint256 public constant MAX_SUPPLY = 1000000;
    address public immutable owner;
    constructor() {
        owner = msg.sender;
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let max_supply = fi
        .declarations
        .values()
        .find(|d| d.name == "MAX_SUPPLY")
        .expect("MAX_SUPPLY should be declared");
    assert!(
        max_supply.is_constant,
        "MAX_SUPPLY should be marked constant"
    );

    let owner_decl = fi
        .declarations
        .values()
        .find(|d| d.name == "owner" && d.kind == DeclKind::StateVariable)
        .expect("owner should be declared");
    assert!(owner_decl.is_immutable, "owner should be marked immutable");
}

// ---------------------------------------------------------------------------
// 18. Empty contract/interface/library bodies
// ---------------------------------------------------------------------------
#[test]
fn test_edge_empty_bodies() {
    let source = r#"
contract EmptyContract {}
interface EmptyInterface {}
library EmptyLibrary {}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let ec = fi
        .declarations
        .values()
        .find(|d| d.name == "EmptyContract" && d.kind == DeclKind::Contract);
    assert!(ec.is_some(), "Empty contract should be declared");
    assert_eq!(
        ec.unwrap().members().len(),
        0,
        "Empty contract should have no members"
    );

    let ei = fi
        .declarations
        .values()
        .find(|d| d.name == "EmptyInterface" && d.kind == DeclKind::Interface);
    assert!(ei.is_some(), "Empty interface should be declared");

    let el = fi
        .declarations
        .values()
        .find(|d| d.name == "EmptyLibrary" && d.kind == DeclKind::Library);
    assert!(el.is_some(), "Empty library should be declared");
}

// ---------------------------------------------------------------------------
// 19. Enum values used as qualified access (Status.Active)
// ---------------------------------------------------------------------------
#[test]
fn test_edge_enum_qualified_access() {
    let source = r#"
contract Foo {
    enum Status { Pending, Active, Closed }
    function isActive(Status s) public pure returns (bool) {
        return s == Status.Active;
    }
    function defaultStatus() public pure returns (Status) {
        return Status.Pending;
    }
}
"#;
    let (st, path) = index(source);

    // "Active" in "Status.Active" should resolve
    let active_pos = source.find("Status.Active").unwrap() + "Status.".len();
    let decl = st.resolve_at(&path, active_pos);
    assert!(decl.is_some(), "Active should resolve via Status.Active");
    assert_eq!(decl.unwrap().name, "Active");

    // "Pending" in "Status.Pending" should resolve
    let pending_pos = source.find("Status.Pending").unwrap() + "Status.".len();
    let decl = st.resolve_at(&path, pending_pos);
    assert!(decl.is_some(), "Pending should resolve via Status.Pending");
    assert_eq!(decl.unwrap().name, "Pending");
}

// ---------------------------------------------------------------------------
// 20. Complex inheritance with multiple bases (A is B, C, D)
// ---------------------------------------------------------------------------
#[test]
fn test_edge_multiple_base_contracts() {
    let source = r#"
contract Ownable {
    function owner() public pure returns (address) {
        return address(0);
    }
}
contract Pausable {
    function paused() public pure returns (bool) {
        return false;
    }
}
contract Token is Ownable, Pausable {
    function doSomething() public pure returns (address) {
        return owner();
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let token = fi
        .declarations
        .values()
        .find(|d| d.name == "Token" && d.kind == DeclKind::Contract)
        .expect("Token should be declared");
    let bases = token.base_contracts();
    assert_eq!(bases.len(), 2, "Token should have 2 base contracts");
    assert!(bases.contains(&"Ownable".to_string()));
    assert!(bases.contains(&"Pausable".to_string()));

    // "owner" call in Token should resolve via Ownable inheritance
    let owner_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "owner" && r.resolved.is_some())
        .collect();
    assert!(
        !owner_refs.is_empty(),
        "owner() call in Token should resolve via Ownable inheritance"
    );
}

// ---------------------------------------------------------------------------
// 21. Abstract contracts
// ---------------------------------------------------------------------------
#[test]
fn test_edge_abstract_contract() {
    let source = r#"
abstract contract AbstractBase {
    function doWork() public virtual returns (uint256);
    function helper() internal pure returns (uint256) {
        return 42;
    }
}
contract Concrete is AbstractBase {
    function doWork() public override returns (uint256) {
        return helper();
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let abstract_base = fi
        .declarations
        .values()
        .find(|d| d.name == "AbstractBase" && d.kind == DeclKind::Contract);
    assert!(
        abstract_base.is_some(),
        "Abstract contract should be declared"
    );

    // "helper" in Concrete should resolve via inheritance
    let helper_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "helper" && r.resolved.is_some())
        .collect();
    assert!(
        !helper_refs.is_empty(),
        "helper() in Concrete should resolve via AbstractBase inheritance"
    );
}

// ---------------------------------------------------------------------------
// 22. Struct with mapping member
// ---------------------------------------------------------------------------
#[test]
fn test_edge_struct_with_mapping_member() {
    let source = r#"
contract Foo {
    struct Account {
        uint256 balance;
        mapping(address => uint256) allowances;
    }
    mapping(address => Account) public accounts;
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let account = fi
        .declarations
        .values()
        .find(|d| d.name == "Account" && d.kind == DeclKind::Struct)
        .expect("Account struct should be declared");
    let members = account.members();
    let member_names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
    assert!(
        member_names.contains(&"balance"),
        "Account should have balance member"
    );
    assert!(
        member_names.contains(&"allowances"),
        "Account should have allowances member"
    );
}

// ---------------------------------------------------------------------------
// 23. Deeply nested structs (struct has struct field)
// ---------------------------------------------------------------------------
#[test]
fn test_edge_deeply_nested_structs() {
    let source = r#"
contract Foo {
    struct Inner { uint256 value; }
    struct Middle { Inner inner; }
    struct Outer { Middle middle; }

    Outer public data;

    function getDeep() public view returns (uint256) {
        return data.middle.inner.value;
    }
}
"#;
    let (st, path) = index(source);

    // "middle" in "data.middle.inner.value" should resolve
    let mid_pos = source.find("data.middle.inner").unwrap() + "data.".len();
    let decl = st.resolve_at(&path, mid_pos);
    assert!(decl.is_some(), "middle should resolve as member of Outer");
    assert_eq!(decl.unwrap().name, "middle");

    // "inner" should resolve as member of Middle
    let inner_pos = source.find("data.middle.inner").unwrap() + "data.middle.".len();
    let decl = st.resolve_at(&path, inner_pos);
    assert!(decl.is_some(), "inner should resolve as member of Middle");
    assert_eq!(decl.unwrap().name, "inner");

    // "value" should resolve as member of Inner
    let value_pos = source.find(".inner.value").unwrap() + ".inner.".len();
    let decl = st.resolve_at(&path, value_pos);
    assert!(decl.is_some(), "value should resolve as member of Inner");
    assert_eq!(decl.unwrap().name, "value");
}

// ---------------------------------------------------------------------------
// 24. Event with indexed parameters
// ---------------------------------------------------------------------------
#[test]
fn test_edge_event_indexed_parameters() {
    let source = r#"
contract Foo {
    event Transfer(address indexed from, address indexed to, uint256 amount);
    function transfer(address to, uint256 amount) public {
        emit Transfer(msg.sender, to, amount);
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let event = fi
        .declarations
        .values()
        .find(|d| d.name == "Transfer" && d.kind == DeclKind::Event)
        .expect("Transfer event should be declared");
    let params = event.parameters();
    assert_eq!(params.len(), 3, "Transfer event should have 3 params");
    assert_eq!(params[0].1, "from");
    assert_eq!(params[1].1, "to");
    assert_eq!(params[2].1, "amount");

    // "Transfer" in emit should resolve
    let pos = source.find("emit Transfer(").unwrap() + "emit ".len();
    let decl = st.resolve_at(&path, pos);
    assert!(decl.is_some(), "Transfer in emit should resolve");
    assert_eq!(decl.unwrap().name, "Transfer");
    assert_eq!(decl.unwrap().kind, DeclKind::Event);
}

// ---------------------------------------------------------------------------
// 25. Error with parameters
// ---------------------------------------------------------------------------
#[test]
fn test_edge_error_with_parameters() {
    let source = r#"
contract Foo {
    error TransferFailed(address from, address to, uint256 amount);
    function doTransfer(address to, uint256 amount) public view {
        revert TransferFailed(msg.sender, to, amount);
    }
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    let error = fi
        .declarations
        .values()
        .find(|d| d.name == "TransferFailed" && d.kind == DeclKind::Error)
        .expect("TransferFailed error should be declared");
    let params = error.parameters();
    assert_eq!(params.len(), 3, "Error should have 3 parameters");
    assert_eq!(params[0], ("address".to_string(), "from".to_string()));
    assert_eq!(params[1], ("address".to_string(), "to".to_string()));
    assert_eq!(params[2], ("uint256".to_string(), "amount".to_string()));

    // "TransferFailed" in revert should resolve
    let pos = source.find("revert TransferFailed(").unwrap() + "revert ".len();
    let decl = st.resolve_at(&path, pos);
    assert!(decl.is_some(), "TransferFailed in revert should resolve");
    assert_eq!(decl.unwrap().kind, DeclKind::Error);
}

// ---------------------------------------------------------------------------
// 26. Modifier body with require
// ---------------------------------------------------------------------------
#[test]
fn test_edge_modifier_body_with_require() {
    let source = r#"
contract Foo {
    address public owner;
    uint256 public threshold;
    modifier onlyOwnerAbove(uint256 minVal) {
        require(msg.sender == owner);
        require(minVal > threshold);
        _;
    }
    function doWork() public onlyOwnerAbove(10) {}
}
"#;
    let (st, path) = index(source);
    let fi = get_fi(&st, &path);

    // "owner" inside modifier body should resolve to the state variable
    let owner_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "owner" && r.resolved.is_some())
        .collect();
    assert!(
        !owner_refs.is_empty(),
        "owner reference inside modifier body should resolve"
    );

    // "threshold" inside modifier body should resolve
    let threshold_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "threshold" && r.resolved.is_some())
        .collect();
    assert!(
        !threshold_refs.is_empty(),
        "threshold reference inside modifier body should resolve"
    );

    // "minVal" inside modifier body should resolve to the modifier parameter
    let minval_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(source) == "minVal" && r.resolved.is_some())
        .collect();
    assert!(
        !minval_refs.is_empty(),
        "minVal parameter reference inside modifier body should resolve"
    );
}

// ---------------------------------------------------------------------------
// 27. Cross-file import and member access resolution
// ---------------------------------------------------------------------------
#[test]
fn test_edge_cross_file_import_member_access() {
    let tmp = tempfile::tempdir().unwrap();
    let types_path = tmp.path().join("Types.sol");
    let main_path = tmp.path().join("Main.sol");

    let types_source = r#"
pragma solidity ^0.8.0;
struct Position {
    int256 x;
    int256 y;
}
enum Direction { Up, Down, Left, Right }
"#;
    let main_source = r#"
pragma solidity ^0.8.0;
import {Position, Direction} from "./Types.sol";
contract Game {
    Position public pos;
    Direction public dir;
    function moveUp() public {
        pos.x = pos.x + 1;
        dir = Direction.Up;
    }
}
"#;
    std::fs::write(&types_path, types_source).unwrap();
    std::fs::write(&main_path, main_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&types_path, types_source, &mut parser);
    st.resolve_file_references(&types_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // "x" in "pos.x" should resolve to Position.x
    let x_pos = main_source.find("pos.x = pos").unwrap() + "pos.".len();
    let decl = st.resolve_at(&main_path, x_pos);
    assert!(
        decl.is_some(),
        "x in pos.x should resolve to struct field from imported Types.sol"
    );
    assert_eq!(decl.unwrap().name, "x");

    // "Up" in "Direction.Up" should resolve to enum value
    let up_pos = main_source.find("Direction.Up").unwrap() + "Direction.".len();
    let decl = st.resolve_at(&main_path, up_pos);
    assert!(
        decl.is_some(),
        "Up in Direction.Up should resolve via cross-file enum import"
    );
    assert_eq!(decl.unwrap().name, "Up");
}

// ---------------------------------------------------------------------------
// 28. Named import resolution
// ---------------------------------------------------------------------------
#[test]
fn test_edge_named_import_resolution() {
    let tmp = tempfile::tempdir().unwrap();
    let token_path = tmp.path().join("Token.sol");
    let main_path = tmp.path().join("Main.sol");

    let token_source = r#"
pragma solidity ^0.8.0;
contract Token {
    uint256 public supply;
    function mint(uint256 amount) public {
        supply = supply + amount;
    }
}
contract Helper {
    function noop() public pure {}
}
"#;
    let main_source = r#"
pragma solidity ^0.8.0;
import {Token} from "./Token.sol";
contract Main {
    Token public token;
    function doMint() public {
        token.mint(100);
    }
}
"#;
    std::fs::write(&token_path, token_source).unwrap();
    std::fs::write(&main_path, main_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&token_path, token_source, &mut parser);
    st.resolve_file_references(&token_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // "Token" type reference should resolve
    let fi = get_fi(&st, &main_path);
    let token_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(main_source) == "Token" && r.resolved.is_some())
        .collect();
    assert!(!token_refs.is_empty(), "Token named import should resolve");

    // "mint" in "token.mint(100)" should resolve
    let mint_pos = main_source.find("token.mint(100)").unwrap() + "token.".len();
    let decl = st.resolve_at(&main_path, mint_pos);
    assert!(decl.is_some(), "mint should resolve via named import");
    assert_eq!(decl.unwrap().name, "mint");
}

// ---------------------------------------------------------------------------
// 29. Alias import resolution
// ---------------------------------------------------------------------------
#[test]
fn test_edge_alias_import_resolution() {
    let tmp = tempfile::tempdir().unwrap();
    let lib_path = tmp.path().join("MathLib.sol");
    let main_path = tmp.path().join("Main.sol");

    let lib_source = r#"
pragma solidity ^0.8.0;
library MathLib {
    function square(uint256 x) internal pure returns (uint256) {
        return x * x;
    }
}
"#;
    let main_source = r#"
pragma solidity ^0.8.0;
import "./MathLib.sol" as ML;
contract Main {
    function calc(uint256 v) public pure returns (uint256) {
        return ML.MathLib.square(v);
    }
}
"#;
    std::fs::write(&lib_path, lib_source).unwrap();
    std::fs::write(&main_path, main_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&lib_path, lib_source, &mut parser);
    st.resolve_file_references(&lib_path, &mut parser);
    st.index_file(&main_path, main_source, &mut parser);
    st.resolve_file_references(&main_path, &mut parser);

    // "ML" should resolve as an import alias
    let fi = get_fi(&st, &main_path);
    let ml_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(main_source) == "ML" && r.resolved.is_some())
        .collect();
    assert!(
        !ml_refs.is_empty(),
        "ML alias import reference should resolve"
    );
}

// ---------------------------------------------------------------------------
// 30. Re-export chains (UNIMPLEMENTED)
// ---------------------------------------------------------------------------
#[test]
fn test_edge_reexport_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let a_path = tmp.path().join("A.sol");
    let b_path = tmp.path().join("B.sol");
    let c_path = tmp.path().join("C.sol");

    let a_source = r#"
pragma solidity ^0.8.0;
struct Foo { uint256 val; }
"#;
    let b_source = r#"
pragma solidity ^0.8.0;
import {Foo} from "./A.sol";
"#;
    let c_source = r#"
pragma solidity ^0.8.0;
import {Foo} from "./B.sol";
contract Bar {
    Foo public f;
}
"#;
    std::fs::write(&a_path, a_source).unwrap();
    std::fs::write(&b_path, b_source).unwrap();
    std::fs::write(&c_path, c_source).unwrap();

    let mut parser = TsParser::new();
    let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
    let mut st = SymbolTable::new(resolver);

    st.index_file(&a_path, a_source, &mut parser);
    st.resolve_file_references(&a_path, &mut parser);
    st.index_file(&b_path, b_source, &mut parser);
    st.resolve_file_references(&b_path, &mut parser);
    st.index_file(&c_path, c_source, &mut parser);
    st.resolve_file_references(&c_path, &mut parser);

    // "Foo" in C.sol should resolve despite being re-exported through B.sol
    let fi = get_fi(&st, &c_path);
    let foo_refs: Vec<_> = fi
        .references
        .iter()
        .filter(|r| r.name(c_source) == "Foo" && r.resolved.is_some())
        .collect();
    assert!(
        !foo_refs.is_empty(),
        "Foo should resolve via re-export chain A -> B -> C"
    );
}
