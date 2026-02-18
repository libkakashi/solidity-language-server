use std::borrow::Cow;
use tree_sitter::{Node, Tree};

use crate::fmt_config::{FmtConfig, IntTypes, NumberUnderscore, QuoteStyle};

// ---------------------------------------------------------------------------
// FormatBuffer — accumulates formatted output
// ---------------------------------------------------------------------------

struct FormatBuffer {
    output: String,
    indent_level: usize,
    /// Current position on the current line (number of chars written since last newline).
    current_line_pos: usize,
    /// Single indent unit (e.g. "    " or "\t").
    indent_unit: String,
    /// Full indent for the current level, cached to avoid recomputing.
    cached_indent: String,
    line_length: usize,
}

impl FormatBuffer {
    fn new(config: &FmtConfig, source_len: usize) -> Self {
        Self {
            // #7: Pre-size to source length to avoid reallocations.
            output: String::with_capacity(source_len + source_len / 10),
            indent_level: 0,
            current_line_pos: 0,
            indent_unit: config.indent_string(),
            cached_indent: String::new(),
            line_length: config.line_length,
        }
    }

    /// Write a string that may contain newlines. Scans for `\n` to track line position.
    fn write(&mut self, s: &str) {
        self.output.push_str(s);
        if let Some(pos) = s.rfind('\n') {
            self.current_line_pos = s.len() - pos - 1;
        } else {
            self.current_line_pos += s.len();
        }
    }

    /// Write a short token guaranteed to contain no newlines. Skips the newline scan. (#5)
    #[inline]
    fn write_token(&mut self, s: &str) {
        self.output.push_str(s);
        self.current_line_pos += s.len();
    }

    /// Write a newline, trimming any trailing whitespace on the current line first. (#2)
    fn write_newline(&mut self) {
        // Trim trailing spaces/tabs from current line before adding newline.
        let bytes = self.output.as_bytes();
        let mut end = bytes.len();
        while end > 0 && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
            end -= 1;
        }
        self.output.truncate(end);
        self.output.push('\n');
        self.current_line_pos = 0;
    }

    /// Write the cached indent string for the current level. (#11)
    fn write_indent(&mut self) {
        self.output.push_str(&self.cached_indent);
        self.current_line_pos = self.cached_indent.len();
    }

    fn write_space(&mut self) {
        self.output.push(' ');
        self.current_line_pos += 1;
    }

    fn indent(&mut self) {
        self.indent_level += 1;
        self.rebuild_indent_cache();
    }

    fn dedent(&mut self) {
        self.indent_level = self.indent_level.saturating_sub(1);
        self.rebuild_indent_cache();
    }

    /// Rebuild the cached full-indent string when level changes. (#11)
    fn rebuild_indent_cache(&mut self) {
        self.cached_indent.clear();
        for _ in 0..self.indent_level {
            self.cached_indent.push_str(&self.indent_unit);
        }
    }

    #[allow(dead_code)]
    fn would_exceed(&self, additional: usize) -> bool {
        self.current_line_pos + additional > self.line_length
    }

    fn finish(self) -> String {
        self.output
    }
}

// ---------------------------------------------------------------------------
// DisableTracker — handles `// forgefmt: disable-*` comments
// ---------------------------------------------------------------------------

struct DisableTracker {
    /// Sorted, non-overlapping disabled byte ranges.
    ranges: Vec<(usize, usize)>,
}

impl DisableTracker {
    fn scan(source: &str) -> Self {
        let mut ranges = Vec::new();
        let mut disable_start: Option<usize> = None;
        // #1: Track running byte offset instead of calling byte_offset_of_line().
        let mut byte_offset: usize = 0;

        for line in source.lines() {
            let trimmed = line.trim();
            let line_len = line.len();
            let next_offset = byte_offset + line_len + 1; // +1 for newline

            if trimmed.contains("forgefmt: disable-start") {
                disable_start = Some(byte_offset);
            } else if trimmed.contains("forgefmt: disable-end") {
                if let Some(start) = disable_start.take() {
                    ranges.push((start, byte_offset + line_len));
                }
            } else if trimmed.contains("forgefmt: disable-next-line") {
                let next_line_start = next_offset.min(source.len());
                let next_line_end = source[next_line_start..]
                    .find('\n')
                    .map(|p| next_line_start + p)
                    .unwrap_or(source.len());
                ranges.push((next_line_start, next_line_end));
            } else if trimmed.contains("forgefmt: disable-line") {
                ranges.push((byte_offset, byte_offset + line_len));
            }

            byte_offset = next_offset;
        }

        // Handle unclosed disable-start.
        if let Some(start) = disable_start {
            ranges.push((start, source.len()));
        }

        Self { ranges }
    }

    /// Check if a byte offset falls within a disabled range. (#12: binary search)
    fn is_disabled(&self, byte_offset: usize) -> bool {
        // Find the last range whose start <= byte_offset.
        let idx = self
            .ranges
            .partition_point(|&(start, _)| start <= byte_offset);
        if idx == 0 {
            return false;
        }
        let (_, end) = self.ranges[idx - 1];
        byte_offset < end
    }
}

// ---------------------------------------------------------------------------
// Formatter — tree-sitter AST walker
// ---------------------------------------------------------------------------

struct Formatter<'a> {
    source_bytes: &'a [u8],
    config: &'a FmtConfig,
    buf: FormatBuffer,
    disable: DisableTracker,
}

impl<'a> Formatter<'a> {
    fn new(source: &'a str, config: &'a FmtConfig) -> Self {
        Self {
            source_bytes: source.as_bytes(),
            config,
            buf: FormatBuffer::new(config, source.len()), // #7
            disable: DisableTracker::scan(source),
        }
    }

