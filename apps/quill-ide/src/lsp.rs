//! Language servers, through limn-language's client: one per language, started the first time
//! a file of it opens, given each open buffer's text at each revision, asked what the person
//! asks, and its answers put where the IDE shows them.

use crate::ide::{Ask, AskKind, Diagnostic, Ide, Job, Location};
use limn_editor::{BufferSnapshot, EditKind, Selection};
use limn_language::lsp::{self, Event, LanguageServer, Request, RequestId, ServerConfig};
use limn_language::{offset_to_utf16, utf16_to_offset, Language};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The servers of the window, the documents each holds, and what was asked of them.
#[derive(Default)]
pub struct Lsp {
    servers: HashMap<Language, LanguageServer>,
    /// Each server's program's name, for what is said of it.
    names: HashMap<Language, String>,
    /// Languages a server was looked for, found or not, so none is looked for each frame.
    tried: HashSet<Language>,
    open: HashMap<PathBuf, Language>,
    pending: HashMap<RequestId, (PathBuf, AskKind)>,
}

impl Lsp {
    /// One frame's work: servers started, documents opened, changed and closed, the IDE's
    /// asks sent and the servers' news taken. Returns the jobs the answers need.
    pub fn sync(&mut self, ide: &mut Ide, wake: &Arc<dyn Fn() + Send + Sync>) -> Vec<Job> {
        let mut shown: HashSet<PathBuf> = HashSet::new();
        for buffer in ide.buffers.iter().filter(|b| !b.loading) {
            let Some(snapshot) = buffer.editor.as_ref().and_then(|e| e.snapshot().ok()) else {
                continue;
            };
            let language = Language::from_path(&buffer.path.to_string_lossy());
            if language == Language::Plain || language == Language::Markdown {
                continue;
            }
            shown.insert(buffer.path.clone());
            self.start(language, &ide.root, wake);
            let Some(server) = self.servers.get(&language).filter(|s| s.is_ready()) else {
                continue;
            };
            let sent = if self.open.contains_key(&buffer.path) {
                server.change(&buffer.path, &snapshot).map(drop)
            } else {
                server.open(&buffer.path, &snapshot, language).map(|_| {
                    self.open.insert(buffer.path.clone(), language);
                })
            };
            if let Err(error) = sent {
                ide.lsp_note = Some(error.message);
            }
        }
        // A file closed, or renamed away, is closed in its server.
        let gone: Vec<PathBuf> = self
            .open
            .keys()
            .filter(|p| !shown.contains(*p))
            .cloned()
            .collect();
        for path in gone {
            if let Some(server) = self.open.remove(&path).and_then(|l| self.servers.get(&l)) {
                let _ = server.close(&path);
            }
        }
        for ask in std::mem::take(&mut ide.lsp_asks) {
            self.ask(ide, ask);
        }
        let mut jobs = vec![];
        let events: Vec<(Language, Event)> = self
            .servers
            .iter()
            .flat_map(|(language, server)| drain(server).into_iter().map(|e| (*language, e)))
            .collect();
        for (language, event) in events {
            if let Event::Stopped { reason } = &event {
                let program = self
                    .names
                    .get(&language)
                    .map_or("The language server", |n| n.as_str());
                ide.lsp_note = Some(format!("{program} stopped: {reason}"));
                continue;
            }
            jobs.extend(self.heard(ide, event));
        }
        jobs
    }

    /// Start the server for `language` once, if one is on this computer.
    fn start(&mut self, language: Language, root: &Path, wake: &Arc<dyn Fn() + Send + Sync>) {
        if self.tried.insert(language) {
            let candidates = lsp::server_candidates(language);
            let last = candidates.len().saturating_sub(1);
            for (at, executable) in candidates.into_iter().enumerate() {
                // A program that does not even say its version (rustup's proxy without the
                // component) gives way to the next one on the PATH.
                if at < last && lsp::probe(&executable, &["--version"]).is_err() {
                    continue;
                }
                let program = executable
                    .file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                let mut config = ServerConfig::new(&executable, root);
                config.args = lsp::stdio_args(&executable);
                if let Ok(server) = LanguageServer::spawn(config) {
                    server.set_waker(wake.clone());
                    self.servers.insert(language, server);
                    self.names.insert(language, program);
                    break;
                }
            }
        }
    }

    fn ask(&mut self, ide: &mut Ide, ask: Ask) {
        let language = Language::from_path(&ask.path.to_string_lossy());
        let snapshot = ide
            .buffers
            .iter()
            .find(|b| b.path == ask.path)
            .and_then(|b| b.editor.as_ref()?.snapshot().ok());
        let (Some(server), Some(snapshot)) = (self.servers.get(&language), snapshot) else {
            ide.lsp_note = Some(format!(
                "No language server for {} files here",
                name(language)
            ));
            return;
        };
        let Some(position) = offset_to_utf16(&snapshot, ask.offset) else {
            return;
        };
        let request = match &ask.kind {
            AskKind::Completion => Request::Completion(position),
            AskKind::Hover => Request::Hover(position),
            AskKind::Definition => Request::Definition(position),
            AskKind::References => Request::References {
                position,
                include_declaration: true,
            },
            AskKind::Rename(new_name) => Request::Rename {
                position,
                new_name: new_name.clone(),
            },
            AskKind::Format => Request::Formatting {
                tab_size: ide.prefs.tab_width,
                insert_spaces: ide.prefs.spaces,
            },
        };
        match server.request(&ask.path, &snapshot, request) {
            Ok(id) => {
                self.pending.insert(id, (ask.path, ask.kind));
            }
            Err(error) => ide.lsp_note = Some(error.message),
        }
    }

