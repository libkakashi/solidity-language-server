use solar::config::ImportRemapping;
use solar::interface::SourceMap;
use solar::interface::source_map::FileResolver;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
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
            Err(ref e) => {
                tracing::warn!(
                    import = import_path,
                    from = %from_file.display(),
                    err = %e,
                    "import resolution failed"
                );
                None
            }
        };

        self.resolve_cache.insert(key, resolved.clone());
        resolved
    }

    /// Log the loaded configuration (for debugging resolution issues).
    pub fn log_config(&self) {
        tracing::info!(
            root = %self.project_root.display(),
            abs_root = %self.abs_root.display(),
            remappings = self.remappings.len(),
            include_paths = self.include_paths.len(),
            "ImportResolver config"
        );
        for r in &self.remappings {
            tracing::info!(
                context = %r.context,
                prefix = %r.prefix,
                target = %r.path,
                "  remapping"
            );
        }
        for p in &self.include_paths {
            tracing::info!(path = %p.display(), "  include_path");
        }
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
/// 4. Auto-detect from lib directories (mimics what forge does)
fn load_remappings(project_root: &Path) -> Vec<ImportRemapping> {
    if let Some(r) = try_forge_remappings(project_root) {
        if !r.is_empty() {
            tracing::info!(count = r.len(), "loaded remappings from `forge remappings`");
            return r;
        }
    }
    if let Some(r) = parse_foundry_toml_remappings(project_root) {
        if !r.is_empty() {
            tracing::info!(count = r.len(), "loaded remappings from foundry.toml");
            return r;
        }
    }
    if let Some(r) = parse_remappings_txt(project_root) {
        if !r.is_empty() {
            tracing::info!(count = r.len(), "loaded remappings from remappings.txt");
            return r;
        }
    }
    // Fallback: auto-detect remappings by scanning lib directories.
    // This mimics what `forge remappings` does — if a library has a `src/`
    // subdirectory, generate `libname/=lib/libname/src/`.
    let r = auto_detect_remappings(project_root);
    if !r.is_empty() {
        tracing::info!(count = r.len(), "auto-detected remappings from lib directories");
    } else {
        tracing::info!("no remappings found for project");
    }
    r
}

fn try_forge_remappings(project_root: &Path) -> Option<Vec<ImportRemapping>> {
    if !project_root.join("foundry.toml").exists() {
        return None;
    }

    let forge_bin = find_forge_binary();
    let output = match std::process::Command::new(&forge_bin)
        .arg("remappings")
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(
                forge = %forge_bin,
                err = %e,
                "failed to run `forge remappings` — is forge installed and in PATH?"
            );
            return None;
        }
    };

    if !output.status.success() {
        tracing::warn!(
            status = ?output.status,
            "forge remappings returned non-zero exit code"
        );
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

/// Find the forge binary, checking common installation paths when it's not
/// in PATH (common when the LSP is spawned by an editor).
fn find_forge_binary() -> String {
    // Check common foundry installation paths first, since the LSP process
    // often inherits a minimal PATH from the editor that doesn't include
    // ~/.foundry/bin.
    if let Some(home) = std::env::var_os("HOME") {
        let foundry_bin = PathBuf::from(home).join(".foundry/bin/forge");
        if foundry_bin.exists() {
            return foundry_bin.to_string_lossy().to_string();
        }
    }
    // Fallback — let the OS try to find it via PATH.
    "forge".to_string()
}

/// Auto-detect remappings by scanning library directories.
///
/// Ported from foundry-compilers `Remapping::find_many()`. This recursively
/// scans each lib directory to find Solidity files and generates remappings
/// using the same rules as `forge remappings`:
///
/// - If a directory contains `.sol` files directly, it's a remapping candidate
/// - `src/` and `contracts/` are "source barriers" — the remapping target
///   points to them (e.g. `repo/=lib/repo/src/`)
/// - `lib/` and `node_modules/` are "library barriers" — they open new
///   nesting windows for discovering nested dependencies
/// - Nested `@scoped/packages` are unified to their common ancestor
fn auto_detect_remappings(project_root: &Path) -> Vec<ImportRemapping> {
    let mut all = Vec::new();
    for lib_dir_name in parse_foundry_toml_libs(project_root) {
        let lib_dir = project_root.join(&lib_dir_name);
        if !lib_dir.is_dir() {
            continue;
        }
        let found = scan_lib_dir_remappings(&lib_dir);
        for (name, abs_path) in found {
            // Convert the absolute target path to a path relative to project root.
            let target = abs_path
                .strip_prefix(project_root)
                .unwrap_or(&abs_path);
            let target_str = format!("{}/", target.display());
            tracing::debug!(prefix = %name, target = %target_str, "auto-detected remapping");
            all.push(ImportRemapping {
                context: String::new(),
                prefix: name,
                path: target_str,
            });
        }
    }
    // Also scan node_modules at the project root if not already a lib dir.
    let nm = project_root.join("node_modules");
    if nm.is_dir() {
        let found = scan_lib_dir_remappings(&nm);
        for (name, abs_path) in found {
            // Skip if we already have a remapping with the same prefix.
            if all.iter().any(|r| r.prefix == name) {
                continue;
            }
            let target = abs_path.strip_prefix(project_root).unwrap_or(&abs_path);
            let target_str = format!("{}/", target.display());
            all.push(ImportRemapping {
                context: String::new(),
                prefix: name,
                path: target_str,
            });
        }
    }
    all
}

// ---------------------------------------------------------------------------
// Ported from foundry-compilers: Remapping::find_many()
// https://github.com/foundry-rs/compilers/blob/main/crates/artifacts/solc/src/remappings/find.rs
// ---------------------------------------------------------------------------

const DAPPTOOLS_CONTRACTS_DIR: &str = "src";
const JS_CONTRACTS_DIR: &str = "contracts";
const DAPPTOOLS_LIB_DIR: &str = "lib";
const JS_LIB_DIR: &str = "node_modules";

/// Scan a single library directory (e.g. `lib/` or `node_modules/`) and return
/// a map of `prefix/ => absolute_source_dir`.
fn scan_lib_dir_remappings(dir: &Path) -> HashMap<String, PathBuf> {
    fn insert_prioritized(mappings: &mut HashMap<String, PathBuf>, key: String, path: PathBuf) {
        match mappings.entry(key) {
            Entry::Occupied(mut e) => {
                if e.get().components().count() > path.components().count()
                    || (path.ends_with(DAPPTOOLS_CONTRACTS_DIR)
                        && !e.get().ends_with(DAPPTOOLS_CONTRACTS_DIR))
                {
                    e.insert(path);
                }
            }
            Entry::Vacant(e) => {
                e.insert(path);
            }
        }
    }

    let mut all_remappings = HashMap::new();
    let is_inside_node_modules = dir.ends_with("node_modules");

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return all_remappings,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Skip hidden directories.
        if entry
            .file_name()
            .to_str()
            .is_some_and(|s| s.starts_with('.'))
        {
            continue;
        }

        let candidates = find_remapping_candidates(&path, &path, 0, is_inside_node_modules);
        for candidate in candidates {
            if let Some(name) = candidate.window_start.file_name().and_then(|s| s.to_str()) {
                insert_prioritized(
                    &mut all_remappings,
                    format!("{name}/"),
                    candidate.source_dir,
                );
            }
        }
    }

    all_remappings
}

