use crate::completion;
use crate::goto;
use crate::hover;
use crate::import_resolver::ImportResolver;
use crate::links;
use crate::lint::LintEngine;
use crate::parser::{self, TsParser};
use crate::references;
use crate::rename;
use crate::solar_checker;
use crate::symbol_table::SymbolTable;
use crate::symbols;
use crate::utils;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::{Client, LanguageServer, lsp_types::*};

// ---------------------------------------------------------------------------
// Worker message types
// ---------------------------------------------------------------------------

struct TsWorkerMsg {
    uri: Url,
    file_path: PathBuf,
    text: Arc<str>, // (Fix #14) shared ownership, no clone
    version: i32,
}

struct SolarWorkerMsg {
    uri: Url,
    file_path: PathBuf,
    /// Monotonic counter to detect stale results. (Fix #2)
    seq: u64,
}

// ---------------------------------------------------------------------------
// Workers
// ---------------------------------------------------------------------------

/// Long-lived tree-sitter worker. Parses once, then lints + indexes from
/// the same tree. (Fix #1)
async fn ts_worker(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<TsWorkerMsg>,
    client: Client,
    ts_parser: Arc<tokio::sync::Mutex<TsParser>>,
    lint_engine: Arc<LintEngine>,
    symbol_table: Arc<RwLock<SymbolTable>>,
    ts_diag_cache: Arc<RwLock<HashMap<String, Vec<Diagnostic>>>>,
) {
    while let Some(mut msg) = rx.recv().await {
        // Drain queued messages — only process the latest.
        while let Ok(newer) = rx.try_recv() {
            msg = newer;
        }

        // Parse once. (Fix #1)
        let tree = {
            let mut parser = ts_parser.lock().await;
            match parser.parse(&msg.text, None) {
                Some(t) => t,
                None => continue,
            }
        };

        // Lint from the parsed tree.
        let mut diags = parser::collect_parse_errors(&tree, &msg.text);
        diags.extend(lint_engine.run(&tree, &msg.text));

        // Cache tree-sitter diagnostics (moved, not cloned). (Fix #13)
        {
            let mut cache = ts_diag_cache.write().await;
            cache.insert(msg.uri.to_string(), diags.clone());
        }
        client
            .publish_diagnostics(msg.uri.clone(), diags, Some(msg.version))
            .await;

        // Re-index symbol table using the same tree. (Fix #1)
        {
            let mut parser = ts_parser.lock().await;
            let mut st = symbol_table.write().await;
            st.index_file_with_tree(&msg.file_path, &msg.text, &tree);
            st.resolve_file_references(&msg.file_path, &mut parser);
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Long-lived solar worker with sequence-based staleness check. (Fix #2)
async fn solar_worker(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<SolarWorkerMsg>,
    client: Client,
    ts_diag_cache: Arc<RwLock<HashMap<String, Vec<Diagnostic>>>>,
    latest_seq: Arc<std::sync::atomic::AtomicU64>,
    symbol_table: Arc<RwLock<SymbolTable>>,
) {
    while let Some(mut msg) = rx.recv().await {
        // Drain queued messages — only process the latest.
        while let Ok(newer) = rx.try_recv() {
            msg = newer;
        }

        let my_seq = msg.seq;

        // Read resolver config from symbol table.
        let solar_config = {
            let st = symbol_table.read().await;
            solar_checker::SolarConfig {
                remappings: st.resolver.remappings().to_vec(),
                include_paths: st.resolver.include_paths().to_vec(),
                base_path: Some(st.resolver.project_root().to_path_buf()),
            }
        };

        // Run solar on a blocking thread.
        let file_path = msg.file_path.clone();
        let solar_diags = tokio::task::spawn_blocking(move || {
            solar_checker::check_file(&file_path, &solar_config)
        })
        .await
        .unwrap_or_default();

        // Check if a newer ts_worker result has arrived since we started.
        // If so, our ts_diag_cache may be stale — skip merging. (Fix #2)
        let current_seq = latest_seq.load(std::sync::atomic::Ordering::Acquire);
        if my_seq < current_seq {
            // Stale — a newer version was processed by ts_worker. Skip.
            continue;
        }

        // Merge with cached tree-sitter diagnostics.
        let uri_str = msg.uri.to_string();
        let ts_diags = ts_diag_cache
            .read()
            .await
            .get(&uri_str)
            .cloned()
            .unwrap_or_default();
        let mut merged = ts_diags;
        merged.extend(solar_diags);

        client.publish_diagnostics(msg.uri, merged, None).await;

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

// ---------------------------------------------------------------------------
// LSP server
// ---------------------------------------------------------------------------

pub struct SolLsp {
    client: Client,
    symbol_table: Arc<RwLock<SymbolTable>>,
    /// In-memory text buffers using Arc<str> for zero-copy sharing. (Fix #14)
    text_cache: Arc<RwLock<HashMap<String, Arc<str>>>>,
    ts_parser: Arc<tokio::sync::Mutex<TsParser>>,
    lint_engine: Arc<LintEngine>,
    ts_tx: tokio::sync::mpsc::UnboundedSender<TsWorkerMsg>,
    solar_tx: tokio::sync::mpsc::UnboundedSender<SolarWorkerMsg>,
    ts_rx: Arc<tokio::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<TsWorkerMsg>>>>,
    solar_rx: Arc<tokio::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<SolarWorkerMsg>>>>,
    ts_diag_cache: Arc<RwLock<HashMap<String, Vec<Diagnostic>>>>,
    /// Monotonic sequence number for staleness detection. (Fix #2)
    seq: Arc<std::sync::atomic::AtomicU64>,
}

impl SolLsp {
    pub fn new(client: Client) -> Self {
        // Use "." as placeholder; will be updated in initialize(). (Fix #25)
        let resolver = ImportResolver::new(std::path::Path::new("."));
        let symbol_table = Arc::new(RwLock::new(SymbolTable::new(resolver)));
        let text_cache = Arc::new(RwLock::new(HashMap::new()));
        let ts_parser = Arc::new(tokio::sync::Mutex::new(TsParser::new()));
        let lint_engine = Arc::new(LintEngine::new());
        let ts_diag_cache = Arc::new(RwLock::new(HashMap::new()));

        let (ts_tx, ts_rx) = tokio::sync::mpsc::unbounded_channel();
        let (solar_tx, solar_rx) = tokio::sync::mpsc::unbounded_channel();

        Self {
            client,
            symbol_table,
            text_cache,
            ts_parser,
            lint_engine,
            ts_tx,
            solar_tx,
            ts_rx: Arc::new(tokio::sync::Mutex::new(Some(ts_rx))),
            solar_rx: Arc::new(tokio::sync::Mutex::new(Some(solar_rx))),
            ts_diag_cache,
            seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    async fn get_source_and_path(&self, uri: &Url) -> Option<(PathBuf, String)> {
        let file_path = uri.to_file_path().ok()?;
        let text_cache = self.text_cache.read().await;
        let source = if let Some(cached) = text_cache.get(uri.as_str()) {
            cached.to_string()
        } else {
            std::fs::read_to_string(&file_path).ok()?
        };
        Some((file_path, source))
    }

    /// Send a file to both workers for processing. (Fix #24: text is Arc<str>)
    fn notify_workers(&self, uri: &Url, file_path: &PathBuf, text: &Arc<str>, version: i32) {
        let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::Release) + 1;
        let _ = self.ts_tx.send(TsWorkerMsg {
            uri: uri.clone(),
            file_path: file_path.clone(),
            text: Arc::clone(text), // no copy, just refcount bump
            version,
        });
        let _ = self.solar_tx.send(SolarWorkerMsg {
            uri: uri.clone(),
            file_path: file_path.clone(),
            seq,
        });
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for SolLsp {
    async fn initialize(
        &self,
        params: InitializeParams,
    ) -> tower_lsp::jsonrpc::Result<InitializeResult> {
        let encoding = utils::PositionEncoding::negotiate(
            params
                .capabilities
                .general
                .as_ref()
                .and_then(|g| g.position_encodings.as_deref()),
        );
        utils::set_encoding(encoding);

        // Update the import resolver with the actual workspace root. (Fix #25)
        if let Some(root_uri) = params.root_uri.as_ref() {
            if let Ok(root_path) = root_uri.to_file_path() {
                let resolver = ImportResolver::new(&root_path);
                let mut st = self.symbol_table.write().await;
                st.resolver = resolver;
            }
        } else if let Some(folders) = params.workspace_folders.as_ref() {
            if let Some(folder) = folders.first() {
                if let Ok(root_path) = folder.uri.to_file_path() {
                    let resolver = ImportResolver::new(&root_path);
                    let mut st = self.symbol_table.write().await;
                    st.resolver = resolver;
                }
            }
        }

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "solidity-language-server".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                position_encoding: Some(encoding.to_encoding_kind()),
                definition_provider: Some(OneOf::Left(true)),
                declaration_provider: Some(DeclarationCapability::Simple(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: Some(false),
                    },
                })),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: Some(false),
                    },
                }),
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        will_save: Some(true),
                        will_save_wait_until: None,
                        open_close: Some(true),
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(true),
                        })),
                        change: Some(TextDocumentSyncKind::FULL),
                    },
                )),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        if let Some(ts_rx) = self.ts_rx.lock().await.take() {
            tokio::spawn(ts_worker(
                ts_rx,
                self.client.clone(),
                self.ts_parser.clone(),
                self.lint_engine.clone(),
                self.symbol_table.clone(),
                self.ts_diag_cache.clone(),
            ));
        }
        if let Some(solar_rx) = self.solar_rx.lock().await.take() {
            tokio::spawn(solar_worker(
                solar_rx,
                self.client.clone(),
                self.ts_diag_cache.clone(),
                self.seq.clone(),
                self.symbol_table.clone(),
            ));
        }

        self.client
            .log_message(MessageType::INFO, "solidity-language-server initialized")
            .await;
    }

    async fn shutdown(&self) -> tower_lsp::jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text: Arc<str> = params.text_document.text.into();
        let version = params.text_document.version;

        if let Ok(file_path) = uri.to_file_path() {
            self.text_cache
                .write()
                .await
                .insert(uri.to_string(), Arc::clone(&text));
            self.notify_workers(&uri, &file_path, &text, version);
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;

        if let Some(change) = params.content_changes.into_iter().next() {
            let text: Arc<str> = change.text.into();
            self.text_cache
                .write()
                .await
                .insert(uri.to_string(), Arc::clone(&text));
            if let Ok(file_path) = uri.to_file_path() {
                self.notify_workers(&uri, &file_path, &text, version);
            }
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        let text: Arc<str> = match params.text {
            Some(t) => t.into(),
            None => {
                // Fix #3: use to_file_path() instead of uri.path()
                match uri.to_file_path() {
                    Ok(path) => match std::fs::read_to_string(&path) {
                        Ok(c) => c.into(),
                        Err(_) => return,
                    },
                    Err(_) => return,
                }
            }
        };

        if let Ok(file_path) = uri.to_file_path() {
            self.text_cache
                .write()
                .await
                .insert(uri.to_string(), Arc::clone(&text));
            self.notify_workers(&uri, &file_path, &text, 0);
        }
    }

    async fn will_save(&self, _params: WillSaveTextDocumentParams) {}

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri_str = params.text_document.uri.to_string();
        self.ts_diag_cache.write().await.remove(&uri_str);
        self.text_cache.write().await.remove(&uri_str);
        // Fix #7: also remove from symbol table.
        if let Ok(file_path) = params.text_document.uri.to_file_path() {
            self.symbol_table.write().await.remove_file(&file_path);
        }
    }

    async fn did_change_configuration(&self, _: DidChangeConfigurationParams) {}

    async fn did_change_workspace_folders(&self, _: DidChangeWorkspaceFoldersParams) {}

    async fn did_change_watched_files(&self, _: DidChangeWatchedFilesParams) {}

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let (file_path, source) = match self.get_source_and_path(uri).await {
            Some(v) => v,
            None => return Ok(None),
        };

        let st = self.symbol_table.read().await;
        match goto::goto_definition(&st, &file_path, &source, position) {
            Some(loc) => Ok(Some(GotoDefinitionResponse::from(loc))),
            None => Ok(None),
        }
    }

    async fn goto_declaration(
        &self,
        params: request::GotoDeclarationParams,
    ) -> tower_lsp::jsonrpc::Result<Option<request::GotoDeclarationResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let (file_path, source) = match self.get_source_and_path(uri).await {
            Some(v) => v,
            None => return Ok(None),
        };

        let st = self.symbol_table.read().await;
        match goto::goto_definition(&st, &file_path, &source, position) {
            Some(loc) => Ok(Some(request::GotoDeclarationResponse::from(loc))),
            None => Ok(None),
        }
    }

    async fn references(
        &self,
        params: ReferenceParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;

        let (file_path, source) = match self.get_source_and_path(uri).await {
            Some(v) => v,
            None => return Ok(None),
        };

        let st = self.symbol_table.read().await;
        let locations =
            references::find_references(&st, &file_path, &source, position, include_declaration);
        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(locations))
        }
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<PrepareRenameResponse>> {
        let uri = &params.text_document.uri;
        let position = params.position;

        let (_file_path, source) = match self.get_source_and_path(uri).await {
            Some(v) => v,
            None => return Ok(None),
        };

        match rename::get_identifier_range(&source, position) {
            Some(range) => Ok(Some(PrepareRenameResponse::Range(range))),
            None => Ok(None),
        }
    }

    async fn rename(
        &self,
        params: RenameParams,
    ) -> tower_lsp::jsonrpc::Result<Option<WorkspaceEdit>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = &params.new_name;

        let (file_path, source) = match self.get_source_and_path(uri).await {
            Some(v) => v,
            None => return Ok(None),
        };

        let current_identifier = match rename::get_identifier_at_position(&source, position) {
            Some(id) => id,
            None => return Ok(None),
        };

        if !utils::is_valid_solidity_identifier(new_name) {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(
                "new name is not a valid solidity identifier",
            ));
        }

        if *new_name == current_identifier {
            return Ok(None);
        }

        let st = self.symbol_table.read().await;
        Ok(rename::rename_symbol(
            &st, &file_path, &source, position, new_name,
        ))
    }

    async fn hover(&self, params: HoverParams) -> tower_lsp::jsonrpc::Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let (file_path, source) = match self.get_source_and_path(uri).await {
            Some(v) => v,
            None => return Ok(None),
        };

        let st = self.symbol_table.read().await;
        Ok(hover::hover_info(&st, &file_path, &source, position))
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let trigger_char = params
            .context
            .as_ref()
            .and_then(|c| c.trigger_character.as_deref());

        let (file_path, source) = match self.get_source_and_path(uri).await {
            Some(v) => v,
            None => return Ok(None),
        };

        let st = self.symbol_table.read().await;
        Ok(completion::handle_completion(
            &st,
            &file_path,
            &source,
            position,
            trigger_char,
        ))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> tower_lsp::jsonrpc::Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;

        let (file_path, source) = match self.get_source_and_path(uri).await {
            Some(v) => v,
            None => return Ok(None),
        };

        let st = self.symbol_table.read().await;
        let symbols = symbols::document_symbols(&st, &file_path, &source);
        if symbols.is_empty() {
            Ok(None)
        } else {
            Ok(Some(DocumentSymbolResponse::Nested(symbols)))
        }
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<SymbolInformation>>> {
        let st = self.symbol_table.read().await;
        let symbols = symbols::workspace_symbols(&st, &params.query);
        if symbols.is_empty() {
            Ok(None)
        } else {
            Ok(Some(symbols))
        }
    }

    async fn document_link(
        &self,
        params: DocumentLinkParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<DocumentLink>>> {
        let uri = &params.text_document.uri;

        let (file_path, source) = match self.get_source_and_path(uri).await {
            Some(v) => v,
            None => return Ok(None),
        };

        let st = self.symbol_table.read().await;
        let links = links::document_links(&st, &file_path, &source);
        if links.is_empty() {
            Ok(None)
        } else {
            Ok(Some(links))
        }
    }
}
