use solar::config::ImportRemapping;
use solar::interface::SourceMap;
use solar::interface::source_map::FileResolver;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Resolves Solidity import paths to absolute filesystem paths.
///
/// Uses solar's `FileResolver` for spec-compliant resolution, with project
/// configuration parsed from `foundry.toml`, `remappings.txt`, or
/// `forge remappings`.
pub struct ImportResolver {
    project_root: PathBuf,
    remappings: Vec<ImportRemapping>,
    include_paths: Vec<PathBuf>,
    source_map: Arc<SourceMap>,
    /// Cache of resolved import paths: (import_path, from_dir) → result.
    resolve_cache: std::collections::HashMap<(String, PathBuf), Option<PathBuf>>,
}

impl ImportResolver {
    /// Create a resolver by finding the project root (walks up from `any_file`
    /// looking for `foundry.toml`) and parsing remappings.
    pub fn new(any_file: &Path) -> Self {
        let project_root = find_project_root(any_file)
            .unwrap_or_else(|| any_file.parent().unwrap_or(Path::new(".")).to_path_buf());
        Self::with_root(project_root)
    }

    /// Create a resolver with an explicit project root.
    pub fn with_root(project_root: PathBuf) -> Self {
        let remappings = load_remappings(&project_root);
        let include_paths = build_include_paths(&project_root);
        let source_map = Arc::new(SourceMap::empty());
        Self {
            project_root,
            remappings,
            include_paths,
            source_map,
            resolve_cache: std::collections::HashMap::new(),
        }
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// The parsed remappings (for sharing with solar checker).
    pub fn remappings(&self) -> &[ImportRemapping] {
        &self.remappings
    }

    /// The include paths (for sharing with solar checker).
    pub fn include_paths(&self) -> &[PathBuf] {
        &self.include_paths
    }

    /// Resolve an import path to an absolute filesystem path.
    ///
    /// `import_path` is the raw string from `import "..."` (without quotes).
    /// `from_file` is the absolute path of the file containing the import.
    pub fn resolve(&mut self, import_path: &str, from_file: &Path) -> Option<PathBuf> {
        let import_path = import_path.trim_matches(|c| c == '"' || c == '\'');
        let from_dir = from_file.parent().unwrap_or(Path::new(".")).to_path_buf();
        let key = (import_path.to_string(), from_dir);

        if let Some(cached) = self.resolve_cache.get(&key) {
            return cached.clone();
        }

        let mut resolver = FileResolver::new(&self.source_map);

        // Configure with our project settings.
        if let Ok(abs_root) = std::fs::canonicalize(&self.project_root) {
            resolver.set_current_dir(&abs_root);
        } else if self.project_root.is_absolute() {
            resolver.set_current_dir(&self.project_root);
        }
        resolver.add_import_remappings(self.remappings.iter().cloned());
        resolver.add_include_paths(self.include_paths.iter().cloned());

        let result = resolver.resolve_file(Path::new(import_path), Some(from_file));
        let resolved = match result {
            Ok(source_file) => source_file.name.as_real().map(|p| p.to_path_buf()),
            Err(_) => None,
        };

        self.resolve_cache.insert(key, resolved.clone());
        resolved
    }
}

/// Walk up from `start` looking for a directory containing `foundry.toml`,
/// `hardhat.config.js`, `hardhat.config.ts`, or `package.json`.
fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };

    let markers = [
        "foundry.toml",
        "hardhat.config.js",
        "hardhat.config.ts",
        "hardhat.config.cjs",
        "hardhat.config.mjs",
        "brownie-config.yaml",
        "truffle-config.js",
        "ape-config.yaml",
    ];

    loop {
        for marker in &markers {
            if dir.join(marker).exists() {
                return Some(dir);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Load remappings from available sources, in priority order:
/// 1. `forge remappings` command (auto-detects everything including git submodules)
/// 2. `foundry.toml` [profile.default] remappings array
/// 3. `remappings.txt` file
fn load_remappings(project_root: &Path) -> Vec<ImportRemapping> {
    if let Some(r) = try_forge_remappings(project_root) {
        if !r.is_empty() {
            return r;
        }
    }
    if let Some(r) = parse_foundry_toml_remappings(project_root) {
        if !r.is_empty() {
            return r;
        }
    }
    if let Some(r) = parse_remappings_txt(project_root) {
        if !r.is_empty() {
            return r;
        }
    }
    Vec::new()
}

/// Run `forge remappings` to auto-detect all remappings (including git submodules).
fn try_forge_remappings(project_root: &Path) -> Option<Vec<ImportRemapping>> {
    // Only try if foundry.toml exists (it's a Foundry project).
    if !project_root.join("foundry.toml").exists() {
        return None;
    }

    let output = std::process::Command::new("forge")
        .arg("remappings")
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let remappings: Vec<ImportRemapping> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| line.trim().parse::<ImportRemapping>().ok())
        .collect();

    Some(remappings)
}

/// Parse remappings from `foundry.toml` using the `toml` crate.
fn parse_foundry_toml_remappings(project_root: &Path) -> Option<Vec<ImportRemapping>> {
    let toml_path = project_root.join("foundry.toml");
    let content = std::fs::read_to_string(toml_path).ok()?;
    let table: toml::Table = content.parse().ok()?;

    // Check top-level `remappings` first.
    if let Some(remappings) = extract_remappings_from_table(&table) {
        return Some(remappings);
    }

    // Check `[profile.default]` section.
    if let Some(profile) = table.get("profile").and_then(|p| p.as_table()) {
        if let Some(default) = profile.get("default").and_then(|d| d.as_table()) {
            if let Some(remappings) = extract_remappings_from_table(default) {
                return Some(remappings);
            }
        }
    }

    None
}

/// Extract remappings array from a TOML table.
fn extract_remappings_from_table(table: &toml::Table) -> Option<Vec<ImportRemapping>> {
    let arr = table.get("remappings")?.as_array()?;
    let remappings: Vec<ImportRemapping> = arr
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(|s| s.parse::<ImportRemapping>().ok())
        .collect();
    if remappings.is_empty() {
        None
    } else {
        Some(remappings)
    }
}

/// Parse remappings from `remappings.txt` (one `[context:]prefix=target` per line).
fn parse_remappings_txt(project_root: &Path) -> Option<Vec<ImportRemapping>> {
    let txt_path = project_root.join("remappings.txt");
    let content = std::fs::read_to_string(txt_path).ok()?;

    let remappings: Vec<ImportRemapping> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| line.parse::<ImportRemapping>().ok())
        .collect();

    if remappings.is_empty() {
        None
    } else {
        Some(remappings)
    }
}

/// Build include paths from project root.
fn build_include_paths(project_root: &Path) -> Vec<PathBuf> {
    let candidates = [project_root.join("node_modules"), project_root.join("lib")];
    candidates.into_iter().filter(|p| p.is_dir()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_find_project_root_foundry() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("foundry.toml"), "[profile.default]").unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        let file = tmp.path().join("src/Foo.sol");
        fs::write(&file, "").unwrap();

        let root = find_project_root(&file).unwrap();
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn test_find_project_root_hardhat() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("hardhat.config.js"), "module.exports = {}").unwrap();
        fs::create_dir_all(tmp.path().join("contracts")).unwrap();
        let file = tmp.path().join("contracts/Foo.sol");
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

        let mut resolver = ImportResolver::with_root(tmp.path().to_path_buf());
        let resolved = resolver.resolve("./Bar.sol", &foo).unwrap();
        assert!(
            resolved.ends_with("src/Bar.sol"),
            "expected path ending with src/Bar.sol, got: {resolved:?}"
        );
    }

    #[test]
    fn test_resolve_remapping_foundry_toml() {
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
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::write(&from_file, "").unwrap();

        let mut resolver = ImportResolver::with_root(tmp.path().to_path_buf());
        let resolved = resolver
            .resolve("@oz/contracts/Token.sol", &from_file)
            .unwrap();
        assert!(
            resolved.ends_with("lib/oz/contracts/Token.sol"),
            "expected path ending with lib/oz/contracts/Token.sol, got: {resolved:?}"
        );
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
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::write(&from_file, "").unwrap();

        let mut resolver = ImportResolver::with_root(tmp.path().to_path_buf());
        let resolved = resolver.resolve("forge-std/Test.sol", &from_file).unwrap();
        assert!(
            resolved.ends_with("lib/forge-std/src/Test.sol"),
            "expected path ending with lib/forge-std/src/Test.sol, got: {resolved:?}"
        );
    }

    #[test]
    fn test_resolve_node_modules() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("node_modules/@openzeppelin/contracts")).unwrap();
        let target = tmp
            .path()
            .join("node_modules/@openzeppelin/contracts/ERC20.sol");
        fs::write(&target, "").unwrap();
        let from_file = tmp.path().join("contracts/Foo.sol");
        fs::create_dir_all(tmp.path().join("contracts")).unwrap();
        fs::write(&from_file, "").unwrap();

        let mut resolver = ImportResolver::with_root(tmp.path().to_path_buf());
        let resolved = resolver
            .resolve("@openzeppelin/contracts/ERC20.sol", &from_file)
            .unwrap();
        assert!(
            resolved.ends_with("node_modules/@openzeppelin/contracts/ERC20.sol"),
            "expected path ending with node_modules/@openzeppelin/contracts/ERC20.sol, got: {resolved:?}"
        );
    }

    #[test]
    fn test_toml_parsing_top_level_remappings() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_content = r#"
remappings = [
    "@oz/=lib/oz/",
    "forge-std/=lib/forge-std/src/",
]
"#;
        fs::write(tmp.path().join("foundry.toml"), toml_content).unwrap();
        let remappings = parse_foundry_toml_remappings(tmp.path()).unwrap();
        assert_eq!(remappings.len(), 2);
        assert_eq!(remappings[0].prefix, "@oz/");
        assert_eq!(remappings[0].path, "lib/oz/");
        assert_eq!(remappings[1].prefix, "forge-std/");
        assert_eq!(remappings[1].path, "lib/forge-std/src/");
    }

    #[test]
    fn test_context_remapping() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("remappings.txt"), "src:@oz/=lib/oz/\n").unwrap();
        let remappings = parse_remappings_txt(tmp.path()).unwrap();
        assert_eq!(remappings.len(), 1);
        assert_eq!(remappings[0].context, "src");
        assert_eq!(remappings[0].prefix, "@oz/");
        assert_eq!(remappings[0].path, "lib/oz/");
    }
}
