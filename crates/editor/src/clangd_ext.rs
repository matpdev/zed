use std::ops::Range;

use anyhow::Context as _;
use gpui::{App, Context, Entity, TaskExt, Window};
use language::{BufferSnapshot, Language};
use project::{Completion, CompletionSource, lsp_store::lsp_ext_command::SwitchSourceHeaderResult};
use rpc::proto;
use text::ToOffset;
use url::Url;
use util::paths::{PathStyle, UrlExt as _};
use workspace::{OpenOptions, OpenVisible};

use crate::lsp_ext::find_specific_language_server_in_selection;

use crate::{Editor, SwitchSourceHeader, element::register_action};

use project::lsp_store::clangd_ext::{CLANGD_SERVER_NAME, CLICE_SERVER_NAME};

fn is_c_language(language: &Language) -> bool {
    language.name() == "C++" || language.name() == "C"
}

pub fn switch_source_header(
    editor: &mut Editor,
    _: &SwitchSourceHeader,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let Some(project) = &editor.project else {
        return;
    };
    let Some(workspace) = editor.workspace() else {
        return;
    };

    let Some((_, _, server_to_query, buffer)) =
        find_specific_language_server_in_selection(editor, cx, is_c_language, CLICE_SERVER_NAME)
            .or_else(|| {
                find_specific_language_server_in_selection(
                    editor,
                    cx,
                    is_c_language,
                    CLANGD_SERVER_NAME,
                )
            })
    else {
        return;
    };
    let project = project.clone();
    let upstream_client = project.read(cx).lsp_store().read(cx).upstream_client();
    cx.spawn_in(window, async move |_editor, cx| {
        let source_file = buffer.read_with(cx, |buffer, _| {
            buffer
                .file()
                .map(|file| file.path())
                .map(|path| path.display(PathStyle::local()).to_string())
                .unwrap_or_else(|| "Unknown".to_string())
        });

        let switch_source_header = if let Some((client, project_id)) = upstream_client {
            let buffer_id = buffer.read_with(cx, |buffer, _| buffer.remote_id());
            let request = proto::LspExtSwitchSourceHeader {
                project_id,
                buffer_id: buffer_id.to_proto(),
                server_id: server_to_query.to_proto(),
            };
            let response = client
                .request(request)
                .await
                .context("lsp ext switch source header proto request")?;
            SwitchSourceHeaderResult(response.target_file)
        } else {
            project
                .update(cx, |project, cx| {
                    project.request_lsp(
                        buffer,
                        project::LanguageServerToQuery::Other(server_to_query),
                        project::lsp_store::lsp_ext_command::SwitchSourceHeader {
                            server_id: server_to_query,
                        },
                        cx,
                    )
                })
                .await
                .with_context(|| {
                    format!("Switch source/header LSP request for path \"{source_file}\" failed")
                })?
        };

        if switch_source_header.0.is_empty() {
            return Ok(());
        }
        let path_style = workspace.update(cx, |ws, cx| ws.path_style(cx));
        let path = Url::parse(&switch_source_header.0).with_context(|| {
            format!(
                "Parsing URL \"{}\" returned from switch source/header failed",
                switch_source_header.0
            )
        })?;
        let path = path.to_file_path_ext(path_style).map_err(|()| {
            anyhow::anyhow!(
                "URL conversion to file path failed for \"{}\"",
                switch_source_header.0
            )
        })?;

        workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.open_abs_path(
                    path,
                    OpenOptions {
                        visible: Some(OpenVisible::None),
                        ..Default::default()
                    },
                    window,
                    cx,
                )
            })
            .with_context(|| {
                format!(
                    "Switch source/header could not open \"{}\" in workspace",
                    switch_source_header.0
                )
            })?
            .await
            .map(|_| ())
    })
    .detach_and_log_err(cx);
}

pub fn apply_related_actions(editor: &Entity<Editor>, window: &mut Window, cx: &mut App) {
    if editor
        .read(cx)
        .buffer()
        .read(cx)
        .all_buffers()
        .into_iter()
        .filter_map(|buffer| buffer.read(cx).language())
        .any(|language| is_c_language(language))
    {
        register_action(editor, window, switch_source_header);
    }
}

