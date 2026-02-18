# Solidity LSP Feature Status

## Legend
- **Y** = Fully Implemented
- **P** = Partially Implemented
- **N** = Not Implemented

## LSP Protocol Features

| Feature | This Server | Hardhat VSCode | Priority |
|---|---|---|---|
| Text Document Sync | **Y** (Full) | **Y** (Incremental) | - |
| Completion | **Y** | **Y** | - |
| Completion Resolve | **N** | **N** | Low |
| Signature Help | **Y** | **Y** | ~~High~~ Done |
| Hover | **Y** | **Y** | - |
| Go to Definition | **Y** | **Y** | - |
| Go to Declaration | **Y** | **N** | - |
| Go to Type Definition | **N** | **Y** | **High** |
| Go to Implementation | **N** | **Y** | **High** |
| Find References | **Y** | **Y** | - |
| Document Symbols | **Y** | **Y** | - |
| Workspace Symbols | **Y** | **N** | - |
| Code Actions / Quick Fixes | **Y** (7 fixes) | **Y** (11 fixes) | ~~Critical~~ Done |
| Code Lens | **N** | **N** | Medium |
| Document Formatting | **Y** | **Y** | - |
| Range Formatting | **N** | **N** | Low |
| Rename / Prepare Rename | **Y** | **Y** | - |
| Document Highlight | **N** | **N** | Medium |
| Document Links | **Y** | **N** | - |
| Folding Ranges | **N** | **N** | Medium |
| Selection Ranges | **N** | **N** | Low |
| Semantic Tokens | **Y** (13 types) | **Y** | ~~High~~ Done |
| Inlay Hints | **N** | **N** | **High** |
| Call Hierarchy | **N** | **N** | Medium |
| Type Hierarchy | **N** | **N** | Medium |
| Workspace Folders | **P** (empty handler) | **Y** | Medium |
| Workspace Configuration | **P** (empty handler) | **Y** | Medium |

## Implementation Plan (ordered by priority)

- [x] 1. Code Actions / Quick Fixes — 7 lint-driven auto-fixes
- [x] 2. Signature Help — parameter tracking on `(` and `,`
- [x] 3. Semantic Tokens — 13 token types, 4 modifiers
- [ ] 4. Go to Type Definition — variable → type navigation
- [ ] 5. Go to Implementation — interface → concrete impl
- [ ] 6. Inlay Hints — parameter names, type annotations
- [ ] 7. Document Highlight — same-symbol highlighting
- [ ] 8. Folding Ranges — collapse functions, contracts, blocks
- [ ] 9. Call Hierarchy — incoming/outgoing call navigation
- [ ] 10. Type Hierarchy — contract inheritance tree
- [ ] 11. Code Lens — reference/implementation counts
- [ ] 12. Selection Ranges — smart expand/shrink selection

## Solidity-Specific Features

| Feature | This Server | Hardhat VSCode |
|---|---|---|
| Compiler integration | Solar (Rust) | solc |
| Import resolution | **Y** | **Y** |
| Remapping support | **Y** | **Y** |
| Framework: Hardhat | **P** | **Y** |
| Framework: Foundry | **Y** | **Y** |
| Framework: Truffle | **P** | **Y** |
| NatSpec in hover | **Y** | **Y** |
| NatSpec completion | **N** | **Y** |
| Contract inheritance | **Y** | **Y** |
| Using-for resolution | **Y** | **P** |
| Custom lint rules (12) | **Y** | **N** |
| Formatting (built-in) | **Y** | **Y** (Prettier/Forge) |
