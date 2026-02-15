use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tree_sitter::Node;

use crate::import_resolver::ImportResolver;
use crate::parser::TsParser;

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
}

impl Reference {
    /// Get the name from source text on demand instead of storing it.
    pub fn name<'a>(&self, source: &'a str) -> &'a str {
        &source[self.range.0..self.range.1]
    }
}

/// Per-file index.
#[derive(Debug, Clone)]
pub struct FileIndex {
    pub file_id: FileId,
    pub scopes: Vec<Scope>,
    pub declarations: HashMap<DeclId, Declaration>,
    pub references: Vec<Reference>,
    pub imports: Vec<ImportInfo>,
}

/// Project-wide symbol table.
pub struct SymbolTable {
    pub files: HashMap<FileId, FileIndex>,
    pub interner: PathInterner,
    pub resolver: ImportResolver,
    /// Reverse index: DeclId → list of (FileId, start_byte, end_byte). (Fix #18)
    pub ref_index: HashMap<DeclId, Vec<(FileId, usize, usize)>>,
    /// Source text cache for resolving reference names. (Fix #12)
    sources: HashMap<FileId, String>,
}

// ---------------------------------------------------------------------------
// SymbolTable API
// ---------------------------------------------------------------------------

impl SymbolTable {
    pub fn new(resolver: ImportResolver) -> Self {
        Self {
            files: HashMap::new(),
            interner: PathInterner::default(),
            resolver,
            ref_index: HashMap::new(),
            sources: HashMap::new(),
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
        self.sources.get(&file_id).map(|s| s.as_str())
    }

    /// Index a single file. Replaces any existing index for this path.
    pub fn index_file(&mut self, path: &Path, source: &str, parser: &mut TsParser) {
        let tree = match parser.parse(source, None) {
            Some(t) => t,
            None => return,
        };
        let file_id = self.interner.get_or_intern(path);

        // Remove old reverse index entries for this file.
        self.ref_index.retain(|_, refs| {
            refs.retain(|(fid, _, _)| *fid != file_id);
            !refs.is_empty()
        });

        let file_index = build_file_index(file_id, source, &tree.root_node(), &self.resolver, path);
        self.files.insert(file_id, file_index);
        self.sources.insert(file_id, source.to_string());
    }

    /// Index from an already-parsed tree — avoids double parsing. (Fix #1)
    pub fn index_file_with_tree(&mut self, path: &Path, source: &str, tree: &tree_sitter::Tree) {
        let file_id = self.interner.get_or_intern(path);

        // Remove old reverse index entries for this file.
        self.ref_index.retain(|_, refs| {
            refs.retain(|(fid, _, _)| *fid != file_id);
            !refs.is_empty()
        });

        let file_index = build_file_index(file_id, source, &tree.root_node(), &self.resolver, path);
        self.files.insert(file_id, file_index);
        self.sources.insert(file_id, source.to_string());
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
            self.ref_index.retain(|_, refs| {
                refs.retain(|(fid, _, _)| *fid != file_id);
                !refs.is_empty()
            });
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
        let mut seen = std::collections::HashSet::new();
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
        declarations: HashMap::new(),
        references: Vec::new(),
        imports: Vec::new(),
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
        "if_statement" | "try_statement" => {
            walk_children(node, scope_id, file_id, source, fi);
        }
        "identifier" => {
            if !is_declaration_name(node) {
                let text = node_text(node, source);
                if !text.is_empty() {
                    fi.references.push(Reference {
                        range: (node.start_byte(), node.end_byte()),
                        scope: scope_id,
                        resolved: None,
                    });
                }
            }
        }
        "user_defined_type" => {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "identifier" {
                        fi.references.push(Reference {
                            range: (child.start_byte(), child.end_byte()),
                            scope: scope_id,
                            resolved: None,
                        });
                        break;
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "member_expression" => {
            if let Some(obj) = node.child_by_field_name("object") {
                walk_node(&obj, scope_id, file_id, source, fi);
            }
            if let Some(prop) = node.child_by_field_name("property") {
                fi.references.push(Reference {
                    range: (prop.start_byte(), prop.end_byte()),
                    scope: scope_id,
                    resolved: None,
                });
            }
            return;
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
    let return_parameters = extract_return_parameters(node, source);
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
                        members.push(MemberInfo {
                            name: node_text(&mname, source).to_string(),
                            type_text: mtype,
                            kind: DeclKind::StateVariable,
                            name_range: (mname.start_byte(), mname.end_byte()),
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
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "enum_value" {
                    enum_values.push(node_text(&child, source).to_string());
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    let members: Vec<MemberInfo> = enum_values
        .iter()
        .map(|v| MemberInfo {
            name: v.clone(),
            type_text: enum_name.clone(),
            kind: DeclKind::EnumValue,
            name_range: (0, 0),
        })
        .collect();

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

fn extract_return_parameters(node: &Node, source: &str) -> Vec<(String, String)> {
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
        let unresolved: Vec<(usize, usize, usize, ScopeId)> = fi
            .references
            .iter()
            .enumerate()
            .filter(|(_, r)| r.resolved.is_none())
            .map(|(i, r)| (i, r.range.0, r.range.1, r.scope))
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

    for (idx, start, end, scope_id) in unresolved {
        let name = &source[start..end];
        let resolved = resolve_single(
            name,
            scope_id,
            &scope_snapshot,
            &import_snapshot,
            file_id,
            st,
        );

        // Update the reference and add to reverse index.
        if let Some(ref decl_id) = resolved {
            st.ref_index
                .entry(*decl_id)
                .or_default()
                .push((file_id, start, end));
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn index(source: &str) -> (SymbolTable, PathBuf) {
        let mut parser = TsParser::new();
        let path = PathBuf::from("test.sol");
        let resolver = ImportResolver::with_root(PathBuf::from("."));
        let mut st = SymbolTable::new(resolver);
        st.index_file(&path, source, &mut parser);
        st.resolve_file_references(&path, &mut parser);
        (st, path)
    }

    fn get_fi<'a>(st: &'a SymbolTable, path: &Path) -> &'a FileIndex {
        st.get_file_index(path).unwrap()
    }

    #[test]
    fn test_contract_declaration() {
        let source = r#"
contract Foo {
    uint256 public x;
    function bar() public returns (uint256) {
        return x;
    }
}
"#;
        let (st, path) = index(source);
        let fi = get_fi(&st, &path);

        let names: Vec<&str> = fi.declarations.values().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"Foo"), "names: {names:?}");
        assert!(names.contains(&"x"), "names: {names:?}");
        assert!(names.contains(&"bar"), "names: {names:?}");

        let foo = fi.declarations.values().find(|d| d.name == "Foo").unwrap();
        assert_eq!(foo.kind, DeclKind::Contract);
        assert_eq!(foo.members().len(), 2);
    }

    #[test]
    fn test_struct_members() {
        let source = r#"
contract Foo {
    struct Point {
        uint256 x;
        uint256 y;
    }
}
"#;
        let (st, path) = index(source);
        let fi = get_fi(&st, &path);
        let point = fi
            .declarations
            .values()
            .find(|d| d.name == "Point")
            .unwrap();
        assert_eq!(point.kind, DeclKind::Struct);
        assert_eq!(point.members().len(), 2);
        assert_eq!(point.members()[0].name, "x");
        assert_eq!(point.members()[1].name, "y");
    }

    #[test]
    fn test_function_parameters() {
        let source = r#"
contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}
"#;
        let (st, path) = index(source);
        let fi = get_fi(&st, &path);
        let add = fi.declarations.values().find(|d| d.name == "add").unwrap();
        let params = add.parameters();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], ("uint256".to_string(), "a".to_string()));
        assert_eq!(params[1], ("uint256".to_string(), "b".to_string()));
        let returns = add.return_parameters();
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].0, "uint256");
    }

    #[test]
    fn test_local_variable_resolution() {
        let source = r#"
contract Foo {
    function bar() public {
        uint256 x = 42;
        uint256 y = x;
    }
}
"#;
        let (st, path) = index(source);
        let fi = get_fi(&st, &path);

        let x_refs: Vec<&Reference> = fi
            .references
            .iter()
            .filter(|r| r.name(source) == "x" && r.resolved.is_some())
            .collect();
        assert!(!x_refs.is_empty(), "x should be resolved.");

        let x_decl = fi
            .declarations
            .values()
            .find(|d| d.name == "x" && d.kind == DeclKind::LocalVariable)
            .unwrap();
        assert_eq!(x_refs[0].resolved.as_ref().unwrap(), &x_decl.id);
    }

    #[test]
    fn test_enum_declaration() {
        let source = r#"
contract Foo {
    enum Status { Active, Inactive, Paused }
}
"#;
        let (st, path) = index(source);
        let fi = get_fi(&st, &path);
        let status = fi
            .declarations
            .values()
            .find(|d| d.name == "Status")
            .unwrap();
        assert_eq!(status.kind, DeclKind::Enum);
        assert_eq!(status.enum_values(), &["Active", "Inactive", "Paused"]);
        assert_eq!(status.members().len(), 3);
    }

    #[test]
    fn test_import_parsing() {
        let source = r#"
import "./Foo.sol";
import {Bar, Baz as B} from "./Bar.sol";
import "./Lib.sol" as Lib;
"#;
        let (st, path) = index(source);
        let fi = get_fi(&st, &path);
        assert_eq!(fi.imports.len(), 3);
        assert!(matches!(fi.imports[0].kind, ImportKind::Glob));
        if let ImportKind::Named(ref names) = fi.imports[1].kind {
            assert_eq!(names.len(), 2);
            assert_eq!(names[0].0, "Bar");
            assert_eq!(names[0].1, None);
            assert_eq!(names[1].0, "Baz");
            assert_eq!(names[1].1, Some("B".to_string()));
        } else {
            panic!("Expected Named import");
        }
        if let ImportKind::Alias(ref alias) = fi.imports[2].kind {
            assert_eq!(alias, "Lib");
        } else {
            panic!("Expected Alias import");
        }
    }

    #[test]
    fn test_natspec_extraction() {
        let source = r#"
/// @notice This is a test function
/// @param x The value
function foo(uint256 x) public pure returns (uint256) {
    return x;
}
"#;
        let (st, path) = index(source);
        let fi = get_fi(&st, &path);
        let foo = fi.declarations.values().find(|d| d.name == "foo").unwrap();
        let natspec = foo.natspec.as_ref().unwrap();
        assert!(natspec.contains("@notice This is a test function"));
        assert!(natspec.contains("@param x The value"));
    }

    #[test]
    fn test_resolve_at() {
        let source = r#"
contract Foo {
    uint256 public x;
    function bar() public returns (uint256) {
        return x;
    }
}
"#;
        let (st, path) = index(source);

        let return_x_pos = source.find("return x;").unwrap() + "return ".len();
        let decl = st.resolve_at(&path, return_x_pos);
        assert!(decl.is_some(), "Should resolve x");
        let decl = decl.unwrap();
        assert_eq!(decl.name, "x");
        assert_eq!(decl.kind, DeclKind::StateVariable);
    }

    #[test]
    fn test_inheritance() {
        let source = r#"
contract Base {
    function foo() public virtual {}
}
contract Child is Base {
    function foo() public override {}
}
"#;
        let (st, path) = index(source);
        let fi = get_fi(&st, &path);
        let child = fi
            .declarations
            .values()
            .find(|d| d.name == "Child")
            .unwrap();
        assert_eq!(child.base_contracts(), &["Base"]);
    }

    #[test]
    fn test_scope_nesting() {
        let source = r#"
contract Foo {
    function bar() public {
        uint256 a = 1;
        {
            uint256 b = 2;
        }
    }
}
"#;
        let (st, path) = index(source);
        let fi = get_fi(&st, &path);

        let a = fi.declarations.values().find(|d| d.name == "a").unwrap();
        let b = fi.declarations.values().find(|d| d.name == "b").unwrap();
        assert_ne!(a.scope, b.scope, "a and b should be in different scopes");

        let b_scope = &fi.scopes[b.scope];
        assert_eq!(b_scope.parent, Some(a.scope));
    }

    #[test]
    fn test_slim_declaration_no_extras_for_variables() {
        let source = r#"
contract Foo {
    function bar() public {
        uint256 x = 1;
    }
}
"#;
        let (st, path) = index(source);
        let fi = get_fi(&st, &path);
        let x = fi.declarations.values().find(|d| d.name == "x").unwrap();
        // Local variables should have no extras allocated.
        assert!(x.extras.is_none(), "local var should not allocate extras");
    }
}