pub(crate) fn expand_out_of_line_cpp_completion(
    completion: &Completion,
    replace_range: &Range<text::Anchor>,
    cursor_position: &text::Anchor,
    buffer_snapshot: &BufferSnapshot,
) -> Option<String> {
    let language = buffer_snapshot.language_at(replace_range.start)?;
    if !is_c_language(language) {
        return None;
    }

    let CompletionSource::Lsp { lsp_completion, .. } = &completion.source else {
        return None;
    };
    if !matches!(
        lsp_completion.kind,
        Some(
            lsp::CompletionItemKind::METHOD
                | lsp::CompletionItemKind::FUNCTION
                | lsp::CompletionItemKind::CONSTRUCTOR
        )
    ) {
        return None;
    }

    let method_name = bare_method_name(&completion.new_text)?;

    let cursor_offset = cursor_position.to_offset(buffer_snapshot);
    let after_end = (cursor_offset + 64).min(buffer_snapshot.len());
    let next_non_ws = buffer_snapshot
        .text_for_range(cursor_offset..after_end)
        .flat_map(|chunk| chunk.chars())
        .take_while(|&ch| ch != '\n')
        .find(|ch| !ch.is_whitespace());
    if next_non_ws == Some('(') {
        return None;
    }

    let replace_start = replace_range.start.to_offset(buffer_snapshot);
    if !is_out_of_line_definition_context(buffer_snapshot, replace_start) {
        return None;
    }

    let (params, qualifiers) = extract_signature_from_lsp_completion(lsp_completion, method_name)?;
    let escaped_name = escape_snippet_text(method_name);
    let escaped_params = escape_snippet_text(&params);
    let escaped_qualifiers = escape_snippet_text(&qualifiers);

    if escaped_qualifiers.is_empty() {
        Some(format!("{escaped_name}({escaped_params}) {{\n    $0\n}}"))
    } else {
        Some(format!(
            "{escaped_name}({escaped_params}) {escaped_qualifiers} {{\n    $0\n}}"
        ))
    }
}

fn bare_method_name(new_text: &str) -> Option<&str> {
    let trimmed = new_text.trim();
    if trimmed.is_empty() || trimmed.contains('{') {
        return None;
    }
    let Some(open_paren) = trimmed.find('(') else {
        return Some(trimmed);
    };
    let name = trimmed[..open_paren].trim();
    let rest = trimmed[open_paren + 1..].trim();
    let inner = rest.strip_suffix(')')?.trim();
    if inner.is_empty() || inner == "$0" || inner == "${0}" || inner == "${1}" {
        (!name.is_empty()).then_some(name)
    } else {
        None
    }
}

fn is_out_of_line_definition_context(
    buffer_snapshot: &BufferSnapshot,
    replace_start: usize,
) -> bool {
    let prefix_start = replace_start.saturating_sub(64);
    let prefix: String = buffer_snapshot
        .text_for_range(prefix_start..replace_start)
        .collect();
    if !prefix.trim_end().ends_with("::") {
        return false;
    }

    let Some(layer) = buffer_snapshot.syntax_layer_at(replace_start) else {
        return true;
    };
    let probe = replace_start.saturating_sub(1);
    let mut node = layer.node().descendant_for_byte_range(probe, probe);
    while let Some(current) = node {
        if matches!(
            current.kind(),
            "compound_statement"
                | "call_expression"
                | "argument_list"
                | "initializer_list"
                | "parameter_list"
                | "field_declaration_list"
                | "class_specifier"
                | "struct_specifier"
        ) {
            return false;
        }
        node = current.parent();
    }
    true
}

fn extract_signature_from_lsp_completion(
    lsp_completion: &lsp::CompletionItem,
    method_name: &str,
) -> Option<(String, String)> {
    let candidates = [
        lsp_completion
            .label_details
            .as_ref()
            .and_then(|details| details.detail.as_deref()),
        Some(lsp_completion.label.as_str()),
        lsp_completion.detail.as_deref(),
    ];

    for candidate in candidates.into_iter().flatten() {
        if let Some(parsed) = parse_signature_candidate(candidate, method_name) {
            return Some(parsed);
        }
    }
    None
}

fn parse_signature_candidate(candidate: &str, method_name: &str) -> Option<(String, String)> {
    let search_start = if let Some(name_pos) = candidate.find(method_name) {
        name_pos + method_name.len()
    } else {
        0
    };
    let tail = &candidate[search_start..];
    let open_rel = tail.find('(')?;
    let after_open = &tail[open_rel + 1..];

    let mut depth = 1usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;
    let mut close_idx = None;

    for (idx, ch) in after_open.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && (in_single_quote || in_double_quote) {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }
        if in_single_quote || in_double_quote {
            continue;
        }
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close_idx = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }

    let close_idx = close_idx?;
    let raw_params = &after_open[..close_idx];
    let raw_trailing = &after_open[close_idx + 1..];

    Some((
        clean_parameter_list(raw_params),
        clean_trailing_qualifiers(raw_trailing),
    ))
}

