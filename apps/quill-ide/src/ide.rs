//! quill-ide's state, what the person's clicks and keys ask of it, and the window (`view`).
//! The disk is the host's: `update` returns `Job`s, and what they find comes back to `loaded`.

use crate::packs::{self, Offered, Pack, Theme};
use crate::prefs::{self, Prefs};
use limn::{
    CodeEditor, Component, ComponentPalette, EditorController, Element, Key, Point, Size, TreeRow,
    UiAction,
};
use limn_editor::{BufferSnapshot, EditKind, Selection, TextBuffer};
use quill_components::look::{self, SMALL};
use quill_components::settings::{self, SETTINGS_CLOSE, SETTINGS_OPEN, SETTINGS_SEARCH};
use quill_components::sidebar::Sidebar;
use quill_components::{icons, pieces::*};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const TREE_TOGGLE: &str = "tree:toggle";
/// A row of the file tree: denser than the sidebar's 32 px, as a reader's tree is.
const ROW: f32 = 26.0;
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
    SavePrefs(Prefs),
    /// Look for icon themes in these folders.
    FindPacks(Vec<PathBuf>),
    /// Read the icon theme at this JSON.
    LoadPack(PathBuf),
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
    Packs(Vec<Offered>),
    Pack(PathBuf, Result<Theme, String>),
    PrefsSaved,
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
    /// The files' tree, which slides shut and peeks as quill's sidebar.
    pub sidebar: Sidebar,
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
    pub prefs: Prefs,
    /// The icons' pack in use, the icon themes found, and why the chosen one did not load.
    pub pack: Pack,
    pub offered: Vec<Offered>,
    pub pack_error: Option<String>,
    /// Settings: the page open, what its search holds and the select whose menu is open.
    pub settings_page: Option<String>,
    pub settings_query: String,
    pub settings_menu: Option<String>,
    /// Where the pointer is and the control under it, which the host sets every frame, for
    /// the tree's peek.
    pub pointer: Option<Point>,
    pub hovered: Option<String>,
    pub reduced_motion: bool,
}

