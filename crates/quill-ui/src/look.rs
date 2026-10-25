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
pub const COMMAND_BOX: Color = Color::rgb(0xf9, 0xf9, 0xf9);
pub const AWAITING: Color = Color::rgb(0xf0, 0xac, 0x55);
pub const RUNNING: Color = Color::rgb(0x3b, 0x5b, 0xdb);
pub const ADDED: Color = Color::rgb(0x1e, 0x9e, 0x3c);
pub const REMOVED: Color = Color::rgb(0xcd, 0x20, 0x54);

/// Sidebar width with its 1 px border.
pub const SIDEBAR_WIDTH: f32 = 212.0;
/// The transcript and composer column: 540 wide at 1408.
pub const COLUMN: f32 = 540.0;
pub const BODY: f32 = 13.0;

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
