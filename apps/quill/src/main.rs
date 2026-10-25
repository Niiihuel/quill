//! quill's window: limn paints quill-ui's view; quill-engine runs the agents.
//! `quill [folder]` opens new sessions in `folder` (the current directory without one).

use eframe::egui;
use limn::{Element, Size, UiTree};
use quill_engine::Engine;
use quill_model::{Event, Intent, SessionId};
use quill_ui::{Pane, Store, TerminalRequest, Ui};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct App {
    tree: UiTree,
    store: Store,
    ui: Ui,
    engine: Engine,
    /// Events the engine reported since the last frame.
    inbox: Arc<Mutex<Vec<Event>>>,
    /// The folder the person chose in the platform's dialog, picked up on the next frame.
    picked: Arc<Mutex<Option<PathBuf>>>,
    /// Files or a folder chosen from "+", to go into the message as @-mentions.
    attached: Arc<Mutex<Vec<PathBuf>>>,
    /// The terminal tabs of each folder, kept while the window lives (features/terminal.md).
    terminals: HashMap<PathBuf, quill_pty::Tabs>,
    fonts_attached: bool,
    /// The session on screen and how much its transcript held last frame.
    transcript: Option<(SessionId, usize, usize)>,
}

impl App {
    /// Start, size and read the shells of the folder on screen while the pane is open, and do
    /// what the person asked of them.
    fn sync_terminal(&mut self, ctx: &egui::Context, window: Size) {
        let folder = quill_ui::folder(&self.store, &self.ui);
        let (cols, rows) = quill_ui::view::terminal_grid(&self.store, &self.ui, window);
        if !self.ui.pane_open(Pane::Terminal) {
            self.ui.terminal.screen = None;
            self.ui.terminal.requests.clear();
            return;
        }
        let pane = &mut self.ui.terminal;
        pane.screen = None;
        let requests = std::mem::take(&mut pane.requests);
        let tabs = self.terminals.entry(folder.clone()).or_default();
        let had = !tabs.is_empty();
        let wake = || {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        };
        for request in requests {
            match request {
                TerminalRequest::New => {
                    if let Err(error) = tabs.open(&folder, cols, rows, wake()) {
                        eprintln!("quill: no terminal in {}: {error}", folder.display());
                    }
                }
                TerminalRequest::Select(tab) => tabs.select(tab),
                TerminalRequest::Close(tab) => tabs.close(tab),
                TerminalRequest::CloseOthers(tab) => tabs.close_others(tab),
                TerminalRequest::Rename(tab, name) => tabs.rename(tab, name),
                TerminalRequest::Clear(tab) => tabs.clear(tab),
                TerminalRequest::Move { from, to } => tabs.move_tab(from, to),
            }
        }
        tabs.reap();
        if tabs.is_empty() && !had {
            if let Err(error) = tabs.open(&folder, cols, rows, wake()) {
                eprintln!("quill: no terminal in {}: {error}", folder.display());
            }
        }
        // Closing the last tab, or `exit` in it, hides the pane; opening it again starts fresh.
        let Some(terminal) = tabs.active_mut() else {
            pane.tabs.clear();
            self.ui.close_pane(Pane::Terminal);
            return;
        };
        terminal.resize(cols, rows);
        pane.screen = Some(terminal.screen());
        pane.tabs = tabs
            .list()
            .into_iter()
            .map(|tab| quill_ui::TerminalTab {
                title: tab.title,
                busy: tab.busy,
                named: tab.named,
            })
            .collect();
        pane.active = tabs.active();
        pane.closing = pane.closing.filter(|closing| match closing {
            quill_ui::Closing::One(tab) | quill_ui::Closing::Others(tab) => *tab < pane.tabs.len(),
        });
    }

    /// Keys typed while the terminal has the focus go to its shell, not to the window.
    fn typed_into_terminal(&mut self, event: &limn::UiEvent) -> bool {
        if self.tree.focused_id() != Some(quill_ui::view::TERMINAL) {
            return false;
        }
        let bytes = match event {
            limn::UiEvent::Text(text) => text.as_bytes().to_vec(),
            limn::UiEvent::Key { key, shift, ctrl } => {
                match quill_pty::key_bytes(*key, *ctrl, *shift) {
                    Some(bytes) => bytes,
                    // Plain letters arrive again as Text.
                    None => return true,
                }
            }
            _ => return false,
        };
        let folder = quill_ui::folder(&self.store, &self.ui);
        if let Some(terminal) = self.terminals.get_mut(&folder).and_then(|t| t.active_mut()) {
            terminal.write(&bytes);
        }
        true
    }

