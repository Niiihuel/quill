//! The line between quill and quill-ide: Unix sockets in the person's runtime folder, one
//! JSON message a line. Each IDE window listens on its folder's socket, so a file of a
//! folder already open goes to that window; quill listens on its own.

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// What one app asks of the other.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Message {
    /// Open `path` in the IDE window of `root`, at a 1-based line and column when given.
    Open {
        root: PathBuf,
        path: PathBuf,
        line: Option<u32>,
        column: Option<u32>,
    },
    /// Ask an agent about code of `root`: the file, its 1-based first and last lines when it
    /// is a part, and that text.
    Ask {
        root: PathBuf,
        path: PathBuf,
        lines: Option<(u32, u32)>,
        text: String,
    },
}

/// The most a message may be: a selection of a large file, not a file of any size.
const LONGEST: u64 = 1 << 20;

/// The folder the sockets live in, the person's own: `$XDG_RUNTIME_DIR/quill`.
fn folder() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let user = std::env::var("USER").unwrap_or_default();
            std::env::temp_dir().join(format!("quill-{user}"))
        });
    base.join("quill")
}

/// quill's socket.
pub fn quill_socket() -> PathBuf {
    folder().join("quill.sock")
}

/// The socket of the IDE window showing `root`: named by a hash of the folder, as a path may
/// be longer than a socket's name can.
pub fn ide_socket(root: &Path) -> PathBuf {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    // FNV-1a: the same in both apps whatever their build, unlike std's hasher.
    let hash = root
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        });
    folder().join(format!("ide-{hash:016x}.sock"))
}

/// Send `message` to whoever listens on `socket`. An error when no one does.
pub fn send(socket: &Path, message: &Message) -> io::Result<()> {
    let mut stream = UnixStream::connect(socket)?;
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    stream.write_all(line.as_bytes())
}

/// A socket being listened on; dropped, it is taken away.
pub struct Listener {
    path: PathBuf,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Listen on `socket`, handing each message heard to `heard` on a thread of its own. A socket
/// left by an app that is gone is replaced; one that answers is someone else's, an error.
pub fn listen(socket: &Path, heard: impl Fn(Message) + Send + 'static) -> io::Result<Listener> {
    if let Some(parent) = socket.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    if UnixStream::connect(socket).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "another window listens there",
        ));
    }
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    // Only the person's own apps may talk to it.
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            for line in BufReader::new(stream.take(LONGEST)).lines() {
                let Ok(line) = line else { break };
                // What does not read as a message is left, never a crash.
                if let Ok(message) = serde_json::from_str(&line) {
                    heard(message);
                }
            }
        }
    });
    Ok(Listener {
        path: socket.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn a_message_sent_is_heard_once_and_a_live_socket_is_not_taken() {
        let dir = std::env::temp_dir().join(format!("quill-link-test-{}", std::process::id()));
        let socket = dir.join("a.sock");
        let (tell, told) = mpsc::channel();
        let listening = listen(&socket, move |m| tell.send(m).unwrap()).unwrap();
        let open = Message::Open {
            root: "/r".into(),
            path: "/r/src/main.rs".into(),
            line: Some(12),
            column: None,
        };
        send(&socket, &open).unwrap();
        assert_eq!(told.recv_timeout(Duration::from_secs(2)).unwrap(), open);
        // Garbage on the line is left; what follows still arrives.
        UnixStream::connect(&socket)
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        let ask = Message::Ask {
            root: "/r".into(),
            path: "/r/a.rs".into(),
            lines: Some((3, 4)),
            text: "x".into(),
        };
        send(&socket, &ask).unwrap();
        assert_eq!(told.recv_timeout(Duration::from_secs(2)).unwrap(), ask);
        assert!(listen(&socket, |_| {}).is_err());
        drop(listening);
        assert!(send(&socket, &ask).is_err());
        assert_eq!(ide_socket(Path::new("/a/b")), ide_socket(Path::new("/a/b")));
        assert_ne!(ide_socket(Path::new("/a/b")), ide_socket(Path::new("/a/c")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
