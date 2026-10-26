//! Finding in the file on screen: limn's FindBar over the editor, its matches marked, and the
//! caret taken from one to the next.

use limn::{EditorController, FindOptions};
use limn_editor::{SearchOptions, SearchResults, Selection};

/// The bar's id; limn derives its controls' from it (`find:query`, `find:next`…).
pub const FIND: &str = "find";
pub const FIND_OPEN: &str = "find:open";
pub const FIND_QUERY: &str = "find:query";

#[derive(Default)]
pub struct Find {
    pub query: String,
    pub options: FindOptions,
    pub results: Option<SearchResults>,
    /// The match the caret is on.
    pub current: Option<usize>,
    pub error: Option<String>,
}

impl Find {
    /// Search `editor` again, the current match the first at or after its caret.
    pub fn search(&mut self, editor: &EditorController) {
        self.error = None;
        self.results = None;
        self.current = None;
        if self.query.is_empty() {
            return;
        }
        let options = SearchOptions {
            case_sensitive: self.options.case,
            whole_word: self.options.word,
            regex: self.options.regex,
            scope: None,
        };
        match editor.search(&self.query, &options) {
            Ok(results) => {
                let caret = editor
                    .snapshot()
                    .map(|s| s.selection().range().start)
                    .unwrap_or(0);
                let matches = &results.matches;
                self.current = matches
                    .iter()
                    .position(|m| m.start >= caret)
                    .or((!matches.is_empty()).then_some(0));
                self.results = Some(results);
            }
            Err(error) => self.error = Some(format!("{error:?}")),
        }
    }

    /// Go `by` matches on, round the ends, and select it.
    pub fn step(&mut self, editor: &EditorController, by: isize) {
        let count = self.results.as_ref().map_or(0, |r| r.matches.len()) as isize;
        if count == 0 {
            return;
        }
        let at = self
            .current
            .map_or(0, |c| (c as isize + by).rem_euclid(count));
        self.current = Some(at as usize);
        self.select(editor);
    }

    /// Select the current match in the editor.
    pub fn select(&self, editor: &EditorController) {
        let (Some(results), Some(at)) = (&self.results, self.current) else {
            return;
        };
        let found = results.matches[at].clone();
        let shared = editor.shared_buffer();
        let Ok(mut buffer) = shared.lock() else {
            return;
        };
        let _ = buffer.set_selection(Selection {
            anchor: found.start,
            head: found.end,
        });
    }

    /// "3 of 12", or nothing before a query.
    pub fn count_label(&self) -> String {
        match (&self.results, self.current) {
            (Some(results), Some(at)) => {
                let more = if results.truncated { "+" } else { "" };
                format!("{} of {}{more}", at + 1, results.matches.len())
            }
            (Some(_), None) => "No results".into(),
            _ => String::new(),
        }
    }
}
