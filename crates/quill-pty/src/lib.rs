//! A shell on a pseudo-terminal in a session's folder (features/terminal.md): its output is
//! parsed into a screen of cells for limn's `TerminalView`, and keys become the bytes a
//! terminal sends. A reader thread wakes the window when new output arrives.

use limn::{Color, Key, TerminalCell, TerminalCursor, TerminalCursorShape, TerminalScreen};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The sixteen ANSI colors of Claude Desktop's light terminal theme, as
/// features/terminal.md records them.
const ANSI: [Color; 16] = [
    Color::rgb(0x1a, 0x1a, 0x1a),
    Color::rgb(0xff, 0x3a, 0x30),
    Color::rgb(0x1e, 0x9e, 0x3c),
    Color::rgb(0x98, 0x80, 0x1f),
    Color::rgb(0x00, 0x73, 0xe6),
    Color::rgb(0xcd, 0x20, 0x54),
    Color::rgb(0x8e, 0x6b, 0xd9),
    Color::rgb(0x99, 0x99, 0x99),
    Color::rgb(0x66, 0x66, 0x66),
    Color::rgb(0xff, 0x50, 0x47),
    Color::rgb(0x1e, 0x9e, 0x3c),
    Color::rgb(0x98, 0x80, 0x1f),
    Color::rgb(0x00, 0x78, 0xf0),
    Color::rgb(0xcd, 0x20, 0x54),
    Color::rgb(0x8e, 0x6b, 0xd9),
    Color::rgb(0xd6, 0xd6, 0xd6),
];
const FOREGROUND: Color = Color::rgb(0x1a, 0x1a, 0x1a);
const SCROLLBACK: usize = 2000;

pub struct Terminal {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Terminal {
    /// The person's login shell in `cwd`, `cols` × `rows`; `wake` runs after new output.
    pub fn spawn(
        cwd: &Path,
        cols: u16,
        rows: u16,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .map_err(std::io::Error::other)?;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut command = CommandBuilder::new(shell);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "quill");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(std::io::Error::other)?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(std::io::Error::other)?;
        let writer = pair.master.take_writer().map_err(std::io::Error::other)?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
        let output = parser.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                output.lock().unwrap().process(&buffer[..read]);
                wake();
            }
        });
        Ok(Self {
            master: pair.master,
            writer,
            parser,
            child,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let (current_rows, current_cols) = self.parser.lock().unwrap().screen().size();
        if (cols, rows) == (current_cols, current_rows) || cols == 0 || rows == 0 {
            return;
        }
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.parser
            .lock()
            .unwrap()
            .screen_mut()
            .set_size(rows, cols);
    }

    /// Whether the shell is still running.
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// The screen as it is now, for `TerminalView`.
    pub fn screen(&self) -> TerminalScreen {
        screen_of(self.parser.lock().unwrap().screen())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn color(c: vt100::Color, fallback: Color) -> Color {
    match c {
        vt100::Color::Default => fallback,
        vt100::Color::Idx(i) if i < 16 => ANSI[i as usize],
        vt100::Color::Idx(i) if i < 232 => {
            // The xterm 6×6×6 cube.
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color::rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        vt100::Color::Idx(i) => {
            let gray = 8 + (i - 232) * 10;
            Color::rgb(gray, gray, gray)
        }
        vt100::Color::Rgb(r, g, b) => Color::rgb(r, g, b),
    }
}

fn screen_of(screen: &vt100::Screen) -> TerminalScreen {
    let (rows, cols) = screen.size();
    let mut cells = Vec::with_capacity(rows as usize * cols as usize);
    for row in 0..rows {
        for col in 0..cols {
            let cell = screen.cell(row, col);
            let ch = cell
                .and_then(|c| c.contents().chars().next())
                .unwrap_or(' ');
            let mut out = TerminalCell::new(
                ch,
                cell.map_or(FOREGROUND, |c| color(c.fgcolor(), FOREGROUND)),
            );
            if let Some(c) = cell {
                out.bg = match c.bgcolor() {
                    vt100::Color::Default => None,
                    other => Some(color(other, FOREGROUND)),
                };
                out.bold = c.bold();
                out.underline = c.underline();
                out.inverse = c.inverse();
            }
            cells.push(out);
        }
    }
    let (row, col) = screen.cursor_position();
    let cursor = (!screen.hide_cursor()).then_some(TerminalCursor {
        col: col as usize,
        row: row as usize,
        shape: TerminalCursorShape::Block,
    });
    TerminalScreen {
        cols: cols as usize,
        rows: rows as usize,
        cells,
        scrollback: vec![],
        cursor,
    }
}

/// The bytes a terminal sends for a key, or `None` for keys the shell does not take.
pub fn key_bytes(key: Key, ctrl: bool, shift: bool) -> Option<Vec<u8>> {
    let letter = |k: Key| -> Option<u8> {
        let name = format!("{k:?}");
        (name.len() == 1).then(|| name.as_bytes()[0].to_ascii_lowercase())
    };
    Some(match key {
        Key::Enter => b"\r".to_vec(),
        Key::Backspace => vec![0x7f],
        Key::Tab if shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Escape => vec![0x1b],
        Key::ArrowUp => b"\x1b[A".to_vec(),
        Key::ArrowDown => b"\x1b[B".to_vec(),
        Key::ArrowRight => b"\x1b[C".to_vec(),
        Key::ArrowLeft => b"\x1b[D".to_vec(),
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        // Ctrl+letter is the letter's control code (Ctrl+C = 0x03).
        other if ctrl => vec![letter(other)? - b'a' + 1],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_become_what_a_terminal_sends() {
        assert_eq!(key_bytes(Key::C, true, false), Some(vec![3]));
        assert_eq!(key_bytes(Key::Enter, false, false), Some(b"\r".to_vec()));
        assert_eq!(
            key_bytes(Key::ArrowUp, false, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(key_bytes(Key::A, false, false), None);
    }

    #[test]
    fn colored_output_becomes_cells() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        parser.process(b"\x1b[32mok\x1b[0m done");
        let screen = screen_of(parser.screen());
        assert_eq!((screen.cols, screen.rows, screen.cells.len()), (10, 2, 20));
        assert_eq!(screen.cells[0].ch, 'o');
        assert_eq!(screen.cells[0].fg, ANSI[2]);
        assert_eq!(screen.cells[3].fg, FOREGROUND);
    }

    #[test]
    fn a_shell_runs_and_its_output_arrives() {
        let dir = std::env::temp_dir();
        let mut terminal = Terminal::spawn(&dir, 40, 5, || {}).unwrap();
        terminal.write(b"echo quill-$((40+2))\r");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let text: String = terminal.screen().cells.iter().map(|c| c.ch).collect();
            if text.contains("quill-42") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no output: {text}");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(terminal.alive());
    }
}