impl Ide {
    pub fn new(root: PathBuf, prefs: Prefs) -> (Self, Vec<Job>) {
        let mut jobs = vec![
            Job::List(root.clone()),
            Job::FindPacks(packs::roots(&prefs.icon_folders)),
        ];
        let pack = match prefs.icons.as_str() {
            packs::QUILL => Pack::Quill,
            key => {
                jobs.extend(
                    key.strip_prefix(packs::THEME)
                        .map(|json| Job::LoadPack(json.into())),
                );
                Pack::Material
            }
        };
        let ide = Self {
            root,
            sidebar: Sidebar::new(look::SIDEBAR_WIDTH),
            expanded: HashSet::new(),
            listed: HashMap::new(),
            dir_stamps: HashMap::new(),
            buffers: vec![],
            active: None,
            polling: false,
            closing: None,
            quitting: false,
            prefs,
            pack,
            offered: vec![],
            pack_error: None,
            settings_page: None,
            settings_query: String::new(),
            settings_menu: None,
            pointer: None,
            hovered: None,
            reduced_motion: false,
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
        if let UiAction::InputChanged { id, value } = action {
            if id == SETTINGS_SEARCH {
                self.settings_query = value.clone();
            }
            return vec![];
        }
        let UiAction::Clicked { id } = action else {
            return vec![];
        };
        if !id.starts_with("close:") {
            self.closing = None;
        }
        if let Some(jobs) = self.settings_clicked(id) {
            return jobs;
        }
        if id == TREE_TOGGLE {
            self.sidebar.toggle();
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

    /// Settings' own clicks: open and close, pick a page, and change a setting, which is kept
    /// at once. `None` when `id` is none of them. A select reports its trigger, `set:{name}`,
    /// then a choice, `set:{name}={value}`.
    fn settings_clicked(&mut self, id: &str) -> Option<Vec<Job>> {
        if id == SETTINGS_OPEN {
            self.settings_page = Some(prefs::FIRST.into());
            self.settings_query.clear();
            return Some(vec![Job::FindPacks(packs::roots(&self.prefs.icon_folders))]);
        }
        self.settings_page.as_ref()?;
        if id == SETTINGS_CLOSE {
            self.settings_page = None;
            self.settings_menu = None;
            return Some(vec![]);
        }
        if let Some(page) = id.strip_prefix("settings:page:") {
            self.settings_page = Some(page.to_owned());
            self.settings_query.clear();
            return Some(vec![]);
        }
        let rest = id.strip_prefix("set:")?;
        if let Some(menu) = rest.strip_suffix(":dismiss") {
            if self.settings_menu.as_deref() == Some(menu) {
                self.settings_menu = None;
            }
            return Some(vec![]);
        }
        let (name, value) = match rest.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None if Prefs::is_switch(rest) => (rest, None),
            // A select's trigger opens its menu, or shuts it.
            None => {
                let open = self.settings_menu.as_deref() == Some(rest);
                self.settings_menu = (!open).then(|| rest.to_owned());
                return Some(vec![]);
            }
        };
        self.settings_menu = None;
        if !self.prefs.set(name, value) {
            return Some(vec![]);
        }
        let mut jobs = vec![Job::SavePrefs(self.prefs.clone())];
        if name == "icons" {
            self.pack_error = None;
            match self.prefs.icons.as_str() {
                packs::QUILL => self.pack = Pack::Quill,
                packs::MATERIAL => self.pack = Pack::Material,
                key => jobs.extend(
                    key.strip_prefix(packs::THEME)
                        .map(|j| Job::LoadPack(j.into())),
                ),
            }
        }
        Some(jobs)
    }

    /// A folder to look for icon themes in, from Settings' "Add folder…".
    pub fn add_icon_folder(&mut self, folder: PathBuf) -> Vec<Job> {
        if !self.prefs.icon_folders.contains(&folder) {
            self.prefs.icon_folders.push(folder);
        }
        vec![
            Job::SavePrefs(self.prefs.clone()),
            Job::FindPacks(packs::roots(&self.prefs.icon_folders)),
        ]
    }

    /// Move what moves on its own: the tree's springs and its peek.
    pub fn tick(&mut self) {
        let now = std::time::Instant::now();
        // The pace of limn's own opening and closing slots, as quill's.
        let motion = limn::SpringMotion::settling_in(std::time::Duration::from_millis(
            u64::from(limn::tokens().motion.move_ms) * 3 / 2,
        ));
        let pointer = quill_components::sidebar::Pointer {
            at: self.pointer,
            on_toggle: self.hovered.as_deref() == Some(TREE_TOGGLE),
            toggle_zone: limn::Rect::new(0.0, 0.0, 48.0, look::TOP_BAR),
            held: self.settings_page.is_some(),
        };
        self.sidebar
            .tick(&pointer, motion, self.reduced_motion, now);
    }

    pub fn animating(&self) -> bool {
        self.sidebar.animating(std::time::Instant::now())
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
            Loaded::Packs(offered) => self.offered = offered,
            Loaded::PrefsSaved => {}
            // Only the theme still chosen is put in use.
            Loaded::Pack(json, theme) => {
                if self.prefs.icons.strip_prefix(packs::THEME) == json.to_str() {
                    match theme {
                        Ok(theme) => self.pack = Pack::Theme(Box::new(theme)),
                        Err(error) => self.pack_error = Some(error),
                    }
                }
            }
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
    let with_line = || Element::row(vec![tree(ide, below), vline(below)]);
    let mut body: Vec<Element> = ide.sidebar.docked(with_line(), below).into_iter().collect();
    let width = window.width - ide.sidebar.docked_width();
    body.push(editor(ide, width, below).flex(1.0));
    let body = Element::row(body).size(Some(window.width), Some(below));
    // Shut, the tree peeks over the editor under the pointer, as quill's sidebar does.
    let body = ide
        .sidebar
        .peeking(body, with_line, Size::new(window.width, below));
    let root = Element::column(vec![
        top_bar(ide),
        hline(window.width, look::pane_border()),
        body,
    ])
    .size(Some(window.width), Some(window.height))
    .background(look::palette().surface);
    match &ide.settings_page {
        Some(page) => settings::dialog(
            root,
            prefs::pages(&ide.prefs, ide.settings_menu.as_deref(), &ide.offered),
            prefs::groups(),
            page,
            &ide.settings_query,
            window,
        ),
        None => root,
    }
}

/// The tree's toggle, then the open files as quill's terminal tabs: pills, the one on screen
/// filled, with a dot while a file has changes not saved; Settings at the end.
fn top_bar(ide: &Ide) -> Element {
    let open = ide.sidebar.open;
    let mut row = vec![
        hgap(9.0),
        toggle(
            TREE_TOGGLE,
            icons::of(icons::SIDEBAR),
            if open { "Hide files" } else { "Show files" },
            open,
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
            look::palette(),
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
    let pills = Element::row(row).gap(4.0).flex(1.0).clip_children();
    let settings = tool_surface(
        SETTINGS_OPEN,
        icons::of(icons::SETTINGS),
        "Settings",
        26.0,
        None,
        look::tool_hover(),
        Some("Ctrl+,"),
    );
    Element::row(vec![pills, settings.centered_y(), hgap(8.0)])
        .gap(4.0)
        .size(None, Some(look::TOP_BAR))
}

/// The folder's files as git sees them, folders first, each folder listed when first opened,
/// with lines from each to its folder and the icons of the pack in use.
fn tree(ide: &Ide, height: f32) -> Element {
    let mut header = vec![
        hgap(8.0),
        muted(name(&ide.root)).font_size(SMALL).centered_y(),
    ];
    if let Some(error) = &ide.pack_error {
        header.push(Element::spacer().flex(1.0));
        let warning = limn::Tooltip::new(
            Element::icon(icons::of(icons::WARNING))
                .foreground(look::awaiting())
                .size(Some(14.0), Some(14.0)),
            "tree:pack-error",
            format!("The icon theme did not load: {error}"),
            look::palette(),
        );
        header.push(warning.render().centered_y());
        header.push(hgap(8.0));
    }
    let mut visible = vec![];
    tree_rows(ide, &ide.root, 0, &mut visible);
    let depths: Vec<usize> = visible.iter().map(|(_, depth)| *depth).collect();
    let shown = ide.active_buffer().map(|b| b.path.as_path());
    let palette = ComponentPalette {
        hover: look::row_hover(),
        selected: look::selected(),
        ..look::palette()
    };
    let width = look::SIDEBAR_WIDTH - 12.0;
    // The muted ink at about a third: the divider's gray is lost on the sidebar's fill.
    let muted = look::muted();
    let guide = limn::Color::rgba(muted.r, muted.g, muted.b, 80);
    let mut rows = vec![Element::row(header).size(None, Some(ROW))];
    for ((entry, _), guides) in visible.iter().zip(limn::guides_for_depths(&depths)) {
        let open = entry.dir && ide.expanded.contains(&entry.path);
        let (icon, ink) = ide.pack.icon(&entry.path, entry.dir, open);
        let icon = Element::icon(icon)
            .foreground(ink)
            .size(Some(16.0), Some(16.0))
            .centered();
        let kind = if entry.dir { "dir" } else { "file" };
        let id = format!("{kind}:{}", entry.path.display());
        let title = text(name(&entry.path)).centered_y();
        let row = TreeRow::new(|c| Element::clickable(id, c), icon, title, guides, palette)
            .height(ROW)
            .width(width)
            .guide_color(guide)
            .selected(shown == Some(entry.path.as_path()));
        rows.push(row.render().radius(6.0));
    }
    Element::scroll("tree:scroll", Element::column(rows).padding(6.0))
        .size(Some(look::SIDEBAR_WIDTH), Some(height))
        .background(look::sidebar())
        .clip_children()
}

/// The rows on show, depth first, with their depths.
fn tree_rows<'a>(ide: &'a Ide, dir: &Path, depth: usize, rows: &mut Vec<(&'a Entry, usize)>) {
    for entry in ide.listed.get(dir).into_iter().flatten() {
        rows.push((entry, depth));
        if entry.dir && ide.expanded.contains(&entry.path) {
            tree_rows(ide, &entry.path, depth + 1, rows);
        }
    }
}

/// A line over the editor that says what the person must decide, with its button.
fn notice(words: String, ink: limn::Color, button: Option<Element>, width: f32) -> Element {
    let mut row = vec![
        hgap(12.0),
        text(words).foreground(ink).centered_y(),
        Element::spacer().flex(1.0),
    ];
    row.extend(button.map(Element::centered_y));
    row.push(hgap(6.0));
    Element::column(vec![
        Element::row(row).gap(8.0).size(None, Some(NOTICE - 1.0)),
        hline(width, look::pane_border()),
    ])
}

fn notices(ide: &Ide, width: f32) -> Vec<Element> {
    let mut shown = vec![];
    if ide.quitting {
        let files = match ide.unsaved() {
            1 => "1 file has".to_owned(),
            n => format!("{n} files have"),
        };
        let words = format!("{files} unsaved changes: close the window again to discard them.");
        shown.push(notice(words, look::palette().text, None, width));
    }
    let Some((i, buffer)) = ide.active.zip(ide.active_buffer()) else {
        return shown;
    };
    if let Some(error) = &buffer.error {
        shown.push(notice(
            format!("Not saved: {error}"),
            look::failed(),
            None,
            width,
        ));
    }
    if buffer.stale {
        let reload = keyed_button(
            &format!("reload:{i}"),
            "Reload",
            "",
            false,
            true,
            look::palette(),
        );
        let words = "Changed on disk. Ctrl+S keeps yours; Reload takes the disk's.";
        shown.push(notice(
            words.into(),
            look::palette().text,
            Some(reload),
            width,
        ));
    }
    if ide.closing.as_ref() == Some(&buffer.path) {
        let words = "Unsaved changes. Close it again to discard them, or save them with Ctrl+S.";
        shown.push(notice(words.into(), look::palette().text, None, width));
    }
    shown
}

fn editor(ide: &Ide, width: f32, height: f32) -> Element {
    let note = |s: &str| {
        let fill = || Element::spacer().flex(1.0);
        Element::column(vec![fill(), muted(s).centered_x(), fill()]).flex(1.0)
    };
    let mut column = notices(ide, width);
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
            // With Vim keys, a file opens in Normal mode: reading, until `i`.
            CodeEditor::new(buffer.editor_id(), editor.clone())
                .palette(look::palette())
                .language(limn::CodeLanguage::from_code_fence(ext))
                .font_size(ide.prefs.font_size as f32)
                .vim_mode(ide.prefs.vim)
                .soft_wrap(ide.prefs.wrap)
                .minimap(ide.prefs.minimap)
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
        let (mut ide, jobs) = Ide::new("/r".into(), Prefs::default());
        assert_eq!(jobs[0], Job::List("/r".into()));
        ide.reduced_motion = true;
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
    fn settings_change_the_icons_and_are_kept_at_once() {
        let mut ide = with_files();
        assert!(
            click(&mut ide, "set:vim").is_empty(),
            "shut, Settings take no clicks"
        );
        let jobs = click(&mut ide, SETTINGS_OPEN);
        assert!(matches!(jobs.as_slice(), [Job::FindPacks(_)]));
        click(&mut ide, "set:icons");
        assert_eq!(ide.settings_menu.as_deref(), Some("icons"));
        let jobs = click(&mut ide, "set:icons=theme:/t/theme.json");
        assert_eq!(ide.settings_menu, None);
        assert!(
            matches!(&jobs[..], [Job::SavePrefs(_), Job::LoadPack(p)] if p == Path::new("/t/theme.json"))
        );
        // A theme that comes late, after another was chosen, is not put in use.
        click(&mut ide, "set:icons=quill");
        ide.loaded(Loaded::Pack("/t/theme.json".into(), Ok(Theme::default())));
        assert!(matches!(ide.pack, Pack::Quill));
        let jobs = click(&mut ide, "set:vim");
        assert!(!ide.prefs.vim && matches!(&jobs[..], [Job::SavePrefs(p)] if !p.vim));
        click(&mut ide, SETTINGS_CLOSE);
        assert_eq!(ide.settings_page, None);
    }

    #[test]
    fn the_shut_tree_peeks_from_its_toggle_and_goes_once_the_pointer_left() {
        let mut ide = with_files();
        let on_toggle = |ide: &mut Ide| {
            ide.hovered = Some(TREE_TOGGLE.into());
            ide.pointer = Some(Point::new(20.0, 18.0));
        };
        let away = |ide: &mut Ide| {
            ide.hovered = None;
            ide.pointer = Some(Point::new(700.0, 300.0));
        };
        on_toggle(&mut ide);
        click(&mut ide, TREE_TOGGLE);
        ide.tick();
        // Hidden by a click, it waits for the pointer to leave its toggle first.
        assert!(!ide.sidebar.peek);
        away(&mut ide);
        ide.tick();
        assert!(!ide.sidebar.peek);
        on_toggle(&mut ide);
        ide.tick();
        assert!(ide.sidebar.peek);
        let window = Size::new(1408.0, 844.0);
        let frame = UiTree::new(view(&ide, window)).layout(window);
        assert!(frame.regions.iter().any(|r| r.id == "file:/r/a.rs"));
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
        ide.tick();
        let frame = UiTree::new(view(&ide, window)).layout(window);
        assert!(!frame.regions.iter().any(|r| r.id == "file:/r/a.rs"));
        assert!(frame.regions.iter().any(|r| r.id == "close:0"));
        // A stale file offers to take the disk's text.
        ide.buffers[0].stale = true;
        let frame = UiTree::new(view(&ide, window)).layout(window);
        assert!(frame.regions.iter().any(|r| r.id == "reload:0"));
    }
}