#[derive(Debug, Clone)]
struct RemappingCandidate {
    /// Directory that opened the current window.
    window_start: PathBuf,
    /// Directory that contains the solidity file.
    source_dir: PathBuf,
    /// Nesting level (incremented at each lib barrier).
    window_level: usize,
}

impl RemappingCandidate {
    /// Merge candidates at the same nesting level.
    ///
    /// Prefers `src/` directories for dapptools-style; uses the window start
    /// for node_modules-style.
    fn merge_on_same_level(
        candidates: &mut Vec<Self>,
        current_dir: &Path,
        current_level: usize,
        window_start: PathBuf,
        is_inside_node_modules: bool,
    ) {
        // If there's exactly one src-dir candidate, keep only that one.
        if let Some(pos) = candidates
            .iter()
            .enumerate()
            .fold((0, None), |(mut count, mut pos), (idx, c)| {
                if c.source_dir.ends_with(DAPPTOOLS_CONTRACTS_DIR) {
                    count += 1;
                    if count == 1 {
                        pos = Some(idx);
                    } else {
                        pos = None;
                    }
                }
                (count, pos)
            })
            .1
        {
            let c = candidates.remove(pos);
            *candidates = vec![c];
        } else {
            candidates.retain(|c| c.window_level != current_level);

            let source_dir = if is_inside_node_modules {
                window_start.clone()
            } else {
                current_dir.to_path_buf()
            };

            if current_level > 0
                && source_dir == window_start
                && (is_scan_source_dir(&source_dir) || is_scan_lib_dir(&source_dir))
            {
                return;
            }
            candidates.push(Self {
                window_start,
                source_dir,
                window_level: current_level,
            });
        }
    }

