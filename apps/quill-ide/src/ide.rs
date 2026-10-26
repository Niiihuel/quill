//! quill-ide's state, what the person's clicks and keys ask of it, and the window (`view`).
//! The disk is the host's: `update` returns `Job`s, and what they find comes back to `loaded`.

use limn::{CodeEditor, Component, EditorController, Element, Key, Size, UiAction};
use limn_editor::{BufferSnapshot, EditKind, Selection, TextBuffer};
use quill_components::look::{self, SMALL};
use quill_components::{icons, pieces::*};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const TREE_TOGGLE: &str = "tree:toggle";
/// A row of the file tree: denser than the sidebar's 32 px, as a reader's tree is.
const ROW: f32 = 26.0;
const INDENT: f32 = 12.0;
const NOTICE: f32 = 40.0;

/// When a file or folder last changed on the disk; `None` when it is not there.
pub type Stamp = Option<SystemTime>;

/// What the host does on the disk, beside the window.
#[derive(Debug, PartialEq)]
pub enum Job {
    List(PathBuf),
    Read(PathBuf),
    Write(PathBuf, Save),
    /// The paths watched, with when each was last seen changed.
    Stat(Vec<(PathBuf, Stamp)>),
}

/// The text a save writes: the buffer as it was when the person asked.
#[derive(Debug)]
pub struct Save(pub BufferSnapshot);

impl PartialEq for Save {
    fn eq(&self, other: &Self) -> bool {
        (self.0.document_id(), self.0.revision()) == (other.0.document_id(), other.0.revision())
    }
}

/// What a `Job` found.
#[derive(Debug)]
pub enum Loaded {
    Listed(PathBuf, Vec<Entry>, Stamp),
    /// The file's text, or `None` when it is not text or could not be read.
    Read(PathBuf, Option<String>, Stamp),
    Written(PathBuf, Save, Result<Stamp, String>),
    /// Of the paths a `Stat` asked about, the ones changed since, with their new stamps.
    Changed(Vec<(PathBuf, Stamp)>),
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
    pub stamp: Stamp,
    /// Changed on the disk while it held changes of the person's, which a reload would lose.
    pub stale: bool,
    /// The person asked for the disk's text over theirs.
    pub reloading: bool,
    /// Why the last save failed.
    pub error: Option<String>,
}

impl Buffer {
    pub fn dirty(&self) -> bool {
        let snapshot = self.editor.as_ref().and_then(|e| e.snapshot().ok());
        snapshot.is_some_and(|s| s.is_dirty())
    }

    fn editor_id(&self) -> String {
        format!("editor:{}", self.path.display())
    }
}

pub struct Ide {
    pub root: PathBuf,
    pub tree_open: bool,
    pub expanded: HashSet<PathBuf>,
    /// The folders listed so far, each with its entries in the order shown.
    pub listed: HashMap<PathBuf, Vec<Entry>>,
    /// When each listed folder last changed, to list it again when it does.
    pub dir_stamps: HashMap<PathBuf, Stamp>,
    pub buffers: Vec<Buffer>,
    pub active: Option<usize>,
    /// A `Stat` is out; the next waits for it.
    pub polling: bool,
    /// The file with unsaved changes the person asked once to close.
    pub closing: Option<PathBuf>,
    /// The person asked once to close the window with unsaved changes.
    pub quitting: bool,
}

