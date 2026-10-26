//! The `Job`s, on the disk: run on a thread of their own, never the window's.

use crate::ide::{Entry, Job, Loaded, Stamp};
use limn_editor::BufferSnapshot;
use std::io::{self, Write};
use std::path::Path;

pub fn run(job: Job) -> Loaded {
    match job {
        // Stamped before they are read, so a change while reading shows at the next look.
        Job::List(dir) => {
            let stamp = stamp(&dir);
            Loaded::Listed(dir.clone(), list(&dir), stamp)
        }
        Job::Read(path) => {
            let stamp = stamp(&path);
            let text = std::fs::read(&path)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok());
            Loaded::Read(path, text, stamp)
        }
        Job::Write(path, save) => {
            let written = write(&path, &save.0).map_err(|e| e.to_string());
            Loaded::Written(path, save, written)
        }
        Job::SavePrefs(prefs) => {
            // shortcut: a failed save of Settings is only said on stderr; worth a line in
            // Settings if it ever happens.
            if let Err(error) = save_prefs(&prefs) {
                eprintln!("quill-ide: Settings not saved: {error}");
            }
            Loaded::PrefsSaved
        }
        Job::FindPacks(roots) => Loaded::Packs(crate::packs::find(&roots)),
        Job::LoadPack(json) => {
            let theme = crate::packs::load(&json);
            Loaded::Pack(json, theme)
        }
        Job::Stat(watched) => Loaded::Changed(
            watched
                .into_iter()
                .filter_map(|(path, known)| {
                    let now = stamp(&path);
                    (now != known).then_some((path, now))
                })
                .collect(),
        ),
    }
}

/// Settings whole or not at all, as a file is saved.
fn save_prefs(prefs: &crate::prefs::Prefs) -> io::Result<()> {
    let path = crate::prefs::path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension(format!("json.{}", std::process::id()));
    let json = serde_json::to_vec_pretty(prefs).map_err(io::Error::other)?;
    std::fs::write(&temporary, json)?;
    std::fs::rename(&temporary, &path)
}

fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Write `snapshot` over `path` whole or not at all: to a file beside it, then renamed over
/// it with its permissions. A link is followed to the file it names, and stays a link.
fn write(path: &Path, snapshot: &BufferSnapshot) -> io::Result<Stamp> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(io::Error::other("not a file's path"));
    };
    let temporary = dir.join(format!(
        ".{}.quill-ide-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let written = (|| {
        let mut out = io::BufWriter::new(std::fs::File::create(&temporary)?);
        snapshot.write_to(&mut out)?;
        out.flush()?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&temporary, meta.permissions())?;
        }
        std::fs::rename(&temporary, &target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written.map(|()| stamp(&target))
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
    use crate::ide::Save;

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

    #[test]
    fn a_save_through_a_link_replaces_the_file_whole_and_keeps_its_mode_and_the_link() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("quill-ide-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (file, link) = (dir.join("run.sh"), dir.join("link.sh"));
        std::fs::write(&file, "old").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        symlink(&file, &link).unwrap();
        let known = stamp(&file);
        let snapshot = limn_editor::TextBuffer::new("new").unwrap().snapshot();
        let Loaded::Written(_, _, Ok(written)) = run(Job::Write(link.clone(), Save(snapshot)))
        else {
            panic!("the save failed");
        };
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        let is_link = std::fs::symlink_metadata(&link).unwrap().is_symlink();
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        let Loaded::Changed(changed) = run(Job::Stat(vec![(file.clone(), known)])) else {
            panic!("a stat finds what changed");
        };
        let content = std::fs::read_to_string(&file).unwrap();
        let now = stamp(&file);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            (content.as_str(), mode, is_link, left.len()),
            ("new", 0o755, true, 2)
        );
        assert_eq!(written, now);
        assert_eq!(changed.len(), usize::from(known != written));
    }
}
