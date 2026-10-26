//! quill's window: limn paints quill-ui's view; quill-engine runs the agents.
//! `quill [folder]` opens new sessions in `folder` (the current directory without one).

mod page;

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
    /// The Chromium chosen in Settings › Browser, kept on the next frame.
    chromium: Arc<Mutex<Option<PathBuf>>>,
    /// Files chosen for a page's file field, sent on the next frame.
    chosen: Arc<Mutex<Option<(SessionId, quill_model::BrowserOp)>>>,
    /// Files or a folder chosen from "+", to go into the message as @-mentions.
    attached: Arc<Mutex<Vec<PathBuf>>>,
    /// The terminal tabs of each folder, kept while the window lives (features/terminal.md).
    terminals: HashMap<PathBuf, quill_pty::Tabs>,
    fonts_attached: bool,
    /// The session on screen and how much its transcript held last frame.
    transcript: Option<(SessionId, usize, usize)>,
    /// The same of the session beside it, in the side pane.
    beside: Option<(SessionId, usize, usize)>,
    /// The motion Settings asked for that is in force.
    applied: Option<quill_model::Motion>,
    /// `QUILL_THEME` pins the theme over Settings, for demos and captures.
    pinned_theme: bool,
    /// Where the browser leaves its pages' frames, presented each frame.
    frames: quill_engine::Frames,
    /// The texture each surface shows from the GPU, as registered with egui's renderer.
    textures: HashMap<String, (egui::TextureId, eframe::wgpu::Texture)>,
    /// The page's share of the window's input.
    page: page::Page,
    /// The tab on screen and its size, as last told to the engine.
    browser_shown: Option<(SessionId, quill_model::BrowserView)>,
    /// What the address bar was last set to.
    address: Option<String>,
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

    /// The page's rectangle in the window and whether it is on top under each position:
    /// what of this frame's input is the page's.
    fn route_page(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
    ) -> (Vec<page::Ask>, Vec<usize>) {
        let view = quill_ui::view::BROWSER_VIEW;
        let frame = self.tree.frame();
        let rect = |r: limn::Rect| {
            egui::Rect::from_min_size(origin + egui::vec2(r.x, r.y), egui::vec2(r.width, r.height))
        };
        let page = frame
            .regions
            .iter()
            .find(|r| r.id == view)
            .map(|r| rect(r.rect));
        let over = |pos: egui::Pos2| {
            let at = limn::Point::new(pos.x - origin.x, pos.y - origin.y);
            frame
                .press_region(at)
                .filter(|r| r.id == view)
                .map(|r| rect(r.rect))
        };
        let focused = self.tree.focused_id() == Some(view);
        let events = ctx.input(|i| i.events.clone());
        self.page.route(&events, over, page, focused)
    }

    /// Carry out what the page asked; the browser's buttons are clicked as the person would.
    fn page_asked(&mut self, ask: page::Ask, window: Size) -> Vec<limn::UiAction> {
        use page::Ask;
        use quill_ui::view::*;
        let clicked = |id: String| vec![limn::UiAction::Clicked { id }];
        let Some(session) = self.ui.selected else {
            return vec![];
        };
        let browser = self
            .store
            .session(session)
            .map(|s| s.browser.clone())
            .unwrap_or_default();
        let Some(tab) = browser.active else {
            return match ask {
                Ask::ToggleBrowser => clicked(BROWSER_TOGGLE.into()),
                _ => vec![],
            };
        };
        let send = |op| Intent::Browser { session, op };
        match ask {
            Ask::Input(input) => {
                self.dispatch(vec![send(quill_model::BrowserOp::Input { tab, input })]);
                vec![]
            }
            Ask::Copy { cut } => {
                let mut ops = vec![send(quill_model::BrowserOp::Copy { tab })];
                if cut {
                    ops.push(send(quill_model::BrowserOp::Input {
                        tab,
                        input: quill_model::BrowserInput::Key {
                            key: "Delete".into(),
                            code: "Delete".into(),
                            key_code: 46,
                            text: String::new(),
                            keys: Default::default(),
                        },
                    }));
                }
                self.dispatch(ops);
                vec![]
            }
            Ask::Focus => {
                self.tree.focus_from_pointer(BROWSER_VIEW);
                vec![]
            }
            Ask::FocusAddress => {
                self.tree.layout(window);
                self.tree.focus(BROWSER_URL);
                self.tree.handle_event(limn::UiEvent::Key {
                    key: limn::Key::A,
                    ctrl: true,
                    shift: false,
                });
                vec![]
            }
            Ask::NewTab => clicked(BROWSER_NEW_TAB.into()),
            Ask::CloseTab => clicked(format!("browser:close:{tab}")),
            Ask::Reload => clicked(BROWSER_RELOAD.into()),
            Ask::ToggleBrowser => clicked(BROWSER_TOGGLE.into()),
            Ask::NextTab(forward) => {
                let at = browser.tabs.iter().position(|t| t.id == tab).unwrap_or(0);
                let count = browser.tabs.len().max(1);
                let next = if forward {
                    (at + 1) % count
                } else {
                    (at + count - 1) % count
                };
                browser
                    .tabs
                    .get(next)
                    .map(|t| clicked(format!("browser:tab:{}", t.id)))
                    .unwrap_or_default()
            }
        }
    }

    /// The address bar shows the tab's address unless the person is typing in it.
    fn sync_address(&mut self) {
        if self.tree.focused_id() == Some(quill_ui::view::BROWSER_URL) {
            return;
        }
        let tab = self
            .ui
            .selected
            .and_then(|s| self.store.session(s))
            .and_then(|s| s.browser.tab().map(|t| (t.id, t.url.clone())));
        let url = match &tab {
            Some((_, url)) if url != "about:blank" => url.clone(),
            _ => String::new(),
        };
        if self.address.as_ref() != Some(&url) {
            self.tree
                .set_input_value(quill_ui::view::BROWSER_URL, url.clone());
            // An address longer than the field shows its start, as a browser's does.
            self.tree.set_input_cursor(quill_ui::view::BROWSER_URL, 0);
            self.address = Some(url);
        }
    }

    /// After painting: the engine hears which tab is on screen and at what size, the pointer
    /// takes the page's shape over it, and an input method follows the page's focus.
    fn page_frame(&mut self, ctx: &egui::Context, frame: &limn::Frame, origin: egui::Pos2) {
        let view = quill_ui::view::BROWSER_VIEW;
        let region = frame.regions.iter().find(|r| r.id == view).map(|r| r.rect);
        let shown = (|| {
            let session = self.ui.selected?;
            let tab = self.store.session(session)?.browser.active?;
            let rect = region.filter(|_| self.ui.pane_open(Pane::Browser))?;
            Some((
                session,
                quill_model::BrowserView {
                    tab,
                    width: rect.width.round() as u32,
                    height: rect.height.round() as u32,
                },
            ))
        })()
        .filter(|(_, view)| view.width > 0 && view.height > 0);
        // While the panes slide, the page keeps its size and takes the new one once they rest.
        let sliding = shown.is_some() && self.ui.animating();
        if shown != self.browser_shown && !sliding {
            let intent = match (shown, self.browser_shown) {
                (Some((session, view)), _) => Intent::Browser {
                    session,
                    op: quill_model::BrowserOp::Show { view: Some(view) },
                },
                (None, Some((session, _))) => Intent::Browser {
                    session,
                    op: quill_model::BrowserOp::Show { view: None },
                },
                (None, None) => unreachable!("they differ"),
            };
            self.engine.send(intent);
            self.browser_shown = shown;
        }
        let Some(rect) = region else {
            return;
        };
        let rect = egui::Rect::from_min_size(
            origin + egui::vec2(rect.x, rect.y),
            egui::vec2(rect.width, rect.height),
        );
        let hovered = ctx
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| rect.contains(p));
        if hovered {
            let cursor = self
                .ui
                .selected
                .and_then(|s| self.store.session(s))
                .map(|s| s.browser.cursor.as_str())
                .unwrap_or_default();
            ctx.set_cursor_icon(match cursor {
                "pointer" => egui::CursorIcon::PointingHand,
                "text" => egui::CursorIcon::Text,
                "move" => egui::CursorIcon::Move,
                "grab" => egui::CursorIcon::Grab,
                "grabbing" => egui::CursorIcon::Grabbing,
                "not-allowed" => egui::CursorIcon::NotAllowed,
                "crosshair" => egui::CursorIcon::Crosshair,
                "col-resize" | "ew-resize" => egui::CursorIcon::ResizeHorizontal,
                "row-resize" | "ns-resize" => egui::CursorIcon::ResizeVertical,
                "wait" | "progress" => egui::CursorIcon::Wait,
                _ => egui::CursorIcon::Default,
            });
        }
        // Text typed through an input method lands in the page.
        if self.tree.focused_id() == Some(view) {
            ctx.output_mut(|output| {
                output.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Normal,
                    rect,
                    cursor_rect: egui::Rect::from_min_size(rect.min, egui::vec2(1.0, 20.0)),
                    should_interrupt_composition: false,
                });
            });
        }
    }

    /// The id egui's renderer draws `texture` by, for the surface `id`: a texture it already
    /// has is drawn again as is, as a page's paints are written into it.
    fn register(
        &mut self,
        render: &eframe::egui_wgpu::RenderState,
        id: &str,
        texture: eframe::wgpu::Texture,
    ) -> egui::TextureId {
        if let Some((known, _)) = self.textures.get(id).filter(|(_, t)| *t == texture) {
            return *known;
        }
        let view = texture.create_view(&Default::default());
        let filter = eframe::wgpu::FilterMode::Linear;
        let mut renderer = render.renderer.write();
        let known = match self.textures.get(id) {
            Some((known, _)) => {
                renderer.update_egui_texture_from_wgpu_texture(
                    &render.device,
                    &view,
                    filter,
                    *known,
                );
                *known
            }
            None => renderer.register_native_texture(&render.device, &view, filter),
        };
        self.textures.insert(id.to_owned(), (known, texture));
        known
    }

    /// The session on screen, its number of items and the length of a reply being written.
    fn transcript_mark(&self, id: Option<SessionId>) -> Option<(SessionId, usize, usize)> {
        let session = self.store.session(id?)?;
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

    /// The theme and the motion Settings chose, in force from this frame: "System" follows the
    /// desktop's theme as it changes, and its motion preference where it can be read.
    fn apply_look(&mut self, ctx: &egui::Context) {
        use quill_model::{Motion, Theme};
        if !self.pinned_theme {
            let dark = match self.ui.settings.theme {
                Theme::Light => false,
                Theme::Dark => true,
                Theme::System => ctx.system_theme() != Some(egui::Theme::Light),
            };
            if dark != quill_ui::look::is_dark() {
                ctx.set_theme(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                quill_ui::look::set_dark(dark);
            }
        }
        let motion = self.ui.settings.motion;
        if self.applied != Some(motion) {
            self.applied = Some(motion);
            let reduced = match motion {
                Motion::Reduced => true,
                Motion::Full => false,
                Motion::System => limn_egui::prefers_reduced_motion().unwrap_or(false),
            };
            self.tree.set_reduced_motion(reduced);
            self.ui.reduced_motion = reduced;
        }
    }

    fn dispatch(&mut self, intents: Vec<Intent>) {
        for intent in intents {
            quill_ui::decided(&mut self.store, &intent);
            self.engine.send(intent);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, host: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.fonts_attached {
            limn_egui::attach_fonts(&mut self.tree, &ctx);
            self.fonts_attached = true;
        }
        // Desktop opens a session at its end and follows it while the person stays there.
        let at_end = self
            .transcript
            .is_none_or(|(id, ..)| self.tree.is_scroll_at_end(&format!("transcript:{id}"), 8.0));
        self.ui.at_end = at_end;
        let beside_at_end = self.beside.is_none_or(|(id, ..)| {
            self.tree
                .is_scroll_at_end(&format!("side:{id}:scroll"), 8.0)
        });
        let events = std::mem::take(&mut *self.inbox.lock().unwrap());
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        for event in events {
            // A session that finished or waits on the person flags quill while it is behind
            // other windows, as Desktop's "Draw attention on notifications".
            let calls = matches!(
                &event,
                Event::Agent {
                    event: quill_model::AgentEvent::TurnFinished { .. },
                    ..
                } | Event::Status {
                    status: quill_model::SessionStatus::Awaiting,
                    ..
                }
            );
            if calls && !focused && self.ui.settings.attention {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                    egui::UserAttentionType::Informational,
                ));
            }
            // What the page had selected goes on the clipboard.
            if let Event::Browser {
                update: quill_model::BrowserUpdate::Copied(text),
                ..
            } = &event
            {
                ctx.copy_text(text.clone());
            }
            // A page's file field: the desktop's dialog, beside the window as it blocks.
            if let Event::Browser {
                session,
                update:
                    quill_model::BrowserUpdate::ChooseFiles {
                        tab,
                        node,
                        multiple,
                    },
            } = &event
            {
                let (chosen, ctx, session, tab, node, multiple) = (
                    self.chosen.clone(),
                    ctx.clone(),
                    *session,
                    *tab,
                    *node,
                    *multiple,
                );
                std::thread::spawn(move || {
                    let dialog = rfd::FileDialog::new();
                    let paths = if multiple {
                        dialog.pick_files().unwrap_or_default()
                    } else {
                        dialog.pick_file().into_iter().collect()
                    };
                    let op = quill_model::BrowserOp::Files { tab, node, paths };
                    *chosen.lock().unwrap() = Some((session, op));
                    ctx.request_repaint();
                });
            }
            // An element picked in the page joins the message being written.
            if let Event::Browser {
                update: quill_model::BrowserUpdate::Selected(block),
                ..
            } = &event
            {
                let mut prompt = self.ui.prompt.trim_end().to_owned();
                if !prompt.is_empty() {
                    prompt.push_str("\n\n");
                }
                prompt.push_str(block);
                self.set_prompt(prompt);
            }
            let intents = self.ui.applied(&event);
            self.store.apply(event);
            self.dispatch(intents);
        }
        self.apply_look(&ctx);
        // Context menus read as quill's other menus, in the theme in force.
        self.tree
            .set_context_menu_palette(quill_ui::look::menu_palette());
        self.tree
            .set_context_menu_sizing(quill_components::pieces::menu_sizing(240.0));
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
        if let Some((session, op)) = self.chosen.lock().unwrap().take() {
            self.engine.send(Intent::Browser { session, op });
        }
        let chromium = self.chromium.lock().unwrap().take();
        if let Some(program) = chromium {
            let mut settings = self.ui.settings.clone();
            settings.browser_program = Some(program.display().to_string());
            self.dispatch(vec![Intent::SaveSettings(settings)]);
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
        // The hidden sidebar peeks while the pointer is on its button or the window's edge.
        self.ui.pointer = ctx
            .input(|i| i.pointer.hover_pos())
            .map(|p| limn::Point::new(p.x - area.min.x, p.y - area.min.y));
        self.ui.hovered = self.tree.hovered_id().map(str::to_owned);
        self.ui.tick(&self.store);
        self.tree
            .set_root(quill_ui::view(&self.store, &self.ui, window));
        self.tree.layout(window);
        let mut actions = vec![];
        let mut focus_terminal = false;
        let was_renaming = self.ui.terminal.renaming.is_some();
        let was_titling = self.ui.title_edit.is_some();
        // Ctrl+, opens Settings, as in Desktop.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Comma)) {
            actions.push(limn::UiAction::Clicked {
                id: quill_ui::view::SETTINGS_OPEN.into(),
            });
        }
        // Ctrl+` shows and hides the terminal, as in Desktop; limn has no key for it.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Backtick)) {
            actions.push(limn::UiAction::Clicked {
                id: quill_ui::view::TERMINAL_TOGGLE.into(),
            });
        }
        // The browser's frames, presented where the view asks for them.
        self.frames.set_scale(ctx.pixels_per_point());
        for (id, frame) in self.frames.take() {
            let size = [frame.width, frame.height];
            match frame.pixels {
                quill_engine::Pixels::Rgba(rgba) => {
                    if let Some(surface) = surface_frame(size, rgba, frame.scale) {
                        limn_egui::present_surface(&ctx, &id, surface);
                    }
                }
                quill_engine::Pixels::Texture(texture) => {
                    if let Some(render) = host.wgpu_render_state() {
                        let texture = self.register(render, &id, texture);
                        limn_egui::present_surface_texture(&ctx, &id, texture, size, frame.scale);
                    }
                }
            }
        }
        let (asks, taken) = self.route_page(&ctx, area.min);
        for ask in asks {
            actions.extend(self.page_asked(ask, window));
        }
        for item in limn_egui::events_indexed(&ctx, area.min) {
            if item
                .source_index
                .is_some_and(|index| taken.contains(&index))
            {
                continue;
            }
            let event = item.event;
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
            // Ctrl+B hides the sidebar and Ctrl+K opens the search, as Desktop's tooltips say;
            // neither reaches past a dialog.
            let dialog = self.ui.deleting.is_some() || self.ui.terminal.closing.is_some();
            if let limn::UiEvent::Key {
                key: key @ (limn::Key::B | limn::Key::K),
                ctrl: true,
                shift: false,
            } = &event
            {
                if dialog {
                    continue;
                }
                let id = if *key == limn::Key::B {
                    quill_ui::view::SIDEBAR_TOGGLE
                } else {
                    quill_ui::view::SEARCH
                };
                actions.push(limn::UiAction::Clicked { id: id.into() });
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
                    // The browser, as Desktop's Ctrl+Shift+B.
                    limn::Key::B => Some(quill_ui::view::BROWSER_TOGGLE),
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
                if let Some(code) = id.strip_prefix(quill_ui::view::COPY_CODE) {
                    ctx.copy_text(code.to_owned());
                }
                // "Open in › File manager" shows the session's folder in the desktop's own.
                if let Some(session) = id
                    .strip_prefix("open-in:files:")
                    .and_then(|s| s.parse().ok())
                    .and_then(|s| self.store.session(s))
                {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(&session.cwd)
                        .spawn();
                }
                // Settings › Browser: the Chromium to run, from the platform's dialog.
                if id == quill_ui::view::BROWSER_PROGRAM_CHOOSE {
                    let (chromium, ctx) = (self.chromium.clone(), ctx.clone());
                    std::thread::spawn(move || {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            *chromium.lock().unwrap() = Some(path);
                            ctx.request_repaint();
                        }
                    });
                }
                // Settings' data folder shows in the desktop's own file manager.
                if id == quill_ui::view::SETTINGS_STORAGE_OPEN {
                    if let Some((path, _)) = &self.ui.storage {
                        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
                    }
                }
                match id.as_str() {
                    quill_ui::view::ATTACH_FILES => self.attach(&ctx, false),
                    quill_ui::view::ATTACH_FOLDER => self.attach(&ctx, true),
                    quill_ui::view::SLASH if !self.ui.prompt.starts_with('/') => {
                        self.set_prompt(format!("/{}", self.ui.prompt));
                    }
                    // Desktop's "See all" opens the plan's usage page.
                    quill_ui::view::USAGE_ALL => {
                        let _ = std::process::Command::new("xdg-open")
                            .arg("https://claude.ai/settings/usage")
                            .spawn();
                    }
                    quill_ui::view::SCROLL_END => {
                        if let Some(session) = self.ui.selected {
                            self.tree.scroll_to_end(&format!("transcript:{session}"));
                        }
                    }
                    quill_ui::view::SEND if !self.ui.prompt.trim().is_empty() => {
                        self.tree.set_input_value(quill_ui::view::COMPOSER, "");
                    }
                    _ => {}
                }
            }
            // An address opened, the page takes the keys, as a browser's does.
            if matches!(&action, limn::UiAction::Submitted { id, .. } if id == quill_ui::view::BROWSER_URL)
            {
                self.address = None;
                self.tree.focus_from_pointer(quill_ui::view::BROWSER_VIEW);
            }
            // The editor keeps its own text; a sent message empties it.
            if let limn::UiAction::Submitted { id, value } = &action {
                let field = [quill_ui::view::COMPOSER, quill_ui::view::SIDE_FIELD]
                    .into_iter()
                    .find(|f| id == f);
                if let Some(field) = field.filter(|_| !value.trim().is_empty()) {
                    self.tree.set_input_value(field, "");
                }
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
        let mark = self.transcript_mark(self.ui.selected);
        let opened = mark.map(|m| m.0) != self.transcript.map(|m| m.0);
        if let Some((id, ..)) = mark.filter(|_| opened || (at_end && mark != self.transcript)) {
            self.tree.scroll_to_end(&format!("transcript:{id}"));
        }
        self.transcript = mark;
        // The session beside opens at its end too, and follows it the same way.
        let mark = self.transcript_mark(self.ui.side_chat);
        let opened = mark.map(|m| m.0) != self.beside.map(|m| m.0);
        if let Some((id, ..)) = mark.filter(|_| opened || (beside_at_end && mark != self.beside)) {
            self.tree.scroll_to_end(&format!("side:{id}:scroll"));
        }
        self.beside = mark;
        self.sync_address();
        self.ui.tick(&self.store);
        self.tree
            .set_root(quill_ui::view(&self.store, &self.ui, window));
        let mut frame = self.tree.layout(window);
        // A pill that just turned into a name field takes the keys.
        if !was_renaming && self.ui.terminal.renaming.is_some() {
            self.tree.focus(quill_ui::view::TERMINAL_RENAME);
            frame = self.tree.layout(window);
        }
        // The title's field takes the keys with its text selected, as Desktop's does.
        if !was_titling && self.ui.title_edit.is_some() {
            self.tree.focus(quill_ui::view::TITLE_FIELD);
            self.tree.handle_event(limn::UiEvent::Key {
                key: limn::Key::A,
                ctrl: true,
                shift: false,
            });
            frame = self.tree.layout(window);
        }
        // A pane just opened takes the keys; focus needs the laid-out tree.
        if focus_terminal && self.tree.focus(quill_ui::view::TERMINAL) {
            frame = self.tree.layout(window);
        }
        limn_egui::paint_at(ui, &frame, area.min);
        limn_egui::set_cursor(&ctx, self.tree.cursor());
        self.page_frame(&ctx, &frame, area.min);
        if let Some(after) = self.tree.animation_repaint_after() {
            ctx.request_repaint_after(after);
        }
        // The effort thumb's spring and the Ultracode field are quill's own, not the tree's.
        if let Some(after) = self.ui.repaint_after() {
            ctx.request_repaint_after(after);
        }
    }
}

