//! Finding text in every file of the folder, as ripgrep would: the files git sees, line by
//! line, with case told apart only when the query has a capital in it.

use std::path::{Path, PathBuf};

pub const GREP_OPEN: &str = "grep:open";
pub const GREP_FIELD: &str = "grep:field";
pub const GREP_CLOSE: &str = "grep:close";
/// The most it finds; and the largest file it reads.
pub const LIMIT: usize = 200;
const LARGEST: u64 = 2 * 1024 * 1024;

/// A line that holds the query: its file from the folder, its row from 0, and its words.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub path: PathBuf,
    pub row: usize,
    pub text: String,
}

/// The lines of `files` under `root` that hold `query`, at most `LIMIT`. Files that are
/// not text, or larger than 2 MB, are passed over.
pub fn grep(root: &Path, files: &[PathBuf], query: &str) -> Vec<Hit> {
    let exact = query.chars().any(char::is_uppercase);
    let wanted = if exact {
        query.to_owned()
    } else {
        query.to_lowercase()
    };
    let mut hits = vec![];
    for path in files {
        let full = root.join(path);
        if std::fs::metadata(&full).map_or(true, |m| m.len() > LARGEST) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&full) else {
            continue;
        };
        for (row, line) in text.lines().enumerate() {
            let holds = if exact {
                line.contains(&wanted)
            } else {
                line.to_lowercase().contains(&wanted)
            };
            if holds {
                let text = line.trim().chars().take(160).collect();
                hits.push(Hit {
                    path: path.clone(),
                    row,
                    text,
                });
                if hits.len() == LIMIT {
                    return hits;
                }
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_found_in_every_file_and_a_capital_tells_case_apart() {
        let root = std::env::temp_dir().join(format!("quill-ide-grep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn main() {}\n    Main();\n").unwrap();
        std::fs::write(root.join("b.bin"), [0xff, 0xfe, 0x00]).unwrap();
        let files: Vec<PathBuf> = ["src/a.rs", "b.bin"].map(PathBuf::from).into();
        let any = grep(&root, &files, "main");
        let exact = grep(&root, &files, "Main");
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(any.len(), 2);
        assert_eq!(
            exact,
            vec![Hit {
                path: "src/a.rs".into(),
                row: 1,
                text: "Main();".into()
            }]
        );
    }
}