    fn node_text(&self, node: Node) -> &'a str {
        node.utf8_text(self.source_bytes).unwrap_or("")
    }

    /// Write the original source text for a node, preserving it verbatim.
    fn write_verbatim(&mut self, node: Node) {
        self.buf.write(self.node_text(node));
    }

    /// Write the original source text but re-indent each line to match the current indent level.
    fn write_verbatim_reindented(&mut self, node: Node) {
        let text = self.node_text(node);
        for (i, line) in text.lines().enumerate() {
            if i > 0 {
                self.buf.write_newline();
                let trimmed = line.trim_start();
                if !trimmed.is_empty() {
                    self.buf.write_indent();
                    self.buf.write(trimmed);
                }
            } else {
                self.buf.write(line.trim_start());
            }
        }
    }

    // -----------------------------------------------------------------------
    // Main dispatch
    // -----------------------------------------------------------------------

    fn format_node(&mut self, node: Node) {
        // Check forgefmt disable comments.
        if self.disable.is_disabled(node.start_byte()) {
            self.write_verbatim(node);
            return;
        }

        match node.kind() {
            "source_file" => self.format_source_file(node),
            "comment" => self.format_comment(node),
            "pragma_directive" => self.format_pragma(node),
            "import_directive" => self.format_import(node),
            "contract_declaration" | "interface_declaration" | "library_declaration" => {
                self.format_contract(node)
            }
            "function_definition" => self.format_function(node),
            "constructor_definition" => self.format_constructor(node),
            "modifier_definition" => self.format_modifier_def(node),
            "fallback_receive_definition" => self.format_fallback_receive(node),
            "state_variable_declaration" | "constant_variable_declaration" => {
                self.format_state_var(node)
            }
            "struct_declaration" => self.format_struct(node),
            "enum_declaration" => self.format_enum(node),
            "event_definition" => self.format_event_or_error(node, "event", "event_parameter"),
            "error_declaration" => self.format_event_or_error(node, "error", "error_parameter"),
            "using_directive" => self.write_verbatim_reindented(node),
            "user_defined_type_definition" => self.write_verbatim_reindented(node),
            "contract_body" => self.format_contract_body(node),
            "function_body" => self.format_block(node),
            "block_statement" => self.format_block(node),
            // statement is a supertype wrapper
            "statement" => {
                if let Some(child) = node.named_child(0) {
                    self.format_node(child);
                }
            }
            "if_statement" => self.format_if(node),
            "for_statement" => self.format_for(node),
            "while_statement" => self.format_while(node),
            "do_while_statement" => self.format_do_while(node),
            "expression_statement" => self.format_expression_stmt(node),
            "return_statement" => self.format_return(node),
            "emit_statement" => self.format_emit(node),
            "revert_statement" => self.format_revert(node),
            "variable_declaration_statement" => self.format_var_decl_stmt(node),
            "assembly_statement" => self.write_verbatim_reindented(node),
            "try_statement" => self.format_try(node),
            "break_statement" => self.buf.write_token("break;"),
            "continue_statement" => self.buf.write_token("continue;"),
            "unchecked" => self.format_unchecked(node),
            // Expressions
            "binary_expression" => self.format_binary_expr(node),
            "unary_expression" => self.format_unary_expr(node),
            "update_expression" => self.write_verbatim(node),
            "ternary_expression" => self.format_ternary_expr(node),
            "assignment_expression" | "augmented_assignment_expression" => {
                self.format_assignment(node)
            }
            "call_expression" => self.format_call_expr(node),
            "member_expression" => self.format_member_expr(node),
            "array_access" => self.format_array_access(node),
            "slice_access" => self.write_verbatim(node),
            "tuple_expression" => self.format_tuple(node),
            "parenthesized_expression" => self.format_parens(node),
            "inline_array_expression" => self.format_inline_array(node),
            "type_cast_expression" => self.format_type_cast(node),
            "new_expression" => self.format_new_expr(node),
            "payable_conversion_expression" => self.write_verbatim(node),
            "struct_expression" => self.write_verbatim_reindented(node),
            "meta_type_expression" => self.write_verbatim(node),
            // Literals
            "string_literal" => self.format_string_literal(node),
            "number_literal" => self.format_number_literal(node),
            "hex_string_literal" => self.write_verbatim(node),
            "unicode_string_literal" => self.write_verbatim(node),
            "boolean_literal" | "true" | "false" => self.write_verbatim(node),
            // Types
            "type_name" => self.format_type_name(node),
            "primitive_type" => self.format_primitive_type(node),
            "user_defined_type" => self.write_verbatim(node),
            // Misc named nodes that appear as children
            "identifier" => self.write_verbatim(node),
            "string" => self.format_string_node(node),
            "visibility" | "state_mutability" | "virtual" | "immutable" | "state_location" => {
                self.write_verbatim(node)
            }
            "override_specifier" => self.format_override_specifier(node),
            "modifier_invocation" => self.write_verbatim(node),
            "parameter" => self.write_verbatim(node),
            "return_type_definition" => self.format_return_type_def(node),
            "return_parameter" => self.write_verbatim(node),
            "inheritance_specifier" => self.write_verbatim(node),
            "catch_clause" => self.format_catch_clause(node),
            "call_argument" => self.format_call_argument(node),
            "call_struct_argument" => self.write_verbatim(node),
            "enum_value" => self.write_verbatim(node),
            "struct_member" => self.write_verbatim(node),
            // Expression supertype
            "expression" => {
                if let Some(child) = node.named_child(0) {
                    self.format_node(child);
                }
            }
            // Fallback: write source verbatim.
            _ => self.write_verbatim(node),
        }
    }

    // -----------------------------------------------------------------------
    // Source file (top-level)
    // -----------------------------------------------------------------------

    fn format_source_file(&mut self, node: Node) {
        // #9: Iterate directly without collecting into Vec.
        if node.named_child_count() == 0 {
            return;
        }

        let mut import_group: Vec<Node> = Vec::new();
        let mut prev_section = "";
        let mut i = 0;
        let mut cursor = node.walk();

        for child in node.named_children(&mut cursor) {
            let kind = child.kind();

            let section = match kind {
                "pragma_directive" => "pragma",
                "import_directive" => "import",
                "comment" => "comment",
                _ => "decl",
            };

            if i > 0 && section != "comment" && prev_section != "comment" {
                if section != prev_section {
                    self.buf.write_newline();
                }
                if section == "decl" && prev_section == "decl" {
                    self.buf.write_newline();
                }
            }

            if kind == "import_directive" {
                import_group.push(child);
            } else {
                if !import_group.is_empty() {
                    self.flush_imports(&import_group);
                    import_group.clear();
                    if section != "import" {
                        self.buf.write_newline();
                    }
                }
                self.format_node(child);
                self.buf.write_newline();
            }

            if section != "comment" {
                prev_section = section;
            }
            i += 1;
        }

        if !import_group.is_empty() {
            self.flush_imports(&import_group);
        }

        if !self.buf.output.ends_with('\n') {
            self.buf.write_newline();
        }
    }

    /// #6: Format imports without creating a new Formatter per import for sorting.
    fn flush_imports(&mut self, imports: &[Node]) {
        if self.config.sort_imports && imports.len() > 1 {
            let saved_pos = self.buf.current_line_pos;
            let mut texts: Vec<String> = Vec::with_capacity(imports.len());

            for n in imports {
                let start = self.buf.output.len();
                self.format_import(*n);
                texts.push(self.buf.output[start..].to_string());
                self.buf.output.truncate(start);
                self.buf.current_line_pos = saved_pos;
            }

            texts.sort();
            for text in &texts {
                self.buf.write(text);
                self.buf.write_newline();
            }
        } else {
            for node in imports {
                self.format_import(*node);
                self.buf.write_newline();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Comments
    // -----------------------------------------------------------------------

    fn format_comment(&mut self, node: Node) {
        self.write_verbatim(node);
    }

    // -----------------------------------------------------------------------
    // Pragma
    // -----------------------------------------------------------------------

    /// #8: Iterate children directly, no intermediate Vec<String>.
    fn format_pragma(&mut self, node: Node) {
        let mut cursor = node.walk();
        let mut first = true;
        for child in node.children(&mut cursor) {
            let text = self.node_text(child);
            if text == ";" {
                self.buf.write_token(";");
            } else {
                if !first {
                    self.buf.write_space();
                }
                self.buf.write_token(text);
                first = false;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Import
    // -----------------------------------------------------------------------

    /// #9: Iterate children directly without collecting into Vec.
    fn format_import(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let text = self.node_text(child);
            match text {
                "import" => {
                    self.buf.write_token("import ");
                }
                "{" => self.buf.write_token("{"),
                "}" => self.buf.write_token("}"),
                "," => self.buf.write_token(", "),
                "*" => self.buf.write_token("*"),
                "from" => self.buf.write_token(" from "),
                "as" => self.buf.write_token(" as "),
                ";" => self.buf.write_token(";"),
                _ => {
                    if child.kind() == "string" {
                        self.format_import_path(child);
                    } else {
                        self.write_verbatim(child);
                    }
                }
            }
        }
    }

    fn format_import_path(&mut self, node: Node) {
        let text = self.node_text(node);
        match self.config.quote_style {
            QuoteStyle::Double => {
                let inner = text.trim_matches(|c| c == '\'' || c == '"');
                self.buf.write_token("\"");
                self.buf.write_token(inner);
                self.buf.write_token("\"");
            }
            QuoteStyle::Single => {
                let inner = text.trim_matches(|c| c == '\'' || c == '"');
                self.buf.write_token("'");
                self.buf.write_token(inner);
                self.buf.write_token("'");
            }
            QuoteStyle::Preserve => self.write_verbatim(node),
        }
    }

    // -----------------------------------------------------------------------
    // Contract / Interface / Library
    // -----------------------------------------------------------------------

    fn format_contract(&mut self, node: Node) {
        let body = node.child_by_field_name("body");
        let body_id = body.map(|b| b.id());
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if body_id == Some(child.id()) {
                continue;
            }

            if !child.is_named() {
                let text = self.node_text(child);
                match text {
                    "abstract" => self.buf.write_token("abstract "),
                    "contract" | "interface" | "library" => {
                        self.buf.write_token(text);
                        self.buf.write_space();
                    }
                    "is" => self.buf.write_token(" is "),
                    "," => self.buf.write_token(", "),
                    _ => {}
                }
            } else {
                match child.kind() {
                    "identifier" => self.buf.write_token(self.node_text(child)),
                    "inheritance_specifier" => self.write_verbatim(child),
                    "comment" => {
                        self.buf.write_space();
                        self.format_comment(child);
                    }
                    _ => self.format_node(child),
                }
            }
        }

        if let Some(body_node) = body {
            self.buf.write_space();
            self.format_contract_body(body_node);
        }
    }

    /// #9: Use named_child_count() + direct iteration instead of collect().
    fn format_contract_body(&mut self, node: Node) {
        self.buf.write_token("{");

        if node.named_child_count() == 0 {
            self.buf.write_token("}");
            return;
        }

        self.buf.write_newline();
        self.buf.indent();

        let mut prev_kind: Option<&str> = None;
        let mut i = 0;
        let mut cursor = node.walk();

        for member in node.named_children(&mut cursor) {
            let kind = member.kind();

            if i > 0 {
                if let Some(pk) = prev_kind {
                    if pk != kind || is_function_like(kind) {
                        self.buf.write_newline();
                    }
                }
            }

            self.buf.write_indent();
            self.format_node(member);
            self.buf.write_newline();

            prev_kind = Some(kind);
            i += 1;
        }

        self.buf.dedent();
        self.buf.write_indent();
        self.buf.write_token("}");
    }

    // -----------------------------------------------------------------------
    // Function / Constructor / Modifier / Fallback
    // -----------------------------------------------------------------------

    fn format_function(&mut self, node: Node) {
        let body = node.child_by_field_name("body");
        let return_type = node.child_by_field_name("return_type");
        let name = node.child_by_field_name("name");
        let body_id = body.map(|b| b.id());
        let return_type_id = return_type.map(|r| r.id());
        let name_id = name.map(|n| n.id());

        let mut cursor = node.walk();
        let mut params: Vec<Node> = Vec::new();
        let mut modifiers: Vec<Node> = Vec::new();

        for child in node.children(&mut cursor) {
            if Some(child.id()) == body_id
                || Some(child.id()) == return_type_id
                || Some(child.id()) == name_id
            {
                continue;
            }
            if child.is_named() {
                match child.kind() {
                    "parameter" => params.push(child),
                    "visibility"
                    | "state_mutability"
                    | "virtual"
                    | "modifier_invocation"
                    | "override_specifier" => modifiers.push(child),
                    _ => {}
                }
            }
        }

        self.buf.write_token("function ");
        if let Some(name_node) = name {
            self.buf.write_token(self.node_text(name_node));
        }
        self.buf.write_token("(");
        self.write_params(&params);
        self.buf.write_token(")");

        for m in &modifiers {
            self.buf.write_space();
            self.format_node(*m);
        }

        if let Some(rt) = return_type {
            self.buf.write_space();
            self.format_return_type_def(rt);
        }

        if let Some(body_node) = body {
            self.buf.write_space();
            self.format_block(body_node);
        } else {
            self.buf.write_token(";");
        }
    }

    fn format_constructor(&mut self, node: Node) {
        let body = node.child_by_field_name("body");
        let body_id = body.map(|b| b.id());

        let mut cursor = node.walk();
        let mut params: Vec<Node> = Vec::new();
        let mut modifiers: Vec<Node> = Vec::new();

        for child in node.children(&mut cursor) {
            if Some(child.id()) == body_id {
                continue;
            }
            if child.is_named() {
                match child.kind() {
                    "parameter" => params.push(child),
                    "visibility" | "state_mutability" | "modifier_invocation" => {
                        modifiers.push(child)
                    }
                    _ => {}
                }
            }
        }

        self.buf.write_token("constructor(");
        self.write_params(&params);
        self.buf.write_token(")");

        for m in &modifiers {
            self.buf.write_space();
            self.format_node(*m);
        }

        if let Some(body_node) = body {
            self.buf.write_space();
            self.format_block(body_node);
        }
    }

    /// #14: Track has_parens during child iteration instead of scanning full source text.
    fn format_modifier_def(&mut self, node: Node) {
        let body = node.child_by_field_name("body");
        let name = node.child_by_field_name("name");
        let body_id = body.map(|b| b.id());
        let name_id = name.map(|n| n.id());

        let mut cursor = node.walk();
        let mut params: Vec<Node> = Vec::new();
        let mut modifiers: Vec<Node> = Vec::new();
        let mut has_parens = false;

        for child in node.children(&mut cursor) {
            if Some(child.id()) == body_id || Some(child.id()) == name_id {
                continue;
            }
            if !child.is_named() {
                if self.node_text(child) == "(" {
                    has_parens = true;
                }
                continue;
            }
            match child.kind() {
                "parameter" => params.push(child),
                "virtual" | "override_specifier" => modifiers.push(child),
                _ => {}
            }
        }

        self.buf.write_token("modifier ");
        if let Some(name_node) = name {
            self.buf.write_token(self.node_text(name_node));
        }

        if has_parens {
            self.buf.write_token("(");
            self.write_params(&params);
            self.buf.write_token(")");
        }

        for m in &modifiers {
            self.buf.write_space();
            self.format_node(*m);
        }

        if let Some(body_node) = body {
            self.buf.write_space();
            self.format_block(body_node);
        } else {
            self.buf.write_token(";");
        }
    }

    fn format_fallback_receive(&mut self, node: Node) {
        let body = node.child_by_field_name("body");
        let body_id = body.map(|b| b.id());

        let mut cursor = node.walk();
        let mut params: Vec<Node> = Vec::new();
        let mut modifiers: Vec<Node> = Vec::new();
        let mut keyword = "";

        for child in node.children(&mut cursor) {
            if Some(child.id()) == body_id {
                continue;
            }
            if !child.is_named() {
                let text = self.node_text(child);
                if text == "fallback" || text == "receive" {
                    keyword = text;
                }
                continue;
            }
            match child.kind() {
                "parameter" => params.push(child),
                "visibility"
                | "state_mutability"
                | "virtual"
                | "modifier_invocation"
                | "override_specifier" => modifiers.push(child),
                _ => {}
            }
        }

        self.buf.write_token(keyword);
        self.buf.write_token("(");
        self.write_params(&params);
        self.buf.write_token(")");

        for m in &modifiers {
            self.buf.write_space();
            self.format_node(*m);
        }

        if let Some(body_node) = body {
            self.buf.write_space();
            self.format_block(body_node);
        } else {
            self.buf.write_token(";");
        }
    }

    fn write_params(&mut self, params: &[Node]) {
        for (i, p) in params.iter().enumerate() {
            if i > 0 {
                self.buf.write_token(", ");
            }
            self.write_verbatim(*p);
        }
    }

    /// #9: Iterate directly without collecting into Vec.
    fn format_return_type_def(&mut self, node: Node) {
        self.buf.write_token("returns (");
        let mut cursor = node.walk();
        let mut first = true;
        for p in node.named_children(&mut cursor) {
            if !first {
                self.buf.write_token(", ");
            }
            self.write_verbatim(p);
            first = false;
        }
        self.buf.write_token(")");
    }

    fn format_override_specifier(&mut self, node: Node) {
        self.write_verbatim(node);
    }

    // -----------------------------------------------------------------------
    // Function / block bodies
    // -----------------------------------------------------------------------

    /// #9: Use named_child_count() + direct iteration.
    fn format_block(&mut self, node: Node) {
        self.buf.write_token("{");

        if node.named_child_count() == 0 {
            self.buf.write_token("}");
            return;
        }

        self.buf.write_newline();
        self.buf.indent();

        let mut cursor = node.walk();
        for stmt in node.named_children(&mut cursor) {
            self.buf.write_indent();
            self.format_node(stmt);
            self.buf.write_newline();
        }

        self.buf.dedent();
        self.buf.write_indent();
        self.buf.write_token("}");
    }

    fn format_unchecked(&mut self, node: Node) {
        self.buf.write_token("unchecked ");
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "block_statement" || child.kind() == "statement" {
                self.format_node(child);
            }
        }
    }

    // -----------------------------------------------------------------------
    // State variables
    // -----------------------------------------------------------------------

    fn format_state_var(&mut self, node: Node) {
        let mut cursor = node.walk();
        let mut first = true;

        for child in node.children(&mut cursor) {
            let text = self.node_text(child);

            if text == ";" {
                self.buf.write_token(";");
                continue;
            }
            if text == "=" {
                self.buf.write_token(" = ");
                continue;
            }
            if text == "constant" {
                if !first {
                    self.buf.write_space();
                }
                self.buf.write_token("constant");
                first = false;
                continue;
            }

            if !first {
                self.buf.write_space();
            }

            if child.is_named() {
                self.format_node(child);
            } else {
                self.buf.write_token(text);
            }
            first = false;
        }
    }

    // -----------------------------------------------------------------------
    // Struct / Enum / Event / Error
    // -----------------------------------------------------------------------

    /// #9: Use named_child_count() + direct iteration.
    fn format_struct(&mut self, node: Node) {
        let name = node.child_by_field_name("name");
        let body = node.child_by_field_name("body");

        self.buf.write_token("struct ");
        if let Some(name_node) = name {
            self.buf.write_token(self.node_text(name_node));
        }
        self.buf.write_token(" {");

        if let Some(body_node) = body {
            if body_node.named_child_count() == 0 {
                self.buf.write_token("}");
                return;
            }

            self.buf.write_newline();
            self.buf.indent();

            let mut cursor = body_node.walk();
            for member in body_node.named_children(&mut cursor) {
                self.buf.write_indent();
                self.write_verbatim_reindented(member);
                let text = self.node_text(member);
                if !text.trim_end().ends_with(';') {
                    self.buf.write_token(";");
                }
                self.buf.write_newline();
            }

            self.buf.dedent();
            self.buf.write_indent();
            self.buf.write_token("}");
        } else {
            self.buf.write_token("}");
        }
    }

    /// #9: Use named_child_count() + direct iteration.
    fn format_enum(&mut self, node: Node) {
        let name = node.child_by_field_name("name");
        let body = node.child_by_field_name("body");

        self.buf.write_token("enum ");
        if let Some(name_node) = name {
            self.buf.write_token(self.node_text(name_node));
        }
        self.buf.write_token(" {");

        if let Some(body_node) = body {
            let count = body_node.named_child_count();
            if count == 0 {
                self.buf.write_token("}");
                return;
            }

            self.buf.write_newline();
            self.buf.indent();

            let mut cursor = body_node.walk();
            let mut i = 0;
            for val in body_node.named_children(&mut cursor) {
                self.buf.write_indent();
                self.write_verbatim(val);
                if i < count - 1 {
                    self.buf.write_token(",");
                }
                self.buf.write_newline();
                i += 1;
            }

            self.buf.dedent();
            self.buf.write_indent();
            self.buf.write_token("}");
        } else {
            self.buf.write_token("}");
        }
    }

    fn format_event_or_error(&mut self, node: Node, keyword: &str, param_kind: &str) {
        let is_event = keyword == "event";
        let mut cursor = node.walk();
        let mut past_keyword = !is_event;
        for child in node.children(&mut cursor) {
            let text = self.node_text(child);
            match text {
                ";" => self.buf.write_token(";"),
                "(" => self.buf.write_token("("),
                ")" => self.buf.write_token(")"),
                "," => self.buf.write_token(", "),
                kw if kw == keyword => {
                    self.buf.write_token(keyword);
                    self.buf.write_token(" ");
                    past_keyword = true;
                }
                "anonymous" if is_event => {
                    self.buf.write_token(" anonymous");
                }
                _ => {
                    if child.is_named() {
                        match child.kind() {
                            "identifier" => self.buf.write_token(self.node_text(child)),
                            k if k == param_kind => self.write_verbatim(child),
                            _ => self.format_node(child),
                        }
                    } else if past_keyword {
                        self.buf.write_token(text);
                    }
                    past_keyword = true;
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------------

    /// #9: Iterate directly, track prev_text instead of indexing into Vec.
    fn format_if(&mut self, node: Node) {
        let condition = node.child_by_field_name("condition");
        let condition_id = condition.map(|c| c.id());

        let mut cursor = node.walk();
        let mut prev_text = "";

        for child in node.children(&mut cursor) {
            let text = self.node_text(child);

            match text {
                "if" => {
                    self.buf.write_token("if (");
                }
                "(" if prev_text == "if" => {
                    // Already wrote "(" with "if (".
                }
                ")" if condition.is_some() => {
                    self.buf.write_token(") ");
                }
                "else" => {
                    self.buf.write_token(" else ");
                }
                _ => {
                    if child.is_named() {
                        if Some(child.id()) == condition_id {
                            self.format_node(child);
                        } else {
                            self.format_node(child);
                        }
                    }
                }
            }
            prev_text = text;
        }
    }

    /// #10: No intermediate String allocations — use &str slices directly.
    fn format_for(&mut self, node: Node) {
        let initial = node.child_by_field_name("initial");
        let condition = node.child_by_field_name("condition");
        let update = node.child_by_field_name("update");
        let body = node.child_by_field_name("body");

        self.buf.write_token("for (");

        if let Some(init) = initial {
            let text = self.node_text(init).trim();
            let text = text.strip_suffix(';').unwrap_or(text);
            self.buf.write(text);
        }
        self.buf.write_token("; ");

        if let Some(cond) = condition {
            let text = self.node_text(cond).trim();
            let text = text.strip_suffix(';').unwrap_or(text);
            self.buf.write(text);
        }
        self.buf.write_token("; ");

        if let Some(upd) = update {
            let text = self.node_text(upd).trim();
            self.buf.write(text);
        }

        self.buf.write_token(") ");

        if let Some(body_node) = body {
            self.format_node(body_node);
        }
    }

    fn format_while(&mut self, node: Node) {
        self.buf.write_token("while (");

        let condition = node.child_by_field_name("condition");
        let body = node.child_by_field_name("body");

        if let Some(cond) = condition {
            self.format_node(cond);
        }

        self.buf.write_token(") ");

        if let Some(body_node) = body {
            self.format_node(body_node);
        }
    }

    fn format_do_while(&mut self, node: Node) {
        let body = node.child_by_field_name("body");
        let condition = node.child_by_field_name("condition");

        self.buf.write_token("do ");

        if let Some(body_node) = body {
            self.format_node(body_node);
        }

        self.buf.write_token(" while (");

        if let Some(cond) = condition {
            self.format_node(cond);
        }

        self.buf.write_token(");");
    }

    fn format_expression_stmt(&mut self, node: Node) {
        if let Some(expr) = node.named_child(0) {
            self.format_node(expr);
        }
        self.buf.write_token(";");
    }

    fn format_return(&mut self, node: Node) {
        self.buf.write_token("return");
        if let Some(expr) = node.named_child(0) {
            self.buf.write_space();
            self.format_node(expr);
        }
        self.buf.write_token(";");
    }

    fn format_emit(&mut self, node: Node) {
        self.buf.write_token("emit ");
        let mut cursor = node.walk();
        let mut first_named = true;
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                let text = self.node_text(child);
                match text {
                    "emit" | ";" => {}
                    "(" => self.buf.write_token("("),
                    ")" => self.buf.write_token(")"),
                    "," => self.buf.write_token(", "),
                    _ => {}
                }
                continue;
            }
            match child.kind() {
                "expression" | "identifier" | "member_expression" | "call_expression" => {
                    self.format_node(child);
                    first_named = false;
                }
                "call_argument" => {
                    self.format_call_argument(child);
                    first_named = false;
                }
                _ => {
                    self.format_node(child);
                    first_named = false;
                }
            }
        }
        let _ = first_named; // suppress unused warning
        self.buf.write_token(";");
    }

    fn format_revert(&mut self, node: Node) {
        self.buf.write_token("revert");
        let error = node.child_by_field_name("error");
        let mut cursor = node.walk();

        if let Some(err) = error {
            self.buf.write_space();
            self.format_node(err);
        }

        for child in node.named_children(&mut cursor) {
            if child.kind() == "revert_arguments" {
                self.write_verbatim(child);
            }
        }

        self.buf.write_token(";");
    }

    fn format_var_decl_stmt(&mut self, node: Node) {
        let mut cursor = node.walk();
        let mut first = true;

        for child in node.children(&mut cursor) {
            let text = self.node_text(child);
            match text {
                ";" => self.buf.write_token(";"),
                "=" => self.buf.write_token(" = "),
                "var" => {
                    self.buf.write_token("var");
                    first = false;
                }
                _ => {
                    if !first && text != "(" && text != ")" && text != "," {
                        self.buf.write_space();
                    }
                    if child.is_named() {
                        self.format_node(child);
                    } else {
                        self.buf.write_token(text);
                    }
                    first = false;
                }
            }
        }
    }

    fn format_try(&mut self, node: Node) {
        let attempt = node.child_by_field_name("attempt");
        let body = node.child_by_field_name("body");

        self.buf.write_token("try ");

        if let Some(expr) = attempt {
            self.format_node(expr);
        }

        let mut cursor = node.walk();
        let mut has_returns = false;
        for child in node.children(&mut cursor) {
            if child.is_named() && child.kind() == "parameter" && !has_returns {
                self.buf.write_token(" returns (");
                has_returns = true;
                self.write_verbatim(child);
            } else if child.is_named() && child.kind() == "parameter" && has_returns {
                self.buf.write_token(", ");
                self.write_verbatim(child);
            }
        }
        if has_returns {
            self.buf.write_token(")");
        }

        if let Some(body_node) = body {
            self.buf.write_space();
            self.format_block(body_node);
        }

        let mut catch_cursor = node.walk();
        for child in node.named_children(&mut catch_cursor) {
            if child.kind() == "catch_clause" {
                self.buf.write_space();
                self.format_catch_clause(child);
            }
        }
    }

    fn format_catch_clause(&mut self, node: Node) {
        let body = node.child_by_field_name("body");
        let body_id = body.map(|b| b.id());

        self.buf.write_token("catch");

        let mut cursor = node.walk();
        let mut has_params = false;
        for child in node.children(&mut cursor) {
            if Some(child.id()) == body_id {
                continue;
            }
            if !child.is_named() {
                let text = self.node_text(child);
                match text {
                    "catch" => {}
                    "(" => {
                        self.buf.write_token(" (");
                        has_params = true;
                    }
                    ")" => self.buf.write_token(")"),
                    "," => self.buf.write_token(", "),
                    _ => {}
                }
            } else if child.kind() == "identifier" && !has_params {
                self.buf.write_space();
                self.buf.write_token(self.node_text(child));
            } else if child.kind() == "parameter" {
                self.write_verbatim(child);
            }
        }

        if let Some(body_node) = body {
            self.buf.write_space();
            self.format_block(body_node);
        }
    }

    // -----------------------------------------------------------------------
    // Expressions
    // -----------------------------------------------------------------------

    fn format_binary_expr(&mut self, node: Node) {
        let left = node.child_by_field_name("left");
        let right = node.child_by_field_name("right");
        let op_node = node.child_by_field_name("operator");

        if let Some(l) = left {
            self.format_node(l);
        }

        if let Some(op) = op_node {
            let op_text = self.node_text(op);
            self.buf.write_space();
            self.buf.write_token(op_text);
            self.buf.write_space();
        }

        if let Some(r) = right {
            self.format_node(r);
        }
    }

    /// #9: Use child_count() + child(i) instead of collecting into Vec.
    fn format_unary_expr(&mut self, node: Node) {
        if node.child_count() == 2 {
            let first = node.child(0).unwrap();
            let second = node.child(1).unwrap();
            if !first.is_named() {
                // Prefix operator.
                self.buf.write_token(self.node_text(first));
                self.format_node(second);
            } else {
                // Postfix operator.
                self.format_node(first);
                self.buf.write_token(self.node_text(second));
            }
        } else {
            self.write_verbatim(node);
        }
    }

    /// #9: Iterate directly without collecting into Vec.
    fn format_ternary_expr(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let text = self.node_text(child);
            match text {
                "?" => self.buf.write_token(" ? "),
                ":" => self.buf.write_token(" : "),
                _ => {
                    if child.is_named() {
                        self.format_node(child);
                    }
                }
            }
        }
    }

    /// #16: Find operator via next_sibling instead of iterating all children.
    fn format_assignment(&mut self, node: Node) {
        let left = node.child_by_field_name("left");
        let right = node.child_by_field_name("right");

        if let Some(l) = left {
            self.format_node(l);

            // The operator is the next sibling after left.
            let mut next = l.next_sibling();
            while let Some(n) = next {
                if !n.is_named() {
                    let text = self.node_text(n);
                    if text.contains('=') {
                        self.buf.write_token(" ");
                        self.buf.write_token(text);
                        self.buf.write_token(" ");
                        break;
                    }
                }
                next = n.next_sibling();
            }
        }

        if let Some(r) = right {
            self.format_node(r);
        }
    }

    fn format_call_expr(&mut self, node: Node) {
        let func = node.child_by_field_name("function");
        let func_id = func.map(|f| f.id());

        if let Some(f) = func {
            self.format_node(f);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Some(child.id()) == func_id {
                continue;
            }
            if !child.is_named() {
                let text = self.node_text(child);
                match text {
                    "(" => self.buf.write_token("("),
                    ")" => self.buf.write_token(")"),
                    "," => self.buf.write_token(", "),
                    "{" => {
                        if self.config.bracket_spacing {
                            self.buf.write_token("{ ");
                        } else {
                            self.buf.write_token("{");
                        }
                    }
                    "}" => {
                        if self.config.bracket_spacing {
                            self.buf.write_token(" }");
                        } else {
                            self.buf.write_token("}");
                        }
                    }
                    _ => self.buf.write_token(text),
                }
            } else if child.kind() == "call_argument" {
                self.format_call_argument(child);
            } else {
                self.format_node(child);
            }
        }
    }

    /// #9: Iterate directly without collecting into Vec.
    fn format_call_argument(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                let text = self.node_text(child);
                if text == "," {
                    self.buf.write_token(", ");
                } else {
                    self.buf.write_token(text);
                }
            } else {
                self.format_node(child);
            }
        }
    }

    fn format_member_expr(&mut self, node: Node) {
        let object = node.child_by_field_name("object");
        let property = node.child_by_field_name("property");

        if let Some(obj) = object {
            self.format_node(obj);
        }
        self.buf.write_token(".");
        if let Some(prop) = property {
            self.buf.write_token(self.node_text(prop));
        }
    }

    fn format_array_access(&mut self, node: Node) {
        let base = node.child_by_field_name("base");
        let index = node.child_by_field_name("index");

        if let Some(b) = base {
            self.format_node(b);
        }
        self.buf.write_token("[");
        if let Some(idx) = index {
            self.format_node(idx);
        }
        self.buf.write_token("]");
    }

    fn format_tuple(&mut self, node: Node) {
        self.buf.write_token("(");

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                let text = self.node_text(child);
                match text {
                    "(" | ")" => {}
                    "," => self.buf.write_token(", "),
                    _ => self.buf.write_token(text),
                }
            } else {
                self.format_node(child);
            }
        }

        self.buf.write_token(")");
    }

    fn format_parens(&mut self, node: Node) {
        self.buf.write_token("(");
        if let Some(expr) = node.named_child(0) {
            self.format_node(expr);
        }
        self.buf.write_token(")");
    }

    fn format_inline_array(&mut self, node: Node) {
        self.buf.write_token("[");
        let mut cursor = node.walk();
        let mut first = true;
        for child in node.named_children(&mut cursor) {
            if !first {
                self.buf.write_token(", ");
            }
            self.format_node(child);
            first = false;
        }
        self.buf.write_token("]");
    }

    fn format_type_cast(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                let text = self.node_text(child);
                match text {
                    "(" => self.buf.write_token("("),
                    ")" => self.buf.write_token(")"),
                    "," => self.buf.write_token(", "),
                    _ => self.buf.write_token(text),
                }
            } else {
                self.format_node(child);
            }
        }
    }

    fn format_new_expr(&mut self, node: Node) {
        self.buf.write_token("new ");
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.format_node(child);
        }
    }

    // -----------------------------------------------------------------------
    // Content transformations
    // -----------------------------------------------------------------------

    fn format_string_literal(&mut self, node: Node) {
        let mut cursor = node.walk();
        let mut first = true;
        for child in node.named_children(&mut cursor) {
            if !first {
                self.buf.write_space();
            }
            self.format_string_node(child);
            first = false;
        }
    }

    fn format_string_node(&mut self, node: Node) {
        let text = self.node_text(node);
        match self.config.quote_style {
            QuoteStyle::Double => {
                if text.starts_with('\'') && text.ends_with('\'') {
                    let inner = &text[1..text.len() - 1];
                    self.buf.write_token("\"");
                    self.buf.write_token(inner);
                    self.buf.write_token("\"");
                } else {
                    self.buf.write_token(text);
                }
            }
            QuoteStyle::Single => {
                if text.starts_with('"') && text.ends_with('"') {
                    let inner = &text[1..text.len() - 1];
                    self.buf.write_token("'");
                    self.buf.write_token(inner);
                    self.buf.write_token("'");
                } else {
                    self.buf.write_token(text);
                }
            }
            QuoteStyle::Preserve => self.buf.write_token(text),
        }
    }

    fn format_number_literal(&mut self, node: Node) {
        let text = self.node_text(node);
        let transformed = transform_number_underscore(text, self.config.number_literal_underscore);
        self.buf.write_token(&transformed);
    }

    /// #15: Check child kind instead of scanning full text for "mapping"/"function".
    fn format_type_name(&mut self, node: Node) {
        // Check first named child kind for complex types.
        let is_complex = node.named_child(0).map_or(false, |c| {
            let k = c.kind();
            k.contains("mapping") || k.contains("function")
        });

        if is_complex {
            self.write_verbatim_reindented(node);
            return;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                self.format_node(child);
            } else {
                self.buf.write_token(self.node_text(child));
            }
        }
    }

    fn format_primitive_type(&mut self, node: Node) {
        let text = self.node_text(node);
        let transformed = transform_int_type(text, self.config.int_types);
        self.buf.write_token(&transformed);
    }
}