/// The local time zone's offset from UTC in seconds, as `date +%z` says it (`-0300`).
fn utc_offset() -> i64 {
    let out = std::process::Command::new("date").arg("+%z").output().ok();
    let zone = out
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let zone = zone.trim();
    let (sign, digits) = match zone.split_at_checked(1) {
        Some(("-", d)) => (-1, d),
        Some(("+", d)) => (1, d),
        _ => return 0,
    };
    let hours: i64 = digits.get(0..2).and_then(|h| h.parse().ok()).unwrap_or(0);
    let minutes: i64 = digits.get(2..4).and_then(|m| m.parse().ok()).unwrap_or(0);
    sign * (hours * 3_600 + minutes * 60)
}

/// The person's first name for the greeting: the account's full name if the system has one,
/// else the login name, capitalized.
fn first_name() -> Option<String> {
    let user = std::env::var("USER").ok()?;
    let full = std::process::Command::new("getent")
        .args(["passwd", &user])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|line| line.split(':').nth(4).map(str::to_owned))
        .filter(|gecos| !gecos.trim().is_empty());
    let name = full
        .as_deref()
        .unwrap_or(&user)
        .split([' ', ','])
        .next()?
        .to_owned();
    let mut chars = name.chars();
    Some(chars.next()?.to_uppercase().chain(chars).collect())
}

