; Detect constant variable names that are not SCREAMING_SNAKE_CASE
; Rust filter checks for `constant` keyword in the declaration
(state_variable_declaration
  name: (identifier) @name) @decl
