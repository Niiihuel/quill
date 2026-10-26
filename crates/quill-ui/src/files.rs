//! The Files pane: the session's folder as quill-ide's tree (`quill_components::tree`), its
//! files opened in the document pane, its listing and file operations run by the engine
//! (`Intent::ListDir`, `Intent::FileOp`).

use crate::{view, Pane, Store, Ui};
use limn::UiAction;
use quill_components::tree::{self, FileTree, Outcome, TreeKey};
use quill_model::{AgentEvent, Event, Intent, SessionId};
use std::path::Path;

/// The engine's intents for the tree's jobs; an operation runs in the session's own folder.
fn intents(session: SessionId, jobs: Vec<tree::Job>) -> Vec<Intent> {
    jobs.into_iter()
        .map(|job| match job {
            tree::Job::List(dir) => Intent::ListDir { session, dir },
            tree::Job::File(_, op) => Intent::FileOp { session, op },
        })
        .collect()
}

/// Open or close the pane. Opening it lists again what the session's tree had listed: the
/// agent may have changed the folder since. A session with no tree gets one (`shown`).
fn toggle(ui: &mut Ui) -> Vec<Intent> {
    if ui.pane_open(Pane::Files) {
        ui.close_pane(Pane::Files);
        return vec![];
    }
    let Some(session) = ui.selected else {
        return vec![];
    };
    ui.open_pane(Pane::Files);
    match ui.files.get(&session) {
        Some(tree) => intents(session, tree.relist()),
        None => vec![],
    }
}

/// The tree of the session on screen, made and listed when the pane is open and it has none.
pub(crate) fn shown(store: &Store, ui: &mut Ui) -> Vec<Intent> {
    if !ui.pane_open(Pane::Files) {
        return vec![];
    }
    let Some(session) = ui.selected.and_then(|s| store.session(s)) else {
        return vec![];
    };
    if ui.files.contains_key(&session.id) {
        return vec![];
    }
    let (tree, jobs) = FileTree::new(session.cwd.clone());
    ui.files.insert(session.id, tree);
    intents(session.id, jobs)
}

/// Whether what is at `at` has changes the document pane has not saved, which the tree
/// neither moves nor trashes under it.
fn busy(document: Option<&crate::Document>, session: SessionId, root: &Path, at: &Path) -> bool {
    document.is_some_and(|d| d.session == session && d.dirty && root.join(&d.path).starts_with(at))
}

/// What the tree asks of the window: its jobs, the document moved with what moved or let go
/// with what went to the trash, the focus, the clipboard, and a file to open.
fn apply(ui: &mut Ui, session: SessionId, root: &Path, outcome: Outcome) -> Vec<Intent> {
    let mut intents = intents(session, outcome.jobs);
    if let Some(document) = ui.document.as_mut().filter(|d| d.session == session) {
        for (old, new) in &outcome.moved {
            let at = root.join(&document.path);
            if let Some(Some(moved)) = quill_files::moved(&at, old, Some(new)) {
                if let Ok(path) = moved.strip_prefix(root) {
                    document.path = path.to_string_lossy().into_owned();
                }
            }
        }
    }
    if let Some(gone) = &outcome.trashed {
        let open = (ui.document.as_ref())
            .is_some_and(|d| d.session == session && root.join(&d.path).starts_with(gone));
        if open {
            ui.document = None;
            ui.close_pane(Pane::Document);
        }
    }
    if outcome.copy.is_some() {
        ui.files_copy = outcome.copy;
    }
    if outcome.focus.is_some() {
        ui.files_focus = outcome.focus;
    }
    if outcome.leave {
        ui.files_focus = Some(view::COMPOSER.into());
    }
    if let Some(path) = (outcome.open.as_deref()).and_then(|f| f.strip_prefix(root).ok()) {
        let id = format!("doc:open:{}", path.display());
        intents.extend(crate::document_clicked(ui, &id).unwrap_or_default());
    }
    intents
}

/// The pane's own clicks and its naming dialog's typing; `None` when `action` is none of them.
pub(crate) fn action(ui: &mut Ui, action: &UiAction) -> Option<Vec<Intent>> {
    if matches!(action, UiAction::Clicked { id } if id == view::FILES_TOGGLE) {
        return Some(toggle(ui));
    }
    if !ui.pane_open(Pane::Files) {
        return None;
    }
    let session = ui.selected?;
    let document = ui.document.as_ref();
    let tree = ui.files.get_mut(&session)?;
    let root = tree.root.clone();
    let busy = |at: &Path| busy(document, session, &root, at);
    let outcome = match action {
        UiAction::Clicked { id } => tree.clicked(id, busy)?,
        UiAction::InputChanged { id, value } if tree.typed(id, value) => Outcome::default(),
        UiAction::Submitted { id, .. } if id == tree::NAME_FIELD => {
            tree.clicked("name:go", busy)?
        }
        _ => return None,
    };
    Some(apply(ui, session, &root, outcome))
}

/// A key on a row of the tree, as quill-ide's take them (the arrows and nvim-tree's letters);
/// `None` when the focus is not on one, or the tree does not take the key.
pub fn key(ui: &mut Ui, focused: Option<&str>, event: &limn::UiEvent) -> Option<Vec<Intent>> {
    let row = focused.filter(|f| tree::is_row(f) && ui.pane_open(Pane::Files))?;
    let key = match event {
        limn::UiEvent::Key {
            key, ctrl: false, ..
        } => match key {
            limn::Key::ArrowUp => TreeKey::Up,
            limn::Key::ArrowDown => TreeKey::Down,
            limn::Key::ArrowLeft => TreeKey::Left,
            limn::Key::ArrowRight => TreeKey::Right,
            limn::Key::Escape => TreeKey::Escape,
            _ => return None,
        },
        limn::UiEvent::Text(typed) => match typed.as_str() {
            "k" => TreeKey::Up,
            "j" => TreeKey::Down,
            "h" => TreeKey::Left,
            "l" => TreeKey::Right,
            "a" => TreeKey::New,
            "r" => TreeKey::Rename,
            "d" => TreeKey::Trash,
            "y" => TreeKey::Copy,
            "x" => TreeKey::Cut,
            "p" => TreeKey::Paste,
            _ => return None,
        },
        _ => return None,
    };
    let session = ui.selected?;
    let document = ui.document.as_ref();
    let tree = ui.files.get_mut(&session)?;
    let root = tree.root.clone();
    let outcome = tree.key(row, key, |at| busy(document, session, &root, at))?;
    Some(apply(ui, session, &root, outcome))
}

/// What the engine reported for the trees: a folder listed, an operation done, and a turn
/// finished, after which the agent may have changed the folder.
pub(crate) fn heard(ui: &mut Ui, event: &Event) -> Vec<Intent> {
    match event {
        Event::Listed {
            session,
            dir,
            entries,
            stamp,
        } => {
            if let Some(tree) = ui.files.get_mut(session) {
                tree.listed(dir.clone(), entries.clone(), *stamp);
            }
            vec![]
        }
        Event::FileOpDone {
            session,
            op,
            result,
        } => {
            let Some(tree) = ui.files.get_mut(session) else {
                return vec![];
            };
            let root = tree.root.clone();
            let outcome = tree.done(op.clone(), result.clone());
            apply(ui, *session, &root, outcome)
        }
        Event::Agent {
            session,
            event: AgentEvent::TurnFinished { .. },
            ..
        } => match ui.files.get(session) {
            Some(tree) => intents(*session, tree.relist()),
            None => vec![],
        },
        _ => vec![],
    }
}