/// `quill --open-url URL`: the agents' `$BROWSER` (the engine's bridge). A local page opens in
/// the session's browser; anything else, or no quill to answer, in the desktop's own.
fn open_url(url: &str) -> bool {
    use std::io::{BufRead, Write};
    let handed = (|| -> Option<bool> {
        let socket = std::env::var_os("QUILL_BROWSER_BRIDGE")?;
        let key = std::env::var("QUILL_BROWSER_KEY").ok()?;
        let mut stream = std::os::unix::net::UnixStream::connect(socket).ok()?;
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .ok()?;
        writeln!(stream, "{}", serde_json::json!({ "key": key, "url": url })).ok()?;
        let mut reply = String::new();
        std::io::BufReader::new(stream).read_line(&mut reply).ok()?;
        Some(reply.trim() == "pane")
    })()
    .unwrap_or(false);
    handed
        || std::process::Command::new("xdg-open")
            .arg(url)
            .status()
            .is_ok_and(|status| status.success())
}

fn main() -> eframe::Result {
    // Chromium's own processes are quill run again; they do their work and leave.
    if let Some(code) = quill_engine::native::subprocess() {
        std::process::exit(code);
    }
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, url, ..] = args.as_slice() {
        if flag == "--open-url" {
            std::process::exit(if open_url(url) { 0 } else { 1 });
        }
    }
    let cwd = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| ".".into());
    let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    let native = native_browser(&saved_settings());
    let started: Arc<Mutex<Option<quill_engine::Native>>> = Arc::default();
    let keep = started.clone();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: wgpu_setup(native),
            ..Default::default()
        },
        viewport: egui::ViewportBuilder::default()
            .with_title("quill")
            .with_app_id("quill")
            .with_inner_size([1408.0, 844.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "quill",
        options,
        Box::new(move |creation| {
            if let Some(fonts) = quill_components::system_fonts() {
                limn::set_fonts(fonts);
            }
            limn_egui::install_fonts(&creation.egui_ctx);
            // `QUILL_THEME=dark|light`, else the system's, else dark.
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
            quill_ui::look::set_dark(dark);
            let inbox: Arc<Mutex<Vec<Event>>> = Arc::default();
            let (events, ctx) = (inbox.clone(), creation.egui_ctx.clone());
            // The engine's thread wakes the window for every event it reports.
            let sink = move |event| {
                events.lock().unwrap().push(event);
                ctx.request_repaint();
            };
            // `QUILL_PROGRAMS=claude=/path/fake-agent` runs another program for an agent, as
            // the engine's tests do, with sessions kept in memory only: a replay to look at.
            // The browser's frames wake the window too.
            let frames = quill_engine::Frames::default();
            let ctx = creation.egui_ctx.clone();
            frames.on_frame(move || ctx.request_repaint());
            let gpu = creation
                .wgpu_render_state
                .as_ref()
                .map(|render| (render.device.clone(), render.queue.clone()));
            let native = native.then(|| start_native(&frames, gpu)).flatten();
            *keep.lock().unwrap() = native.clone();
            let engine = match std::env::var("QUILL_PROGRAMS") {
                Ok(programs) => Engine::start_with(
                    sink,
                    quill_engine::Config {
                        programs: programs
                            .split(',')
                            .filter_map(|p| p.split_once('='))
                            .map(|(agent, program)| (agent.to_owned(), program.to_owned()))
                            .collect(),
                        frames: frames.clone(),
                        native: native.clone(),
                        ..Default::default()
                    },
                ),
                Err(_) => Engine::start_with(
                    sink,
                    quill_engine::Config {
                        frames: frames.clone(),
                        native: native.clone(),
                        ..quill_engine::Config::default_database()
                    },
                ),
            };
            Ok(Box::new(App {
                tree: UiTree::new(Element::spacer()),
                store: Store::default(),
                ui: Ui {
                    // Tests and demos pin a cheap model; the picker changes it per session.
                    model: std::env::var("QUILL_MODEL").ok(),
                    name: first_name(),
                    utc_offset: utc_offset(),
                    ..Ui::new(cwd)
                },
                engine,
                inbox,
                picked: Arc::default(),
                chosen: Arc::default(),
                chromium: Arc::default(),
                attached: Arc::default(),
                terminals: HashMap::new(),
                transcript: None,
                beside: None,
                applied: None,
                pinned_theme: std::env::var_os("QUILL_THEME").is_some(),
                frames,
                textures: HashMap::new(),
                page: page::Page::default(),
                browser_shown: None,
                address: None,
                fonts_attached: false,
            }))
        }),
    );
    // Chromium stops on the thread it started on, its tabs closed first.
    if let Some(native) = started.lock().unwrap().take() {
        native.shutdown();
    }
    result
}

