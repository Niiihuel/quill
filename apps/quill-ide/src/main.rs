//! quill-ide's window: limn paints `ide::view`; listing folders and reading files run beside it.
//! `quill-ide [folder | file]` opens the folder (a file's own, and the file), the current
//! directory without one.

mod disk;
mod find;
mod grep;
mod ide;
mod marks;
mod packs;
mod prefs;
mod quick;
mod session;

use eframe::egui;
use limn::{Element, Size, UiTree};
use quill_components::look;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct App {
    tree: UiTree,
    ide: ide::Ide,
    /// What the disk found since the last frame.
    inbox: Arc<Mutex<Vec<ide::Loaded>>>,
    ctx: egui::Context,
    fonts_attached: bool,
    /// `QUILL_THEME` pins the theme over the system's, for demos and captures.
    pinned_theme: bool,
    /// When the disk was last looked at for files and folders changed by others.
    polled: std::time::Instant,
    /// A folder of icon themes chosen in the platform's dialog, picked up on the next frame.
    picked: Arc<Mutex<Option<PathBuf>>>,
    /// What was open when it was last kept.
    kept: session::Session,
}

impl App {
    fn run(&self, jobs: Vec<ide::Job>) {
        for job in jobs {
            let (inbox, ctx) = (self.inbox.clone(), self.ctx.clone());
            std::thread::spawn(move || {
                let loaded = disk::run(job);
                inbox.lock().unwrap().push(loaded);
                ctx.request_repaint();
            });
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
        for loaded in std::mem::take(&mut *self.inbox.lock().unwrap()) {
            let jobs = self.ide.loaded(loaded);
            self.run(jobs);
        }
        let jobs = self.ide.marks_due();
        self.run(jobs);
        // What an agent or another editor writes shows within a second.
        if self.polled.elapsed() >= POLL {
            self.polled = std::time::Instant::now();
            let jobs = self.ide.poll();
            self.run(jobs);
        }
        // The window closes with unsaved changes only when asked twice.
        if ctx.input(|i| i.viewport().close_requested())
            && self.ide.unsaved() > 0
            && !self.ide.quitting
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.ide.quitting = true;
        }
        if let Some(folder) = self.picked.lock().unwrap().take() {
            let jobs = self.ide.add_icon_folder(folder);
            self.run(jobs);
        }
        let dark = match self.ide.prefs.theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => ctx.system_theme() != Some(egui::Theme::Light),
        };
        if !self.pinned_theme && dark != look::is_dark() {
            ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            look::set_dark(dark);
        }
        // A family chosen in Settings: laid out and painted in from the next frame on, when
        // the tree measures with it again.
        if let Some(fonts) = self.ide.fonts_due.take() {
            limn::set_fonts(fonts);
            limn_egui::install_fonts(&ctx);
            self.fonts_attached = false;
            ctx.request_repaint();
        }
        // Context menus read as quill's other menus, in the theme in force.
        self.tree.set_context_menu_palette(look::menu_palette());
        self.tree
            .set_context_menu_sizing(quill_components::pieces::menu_sizing(240.0));
        limn_egui::normalize_keyboard_input(&ctx);
        let area = ui.max_rect();
        let window = Size::new(area.width(), area.height());
        // The tree peeks while the pointer is on its toggle or the window's edge.
        self.ide.pointer = ctx
            .input(|i| i.pointer.hover_pos())
            .map(|p| limn::Point::new(p.x - area.min.x, p.y - area.min.y));
        self.ide.hovered = self.tree.hovered_id().map(str::to_owned);
        self.ide.vim_mode = self
            .ide
            .editor_id()
            .and_then(|id| self.tree.code_editor_vim_mode(&id));
        self.ide.tick();
        self.ide.vim_mode = self
            .ide
            .editor_id()
            .and_then(|id| self.tree.code_editor_vim_mode(&id));
        self.tree.set_root(ide::view(&self.ide, window));
        self.tree.layout(window);
        let mut actions = vec![];
        // Ctrl+, opens Settings, as quill's; limn has no key for it.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Comma)) {
            actions.push(limn::UiAction::Clicked {
                id: quill_components::settings::SETTINGS_OPEN.into(),
            });
        }
        let focused = self.tree.focused_id().map(str::to_owned);
        for event in limn_egui::events(&ctx, area.min) {
            // Opening by name and finding in every file: the arrows move the row Enter opens.
            if self.ide.quick.is_some() || self.ide.grep.is_some() {
                if let limn::UiEvent::Key {
                    key: key @ (limn::Key::ArrowUp | limn::Key::ArrowDown),
                    ..
                } = &event
                {
                    self.ide
                        .step_chosen(if *key == limn::Key::ArrowUp { -1 } else { 1 });
                    continue;
                }
            }
            // In the find bar, Escape closes it, as its own close does.
            if focused.as_deref().is_some_and(|f| f.starts_with("find:"))
                && matches!(
                    &event,
                    limn::UiEvent::Key {
                        key: limn::Key::Escape,
                        ..
                    }
                )
            {
                actions.push(limn::UiAction::Clicked {
                    id: "find:dismiss".into(),
                });
                continue;
            }
            // `/` in Vim's Normal mode finds, as in Vim.
            let editor = self.ide.editor_id();
            if matches!(&event, limn::UiEvent::Text(typed) if typed == "/")
                && self.ide.vim_mode == Some(limn::VimMode::Normal)
                && focused.is_some()
                && focused == editor
            {
                actions.push(limn::UiAction::Clicked {
                    id: find::FIND_OPEN.into(),
                });
                continue;
            }
            if let limn::UiEvent::Key {
                key,
                ctrl: true,
                shift,
            } = &event
            {
                if let Some(id) = self.ide.shortcut(*key, *shift) {
                    actions.push(limn::UiAction::Clicked { id });
                    continue;
                }
            }
            actions.extend(self.tree.handle_event(event));
        }
        actions.extend(limn_egui::apply_access_requests(&ctx, &mut self.tree));
        for action in actions {
            if let limn::UiAction::CopyRequested { text } = &action {
                ctx.copy_text(text.clone());
            }
            if matches!(&action, limn::UiAction::Clicked { id } if id == prefs::ICON_FOLDER_ADD) {
                // The dialog blocks until the person answers, so it runs beside the window.
                let (picked, ctx) = (self.picked.clone(), ctx.clone());
                std::thread::spawn(move || {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        *picked.lock().unwrap() = Some(folder);
                        ctx.request_repaint();
                    }
                });
            }
            let jobs = self.ide.update(&action);
            self.run(jobs);
        }
        self.ide.tick();
        self.tree.set_root(ide::view(&self.ide, window));
        let mut frame = self.tree.layout(window);
        // What the IDE asked of the tree: a control to take the keys, the caret in view.
        if let Some(id) = self.ide.focus.take() {
            if self.tree.focus(&id) {
                frame = self.tree.layout(window);
            }
        }
        if let Some(text) = self.ide.copy.take() {
            ctx.copy_text(text);
        }
        if let Some(row) = self.ide.reveal_row.take() {
            self.tree.reveal(&row);
            frame = self.tree.layout(window);
        }
        if std::mem::take(&mut self.ide.reveal) {
            if let Some(id) = self.ide.editor_id() {
                if self.tree.reveal_code_caret(&id) {
                    frame = self.tree.layout(window);
                }
            }
        }
        // What is open is kept as it changes, to open again next time.
        let session = self.ide.session();
        if session != self.kept && self.ide.buffers.iter().all(|b| !b.loading) {
            self.kept = session.clone();
            self.run(vec![ide::Job::SaveSession(self.ide.root.clone(), session)]);
        }
        limn_egui::paint_at(ui, &frame, area.min);
        limn_egui::set_cursor(&ctx, self.tree.cursor());
        if let Some(after) = self.tree.animation_repaint_after() {
            ctx.request_repaint_after(after);
        }
        // The tree's springs are the IDE's own, not the tree's.
        if self.ide.animating() {
            ctx.request_repaint();
        }
        ctx.request_repaint_after(POLL);
    }
}

