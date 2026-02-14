use std::path::{Path, PathBuf};

/// A single remapping entry: `prefix=target` (with optional context).
#[derive(Debug, Clone)]
pub struct Remapping {
    pub prefix: String,
    pub target: String,
}

/// Resolves Solidity import paths to absolute filesystem paths.
///
/// Supports:
/// - Relative paths (`./Foo.sol`, `../lib/Bar.sol`)
/// - Foundry remappings from `foundry.toml` or `remappings.txt`
/// - Fallback to `lib/` and `node_modules/`
pub struct ImportResolver {
    project_root: PathBuf,
    remappings: Vec<Remapping>,
}

impl ImportResolver {
    /// Create a resolver by finding the project root (walks up from `any_file`
    /// looking for `foundry.toml`) and parsing remappings.
    pub fn new(any_file: &Path) -> Self {
        let project_root = find_project_root(any_file)
            .unwrap_or_else(|| any_file.parent().unwrap_or(Path::new(".")).to_path_buf());
        let mut remappings = parse_remappings(&project_root);
        // Sort longest prefix first for greedy matching
        remappings.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));
        Self {
            project_root,
            remappings,
        }
    }

    /// Create a resolver with an explicit project root (useful for tests).
    pub fn with_root(project_root: PathBuf) -> Self {
        let mut remappings = parse_remappings(&project_root);
        remappings.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));
        Self {
            project_root,
            remappings,
        }
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Resolve an import path to an absolute filesystem path.
    ///
    /// `import_path` is the raw string from `import "..."` (without quotes).
    /// `from_file` is the absolute path of the file containing the import.
    pub fn resolve(&self, import_path: &str, from_file: &Path) -> Option<PathBuf> {
        let import_path = import_path.trim_matches(|c| c == '"' || c == '\'');

        // 1. Relative path
        if import_path.starts_with("./") || import_path.starts_with("../") {
            let base_dir = from_file.parent()?;
            let resolved = base_dir.join(import_path);
            let canonical = resolved.canonicalize().ok()?;
            if canonical.exists() {
                return Some(canonical);
            }
            return None;
        }

        // 2. Remapping prefix match (longest first)
        for remapping in &self.remappings {
            if import_path.starts_with(&remapping.prefix) {
                let remainder = &import_path[remapping.prefix.len()..];
                let target_base = if Path::new(&remapping.target).is_absolute() {
                    PathBuf::from(&remapping.target)
                } else {
                    self.project_root.join(&remapping.target)
                };
                let resolved = target_base.join(remainder);
                if resolved.exists() {
                    return resolved.canonicalize().ok();
                }
            }
        }

        // 3. Fallback: try lib/ then node_modules/
        let lib_path = self.project_root.join("lib").join(import_path);
        if lib_path.exists() {
            return lib_path.canonicalize().ok();
        }

        let node_modules_path = self.project_root.join("node_modules").join(import_path);
        if node_modules_path.exists() {
            return node_modules_path.canonicalize().ok();
        }

        // 4. Try as absolute from project root
        let from_root = self.project_root.join(import_path);
        if from_root.exists() {
            return from_root.canonicalize().ok();
        }

        None
    }
}

/// Walk up from `start` looking for a directory containing `foundry.toml`.
fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        if dir.join("foundry.toml").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Parse remappings from `foundry.toml` or `remappings.txt`.
fn parse_remappings(project_root: &Path) -> Vec<Remapping> {
    // Try foundry.toml first
    if let Some(remappings) = parse_foundry_toml_remappings(project_root) {
        return remappings;
    }
    // Fall back to remappings.txt
    if let Some(remappings) = parse_remappings_txt(project_root) {
        return remappings;
    }
    Vec::new()
}

/// Parse remappings from `foundry.toml`.
/// Looks for `remappings = ["prefix=target", ...]` in any profile section.
fn parse_foundry_toml_remappings(project_root: &Path) -> Option<Vec<Remapping>> {
    let toml_path = project_root.join("foundry.toml");
    let content = std::fs::read_to_string(toml_path).ok()?;

    let mut remappings = Vec::new();
    let mut in_remappings = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("remappings") && trimmed.contains('=') {
            in_remappings = true;
            // Check if it's a single-line array: remappings = ["a=b", "c=d"]
            if let Some(bracket_start) = trimmed.find('[') {
                let rest = &trimmed[bracket_start..];
                if let Some(bracket_end) = rest.find(']') {
                    let array_content = &rest[1..bracket_end];
                    for entry in array_content.split(',') {
                        if let Some(r) = parse_remapping_entry(entry) {
                            remappings.push(r);
                        }
                    }
                    in_remappings = false;
                }
            }
            continue;
        }

        if in_remappings {
            if trimmed.starts_with(']') {
                in_remappings = false;
                continue;
            }
            if let Some(r) = parse_remapping_entry(trimmed) {
                remappings.push(r);
            }
        }
    }

    if remappings.is_empty() {
        None
    } else {
        Some(remappings)
    }
}