/// A page's frame as egui holds pixels: they are already in its form, so the buffer is
/// taken as it is rather than converted again on the UI thread.
fn surface_frame(size: [usize; 2], rgba: Vec<u8>, scale: f32) -> Option<limn_egui::SurfaceFrame> {
    let pixels: Vec<egui::Color32> = bytemuck::allocation::try_cast_vec(rgba)
        .unwrap_or_else(|(_, rgba)| bytemuck::cast_slice(&rgba).to_vec());
    if pixels.len() != size[0] * size[1] {
        return None;
    }
    limn_egui::SurfaceFrame::from_image(egui::ColorImage::new(size, pixels), scale)
}

/// eframe's device; for native tabs, one that can take in Chromium's GPU buffers (dmabufs),
/// where the driver can, so their paints never leave the GPU.
fn wgpu_setup(native: bool) -> eframe::egui_wgpu::WgpuSetup {
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    if native {
        let base = setup.device_descriptor.clone();
        setup.device_descriptor = Arc::new(move |adapter| {
            let mut descriptor = base(adapter);
            descriptor.required_features |=
                adapter.features() & eframe::wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
            descriptor
        });
    }
    eframe::egui_wgpu::WgpuSetup::CreateNew(setup)
}

/// Whether the browser's pages are drawn by Chromium in quill's own process, rather than by
/// a headless one's screencast: when Settings asks. `QUILL_BROWSER=native` or `headless`
/// decides instead.
fn native_browser(settings: &quill_model::Settings) -> bool {
    match std::env::var("QUILL_BROWSER").as_deref() {
        Ok("native") => true,
        Ok("headless") => false,
        _ => settings.browser_native,
    }
}

/// The person's settings as the engine keeps them, read before the window opens.
fn saved_settings() -> quill_model::Settings {
    quill_engine::Config::default_database()
        .settings
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Start Chromium in quill's process, for native tabs, its paints copied on the window's
/// `gpu` where it can; `None` keeps the headless one.
fn start_native(
    frames: &quill_engine::Frames,
    gpu: Option<(eframe::wgpu::Device, eframe::wgpu::Queue)>,
) -> Option<quill_engine::Native> {
    let data = quill_engine::Config::default_database()
        .database
        .and_then(|db| db.parent().map(|dir| dir.join("chromium")))
        .unwrap_or_else(|| std::env::temp_dir().join("quill-chromium"));
    match quill_engine::Native::start(&data, frames.clone(), gpu) {
        Ok(native) => Some(native),
        Err(error) => {
            eprintln!("quill: the browser draws without native tabs: {error}");
            None
        }
    }
}