    /// The session on screen, its number of items and the length of a reply being written.
    fn transcript_mark(&self) -> Option<(SessionId, usize, usize)> {
        let session = self.store.session(self.ui.selected?)?;
        let tail = match session.items.last() {
            Some(quill_ui::Item::Assistant { text, .. }) => text.len(),
            _ => 0,
        };
        Some((session.id, session.items.len(), tail))
    }

    /// Put `prompt` in the composer, its editor and the UI's copy alike, and give it the keys.
    fn set_prompt(&mut self, prompt: String) {
        self.tree.set_input_value(quill_ui::view::COMPOSER, &prompt);
        self.ui.prompt = prompt;
        self.tree.focus(quill_ui::view::COMPOSER);
    }

    /// Ask the platform for files or a folder beside the window; they land in `attached`.
    fn attach(&self, ctx: &egui::Context, folder: bool) {
        let (attached, start, ctx) = (
            self.attached.clone(),
            quill_ui::folder(&self.store, &self.ui),
            ctx.clone(),
        );
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new().set_directory(start);
            let paths = if folder {
                dialog.pick_folder().into_iter().collect()
            } else {
                dialog.pick_files().unwrap_or_default()
            };
            if !paths.is_empty() {
                *attached.lock().unwrap() = paths;
                ctx.request_repaint();
            }
        });
    }

    fn dispatch(&mut self, intents: Vec<Intent>) {
        for intent in intents {
            quill_ui::decided(&mut self.store, &intent);
            self.engine.send(intent);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.fonts_attached {
            limn_egui::attach_fonts(&mut self.tree, &ctx);
            self.fonts_attached = true;
        }
        // Desktop opens a session at its end and follows it while the person stays there.
        let at_end = self
            .transcript
            .is_none_or(|(id, ..)| self.tree.is_scroll_at_end(&format!("transcript:{id}"), 8.0));
        let events = std::mem::take(&mut *self.inbox.lock().unwrap());
        for event in events {
            let intents = self.ui.applied(&event);
            self.store.apply(event);
            self.dispatch(intents);
        }
        let attached = std::mem::take(&mut *self.attached.lock().unwrap());
        if !attached.is_empty() {
            // Paths inside the folder are written from it, as the agent reads them.
            let base = quill_ui::folder(&self.store, &self.ui);
            let mentions: Vec<String> = attached
                .iter()
                .map(|p| format!("@{}", p.strip_prefix(&base).unwrap_or(p).display()))
                .collect();
            let mut prompt = self.ui.prompt.trim_end().to_owned();
            if !prompt.is_empty() {
                prompt.push(' ');
            }
            prompt.push_str(&mentions.join(" "));
            prompt.push(' ');
            self.set_prompt(prompt);
        }
        if let Some(folder) = self.picked.lock().unwrap().take() {
            self.ui.cwd = folder;
        }
        limn_egui::normalize_keyboard_input(&ctx);
        let area = ui.max_rect();
        let window = Size::new(area.width(), area.height());
        self.ui.window = window;
        // New output keeps the terminal at its end only if the person left it there.
        let shown = self.ui.terminal.screen.clone();
        let follow = shown.is_none()
            || self
                .tree
                .is_scroll_at_end(quill_ui::view::TERMINAL_SCROLL, 4.0);
        self.sync_terminal(&ctx, window);

        self.tree
            .set_root(quill_ui::view(&self.store, &self.ui, window));
        self.tree.layout(window);
        let mut actions = vec![];
        let mut focus_terminal = false;
        let was_renaming = self.ui.terminal.renaming.is_some();
        // Ctrl+` shows and hides the terminal, as in Desktop; limn has no key for it.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Backtick)) {
            actions.push(limn::UiAction::Clicked {
                id: quill_ui::view::TERMINAL_TOGGLE.into(),
            });
        }
        for event in limn_egui::events(&ctx, area.min) {
            // Permission keys work wherever the focus is, the composer included.
            // Ctrl+U adds files; a digit picks that row of an open composer menu, as in Desktop.
            if matches!(
                &event,
                limn::UiEvent::Key {
                    key: limn::Key::U,
                    ctrl: true,
                    shift: false
                }
            ) {
                actions.push(limn::UiAction::Clicked {
                    id: quill_ui::view::ATTACH_FILES.into(),
                });
                continue;
            }
            // Ctrl+N starts a new session, as Desktop's sidebar says.
            if matches!(
                &event,
                limn::UiEvent::Key {
                    key: limn::Key::N,
                    ctrl: true,
                    shift: false
                }
            ) {
                actions.push(limn::UiAction::Clicked {
                    id: quill_ui::view::NEW_SESSION.into(),
                });
                continue;
            }
            // Ctrl+Shift+I and Ctrl+Shift+M open the model and mode menus, as in Desktop.
            if let limn::UiEvent::Key {
                key,
                ctrl: true,
                shift: true,
            } = &event
            {
                let menu = match key {
                    limn::Key::I => Some(quill_ui::view::MODEL_MENU),
                    limn::Key::M => Some(quill_ui::view::MODE_MENU),
                    limn::Key::D => Some(quill_ui::view::DIFF_TOGGLE),
                    _ => None,
                };
                if let Some(id) = menu {
                    actions.push(limn::UiAction::Clicked { id: id.into() });
                    continue;
                }
            }
            if let limn::UiEvent::Text(typed) = &event {
                if let Some(id) = quill_ui::menu_digit(&self.store, &self.ui, typed) {
                    actions.push(limn::UiAction::Clicked { id });
                    continue;
                }
            }
            if let limn::UiEvent::Key { key, shift, ctrl } = &event {
                if let Some(id) = quill_ui::shortcut(&self.store, &self.ui, *key, *ctrl, *shift) {
                    actions.push(limn::UiAction::Clicked { id });
                    continue;
                }
                let focused = self.tree.focused_id().map(str::to_owned);
                if quill_ui::terminal_key(&mut self.ui, focused.as_deref(), *key, *ctrl, *shift) {
                    continue;
                }
            }
            if !self.typed_into_terminal(&event) {
                actions.extend(self.tree.handle_event(event));
            }
        }
        actions.extend(limn_egui::apply_access_requests(&ctx, &mut self.tree));
        for action in actions {
            if let limn::UiAction::CopyRequested { text } = &action {
                ctx.copy_text(text.clone());
            }
            if matches!(&action, limn::UiAction::Clicked { id } if id == quill_ui::view::PICK_FOLDER)
            {
                // The dialog blocks until the person answers, so it runs beside the window.
                let (picked, start, ctx) = (self.picked.clone(), self.ui.cwd.clone(), ctx.clone());
                std::thread::spawn(move || {
                    if let Some(folder) = rfd::FileDialog::new().set_directory(start).pick_folder()
                    {
                        *picked.lock().unwrap() = Some(folder);
                        ctx.request_repaint();
                    }
                });
            }
            if let limn::UiAction::Clicked { id } = &action {
                if let Some(words) = quill_ui::copied_text(&self.store, id) {
                    ctx.copy_text(words);
                }
                match id.as_str() {
                    quill_ui::view::ATTACH_FILES => self.attach(&ctx, false),
                    quill_ui::view::ATTACH_FOLDER => self.attach(&ctx, true),
                    quill_ui::view::SLASH if !self.ui.prompt.starts_with('/') => {
                        self.set_prompt(format!("/{}", self.ui.prompt));
                    }
                    quill_ui::view::SEND if !self.ui.prompt.trim().is_empty() => {
                        self.tree.set_input_value(quill_ui::view::COMPOSER, "");
                    }
                    _ => {}
                }
            }
            // The editor keeps its own text; a sent message empties it.
            if matches!(&action, limn::UiAction::Submitted { id, value } if id == quill_ui::view::COMPOSER && !value.trim().is_empty())
            {
                self.tree.set_input_value(quill_ui::view::COMPOSER, "");
            }
            let opened = !self.ui.pane_open(Pane::Terminal);
            let intents = quill_ui::update(&self.store, &mut self.ui, &action);
            self.dispatch(intents);
            // A pane just opened, or a tab just picked or made, takes the keys.
            focus_terminal |= (opened && self.ui.pane_open(Pane::Terminal))
                || self.ui.terminal.requests.iter().any(|r| {
                    !matches!(r, TerminalRequest::Close(_) | TerminalRequest::Move { .. })
                });
        }
        self.sync_terminal(&ctx, window);
        let changed = match (&shown, &self.ui.terminal.screen) {
            (Some(before), Some(now)) => !Arc::ptr_eq(before, now),
            (None, Some(_)) => true,
            _ => false,
        };
        if (follow && changed) || focus_terminal {
            self.tree.scroll_to_end(quill_ui::view::TERMINAL_SCROLL);
        }
        let mark = self.transcript_mark();
        let opened = mark.map(|m| m.0) != self.transcript.map(|m| m.0);
        if let Some((id, ..)) = mark.filter(|_| opened || (at_end && mark != self.transcript)) {
            self.tree.scroll_to_end(&format!("transcript:{id}"));
        }
        self.transcript = mark;
        self.tree
            .set_root(quill_ui::view(&self.store, &self.ui, window));
        let mut frame = self.tree.layout(window);
        // A pill that just turned into a name field takes the keys.
        if !was_renaming && self.ui.terminal.renaming.is_some() {
            self.tree.focus(quill_ui::view::TERMINAL_RENAME);
            frame = self.tree.layout(window);
        }
        // A pane just opened takes the keys; focus needs the laid-out tree.
        if focus_terminal && self.tree.focus(quill_ui::view::TERMINAL) {
            frame = self.tree.layout(window);
        }
        limn_egui::paint_at(ui, &frame, area.min);
        limn_egui::set_cursor(&ctx, self.tree.cursor());
        if let Some(after) = self.tree.animation_repaint_after() {
            ctx.request_repaint_after(after);
        }
    }
}

