//! quill-ide's state, what the person's clicks and keys ask of it, and the window (`view`).
//! The disk is the host's: `update` returns `Job`s, and what they find comes back to `loaded`.

use limn::{CodeEditor, Component, EditorController, Element, Key, Size, UiAction};
use quill_components::look::{self, SMALL};
use quill_components::{icons, pieces::*};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const TREE_TOGGLE: &str = "tree:toggle";
/// A row of the file tree: denser than the sidebar's 32 px, as a reader's tree is.
const ROW: f32 = 26.0;
const INDENT: f32 = 12.0;

/// What the host does on the disk, beside the window.
#[derive(Debug, PartialEq)]
pub enum Job {
    List(PathBuf),
    Read(PathBuf),
}

/// What a `Job` found.
#[derive(Debug)]
pub enum Loaded {
    Listed(PathBuf, Vec<Entry>),
    /// The file's text, or `None` when it is not text or could not be read.
    Read(PathBuf, Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub dir: bool,
}

pub struct Buffer {
    pub path: PathBuf,
    /// `None` for a file that can't be shown as text.
    pub editor: Option<EditorController>,
}

pub struct Ide {
    pub root: PathBuf,
    pub tree_open: bool,
    pub expanded: HashSet<PathBuf>,
    /// The folders listed so far, each with its entries in the order shown.
    pub listed: HashMap<PathBuf, Vec<Entry>>,
    pub buffers: Vec<Buffer>,
    pub active: Option<usize>,
}

impl Ide {
    pub fn new(root: PathBuf) -> (Self, Vec<Job>) {
        let jobs = vec![Job::List(root.clone())];
        let ide = Self {
            root,
            tree_open: true,
            expanded: HashSet::new(),
            listed: HashMap::new(),
            buffers: vec![],
            active: None,
        };
        (ide, jobs)
    }

    /// Show `path` if it is open, else read it.
    pub fn open(&mut self, path: PathBuf) -> Vec<Job> {
        match self.buffers.iter().position(|b| b.path == path) {
            Some(i) => {
                self.active = Some(i);
                vec![]
            }
            None => vec![Job::Read(path)],
        }
    }

    pub fn close(&mut self, i: usize) {
        if i >= self.buffers.len() {
            return;
        }
        self.buffers.remove(i);
        self.active = match self.active {
            _ if self.buffers.is_empty() => None,
            Some(a) if a > i || a == self.buffers.len() => Some(a - 1),
            active => active,
        };
    }

    pub fn update(&mut self, action: &UiAction) -> Vec<Job> {
        let UiAction::Clicked { id } = action else {
            return vec![];
        };
        if id == TREE_TOGGLE {
            self.tree_open = !self.tree_open;
        } else if let Some(dir) = id.strip_prefix("dir:").map(PathBuf::from) {
            if !self.expanded.remove(&dir) {
                self.expanded.insert(dir.clone());
                if !self.listed.contains_key(&dir) {
                    return vec![Job::List(dir)];
                }
            }
        } else if let Some(file) = id.strip_prefix("file:") {
            return self.open(file.into());
        } else if let Some(i) = id.strip_prefix("buffer:").and_then(|i| i.parse().ok()) {
            if i < self.buffers.len() {
                self.active = Some(i);
            }
        } else if let Some(i) = id.strip_prefix("close:").and_then(|i| i.parse().ok()) {
            self.close(i);
        }
        vec![]
    }

    pub fn loaded(&mut self, loaded: Loaded) {
        match loaded {
            Loaded::Listed(dir, entries) => {
                self.listed.insert(dir, entries);
            }
            Loaded::Read(path, text) => {
                let editor = text
                    .and_then(|t| limn_editor::TextBuffer::new(&t).ok())
                    .map(EditorController::new);
                let buffer = Buffer { path, editor };
                match self.buffers.iter().position(|b| b.path == buffer.path) {
                    Some(i) => {
                        self.buffers[i] = buffer;
                        self.active = Some(i);
                    }
                    None => {
                        self.buffers.push(buffer);
                        self.active = Some(self.buffers.len() - 1);
                    }
                }
            }
        }
    }

    /// The control a Ctrl shortcut clicks. Vim's own Ctrl keys (B, F, U, D, J, N, P, R) stay
    /// the editor's.
    pub fn shortcut(&self, key: Key, shift: bool) -> Option<String> {
        let step = |by: isize| {
            let n = self.buffers.len() as isize;
            self.active
                .map(|a| format!("buffer:{}", (a as isize + by).rem_euclid(n)))
        };
        match (key, shift) {
            (Key::E, true) => Some(TREE_TOGGLE.into()),
            (Key::W, false) => self.active.map(|a| format!("close:{a}")),
            (Key::PageDown, false) => step(1),
            (Key::PageUp, false) => step(-1),
            _ => None,
        }
    }

