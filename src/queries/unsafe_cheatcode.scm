; Detect calls to potentially unsafe Foundry cheatcodes
(call_expression
  function: (expression
    (member_expression
      property: (identifier) @method
      (#any-of? @method
        "ffi"
        "readFile" "readFileBinary" "readDir" "readLink"
        "writeFile" "writeLine" "writeFileBinary"
        "removeFile" "removeDir"
        "closeFile"
        "setEnv"
        "deriveKey")))) @call