// ---------------------------------------------------------------------------
// Content transformation helpers (pure functions)
// ---------------------------------------------------------------------------

/// #3: Return Cow to avoid allocating when no transformation is needed.
fn transform_int_type<'a>(text: &'a str, int_types: IntTypes) -> Cow<'a, str> {
    match int_types {
        IntTypes::Long => match text {
            "uint" => Cow::Borrowed("uint256"),
            "int" => Cow::Borrowed("int256"),
            _ => Cow::Borrowed(text),
        },
        IntTypes::Short => match text {
            "uint256" => Cow::Borrowed("uint"),
            "int256" => Cow::Borrowed("int"),
            _ => Cow::Borrowed(text),
        },
        IntTypes::Preserve => Cow::Borrowed(text),
    }
}

/// #4: Return Cow to avoid allocating when no transformation is needed.
fn transform_number_underscore<'a>(text: &'a str, style: NumberUnderscore) -> Cow<'a, str> {
    match style {
        NumberUnderscore::Preserve => Cow::Borrowed(text),
        NumberUnderscore::Remove => {
            if !text.contains('_') {
                Cow::Borrowed(text)
            } else {
                Cow::Owned(text.replace('_', ""))
            }
        }
        NumberUnderscore::Thousands => {
            let needs_strip = text.contains('_');
            let stripped: Cow<'a, str> = if needs_strip {
                Cow::Owned(text.replace('_', ""))
            } else {
                Cow::Borrowed(text)
            };

            // Hex and scientific notation: return as-is (without underscores).
            if stripped.starts_with("0x")
                || stripped.starts_with("0X")
                || stripped.contains('e')
                || stripped.contains('E')
            {
                return stripped;
            }

            // Decimal numbers.
            if let Some(dot_pos) = stripped.find('.') {
                let integer_part = &stripped[..dot_pos];
                let decimal_part = &stripped[dot_pos..];
                if integer_part.len() <= 3 {
                    return stripped;
                }
                return Cow::Owned(format!(
                    "{}{}",
                    insert_thousands_separator(integer_part),
                    decimal_part
                ));
            }

            if stripped.len() <= 3 {
                return stripped;
            }

            Cow::Owned(insert_thousands_separator(&stripped))
        }
    }
}

