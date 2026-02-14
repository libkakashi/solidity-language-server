; Detect immutable variable names that are not SCREAMING_SNAKE_CASE
; Rust filter checks for `immutable` node in the declaration
(state_variable_declaration
  (immutable)
  name: (identifier) @name) @decl
