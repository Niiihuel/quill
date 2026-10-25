//! quill's tokens. Neutrals measured from Claude Desktop's Code surface at 1408×844
//! ;
//! git colors from its static tokens; the accent is quill's own (spec.md §6).

use limn::{Color, Theme};
use std::sync::atomic::{AtomicBool, Ordering};

static DARK: AtomicBool = AtomicBool::new(false);

/// Draw dark from now on, or light again; the tooltips follow at once.
pub fn set_dark(dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
    install();
}

pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

const fn hex(rgb: u32) -> Color {
    Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Each color as a function of the light and dark looks. The dark one is the person's own
/// Desktop in dark mode (its canvas #191919, text #EFEFEF, muted #9C9C9C, filled
/// chips #303030, raised bands #242424, hairlines #353535, git #32D74B / #FF2C56); what that
/// capture does not show is inferred, and says so.
macro_rules! tones {
    ($($(#[$doc:meta])* $name:ident = $light:expr, $dark:expr;)*) => {
        $(
            $(#[$doc])*
            pub fn $name() -> Color {
                if is_dark() { $dark } else { $light }
            }
        )*
    };
}

tones! {
    /// The sidebar's fill (dark: inferred, the capture hides it).
    sidebar = hex(0xfafafa), hex(0x1f1f1f);
    divider = hex(0xe5e5e5), hex(0x303030);
    /// The person's bubble (dark: inferred, as the filled chips).
    user_bubble = hex(0xf3f3f3), hex(0x303030);
    muted = hex(0x7d7d7d), hex(0x9c9c9c);
    /// A command's box and inline code.
    command_box = hex(0xf3f3f3), hex(0x2a2a2a);
    awaiting = hex(0xf0ac55), hex(0xf0ac55);
    /// The ring round the waiting dot and the edge of its "Needs your input" pill
    /// (dark: inferred).
    awaiting_edge = hex(0xfee9bb), hex(0x5c4520);
    running = hex(0x3b5bdb), hex(0x3b5bdb);
    added = hex(0x1e9e3c), hex(0x32d74b);
    /// A failed step's label (dark: inferred).
    failed = hex(0xa42528), hex(0xff6b6b);
    removed = hex(0xcd2054), hex(0xff2c56);
    /// The hairline under the top bar and pane headers, a shade darker than the divider.
    pane_border = hex(0xe0e0e0), hex(0x303030);
    /// The terminal's cursor, the blue of its ANSI palette (dark: inferred).
    cursor = hex(0x0073e6), hex(0x3b8eea);
    /// The selected sidebar row (dark: inferred).
    selected = hex(0xe0e0e0), hex(0x303030);
    /// A filled main button under the pointer (inferred: the hover was not captured).
    primary_hover = hex(0x333333), hex(0xd6d6d6);
    /// The confirmation dialog's Cancel and its soft destructive action.
    cancel = hex(0xf2f2f2), hex(0x303030);
    danger_fill = hex(0xfbe7e7), hex(0x3a1e1e);
    danger_border = hex(0xdc3c3d), hex(0xdc3c3d);
    danger_ink = hex(0xa42528), hex(0xff7b7b);
    /// The dialog's backdrop: black at about 41 %, #969696 over white.
    scrim = Color::rgba(0, 0, 0, 105), Color::rgba(0, 0, 0, 150);
    /// The permission card's hairline and the ink of the command it shows.
    card_border = hex(0xebebeb), hex(0x353535);
    command_ink = hex(0x616161), hex(0xb0b0b0);
    /// A boundary under the pointer or being dragged.
    handle = hex(0x257de0), hex(0x257de0);
    /// The composer: its box's edge and shadow, its buttons under the pointer and
    /// with their menu open, the footer chips likewise, and the amber Manual reads in.
    composer_border = hex(0xd9d9d9), hex(0x404040);
    composer_shadow = Color::rgba(0, 0, 0, 16), Color::rgba(0, 0, 0, 60);
    composer_hover = hex(0xe5e5e5), hex(0x3a3a3a);
    composer_open = hex(0xd2d2d2), hex(0x444444);
    chip_hover = hex(0xe0e0e0), hex(0x303030);
    chip_open = hex(0xcacaca), hex(0x3a3a3a);
    manual = hex(0x794c02), hex(0xe6b35c);
    /// Hover fills: a sidebar row, a selected one, an icon button, and
    /// an active toggle (dark: inferred).
    row_hover = hex(0xededed), hex(0x272727);
    selected_hover = hex(0xd5d5d5), hex(0x383838);
    tool_hover = hex(0xe5e5e5), hex(0x303030);
    active_hover = hex(0xd8d8d8), hex(0x3a3a3a);
    /// Keywords in code (`def`, the f of an f-string); functions take the cursor's
    /// blue, strings the added green and f-string braces the removed red.
    keyword = hex(0xc5621b), hex(0xf0a35c);
    /// The greeting's spark, Claude's orange.
    spark = hex(0xd97757), hex(0xd97757);
    /// The usage ring's track: Desktop's #E5E5E5 at rest, #CDCDCD under the pointer, met
    /// halfway so it shows on both.
    ring_track = hex(0xd6d6d6), hex(0x404040);
    /// The effort slider's pill and its marks.
    slider_track = hex(0xe6e6e6), hex(0x333333);
    slider_mark = hex(0xafafaf), hex(0x6a6a6a);
    /// An active toggle or tab.
    active = hex(0xe5e5e5), hex(0x303030);
}

// Sizes are Desktop's at 1× (Xvfb at 96 dpi, 1408×844), from private notes
// `rea/evidence/runtime/20261025-062138-auth/`. The earlier Xephyr captures () drew
// at 0.75×, so sizes read from them are scaled.
/// Sidebar width with its 1 px border.
pub const SIDEBAR_WIDTH: f32 = 288.0;
/// The transcript and composer column, at most.
pub const COLUMN: f32 = 770.0;
/// The right panel: 280 px cards with 8 px around them.
pub const PANEL_WIDTH: f32 = 296.0;
pub const TOP_BAR: f32 = 36.0;
pub const PANE_HEADER: f32 = 43.0;
/// UI text; the conversation's text is larger.
pub const BODY: f32 = 14.0;
pub const CHAT: f32 = 16.0;
/// Section labels and captions.
pub const SMALL: f32 = 13.0;

pub fn theme() -> Theme {
    Theme {
        background: palette().surface,
        foreground: palette().text,
        accent: running(),
    }
}

/// Desktop's surfaces are its canvas under a hairline, not tinted levels; menus
/// and cards lift with a faint shadow only.
pub fn palette() -> limn::ComponentPalette {
    let dark = is_dark();
    let pick = |light: Color, dark_one: Color| if dark { dark_one } else { light };
    limn::ComponentPalette {
        surface: pick(hex(0xffffff), hex(0x191919)),
        raised: pick(hex(0xffffff), hex(0x242424)),
        hover: pick(
            Color::rgba(0x19, 0x19, 0x19, 13),
            Color::rgba(0xff, 0xff, 0xff, 13),
        ),
        selected: pick(
            Color::rgba(0x19, 0x19, 0x19, 31),
            Color::rgba(0xff, 0xff, 0xff, 31),
        ),
        border: divider(),
        text: pick(hex(0x191919), hex(0xefefef)),
        muted: muted(),
        shadow: pick(Color::rgba(0, 0, 0, 24), Color::rgba(0, 0, 0, 80)),
    }
}

/// Menus: rows light gray under the pointer; Desktop marks the current row with
/// its check alone.
pub fn menu_palette() -> limn::ComponentPalette {
    let palette = palette();
    limn::ComponentPalette {
        hover: if is_dark() {
            hex(0x2e2e2e)
        } else {
            hex(0xf0f0f0)
        },
        selected: palette.raised,
        ..palette
    }
}

/// The composer's editor sits on the card without a fill of its own, as Desktop's does
///; limn fills both the card and the editor with `selected`, so here it is the
/// card's: white, or the dark composer's #303030.
pub fn composer_palette() -> limn::ComponentPalette {
    limn::ComponentPalette {
        selected: if is_dark() {
            hex(0x303030)
        } else {
            hex(0xffffff)
        },
        ..palette()
    }
}

/// Desktop's tooltips are near-black with white words and a gray shortcut; limn
/// takes one style for every tooltip from its tokens.
pub fn install() {
    let mut tokens = limn::tokens();
    // Shortcuts are words, not key caps, in menus and tooltips alike.
    tokens.shortcut_style = limn::ShortcutStyle::Text;
    let ink = Color::rgb(0x0b, 0x0b, 0x0b);
    tokens.tooltip = Some(limn::TooltipStyle {
        background: ink,
        border: ink,
        keycap: ink,
        text: Color::rgb(0xff, 0xff, 0xff),
        muted: Color::rgb(0xa3, 0xa3, 0xa3),
        shadow: Color::rgba(0, 0, 0, 40),
    });
    limn::set_tokens(tokens);
}
