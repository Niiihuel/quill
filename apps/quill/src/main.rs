//! quill's window: limn paints quill-ui's view; quill-engine runs the agents.
//! `quill [folder]` opens new sessions in `folder` (the current directory without one).

use eframe::egui;
use limn::{Element, Size, UiTree};
use quill_engine::Engine;
use quill_model::{Event, Intent};
use quill_ui::{Store, Ui};
use std::sync::{Arc, Mutex};

struct App {
    tree: UiTree,
    store: Store,
    ui: Ui,
    engine: Engine,
    /// Events the engine reported since the last frame.
    inbox: Arc<Mutex<Vec<Event>>>,
    /// The folder the person chose in the platform's dialog, picked up on the next frame.
    picked: Arc<Mutex<Option<std::path::PathBuf>>>,
    fonts_attached: bool,
}

impl App {
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
        let events = std::mem::take(&mut *self.inbox.lock().unwrap());
        for event in events {
            let intents = self.ui.applied(&event);
            self.store.apply(event);
            self.dispatch(intents);
        }
        if let Some(folder) = self.picked.lock().unwrap().take() {
            self.ui.cwd = folder;
        }
        limn_egui::normalize_keyboard_input(&ctx);
        let area = ui.max_rect();
        let window = Size::new(area.width(), area.height());

        self.tree
            .set_root(quill_ui::view(&self.store, &self.ui, window));
        self.tree.layout(window);
        let mut actions = vec![];
        for event in limn_egui::events(&ctx, area.min) {
            actions.extend(self.tree.handle_event(event));
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
            // The editor keeps its own text; a sent message empties it.
            if matches!(&action, limn::UiAction::Submitted { id, value } if id == quill_ui::view::COMPOSER && !value.trim().is_empty())
            {
                self.tree.set_input_value(quill_ui::view::COMPOSER, "");
            }
            let intents = quill_ui::update(&self.store, &mut self.ui, &action);
            self.dispatch(intents);
        }
        self.tree
            .set_root(quill_ui::view(&self.store, &self.ui, window));
        let frame = self.tree.layout(window);
        limn_egui::paint_at(ui, &frame, area.min);
        limn_egui::set_cursor(&ctx, self.tree.cursor());
        if let Some(after) = self.tree.animation_repaint_after() {
            ctx.request_repaint_after(after);
        }
    }
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
            limn_egui::install_fonts(&creation.egui_ctx);
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
                fonts_attached: false,
            }))
        }),
    )
}
