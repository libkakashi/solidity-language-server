; Detect division before multiplication: (a / b) * c
; Match any binary_expression that contains a nested binary_expression anywhere
; in its left subtree. The Rust filter checks the outer op is * and the inner op is /.
(binary_expression
  left: (_) @left_subtree
  right: (_) @multiplier
) @outer
