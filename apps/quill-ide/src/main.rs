//! quill-ide's window: limn paints `ide::view`; listing folders and reading files run beside it.
//! `quill-ide [folder | file]` opens the folder (a file's own, and the file), the current
//! directory without one.

mod disk;
mod ide;
mod packs;
mod prefs;

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
        for event in limn_egui::events(&ctx, area.min) {
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
        let frame = self.tree.layout(window);
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
            let (mut ide, mut jobs) = ide::Ide::new(root, prefs::load());
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
            };
            app.run(jobs);
            Ok(Box::new(app))
        }),
    )
}
