use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};
use tree_sitter::Node;

use crate::import_resolver::ImportResolver;
use crate::parser::TsParser;

type HashMap<K, V> = FxHashMap<K, V>;

// ---------------------------------------------------------------------------
// Path interning — every file gets a small integer FileId instead of
// duplicating PathBuf everywhere.  (Fix #10)
// ---------------------------------------------------------------------------

pub type FileId = u32;

#[derive(Debug, Default)]
pub struct PathInterner {
    to_id: HashMap<PathBuf, FileId>,
    to_path: Vec<PathBuf>,
}

impl PathInterner {
    pub fn get_or_intern(&mut self, path: &Path) -> FileId {
        if let Some(&id) = self.to_id.get(path) {
            return id;
        }
        let id = self.to_path.len() as FileId;
        self.to_path.push(path.to_path_buf());
        self.to_id.insert(path.to_path_buf(), id);
        id
    }

    pub fn resolve(&self, id: FileId) -> &Path {
        &self.to_path[id as usize]
    }

    pub fn lookup(&self, path: &Path) -> Option<FileId> {
        self.to_id.get(path).copied()
    }
}

// ---------------------------------------------------------------------------
// Core types — slimmed down (Fixes #9, #10, #12)
// ---------------------------------------------------------------------------

/// Stable identifier for a declaration: interned file id + byte offset.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct DeclId {
    pub file: FileId,
    pub byte_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Contract,
    Interface,
    Library,
    Function,
    Constructor,
    FallbackReceive,
    Modifier,
    Event,
    Error,
    Struct,
    Enum,
    EnumValue,
    StateVariable,
    LocalVariable,
    Parameter,
    Constant,
    UserDefinedType,
    ImportAlias,
}

/// Extra data only needed for certain declaration kinds. (Fix #9)
/// Simple declarations (variables, parameters) carry no extra weight.
#[derive(Debug, Clone, Default)]
pub struct DeclExtras {
    /// For functions/events/errors: parameter list as (type, name).
    pub parameters: Vec<(String, String)>,
    /// For functions: return parameters.
    pub return_parameters: Vec<(String, String)>,
    /// For contracts/interfaces: inherited type names.
    pub base_contracts: Vec<String>,
    /// For structs: field info. For contracts: member declarations.
    pub members: Vec<MemberInfo>,
    /// For enums: value names.
    pub enum_values: Vec<String>,
}

/// A single declaration extracted from the CST.
#[derive(Debug, Clone)]
pub struct Declaration {
    pub id: DeclId,
    pub name: String,
    pub kind: DeclKind,
    /// Byte range of the entire declaration node.
    pub full_range: (usize, usize),
    /// Byte range of just the name identifier.
    pub name_range: (usize, usize),
    /// Scope this declaration lives in.
    pub scope: ScopeId,
    /// Syntactic type text (e.g. "uint256", "address payable").
    pub type_text: Option<String>,
    pub visibility: Option<String>,
    pub state_mutability: Option<String>,
    pub is_constant: bool,
    pub is_immutable: bool,
    /// NatSpec from preceding comment nodes.
    pub natspec: Option<String>,
    /// Heavy fields only allocated when needed. (Fix #9)
    pub extras: Option<Box<DeclExtras>>,
}

impl Declaration {
    /// Get parameters (returns empty slice if none).
    pub fn parameters(&self) -> &[(String, String)] {
        self.extras
            .as_ref()
            .map(|e| e.parameters.as_slice())
            .unwrap_or(&[])
    }

    /// Get return parameters (returns empty slice if none).
    pub fn return_parameters(&self) -> &[(String, String)] {
        self.extras
            .as_ref()
            .map(|e| e.return_parameters.as_slice())
            .unwrap_or(&[])
    }

    /// Get base contracts (returns empty slice if none).
    pub fn base_contracts(&self) -> &[String] {
        self.extras
            .as_ref()
            .map(|e| e.base_contracts.as_slice())
            .unwrap_or(&[])
    }

    /// Get members (returns empty slice if none).
    pub fn members(&self) -> &[MemberInfo] {
        self.extras
            .as_ref()
            .map(|e| e.members.as_slice())
            .unwrap_or(&[])
    }

    /// Get enum values (returns empty slice if none).
    pub fn enum_values(&self) -> &[String] {
        self.extras
            .as_ref()
            .map(|e| e.enum_values.as_slice())
            .unwrap_or(&[])
    }

    /// Get or create mutable extras.
    fn extras_mut(&mut self) -> &mut DeclExtras {
        self.extras
            .get_or_insert_with(|| Box::new(DeclExtras::default()))
    }
}

#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub name: String,
    pub type_text: String,
    pub kind: DeclKind,
    pub name_range: (usize, usize),
    /// DeclId for this member, if it has a full Declaration entry.
    pub decl_id: Option<DeclId>,
}

pub type ScopeId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeKind {
    File,
    Contract,
    Interface,
    Library,
    Function,
    Modifier,
    Block,
}

/// Scope declarations stored as Vec for cache-friendly small-scope lookup. (Fix #11)
#[derive(Debug, Clone)]
pub struct Scope {
    pub id: ScopeId,
    pub parent: Option<ScopeId>,
    pub kind: ScopeKind,
    pub range: (usize, usize),
    /// Declarations in this scope — Vec is faster than HashMap for <~16 entries
    /// due to cache locality. Most Solidity scopes are small.
    pub declarations: Vec<(String, DeclId)>,
}

impl Scope {
    pub fn get_decl(&self, name: &str) -> Option<&DeclId> {
        self.declarations
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| id)
    }
}

#[derive(Debug, Clone)]
pub struct ImportInfo {
    pub source_path: String,
    pub resolved_path: Option<PathBuf>,
    pub kind: ImportKind,
    pub range: (usize, usize),
    pub path_range: (usize, usize),
}

#[derive(Debug, Clone)]
pub enum ImportKind {
    /// `import "foo.sol"`
    Glob,
    /// `import {A, B as C} from "foo.sol"`
    Named(Vec<(String, Option<String>)>),
    /// `import "foo.sol" as Foo`
    Alias(String),
}

/// An identifier usage that may reference a declaration. (Fix #12)
/// Stores byte range instead of an owned name String.
#[derive(Debug, Clone)]
pub struct Reference {
    pub range: (usize, usize),
    pub scope: ScopeId,
    pub resolved: Option<DeclId>,
    /// If this reference is the property part of a qualified name (e.g. the
    /// `FeeUpdated` in `IFees.FeeUpdated`), this stores the index of the
    /// object/container reference in the same `FileIndex.references` vec.
    pub member_of: Option<usize>,
}

impl Reference {
    /// Get the name from source text on demand instead of storing it.
    pub fn name<'a>(&self, source: &'a str) -> &'a str {
        &source[self.range.0..self.range.1]
    }
}

/// A `using Library for Type` directive.
#[derive(Debug, Clone)]
pub struct UsingDirective {
    /// The library/type being attached (e.g., "SafeMath").
    pub library_name: String,
    /// The target type (e.g., "uint256"), or None for `using X for *`.
    pub target_type: Option<String>,
    /// Scope this using directive is declared in.
    pub scope: ScopeId,
}

/// Per-file index.
#[derive(Debug, Clone)]
pub struct FileIndex {
    pub file_id: FileId,
    pub scopes: Vec<Scope>,
    pub declarations: HashMap<DeclId, Declaration>,
    pub references: Vec<Reference>,
    pub imports: Vec<ImportInfo>,
    pub using_directives: Vec<UsingDirective>,
}

/// Project-wide symbol table.
pub struct SymbolTable {
    pub files: HashMap<FileId, FileIndex>,
    pub interner: PathInterner,
    pub resolver: ImportResolver,
    /// Reverse index: DeclId → list of (FileId, start_byte, end_byte). (Fix #18)
    pub ref_index: HashMap<DeclId, Vec<(FileId, usize, usize)>>,
    /// Secondary index: FileId → DeclIds that have refs from this file. O(1) cleanup.
    ref_index_by_file: HashMap<FileId, Vec<DeclId>>,
    /// Source text cache for resolving reference names. Arc<str> for cheap cloning. (Fix #12)
    sources: HashMap<FileId, Arc<str>>,
}

// ---------------------------------------------------------------------------
// SymbolTable API
// ---------------------------------------------------------------------------

impl SymbolTable {
    pub fn new(resolver: ImportResolver) -> Self {
        Self {
            files: Default::default(),
            interner: PathInterner::default(),
            resolver,
            ref_index: Default::default(),
            ref_index_by_file: Default::default(),
            sources: Default::default(),
        }
    }

    /// Resolve a FileId back to a path.
    pub fn resolve_path(&self, id: FileId) -> &Path {
        self.interner.resolve(id)
    }

    /// Look up a FileId for a path.
    pub fn lookup_file_id(&self, path: &Path) -> Option<FileId> {
        self.interner.lookup(path)
    }

    /// Get stored source text for a file.
    pub fn get_source(&self, file_id: FileId) -> Option<&str> {
        self.sources.get(&file_id).map(|s| &**s)
    }

    /// Remove all ref_index entries contributed by a given file. O(k) where k is
    /// the number of DeclIds referenced from this file, rather than O(total refs).
    fn clear_refs_for_file(&mut self, file_id: FileId) {
        if let Some(decl_ids) = self.ref_index_by_file.remove(&file_id) {
            for decl_id in decl_ids {
                if let Some(refs) = self.ref_index.get_mut(&decl_id) {
                    refs.retain(|(fid, _, _)| *fid != file_id);
                    if refs.is_empty() {
                        self.ref_index.remove(&decl_id);
                    }
                }
            }
        }
    }

