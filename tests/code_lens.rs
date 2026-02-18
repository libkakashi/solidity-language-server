use std::path::PathBuf;

use solidity_language_server::code_lens::code_lens;
use solidity_language_server::import_resolver::ImportResolver;
use solidity_language_server::parser::TsParser;
use solidity_language_server::symbol_table::SymbolTable;
use solidity_language_server::utils::LineIndex;

fn setup(source: &str) -> (SymbolTable, PathBuf) {
    let mut parser = TsParser::new();
    let path = PathBuf::from("/tmp/test.sol");
    let resolver = ImportResolver::with_root(PathBuf::from("/tmp"));
    let mut st = SymbolTable::new(resolver);
    st.index_file(&path, source, &mut parser);
    st.resolve_file_references(&path, &mut parser);
    (st, path)
}

// ---------------------------------------------------------------------------
// 1. Function with references
// ---------------------------------------------------------------------------

#[test]
fn function_with_references_shows_count() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Calculator {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }

    function compute() public pure returns (uint256) {
        return add(1, 2) + add(3, 4);
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let add_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let cmd = l.command.as_ref().unwrap();
            cmd.title.contains("references") && cmd.command == "solidity.showReferences"
        })
        .filter(|l| {
            // The lens sits on the "add" function declaration line
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("function add(")
        })
        .collect();

    assert_eq!(
        add_lens.len(),
        1,
        "Expected exactly one references lens for 'add'"
    );
    let title = &add_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "2 references",
        "Expected '2 references' for add(), got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 2. Function with zero references
// ---------------------------------------------------------------------------

#[test]
fn function_with_zero_references() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Unused {
    function neverCalled() public pure returns (uint256) {
        return 42;
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let fn_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("function neverCalled")
        })
        .collect();

    assert!(!fn_lens.is_empty(), "Should have a lens for neverCalled");
    let title = &fn_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "0 references",
        "Expected '0 references' for unused function, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 3. State variable with references
// ---------------------------------------------------------------------------

#[test]
fn state_variable_with_references() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public totalSupply;

    function mint(uint256 amount) public {
        totalSupply += amount;
    }

    function burn(uint256 amount) public {
        totalSupply -= amount;
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let supply_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("totalSupply")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        supply_lens.len(),
        1,
        "Expected one references lens for totalSupply"
    );
    let title = &supply_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "2 references",
        "Expected '2 references' for totalSupply, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 4. Event with references (emit counts)
// ---------------------------------------------------------------------------