    fn source_dir_ends_with_js_source(&self) -> bool {
        self.source_dir.ends_with(JS_CONTRACTS_DIR)
            || self.source_dir.ends_with("contracts/src/")
    }
}

fn is_scan_source_dir(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|p| p.to_str())
        .is_some_and(|name| name == DAPPTOOLS_CONTRACTS_DIR || name == JS_CONTRACTS_DIR)
}

fn is_scan_lib_dir(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|p| p.to_str())
        .is_some_and(|name| name == DAPPTOOLS_LIB_DIR || name == JS_LIB_DIR)
}

/// Recursively find remapping candidates in a directory tree.
fn find_remapping_candidates(
    current_dir: &Path,
    open: &Path,
    current_level: usize,
    is_inside_node_modules: bool,
) -> Vec<RemappingCandidate> {
    let mut is_candidate = false;
    let mut candidates = Vec::new();

    let entries = match std::fs::read_dir(current_dir) {
        Ok(e) => e,
        Err(_) => return candidates,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();

        // Skip hidden entries.
        if name.to_str().is_some_and(|s| s.starts_with('.')) {
            continue;
        }

        if path.is_file() {
            // Found a .sol file directly in this directory.
            if !is_candidate && path.extension().is_some_and(|e| e == "sol") {
                is_candidate = true;
            }
        } else if path.is_dir() {
            // Handle symlinks pointing to parent directories.
            if path.read_link().is_ok() {
                if let Ok(target) = std::fs::canonicalize(&path) {
                    if open.components().count() > target.components().count() {
                        if let Some(ancestor) = common_ancestor(open, &target) {
                            if !ancestor.as_os_str().is_empty() {
                                return Vec::new();
                            }
                        }
                    }
                }
            }

            let dir_name_str = name.to_str().unwrap_or("");
            // Skip test/demo directories.
            if dir_name_str == "tests" || dir_name_str == "test" || dir_name_str == "demo" {
                continue;
            }

            if is_scan_lib_dir(&path) {
                // Library barrier — open a new window.
                candidates.extend(find_remapping_candidates(
                    &path,
                    &path,
                    current_level + 1,
                    is_inside_node_modules,
                ));
            } else {
                // Continue scanning in the current window.
                candidates.extend(find_remapping_candidates(
                    &path,
                    open,
                    current_level,
                    is_inside_node_modules,
                ));
            }
        }
    }

    let window_start = next_nested_window(open, current_dir);

    if is_candidate
        || candidates
            .iter()
            .filter(|c| c.window_level == current_level && c.window_start == window_start)
            .count()
            > 1
    {
        RemappingCandidate::merge_on_same_level(
            &mut candidates,
            current_dir,
            current_level,
            window_start,
            is_inside_node_modules,
        );
    } else if let Some(candidate) =
        candidates.iter_mut().find(|c| c.window_level == current_level)
    {
        let distance = dir_distance(&candidate.window_start, &candidate.source_dir);
        if distance > 1 && candidate.source_dir_ends_with_js_source() {
            candidate.source_dir = window_start;
        } else if !is_scan_source_dir(&candidate.source_dir)
            && candidate.source_dir != candidate.window_start
        {
            candidate.source_dir = last_nested_source_dir(open, &candidate.source_dir);
        }
    }

    candidates
}

