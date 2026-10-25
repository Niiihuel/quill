//! quill's tokens. Neutrals measured from Claude Desktop's Code surface at 1408×844
//! ;
//! git colors from its static tokens; the accent is quill's own (spec.md §6).

use limn::{Color, Theme};

pub const THEME: Theme = Theme {
    background: Color::rgb(0xff, 0xff, 0xff),
    foreground: Color::rgb(0x19, 0x19, 0x19),
    accent: Color::rgb(0x3b, 0x5b, 0xdb),
};
pub const SIDEBAR: Color = Color::rgb(0xfa, 0xfa, 0xfa);
pub const DIVIDER: Color = Color::rgb(0xe5, 0xe5, 0xe5);
pub const USER_BUBBLE: Color = Color::rgb(0xf3, 0xf3, 0xf3);
pub const MUTED: Color = Color::rgb(0x7d, 0x7d, 0x7d);
pub const COMMAND_BOX: Color = Color::rgb(0xf3, 0xf3, 0xf3);
pub const AWAITING: Color = Color::rgb(0xf0, 0xac, 0x55);
pub const RUNNING: Color = Color::rgb(0x3b, 0x5b, 0xdb);
pub const ADDED: Color = Color::rgb(0x1e, 0x9e, 0x3c);
pub const REMOVED: Color = Color::rgb(0xcd, 0x20, 0x54);

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
/// The hairline under the top bar and pane headers, a shade darker than DIVIDER.
pub const PANE_BORDER: Color = Color::rgb(0xe0, 0xe0, 0xe0);
/// The terminal's cursor, the blue of its ANSI palette.
pub const CURSOR: Color = Color::rgb(0x00, 0x73, 0xe6);
/// The selected sidebar row.
pub const SELECTED: Color = Color::rgb(0xe0, 0xe0, 0xe0);
/// The confirmation dialog's Cancel and its soft destructive action.
pub const CANCEL: Color = Color::rgb(0xf2, 0xf2, 0xf2);
pub const DANGER_FILL: Color = Color::rgb(0xfb, 0xe7, 0xe7);
pub const DANGER_BORDER: Color = Color::rgb(0xdc, 0x3c, 0x3d);
pub const DANGER_INK: Color = Color::rgb(0xa4, 0x25, 0x28);
/// The dialog's backdrop: black at about 41 %, #969696 over white.
pub const SCRIM: Color = Color::rgba(0, 0, 0, 105);
/// The permission card's hairline and the ink of the command it shows.
pub const CARD_BORDER: Color = Color::rgb(0xeb, 0xeb, 0xeb);
pub const COMMAND_INK: Color = Color::rgb(0x61, 0x61, 0x61);
/// An active toggle or tab.
pub const ACTIVE: Color = Color::rgb(0xe5, 0xe5, 0xe5);

/// Desktop's surfaces are white under a hairline, not tinted levels; menus and
/// cards lift with a faint shadow only.
pub const PALETTE: limn::ComponentPalette = limn::ComponentPalette {
    surface: Color::rgb(0xff, 0xff, 0xff),
    raised: Color::rgb(0xff, 0xff, 0xff),
    hover: Color::rgba(0x19, 0x19, 0x19, 13),
    selected: Color::rgba(0x19, 0x19, 0x19, 31),
    border: DIVIDER,
    text: Color::rgb(0x19, 0x19, 0x19),
    muted: MUTED,
    shadow: Color::rgba(0, 0, 0, 24),
};

/// The composer's editor sits on the card without a fill of its own, as Desktop's does
///; limn fills both the card and the editor with `selected`, so here it is white.
pub const COMPOSER_PALETTE: limn::ComponentPalette = limn::ComponentPalette {
    selected: Color::rgb(0xff, 0xff, 0xff),
    ..PALETTE
};
