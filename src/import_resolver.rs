use solar::config::ImportRemapping;
use solar::interface::SourceMap;
use solar::interface::source_map::FileResolver;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Discovers project configuration (remappings, include paths, base path) and
/// feeds it to solar's `FileResolver`, which implements the full Solidity path
/// resolution spec.
pub struct ImportResolver {
    project_root: PathBuf,
    abs_root: PathBuf,
    remappings: Vec<ImportRemapping>,
    include_paths: Vec<PathBuf>,
    source_map: Arc<SourceMap>,
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
        let abs_root = std::fs::canonicalize(&project_root).unwrap_or_else(|_| {
            if project_root.is_absolute() {
                project_root.clone()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(&project_root))
                    .unwrap_or_else(|_| project_root.clone())
            }
        });
        Self {
            project_root,
            abs_root,
            remappings,
            include_paths,
            source_map,
            resolve_cache: std::collections::HashMap::new(),
        }
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn remappings(&self) -> &[ImportRemapping] {
        &self.remappings
    }

    pub fn include_paths(&self) -> &[PathBuf] {
        &self.include_paths
    }

    /// Resolve an import path to an absolute filesystem path.
    ///
    /// Delegates entirely to solar's `FileResolver::resolve_file` which
    /// implements the full Solidity path resolution spec.
    pub fn resolve(&mut self, import_path: &str, from_file: &Path) -> Option<PathBuf> {
        let import_path = import_path.trim_matches(|c| c == '"' || c == '\'');
        let from_dir = from_file.parent().unwrap_or(Path::new(".")).to_path_buf();
        let key = (import_path.to_string(), from_dir);

        if let Some(cached) = self.resolve_cache.get(&key) {
            return cached.clone();
        }

        let mut resolver = FileResolver::new(&self.source_map);
        resolver.set_current_dir(&self.abs_root);
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

// ---------------------------------------------------------------------------
// Project discovery
// ---------------------------------------------------------------------------

/// Walk up from `start` looking for a project config marker.
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

// ---------------------------------------------------------------------------
// Remapping discovery
// ---------------------------------------------------------------------------

/// Load remappings from available sources, in priority order:
/// 1. `forge remappings` (auto-detects everything including git submodules)
/// 2. `foundry.toml` remappings array
/// 3. `remappings.txt`
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

fn try_forge_remappings(project_root: &Path) -> Option<Vec<ImportRemapping>> {
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
    Some(
        stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| line.trim().parse::<ImportRemapping>().ok())
            .collect(),
    )
}

fn parse_foundry_toml_remappings(project_root: &Path) -> Option<Vec<ImportRemapping>> {
    let content = std::fs::read_to_string(project_root.join("foundry.toml")).ok()?;
    let table: toml::Table = content.parse().ok()?;

    if let Some(r) = extract_remappings_from_table(&table) {
        return Some(r);
    }
    if let Some(default) = table
        .get("profile")
        .and_then(|p| p.as_table())
        .and_then(|p| p.get("default"))
        .and_then(|d| d.as_table())
    {
        if let Some(r) = extract_remappings_from_table(default) {
            return Some(r);
        }
    }
    None
}

fn extract_remappings_from_table(table: &toml::Table) -> Option<Vec<ImportRemapping>> {
    let remappings: Vec<ImportRemapping> = table
        .get("remappings")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(|s| s.parse::<ImportRemapping>().ok())
        .collect();
    if remappings.is_empty() { None } else { Some(remappings) }
}

fn parse_remappings_txt(project_root: &Path) -> Option<Vec<ImportRemapping>> {
    let content = std::fs::read_to_string(project_root.join("remappings.txt")).ok()?;
    let remappings: Vec<ImportRemapping> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| line.parse::<ImportRemapping>().ok())
        .collect();
    if remappings.is_empty() { None } else { Some(remappings) }
}

// ---------------------------------------------------------------------------
// Include path discovery
// ---------------------------------------------------------------------------

/// Build the include paths that solar needs for non-relative, non-remapped
/// imports. Reads `libs` from `foundry.toml` when present, and walks up the
/// directory tree to find `node_modules/` directories (Node.js-style).
fn build_include_paths(project_root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Foundry: honour the `libs` config (defaults to ["lib"]).
    for lib in parse_foundry_toml_libs(project_root) {
        let p = project_root.join(&lib);
        if p.is_dir() {
            paths.push(p);
        }
    }

    // Node.js / Hardhat: walk up collecting node_modules/ directories.
    let mut dir = project_root.to_path_buf();
    loop {
        let nm = dir.join("node_modules");
        if nm.is_dir() && !paths.contains(&nm) {
            paths.push(nm);
        }
        if !dir.pop() {
            break;
        }
    }

    paths
}