fn clean_parameter_list(raw_params: &str) -> String {
    let trimmed = raw_params.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut params = Vec::new();
    let mut current_start = 0usize;
    let mut paren_depth = 0usize;
    let mut angle_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for (idx, ch) in trimmed.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && (in_single_quote || in_double_quote) {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }
        if in_single_quote || in_double_quote {
            continue;
        }
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ',' if paren_depth == 0
                && angle_depth == 0
                && brace_depth == 0
                && bracket_depth == 0 =>
            {
                params.push(strip_default_argument(&trimmed[current_start..idx]));
                current_start = idx + 1;
            }
            _ => {}
        }
    }
    params.push(strip_default_argument(&trimmed[current_start..]));

    params
        .into_iter()
        .filter(|param| !param.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn strip_default_argument(param: &str) -> String {
    let mut paren_depth = 0usize;
    let mut angle_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    let bytes = param.as_bytes();
    for (idx, ch) in param.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && (in_single_quote || in_double_quote) {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }
        if in_single_quote || in_double_quote {
            continue;
        }
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '=' if paren_depth == 0
                && angle_depth == 0
                && brace_depth == 0
                && bracket_depth == 0 =>
            {
                let next = bytes.get(idx + 1).copied();
                let prev = idx.checked_sub(1).and_then(|i| bytes.get(i).copied());
                if next != Some(b'=') && prev != Some(b'=') && prev != Some(b'!') {
                    return param[..idx].trim().to_string();
                }
            }
            _ => {}
        }
    }
    param.trim().to_string()
}

fn clean_trailing_qualifiers(raw_trailing: &str) -> String {
    let mut result = Vec::new();
    let mut rest = raw_trailing.trim_start();

    while !rest.is_empty() {
        if let Some(after) = strip_keyword(rest, "const") {
            result.push("const".to_string());
            rest = after.trim_start();
        } else if let Some(after) = strip_keyword(rest, "volatile") {
            result.push("volatile".to_string());
            rest = after.trim_start();
        } else if let Some(after) = rest.strip_prefix("&&") {
            result.push("&&".to_string());
            rest = after.trim_start();
        } else if let Some(after) = rest.strip_prefix('&') {
            result.push("&".to_string());
            rest = after.trim_start();
        } else if let Some(after) = strip_keyword(rest, "noexcept") {
            let after_ws = after.trim_start();
            if after_ws.starts_with('(') {
                if let Some((expr, remaining)) = take_balanced_parens(after_ws) {
                    result.push(format!("noexcept{expr}"));
                    rest = remaining.trim_start();
                    continue;
                }
            }
            result.push("noexcept".to_string());
            rest = after_ws;
        } else if strip_keyword(rest, "override").is_some()
            || strip_keyword(rest, "final").is_some()
            || rest.starts_with('=')
            || rest.starts_with(';')
            || rest.starts_with('{')
        {
            break;
        } else {
            break;
        }
    }

    result.join(" ")
}

fn strip_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    let after = input.strip_prefix(keyword)?;
    if after
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    Some(after)
}

fn take_balanced_parens(input: &str) -> Option<(&str, &str)> {
    if !input.starts_with('(') {
        return None;
    }
    let mut depth = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let end = idx + 1;
                    return Some((&input[..end], &input[end..]));
                }
            }
            _ => {}
        }
    }
    None
}