    fn heard(&mut self, ide: &mut Ide, event: Event) -> Vec<Job> {
        match event {
            Event::Initialized { server_info, .. } => {
                ide.lsp_note = server_info["name"].as_str().map(str::to_owned);
            }
            Event::Diagnostics { document, items } => {
                let Ok(path) = lsp::uri_path(&document.uri) else {
                    return vec![];
                };
                let Some(snapshot) = snapshot_at(ide, &path, document.revision) else {
                    return vec![];
                };
                let found = items
                    .iter()
                    .filter_map(|item| {
                        let start = utf16_to_offset(&snapshot, item.range.start)?;
                        let end = utf16_to_offset(&snapshot, item.range.end)?;
                        Some(Diagnostic {
                            range: start..end.max(start),
                            severity: item.severity.unwrap_or(1),
                            message: item.message.clone(),
                        })
                    })
                    .collect();
                let text: String = snapshot.chunks().collect();
                let diagnostics = crate::ide::Diagnostics {
                    revision: document.revision,
                    text: text.into(),
                    items: found,
                    carried: None,
                };
                ide.diagnostics.insert(path, diagnostics);
            }
            Event::UnversionedDiagnostics { .. } => {}
            Event::Response {
                id,
                document,
                result,
                ..
            } => {
                let Some((path, kind)) = self.pending.remove(&id) else {
                    return vec![];
                };
                let value = match result {
                    Ok(value) => value,
                    Err(error) => {
                        ide.lsp_note = Some(error.message);
                        return vec![];
                    }
                };
                return answered(ide, &path, document.revision, kind, &value);
            }
            Event::Stopped { reason } => ide.lsp_note = Some(reason),
        }
        vec![]
    }
}

fn drain(server: &LanguageServer) -> Vec<Event> {
    std::iter::from_fn(|| server.try_recv()).collect()
}

fn name(language: Language) -> &'static str {
    match language {
        Language::Rust => "Rust",
        Language::JavaScript => "JavaScript",
        Language::TypeScript | Language::Tsx => "TypeScript",
        Language::Python => "Python",
        Language::Json => "JSON",
        Language::Toml => "TOML",
        Language::Css => "CSS",
        Language::Html => "HTML",
        _ => "these",
    }
}

/// The open buffer of `path` as it is, if it is at `revision`.
fn snapshot_at(ide: &Ide, path: &Path, revision: u64) -> Option<BufferSnapshot> {
    let buffer = ide.buffers.iter().find(|b| b.path == path)?;
    let snapshot = buffer.editor.as_ref()?.snapshot().ok()?;
    (snapshot.revision() == revision).then_some(snapshot)
}

/// A server's answer, where the IDE shows it.
fn answered(ide: &mut Ide, path: &Path, revision: u64, kind: AskKind, value: &Value) -> Vec<Job> {
    match kind {
        AskKind::Completion => {
            let offers = crate::complete::offers(value);
            match ide.completion.as_mut().filter(|c| c.path == path) {
                Some(completion) if !offers.is_empty() => completion.offers = offers,
                _ => ide.completion = None,
            }
        }
        AskKind::Hover => {
            ide.hover = Some(hover_text(value).unwrap_or_else(|| "Nothing to say here.".into()));
        }
        AskKind::Definition => match locations(value).into_iter().next() {
            Some(found) => return ide.goto_at(found.path, found.row, found.column),
            None => ide.lsp_note = Some("No definition found".into()),
        },
        AskKind::References => {
            let found = locations(value);
            if found.is_empty() {
                ide.lsp_note = Some("No references found".into());
            } else {
                ide.references = Some((found, 0));
            }
        }
        AskKind::Format => {
            if let Some(snapshot) = snapshot_at(ide, path, revision) {
                apply(ide, path, &snapshot, value);
            }
        }
        AskKind::Rename(_) => {
            let mut jobs = vec![];
            for (file, edits) in workspace_edits(value) {
                let open = ide
                    .buffers
                    .iter()
                    .find(|b| b.path == file)
                    .and_then(|b| b.editor.as_ref()?.snapshot().ok());
                match open {
                    Some(snapshot) => apply(ide, &file, &snapshot, &edits),
                    // A file not open is changed on the disk, whole or not at all.
                    None => jobs.push(Job::ApplyEdits(file, edits.to_string())),
                }
            }
            return jobs;
        }
    }
    vec![]
}

