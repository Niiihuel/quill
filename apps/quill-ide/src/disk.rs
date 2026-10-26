//! The `Job`s, on the disk: run on a thread of their own, never the window's.

use crate::ide::{Entry, Job, Loaded};
use std::path::Path;

pub fn run(job: Job) -> Loaded {
    match job {
        Job::List(dir) => {
            let entries = list(&dir);
            Loaded::Listed(dir, entries)
        }
        Job::Read(path) => {
            let text = std::fs::read(&path)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok());
            Loaded::Read(path, text)
        }
    }
}

/// `dir`'s entries as git sees them: dotfiles shown, .gitignore'd ones and .git left out,
/// folders first, then by name.
fn list(dir: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(false)
        // A folder that is not a repository still keeps its .gitignore.
        .require_git(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.depth() == 1)
        .map(|e| Entry {
            dir: e.file_type().is_some_and(|t| t.is_dir()),
            path: e.into_path(),
        })
        .collect();
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.path.cmp(&b.path)));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_lists_as_git_sees_it() {
        let dir = std::env::temp_dir().join(format!("quill-ide-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["src", ".git", "target"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for (f, body) in [(".gitignore", "target\n"), ("b.rs", ""), ("a.rs", "")] {
            std::fs::write(dir.join(f), body).unwrap();
        }
        let names: Vec<String> = list(&dir)
            .iter()
            .map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["src", ".gitignore", "a.rs", "b.rs"]);
    }
}
