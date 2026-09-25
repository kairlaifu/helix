use crate::job;
use helix_core::syntax::config::LanguageServerFeature;
use helix_event::{cancelable_future, register_hook};
use helix_lsp::lsp::{DocumentSymbol, DocumentSymbolResponse, SymbolInformation};
use helix_view::{
    events::{
        ConfigDidChange, DocumentDidChange, DocumentDidOpen, LanguageServerExited,
        LanguageServerInitialized, SelectionDidChange,
    },
    handlers::Handlers,
    DocumentId, Editor,
};

fn flat_symbols_to_nested(symbols: Vec<SymbolInformation>) -> Vec<DocumentSymbol> {
    fn contains(outer: helix_lsp::lsp::Range, inner: helix_lsp::lsp::Range) -> bool {
        outer.start <= inner.start && outer.end >= inner.end
    }

    fn insert_symbol(symbols: &mut Vec<DocumentSymbol>, symbol: DocumentSymbol) {
        if let Some(parent) = symbols
            .iter_mut()
            .find(|parent| contains(parent.range, symbol.range))
        {
            insert_symbol(parent.children.get_or_insert_with(Vec::new), symbol);
        } else {
            symbols.push(symbol);
        }
    }

    let mut symbols = symbols
        .into_iter()
        .map(|symbol| DocumentSymbol {
            name: symbol.name,
            detail: None,
            kind: symbol.kind,
            tags: symbol.tags,
            deprecated: None,
            range: symbol.location.range,
            selection_range: symbol.location.range,
            children: None,
        })
        .collect::<Vec<_>>();

    symbols.sort_by_key(|symbol| (symbol.range.start, std::cmp::Reverse(symbol.range.end)));

    let mut nested = Vec::new();
    for symbol in symbols {
        insert_symbol(&mut nested, symbol);
    }
    nested
}

fn request_document_symbols(editor: &mut Editor, doc_id: DocumentId) {
    if !editor.config().breadcrumb.enable {
        return;
    }

    let Some(doc) = editor.document_mut(doc_id) else {
        return;
    };

    let Some(language_server) = doc
        // Get the first LSP Server that supports `DocumentSymbols`.
        .language_servers_with_feature(LanguageServerFeature::DocumentSymbols)
        .next()
    else {
        return;
    };

    let offset_encoding = language_server.offset_encoding();
    let Some(future) = language_server.document_symbols(doc.identifier()) else {
        return;
    };
    let cancel = doc.document_symbols_controller.restart();

    tokio::spawn(async move {
        let Some(Ok(Some(response))) = cancelable_future(future, &cancel).await else {
            return;
        };

        job::dispatch(move |editor, _| {
            if let Some(doc) = editor.document_mut(doc_id) {
                match response {
                    DocumentSymbolResponse::Nested(symbols) => {
                        doc.set_document_symbols(symbols, offset_encoding);
                    }
                    DocumentSymbolResponse::Flat(symbols) => {
                        doc.set_document_symbols(flat_symbols_to_nested(symbols), offset_encoding);
                    }
                }
            }
        })
        .await;
    });
}

#[cfg(test)]
mod tests {
    use super::flat_symbols_to_nested;
    use helix_lsp::lsp::{Location, Position, Range, SymbolInformation, SymbolKind};
    use url::Url;

    #[test]
    fn flat_symbols_are_nested_by_range() {
        let uri = Url::parse("file:///Test.java").unwrap();
        let symbols = flat_symbols_to_nested(vec![
            SymbolInformation {
                name: "method".into(),
                kind: SymbolKind::METHOD,
                tags: None,
                deprecated: None,
                location: Location {
                    uri: uri.clone(),
                    range: Range::new(Position::new(2, 4), Position::new(4, 5)),
                },
                container_name: Some("Test".into()),
            },
            SymbolInformation {
                name: "Test".into(),
                kind: SymbolKind::CLASS,
                tags: None,
                deprecated: None,
                location: Location {
                    uri,
                    range: Range::new(Position::new(0, 0), Position::new(6, 1)),
                },
                container_name: None,
            },
        ]);

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Test");
        assert_eq!(symbols[0].children.as_ref().unwrap()[0].name, "method");
    }
}

pub(super) fn register_hooks(_handlers: &Handlers) {
    register_hook!(move |event: &mut DocumentDidOpen<'_>| {
        let doc_id = event.doc;
        let view_id = event.editor.tree.focus;
        request_document_symbols(event.editor, doc_id);
        if let Some(doc) = event.editor.document_mut(doc_id) {
            doc.update_breadcrumbs_for_view(view_id);
        }
        Ok(())
    });

    register_hook!(move |event: &mut DocumentDidChange<'_>| {
        if !event.ghost_transaction {
            // Cancel the ongoing request, if present.
            event.doc.document_symbols_controller.cancel();
            let view_id = event.view;
            let doc_id = event.doc.id();
            job::dispatch_blocking(move |editor, _| {
                request_document_symbols(editor, doc_id);
                if let Some(doc) = editor.document_mut(doc_id) {
                    doc.update_breadcrumbs_for_view(view_id);
                }
            });
        }
        Ok(())
    });

    register_hook!(move |event: &mut LanguageServerInitialized<'_>| {
        let view_id = event.editor.tree.focus;
        if let Some(view) = event.editor.tree.try_get(view_id) {
            let doc_id = view.doc;
            request_document_symbols(event.editor, doc_id);
            if let Some(doc) = event.editor.document_mut(doc_id) {
                doc.update_breadcrumbs_for_view(view_id);
            }
        }
        Ok(())
    });

    register_hook!(move |event: &mut LanguageServerExited<'_>| {
        for doc in event.editor.documents_mut() {
            if doc.supports_language_server(event.server_id) {
                doc.clear_document_symbols();
            }
        }
        Ok(())
    });

    register_hook!(move |event: &mut ConfigDidChange<'_>| {
        if !event.old.breadcrumb.enable && event.new.breadcrumb.enable {
            let view_id = event.editor.tree.focus;
            if let Some(view) = event.editor.tree.try_get(view_id) {
                let doc_id = view.doc;
                request_document_symbols(event.editor, doc_id);
                if let Some(doc) = event.editor.document_mut(doc_id) {
                    doc.update_breadcrumbs_for_view(view_id);
                }
            }
            return Ok(());
        }

        if event.old.breadcrumb.enable && !event.new.breadcrumb.enable {
            for doc in event.editor.documents_mut() {
                doc.clear_document_symbols();
            }
        }

        Ok(())
    });

    register_hook!(move |event: &mut SelectionDidChange<'_>| {
        let doc_id = event.doc.id();
        let view_id = event.view;

        job::dispatch_blocking(move |editor, _| {
            if let Some(doc) = editor.document_mut(doc_id) {
                doc.update_breadcrumbs_for_view_inlined(view_id);
            }
        });
        Ok(())
    });
}
