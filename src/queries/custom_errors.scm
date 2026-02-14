; Detect require() calls with string message arguments
; e.g. `require(condition, "error message")`
(call_expression
  function: (expression
    (identifier) @fn_name
    (#eq? @fn_name "require"))) @call
