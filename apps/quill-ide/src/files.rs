//! What the file tree may do to the disk, after openADE's: every target is a plain path under
//! the folder, links are never followed, nothing is overwritten, and deleting only moves to
//! the desktop's trash. Messages are for the person, without absolute paths.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// One operation; every path is from the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    CreateFile { parent: PathBuf, name: String },
    CreateDir { parent: PathBuf, name: String },
    Rename { path: PathBuf, name: String },
    Trash { path: PathBuf },
}

/// Moves one absolute path to the trash: the desktop's, or a test's stand-in.
pub type Trash = fn(&Path) -> Result<(), String>;

pub fn system_trash(path: &Path) -> Result<(), String> {
    // The crate's own message can name paths outside the folder.
    trash::delete(path).map_err(|_| "the desktop's trash is not available".to_owned())
}

/// Runs `op` under `root`, and says the path it made or moved to, from the folder.
pub fn run(root: &Path, op: &Op, trash: Trash) -> Result<Option<PathBuf>, String> {
    Ok(match op {
        Op::CreateFile { parent, name } => Some(create(root, parent, name, false)?),
        Op::CreateDir { parent, name } => Some(create(root, parent, name, true)?),
        Op::Rename { path, name } => Some(rename(root, path, name)?),
        Op::Trash { path } => {
            trash_path(root, path, trash)?;
            None
        }
    })
}

/// Where `path` is once `old` moved to `new`, or `Some(None)` once `old` is gone; `None` when
/// `path` is neither `old` nor under it.
pub fn moved(path: &Path, old: &Path, new: Option<&Path>) -> Option<Option<PathBuf>> {
    let rest = path.strip_prefix(old).ok()?;
    Some(new.map(|new| {
        if rest.as_os_str().is_empty() {
            new.to_path_buf()
        } else {
            new.join(rest)
        }
    }))
}

pub fn valid_name(name: &str) -> Result<(), String> {
    Err(if name.trim().is_empty() {
        "Type a name"
    } else if name == "." || name == ".." {
        "\u{201c}.\u{201d} and \u{201c}..\u{201d} are not names"
    } else if name.contains(['/', '\\', '\0']) {
        "A name has no slashes"
    } else {
        return Ok(());
    }
    .to_owned())
}

/// The cause of an error without the path the system may add to it, as a sentence.
fn reason(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "It is not there anymore".into(),
        io::ErrorKind::AlreadyExists => "Something has that name already".into(),
        io::ErrorKind::PermissionDenied => "Permission denied".into(),
        io::ErrorKind::StorageFull => "The disk is full".into(),
        kind => format!("The system refused it ({kind:?})"),
    }
}

/// The absolute path of `relative` under the real root. Only plain names are taken, a linked
/// folder on the way is refused rather than followed, and the last name is not resolved.
fn locate(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    const OUTSIDE: &str = "That is outside the folder";
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(OUTSIDE.into());
    }
    let root = root.canonicalize().map_err(|e| reason(&e))?;
    if relative.as_os_str().is_empty() {
        return Ok(root);
    }
    let target = root.join(relative);
    let parent = target.parent().unwrap_or(&root);
    if parent.canonicalize().map_err(|e| reason(&e))? != parent {
        return Err(OUTSIDE.into());
    }
    Ok(target)
}

fn create(root: &Path, parent: &Path, name: &str, folder: bool) -> Result<PathBuf, String> {
    valid_name(name)?;
    let dir = locate(root, parent)?;
    // A link to a folder is a link, not a place to make things in.
    if !fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return Err("Its folder is not there anymore".into());
    }
    let target = dir.join(name);
    // Both fail on anything already there, a dangling link too.
    if folder {
        fs::create_dir(&target)
    } else {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map(drop)
    }
    .map_err(|e| reason(&e))?;
    Ok(parent.join(name))
}

fn rename(root: &Path, path: &Path, name: &str) -> Result<PathBuf, String> {
    valid_name(name)?;
    if path.file_name().is_none() {
        return Err("The folder itself is not renamed here".into());
    }
    let source = locate(root, path)?;
    fs::symlink_metadata(&source).map_err(|e| reason(&e))?;
    move_without_replacing(&source, &source.with_file_name(name)).map_err(|e| reason(&e))?;
    Ok(path.with_file_name(name))
}

fn trash_path(root: &Path, path: &Path, trash: Trash) -> Result<(), String> {
    if path.file_name().is_none() {
        return Err("The folder itself is not trashed here".into());
    }
    let target = locate(root, path)?;
    fs::symlink_metadata(&target).map_err(|e| reason(&e))?;
    trash(&target)
}

/// Rename `source` to `target` only if nothing is there: atomically on Linux.
fn move_without_replacing(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_path = |path: &Path| {
            std::ffi::CString::new(path.as_os_str().as_bytes())
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
        };
        let (from, to) = (c_path(source)?, c_path(target)?);
        // SAFETY: both pointers are NUL-terminated paths that outlive the call.
        let moved = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if moved == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        // A filesystem without the flag takes the checked rename below.
        if !matches!(error.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS)) {
            return Err(error);
        }
    }
    // shortcut: between this check and the rename another process can make `target` and lose
    // it; only Linux has the atomic form above.
    if fs::symlink_metadata(target).is_ok() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    fs::rename(source, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(path: &Path) -> Result<(), String> {
        let dir = path.parent().unwrap().join(".trashed");
        fs::create_dir_all(&dir).unwrap();
        fs::rename(path, dir.join(path.file_name().unwrap())).map_err(|e| e.to_string())
    }

    #[test]
    fn files_are_made_renamed_and_trashed_without_overwriting_or_leaving_the_folder() {
        let root = std::env::temp_dir().join(format!("quill-ide-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let op = |op| run(&root, &op, kept);
        let file = |parent: &str, name: &str| Op::CreateFile {
            parent: parent.into(),
            name: name.into(),
        };
        assert_eq!(
            op(Op::CreateDir {
                parent: "".into(),
                name: "src".into()
            }),
            Ok(Some("src".into()))
        );
        assert_eq!(op(file("src", "a.rs")), Ok(Some("src/a.rs".into())));
        fs::write(root.join("src/a.rs"), "kept").unwrap();
        assert_eq!(
            op(file("src", "a.rs")),
            Err("Something has that name already".into())
        );
        assert_eq!(op(file("src", "x/y")), Err("A name has no slashes".into()));
        assert_eq!(
            op(file("../out", "z")),
            Err("That is outside the folder".into())
        );
        op(file("src", "b.rs")).unwrap();
        let rename = |path: &str, name: &str| Op::Rename {
            path: path.into(),
            name: name.into(),
        };
        assert_eq!(
            op(rename("src/b.rs", "a.rs")),
            Err("Something has that name already".into())
        );
        assert_eq!(fs::read_to_string(root.join("src/a.rs")).unwrap(), "kept");
        assert_eq!(op(rename("src/b.rs", "c.rs")), Ok(Some("src/c.rs".into())));
        assert_eq!(
            op(Op::Trash {
                path: "src/c.rs".into()
            }),
            Ok(None)
        );
        let left = root.join("src/.trashed/c.rs").exists() && !root.join("src/c.rs").exists();
        fs::remove_dir_all(&root).unwrap();
        assert!(left);
        let at = Path::new("src/a.rs");
        assert_eq!(
            moved(at, Path::new("src"), Some(Path::new("lib"))),
            Some(Some("lib/a.rs".into()))
        );
        assert_eq!(moved(at, Path::new("src/a.rs"), None), Some(None));
        assert_eq!(moved(at, Path::new("docs"), None), None);
    }
}