/// Parse `libs` from `foundry.toml`. Returns Foundry's default `["lib"]` when
/// the file exists but doesn't specify `libs`. Returns empty vec when there is
/// no `foundry.toml` (not a Foundry project).
fn parse_foundry_toml_libs(project_root: &Path) -> Vec<String> {
    let content = match std::fs::read_to_string(project_root.join("foundry.toml")) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(_) => return vec!["lib".to_string()],
    };

    if let Some(libs) = extract_string_array(&table, "libs") {
        return libs;
    }
    if let Some(libs) = table
        .get("profile")
        .and_then(|p| p.as_table())
        .and_then(|p| p.get("default"))
        .and_then(|d| d.as_table())
        .and_then(|d| extract_string_array(d, "libs"))
    {
        return libs;
    }

    vec!["lib".to_string()]
}

fn extract_string_array(table: &toml::Table, key: &str) -> Option<Vec<String>> {
    let values: Vec<String> = table
        .get(key)?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if values.is_empty() { None } else { Some(values) }
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

    #[test]
    fn test_foundry_toml_custom_libs() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_content = r#"
[profile.default]
libs = ["dependencies", "node_modules"]
"#;
        fs::write(tmp.path().join("foundry.toml"), toml_content).unwrap();
        fs::create_dir_all(tmp.path().join("dependencies")).unwrap();
        fs::create_dir_all(tmp.path().join("node_modules")).unwrap();

        let libs = parse_foundry_toml_libs(tmp.path());
        assert_eq!(libs, vec!["dependencies", "node_modules"]);

        let include_paths = build_include_paths(tmp.path());
        assert!(
            include_paths.contains(&tmp.path().join("dependencies")),
            "expected dependencies/ in include paths, got: {include_paths:?}"
        );
    }

    #[test]
    fn test_foundry_toml_default_libs() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("foundry.toml"), "[profile.default]\n").unwrap();
        fs::create_dir_all(tmp.path().join("lib")).unwrap();

        let libs = parse_foundry_toml_libs(tmp.path());
        assert_eq!(libs, vec!["lib"]);

        let include_paths = build_include_paths(tmp.path());
        assert!(
            include_paths.contains(&tmp.path().join("lib")),
            "expected lib/ in include paths, got: {include_paths:?}"
        );
    }

    #[test]
    fn test_foundry_toml_top_level_libs() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_content = r#"
libs = ["custom-deps"]
"#;
        fs::write(tmp.path().join("foundry.toml"), toml_content).unwrap();
        fs::create_dir_all(tmp.path().join("custom-deps")).unwrap();

        let libs = parse_foundry_toml_libs(tmp.path());
        assert_eq!(libs, vec!["custom-deps"]);
    }

    #[test]
    fn test_resolve_custom_libs_path() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_content = r#"
[profile.default]
libs = ["dependencies"]
"#;
        fs::write(tmp.path().join("foundry.toml"), toml_content).unwrap();
        fs::create_dir_all(tmp.path().join("dependencies/forge-std/src")).unwrap();
        let target = tmp.path().join("dependencies/forge-std/src/Test.sol");
        fs::write(&target, "").unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        let from_file = tmp.path().join("src/Foo.sol");
        fs::write(&from_file, "").unwrap();

        fs::write(
            tmp.path().join("remappings.txt"),
            "forge-std/=dependencies/forge-std/src/\n",
        )
        .unwrap();

        let mut resolver = ImportResolver::with_root(tmp.path().to_path_buf());
        let resolved = resolver.resolve("forge-std/Test.sol", &from_file).unwrap();
        assert!(
            resolved.ends_with("dependencies/forge-std/src/Test.sol"),
            "expected path ending with dependencies/forge-std/src/Test.sol, got: {resolved:?}"
        );
    }

    #[test]
    fn test_no_foundry_toml_no_default_libs() {
        let tmp = tempfile::tempdir().unwrap();
        let libs = parse_foundry_toml_libs(tmp.path());
        assert!(libs.is_empty(), "expected no libs for non-Foundry project, got: {libs:?}");
    }
}
