//! A shadow git repository per folder, outside the folder (spec.md D4): a snapshot of the
//! work tree before and after each request tells what that request created, changed,
//! removed or moved, whoever did it, and puts files back for a rewind. The project's own
//! `.git` is never touched. Built on the `git` binary (spec.md D9); calls block.
//!
//! shortcut: every snapshot runs `git add -A` over the whole folder; for very large
//! folders switch to snapshots per tool from the agent's hooks (engine.md §6).

use quill_model::{ChangeKind, FileEffect};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A patch longer than this is cut, so one generated file cannot flood the transcript.
const PATCH_LIMIT: usize = 200_000;

pub struct Shadow {
    git_dir: PathBuf,
    work: PathBuf,
}

impl Shadow {
    /// The shadow of `work`, kept under `root` (created on first use).
    pub fn open(root: &Path, work: &Path) -> io::Result<Self> {
        let work = work.canonicalize()?;
        let name: String = work
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let git_dir = root.join(format!("{}.git", name.trim_matches('-')));
        let shadow = Self { git_dir, work };
        if !shadow.git_dir.join("HEAD").exists() {
            std::fs::create_dir_all(&shadow.git_dir)?;
            // `init --bare` takes no work tree; the shadow is told its folder on each command.
            let status = Command::new("git")
                .args(["init", "-q", "--bare"])
                .arg(&shadow.git_dir)
                .status()?;
            if !status.success() {
                return Err(io::Error::other(
                    "git init failed for the shadow repository",
                ));
            }
        }
        Ok(shadow)
    }

    fn git(&self, args: &[&str]) -> io::Result<String> {
        let output = Command::new("git")
            .env("GIT_DIR", &self.git_dir)
            .env("GIT_WORK_TREE", &self.work)
            .env("GIT_INDEX_FILE", self.git_dir.join("index"))
            .args(["-c", "core.quotePath=false", "-c", "core.autocrlf=false"])
            .args(args)
            .current_dir(&self.work)
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// The folder as it is now, as a tree id.
    pub fn snapshot(&self) -> io::Result<String> {
        self.git(&["add", "-A", "--", "."])?;
        Ok(self.git(&["write-tree"])?.trim().to_owned())
    }

    /// What changed from tree `from` to tree `to`, file by file, with each file's patch.
    pub fn changes(&self, from: &str, to: &str) -> io::Result<Vec<FileEffect>> {
        let listing = self.git(&["diff-tree", "-r", "-M", "--name-status", from, to])?;
        let mut files = vec![];
        for line in listing.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            let (kind, paths) = match parts.as_slice() {
                [status, path] if status.starts_with('A') => (ChangeKind::Added, vec![*path]),
                [status, path] if status.starts_with('D') => (ChangeKind::Deleted, vec![*path]),
                [status, from_path, path] if status.starts_with('R') => (
                    ChangeKind::Renamed {
                        from: from_path.to_string(),
                    },
                    vec![*from_path, *path],
                ),
                [_, path] => (ChangeKind::Modified, vec![*path]),
                _ => continue,
            };
            let mut args = vec!["diff", "-M", "--no-color", from, to, "--"];
            args.extend(paths.iter().copied());
            let mut patch = self.git(&args)?;
            let (added, removed) = counts(&patch);
            if patch.len() > PATCH_LIMIT {
                let cut = patch
                    .char_indices()
                    .nth(PATCH_LIMIT)
                    .map_or(patch.len(), |(i, _)| i);
                patch.truncate(cut);
                patch.push_str("\n… (cut)\n");
            }
            files.push(FileEffect {
                path: paths.last().unwrap().to_string(),
                kind,
                added,
                removed,
                patch,
            });
        }
        Ok(files)
    }

    /// Put the folder's files back as they were in tree `tree`: changed files return, files
    /// created since are removed. Ignored files are left alone.
    pub fn restore(&self, tree: &str) -> io::Result<()> {
        self.snapshot()?;
        self.git(&["read-tree", "-u", "--reset", tree])?;
        Ok(())
    }
}

/// Lines added and removed in a unified diff.
fn counts(patch: &str) -> (u32, u32) {
    patch.lines().fold((0, 0), |(a, r), line| {
        if line.starts_with("+++") || line.starts_with("---") {
            (a, r)
        } else if line.starts_with('+') {
            (a + 1, r)
        } else if line.starts_with('-') {
            (a, r + 1)
        } else {
            (a, r)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn folder(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("quill-git-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (root, work) = (base.join("shadow"), base.join("work"));
        fs::create_dir_all(&work).unwrap();
        (root, work)
    }

    #[test]
    fn a_request_made_with_a_shell_is_attributed_and_rewound() {
        let (root, work) = folder("attribute");
        fs::write(work.join("keep.txt"), "one\n").unwrap();
        fs::write(work.join("notes.txt"), "hello\n").unwrap();
        fs::write(work.join("old.txt"), "same\n").unwrap();
        let shadow = Shadow::open(&root, &work).unwrap();
        let before = shadow.snapshot().unwrap();

        // What an agent might do with Edit and Bash: change, create, remove, move.
        fs::write(work.join("keep.txt"), "one\ntwo\n").unwrap();
        fs::write(work.join("new.txt"), "fresh\n").unwrap();
        fs::remove_file(work.join("notes.txt")).unwrap();
        fs::rename(work.join("old.txt"), work.join("moved.txt")).unwrap();
        let after = shadow.snapshot().unwrap();

        let files = shadow.changes(&before, &after).unwrap();
        let kind = |p: &str| files.iter().find(|f| f.path == p).map(|f| f.kind.clone());
        assert_eq!(kind("keep.txt"), Some(ChangeKind::Modified));
        assert_eq!(kind("new.txt"), Some(ChangeKind::Added));
        assert_eq!(kind("notes.txt"), Some(ChangeKind::Deleted));
        assert_eq!(
            kind("moved.txt"),
            Some(ChangeKind::Renamed {
                from: "old.txt".into()
            })
        );
        let keep = files.iter().find(|f| f.path == "keep.txt").unwrap();
        assert_eq!((keep.added, keep.removed), (1, 0));
        assert!(keep.patch.contains("+two"));

        shadow.restore(&before).unwrap();
        assert_eq!(fs::read_to_string(work.join("keep.txt")).unwrap(), "one\n");
        assert_eq!(
            fs::read_to_string(work.join("notes.txt")).unwrap(),
            "hello\n"
        );
        assert!(
            work.join("old.txt").exists()
                && !work.join("moved.txt").exists()
                && !work.join("new.txt").exists()
        );
    }

    #[test]
    fn the_projects_own_git_and_ignored_files_are_left_alone() {
        let (root, work) = folder("ignore");
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&work)
            .status()
            .unwrap();
        fs::write(work.join(".gitignore"), "secret.env\n").unwrap();
        fs::write(work.join("secret.env"), "TOKEN=1\n").unwrap();
        let shadow = Shadow::open(&root, &work).unwrap();
        let before = shadow.snapshot().unwrap();
        fs::write(work.join("secret.env"), "TOKEN=2\n").unwrap();
        let after = shadow.snapshot().unwrap();
        assert!(shadow.changes(&before, &after).unwrap().is_empty());
        assert!(work.join(".git/HEAD").exists());
    }
}
