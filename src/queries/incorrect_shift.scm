; Detect shift operations where left operand is a literal (likely reversed)
; e.g. `256 << x` should probably be `x << 256`
(binary_expression
  left: (expression (number_literal) @literal)
  right: (_) @variable
) @expr
