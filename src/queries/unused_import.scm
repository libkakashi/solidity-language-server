; Collect named imports for unused-import checking
; Rust code does a two-pass: collect these, then scan tree for references
(import_directive
  import_name: (identifier) @imported_name) @import
