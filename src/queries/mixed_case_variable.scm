; Detect mutable state variable names that are not mixedCase
; Rust filter excludes constant/immutable variables
(state_variable_declaration
  name: (identifier) @name) @decl