fn dir_distance(root: &Path, current: &Path) -> usize {
    if root == current {
        return 0;
    }
    current
        .strip_prefix(root)
        .map(|rem| rem.components().count())
        .unwrap_or(0)
}

fn next_nested_window(root: &Path, current: &Path) -> PathBuf {
    if !is_scan_lib_dir(root) || root == current {
        return root.to_path_buf();
    }
    if let Ok(rem) = current.strip_prefix(root) {
        let mut p = root.to_path_buf();
        for c in rem.components() {
            let next = p.join(c);
            if !is_scan_lib_dir(&next) || !next.ends_with(JS_CONTRACTS_DIR) {
                return next;
            }
            p = next;
        }
    }
    root.to_path_buf()
}

fn last_nested_source_dir(root: &Path, dir: &Path) -> PathBuf {
    if is_scan_source_dir(dir) {
        return dir.to_path_buf();
    }
    let mut p = dir;
    while let Some(parent) = p.parent() {
        if parent == root {
            return root.to_path_buf();
        }
        if is_scan_source_dir(parent) {
            return parent.to_path_buf();
        }
        p = parent;
    }
    root.to_path_buf()
}

/// Find the longest common ancestor of two paths.
fn common_ancestor(a: &Path, b: &Path) -> Option<PathBuf> {
    let mut ret = PathBuf::new();
    let mut found = false;
    for (c1, c2) in a.components().zip(b.components()) {
        if c1 == c2 {
            ret.push(c1);
            found = true;
        } else {
            break;
        }
    }
    if found { Some(ret) } else { None }
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

    /// Reproduces the scenario where forge remappings outputs
    /// `lib-name/=lib/lib-name/src/` and `lib/` is also an include path.
    #[test]
    fn test_forge_style_remapping_with_lib_include_path() {
        let tmp = tempfile::tempdir().unwrap();
        // Foundry project with lib/ as default libs dir.
        fs::write(tmp.path().join("foundry.toml"), "[profile.default]\n").unwrap();

        // Create the library structure: lib/my-lib/src/contracts/Token.sol
        fs::create_dir_all(tmp.path().join("lib/my-lib/src/contracts")).unwrap();
        let target = tmp.path().join("lib/my-lib/src/contracts/Token.sol");
        fs::write(&target, "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.0;").unwrap();

        // Create source file
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        let from_file = tmp.path().join("src/Foo.sol");
        fs::write(&from_file, "").unwrap();

        // Remapping as forge would generate: my-lib/=lib/my-lib/src/
        fs::write(
            tmp.path().join("remappings.txt"),
            "my-lib/=lib/my-lib/src/\n",
        )
        .unwrap();

        let mut resolver = ImportResolver::with_root(tmp.path().to_path_buf());

        // Debug: print what was loaded.
        eprintln!("remappings: {:?}", resolver.remappings());
        eprintln!("include_paths: {:?}", resolver.include_paths());
        eprintln!("abs_root: {:?}", resolver.abs_root);

        // import "my-lib/contracts/Token.sol" should resolve to
        // lib/my-lib/src/contracts/Token.sol
        let resolved = resolver.resolve("my-lib/contracts/Token.sol", &from_file);
        assert!(
            resolved.is_some(),
            "expected to resolve my-lib/contracts/Token.sol, but got None.\n\
             remappings: {:?}\n\
             include_paths: {:?}",
            resolver.remappings(),
            resolver.include_paths(),
        );
        assert!(
            resolved.as_ref().unwrap().ends_with("lib/my-lib/src/contracts/Token.sol"),
            "expected path ending with lib/my-lib/src/contracts/Token.sol, got: {resolved:?}"
        );
    }

    #[test]
    fn test_no_foundry_toml_no_default_libs() {
        let tmp = tempfile::tempdir().unwrap();
        let libs = parse_foundry_toml_libs(tmp.path());
        assert!(libs.is_empty(), "expected no libs for non-Foundry project, got: {libs:?}");
    }

    /// Test auto-detection of remappings from lib directory structure.
    /// The ported foundry logic discovers candidates by scanning for `.sol` files.
    #[test]
    fn test_auto_detect_remappings() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("foundry.toml"), "[profile.default]\n").unwrap();

        // Create libs with src/ subdirectories and .sol files in them.
        fs::create_dir_all(tmp.path().join("lib/forge-std/src")).unwrap();
        fs::write(tmp.path().join("lib/forge-std/src/Test.sol"), "").unwrap();
        fs::create_dir_all(tmp.path().join("lib/openzeppelin-contracts/src")).unwrap();
        fs::write(
            tmp.path().join("lib/openzeppelin-contracts/src/ERC20.sol"),
            "",
        )
        .unwrap();
        // This one has .sol files directly in its root (no src/).
        fs::create_dir_all(tmp.path().join("lib/ds-test")).unwrap();
        fs::write(tmp.path().join("lib/ds-test/test.sol"), "").unwrap();

        let remappings = auto_detect_remappings(tmp.path());
        let by_prefix: std::collections::HashMap<&str, &str> = remappings
            .iter()
            .map(|r| (r.prefix.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(
            by_prefix.get("forge-std/"),
            Some(&"lib/forge-std/src/"),
            "forge-std should map to lib/forge-std/src/"
        );
        assert_eq!(
            by_prefix.get("openzeppelin-contracts/"),
            Some(&"lib/openzeppelin-contracts/src/"),
            "openzeppelin-contracts should map to lib/openzeppelin-contracts/src/"
        );
        assert_eq!(
            by_prefix.get("ds-test/"),
            Some(&"lib/ds-test/"),
            "ds-test (no src/) should map to lib/ds-test/"
        );
    }

    /// End-to-end: auto-detect remappings resolve imports correctly.
    /// The library has a foundry.toml with src config, so auto-detect
    /// should append src/ to the remapping target.
    #[test]
    fn test_auto_detect_remappings_resolve() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("foundry.toml"), "[profile.default]\n").unwrap();

        // Library with its own foundry.toml and src/ directory
        fs::create_dir_all(tmp.path().join("lib/my-lib/src/contracts")).unwrap();
        let target = tmp.path().join("lib/my-lib/src/contracts/Token.sol");
        fs::write(&target, "").unwrap();
        // Library needs its own foundry.toml to trigger src/ detection
        fs::write(
            tmp.path().join("lib/my-lib/foundry.toml"),
            "[profile.default]\nsrc = \"src\"\n",
        )
        .unwrap();

        fs::create_dir_all(tmp.path().join("src")).unwrap();
        let from_file = tmp.path().join("src/Foo.sol");
        fs::write(&from_file, "").unwrap();

        // Use remappings.txt to bypass forge (test auto-detect logic separately above)
        fs::write(
            tmp.path().join("remappings.txt"),
            "my-lib/=lib/my-lib/src/\n",
        )
        .unwrap();

        let mut resolver = ImportResolver::with_root(tmp.path().to_path_buf());

        let resolved = resolver.resolve("my-lib/contracts/Token.sol", &from_file);
        assert!(
            resolved.is_some(),
            "expected to resolve my-lib/contracts/Token.sol, but got None.\n\
             remappings: {:?}\n\
             include_paths: {:?}",
            resolver.remappings(),
            resolver.include_paths(),
        );
        assert!(
            resolved.as_ref().unwrap().ends_with("lib/my-lib/src/contracts/Token.sol"),
            "expected path ending with lib/my-lib/src/contracts/Token.sol, got: {resolved:?}"
        );
    }
}