#[test]
fn event_with_references_from_emit() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ledger {
    event Transfer(address indexed from, address indexed to, uint256 amount);

    function send(address to, uint256 amount) external {
        emit Transfer(msg.sender, to, amount);
    }

    function batchSend(address to, uint256 a1, uint256 a2) external {
        emit Transfer(msg.sender, to, a1);
        emit Transfer(msg.sender, to, a2);
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let event_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("event Transfer")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        event_lens.len(),
        1,
        "Expected one references lens for Transfer event"
    );
    let title = &event_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "3 references",
        "Expected '3 references' for Transfer event, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 5. Error with references (revert counts)
// ---------------------------------------------------------------------------

#[test]
fn error_with_references_from_revert() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Guarded {
    error Unauthorized(address caller);

    function restricted() public view {
        revert Unauthorized(msg.sender);
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let err_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("error Unauthorized")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        err_lens.len(),
        1,
        "Expected one references lens for Unauthorized error"
    );
    let title = &err_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "1 reference",
        "Expected '1 reference' for Unauthorized error, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 6. Contract with references
// ---------------------------------------------------------------------------

#[test]
fn contract_with_references() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Token {
    uint256 public supply;
}

contract Factory {
    function create() public returns (Token) {
        Token t = new Token();
        return t;
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let token_ref_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("contract Token")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        token_ref_lens.len(),
        1,
        "Expected one references lens for Token contract"
    );
    let title = &token_ref_lens[0].command.as_ref().unwrap().title;
    // Token is used in return type + variable type + new call -- at least some references
    assert!(
        title.contains("references") || title.contains("reference"),
        "Expected references count for Token contract, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 7. Struct with references
// ---------------------------------------------------------------------------

#[test]
fn struct_with_references() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Vault {
    struct Position {
        uint256 size;
        uint256 collateral;
    }

    function getPosition() external view returns (Position memory) {
        return Position(0, 0);
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let struct_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("struct Position")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        struct_lens.len(),
        1,
        "Expected one references lens for Position struct"
    );
    let title = &struct_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "2 references",
        "Expected '2 references' for Position struct, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 8. Enum with references
// ---------------------------------------------------------------------------

#[test]
fn enum_with_references() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract StateMachine {
    enum Status { Active, Paused, Stopped }

    Status public current;

    function getStatus() external view returns (Status) {
        return current;
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let enum_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("enum Status")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        enum_lens.len(),
        1,
        "Expected one references lens for Status enum"
    );
    let title = &enum_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "2 references",
        "Expected '2 references' for Status enum, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 9. Modifier with references
// ---------------------------------------------------------------------------

#[test]
fn modifier_with_references() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Access {
    modifier onlyOwner() {
        _;
    }

    function withdraw() public onlyOwner {
    }

    function pause() public onlyOwner {
    }

    function resume() external onlyOwner {
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let mod_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("modifier onlyOwner")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        mod_lens.len(),
        1,
        "Expected one references lens for onlyOwner modifier"
    );
    let title = &mod_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "3 references",
        "Expected '3 references' for onlyOwner, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 10. Interface with implementation count
// ---------------------------------------------------------------------------

#[test]
fn interface_with_implementation_count() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IERC20 {
    function totalSupply() external view returns (uint256);
}

contract TokenA is IERC20 {
    function totalSupply() external pure returns (uint256) {
        return 100;
    }
}

contract TokenB is IERC20 {
    function totalSupply() external pure returns (uint256) {
        return 200;
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let impl_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("interface IERC20")
                && l.command.as_ref().unwrap().command == "solidity.showImplementations"
        })
        .collect();

    assert_eq!(
        impl_lens.len(),
        1,
        "Expected one implementations lens for IERC20"
    );
    let title = &impl_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "2 implementations",
        "Expected '2 implementations' for IERC20, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 11. Contract implementation count (base contract shows implementation count)
// ---------------------------------------------------------------------------

#[test]
fn contract_base_shows_implementation_count() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Base {
    function baseFn() internal pure returns (uint256) {
        return 42;
    }
}

contract Derived is Base {
    function derivedFn() public pure returns (uint256) {
        return baseFn();
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let impl_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("contract Base")
                && l.command.as_ref().unwrap().command == "solidity.showImplementations"
        })
        .collect();

    assert_eq!(
        impl_lens.len(),
        1,
        "Expected one implementations lens for Base contract"
    );
    let title = &impl_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "1 implementation",
        "Expected '1 implementation' for Base, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 12. No implementations -- only references lens, no implementations lens
// ---------------------------------------------------------------------------

#[test]
fn no_implementations_only_references_lens() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IStandalone {
    function doSomething() external;
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let impl_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("interface IStandalone")
                && l.command.as_ref().unwrap().command == "solidity.showImplementations"
        })
        .collect();

    assert!(
        impl_lens.is_empty(),
        "Should have NO implementations lens for unimplemented interface"
    );

    let ref_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("interface IStandalone")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        ref_lens.len(),
        1,
        "Should have a references lens for IStandalone"
    );
    let title = &ref_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "0 references",
        "Expected '0 references' for unimplemented interface, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 13. Multiple functions each get their own lens
// ---------------------------------------------------------------------------

#[test]
fn multiple_functions_each_get_own_lens() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Multi {
    function alpha() public pure returns (uint256) {
        return 1;
    }

    function beta() public pure returns (uint256) {
        return 2;
    }

    function gamma() public pure returns (uint256) {
        return 3;
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let fn_ref_lenses: Vec<_> = lenses
        .iter()
        .filter(|l| l.command.as_ref().unwrap().command == "solidity.showReferences")
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("function alpha")
                || line_text.contains("function beta")
                || line_text.contains("function gamma")
        })
        .collect();

    assert_eq!(
        fn_ref_lenses.len(),
        3,
        "Expected 3 separate lenses for 3 functions, got {}",
        fn_ref_lenses.len()
    );

    // Each should show 0 references since none are called
    for lens in &fn_ref_lenses {
        let title = &lens.command.as_ref().unwrap().title;
        assert_eq!(
            title, "0 references",
            "Unused function should show '0 references', got '{title}'"
        );
    }
}

// ---------------------------------------------------------------------------
// 14. Constructor lens
// ---------------------------------------------------------------------------

#[test]
fn constructor_gets_lens() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Owned {
    address public owner;

    constructor(address _owner) {
        owner = _owner;
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let ctor_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("constructor")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        ctor_lens.len(),
        1,
        "Expected one references lens for constructor"
    );
    let title = &ctor_lens[0].command.as_ref().unwrap().title;
    assert!(
        title.contains("references") || title.contains("reference"),
        "Constructor lens should show references count, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 15. Library lens
// ---------------------------------------------------------------------------

#[test]
fn library_gets_lens() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

contract Calculator {
    function compute() public pure returns (uint256) {
        return MathLib.add(1, 2);
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let lib_ref_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("library MathLib")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        lib_ref_lens.len(),
        1,
        "Expected one references lens for MathLib library"
    );
    let title = &lib_ref_lens[0].command.as_ref().unwrap().title;
    assert!(
        title.contains("reference"),
        "Library lens should show references, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 16. Lenses sorted by position
// ---------------------------------------------------------------------------

#[test]
fn lenses_sorted_by_position() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Ordered {
    uint256 public alpha;
    uint256 public beta;

    event Gamma(uint256 val);

    function delta() public {}
    function epsilon() public {}
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    assert!(
        lenses.len() >= 2,
        "Should have multiple lenses to verify ordering"
    );

    for i in 1..lenses.len() {
        let prev_line = lenses[i - 1].range.start.line;
        let prev_col = lenses[i - 1].range.start.character;
        let curr_line = lenses[i].range.start.line;
        let curr_col = lenses[i].range.start.character;
        assert!(
            (curr_line, curr_col) >= (prev_line, prev_col),
            "Lenses should be sorted by position: lens[{}] ({},{}) should come after lens[{}] ({},{})",
            i,
            curr_line,
            curr_col,
            i - 1,
            prev_line,
            prev_col
        );
    }
}

// ---------------------------------------------------------------------------
// 17. Single reference -- singular "1 reference" text
// ---------------------------------------------------------------------------

#[test]
fn single_reference_singular_text() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Singular {
    function helper() internal pure returns (uint256) {
        return 1;
    }

    function caller() public pure returns (uint256) {
        return helper();
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let helper_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("function helper")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(helper_lens.len(), 1);
    let title = &helper_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "1 reference",
        "Expected singular '1 reference', got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 18. Single implementation -- singular "1 implementation" text
// ---------------------------------------------------------------------------

#[test]
fn single_implementation_singular_text() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IVault {
    function deposit() external;
}

contract Vault is IVault {
    function deposit() external {}
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let impl_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("interface IVault")
                && l.command.as_ref().unwrap().command == "solidity.showImplementations"
        })
        .collect();

    assert_eq!(
        impl_lens.len(),
        1,
        "Expected one implementations lens for IVault"
    );
    let title = &impl_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "1 implementation",
        "Expected singular '1 implementation', got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 19. Complex contract -- many declarations each have lenses
// ---------------------------------------------------------------------------

#[test]
fn complex_contract_all_declarations_have_lenses() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Complex {
    struct Data {
        uint256 value;
    }

    enum Phase { Init, Running, Done }

    event DataUpdated(uint256 newValue);
    error InvalidData(uint256 provided);

    uint256 public counter;
    Phase public currentPhase;

    modifier onlyInit() {
        _;
    }

    constructor() {
        counter = 0;
    }

    function increment() public onlyInit {
        counter += 1;
        emit DataUpdated(counter);
    }

    function validate(uint256 x) internal pure {
        if (x == 0) revert InvalidData(x);
    }

    function getData() external view returns (Data memory) {
        return Data(counter);
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    // Collect all reference lens titles
    let ref_lenses: Vec<_> = lenses
        .iter()
        .filter(|l| l.command.as_ref().unwrap().command == "solidity.showReferences")
        .collect();

    // We expect lenses for: Complex (contract), Data (struct), Phase (enum),
    // DataUpdated (event), InvalidData (error), counter (state var),
    // currentPhase (state var), onlyInit (modifier), constructor,
    // increment (function), validate (function), getData (function)
    // That is at least 12 reference lenses
    assert!(
        ref_lenses.len() >= 10,
        "Expected at least 10 reference lenses for complex contract, got {}",
        ref_lenses.len()
    );

    // Verify specific declarations got lenses by checking line content
    let expected_declarations = [
        "contract Complex",
        "struct Data",
        "enum Phase",
        "event DataUpdated",
        "error InvalidData",
        "modifier onlyInit",
        "function increment",
        "function validate",
        "function getData",
    ];

    for expected in &expected_declarations {
        let matching: Vec<_> = ref_lenses
            .iter()
            .filter(|l| {
                let line = l.range.start.line as usize;
                let line_text = source.lines().nth(line).unwrap_or("");
                line_text.contains(expected)
            })
            .collect();

        assert!(
            !matching.is_empty(),
            "Expected a references lens for declaration containing '{expected}'"
        );
    }

    // Check that used items have non-zero counts
    // DataUpdated is emitted once
    let data_updated_lens = ref_lenses
        .iter()
        .find(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("event DataUpdated")
        })
        .expect("DataUpdated lens must exist");
    let title = &data_updated_lens.command.as_ref().unwrap().title;
    assert_eq!(
        title, "1 reference",
        "DataUpdated emitted once, expected '1 reference', got '{title}'"
    );

    // InvalidData is reverted once
    let invalid_data_lens = ref_lenses
        .iter()
        .find(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("error InvalidData")
        })
        .expect("InvalidData lens must exist");
    let title = &invalid_data_lens.command.as_ref().unwrap().title;
    assert_eq!(
        title, "1 reference",
        "InvalidData reverted once, expected '1 reference', got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 20. Local variables excluded -- no lenses for locals/params
// ---------------------------------------------------------------------------

#[test]
fn local_variables_excluded_from_lenses() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Locals {
    function compute(uint256 param1, uint256 param2) public pure returns (uint256) {
        uint256 localA = param1 + param2;
        uint256 localB = localA * 2;
        return localB;
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    // No lens should sit on a line that only declares local variables
    for lens in &lenses {
        let title = &lens.command.as_ref().unwrap().title;
        let line = lens.range.start.line as usize;
        let line_text = source.lines().nth(line).unwrap_or("");

        // Lines with local variable declarations should not have lenses
        assert!(
            !line_text.trim_start().starts_with("uint256 localA")
                && !line_text.trim_start().starts_with("uint256 localB"),
            "Local variable should NOT get a lens, but found '{title}' on line: '{}'",
            line_text.trim()
        );
    }

    // We should still have lenses for the contract and the function
    let ref_lenses: Vec<_> = lenses
        .iter()
        .filter(|l| l.command.as_ref().unwrap().command == "solidity.showReferences")
        .collect();

    assert!(
        ref_lenses.len() >= 2,
        "Expected at least 2 lenses (contract + function), got {}",
        ref_lenses.len()
    );
}

// ---------------------------------------------------------------------------
// 21. Empty file produces no lenses
// ---------------------------------------------------------------------------

#[test]
fn file_with_no_important_declarations_produces_no_function_lenses() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    // There should be no lenses for functions, events, errors, contracts, etc.
    let important_lenses: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("function")
                || line_text.contains("event")
                || line_text.contains("error")
                || line_text.contains("contract")
                || line_text.contains("struct")
                || line_text.contains("enum")
        })
        .collect();

    assert!(
        important_lenses.is_empty(),
        "File with no contract/function declarations should produce no relevant lenses, got {:?}",
        important_lenses
            .iter()
            .map(|l| &l.command.as_ref().unwrap().title)
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 22. Unknown file path produces no lenses
// ---------------------------------------------------------------------------

#[test]
fn unknown_file_produces_no_lenses() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {}
"#;
    let (st, _path) = setup(source);
    let li = LineIndex::new(source);
    let unknown_path = PathBuf::from("/tmp/unknown.sol");
    let lenses = code_lens(&st, &unknown_path, source, &li);

    assert!(lenses.is_empty(), "Unknown file should produce no lenses");
}

// ---------------------------------------------------------------------------
// 23. Multiple contracts in same file
// ---------------------------------------------------------------------------

#[test]
fn multiple_contracts_in_same_file() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    function doA() public pure returns (uint256) {
        return 1;
    }
}

contract B {
    function doB() public pure returns (uint256) {
        return 2;
    }
}

contract C {
    function doC() public pure returns (uint256) {
        return 3;
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let contract_lenses: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.starts_with("contract ")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        contract_lenses.len(),
        3,
        "Expected 3 contract reference lenses (A, B, C), got {}",
        contract_lenses.len()
    );
}

// ---------------------------------------------------------------------------
// 24. Interface implementation and references both present
// ---------------------------------------------------------------------------

#[test]
fn interface_has_both_references_and_implementations_lenses() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IToken {
    function mint() external;
}

contract Token is IToken {
    function mint() external {}
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let itoken_lenses: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("interface IToken")
        })
        .collect();

    // IToken should have both a references lens and an implementations lens
    let ref_lens: Vec<_> = itoken_lenses
        .iter()
        .filter(|l| l.command.as_ref().unwrap().command == "solidity.showReferences")
        .collect();
    let impl_lens: Vec<_> = itoken_lenses
        .iter()
        .filter(|l| l.command.as_ref().unwrap().command == "solidity.showImplementations")
        .collect();

    assert_eq!(
        ref_lens.len(),
        1,
        "IToken should have exactly 1 references lens"
    );
    assert_eq!(
        impl_lens.len(),
        1,
        "IToken should have exactly 1 implementations lens"
    );
}

// ---------------------------------------------------------------------------
// 25. Lens command field is correctly set
// ---------------------------------------------------------------------------

#[test]
fn lens_command_field_is_correct() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IBase {
    function run() external;
}

contract Impl is IBase {
    function run() external {}
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    for lens in &lenses {
        let cmd = lens
            .command
            .as_ref()
            .expect("Every lens should have a command");
        assert!(
            cmd.command == "solidity.showReferences"
                || cmd.command == "solidity.showImplementations",
            "Unexpected command: '{}'",
            cmd.command
        );
        assert!(!cmd.title.is_empty(), "Command title should not be empty");
    }
}

// ---------------------------------------------------------------------------
// 26. Free function gets a lens
// ---------------------------------------------------------------------------

#[test]
fn free_function_gets_lens() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

function freeHelper() pure returns (uint256) {
    return 42;
}

contract User {
    function use_it() public pure returns (uint256) {
        return freeHelper();
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let free_fn_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("function freeHelper")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .collect();

    assert_eq!(
        free_fn_lens.len(),
        1,
        "Expected one references lens for free function"
    );
    let title = &free_fn_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "1 reference",
        "Expected '1 reference' for freeHelper, got '{title}'"
    );
}

// ---------------------------------------------------------------------------
// 27. Lens range points to declaration name
// ---------------------------------------------------------------------------

#[test]
fn lens_range_points_to_declaration_name() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract MyContract {
    function myFunction() public pure returns (uint256) {
        return 0;
    }
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    // Find the lens for myFunction
    let fn_lens = lenses
        .iter()
        .find(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("function myFunction")
                && l.command.as_ref().unwrap().command == "solidity.showReferences"
        })
        .expect("Should find lens for myFunction");

    // The range should be on the same line as the function declaration
    let line_text = source
        .lines()
        .nth(fn_lens.range.start.line as usize)
        .unwrap();
    assert!(
        line_text.contains("myFunction"),
        "Lens range line should contain the function name"
    );
}

// ---------------------------------------------------------------------------
// 28. Multiple implementations shown correctly
// ---------------------------------------------------------------------------

#[test]
fn multiple_implementations_shown_correctly() {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IHandler {
    function handle() external;
}

contract HandlerA is IHandler {
    function handle() external {}
}

contract HandlerB is IHandler {
    function handle() external {}
}

contract HandlerC is IHandler {
    function handle() external {}
}
"#;
    let (st, path) = setup(source);
    let li = LineIndex::new(source);
    let lenses = code_lens(&st, &path, source, &li);

    let impl_lens: Vec<_> = lenses
        .iter()
        .filter(|l| {
            let line = l.range.start.line as usize;
            let line_text = source.lines().nth(line).unwrap_or("");
            line_text.contains("interface IHandler")
                && l.command.as_ref().unwrap().command == "solidity.showImplementations"
        })
        .collect();

    assert_eq!(impl_lens.len(), 1);
    let title = &impl_lens[0].command.as_ref().unwrap().title;
    assert_eq!(
        title, "3 implementations",
        "Expected '3 implementations' for IHandler, got '{title}'"
    );
}