/// Parse remappings from `remappings.txt` (one `prefix=target` per line).
fn parse_remappings_txt(project_root: &Path) -> Option<Vec<Remapping>> {
    let txt_path = project_root.join("remappings.txt");
    let content = std::fs::read_to_string(txt_path).ok()?;

    let remappings: Vec<Remapping> = content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let (prefix, target) = trimmed.split_once('=')?;
            Some(Remapping {
                prefix: prefix.trim().to_string(),
                target: target.trim().to_string(),
            })
        })
        .collect();

    if remappings.is_empty() {
        None
    } else {
        Some(remappings)
    }
}

/// Parse a single remapping entry like `"@openzeppelin/=lib/openzeppelin-contracts/"`.
fn parse_remapping_entry(entry: &str) -> Option<Remapping> {
    let trimmed = entry
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == ',');
    if trimmed.is_empty() {
        return None;
    }
    let (prefix, target) = trimmed.split_once('=')?;
    let prefix = prefix.trim();
    let target = target.trim();
    if prefix.is_empty() || target.is_empty() {
        return None;
    }
    Some(Remapping {
        prefix: prefix.to_string(),
        target: target.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_remapping_entry() {
        let r = parse_remapping_entry(r#""@openzeppelin/=lib/openzeppelin-contracts/""#).unwrap();
        assert_eq!(r.prefix, "@openzeppelin/");
        assert_eq!(r.target, "lib/openzeppelin-contracts/");
    }

    #[test]
    fn test_parse_remapping_entry_no_quotes() {
        let r = parse_remapping_entry("forge-std/=lib/forge-std/src/").unwrap();
        assert_eq!(r.prefix, "forge-std/");
        assert_eq!(r.target, "lib/forge-std/src/");
    }

    #[test]
    fn test_parse_remapping_entry_empty() {
        assert!(parse_remapping_entry("").is_none());
        assert!(parse_remapping_entry("   ").is_none());
    }

    #[test]
    fn test_find_project_root() {
        // Use a temp dir with foundry.toml
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("foundry.toml"), "[profile.default]").unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        let file = tmp.path().join("src/Foo.sol");
        fs::write(&file, "").unwrap();

        let root = find_project_root(&file).unwrap();
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn test_resolve_relative() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("foundry.toml"), "").unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        let foo = tmp.path().join("src/Foo.sol");
        let bar = tmp.path().join("src/Bar.sol");
        fs::write(&foo, "").unwrap();
        fs::write(&bar, "").unwrap();

        let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
        let resolved = resolver.resolve("./Bar.sol", &foo).unwrap();
        assert_eq!(resolved, bar.canonicalize().unwrap());
    }

    #[test]
    fn test_resolve_remapping() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_content = r#"
[profile.default]
remappings = ["@oz/=lib/oz/"]
"#;
        fs::write(tmp.path().join("foundry.toml"), toml_content).unwrap();
        fs::create_dir_all(tmp.path().join("lib/oz/contracts")).unwrap();
        let target = tmp.path().join("lib/oz/contracts/Token.sol");
        fs::write(&target, "").unwrap();
        let from_file = tmp.path().join("src/Foo.sol");

        let resolver = ImportResolver::new(&from_file);
        let resolved = resolver
            .resolve("@oz/contracts/Token.sol", &from_file)
            .unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
    }

    #[test]
    fn test_resolve_remappings_txt() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("remappings.txt"),
            "forge-std/=lib/forge-std/src/\n",
        )
        .unwrap();
        fs::create_dir_all(tmp.path().join("lib/forge-std/src")).unwrap();
        let target = tmp.path().join("lib/forge-std/src/Test.sol");
        fs::write(&target, "").unwrap();
        let from_file = tmp.path().join("src/Foo.sol");

        let resolver = ImportResolver::with_root(tmp.path().to_path_buf());
        let resolved = resolver.resolve("forge-std/Test.sol", &from_file).unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
    }
}