/// #13: Iterate bytes directly (number literals are ASCII), no intermediate Vec<char>.
fn insert_thousands_separator(s: &str) -> String {
    let len = s.len();
    if len <= 3 {
        return s.to_string();
    }

    let mut result = String::with_capacity(len + len / 3);
    for (i, b) in s.bytes().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push('_');
        }
        result.push(b as char);
    }
    result
}

fn is_function_like(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition"
            | "constructor_definition"
            | "modifier_definition"
            | "fallback_receive_definition"
    )
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Format a Solidity source file using the given tree-sitter parse tree and config.
///
/// Returns the formatted source string. If the tree contains parse errors,
/// returns the original source unchanged (as a borrowed Cow, zero allocation). (#17)
pub fn format<'a>(source: &'a str, tree: &Tree, config: &FmtConfig) -> Cow<'a, str> {
    // Graceful degradation: if there are parse errors, return source unchanged.
    // #17: Cow::Borrowed avoids allocating a full copy of the source.
    if tree.root_node().has_error() {
        return Cow::Borrowed(source);
    }

    let mut formatter = Formatter::new(source, config);
    formatter.format_node(tree.root_node());
    let mut result = formatter.buf.finish();

    // #2: Trailing whitespace is already trimmed by write_newline().
    // Just ensure single trailing newline, trimming any trailing whitespace on the last line.
    if !result.ends_with('\n') {
        while result.ends_with(' ') || result.ends_with('\t') {
            result.pop();
        }
        result.push('\n');
    }

    Cow::Owned(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::TsParser;

    fn fmt(source: &str) -> String {
        let mut parser = TsParser::new();
        let tree = parser.parse(source, None).expect("parse failed");
        let config = FmtConfig::default();
        format(source, &tree, &config).into_owned()
    }

    fn fmt_with(source: &str, config: &FmtConfig) -> String {
        let mut parser = TsParser::new();
        let tree = parser.parse(source, None).expect("parse failed");
        format(source, &tree, config).into_owned()
    }

    #[test]
    fn test_empty_file() {
        assert_eq!(fmt(""), "\n");
    }

    #[test]
    fn test_pragma() {
        let result = fmt("pragma solidity ^0.8.29;");
        assert_eq!(result, "pragma solidity ^0.8.29;\n");
    }

    #[test]
    fn test_simple_contract() {
        let source = r#"pragma solidity ^0.8.29;
contract Foo {
}"#;
        let result = fmt(source);
        assert!(result.contains("contract Foo {"));
        assert!(result.contains("}"));
    }

    #[test]
    fn test_function_formatting() {
        let source = r#"pragma solidity ^0.8.29;
contract Foo {
    function bar( uint256 a , uint256 b ) public pure returns ( uint256 ) {
        return a + b;
    }
}"#;
        let result = fmt(source);
        assert!(
            result.contains("function bar(uint256 a, uint256 b) public pure returns (uint256)")
        );
    }

    #[test]
    fn test_int_types_long() {
        let source = r#"pragma solidity ^0.8.29;
contract Foo {
    uint public x;
}"#;
        let result = fmt(source);
        assert!(
            result.contains("uint256"),
            "Expected uint256, got:\n{result}"
        );
    }

    #[test]
    fn test_int_types_short() {
        let source = r#"pragma solidity ^0.8.29;
contract Foo {
    uint256 public x;
}"#;
        let mut config = FmtConfig::default();
        config.int_types = IntTypes::Short;
        let result = fmt_with(source, &config);
        assert!(result.contains("uint "), "Expected uint, got:\n{result}");
    }

    #[test]
    fn test_quote_style_double() {
        let source = r#"import {Foo} from './Foo.sol';"#;
        let result = fmt(source);
        assert!(
            result.contains("\"./Foo.sol\""),
            "Expected double quotes, got:\n{result}"
        );
    }

    #[test]
    fn test_number_underscore_remove() {
        assert_eq!(
            transform_number_underscore("1_000_000", NumberUnderscore::Remove).as_ref(),
            "1000000"
        );
    }

    #[test]
    fn test_number_underscore_thousands() {
        assert_eq!(
            transform_number_underscore("1000000", NumberUnderscore::Thousands).as_ref(),
            "1_000_000"
        );
    }

    #[test]
    fn test_int_type_transform() {
        assert_eq!(
            transform_int_type("uint", IntTypes::Long).as_ref(),
            "uint256"
        );
        assert_eq!(transform_int_type("int", IntTypes::Long).as_ref(), "int256");
        assert_eq!(
            transform_int_type("uint256", IntTypes::Short).as_ref(),
            "uint"
        );
        assert_eq!(
            transform_int_type("uint128", IntTypes::Short).as_ref(),
            "uint128"
        );
        assert_eq!(
            transform_int_type("uint", IntTypes::Preserve).as_ref(),
            "uint"
        );
    }

    #[test]
    fn test_disable_next_line() {
        let source = r#"pragma solidity ^0.8.29;
contract Foo {
    // forgefmt: disable-next-line
    uint   public   x ;
    uint public y;
}"#;
        let result = fmt(source);
        assert!(
            result.contains("uint   public   x ;"),
            "Expected preserved line, got:\n{result}"
        );
    }

    #[test]
    fn test_idempotency() {
        let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract Foo {
    uint256 public x;

    function bar(uint256 a) public pure returns (uint256) {
        return a + 1;
    }
}
"#;
        let first = fmt(source);
        let second = fmt(&first);
        assert_eq!(first, second, "Formatting is not idempotent");
    }

    #[test]
    fn test_transform_cow_no_alloc() {
        // Verify Cow::Borrowed is returned for no-op transforms.
        let result = transform_int_type("uint128", IntTypes::Long);
        assert!(matches!(result, Cow::Borrowed(_)));

        let result = transform_int_type("address", IntTypes::Short);
        assert!(matches!(result, Cow::Borrowed(_)));

        let result = transform_number_underscore("42", NumberUnderscore::Preserve);
        assert!(matches!(result, Cow::Borrowed(_)));

        let result = transform_number_underscore("42", NumberUnderscore::Remove);
        assert!(matches!(result, Cow::Borrowed(_)));

        let result = transform_number_underscore("42", NumberUnderscore::Thousands);
        assert!(matches!(result, Cow::Borrowed(_)));
    }

    #[test]
    fn test_error_returns_borrowed() {
        let source = "contract { invalid }}}";
        let mut parser = TsParser::new();
        let tree = parser.parse(source, None).expect("parse failed");
        let config = FmtConfig::default();
        let result = format(source, &tree, &config);
        // Should return Cow::Borrowed for parse errors (zero allocation).
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(&*result, source);
    }
}