const POLL: std::time::Duration = std::time::Duration::from_secs(1);

fn main() -> eframe::Result {
    let given = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| ".".into());
    let given = std::fs::canonicalize(&given).unwrap_or(given);
    let (root, file) = match given.parent() {
        Some(parent) if given.is_file() => (parent.to_path_buf(), Some(given.clone())),
        _ => (given, None),
    };
    let title = format!(
        "quill-ide · {}",
        root.file_name().unwrap_or_default().to_string_lossy()
    );
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_app_id("quill-ide")
            .with_inner_size([1408.0, 844.0])
            .with_min_inner_size([560.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        "quill-ide",
        options,
        Box::new(move |creation| {
            if let Some(fonts) = quill_components::system_fonts() {
                limn::set_fonts(fonts);
            }
            limn_egui::install_fonts(&creation.egui_ctx);
            // `QUILL_THEME=dark|light`, else the system's, else dark, as quill.
            let dark = match std::env::var("QUILL_THEME").as_deref() {
                Ok("light") => false,
                Ok("dark") => true,
                _ => creation.egui_ctx.system_theme() != Some(egui::Theme::Light),
            };
            creation.egui_ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            look::set_dark(dark);
            let kept = session::load(&root);
            let (mut ide, mut jobs) = ide::Ide::new(root, prefs::load(), kept.clone());
            jobs.extend(file.map_or(vec![], |f| ide.open(f)));
            let app = App {
                tree: UiTree::new(Element::spacer()),
                ide,
                inbox: Arc::default(),
                ctx: creation.egui_ctx.clone(),
                fonts_attached: false,
                pinned_theme: std::env::var_os("QUILL_THEME").is_some(),
                polled: std::time::Instant::now(),
                picked: Arc::default(),
                kept,
            };
            app.run(jobs);
            Ok(Box::new(app))
        }),
    )
}
