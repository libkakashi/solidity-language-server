use std::path::Path;

// ---------------------------------------------------------------------------
// Config enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentStyle {
    Tabs,
    Spaces,
}

impl Default for IndentStyle {
    fn default() -> Self {
        IndentStyle::Spaces
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStyle {
    Double,
    Single,
    Preserve,
}

impl Default for QuoteStyle {
    fn default() -> Self {
        QuoteStyle::Double
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberUnderscore {
    Remove,
    Thousands,
    Preserve,
}

impl Default for NumberUnderscore {
    fn default() -> Self {
        NumberUnderscore::Preserve
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntTypes {
    Long,
    Short,
    Preserve,
}

impl Default for IntTypes {
    fn default() -> Self {
        IntTypes::Long
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultilineFuncHeader {
    AttributesFirst,
    ParamsFirst,
    AllParams,
    Preserve,
}

impl Default for MultilineFuncHeader {
    fn default() -> Self {
        MultilineFuncHeader::AttributesFirst
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SingleLineBlocks {
    Single,
    Multi,
    Preserve,
}

impl Default for SingleLineBlocks {
    fn default() -> Self {
        SingleLineBlocks::Preserve
    }
}

// ---------------------------------------------------------------------------
// FmtConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FmtConfig {
    pub line_length: usize,
    pub tab_width: usize,
    pub indent_style: IndentStyle,
    pub bracket_spacing: bool,
    pub override_spacing: bool,
    pub quote_style: QuoteStyle,
    pub number_literal_underscore: NumberUnderscore,
    pub hex_underscore: NumberUnderscore,
    pub int_types: IntTypes,
    pub multiline_func_header: MultilineFuncHeader,
    pub single_line_statement_blocks: SingleLineBlocks,
    pub contract_new_lines: bool,
    pub sort_imports: bool,
    pub wrap_comments: bool,
    pub ignore: Vec<String>,
}

impl Default for FmtConfig {
    fn default() -> Self {
        Self {
            line_length: 120,
            tab_width: 4,
            indent_style: IndentStyle::Spaces,
            bracket_spacing: false,
            override_spacing: true,
            quote_style: QuoteStyle::Double,
            number_literal_underscore: NumberUnderscore::Preserve,
            hex_underscore: NumberUnderscore::Remove,
            int_types: IntTypes::Long,
            multiline_func_header: MultilineFuncHeader::AttributesFirst,
            single_line_statement_blocks: SingleLineBlocks::Preserve,
            contract_new_lines: false,
            sort_imports: false,
            wrap_comments: false,
            ignore: Vec::new(),
        }
    }
}

impl FmtConfig {
    pub fn indent_string(&self) -> String {
        match self.indent_style {
            IndentStyle::Tabs => "\t".to_string(),
            IndentStyle::Spaces => " ".repeat(self.tab_width),
        }
    }
}

// ---------------------------------------------------------------------------
// Config loader (framework-agnostic)
// ---------------------------------------------------------------------------

/// Load formatter config from the project root, trying multiple sources:
/// 1. `.solidityfmt.toml` — standalone, framework-agnostic config
/// 2. `foundry.toml` — Foundry projects (`[fmt]` or `[profile.default.fmt]`)
/// 3. Falls back to sensible defaults if no config file is found.
pub fn load_fmt_config(project_root: &Path) -> FmtConfig {
    // 1. Try standalone .solidityfmt.toml (top-level keys, no section needed).
    let standalone = project_root.join(".solidityfmt.toml");
    if let Ok(content) = std::fs::read_to_string(&standalone) {
        if let Ok(table) = content.parse::<toml::Table>() {
            return parse_fmt_table(&table);
        }
    }

    // 2. Try foundry.toml.
    let foundry = project_root.join("foundry.toml");
    if let Ok(content) = std::fs::read_to_string(&foundry) {
        if let Ok(table) = content.parse::<toml::Table>() {
            // [fmt] section.
            if let Some(fmt) = table.get("fmt").and_then(|v| v.as_table()) {
                return parse_fmt_table(fmt);
            }
            // [profile.default.fmt] section.
            if let Some(profile) = table.get("profile").and_then(|v| v.as_table()) {
                if let Some(default) = profile.get("default").and_then(|v| v.as_table()) {
                    if let Some(fmt) = default.get("fmt").and_then(|v| v.as_table()) {
                        return parse_fmt_table(fmt);
                    }
                }
            }
        }
    }

    FmtConfig::default()
}

fn parse_fmt_table(table: &toml::Table) -> FmtConfig {
    let mut cfg = FmtConfig::default();

    if let Some(v) = table.get("line_length").and_then(|v| v.as_integer()) {
        cfg.line_length = v as usize;
    }
    if let Some(v) = table.get("tab_width").and_then(|v| v.as_integer()) {
        cfg.tab_width = v as usize;
    }
    if let Some(v) = table.get("bracket_spacing").and_then(|v| v.as_bool()) {
        cfg.bracket_spacing = v;
    }
    if let Some(v) = table.get("override_spacing").and_then(|v| v.as_bool()) {
        cfg.override_spacing = v;
    }
    if let Some(v) = table.get("contract_new_lines").and_then(|v| v.as_bool()) {
        cfg.contract_new_lines = v;
    }
    if let Some(v) = table.get("sort_imports").and_then(|v| v.as_bool()) {
        cfg.sort_imports = v;
    }
    if let Some(v) = table.get("wrap_comments").and_then(|v| v.as_bool()) {
        cfg.wrap_comments = v;
    }

    if let Some(v) = table.get("indent_style").and_then(|v| v.as_str()) {
        cfg.indent_style = match v {
            "tab" | "tabs" => IndentStyle::Tabs,
            _ => IndentStyle::Spaces,
        };
    }
    if let Some(v) = table.get("quote_style").and_then(|v| v.as_str()) {
        cfg.quote_style = match v {
            "single" => QuoteStyle::Single,
            "double" => QuoteStyle::Double,
            _ => QuoteStyle::Preserve,
        };
    }
    if let Some(v) = table.get("number_literal_underscore").and_then(|v| v.as_str()) {
        cfg.number_literal_underscore = parse_underscore(v);
    }
    if let Some(v) = table.get("hex_underscore").and_then(|v| v.as_str()) {
        cfg.hex_underscore = parse_underscore(v);
    }
    if let Some(v) = table.get("int_types").and_then(|v| v.as_str()) {
        cfg.int_types = match v {
            "long" => IntTypes::Long,
            "short" => IntTypes::Short,
            _ => IntTypes::Preserve,
        };
    }
    if let Some(v) = table.get("multiline_func_header").and_then(|v| v.as_str()) {
        cfg.multiline_func_header = match v {
            "attributes_first" => MultilineFuncHeader::AttributesFirst,
            "params_first" => MultilineFuncHeader::ParamsFirst,
            "all_params" => MultilineFuncHeader::AllParams,
            _ => MultilineFuncHeader::Preserve,
        };
    }
    if let Some(v) = table.get("single_line_statement_blocks").and_then(|v| v.as_str()) {
        cfg.single_line_statement_blocks = match v {
            "single" => SingleLineBlocks::Single,
            "multi" => SingleLineBlocks::Multi,
            _ => SingleLineBlocks::Preserve,
        };
    }

    if let Some(arr) = table.get("ignore").and_then(|v| v.as_array()) {
        cfg.ignore = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
    }

    cfg
}

fn parse_underscore(s: &str) -> NumberUnderscore {
    match s {
        "remove" => NumberUnderscore::Remove,
        "thousands" => NumberUnderscore::Thousands,
        _ => NumberUnderscore::Preserve,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = FmtConfig::default();
        assert_eq!(cfg.line_length, 120);
        assert_eq!(cfg.tab_width, 4);
        assert_eq!(cfg.indent_style, IndentStyle::Spaces);
        assert!(!cfg.bracket_spacing);
        assert!(cfg.override_spacing);
        assert_eq!(cfg.quote_style, QuoteStyle::Double);
        assert_eq!(cfg.int_types, IntTypes::Long);
        assert!(!cfg.sort_imports);
    }

    #[test]
    fn test_indent_string_spaces() {
        let cfg = FmtConfig::default();
        assert_eq!(cfg.indent_string(), "    ");
    }

    #[test]
    fn test_indent_string_tabs() {
        let mut cfg = FmtConfig::default();
        cfg.indent_style = IndentStyle::Tabs;
        assert_eq!(cfg.indent_string(), "\t");
    }

    #[test]
    fn test_parse_fmt_table() {
        let toml_str = r#"
line_length = 80
tab_width = 2
bracket_spacing = true
int_types = "short"
quote_style = "single"
sort_imports = true
"#;
        let table: toml::Table = toml_str.parse().unwrap();
        let cfg = parse_fmt_table(&table);
        assert_eq!(cfg.line_length, 80);
        assert_eq!(cfg.tab_width, 2);
        assert!(cfg.bracket_spacing);
        assert_eq!(cfg.int_types, IntTypes::Short);
        assert_eq!(cfg.quote_style, QuoteStyle::Single);
        assert!(cfg.sort_imports);
    }

    #[test]
    fn test_load_fmt_config_missing_file() {
        let cfg = load_fmt_config(Path::new("/nonexistent/path"));
        assert_eq!(cfg.line_length, 120); // default
    }

    #[test]
    fn test_load_fmt_config_from_solidityfmt_toml() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join(".solidityfmt.toml");
        std::fs::write(
            &toml_path,
            r#"
line_length = 100
tab_width = 2
bracket_spacing = true
"#,
        )
        .unwrap();

        let cfg = load_fmt_config(dir.path());
        assert_eq!(cfg.line_length, 100);
        assert_eq!(cfg.tab_width, 2);
        assert!(cfg.bracket_spacing);
    }

    #[test]
    fn test_load_solidityfmt_takes_priority_over_foundry() {
        let dir = tempfile::tempdir().unwrap();
        // Write both config files.
        std::fs::write(
            dir.path().join(".solidityfmt.toml"),
            r#"line_length = 80"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("foundry.toml"),
            r#"
[fmt]
line_length = 100
"#,
        )
        .unwrap();

        let cfg = load_fmt_config(dir.path());
        // .solidityfmt.toml should win.
        assert_eq!(cfg.line_length, 80);
    }

    #[test]
    fn test_load_fmt_config_from_foundry_toml() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("foundry.toml");
        std::fs::write(
            &toml_path,
            r#"
[fmt]
line_length = 100
tab_width = 2
bracket_spacing = true
"#,
        )
        .unwrap();

        let cfg = load_fmt_config(dir.path());
        assert_eq!(cfg.line_length, 100);
        assert_eq!(cfg.tab_width, 2);
        assert!(cfg.bracket_spacing);
    }

    #[test]
    fn test_load_fmt_config_profile_default() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("foundry.toml");
        std::fs::write(
            &toml_path,
            r#"
[profile.default.fmt]
line_length = 80
int_types = "short"
"#,
        )
        .unwrap();

        let cfg = load_fmt_config(dir.path());
        assert_eq!(cfg.line_length, 80);
        assert_eq!(cfg.int_types, IntTypes::Short);
    }
}