fn escape_snippet_text(text: &str) -> String {
    text.replace('\\', "\\\\").replace('$', "\\$")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_parameter_list_strips_defaults_and_preserves_templates() {
        assert_eq!(
            clean_parameter_list("int a, float b = 1.0f, std::vector<int> items = {1, 2}"),
            "int a, float b, std::vector<int> items"
        );
        assert_eq!(
            clean_parameter_list("const std::string& name = \"default, val\", int flags = (1 | 2)"),
            "const std::string& name, int flags"
        );
        assert_eq!(clean_parameter_list("  "), "");
    }

    #[test]
    fn parse_signature_candidate_extracts_qualifiers_and_strips_override() {
        let parsed = parse_signature_candidate(
            "void render(VkCommandBuffer cmd, uint32_t frame = 0) const noexcept override",
            "render",
        );
        assert_eq!(
            parsed,
            Some((
                "VkCommandBuffer cmd, uint32_t frame".to_string(),
                "const noexcept".to_string()
            ))
        );

        let pure_virtual = parse_signature_candidate("int size() const = 0", "size");
        assert_eq!(pure_virtual, Some((String::new(), "const".to_string())));

        let operator_call = parse_signature_candidate("void operator()(int x = 1) const", "operator()");
        assert_eq!(
            operator_call,
            Some(("int x".to_string(), "const".to_string()))
        );
    }

    #[test]
    fn bare_method_name_handles_plain_and_empty_snippets() {
        assert_eq!(bare_method_name("render"), Some("render"));
        assert_eq!(bare_method_name("render()"), Some("render"));
        assert_eq!(bare_method_name("render($0)"), Some("render"));
        assert_eq!(bare_method_name("render(int a)"), None);
        assert_eq!(bare_method_name("render() {}"), None);
    }

    #[gpui::test]
    async fn expand_out_of_line_cpp_completion_respects_syntax_context(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext as _;
        use language::{Buffer, CodeLabel, LanguageConfig};
        use std::sync::Arc;

        let cpp_language = Arc::new(Language::new(
            LanguageConfig {
                name: "C++".into(),
                ..LanguageConfig::default()
            },
            Some(tree_sitter_c::LANGUAGE.into()),
        ));

        let make_completion = |replace_range: Range<text::Anchor>| Completion {
            replace_range,
            new_text: "render".to_string(),
            label: CodeLabel::plain("render".to_string(), None),
            documentation: None,
            source: CompletionSource::Lsp {
                insert_range: None,
                server_id: lsp::LanguageServerId(0),
                lsp_completion: Box::new(lsp::CompletionItem {
                    label: "render".to_string(),
                    kind: Some(lsp::CompletionItemKind::METHOD),
                    detail: Some(
                        "void render(VkCommandBuffer cmd, uint32_t frame = 0) const override"
                            .to_string(),
                    ),
                    ..Default::default()
                }),
                lsp_defaults: None,
                resolved: true,
            },
            icon_path: None,
            icon_color: None,
            insert_text_mode: None,
            confirm: None,
            match_start: None,
            snippet_deduplication_key: None,
            group: None,
        };

        // 1. Top-level out-of-line definition expands parameters, qualifiers, and body.
        let text = "void Renderer::ren";
        let buffer = cx.new(|cx| Buffer::local(text, cx).with_language(cpp_language.clone(), cx));
        cx.run_until_parked();
        buffer.read_with(cx, |buffer, _| {
            let snapshot = buffer.snapshot();
            let start = snapshot.anchor_before(text.len() - 3);
            let cursor = snapshot.anchor_before(text.len());
            let completion = make_completion(start..cursor);
            assert_eq!(
                expand_out_of_line_cpp_completion(
                    &completion,
                    &(start..cursor),
                    &cursor,
                    &snapshot
                ),
                Some("render(VkCommandBuffer cmd, uint32_t frame) const {\n    $0\n}".to_string())
            );
        });

        // 2. Inside a function body (`compound_statement`), do NOT expand `{}`.
        let inside_fn = "void main() {\n    Renderer::ren\n}";
        let buffer =
            cx.new(|cx| Buffer::local(inside_fn, cx).with_language(cpp_language.clone(), cx));
        cx.run_until_parked();
        buffer.read_with(cx, |buffer, _| {
            let snapshot = buffer.snapshot();
            let ren_end = inside_fn.find("\n}").unwrap();
            let start = snapshot.anchor_before(ren_end - 3);
            let cursor = snapshot.anchor_before(ren_end);
            let completion = make_completion(start..cursor);
            assert_eq!(
                expand_out_of_line_cpp_completion(
                    &completion,
                    &(start..cursor),
                    &cursor,
                    &snapshot
                ),
                None
            );
        });

        // 3. If `(` already follows the cursor, do NOT duplicate `(`.
        let with_parens = "void Renderer::ren(int x)";
        let buffer =
            cx.new(|cx| Buffer::local(with_parens, cx).with_language(cpp_language.clone(), cx));
        cx.run_until_parked();
        buffer.read_with(cx, |buffer, _| {
            let snapshot = buffer.snapshot();
            let paren_pos = with_parens.find('(').unwrap();
            let start = snapshot.anchor_before(paren_pos - 3);
            let cursor = snapshot.anchor_before(paren_pos);
            let completion = make_completion(start..cursor);
            assert_eq!(
                expand_out_of_line_cpp_completion(
                    &completion,
                    &(start..cursor),
                    &cursor,
                    &snapshot
                ),
                None
            );
        });

        // 4. Without `::` prefix, do NOT expand.
        let no_scope = "void ren";
        let buffer = cx.new(|cx| Buffer::local(no_scope, cx).with_language(cpp_language, cx));
        cx.run_until_parked();
        buffer.read_with(cx, |buffer, _| {
            let snapshot = buffer.snapshot();
            let start = snapshot.anchor_before(no_scope.len() - 3);
            let cursor = snapshot.anchor_before(no_scope.len());
            let completion = make_completion(start..cursor);
            assert_eq!(
                expand_out_of_line_cpp_completion(
                    &completion,
                    &(start..cursor),
                    &cursor,
                    &snapshot
                ),
                None
            );
        });
    }
}