impl Ide {
    pub fn new(root: PathBuf) -> (Self, Vec<Job>) {
        let jobs = vec![Job::List(root.clone())];
        let ide = Self {
            root,
            tree_open: true,
            expanded: HashSet::new(),
            listed: HashMap::new(),
            dir_stamps: HashMap::new(),
            buffers: vec![],
            active: None,
            polling: false,
            closing: None,
            quitting: false,
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

    /// Close a file; one with unsaved changes closes when asked a second time in a row.
    pub fn close(&mut self, i: usize) {
        let Some(buffer) = self.buffers.get(i) else {
            return;
        };
        if buffer.dirty() && self.closing.as_ref() != Some(&buffer.path) {
            self.closing = Some(buffer.path.clone());
            self.active = Some(i);
            return;
        }
        self.closing = None;
        self.buffers.remove(i);
        self.active = match self.active {
            _ if self.buffers.is_empty() => None,
            Some(a) if a > i || a == self.buffers.len() => Some(a - 1),
            active => active,
        };
    }

    /// How many open files have changes not saved.
    pub fn unsaved(&self) -> usize {
        self.buffers.iter().filter(|b| b.dirty()).count()
    }

    pub fn update(&mut self, action: &UiAction) -> Vec<Job> {
        if let UiAction::EditorSaveRequested { id, .. } = action {
            let buffer = self.buffers.iter().find(|b| b.editor_id() == *id);
            let snapshot = buffer.and_then(|b| Some((b, b.editor.as_ref()?.snapshot().ok()?)));
            return snapshot
                .map(|(b, s)| vec![Job::Write(b.path.clone(), Save(s))])
                .unwrap_or_default();
        }
        let UiAction::Clicked { id } = action else {
            return vec![];
        };
        if !id.starts_with("close:") {
            self.closing = None;
        }
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
        } else if let Some(i) = id.strip_prefix("reload:").and_then(|i| i.parse().ok()) {
            if let Some(buffer) = self.buffers.get_mut::<usize>(i) {
                buffer.reloading = true;
                return vec![Job::Read(buffer.path.clone())];
            }
        }
        vec![]
    }

    /// What to look at on the disk, once a second: the open files and the folders listed.
    pub fn poll(&mut self) -> Vec<Job> {
        if self.polling {
            return vec![];
        }
        self.polling = true;
        let mut watched: Vec<(PathBuf, Stamp)> = self
            .dir_stamps
            .iter()
            .map(|(p, s)| (p.clone(), *s))
            .collect();
        watched.extend(self.buffers.iter().map(|b| (b.path.clone(), b.stamp)));
        vec![Job::Stat(watched)]
    }

    pub fn loaded(&mut self, loaded: Loaded) -> Vec<Job> {
        match loaded {
            Loaded::Listed(dir, entries, stamp) => {
                self.dir_stamps.insert(dir.clone(), stamp);
                self.listed.insert(dir, entries);
            }
            Loaded::Read(path, text, stamp) => self.read(path, text, stamp),
            Loaded::Written(path, Save(snapshot), result) => {
                let Some(buffer) = self.buffers.iter_mut().find(|b| b.path == path) else {
                    return vec![];
                };
                match result {
                    Ok(stamp) => {
                        let shared = buffer.editor.as_ref().map(|e| e.shared_buffer());
                        if let Some(Ok(mut text)) = shared.as_ref().map(|s| s.lock()) {
                            let _ = text.mark_saved_snapshot(&snapshot);
                        }
                        buffer.stamp = stamp;
                        buffer.stale = false;
                        buffer.error = None;
                    }
                    Err(error) => buffer.error = Some(error),
                }
            }
            Loaded::Changed(changed) => {
                self.polling = false;
                let mut jobs = vec![];
                for (path, stamp) in changed {
                    if self.dir_stamps.contains_key(&path) {
                        jobs.push(Job::List(path.clone()));
                    }
                    if let Some(buffer) = self.buffers.iter_mut().find(|b| b.path == path) {
                        if buffer.dirty() {
                            buffer.stale = true;
                            buffer.stamp = stamp;
                        } else {
                            jobs.push(Job::Read(path));
                        }
                    }
                }
                return jobs;
            }
        }
        vec![]
    }

    /// A file read: opened, or what the disk holds now in place of what it showed, unless that
    /// would lose the person's changes.
    fn read(&mut self, path: PathBuf, text: Option<String>, stamp: Stamp) {
        let Some(buffer) = self.buffers.iter_mut().find(|b| b.path == path) else {
            let editor = text
                .and_then(|t| TextBuffer::new(&t).ok())
                .map(EditorController::new);
            self.buffers.push(Buffer {
                path,
                editor,
                stamp,
                stale: false,
                reloading: false,
                error: None,
            });
            self.active = Some(self.buffers.len() - 1);
            return;
        };
        let keep_theirs = buffer.dirty() && !buffer.reloading;
        match (&buffer.editor, text) {
            (Some(editor), Some(text)) if !keep_theirs => buffer.stale = !reload(editor, &text),
            // A file that was not text may be now.
            (None, text) => {
                buffer.editor = text
                    .and_then(|t| TextBuffer::new(&t).ok())
                    .map(EditorController::new)
            }
            _ => buffer.stale = true,
        }
        buffer.stamp = stamp;
        buffer.reloading = false;
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

/// Put the disk's `text` in the buffer as one undoable edit of only what differs, so the caret
/// and the rest stay where they were, and the buffer reads as saved. False when the buffer
/// refused the edit.
fn reload(editor: &EditorController, text: &str) -> bool {
    let shared = editor.shared_buffer();
    let Ok(mut buffer) = shared.lock() else {
        return false;
    };
    let old: Vec<char> = buffer.snapshot().chunks().flat_map(str::chars).collect();
    let new: Vec<char> = text.chars().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let end = old.len() - suffix;
    let replacement: String = new[prefix..new.len() - suffix].iter().collect();
    if end > prefix || !replacement.is_empty() {
        let head = buffer.selection().head;
        // After the change it moves with the text; inside it, to its start.
        let head = if head >= end {
            head + new.len() - old.len()
        } else {
            head.min(prefix)
        };
        let edit = buffer.edit(
            prefix..end,
            &replacement,
            Selection::caret(head),
            EditKind::Replace,
            0,
        );
        if edit.is_err() {
            return false;
        }
    }
    buffer.mark_saved();
    true
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
/// filled, with a dot while a file has changes not saved.
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
        let mut items = vec![hgap(4.0), text(name(&buffer.path)).centered_y()];
        let mark = match buffer {
            b if b.error.is_some() => Some(look::failed()),
            b if b.stale => Some(look::awaiting()),
            b if b.dirty() => Some(look::muted()),
            _ => None,
        };
        if let Some(color) = mark {
            items.push(limn::StatusDot::new(color).size(6.0).render().centered_y());
        }
        items.push(icon_tool(
            &format!("close:{i}"),
            icons::of(icons::CLOSE),
            "Close file",
            20.0,
        ));
        let pill = Element::row(items)
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

/// A line over the editor that says what the person must decide, with its button.
fn notice(words: String, ink: limn::Color, button: Option<Element>) -> Element {
    let mut row = vec![
        hgap(12.0),
        text(words).foreground(ink).centered_y(),
        Element::spacer().flex(1.0),
    ];
    row.extend(button.map(Element::centered_y));
    row.push(hgap(6.0));
    Element::column(vec![
        Element::row(row).gap(8.0).size(None, Some(NOTICE - 1.0)),
        hline(None, look::pane_border()),
    ])
}

fn notices(ide: &Ide) -> Vec<Element> {
    let mut shown = vec![];
    if ide.quitting {
        let files = match ide.unsaved() {
            1 => "1 file has".to_owned(),
            n => format!("{n} files have"),
        };
        let words = format!("{files} unsaved changes: close the window again to discard them.");
        shown.push(notice(words, look::palette().text, None));
    }
    let Some((i, buffer)) = ide.active.zip(ide.active_buffer()) else {
        return shown;
    };
    if let Some(error) = &buffer.error {
        shown.push(notice(format!("Not saved: {error}"), look::failed(), None));
    }
    if buffer.stale {
        let reload = keyed_button(&format!("reload:{i}"), "Reload", "", false);
        let words = "Changed on disk. Ctrl+S keeps yours; Reload takes the disk's.";
        shown.push(notice(words.into(), look::palette().text, Some(reload)));
    }
    if ide.closing.as_ref() == Some(&buffer.path) {
        let words = "Unsaved changes. Close it again to discard them, or save them with Ctrl+S.";
        shown.push(notice(words.into(), look::palette().text, None));
    }
    shown
}

fn editor(ide: &Ide, height: f32) -> Element {
    let note = |s: &str| {
        let fill = || Element::spacer().flex(1.0);
        Element::column(vec![fill(), muted(s).centered_x(), fill()]).flex(1.0)
    };
    let mut column = notices(ide);
    let height = height - column.len() as f32 * NOTICE;
    column.push(match ide.active_buffer() {
        None => note("Pick a file to read it"),
        Some(Buffer { editor: None, .. }) => note("This file can't be shown as text."),
        Some(
            buffer @ Buffer {
                path,
                editor: Some(editor),
                ..
            },
        ) => {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            // An editor per file keeps each one's caret and scroll while another is on screen.
            // Vim starts in Normal mode: reading, until `i`.
            CodeEditor::new(buffer.editor_id(), editor.clone())
                .palette(look::palette())
                .language(limn::CodeLanguage::from_code_fence(ext))
                .vim_mode(true)
                .status_bar(true)
                .height(height)
                .render()
        }
    });
    Element::column(column)
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
            None,
        ));
        ide
    }

    fn read(ide: &mut Ide, path: &str, text: &str) -> Vec<Job> {
        ide.loaded(Loaded::Read(path.into(), Some(text.into()), None))
    }

    fn type_in(buffer: &Buffer, at: usize, text: &str) {
        let shared = buffer.editor.as_ref().unwrap().shared_buffer();
        let mut buffer = shared.lock().unwrap();
        let caret = Selection::caret(at + text.chars().count());
        buffer
            .edit(at..at, text, caret, EditKind::Typing, 0)
            .unwrap();
    }

    fn text_of(buffer: &Buffer) -> String {
        let snapshot = buffer.editor.as_ref().unwrap().snapshot().unwrap();
        snapshot.chunks().collect()
    }

    fn stamp(secs: u64) -> Stamp {
        Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs))
    }

    #[test]
    fn a_folder_is_listed_once_and_folds() {
        let mut ide = with_files();
        assert_eq!(
            click(&mut ide, "dir:/r/src"),
            vec![Job::List("/r/src".into())]
        );
        ide.loaded(Loaded::Listed("/r/src".into(), vec![], None));
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
        read(&mut ide, "/r/a.rs", "fn a() {}");
        ide.loaded(Loaded::Read("/r/b.bin".into(), None, None));
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
            read(&mut ide, p, "");
        }
        ide.active = Some(2);
        ide.close(0);
        assert_eq!(ide.buffers[ide.active.unwrap()].path, Path::new("/r/3"));
    }

    #[test]
    fn a_file_with_unsaved_changes_closes_when_asked_twice_in_a_row() {
        let mut ide = with_files();
        read(&mut ide, "/r/a.rs", "a");
        type_in(&ide.buffers[0], 0, "x");
        click(&mut ide, "close:0");
        assert_eq!(ide.buffers.len(), 1);
        click(&mut ide, TREE_TOGGLE);
        click(&mut ide, "close:0");
        assert_eq!(ide.buffers.len(), 1);
        click(&mut ide, "close:0");
        assert!(ide.buffers.is_empty());
    }

    #[test]
    fn a_save_writes_what_the_buffer_held_and_then_it_reads_as_saved() {
        let mut ide = with_files();
        read(&mut ide, "/r/a.rs", "a");
        type_in(&ide.buffers[0], 1, "b");
        assert!(ide.buffers[0].dirty());
        let jobs = ide.update(&UiAction::EditorSaveRequested {
            id: "editor:/r/a.rs".into(),
            document_id: 0,
            revision: 0,
        });
        let [Job::Write(path, save)] = <[Job; 1]>::try_from(jobs).unwrap() else {
            panic!("a save writes");
        };
        assert_eq!(save.0.chunks().collect::<String>(), "ab");
        type_in(&ide.buffers[0], 2, "c");
        ide.loaded(Loaded::Written(path, save, Ok(stamp(5))));
        // What was typed after the save asked is still to save.
        assert!(ide.buffers[0].dirty());
        assert_eq!(ide.buffers[0].stamp, stamp(5));
        ide.loaded(Loaded::Written(
            "/r/a.rs".into(),
            Save(ide.buffers[0].editor.as_ref().unwrap().snapshot().unwrap()),
            Err("disk full".into()),
        ));
        assert_eq!(ide.buffers[0].error.as_deref(), Some("disk full"));
    }

    #[test]
    fn a_file_changed_on_disk_reloads_in_place_keeping_the_caret() {
        let mut ide = with_files();
        read(&mut ide, "/r/a.rs", "abc\ndef");
        let shared = ide.buffers[0].editor.as_ref().unwrap().shared_buffer();
        shared
            .lock()
            .unwrap()
            .set_selection(Selection::caret(5))
            .unwrap();
        let [Job::Stat(watched)] = <[Job; 1]>::try_from(ide.poll()).unwrap() else {
            panic!("a poll stats");
        };
        assert!(watched.contains(&("/r/a.rs".into(), None)));
        assert!(ide.poll().is_empty(), "one at a time");
        let jobs = ide.loaded(Loaded::Changed(vec![("/r/a.rs".into(), stamp(1))]));
        assert_eq!(jobs, vec![Job::Read("/r/a.rs".into())]);
        ide.loaded(Loaded::Read(
            "/r/a.rs".into(),
            Some("aXbc\ndef".into()),
            stamp(1),
        ));
        let buffer = &ide.buffers[0];
        assert_eq!(text_of(buffer), "aXbc\ndef");
        assert!(!buffer.dirty());
        assert_eq!(shared.lock().unwrap().selection().head, 6);
        assert!(shared.lock().unwrap().can_undo());
        assert_eq!(ide.poll().len(), 1);
    }

    #[test]
    fn unsaved_changes_are_kept_over_the_disk_until_the_person_reloads() {
        let mut ide = with_files();
        read(&mut ide, "/r/a.rs", "a");
        type_in(&ide.buffers[0], 1, "b");
        let jobs = ide.loaded(Loaded::Changed(vec![("/r/a.rs".into(), stamp(2))]));
        assert!(jobs.is_empty());
        assert!(ide.buffers[0].stale);
        assert_eq!(text_of(&ide.buffers[0]), "ab");
        assert_eq!(
            click(&mut ide, "reload:0"),
            vec![Job::Read("/r/a.rs".into())]
        );
        ide.loaded(Loaded::Read(
            "/r/a.rs".into(),
            Some("disk".into()),
            stamp(2),
        ));
        assert_eq!(text_of(&ide.buffers[0]), "disk");
        assert!(!ide.buffers[0].stale && !ide.buffers[0].dirty());
    }

    #[test]
    fn a_listed_folder_that_changes_is_listed_again() {
        let mut ide = with_files();
        let jobs = ide.loaded(Loaded::Changed(vec![("/r".into(), stamp(3))]));
        assert_eq!(jobs, vec![Job::List("/r".into())]);
    }

    #[test]
    fn shortcuts_leave_vim_its_keys_and_cycle_the_files() {
        let mut ide = with_files();
        assert_eq!(ide.shortcut(Key::B, false), None);
        assert_eq!(ide.shortcut(Key::W, false), None);
        for p in ["/r/1", "/r/2"] {
            read(&mut ide, p, "");
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
        read(&mut ide, "/r/a.rs", "fn a() {}\n");
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
        // A stale file offers to take the disk's text.
        ide.buffers[0].stale = true;
        let frame = UiTree::new(view(&ide, window)).layout(window);
        assert!(frame.regions.iter().any(|r| r.id == "reload:0"));
    }
}
