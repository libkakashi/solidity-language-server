; Detect low-level calls (.call, .delegatecall, .staticcall) where the return
; value is discarded (wrapped in expression_statement, not assigned)
(expression_statement
  (expression
    (call_expression
      function: (expression
        (member_expression
          property: (identifier) @method
          (#any-of? @method "call" "delegatecall" "staticcall")))))) @stmt
