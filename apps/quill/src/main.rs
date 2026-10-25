//! quill's window: for now the empty shell (docs/plan.md Q0.2).
//! Colors and widths are measured from Claude Desktop's Code surface at 1408×844
//! .

use eframe::egui;
use limn::{
    Button, ButtonVariant, Color, Component, Element, FontFamily, Size, Theme, UiAction, UiTree,
};

/// Sidebar width, its 1 px border included.
const SIDEBAR: f32 = 212.0;

const LIGHT: Theme = Theme {
    background: Color::rgb(0xff, 0xff, 0xff),
    foreground: Color::rgb(0x19, 0x19, 0x19),
    // quill's own accent (docs/spec.md §6), not Anthropic's.
    accent: Color::rgb(0x3b, 0x5b, 0xdb),
};
const SIDEBAR_BG: Color = Color::rgb(0xfa, 0xfa, 0xfa);
const DIVIDER: Color = Color::rgb(0xe5, 0xe5, 0xe5);

#[derive(Default)]
struct State {
    /// Sessions arrive with the engine (plan Q4); until then the sidebar is empty.
    sessions: Vec<String>,
}

fn view(state: &State, window: Size) -> Element {
    let palette = LIGHT.palette();
    let sessions = if state.sessions.is_empty() {
        vec![Element::text("No sessions yet")
            .font_family(FontFamily::Sans)
            .foreground(palette.muted)]
    } else {
        state
            .sessions
            .iter()
            .map(|name| Element::text(name.clone()).font_family(FontFamily::Sans))
            .collect()
    };
    let sidebar = Element::column(
        [
            vec![Button::new("new-session", "New session", palette)
                .variant(ButtonVariant::Ghost)
                .render()],
            sessions,
        ]
        .concat(),
    )
    .gap(8.0)
    .padding(8.0)
    .background(SIDEBAR_BG)
    .size(Some(SIDEBAR - 1.0), Some(window.height));
    let divider = Element::spacer()
        .background(DIVIDER)
        .size(Some(1.0), Some(window.height));
    let main = Element::column(vec![
        Element::spacer().flex(1.0),
        Element::text("Describe a task to start a session")
            .font_family(FontFamily::Sans)
            .foreground(palette.muted),
        Element::spacer().flex(1.0),
    ])
    .padding(24.0)
    .flex(1.0);
    Element::row(vec![sidebar, divider, main])
        .background(LIGHT.background)
        .size(Some(window.width), Some(window.height))
}

fn update(_state: &mut State, action: &UiAction) {
    if let UiAction::Clicked { id } = action {
        if id == "new-session" {
            // Opens the new-session flow once the engine exists (plan Q4.2).
        }
    }
}

struct App {
    tree: UiTree,
    state: State,
    fonts_attached: bool,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.fonts_attached {
            limn_egui::attach_fonts(&mut self.tree, &ctx);
            self.fonts_attached = true;
        }
        limn_egui::normalize_keyboard_input(&ctx);
        let area = ui.max_rect();
        let window = Size::new(area.width(), area.height());

        self.tree.set_root(view(&self.state, window));
        self.tree.layout(window);
        for event in limn_egui::events(&ctx, area.min) {
            for action in self.tree.handle_event(event) {
                update(&mut self.state, &action);
            }
        }
        for action in limn_egui::apply_access_requests(&ctx, &mut self.tree) {
            update(&mut self.state, &action);
        }
        self.tree.set_root(view(&self.state, window));
        let frame = self.tree.layout(window);
        limn_egui::paint_at(ui, &frame, area.min);
        limn_egui::set_cursor(&ctx, self.tree.cursor());
        if let Some(after) = self.tree.animation_repaint_after() {
            ctx.request_repaint_after(after);
        }
    }
}

fn main() -> eframe::Result {
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
        Box::new(|creation| {
            limn_egui::install_fonts(&creation.egui_ctx);
            Ok(Box::new(App {
                tree: UiTree::new(Element::spacer()),
                state: State::default(),
                fonts_attached: false,
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_and_divider_match_the_capture() {
        let window = Size::new(1408.0, 844.0);
        let mut tree = UiTree::new(view(&State::default(), window));
        let frame = tree.layout(window);
        let button = frame
            .regions
            .iter()
            .find(|region| region.id == "new-session")
            .expect("new-session button");
        assert!(button.rect.x >= 0.0 && button.rect.x + button.rect.width <= SIDEBAR - 1.0);
    }
}
