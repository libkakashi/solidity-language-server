; Detect functions with address parameters (candidates for zero-address check).
; The filter function checks if the body contains a check against address(0).
(function_definition
  name: (identifier) @name
  (parameter
    type: (type_name) @type
    name: (identifier) @param)
  body: (function_body) @body) @func
