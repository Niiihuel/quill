//! quill-ide's window: limn paints `ide::view`; listing folders and reading files run beside it.
//! `quill-ide [folder | file]` opens the folder (a file's own, and the file), the current
//! directory without one.

mod disk;
mod ide;

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
        let dark = ctx.system_theme() != Some(egui::Theme::Light);
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
        self.tree.set_root(ide::view(&self.ide, window));
        self.tree.layout(window);
        let mut actions = vec![];
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
            let jobs = self.ide.update(&action);
            self.run(jobs);
        }
        self.tree.set_root(ide::view(&self.ide, window));
        let frame = self.tree.layout(window);
        limn_egui::paint_at(ui, &frame, area.min);
        limn_egui::set_cursor(&ctx, self.tree.cursor());
        if let Some(after) = self.tree.animation_repaint_after() {
            ctx.request_repaint_after(after);
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
            let (mut ide, mut jobs) = ide::Ide::new(root);
            jobs.extend(file.map_or(vec![], |f| ide.open(f)));
            let app = App {
                tree: UiTree::new(Element::spacer()),
                ide,
                inbox: Arc::default(),
                ctx: creation.egui_ctx.clone(),
                fonts_attached: false,
                pinned_theme: std::env::var_os("QUILL_THEME").is_some(),
                polled: std::time::Instant::now(),
            };
            app.run(jobs);
            Ok(Box::new(app))
        }),
    )
}