    fn active_buffer(&self) -> Option<&Buffer> {
        self.active.and_then(|i| self.buffers.get(i))
    }
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub fn view(ide: &Ide, window: Size) -> Element {
    let below = (window.height - look::TOP_BAR - 1.0).max(0.0);
    let mut body = vec![];
    if ide.tree_open {
        body.push(tree(ide, below));
        body.push(vline(Some(below)));
    }
    body.push(editor(ide, below).flex(1.0));
    Element::column(vec![
        top_bar(ide),
        hline(None, look::pane_border()),
        Element::row(body).size(None, Some(below)),
    ])
    .size(Some(window.width), Some(window.height))
    .background(look::palette().surface)
}

/// The tree's toggle, then the open files as quill's terminal tabs: pills, the one on screen
/// filled.
fn top_bar(ide: &Ide) -> Element {
    let mut row = vec![
        hgap(9.0),
        toggle(
            TREE_TOGGLE,
            icons::of(icons::SIDEBAR),
            if ide.tree_open {
                "Hide files"
            } else {
                "Show files"
            },
            ide.tree_open,
            Some("Ctrl+Shift+E"),
        ),
        hgap(8.0),
    ];
    if ide.buffers.is_empty() {
        row.push(muted(name(&ide.root)).centered_y());
    }
    for (i, buffer) in ide.buffers.iter().enumerate() {
        let pill = Element::row(vec![
            hgap(4.0),
            text(name(&buffer.path)).centered_y(),
            icon_tool(
                &format!("close:{i}"),
                icons::of(icons::CLOSE),
                "Close file",
                20.0,
            ),
        ])
        .gap(6.0)
        .size(None, Some(28.0))
        .radius(8.0);
        let pill = if ide.active == Some(i) {
            pill.background(look::active())
        } else {
            pill
        };
        row.push(Element::clickable(format!("buffer:{i}"), pill).centered_y());
    }
    Element::row(row)
        .gap(4.0)
        .size(None, Some(look::TOP_BAR))
        .clip_children()
}

/// The folder's files as git sees them, folders first, each folder listed when first opened.
fn tree(ide: &Ide, height: f32) -> Element {
    let mut rows = vec![Element::row(vec![
        hgap(8.0),
        muted(name(&ide.root)).font_size(SMALL).centered_y(),
    ])
    .size(None, Some(ROW))];
    let shown = ide.active_buffer().map(|b| b.path.as_path());
    tree_rows(ide, &ide.root, 0, shown, &mut rows);
    Element::scroll("tree:scroll", Element::column(rows).padding(6.0))
        .size(Some(look::SIDEBAR_WIDTH), Some(height))
        .background(look::sidebar())
        .clip_children()
}

fn tree_rows(ide: &Ide, dir: &Path, depth: usize, shown: Option<&Path>, rows: &mut Vec<Element>) {
    for entry in ide.listed.get(dir).into_iter().flatten() {
        let indent = hgap(2.0 + depth as f32 * INDENT);
        let label = text(name(&entry.path)).centered_y();
        if entry.dir {
            let open = ide.expanded.contains(&entry.path);
            let caret = Element::icon(icons::of(if open {
                icons::CARET_DOWN
            } else {
                icons::CARET_RIGHT
            }))
            .foreground(look::muted())
            .size(Some(10.0), Some(10.0))
            .centered_y();
            let id = format!("dir:{}", entry.path.display());
            rows.push(side_row(id, vec![indent, caret, label], false, ROW));
            if open {
                tree_rows(ide, &entry.path, depth + 1, shown, rows);
            }
        } else {
            let id = format!("file:{}", entry.path.display());
            let selected = shown == Some(entry.path.as_path());
            // Past the caret's 10 px and the row's gap, under the folders' names.
            rows.push(side_row(id, vec![indent, hgap(10.0), label], selected, ROW));
        }
    }
}

fn editor(ide: &Ide, height: f32) -> Element {
    let note = |s: &str| {
        let fill = || Element::spacer().flex(1.0);
        Element::column(vec![fill(), muted(s).centered_x(), fill()]).flex(1.0)
    };
    match ide.active_buffer() {
        None => note("Pick a file to read it"),
        Some(Buffer { editor: None, .. }) => note("This file can't be shown as text."),
        Some(Buffer {
            path,
            editor: Some(editor),
        }) => {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            // An editor per file keeps each one's caret and scroll while another is on screen.
            CodeEditor::new(format!("editor:{}", path.display()), editor.clone())
                .palette(look::palette())
                .language(limn::CodeLanguage::from_code_fence(ext))
                .read_only(true)
                .vim_mode(true)
                .status_bar(true)
                .height(height)
                .render()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use limn::UiTree;

    fn click(ide: &mut Ide, id: &str) -> Vec<Job> {
        ide.update(&UiAction::Clicked { id: id.into() })
    }

    fn with_files() -> Ide {
        let (mut ide, jobs) = Ide::new("/r".into());
        assert_eq!(jobs, vec![Job::List("/r".into())]);
        let entry = |p: &str, dir| Entry {
            path: p.into(),
            dir,
        };
        ide.loaded(Loaded::Listed(
            "/r".into(),
            vec![entry("/r/src", true), entry("/r/a.rs", false)],
        ));
        ide
    }

    #[test]
    fn a_folder_is_listed_once_and_folds() {
        let mut ide = with_files();
        assert_eq!(
            click(&mut ide, "dir:/r/src"),
            vec![Job::List("/r/src".into())]
        );
        ide.loaded(Loaded::Listed("/r/src".into(), vec![]));
        assert!(click(&mut ide, "dir:/r/src").is_empty());
        assert!(!ide.expanded.contains(Path::new("/r/src")));
        assert!(click(&mut ide, "dir:/r/src").is_empty());
    }

    #[test]
    fn files_open_once_and_close_to_their_neighbour() {
        let mut ide = with_files();
        assert_eq!(
            click(&mut ide, "file:/r/a.rs"),
            vec![Job::Read("/r/a.rs".into())]
        );
        ide.loaded(Loaded::Read("/r/a.rs".into(), Some("fn a() {}".into())));
        ide.loaded(Loaded::Read("/r/b.bin".into(), None));
        assert_eq!(ide.active, Some(1));
        assert!(click(&mut ide, "file:/r/a.rs").is_empty());
        assert_eq!(ide.active, Some(0));
        assert!(ide.buffers[1].editor.is_none());
        ide.active = Some(1);
        click(&mut ide, "close:1");
        assert_eq!(ide.active, Some(0));
        click(&mut ide, "close:0");
        assert_eq!(ide.active, None);
    }

    #[test]
    fn closing_before_the_one_on_screen_keeps_it_on_screen() {
        let mut ide = with_files();
        for p in ["/r/1", "/r/2", "/r/3"] {
            ide.loaded(Loaded::Read(p.into(), Some(String::new())));
        }
        ide.active = Some(2);
        ide.close(0);
        assert_eq!(ide.buffers[ide.active.unwrap()].path, Path::new("/r/3"));
    }

    #[test]
    fn shortcuts_leave_vim_its_keys_and_cycle_the_files() {
        let mut ide = with_files();
        assert_eq!(ide.shortcut(Key::B, false), None);
        assert_eq!(ide.shortcut(Key::W, false), None);
        for p in ["/r/1", "/r/2"] {
            ide.loaded(Loaded::Read(p.into(), Some(String::new())));
        }
        assert_eq!(
            ide.shortcut(Key::PageDown, false).as_deref(),
            Some("buffer:0")
        );
        assert_eq!(
            ide.shortcut(Key::PageUp, false).as_deref(),
            Some("buffer:0")
        );
        assert_eq!(ide.shortcut(Key::W, false).as_deref(), Some("close:1"));
        assert_eq!(ide.shortcut(Key::E, true).as_deref(), Some(TREE_TOGGLE));
    }

    #[test]
    fn the_window_lays_out_with_and_without_the_tree() {
        let mut ide = with_files();
        ide.loaded(Loaded::Read("/r/a.rs".into(), Some("fn a() {}\n".into())));
        let window = Size::new(1408.0, 844.0);
        let frame = UiTree::new(view(&ide, window)).layout(window);
        let row = frame
            .regions
            .iter()
            .find(|r| r.id == "file:/r/a.rs")
            .unwrap();
        assert!(row.rect.width > 200.0);
        click(&mut ide, TREE_TOGGLE);
        let frame = UiTree::new(view(&ide, window)).layout(window);
        assert!(!frame.regions.iter().any(|r| r.id == "file:/r/a.rs"));
        assert!(frame.regions.iter().any(|r| r.id == "close:0"));
    }
}
