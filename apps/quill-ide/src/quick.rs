//! Opening a file by its name, as nvim's pickers do: what is typed matches a file's path when
//! its letters appear in it in order.

use std::path::PathBuf;

pub const QUICK_OPEN: &str = "quick:open";
pub const QUICK_FIELD: &str = "quick:field";
pub const QUICK_CLOSE: &str = "quick:close";
/// The most it lists.
pub const SHOWN: usize = 50;

/// How well `query` matches `path`: all its letters in order, case aside, scored higher when
/// they run together, begin a word or fall in the file's name, and for shorter paths. `None`
/// when they do not all appear.
pub fn score(query: &str, path: &str) -> Option<i32> {
    let name_at = path.rfind('/').map_or(0, |at| at + 1);
    let path: Vec<char> = path.chars().collect();
    let mut score = 0;
    let mut from = 0;
    let mut last: Option<usize> = None;
    for wanted in query.chars().flat_map(char::to_lowercase) {
        let at = (from..path.len()).find(|&i| path[i].to_lowercase().eq([wanted]))?;
        score += 1;
        if last == Some(at.wrapping_sub(1)) {
            score += 5;
        }
        if at == 0 || matches!(path[at - 1], '/' | '_' | '-' | '.' | ' ') {
            score += 8;
        }
        if at >= name_at {
            score += 3;
        }
        last = Some(at);
        from = at + 1;
    }
    Some(score * 10 - path.len() as i32)
}

/// The files that match `query` best, at most `SHOWN`; with no query, the first ones.
pub fn best<'a>(query: &str, files: &'a [PathBuf]) -> Vec<&'a PathBuf> {
    if query.is_empty() {
        return files.iter().take(SHOWN).collect();
    }
    let mut scored: Vec<(i32, &PathBuf)> = files
        .iter()
        .filter_map(|f| Some((score(query, &f.to_string_lossy())?, f)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(SHOWN).map(|(_, f)| f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_in_order_match_and_names_and_word_starts_win() {
        assert_eq!(score("xyz", "src/main.rs"), None);
        assert!(score("mrs", "src/main.rs").is_some());
        let files: Vec<PathBuf> = [
            "docs/engine.md",
            "crates/quill-engine/src/lib.rs",
            "apps/quill-ide/src/ide.rs",
            "crates/quill-ui/src/view.rs",
        ]
        .map(PathBuf::from)
        .into();
        assert_eq!(
            best("ide", &files)[0],
            &PathBuf::from("apps/quill-ide/src/ide.rs")
        );
        assert_eq!(
            best("view", &files)[0],
            &PathBuf::from("crates/quill-ui/src/view.rs")
        );
        assert_eq!(best("engmd", &files)[0], &PathBuf::from("docs/engine.md"));
        assert_eq!(best("", &files).len(), 4);
    }
}
