//! The small pieces quill's views are built from, for every app of quill to draw the same.

use crate::look::{self, BODY, SMALL};
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

pub fn vline(height: f32) -> Element {
    Element::spacer()
        .background(look::divider())
        .size(Some(1.0), Some(height))
}

pub fn hline(width: f32, color: limn::Color) -> Element {
    Element::spacer()
        .background(color)
        .size(Some(width), Some(1.0))
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
pub fn icon_tool(
    id: &str,
    icon: limn::Icon,
    label: &str,
    side: f32,
    _palette: ComponentPalette,
) -> Element {
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

/// A button with its keys in caps after the label, Desktop's "Revise… Esc" and
/// "Next Ctrl ⏎": the light fill, or the near-black one when
/// `primary`.
pub fn keyed_button(
    id: &str,
    label: &str,
    keys: &str,
    primary: bool,
    enabled: bool,
    palette: ComponentPalette,
) -> Element {
    let (fill, hover, ink) = if primary {
        (look::palette().text, look::primary_hover(), palette.surface)
    } else {
        (look::cancel(), look::tool_hover(), look::palette().text)
    };
    let ink = if enabled { ink } else { look::muted() };
    let mut content = vec![
        hgap(10.0),
        text(label).font_size(BODY).foreground(ink).centered_y(),
    ];
    if !keys.is_empty() {
        content.push(
            limn::Kbd::new(
                keys,
                ComponentPalette {
                    text: ink,
                    ..palette
                },
            )
            .render()
            .centered_y(),
        );
    }
    content.push(hgap(10.0));
    let row = Element::row(content).gap(8.0).size(None, Some(30.0));
    if !enabled {
        return row.background(look::cancel()).radius(8.0);
    }
    Element::clickable(id, row)
        .background(fill)
        .hover_background(hover)
        .radius(8.0)
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

/// The measures of quill's menus, `width` wide: Desktop's rows are 28 px with 14 px labels
/// and 13 px captions. Context menus take them too.
pub fn menu_sizing(width: f32) -> limn::SelectorSizing {
    limn::SelectorSizing {
        preferred_width: width,
        min_width: width,
        row_height: 28.0,
        detail_row_height: 44.0,
        header_height: 26.0,
        icon_size: 16.0,
        label_size: BODY,
        detail_size: SMALL,
        padding: 8.0,
        max_visible_items: 12,
        ..limn::SelectorSizing::COMPACT
    }
}

/// A menu in Desktop's style: rounded, 28 px rows, the current one marked by a blue check.
pub fn menu(
    anchor: Element,
    entries: Vec<limn::DropdownMenuEntry>,
    open: bool,
    id: &str,
    width: f32,
    placement: limn::PopoverPlacement,
) -> Element {
    let sizing = menu_sizing(width);
    limn::DropdownMenu::new(
        anchor,
        entries,
        open,
        format!("{id}:dismiss"),
        look::menu_palette(),
    )
    .sizing(sizing)
    // The current row has only Desktop's blue check, no fill (limn #127).
    .check_ink(look::handle())
    .selected_fill(false)
    .placement(placement)
    .render()
}

/// A pane's header: what it shows on the left, its tools on the right, 44 px over a
/// hairline.
pub fn pane_header(lead: Element, tools: Vec<Element>, width: f32) -> Element {
    let mut row = vec![hgap(12.0), lead, Element::spacer().flex(1.0)];
    // At their own height in the middle, a button as much as an icon.
    row.extend(tools.into_iter().map(Element::centered_y));
    row.push(hgap(4.0));
    Element::column(vec![
        Element::row(row)
            .gap(8.0)
            .size(Some(width), Some(look::PANE_HEADER)),
        hline(width, look::pane_border()),
    ])
}