    /// Index a single file. Replaces any existing index for this path.
    pub fn index_file(&mut self, path: &Path, source: &str, parser: &mut TsParser) {
        let tree = match parser.parse(source, None) {
            Some(t) => t,
            None => return,
        };
        let file_id = self.interner.get_or_intern(path);
        self.clear_refs_for_file(file_id);

        let file_index = build_file_index(file_id, source, &tree.root_node(), &self.resolver, path);
        self.files.insert(file_id, file_index);
        self.sources.insert(file_id, Arc::from(source));
    }

    /// Index from an already-parsed tree — avoids double parsing. (Fix #1)
    pub fn index_file_with_tree(&mut self, path: &Path, source: &str, tree: &tree_sitter::Tree) {
        let file_id = self.interner.get_or_intern(path);
        self.clear_refs_for_file(file_id);

        let file_index = build_file_index(file_id, source, &tree.root_node(), &self.resolver, path);
        self.files.insert(file_id, file_index);
        self.sources.insert(file_id, Arc::from(source));
    }

    /// Ensure a file is indexed, reading from disk if necessary.
    pub fn ensure_indexed(&mut self, path: &Path, parser: &mut TsParser) {
        if let Some(id) = self.interner.lookup(path) {
            if self.files.contains_key(&id) {
                return;
            }
        }
        if let Ok(source) = std::fs::read_to_string(path) {
            self.index_file(path, &source, parser);
        }
    }

    /// Resolve all references in a single file. (Fix #22 — minimized cloning)
    pub fn resolve_file_references(&mut self, path: &Path, parser: &mut TsParser) {
        let file_id = match self.interner.lookup(path) {
            Some(id) => id,
            None => return,
        };

        // Collect resolved paths from imports (only clone the paths we need).
        let import_paths: Vec<PathBuf> = self
            .files
            .get(&file_id)
            .map(|fi| {
                fi.imports
                    .iter()
                    .filter_map(|imp| imp.resolved_path.clone())
                    .collect()
            })
            .unwrap_or_default();

        // Ensure all imported files are indexed.
        for p in &import_paths {
            self.ensure_indexed(p, parser);
        }

        // Resolve references — we need to work with indices to avoid borrow issues.
        resolve_references(self, file_id);
    }

    /// Remove a file from the symbol table. (Fix #7)
    pub fn remove_file(&mut self, path: &Path) {
        if let Some(file_id) = self.interner.lookup(path) {
            self.files.remove(&file_id);
            self.sources.remove(&file_id);
            self.clear_refs_for_file(file_id);
        }
    }

    /// Look up the declaration that the identifier at `byte_offset` refers to.
    pub fn resolve_at(&self, path: &Path, byte_offset: usize) -> Option<&Declaration> {
        let file_id = self.interner.lookup(path)?;
        let fi = self.files.get(&file_id)?;

        // First check if cursor is directly on a declaration name.
        for decl in fi.declarations.values() {
            if decl.name_range.0 <= byte_offset && byte_offset < decl.name_range.1 {
                return Some(decl);
            }
        }

        // Then check references.
        let reference = fi
            .references
            .iter()
            .find(|r| r.range.0 <= byte_offset && byte_offset < r.range.1)?;
        let decl_id = reference.resolved.as_ref()?;
        self.get_declaration(decl_id)
    }

    /// Get a declaration by its DeclId.
    pub fn get_declaration(&self, id: &DeclId) -> Option<&Declaration> {
        self.files.get(&id.file)?.declarations.get(id)
    }

