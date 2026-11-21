use std::{borrow::Cow, sync::Arc};

use ::serde::{Deserialize, Serialize};
use gpui::WeakEntity;
use language::{CachedLspAdapter, Diagnostic, DiagnosticSourceKind};
use lsp::{LanguageServer, LanguageServerName};
use util::ResultExt as _;

use crate::{LspStore, lsp_store::DocumentDiagnosticsUpdate};

pub const CLANGD_SERVER_NAME: LanguageServerName = LanguageServerName::new_static("clangd");
pub const CLICE_SERVER_NAME: LanguageServerName = LanguageServerName::new_static("clice");
const INACTIVE_REGION_MESSAGE: &str = "inactive region";
const INACTIVE_DIAGNOSTIC_SEVERITY: lsp::DiagnosticSeverity = lsp::DiagnosticSeverity::INFORMATION;

fn is_clang_family_server(name: &LanguageServerName) -> bool {
    *name == CLANGD_SERVER_NAME || *name == CLICE_SERVER_NAME
}

fn is_clang_family_source(source: Option<&str>) -> bool {
    source.is_some_and(|value| CLANGD_SERVER_NAME.0 == value || CLICE_SERVER_NAME.0 == value)
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InactiveRegionsParams {
    pub text_document: lsp::OptionalVersionedTextDocumentIdentifier,
    pub regions: Vec<lsp::Range>,
}

/// InactiveRegions is a clangd extension that marks regions of inactive code.
pub struct InactiveRegions;

impl lsp::notification::Notification for InactiveRegions {
    type Params = InactiveRegionsParams;
    const METHOD: &'static str = "textDocument/inactiveRegions";
}

pub fn is_inactive_region(diag: &Diagnostic) -> bool {
    diag.is_unnecessary
        && diag.severity == INACTIVE_DIAGNOSTIC_SEVERITY
        && diag.message.as_str() == INACTIVE_REGION_MESSAGE
        && is_clang_family_source(diag.source.as_deref())
}

pub fn is_lsp_inactive_region(diag: &lsp::Diagnostic) -> bool {
    diag.severity == Some(INACTIVE_DIAGNOSTIC_SEVERITY)
        && diag.message.as_str() == INACTIVE_REGION_MESSAGE
        && is_clang_family_source(diag.source.as_deref())
}

pub fn register_notifications(
    lsp_store: WeakEntity<LspStore>,
    language_server: &LanguageServer,
    adapter: Arc<CachedLspAdapter>,
) {
    let server_name = language_server.name();
    if !is_clang_family_server(&server_name) {
        return;
    }
    let server_id = language_server.server_id();
    let server_source = server_name.to_string();

    language_server
        .on_notification::<InactiveRegions, _>({
            let adapter = adapter;
            let this = lsp_store;

            move |params: InactiveRegionsParams, cx| {
                let adapter = adapter.clone();
                let server_source = server_source.clone();
                this.update(cx, |this, cx| {
                    let diagnostics = params
                        .regions
                        .into_iter()
                        .map(|range| lsp::Diagnostic {
                            range,
                            severity: Some(INACTIVE_DIAGNOSTIC_SEVERITY),
                            source: Some(server_source.clone()),
                            message: lsp::DiagnosticMessage::from(INACTIVE_REGION_MESSAGE),
                            tags: Some(vec![lsp::DiagnosticTag::UNNECESSARY]),
                            ..lsp::Diagnostic::default()
                        })
                        .collect();
                    let mapped_diagnostics = lsp::PublishDiagnosticsParams {
                        uri: params.text_document.uri,
                        version: params.text_document.version,
                        diagnostics,
                    };
                    this.merge_lsp_diagnostics(
                        DiagnosticSourceKind::Pushed,
                        vec![DocumentDiagnosticsUpdate {
                            server_id,
                            diagnostics: mapped_diagnostics,
                            result_id: None,
                            disk_based_sources: Cow::Borrowed(
                                &adapter.disk_based_diagnostic_sources,
                            ),
                            registration_id: None,
                        }],
                        |_, diag, _| !is_inactive_region(diag),
                        cx,
                    )
                    .log_err();
                })
                .ok();
            }
        })
        .detach();
}
