; Detect tx.origin usage — potential phishing/authorization vulnerability
(member_expression
  object: (identifier) @obj
  property: (identifier) @prop
  (#eq? @obj "tx")
  (#eq? @prop "origin")) @expr
