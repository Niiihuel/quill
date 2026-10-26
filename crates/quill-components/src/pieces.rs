//! The small pieces quill's views are built from, as quill-ui's view draws them, for every app
//! to draw the same. quill-ui's `view.rs` keeps its own until the work in progress on it lands
//! on main; then it takes these.

use crate::look::{self, BODY};
use limn::{Component, ComponentPalette, Element, FontFamily};

pub fn text(s: impl Into<String>) -> Element {
    Element::text(s.into())
        .font_family(FontFamily::Sans)
        .font_size(BODY)
        .foreground(look::palette().text)
}

pub fn muted(s: impl Into<String>) -> Element {
    text(s).foreground(look::muted())
}

pub fn hgap(width: f32) -> Element {
    Element::spacer().size(Some(width), None)
}

pub fn vline(height: Option<f32>) -> Element {
    Element::spacer()
        .background(look::divider())
        .size(Some(1.0), height)
}

pub fn hline(width: Option<f32>, color: limn::Color) -> Element {
    Element::spacer().background(color).size(width, Some(1.0))
}

/// An icon button as Desktop draws them: rounded 6, its icon in the text's ink,
/// `hover` under the pointer, an optional fill at rest, and a tooltip with its shortcut.
pub fn tool_surface(
    id: &str,
    icon: limn::Icon,
    label: &str,
    side: f32,
    fill: Option<limn::Color>,
    hover: limn::Color,
    shortcut: Option<&str>,
) -> Element {
    let palette = ComponentPalette {
        hover,
        selected: hover,
        ..look::palette()
    };
    // The icon is centered by hand: limn's icon button sets it a pixel to the left.
    let glyph = Element::icon(icon)
        .foreground(look::palette().text)
        .size(Some(16.0), Some(16.0))
        .centered();
    let button = palette
        .interactive_surface(Element::clickable(id, glyph).accessibility_label(label))
        .radius(6.0)
        .size(Some(side), Some(side));
    let button = match fill {
        Some(fill) => button.background(fill),
        None => button,
    };
    let tip = limn::Tooltip::new(button, id, label, look::palette());
    match shortcut {
        Some(keys) => tip.shortcut(keys),
        None => tip,
    }
    .render()
}

/// An icon button with no fill at rest.
pub fn icon_tool(id: &str, icon: limn::Icon, label: &str, side: f32) -> Element {
    tool_surface(id, icon, label, side, None, look::tool_hover(), None).centered_y()
}

/// A top-bar toggle: its icon on the active fill while what it shows is open.
pub fn toggle(
    id: &str,
    icon: limn::Icon,
    label: &str,
    on: bool,
    shortcut: Option<&str>,
) -> Element {
    let (fill, hover) = if on {
        (Some(look::active()), look::active_hover())
    } else {
        (None, look::tool_hover())
    };
    tool_surface(id, icon, label, 26.0, fill, hover, shortcut).centered_y()
}

/// A sidebar row `height` tall, rounded, lit under the pointer and filled while selected
///.
pub fn side_row(id: String, content: Vec<Element>, selected: bool, height: f32) -> Element {
    let palette = ComponentPalette {
        hover: if selected {
            look::selected_hover()
        } else {
            look::row_hover()
        },
        ..look::palette()
    };
    let row = Element::row(content).gap(6.0).size(None, Some(height));
    let row = palette
        .interactive_surface(Element::clickable(id, row))
        .radius(6.0)
        .hover_region();
    if selected {
        row.background(look::selected())
    } else {
        row
    }
}