/// The system's sans and monospace faces, as Desktop's Code surface draws in: it sets
/// `--font-anthropic-sans` to the system fallbacks, which fontconfig resolves (DejaVu on
/// NixOS). `None` keeps limn's bundled faces when fontconfig cannot say.
fn system_fonts() -> Option<limn::Fonts> {
    let face = |pattern: &str| -> Option<limn::Font> {
        let out = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", pattern])
            .output()
            .ok()?;
        let path = String::from_utf8(out.stdout).ok()?;
        limn::Font::new(std::fs::read(path.trim()).ok()?).ok()
    };
    use limn::FontFamily::*;
    let mut fonts = limn::Fonts::bundled();
    for (family, pattern) in [
        (Sans, "sans-serif"),
        (SansMedium, "sans-serif:bold"),
        (SansBold, "sans-serif:bold"),
        (Monospace, "monospace"),
        (MonospaceBold, "monospace:bold"),
    ] {
        fonts = fonts.face(family, face(pattern)?);
    }
    Some(fonts)
}

fn main() -> eframe::Result {
    let cwd = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| ".".into());
    let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("quill")
            .with_app_id("quill")
            .with_inner_size([1408.0, 844.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "quill",
        options,
        Box::new(move |creation| {
            if let Some(fonts) = system_fonts() {
                limn::set_fonts(fonts);
            }
            limn_egui::install_fonts(&creation.egui_ctx);
            quill_ui::look::install();
            let inbox: Arc<Mutex<Vec<Event>>> = Arc::default();
            let (events, ctx) = (inbox.clone(), creation.egui_ctx.clone());
            // The engine's thread wakes the window for every event it reports.
            let engine = Engine::start(move |event| {
                events.lock().unwrap().push(event);
                ctx.request_repaint();
            });
            Ok(Box::new(App {
                tree: UiTree::new(Element::spacer()),
                store: Store::default(),
                ui: Ui {
                    // Tests and demos pin a cheap model; the picker changes it per session.
                    model: std::env::var("QUILL_MODEL").ok(),
                    ..Ui::new(cwd)
                },
                engine,
                inbox,
                picked: Arc::default(),
                attached: Arc::default(),
                terminals: HashMap::new(),
                transcript: None,
                fonts_attached: false,
            }))
        }),
    )
}