/// Server edits put in `path`'s buffer as one undoable step, the caret kept in range.
fn apply(ide: &mut Ide, path: &Path, snapshot: &BufferSnapshot, edits: &Value) {
    let Some(editor) = ide
        .buffers
        .iter()
        .find(|b| b.path == path)
        .and_then(|b| b.editor.clone())
    else {
        return;
    };
    let edits = match lsp::text_edits(snapshot, edits) {
        Ok(edits) => edits,
        Err(error) => {
            ide.lsp_note = Some(error.message);
            return;
        }
    };
    let shared = editor.shared_buffer();
    let Ok(mut buffer) = shared.lock() else {
        return;
    };
    let grown: isize = edits
        .iter()
        .map(|e| e.replacement.chars().count() as isize - e.range.len() as isize)
        .sum();
    let after = (snapshot.char_count() as isize + grown).max(0) as usize;
    let caret = Selection::caret(snapshot.selection().head.min(after));
    if let Err(error) = buffer.apply_transaction(&edits, caret, EditKind::Replace, 0) {
        ide.lsp_note = Some(format!("{error:?}"));
    }
}

/// The words of a hover answer: markup, a marked string, or a list of them.
pub fn hover_text(value: &Value) -> Option<String> {
    let piece = |part: &Value| -> Option<String> {
        match part {
            Value::String(text) => Some(text.clone()),
            Value::Object(_) => {
                let text = part["value"].as_str()?;
                Some(match part["language"].as_str() {
                    Some(language) => format!("```{language}\n{text}\n```"),
                    None => text.to_owned(),
                })
            }
            _ => None,
        }
    };
    let contents = &value["contents"];
    let text = match contents {
        Value::Array(parts) => parts
            .iter()
            .filter_map(piece)
            .collect::<Vec<_>>()
            .join("\n\n"),
        other => piece(other)?,
    };
    (!text.trim().is_empty()).then_some(text)
}

/// The places an answer names: one Location, a list of them, or LocationLinks.
pub fn locations(value: &Value) -> Vec<Location> {
    let one = |item: &Value| -> Option<Location> {
        let uri = item["uri"].as_str().or(item["targetUri"].as_str())?;
        let range = if item["range"].is_object() {
            &item["range"]
        } else {
            &item["targetSelectionRange"]
        };
        Some(Location {
            path: lsp::uri_path(uri).ok()?,
            row: range["start"]["line"].as_u64()? as usize,
            column: range["start"]["character"].as_u64()? as u32,
        })
    };
    match value {
        Value::Array(items) => items.iter().filter_map(one).collect(),
        Value::Object(_) => one(value).into_iter().collect(),
        _ => vec![],
    }
}

/// A WorkspaceEdit's text edits by file: from `changes`, or `documentChanges`' text ones.
pub fn workspace_edits(value: &Value) -> Vec<(PathBuf, Value)> {
    let mut found = vec![];
    if let Some(changes) = value["changes"].as_object() {
        for (uri, edits) in changes {
            if let Ok(path) = lsp::uri_path(uri) {
                found.push((path, edits.clone()));
            }
        }
    }
    for change in value["documentChanges"].as_array().into_iter().flatten() {
        let (Some(uri), Some(edits)) =
            (change["textDocument"]["uri"].as_str(), change.get("edits"))
        else {
            continue;
        };
        if let Ok(path) = lsp::uri_path(uri) {
            found.push((path, edits.clone()));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn answers_read_as_the_protocol_allows_them() {
        let markup = json!({"contents": {"kind": "markdown", "value": "fn go()"}});
        assert_eq!(hover_text(&markup).as_deref(), Some("fn go()"));
        let marked = json!({"contents": [{"language": "rust", "value": "fn go()"}, "Goes."]});
        assert_eq!(
            hover_text(&marked).as_deref(),
            Some("```rust\nfn go()\n```\n\nGoes.")
        );
        assert_eq!(hover_text(&json!({"contents": ""})), None);
        let range =
            json!({"start": {"line": 4, "character": 2}, "end": {"line": 4, "character": 5}});
        let one = json!({"uri": "file:///r/a.rs", "range": range});
        let link = json!([{"targetUri": "file:///r/b.rs", "targetRange": range, "targetSelectionRange": range}]);
        assert_eq!(
            locations(&one),
            vec![Location {
                path: "/r/a.rs".into(),
                row: 4,
                column: 2
            }]
        );
        assert_eq!(locations(&link)[0].path, PathBuf::from("/r/b.rs"));
        assert!(locations(&Value::Null).is_empty());
        let edit = json!([{"range": range, "newText": "x"}]);
        let both = json!({
            "changes": {"file:///r/a.rs": edit},
            "documentChanges": [{"textDocument": {"uri": "file:///r/b.rs", "version": 1}, "edits": edit}]
        });
        let files: Vec<PathBuf> = workspace_edits(&both).into_iter().map(|(p, _)| p).collect();
        assert_eq!(files, [PathBuf::from("/r/a.rs"), PathBuf::from("/r/b.rs")]);
    }
}