    /// Find all reference locations to a given declaration using the reverse index. (Fix #18)
    pub fn find_references(&self, decl_id: &DeclId) -> Vec<(PathBuf, usize, usize)> {
        match self.ref_index.get(decl_id) {
            Some(refs) => refs
                .iter()
                .map(|(fid, start, end)| (self.interner.resolve(*fid).to_path_buf(), *start, *end))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Get all declarations visible at a given scope in a file.
    pub fn visible_declarations(&self, path: &Path, scope_id: ScopeId) -> Vec<&Declaration> {
        let file_id = match self.interner.lookup(path) {
            Some(id) => id,
            None => return vec![],
        };
        let fi = match self.files.get(&file_id) {
            Some(fi) => fi,
            None => return vec![],
        };
        let mut result = Vec::new();
        let mut current = Some(scope_id);
        let mut seen = FxHashSet::default();
        while let Some(sid) = current {
            if let Some(scope) = fi.scopes.get(sid) {
                for (_, decl_id) in &scope.declarations {
                    if seen.insert(*decl_id) {
                        if let Some(decl) = fi.declarations.get(decl_id) {
                            result.push(decl);
                        }
                    }
                }
                current = scope.parent;
            } else {
                break;
            }
        }

        // Add inherited declarations from base contracts.
        let mut cs = Some(scope_id);
        while let Some(sid) = cs {
            if let Some(scope) = fi.scopes.get(sid) {
                if matches!(
                    scope.kind,
                    ScopeKind::Contract | ScopeKind::Interface
                ) {
                    for decl in fi.declarations.values() {
                        if matches!(
                            decl.kind,
                            DeclKind::Contract | DeclKind::Interface
                        ) && scope.range.0 >= decl.full_range.0
                            && scope.range.1 <= decl.full_range.1
                        {
                            for base_name in decl.base_contracts() {
                                self.collect_base_declarations(
                                    file_id, base_name, &mut result, &mut seen,
                                );
                            }
                            break;
                        }
                    }
                    break;
                }
                cs = scope.parent;
            } else {
                break;
            }
        }

        // Also add imported declarations.
        for imp in &fi.imports {
            if let Some(ref resolved_path) = imp.resolved_path {
                let target_fid = match self.interner.lookup(resolved_path) {
                    Some(id) => id,
                    None => continue,
                };
                let target_fi = match self.files.get(&target_fid) {
                    Some(fi) => fi,
                    None => continue,
                };
                match &imp.kind {
                    ImportKind::Glob => {
                        for decl in target_fi.declarations.values() {
                            if decl.scope == 0 && seen.insert(decl.id) {
                                result.push(decl);
                            }
                        }
                    }
                    ImportKind::Named(names) => {
                        for (name, _alias) in names {
                            if let Some(decl) = find_top_level_by_name(target_fi, name) {
                                if seen.insert(decl.id) {
                                    result.push(decl);
                                }
                            }
                        }
                    }
                    ImportKind::Alias(_) => {
                        // The alias itself is a declaration in the current file.
                    }
                }
            }
        }

        result
    }

    /// Get members of a named type (for dot-completion). Returns a reference. (Fix #23)
    pub fn members_of(&self, type_name: &str, path: &Path) -> &[MemberInfo] {
        let file_id = match self.interner.lookup(path) {
            Some(id) => id,
            None => return &[],
        };

        // Search current file first, then imported files.
        let fi = match self.files.get(&file_id) {
            Some(fi) => fi,
            None => return &[],
        };

        // Check current file
        for decl in fi.declarations.values() {
            if decl.name == type_name && is_member_bearing_kind(decl.kind) {
                let members = decl.members();
                if !members.is_empty() {
                    return members;
                }
            }
        }

        // Check imported files
        for imp in &fi.imports {
            if let Some(ref resolved) = imp.resolved_path {
                if let Some(target_fid) = self.interner.lookup(resolved) {
                    if let Some(target_fi) = self.files.get(&target_fid) {
                        for decl in target_fi.declarations.values() {
                            if decl.name == type_name && is_member_bearing_kind(decl.kind) {
                                let members = decl.members();
                                if !members.is_empty() {
                                    return members;
                                }
                            }
                        }
                    }
                }
            }
        }

        &[]
    }

    /// Find the scope containing a given byte offset in a file. (Fix #5)
    pub fn scope_at(&self, path: &Path, byte_offset: usize) -> Option<ScopeId> {
        let file_id = self.interner.lookup(path)?;
        let fi = self.files.get(&file_id)?;
        find_scope_at(fi, byte_offset)
    }

    /// Find the import info whose path range contains the given byte offset.
    pub fn import_at(&self, path: &Path, byte_offset: usize) -> Option<&ImportInfo> {
        let file_id = self.interner.lookup(path)?;
        let fi = self.files.get(&file_id)?;
        fi.imports
            .iter()
            .find(|imp| imp.path_range.0 <= byte_offset && byte_offset < imp.path_range.1)
    }

    /// Get file index by path.
    pub fn get_file_index(&self, path: &Path) -> Option<&FileIndex> {
        let file_id = self.interner.lookup(path)?;
        self.files.get(&file_id)
    }

    /// Collect declarations from a base contract (for inherited member completion).
    /// Recursively collects from grandparent bases too.
    fn collect_base_declarations<'a>(
        &'a self,
        origin_file: FileId,
        base_name: &str,
        result: &mut Vec<&'a Declaration>,
        seen: &mut FxHashSet<DeclId>,
    ) {
        let base_decl_id = match find_type_declaration(self, origin_file, base_name) {
            Some(id) => id,
            None => return,
        };
        let base_fi = match self.files.get(&base_decl_id.file) {
            Some(fi) => fi,
            None => return,
        };
        let base_decl = match base_fi.declarations.get(&base_decl_id) {
            Some(d) => d,
            None => return,
        };

        // Find the scope of the base contract and add its declarations.
        for scope in &base_fi.scopes {
            let in_range = scope.range.0 >= base_decl.full_range.0
                && scope.range.1 <= base_decl.full_range.1;
            let is_ns = matches!(
                scope.kind,
                ScopeKind::Contract | ScopeKind::Interface | ScopeKind::Library
            );
            if in_range && is_ns {
                for (_, decl_id) in &scope.declarations {
                    if seen.insert(*decl_id) {
                        if let Some(decl) = base_fi.declarations.get(decl_id) {
                            // Skip private members.
                            if decl.visibility.as_deref() != Some("private") {
                                result.push(decl);
                            }
                        }
                    }
                }
            }
        }

        // Recursively add from grandparent bases.
        let grandparent_names: Vec<String> = base_decl.base_contracts().to_vec();
        for gp_name in &grandparent_names {
            self.collect_base_declarations(base_decl_id.file, gp_name, result, seen);
        }
    }

    /// Get using-for library methods that apply to a given type in a scope.
    pub fn using_for_members(
        &self,
        type_text: &str,
        path: &Path,
        _scope_id: ScopeId,
    ) -> Vec<MemberInfo> {
        let file_id = match self.interner.lookup(path) {
            Some(id) => id,
            None => return vec![],
        };
        let fi = match self.files.get(&file_id) {
            Some(fi) => fi,
            None => return vec![],
        };

        let stripped = strip_type_modifiers(type_text);
        let mut result = Vec::new();

        for using in &fi.using_directives {
            let applies = match &using.target_type {
                None => true, // `using X for *`
                Some(target) => strip_type_modifiers(target) == stripped,
            };
            if !applies {
                continue;
            }

            // Get members of the library.
            let lib_members = self.members_of(&using.library_name, path);
            for m in lib_members {
                if m.kind == DeclKind::Function {
                    result.push(m.clone());
                }
            }
        }

        result
    }
}

fn is_member_bearing_kind(kind: DeclKind) -> bool {
    matches!(
        kind,
        DeclKind::Contract
            | DeclKind::Interface
            | DeclKind::Library
            | DeclKind::Struct
            | DeclKind::Enum
    )
}

// ---------------------------------------------------------------------------
// File index builder
// ---------------------------------------------------------------------------

fn build_file_index(
    file_id: FileId,
    source: &str,
    root: &Node,
    resolver: &ImportResolver,
    file_path: &Path,
) -> FileIndex {
    let mut fi = FileIndex {
        file_id,
        scopes: Vec::new(),
        declarations: Default::default(),
        references: Vec::new(),
        imports: Vec::new(),
        using_directives: Vec::new(),
    };

    // Create file-level scope.
    fi.scopes.push(Scope {
        id: 0,
        parent: None,
        kind: ScopeKind::File,
        range: (root.start_byte(), root.end_byte()),
        declarations: Vec::new(),
    });

    // Walk the CST.
    walk_node(root, 0, file_id, source, &mut fi);

    // Resolve import paths.
    for imp in &mut fi.imports {
        imp.resolved_path = resolver.resolve(&imp.source_path, file_path);
    }

    fi
}

fn walk_node(node: &Node, scope_id: ScopeId, file_id: FileId, source: &str, fi: &mut FileIndex) {
    match node.kind() {
        "contract_declaration" | "interface_declaration" | "library_declaration" => {
            walk_contract(node, scope_id, file_id, source, fi);
        }
        "function_definition" => {
            walk_function(node, scope_id, file_id, source, fi);
        }
        "constructor_definition" => {
            walk_constructor(node, scope_id, file_id, source, fi);
        }
        "fallback_receive_definition" => {
            walk_fallback_receive(node, scope_id, file_id, source, fi);
        }
        "modifier_definition" => {
            walk_modifier(node, scope_id, file_id, source, fi);
        }
        "state_variable_declaration" => {
            walk_state_variable(node, scope_id, file_id, source, fi);
        }
        "constant_variable_declaration" => {
            walk_constant_variable(node, scope_id, file_id, source, fi);
        }
        "struct_declaration" => {
            walk_struct(node, scope_id, file_id, source, fi);
        }
        "enum_declaration" => {
            walk_enum(node, scope_id, file_id, source, fi);
        }
        "event_definition" => {
            walk_event(node, scope_id, file_id, source, fi);
        }
        "error_declaration" => {
            walk_error_decl(node, scope_id, file_id, source, fi);
        }
        "user_defined_type_definition" => {
            walk_user_defined_type_def(node, scope_id, file_id, source, fi);
        }
        "import_directive" => {
            walk_import(node, scope_id, file_id, source, fi);
        }
        "using_directive" => {
            walk_using_directive(node, scope_id, source, fi);
        }
        "variable_declaration_statement" => {
            walk_variable_decl_stmt(node, scope_id, file_id, source, fi);
        }
        "block_statement" => {
            let new_scope = create_scope(fi, Some(scope_id), ScopeKind::Block, node);
            walk_children(node, new_scope, file_id, source, fi);
        }
        "for_statement" | "while_statement" | "do_while_statement" => {
            let new_scope = create_scope(fi, Some(scope_id), ScopeKind::Block, node);
            walk_children(node, new_scope, file_id, source, fi);
        }
        "if_statement" => {
            walk_children(node, scope_id, file_id, source, fi);
        }
        "try_statement" => {
            // Walk the attempt expression in current scope.
            if let Some(attempt) = node.child_by_field_name("attempt") {
                walk_node(&attempt, scope_id, file_id, source, fi);
            }
            // Create scope for try returns params + body.
            let try_scope = create_scope(fi, Some(scope_id), ScopeKind::Block, node);
            // Declare return parameters as local variables.
            declare_parameters_as_locals(node, try_scope, file_id, source, fi);
            // Walk the try body.
            if let Some(body) = node.child_by_field_name("body") {
                walk_node(&body, try_scope, file_id, source, fi);
            }
            // Handle catch clauses.
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    if cursor.node().kind() == "catch_clause" {
                        let catch_node = cursor.node();
                        let catch_scope =
                            create_scope(fi, Some(scope_id), ScopeKind::Block, &catch_node);
                        declare_parameters_as_locals(&catch_node, catch_scope, file_id, source, fi);
                        if let Some(body) = catch_node.child_by_field_name("body") {
                            walk_node(&body, catch_scope, file_id, source, fi);
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "identifier" => {
            if !is_declaration_name(node) {
                let text = node_text(node, source);
                if !text.is_empty() {
                    fi.references.push(Reference {
                        range: (node.start_byte(), node.end_byte()),
                        scope: scope_id,
                        resolved: None,
                        member_of: None,
                    });
                }
            }
        }
        "user_defined_type" => {
            // Handle qualified types like `IFees.FeeUpdated` which have
            // multiple identifier children.  The first is the container, the
            // rest are members.
            let mut cursor = node.walk();
            let mut prev_ref_idx: Option<usize> = None;
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "identifier" {
                        let idx = fi.references.len();
                        fi.references.push(Reference {
                            range: (child.start_byte(), child.end_byte()),
                            scope: scope_id,
                            resolved: None,
                            member_of: prev_ref_idx,
                        });
                        prev_ref_idx = Some(idx);
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "member_expression" => {
            let obj_ref_start = fi.references.len();
            if let Some(obj) = node.child_by_field_name("object") {
                walk_node(&obj, scope_id, file_id, source, fi);
            }
            if let Some(prop) = node.child_by_field_name("property") {
                // Determine which reference represents the "result" of the
                // object expression:
                // - For chained member access (a.b.c) the last ref is the
                //   innermost property `b` which is correct.
                // - For subscript expressions (items[i].field) the last ref
                //   is the index `i` which is wrong — the container variable
                //   `items` is at obj_ref_start.
                //
                // Heuristic: use the last ref if it is itself a member
                // (member_of is set) or is the only ref pushed (simple
                // identifier). Otherwise fall back to the first ref which
                // is the root variable of the expression.
                let obj_ref_idx = if fi.references.len() > obj_ref_start {
                    let last = fi.references.len() - 1;
                    if last == obj_ref_start {
                        // Single ref pushed (simple identifier) — use it.
                        Some(last)
                    } else if fi.references[last].member_of.is_some() {
                        // Last ref is a member/property — use it (chained access).
                        Some(last)
                    } else {
                        // Multiple refs but last isn't a member (e.g. subscript
                        // index, function arg). Use the first ref which is the
                        // root variable.
                        Some(obj_ref_start)
                    }
                } else {
                    None
                };
                fi.references.push(Reference {
                    range: (prop.start_byte(), prop.end_byte()),
                    scope: scope_id,
                    resolved: None,
                    member_of: obj_ref_idx,
                });
            }
            return;
        }
        "call_struct_argument" => {
            // Struct literal field: `{fieldName: value}`.
            // The first identifier is the field name — skip it (don't create a
            // reference that would resolve to a same-named local variable).
            // Only walk the expression (value) child.
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "expression" {
                        walk_node(&child, scope_id, file_id, source, fi);
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        _ => {
            walk_children(node, scope_id, file_id, source, fi);
        }
    }
}

fn walk_children(
    node: &Node,
    scope_id: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk_node(&cursor.node(), scope_id, file_id, source, fi);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Declaration walkers
// ---------------------------------------------------------------------------

/// Helper to create a minimal declaration with no extras.
fn make_decl(
    file_id: FileId,
    name_node: &Node,
    node: &Node,
    source: &str,
    kind: DeclKind,
    scope: ScopeId,
) -> (DeclId, Declaration) {
    let name = node_text(name_node, source).to_string();
    let decl_id = DeclId {
        file: file_id,
        byte_offset: name_node.start_byte(),
    };
    let decl = Declaration {
        id: decl_id,
        name,
        kind,
        full_range: (node.start_byte(), node.end_byte()),
        name_range: (name_node.start_byte(), name_node.end_byte()),
        scope,
        type_text: None,
        visibility: None,
        state_mutability: None,
        is_constant: false,
        is_immutable: false,
        natspec: None,
        extras: None,
    };
    (decl_id, decl)
}

fn walk_contract(
    node: &Node,
    parent_scope: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let kind = match node.kind() {
        "interface_declaration" => DeclKind::Interface,
        "library_declaration" => DeclKind::Library,
        _ => DeclKind::Contract,
    };
    let scope_kind = match kind {
        DeclKind::Interface => ScopeKind::Interface,
        DeclKind::Library => ScopeKind::Library,
        _ => ScopeKind::Contract,
    };

    let contract_scope = create_scope(fi, Some(parent_scope), scope_kind, node);

    // Extract base contracts from inheritance_specifier children.
    let mut base_contracts = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "inheritance_specifier" {
                if let Some(ancestor) = child.child_by_field_name("ancestor") {
                    let text = node_text(&ancestor, source).to_string();
                    base_contracts.push(text);
                    let mut inner = ancestor.walk();
                    if inner.goto_first_child() {
                        loop {
                            if inner.node().kind() == "identifier" {
                                fi.references.push(Reference {
                                    range: (inner.node().start_byte(), inner.node().end_byte()),
                                    scope: parent_scope,
                                    resolved: None,
                                    member_of: None,
                                });
                                break;
                            }
                            if !inner.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    // Collect members by walking body children.
    let mut members = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut body_cursor = body.walk();
        if body_cursor.goto_first_child() {
            loop {
                let child = body_cursor.node();
                match child.kind() {
                    "function_definition" | "fallback_receive_definition" => {
                        if let Some(n) = child.child_by_field_name("name") {
                            members.push(MemberInfo {
                                name: node_text(&n, source).to_string(),
                                type_text: "function".to_string(),
                                kind: DeclKind::Function,
                                name_range: (n.start_byte(), n.end_byte()),
                                decl_id: None,
                            });
                        }
                    }
                    "state_variable_declaration" => {
                        if let Some(n) = child.child_by_field_name("name") {
                            let type_text = child
                                .child_by_field_name("type")
                                .map(|t| node_text(&t, source).to_string())
                                .unwrap_or_default();
                            members.push(MemberInfo {
                                name: node_text(&n, source).to_string(),
                                type_text,
                                kind: DeclKind::StateVariable,
                                name_range: (n.start_byte(), n.end_byte()),
                                decl_id: None,
                            });
                        }
                    }
                    "struct_declaration"
                    | "enum_declaration"
                    | "event_definition"
                    | "error_declaration"
                    | "modifier_definition" => {
                        if let Some(n) = child.child_by_field_name("name") {
                            let mk = match child.kind() {
                                "struct_declaration" => DeclKind::Struct,
                                "enum_declaration" => DeclKind::Enum,
                                "event_definition" => DeclKind::Event,
                                "error_declaration" => DeclKind::Error,
                                "modifier_definition" => DeclKind::Modifier,
                                _ => DeclKind::Function,
                            };
                            members.push(MemberInfo {
                                name: node_text(&n, source).to_string(),
                                type_text: child.kind().to_string(),
                                kind: mk,
                                name_range: (n.start_byte(), n.end_byte()),
                                decl_id: None,
                            });
                        }
                    }
                    _ => {}
                }
                if !body_cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    let natspec = extract_natspec(node, source);
    let (decl_id, mut decl) = make_decl(file_id, &name_node, node, source, kind, parent_scope);

    if !base_contracts.is_empty() || !members.is_empty() {
        let extras = decl.extras_mut();
        extras.base_contracts = base_contracts;
        extras.members = members;
    }
    decl.natspec = natspec;

    let name = decl.name.clone();
    fi.declarations.insert(decl_id, decl);
    register_in_scope(fi, parent_scope, &name, &decl_id);

    if let Some(body) = node.child_by_field_name("body") {
        walk_children(&body, contract_scope, file_id, source, fi);
    }
}

fn walk_function(
    node: &Node,
    parent_scope: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };

    let fn_scope = create_scope(fi, Some(parent_scope), ScopeKind::Function, node);
    let parameters = extract_parameters(node, source, fi, fn_scope, file_id);
    let return_parameters = extract_return_parameters(node, source, fi, fn_scope, file_id);
    let visibility = extract_child_kind(node, "visibility", source);
    let state_mutability = extract_child_kind(node, "state_mutability", source);
    let natspec = extract_natspec(node, source);

    let (decl_id, mut decl) = make_decl(
        file_id,
        &name_node,
        node,
        source,
        DeclKind::Function,
        parent_scope,
    );
    decl.visibility = visibility;
    decl.state_mutability = state_mutability;
    decl.natspec = natspec;

    if !parameters.is_empty() || !return_parameters.is_empty() {
        let extras = decl.extras_mut();
        extras.parameters = parameters;
        extras.return_parameters = return_parameters;
    }

    let name = decl.name.clone();
    fi.declarations.insert(decl_id, decl);
    register_in_scope(fi, parent_scope, &name, &decl_id);

    walk_modifier_invocations(node, fn_scope, source, fi);

    // Walk parameter types so user-defined types generate references.
    walk_parameter_types(node, fn_scope, file_id, source, fi);

    // Walk return type so user-defined types generate references.
    if let Some(return_type) = node.child_by_field_name("return_type") {
        walk_parameter_types(&return_type, fn_scope, file_id, source, fi);
    }

    if let Some(body) = node.child_by_field_name("body") {
        walk_children(&body, fn_scope, file_id, source, fi);
    }
}

fn walk_constructor(
    node: &Node,
    parent_scope: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let fn_scope = create_scope(fi, Some(parent_scope), ScopeKind::Function, node);
    let parameters = extract_parameters(node, source, fi, fn_scope, file_id);
    let natspec = extract_natspec(node, source);

    let decl_id = DeclId {
        file: file_id,
        byte_offset: node.start_byte(),
    };

    let mut decl = Declaration {
        id: decl_id,
        name: "constructor".to_string(),
        kind: DeclKind::Constructor,
        full_range: (node.start_byte(), node.end_byte()),
        name_range: (node.start_byte(), node.start_byte() + "constructor".len()),
        scope: parent_scope,
        type_text: None,
        visibility: None,
        state_mutability: None,
        is_constant: false,
        is_immutable: false,
        natspec,
        extras: None,
    };

    if !parameters.is_empty() {
        decl.extras_mut().parameters = parameters;
    }

    fi.declarations.insert(decl_id, decl);

    walk_parameter_types(node, fn_scope, file_id, source, fi);

    if let Some(body) = node.child_by_field_name("body") {
        walk_children(&body, fn_scope, file_id, source, fi);
    }
}

fn walk_fallback_receive(
    node: &Node,
    parent_scope: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let fn_scope = create_scope(fi, Some(parent_scope), ScopeKind::Function, node);
    let parameters = extract_parameters(node, source, fi, fn_scope, file_id);
    let natspec = extract_natspec(node, source);

    let text = node_text(node, source);
    let name = if text.starts_with("receive") {
        "receive"
    } else {
        "fallback"
    };

    let decl_id = DeclId {
        file: file_id,
        byte_offset: node.start_byte(),
    };

    let mut decl = Declaration {
        id: decl_id,
        name: name.to_string(),
        kind: DeclKind::FallbackReceive,
        full_range: (node.start_byte(), node.end_byte()),
        name_range: (node.start_byte(), node.start_byte() + name.len()),
        scope: parent_scope,
        type_text: None,
        visibility: extract_child_kind(node, "visibility", source),
        state_mutability: extract_child_kind(node, "state_mutability", source),
        is_constant: false,
        is_immutable: false,
        natspec,
        extras: None,
    };

    if !parameters.is_empty() {
        decl.extras_mut().parameters = parameters;
    }

    fi.declarations.insert(decl_id, decl);

    walk_parameter_types(node, fn_scope, file_id, source, fi);

    if let Some(body) = node.child_by_field_name("body") {
        walk_children(&body, fn_scope, file_id, source, fi);
    }
}

fn walk_modifier(
    node: &Node,
    parent_scope: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };

    let mod_scope = create_scope(fi, Some(parent_scope), ScopeKind::Modifier, node);
    let parameters = extract_parameters(node, source, fi, mod_scope, file_id);
    let natspec = extract_natspec(node, source);

    let (decl_id, mut decl) = make_decl(
        file_id,
        &name_node,
        node,
        source,
        DeclKind::Modifier,
        parent_scope,
    );
    decl.natspec = natspec;

    if !parameters.is_empty() {
        decl.extras_mut().parameters = parameters;
    }

    let name = decl.name.clone();
    fi.declarations.insert(decl_id, decl);
    register_in_scope(fi, parent_scope, &name, &decl_id);

    walk_parameter_types(node, mod_scope, file_id, source, fi);

    if let Some(body) = node.child_by_field_name("body") {
        walk_children(&body, mod_scope, file_id, source, fi);
    }
}

fn walk_state_variable(
    node: &Node,
    scope_id: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let type_text = node
        .child_by_field_name("type")
        .map(|t| node_text(&t, source).to_string());
    let visibility = node
        .child_by_field_name("visibility")
        .map(|v| node_text(&v, source).to_string());
    let is_constant = has_child_kind(node, "constant");
    let is_immutable = has_child_kind(node, "immutable");
    let natspec = extract_natspec(node, source);

    let (decl_id, mut decl) = make_decl(
        file_id,
        &name_node,
        node,
        source,
        DeclKind::StateVariable,
        scope_id,
    );
    decl.type_text = type_text;
    decl.visibility = visibility;
    decl.is_constant = is_constant;
    decl.is_immutable = is_immutable;
    decl.natspec = natspec;

    let name = decl.name.clone();
    fi.declarations.insert(decl_id, decl);
    register_in_scope(fi, scope_id, &name, &decl_id);

    if let Some(value) = node.child_by_field_name("value") {
        walk_node(&value, scope_id, file_id, source, fi);
    }
    if let Some(type_node) = node.child_by_field_name("type") {
        walk_node(&type_node, scope_id, file_id, source, fi);
    }
}

fn walk_constant_variable(
    node: &Node,
    scope_id: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let type_text = node
        .child_by_field_name("type")
        .map(|t| node_text(&t, source).to_string());
    let natspec = extract_natspec(node, source);

    let (decl_id, mut decl) = make_decl(
        file_id,
        &name_node,
        node,
        source,
        DeclKind::Constant,
        scope_id,
    );
    decl.type_text = type_text;
    decl.is_constant = true;
    decl.natspec = natspec;

    let name = decl.name.clone();
    fi.declarations.insert(decl_id, decl);
    register_in_scope(fi, scope_id, &name, &decl_id);

    if let Some(value) = node.child_by_field_name("value") {
        walk_node(&value, scope_id, file_id, source, fi);
    }
    if let Some(type_node) = node.child_by_field_name("type") {
        walk_node(&type_node, scope_id, file_id, source, fi);
    }
}

fn walk_struct(node: &Node, scope_id: ScopeId, file_id: FileId, source: &str, fi: &mut FileIndex) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let natspec = extract_natspec(node, source);

    let mut members = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "struct_member" {
                    if let Some(mname) = child.child_by_field_name("name") {
                        let mtype = child
                            .child_by_field_name("type")
                            .map(|t| node_text(&t, source).to_string())
                            .unwrap_or_default();

                        // Register struct field as a Declaration.
                        let (field_decl_id, mut field_decl) = make_decl(
                            file_id,
                            &mname,
                            &child,
                            source,
                            DeclKind::StateVariable,
                            scope_id,
                        );
                        field_decl.type_text = Some(mtype.clone());
                        fi.declarations.insert(field_decl_id, field_decl);

                        members.push(MemberInfo {
                            name: node_text(&mname, source).to_string(),
                            type_text: mtype,
                            kind: DeclKind::StateVariable,
                            name_range: (mname.start_byte(), mname.end_byte()),
                            decl_id: Some(field_decl_id),
                        });
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    let (decl_id, mut decl) = make_decl(
        file_id,
        &name_node,
        node,
        source,
        DeclKind::Struct,
        scope_id,
    );
    decl.natspec = natspec;

    if !members.is_empty() {
        decl.extras_mut().members = members;
    }

    let name = decl.name.clone();
    fi.declarations.insert(decl_id, decl);
    register_in_scope(fi, scope_id, &name, &decl_id);
}

fn walk_enum(node: &Node, scope_id: ScopeId, file_id: FileId, source: &str, fi: &mut FileIndex) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let enum_name = node_text(&name_node, source).to_string();
    let natspec = extract_natspec(node, source);

    let mut enum_values = Vec::new();
    let mut members = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "enum_value" {
                    let val_name = node_text(&child, source).to_string();

                    // Register enum value as a Declaration.
                    let val_decl_id = DeclId {
                        file: file_id,
                        byte_offset: child.start_byte(),
                    };
                    let val_decl = Declaration {
                        id: val_decl_id,
                        name: val_name.clone(),
                        kind: DeclKind::EnumValue,
                        full_range: (child.start_byte(), child.end_byte()),
                        name_range: (child.start_byte(), child.end_byte()),
                        scope: scope_id,
                        type_text: Some(enum_name.clone()),
                        visibility: None,
                        state_mutability: None,
                        is_constant: false,
                        is_immutable: false,
                        natspec: None,
                        extras: None,
                    };
                    fi.declarations.insert(val_decl_id, val_decl);

                    members.push(MemberInfo {
                        name: val_name.clone(),
                        type_text: enum_name.clone(),
                        kind: DeclKind::EnumValue,
                        name_range: (child.start_byte(), child.end_byte()),
                        decl_id: Some(val_decl_id),
                    });

                    enum_values.push(val_name);
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    let (decl_id, mut decl) =
        make_decl(file_id, &name_node, node, source, DeclKind::Enum, scope_id);
    decl.natspec = natspec;

    if !enum_values.is_empty() || !members.is_empty() {
        let extras = decl.extras_mut();
        extras.members = members;
        extras.enum_values = enum_values;
    }

    let name = decl.name.clone();
    fi.declarations.insert(decl_id, decl);
    register_in_scope(fi, scope_id, &name, &decl_id);
}

fn walk_event(node: &Node, scope_id: ScopeId, file_id: FileId, source: &str, fi: &mut FileIndex) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let natspec = extract_natspec(node, source);

    let mut params = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "event_parameter" {
                let ptype = child
                    .child_by_field_name("type")
                    .map(|t| node_text(&t, source).to_string())
                    .unwrap_or_default();
                let pname = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source).to_string())
                    .unwrap_or_default();
                if let Some(type_node) = child.child_by_field_name("type") {
                    walk_node(&type_node, scope_id, file_id, source, fi);
                }
                params.push((ptype, pname));
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    let (decl_id, mut decl) =
        make_decl(file_id, &name_node, node, source, DeclKind::Event, scope_id);
    decl.natspec = natspec;

    if !params.is_empty() {
        decl.extras_mut().parameters = params;
    }

    let name = decl.name.clone();
    fi.declarations.insert(decl_id, decl);
    register_in_scope(fi, scope_id, &name, &decl_id);
}

fn walk_error_decl(
    node: &Node,
    scope_id: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let natspec = extract_natspec(node, source);

    let mut params = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "error_parameter" {
                let ptype = child
                    .child_by_field_name("type")
                    .map(|t| node_text(&t, source).to_string())
                    .unwrap_or_default();
                let pname = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source).to_string())
                    .unwrap_or_default();
                if let Some(type_node) = child.child_by_field_name("type") {
                    walk_node(&type_node, scope_id, file_id, source, fi);
                }
                params.push((ptype, pname));
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    let (decl_id, mut decl) =
        make_decl(file_id, &name_node, node, source, DeclKind::Error, scope_id);
    decl.natspec = natspec;

    if !params.is_empty() {
        decl.extras_mut().parameters = params;
    }

    let name = decl.name.clone();
    fi.declarations.insert(decl_id, decl);
    register_in_scope(fi, scope_id, &name, &decl_id);
}

fn walk_user_defined_type_def(
    node: &Node,
    scope_id: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };

    let (decl_id, decl) = make_decl(
        file_id,
        &name_node,
        node,
        source,
        DeclKind::UserDefinedType,
        scope_id,
    );
    let name = decl.name.clone();
    fi.declarations.insert(decl_id, decl);
    register_in_scope(fi, scope_id, &name, &decl_id);
}

fn walk_using_directive(node: &Node, scope_id: ScopeId, source: &str, fi: &mut FileIndex) {
    // Grammar: using_directive has child `type_alias` (containing the library identifier)
    // and field `source` (the target type, or `any_source_type` for `*`).
    let mut library_name: Option<String> = None;
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "type_alias" {
                // The type_alias contains identifier children — take the first.
                let mut inner = child.walk();
                if inner.goto_first_child() {
                    loop {
                        if inner.node().kind() == "identifier" {
                            library_name = Some(node_text(&inner.node(), source).to_string());
                            break;
                        }
                        if !inner.goto_next_sibling() {
                            break;
                        }
                    }
                }
                break;
            }
            // Fallback: bare identifier at top level (simple `using Lib for Type`)
            if child.kind() == "user_defined_type" || child.kind() == "identifier" {
                if library_name.is_none() {
                    library_name = Some(node_text(&child, source).to_string());
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    let library_name = match library_name {
        Some(n) => n,
        None => return,
    };

    // Extract the target type from the `source` field.
    let target_type = node.child_by_field_name("source").and_then(|t| {
        if t.kind() == "any_source_type" {
            None // `using X for *`
        } else {
            Some(node_text(&t, source).to_string())
        }
    });

    fi.using_directives.push(UsingDirective {
        library_name,
        target_type,
        scope: scope_id,
    });
}

fn walk_import(node: &Node, scope_id: ScopeId, file_id: FileId, source: &str, fi: &mut FileIndex) {
    let source_node = match node.child_by_field_name("source") {
        Some(n) => n,
        None => return,
    };
    let source_text = node_text(&source_node, source)
        .trim_matches(|c: char| c == '"' || c == '\'')
        .to_string();

    let path_range = (source_node.start_byte(), source_node.end_byte());

    let mut import_entries: Vec<(Node, Option<Node>)> = Vec::new();
    let mut standalone_alias: Option<Node> = None;
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let field = cursor.field_name();
            let child = cursor.node();
            if field == Some("import_name") {
                import_entries.push((child, None));
            } else if field == Some("alias") {
                if let Some(last) = import_entries.last_mut() {
                    last.1 = Some(child);
                } else {
                    standalone_alias = Some(child);
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    let import_names: Vec<Node> = import_entries.iter().map(|(n, _)| *n).collect();

    let kind = if !import_entries.is_empty() {
        let mut names = Vec::new();
        for (name_node, alias_node) in &import_entries {
            let name = node_text(name_node, source).to_string();
            let alias = alias_node.map(|a| node_text(&a, source).to_string());
            names.push((name, alias));
        }
        ImportKind::Named(names)
    } else if let Some(alias_node) = standalone_alias {
        let alias_text = node_text(&alias_node, source).to_string();
        let decl_id = DeclId {
            file: file_id,
            byte_offset: alias_node.start_byte(),
        };
        let decl = Declaration {
            id: decl_id,
            name: alias_text.clone(),
            kind: DeclKind::ImportAlias,
            full_range: (node.start_byte(), node.end_byte()),
            name_range: (alias_node.start_byte(), alias_node.end_byte()),
            scope: scope_id,
            type_text: None,
            visibility: None,
            state_mutability: None,
            is_constant: false,
            is_immutable: false,
            natspec: None,
            extras: None,
        };
        fi.declarations.insert(decl_id, decl);
        register_in_scope(fi, scope_id, &alias_text, &decl_id);
        ImportKind::Alias(alias_text)
    } else {
        ImportKind::Glob
    };

    if let ImportKind::Named(ref names) = kind {
        for (name, alias) in names {
            let local_name = alias.as_ref().unwrap_or(name);
            for name_node in &import_names {
                if node_text(name_node, source) == name.as_str() {
                    let decl_id = DeclId {
                        file: file_id,
                        byte_offset: name_node.start_byte(),
                    };
                    let decl = Declaration {
                        id: decl_id,
                        name: local_name.clone(),
                        kind: DeclKind::ImportAlias,
                        full_range: (node.start_byte(), node.end_byte()),
                        name_range: (name_node.start_byte(), name_node.end_byte()),
                        scope: scope_id,
                        type_text: None,
                        visibility: None,
                        state_mutability: None,
                        is_constant: false,
                        is_immutable: false,
                        natspec: None,
                        extras: None,
                    };
                    fi.declarations.insert(decl_id, decl);
                    register_in_scope(fi, scope_id, local_name, &decl_id);
                    break;
                }
            }
        }
    }

    fi.imports.push(ImportInfo {
        source_path: source_text,
        resolved_path: None,
        kind,
        range: (node.start_byte(), node.end_byte()),
        path_range,
    });
}

fn walk_variable_decl_stmt(
    node: &Node,
    scope_id: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            match child.kind() {
                "variable_declaration" => {
                    let name_node = match child.child_by_field_name("name") {
                        Some(n) => n,
                        None => {
                            if !cursor.goto_next_sibling() {
                                break;
                            }
                            continue;
                        }
                    };
                    let type_text = child
                        .child_by_field_name("type")
                        .map(|t| node_text(&t, source).to_string());

                    let (decl_id, mut decl) = make_decl(
                        file_id,
                        &name_node,
                        &child,
                        source,
                        DeclKind::LocalVariable,
                        scope_id,
                    );
                    decl.type_text = type_text;

                    let name = decl.name.clone();
                    fi.declarations.insert(decl_id, decl);
                    register_in_scope(fi, scope_id, &name, &decl_id);

                    if let Some(type_node) = child.child_by_field_name("type") {
                        walk_node(&type_node, scope_id, file_id, source, fi);
                    }
                }
                "variable_declaration_tuple" => {
                    let mut tuple_cursor = child.walk();
                    if tuple_cursor.goto_first_child() {
                        loop {
                            let tc = tuple_cursor.node();
                            if tc.kind() == "variable_declaration" {
                                if let Some(name_node) = tc.child_by_field_name("name") {
                                    let type_text = tc
                                        .child_by_field_name("type")
                                        .map(|t| node_text(&t, source).to_string());

                                    let (decl_id, mut decl) = make_decl(
                                        file_id,
                                        &name_node,
                                        &tc,
                                        source,
                                        DeclKind::LocalVariable,
                                        scope_id,
                                    );
                                    decl.type_text = type_text;

                                    let name = decl.name.clone();
                                    fi.declarations.insert(decl_id, decl);
                                    register_in_scope(fi, scope_id, &name, &decl_id);
                                }
                            }
                            if !tuple_cursor.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                _ => {
                    walk_node(&child, scope_id, file_id, source, fi);
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_scope(
    fi: &mut FileIndex,
    parent: Option<ScopeId>,
    kind: ScopeKind,
    node: &Node,
) -> ScopeId {
    let id = fi.scopes.len();
    fi.scopes.push(Scope {
        id,
        parent,
        kind,
        range: (node.start_byte(), node.end_byte()),
        declarations: Vec::new(),
    });
    id
}

fn register_in_scope(fi: &mut FileIndex, scope_id: ScopeId, name: &str, decl_id: &DeclId) {
    if let Some(scope) = fi.scopes.get_mut(scope_id) {
        scope.declarations.push((name.to_string(), *decl_id));
    }
}

fn node_text<'a>(node: &Node, source: &'a str) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

fn extract_parameters(
    node: &Node,
    source: &str,
    fi: &mut FileIndex,
    fn_scope: ScopeId,
    file_id: FileId,
) -> Vec<(String, String)> {
    let mut params = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "parameter" {
                let ptype = child
                    .child_by_field_name("type")
                    .map(|t| node_text(&t, source).to_string())
                    .unwrap_or_default();
                let pname = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source).to_string())
                    .unwrap_or_default();

                if let Some(name_node) = child.child_by_field_name("name") {
                    let (decl_id, mut decl) = make_decl(
                        file_id,
                        &name_node,
                        &child,
                        source,
                        DeclKind::Parameter,
                        fn_scope,
                    );
                    decl.type_text = Some(ptype.clone());

                    let pname_clone = decl.name.clone();
                    fi.declarations.insert(decl_id, decl);
                    register_in_scope(fi, fn_scope, &pname_clone, &decl_id);
                }

                params.push((ptype, pname));
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    params
}

/// Declare `parameter` children of a node as LocalVariable declarations in the
/// given scope.  Used for try-statement return params and catch-clause params.
fn declare_parameters_as_locals(
    node: &Node,
    scope_id: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "parameter" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let type_text = child
                        .child_by_field_name("type")
                        .map(|t| node_text(&t, source).to_string());

                    let (decl_id, mut decl) = make_decl(
                        file_id,
                        &name_node,
                        &child,
                        source,
                        DeclKind::LocalVariable,
                        scope_id,
                    );
                    decl.type_text = type_text;

                    let name = decl.name.clone();
                    fi.declarations.insert(decl_id, decl);
                    register_in_scope(fi, scope_id, &name, &decl_id);

                    if let Some(type_node) = child.child_by_field_name("type") {
                        walk_node(&type_node, scope_id, file_id, source, fi);
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Walk the type nodes of `parameter` children so that user-defined types
/// inside function parameters / return parameters generate references.
fn walk_parameter_types(
    node: &Node,
    scope_id: ScopeId,
    file_id: FileId,
    source: &str,
    fi: &mut FileIndex,
) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "parameter" {
                if let Some(type_node) = child.child_by_field_name("type") {
                    walk_node(&type_node, scope_id, file_id, source, fi);
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn extract_return_parameters(
    node: &Node,
    source: &str,
    fi: &mut FileIndex,
    fn_scope: ScopeId,
    file_id: FileId,
) -> Vec<(String, String)> {
    let return_type = match node.child_by_field_name("return_type") {
        Some(rt) => rt,
        None => return Vec::new(),
    };
    let mut params = Vec::new();
    let mut cursor = return_type.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "parameter" {
                let ptype = child
                    .child_by_field_name("type")
                    .map(|t| node_text(&t, source).to_string())
                    .unwrap_or_default();
                let pname = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source).to_string())
                    .unwrap_or_default();

                if let Some(name_node) = child.child_by_field_name("name") {
                    let (decl_id, mut decl) = make_decl(
                        file_id,
                        &name_node,
                        &child,
                        source,
                        DeclKind::Parameter,
                        fn_scope,
                    );
                    decl.type_text = Some(ptype.clone());

                    let pname_clone = decl.name.clone();
                    fi.declarations.insert(decl_id, decl);
                    register_in_scope(fi, fn_scope, &pname_clone, &decl_id);
                }

                params.push((ptype, pname));
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    params
}

fn extract_child_kind(node: &Node, kind: &str, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == kind {
                return Some(node_text(&child, source).to_string());
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

fn has_child_kind(node: &Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.node().kind() == kind {
                return true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

fn walk_modifier_invocations(node: &Node, scope_id: ScopeId, _source: &str, fi: &mut FileIndex) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "modifier_invocation" {
                let mut inner = child.walk();
                if inner.goto_first_child() {
                    loop {
                        let ic = inner.node();
                        if ic.kind() == "identifier" {
                            fi.references.push(Reference {
                                range: (ic.start_byte(), ic.end_byte()),
                                scope: scope_id,
                                resolved: None,
                                member_of: None,
                            });
                            break;
                        }
                        if !inner.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn is_declaration_name(node: &Node) -> bool {
    if let Some(parent) = node.parent() {
        match parent.kind() {
            "contract_declaration"
            | "interface_declaration"
            | "library_declaration"
            | "function_definition"
            | "modifier_definition"
            | "struct_declaration"
            | "enum_declaration"
            | "event_definition"
            | "error_declaration"
            | "state_variable_declaration"
            | "constant_variable_declaration"
            | "variable_declaration"
            | "parameter"
            | "event_parameter"
            | "error_parameter"
            | "struct_member"
            | "user_defined_type_definition" => {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    return name_node.id() == node.id();
                }
            }
            "import_directive" => {
                let mut cursor = parent.walk();
                if cursor.goto_first_child() {
                    loop {
                        if cursor.node().id() == node.id() {
                            let field = cursor.field_name();
                            if field == Some("import_name") || field == Some("alias") {
                                return true;
                            }
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn extract_natspec(node: &Node, source: &str) -> Option<String> {
    let mut comments = Vec::new();
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        if p.kind() == "comment" {
            comments.push(node_text(&p, source));
            prev = p.prev_named_sibling();
        } else {
            break;
        }
    }
    if comments.is_empty() {
        return None;
    }
    comments.reverse();
    let mut lines = Vec::new();
    for comment in comments {
        let trimmed = comment.trim();
        if let Some(rest) = trimmed.strip_prefix("///") {
            lines.push(rest.trim_start().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("/**") {
            let inner = rest.strip_suffix("*/").unwrap_or(rest);
            for line in inner.lines() {
                let l = line.trim().trim_start_matches('*').trim_start();
                if !l.is_empty() {
                    lines.push(l.to_string());
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("//") {
            lines.push(rest.trim_start().to_string());
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Find the innermost scope containing byte_offset. (Fix #5)
/// Uses the fact that scopes are created in tree order, so we can scan
/// and pick the smallest range containing the offset.
fn find_scope_at(fi: &FileIndex, byte_offset: usize) -> Option<ScopeId> {
    let mut best: Option<ScopeId> = None;
    let mut best_size = usize::MAX;
    for scope in &fi.scopes {
        if scope.range.0 <= byte_offset && byte_offset < scope.range.1 {
            let size = scope.range.1 - scope.range.0;
            if size < best_size {
                best_size = size;
                best = Some(scope.id);
            }
        }
    }
    best
}

fn find_top_level_by_name<'a>(fi: &'a FileIndex, name: &str) -> Option<&'a Declaration> {
    // Search file scope (scope 0) first.
    if let Some(scope) = fi.scopes.first() {
        if let Some(decl_id) = scope.get_decl(name) {
            return fi.declarations.get(decl_id);
        }
    }
    // Also search inside contracts.
    for decl in fi.declarations.values() {
        if decl.name == name && decl.scope == 0 {
            return Some(decl);
        }
    }
    // Search in contract-level scopes.
    for scope in &fi.scopes {
        if matches!(
            scope.kind,
            ScopeKind::Contract | ScopeKind::Interface | ScopeKind::Library
        ) {
            if let Some(decl_id) = scope.get_decl(name) {
                return fi.declarations.get(decl_id);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Reference resolution (Fix #22 — minimal cloning)
// ---------------------------------------------------------------------------

fn resolve_references(st: &mut SymbolTable, file_id: FileId) {
    // Collect the data we need to resolve without holding a mutable borrow.
    let (unresolved, scope_snapshot, import_snapshot, source) = {
        let fi = match st.files.get(&file_id) {
            Some(fi) => fi,
            None => return,
        };
        let source = st.sources.get(&file_id).cloned().unwrap_or_default();
        let unresolved: Vec<(usize, usize, usize, ScopeId, Option<usize>)> = fi
            .references
            .iter()
            .enumerate()
            .filter(|(_, r)| r.resolved.is_none())
            .map(|(i, r)| (i, r.range.0, r.range.1, r.scope, r.member_of))
            .collect();
        // We only clone the scope declarations (Vec<(String, DeclId)>) and imports,
        // not the full declarations HashMap.
        let scope_snapshot: Vec<(Option<ScopeId>, Vec<(String, DeclId)>)> = fi
            .scopes
            .iter()
            .map(|s| (s.parent, s.declarations.clone()))
            .collect();
        let import_snapshot = fi.imports.clone();
        (unresolved, scope_snapshot, import_snapshot, source)
    };

    // Pass 1: resolve non-member references (normal scope-chain + imports).
    for &(idx, start, end, scope_id, member_of) in &unresolved {
        if member_of.is_some() {
            continue;
        }
        let name = &source[start..end];
        let resolved = resolve_single(
            name,
            scope_id,
            &scope_snapshot,
            &import_snapshot,
            file_id,
            st,
        );

        if let Some(ref decl_id) = resolved {
            st.ref_index
                .entry(*decl_id)
                .or_default()
                .push((file_id, start, end));
            st.ref_index_by_file
                .entry(file_id)
                .or_default()
                .push(*decl_id);
        }
        if let Some(fi) = st.files.get_mut(&file_id) {
            if let Some(r) = fi.references.get_mut(idx) {
                r.resolved = resolved;
            }
        }
    }

    // Pass 2: resolve member references (property part of dot expressions).
    for &(idx, start, end, _scope_id, member_of) in &unresolved {
        let container_ref_idx = match member_of {
            Some(i) => i,
            None => continue,
        };

        let member_name = &source[start..end];
        let resolved = resolve_member(st, file_id, container_ref_idx, member_name);

        if let Some(ref decl_id) = resolved {
            st.ref_index
                .entry(*decl_id)
                .or_default()
                .push((file_id, start, end));
            st.ref_index_by_file
                .entry(file_id)
                .or_default()
                .push(*decl_id);
        }
        if let Some(fi) = st.files.get_mut(&file_id) {
            if let Some(r) = fi.references.get_mut(idx) {
                r.resolved = resolved;
            }
        }
    }
}

fn resolve_single(
    name: &str,
    scope_id: ScopeId,
    scopes: &[(Option<ScopeId>, Vec<(String, DeclId)>)],
    imports: &[ImportInfo],
    file_id: FileId,
    st: &SymbolTable,
) -> Option<DeclId> {
    // 1. Walk up scope tree.
    let mut current = Some(scope_id);
    while let Some(sid) = current {
        if let Some((parent, decls)) = scopes.get(sid) {
            if let Some((_, decl_id)) = decls.iter().find(|(n, _)| n == name) {
                return Some(*decl_id);
            }
            current = *parent;
        } else {
            break;
        }
    }

    // 1.5. Search inherited base contracts.
    // Walk the scope chain again to find the enclosing contract scope, then
    // search base contracts for the name.
    if let Some(fi) = st.files.get(&file_id) {
        let mut current = Some(scope_id);
        while let Some(sid) = current {
            if let Some(scope) = fi.scopes.get(sid) {
                if matches!(scope.kind, ScopeKind::Contract | ScopeKind::Interface) {
                    // Find the contract declaration that owns this scope.
                    for decl in fi.declarations.values() {
                        if matches!(decl.kind, DeclKind::Contract | DeclKind::Interface)
                            && scope.range.0 >= decl.full_range.0
                            && scope.range.1 <= decl.full_range.1
                        {
                            // Search each base contract.
                            for base_name in decl.base_contracts() {
                                if let Some(result) =
                                    resolve_in_base_contract(st, file_id, base_name, name)
                                {
                                    return Some(result);
                                }
                            }
                            break;
                        }
                    }
                    break;
                }
                current = scope.parent;
            } else {
                break;
            }
        }
    }

    // 2. Check imports.
    for imp in imports {
        match &imp.kind {
            ImportKind::Named(names) => {
                for (import_name, alias) in names {
                    let local_name = alias.as_ref().unwrap_or(import_name);
                    if local_name == name {
                        if let Some(ref resolved_path) = imp.resolved_path {
                            if let Some(target_fid) = st.interner.lookup(resolved_path) {
                                if let Some(target_fi) = st.files.get(&target_fid) {
                                    if let Some(decl) =
                                        find_top_level_by_name(target_fi, import_name)
                                    {
                                        return Some(decl.id);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            ImportKind::Glob => {
                if let Some(ref resolved_path) = imp.resolved_path {
                    if let Some(target_fid) = st.interner.lookup(resolved_path) {
                        if let Some(target_fi) = st.files.get(&target_fid) {
                            if let Some(decl) = find_top_level_by_name(target_fi, name) {
                                return Some(decl.id);
                            }
                        }
                    }
                }
            }
            ImportKind::Alias(alias) => {
                if alias == name {
                    if let Some(fi) = st.files.get(&file_id) {
                        for decl in fi.declarations.values() {
                            if decl.name == name && decl.kind == DeclKind::ImportAlias {
                                return Some(decl.id);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Member resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a member name by looking inside the container that the
/// `container_ref_idx` reference resolved to.
fn resolve_member(
    st: &SymbolTable,
    file_id: FileId,
    container_ref_idx: usize,
    member_name: &str,
) -> Option<DeclId> {
    // 1. Get the container reference's resolved DeclId.
    let container_decl_id = {
        let fi = st.files.get(&file_id)?;
        let container_ref = fi.references.get(container_ref_idx)?;
        container_ref.resolved?
    };

    // 2. Get the container declaration.
    let container_decl = st.get_declaration(&container_decl_id)?;
    let container_kind = container_decl.kind;
    let container_file = container_decl_id.file;

    match container_kind {
        // Namespace kinds: search members directly.
        DeclKind::Contract | DeclKind::Interface | DeclKind::Library => {
            find_member_in_scope(st, &container_decl_id, member_name)
        }
        DeclKind::Struct | DeclKind::Enum => {
            find_member_by_decl_id(st, &container_decl_id, member_name)
        }
        // Variable kinds: resolve the type, then search inside it.
        DeclKind::StateVariable
        | DeclKind::LocalVariable
        | DeclKind::Parameter
        | DeclKind::Constant => {
            let type_text = st.get_declaration(&container_decl_id)?.type_text.clone()?;
            let base_type = strip_type_modifiers(&type_text);
            // Try regular type-based member lookup first.
            if let Some(type_decl_id) = find_type_declaration(st, container_file, base_type) {
                let type_decl = st.get_declaration(&type_decl_id)?;
                match type_decl.kind {
                    DeclKind::Contract | DeclKind::Interface | DeclKind::Library => {
                        return find_member_in_scope(st, &type_decl_id, member_name);
                    }
                    DeclKind::Struct | DeclKind::Enum => {
                        return find_member_by_decl_id(st, &type_decl_id, member_name);
                    }
                    _ => {}
                }
            }
            // Fallback: check using-for directives.
            resolve_using_for_member(st, file_id, &type_text, member_name)
        }
        // Function kinds: resolve the return type, then search inside it.
        DeclKind::Function | DeclKind::Constructor => {
            let container = st.get_declaration(&container_decl_id)?;
            let ret_params = container.return_parameters();
            // Single return type — resolve member on that type.
            if ret_params.len() == 1 {
                let ret_type = &ret_params[0].0;
                let base_type = strip_type_modifiers(ret_type);
                let type_decl_id = find_type_declaration(st, container_file, base_type)?;
                let type_decl = st.get_declaration(&type_decl_id)?;
                match type_decl.kind {
                    DeclKind::Contract | DeclKind::Interface | DeclKind::Library => {
                        find_member_in_scope(st, &type_decl_id, member_name)
                    }
                    DeclKind::Struct | DeclKind::Enum => {
                        find_member_by_decl_id(st, &type_decl_id, member_name)
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        // Import alias: search the imported file's top-level declarations.
        DeclKind::ImportAlias => {
            resolve_import_alias_member(st, file_id, &container_decl_id, member_name)
        }
        _ => None,
    }
}

/// Resolve a name inside a base contract (for inheritance lookup).
/// Finds the base contract declaration (in the current file or imports), then
/// searches its scope for the name.
fn resolve_in_base_contract(
    st: &SymbolTable,
    file_id: FileId,
    base_name: &str,
    member_name: &str,
) -> Option<DeclId> {
    // Find the base contract declaration.
    let base_decl_id = find_type_declaration(st, file_id, base_name)?;
    // Search its scope for the member.
    if let Some(found) = find_member_in_scope(st, &base_decl_id, member_name) {
        return Some(found);
    }
    // Recurse through grandparent bases.
    let base_decl = st.get_declaration(&base_decl_id)?;
    let grandparent_names: Vec<String> = base_decl.base_contracts().to_vec();
    for gp_name in &grandparent_names {
        if let Some(found) = resolve_in_base_contract(st, base_decl_id.file, gp_name, member_name)
        {
            return Some(found);
        }
    }
    None
}

/// Resolve a member via using-for directives (e.g., `using SafeMath for uint256`).
fn resolve_using_for_member(
    st: &SymbolTable,
    file_id: FileId,
    type_text: &str,
    member_name: &str,
) -> Option<DeclId> {
    let fi = st.files.get(&file_id)?;
    let stripped = strip_type_modifiers(type_text);
    for using in &fi.using_directives {
        let applies = match &using.target_type {
            None => true, // `using X for *`
            Some(target) => strip_type_modifiers(target) == stripped,
        };
        if !applies {
            continue;
        }
        if let Some(lib_decl_id) = find_type_declaration(st, file_id, &using.library_name) {
            if let Some(found) = find_member_in_scope(st, &lib_decl_id, member_name) {
                return Some(found);
            }
        }
    }
    None
}

/// Find a member inside a contract/interface/library by searching its scope.
fn find_member_in_scope(
    st: &SymbolTable,
    container_decl_id: &DeclId,
    member_name: &str,
) -> Option<DeclId> {
    let fi = st.files.get(&container_decl_id.file)?;
    let container_decl = fi.declarations.get(container_decl_id)?;

    // Find the scope whose range is within the container's full_range
    // and has a matching ScopeKind.
    for scope in &fi.scopes {
        let in_range = scope.range.0 >= container_decl.full_range.0
            && scope.range.1 <= container_decl.full_range.1;
        let is_ns_scope = matches!(
            scope.kind,
            ScopeKind::Contract | ScopeKind::Interface | ScopeKind::Library
        );
        if in_range && is_ns_scope {
            if let Some(decl_id) = scope.get_decl(member_name) {
                return Some(*decl_id);
            }
        }
    }
    None
}

/// Find a member of a struct/enum via MemberInfo.decl_id.
fn find_member_by_decl_id(
    st: &SymbolTable,
    container_decl_id: &DeclId,
    member_name: &str,
) -> Option<DeclId> {
    let container_decl = st.get_declaration(container_decl_id)?;
    for member in container_decl.members() {
        if member.name == member_name {
            return member.decl_id;
        }
    }
    None
}

/// Find a type declaration by name, searching the given file and its imports.
fn find_type_declaration(st: &SymbolTable, origin_file: FileId, type_name: &str) -> Option<DeclId> {
    // Collect import target FileIds without cloning ImportInfo.
    let import_fids: Vec<FileId>;
    if let Some(fi) = st.files.get(&origin_file) {
        // Search current file.
        for decl in fi.declarations.values() {
            if decl.name == type_name && is_member_bearing_kind(decl.kind) {
                return Some(decl.id);
            }
        }
        // Collect resolved import FileIds (cheap — just u32 copies).
        import_fids = fi
            .imports
            .iter()
            .filter_map(|imp| imp.resolved_path.as_ref())
            .filter_map(|p| st.interner.lookup(p))
            .collect();
    } else {
        return None;
    }
    // Search imported files (no longer borrows fi).
    for target_fid in import_fids {
        if let Some(target_fi) = st.files.get(&target_fid) {
            if let Some(decl) = find_top_level_by_name(target_fi, type_name) {
                if is_member_bearing_kind(decl.kind) {
                    return Some(decl.id);
                }
            }
        }
    }
    None
}

/// Strip array brackets, `memory`, `storage`, `calldata` suffixes from a type.
fn strip_type_modifiers(type_text: &str) -> &str {
    let s = type_text.trim();
    // Extract value type from mapping: mapping(K => V) → V
    if s.starts_with("mapping(") {
        if let Some(arrow) = s.find("=>") {
            let after_arrow = &s[arrow + 2..];
            // Find the matching closing paren, handling nested mappings.
            let mut depth = 0i32;
            let mut end = after_arrow.len();
            for (i, c) in after_arrow.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        if depth == 0 {
                            end = i;
                            break;
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
            }
            let value_type = after_arrow[..end].trim();
            // Recursively strip in case of nested mappings or arrays.
            return strip_type_modifiers(value_type);
        }
    }
    let s = s
        .strip_suffix(" memory")
        .or_else(|| s.strip_suffix(" storage"))
        .or_else(|| s.strip_suffix(" calldata"))
        .unwrap_or(s);
    if let Some(bracket_pos) = s.find('[') {
        &s[..bracket_pos]
    } else {
        s
    }
    .trim()
}

/// Resolve `Alias.Member` where `Alias` is an import alias.
/// Handles both `import "X" as Alias` and `import {Name} from "X"` where
/// `Name` is a contract/library/interface and `Member` is accessed via dot.
fn resolve_import_alias_member(
    st: &SymbolTable,
    file_id: FileId,
    alias_decl_id: &DeclId,
    member_name: &str,
) -> Option<DeclId> {
    let fi = st.files.get(&file_id)?;
    let alias_decl = fi.declarations.get(alias_decl_id)?;
    let alias_name = &alias_decl.name;

    for imp in &fi.imports {
        let matches = match &imp.kind {
            ImportKind::Alias(alias) => alias == alias_name,
            ImportKind::Named(names) => names.iter().any(|(name, al)| {
                let local = al.as_ref().unwrap_or(name);
                local == alias_name
            }),
            ImportKind::Glob => false,
        };
        if !matches {
            continue;
        }

        if let Some(ref resolved_path) = imp.resolved_path {
            if let Some(target_fid) = st.interner.lookup(resolved_path) {
                if let Some(target_fi) = st.files.get(&target_fid) {
                    // For Alias imports, search top-level declarations.
                    if matches!(imp.kind, ImportKind::Alias(_)) {
                        if let Some(decl) = find_top_level_by_name(target_fi, member_name) {
                            return Some(decl.id);
                        }
                    }
                    // For Named imports, the alias is a specific type — search its
                    // scope for the member (e.g. MathLib.add where MathLib is a library).
                    if let ImportKind::Named(names) = &imp.kind {
                        let original_name = names
                            .iter()
                            .find(|(name, al)| {
                                let local = al.as_ref().unwrap_or(name);
                                local == alias_name
                            })
                            .map(|(name, _)| name.as_str())
                            .unwrap_or(alias_name);
                        // Find the actual declaration (contract/library/interface) in the target file.
                        if let Some(container_decl) =
                            find_top_level_by_name(target_fi, original_name)
                        {
                            let container_decl_id = container_decl.id;
                            let container_kind = container_decl.kind;
                            match container_kind {
                                DeclKind::Contract | DeclKind::Interface | DeclKind::Library => {
                                    return find_member_in_scope(
                                        st,
                                        &container_decl_id,
                                        member_name,
                                    );
                                }
                                DeclKind::Struct | DeclKind::Enum => {
                                    return find_member_by_decl_id(
                                        st,
                                        &container_decl_id,
                                        member_name,
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }
    None
}
