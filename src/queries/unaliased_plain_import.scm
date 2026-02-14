; Detect plain imports without named imports or alias
; e.g. `import "foo.sol";` instead of `import {Foo} from "foo.sol";`
(import_directive
  source: (string) @path) @import
