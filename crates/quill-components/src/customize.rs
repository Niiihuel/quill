//! Customize's pages as Desktop draws them (3.6556.2 Skills and Plugins, captured dark at 1×
//! on 2026-10-26): a header of a bold title and its tools, a card that says what the page is
//! for, sections titled with their count, and items on an icon's tile with their menu. The
//! app says what each holds; these only lay it out.

use crate::icons;
use crate::look::{self, BODY, SMALL};
use crate::pieces::{hgap, menu, muted, text};
use limn::{Component, ComponentPalette, Element, FontFamily};

/// The header: the page's bold 22 px title, then `lead` beside it (tabs), and `tools` at the
/// end (a search, buttons), 28 high, as Desktop's.
pub fn header(title: &str, lead: Vec<Element>, tools: Vec<Element>, width: f32) -> Element {
    let mut row = vec![text(title)
        .font_family(FontFamily::SansBold)
        .font_size(22.0)
        .centered_y()];
    row.extend(lead.into_iter().map(|e| e.centered_y()));
    row.push(Element::spacer().flex(1.0));
    row.extend(tools.into_iter().map(|e| e.centered_y()));
    Element::row(row).gap(12.0).size(Some(width), Some(28.0))
}

/// The page's own search, 256 wide on the field's fill.
pub fn search(id: &str, value: &str, placeholder: &str) -> Element {
    limn::SearchField::new(
        id,
        value,
        placeholder,
        ComponentPalette {
            surface: look::customize_card(),
            raised: look::customize_card(),
            border: look::settings_field_edge(),
            ..look::palette()
        },
    )
    .size(limn::ControlSize::Compact)
    .width(256.0)
    .render()
}

/// A button of the header: filled ("Discover") or, `primary`, in the text's ink ("+ Add"),
/// 28 high, radius 6, with an optional icon before its label and chevron after it.
pub fn button(
    id: &str,
    label: &str,
    icon: Option<&'static str>,
    chevron: bool,
    primary: bool,
) -> Element {
    let (fill, ink, hover) = if primary {
        (
            look::palette().text,
            look::palette().surface,
            look::primary_hover(),
        )
    } else {
        (
            look::customize_button(),
            look::palette().text,
            look::tool_hover(),
        )
    };
    let glyph = |name: &'static str| {
        Element::icon(icons::of(name))
            .foreground(ink)
            .size(Some(14.0), Some(14.0))
            .centered_y()
    };
    let mut row = vec![hgap(4.0)];
    row.extend(icon.map(glyph));
    row.push(text(label).font_size(BODY).foreground(ink).centered_y());
    if chevron {
        row.push(glyph(icons::CARET_DOWN));
    }
    row.push(hgap(4.0));
    Element::clickable(id, Element::row(row).gap(6.0).size(None, Some(28.0)))
        .background(fill)
        .hover_background(hover)
        .radius(6.0)
}

/// A button that opens a menu of `entries` under its end, `open` while it shows.
pub fn menu_button(
    trigger: Element,
    id: &str,
    entries: Vec<limn::DropdownMenuEntry>,
    open: bool,
) -> Element {
    menu(
        trigger,
        entries,
        open,
        id,
        280.0,
        limn::PopoverPlacement::BottomEnd,
    )
}

/// The card at the page's top: what it is for, and a button to start (Desktop's "Add your
/// first …"), on the card's fill with radius 12 and 16 around.
pub fn hero(title: &str, words: &str, action: Option<Element>, width: f32) -> Element {
    let mut column = vec![
        text(title)
            .font_family(FontFamily::SansBold)
            .font_size(16.0),
        muted(words).font_size(BODY),
    ];
    if let Some(action) = action {
        column.push(Element::spacer().size(None, Some(6.0)));
        column.push(Element::row(vec![action]));
    }
    Element::column(column)
        .gap(8.0)
        .padding(16.0)
        .size(Some(width), None)
        .background(look::customize_card())
        .radius(12.0)
}

/// A section's title, 16 px, with its count in a pill, and what goes at its end (the
/// project's folder).
pub fn section(
    title: &str,
    count: Option<usize>,
    trailing: Option<Element>,
    width: f32,
) -> Element {
    let mut row = vec![text(title).font_size(16.0).centered_y()];
    if let Some(count) = count {
        row.push(
            Element::row(vec![
                hgap(6.0),
                muted(count.to_string()).font_size(11.0).centered_y(),
                hgap(6.0),
            ])
            .size(None, Some(16.0))
            .background(look::customize_count())
            .radius(8.0)
            .centered_y(),
        );
    }
    row.push(Element::spacer().flex(1.0));
    row.extend(trailing.map(|e| e.centered_y()));
    Element::row(row).gap(8.0).size(Some(width), Some(34.0))
}

/// An agent's heading over what its program lists, quill's own (Desktop has one agent): its
/// logo before its name, 18 px, and the count in a pill, over a hairline.
pub fn agent_heading(logo: &'static str, name: &str, count: Option<usize>, width: f32) -> Element {
    let mut row = vec![
        Element::icon(icons::of(logo))
            .foreground(look::palette().text)
            .size(Some(18.0), Some(18.0))
            .centered_y(),
        text(name)
            .font_family(FontFamily::SansBold)
            .font_size(18.0)
            .centered_y(),
    ];
    if let Some(count) = count {
        row.push(
            Element::row(vec![
                hgap(6.0),
                muted(count.to_string()).font_size(11.0).centered_y(),
                hgap(6.0),
            ])
            .size(None, Some(16.0))
            .background(look::customize_count())
            .radius(8.0)
            .centered_y(),
        );
    }
    Element::column(vec![
        Element::row(row).gap(8.0).size(Some(width), Some(34.0)),
        Element::spacer()
            .background(look::customize_line())
            .size(Some(width), Some(1.0)),
    ])
}

/// The agents an item is active in, at its end: each one's logo, 14 px, with its name for a
/// tooltip. `id` tells the item's marks from another's.
pub fn agent_marks(id: &str, agents: &[(&'static str, String)]) -> Element {
    let marks = agents.iter().enumerate().map(|(at, (logo, name))| {
        let mark = Element::icon(icons::of(logo))
            .foreground(look::muted())
            .size(Some(14.0), Some(14.0))
            .centered_y();
        limn::Tooltip::new(mark, format!("{id}:{at}"), name.as_str(), look::palette()).render()
    });
    Element::row(marks.collect()).gap(6.0)
}

/// How an "All agents" item stands in one agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Added to the agent's sessions.
    On,
    /// The person turned it off for this agent.
    Off,
    /// The agent has one of its own that wins.
    Replaced,
    /// The agent cannot use it.
    Refused,
}

/// Every agent's logo at an item's end, inked by how the item stands there (the text's ink
/// when on, faint when off, amber when replaced, red when refused), with the reason as its
/// tooltip. `id` tells the item's marks from another's.
pub fn agent_standing(id: &str, agents: &[(&'static str, String, Standing)]) -> Element {
    let marks = agents
        .iter()
        .enumerate()
        .map(|(at, (logo, why, standing))| {
            let ink = match standing {
                Standing::On => look::palette().text,
                Standing::Off => look::customize_edge(),
                Standing::Replaced => look::awaiting(),
                Standing::Refused => look::failed(),
            };
            let mark = Element::icon(icons::of(logo))
                .foreground(ink)
                .size(Some(14.0), Some(14.0))
                .centered_y();
            limn::Tooltip::new(mark, format!("{id}:{at}"), why.as_str(), look::palette()).render()
        });
    Element::row(marks.collect()).gap(6.0)
}

/// The folder a section is about, outlined, its icon before its name (Desktop's "work"), 28
/// high as the header's buttons.
pub fn folder_chip(id: &str, name: &str) -> Element {
    let row = Element::row(vec![
        hgap(4.0),
        Element::icon(icons::of(icons::FOLDER))
            .foreground(look::palette().text)
            .size(Some(14.0), Some(14.0))
            .centered_y(),
        text(name).font_size(SMALL).centered_y(),
        hgap(4.0),
    ])
    .gap(6.0)
    .size(None, Some(28.0));
    Element::clickable(id, row)
        .hover_background(look::tool_hover())
        .border(look::customize_edge(), 1.0)
        .radius(8.0)
}

/// One item: its icon on a 36 px tile, its name over "from {who} · {words}" on one line, what
/// goes at its end (a date, a state) and its menu, 63 high over a hairline, as Desktop's.
pub fn item(
    icon: &'static str,
    name: &str,
    words: &str,
    end: Option<Element>,
    more: Option<Element>,
    width: f32,
) -> Element {
    let picture = Element::icon(icons::of(icon))
        .foreground(look::palette().text)
        .size(Some(16.0), Some(16.0));
    item_with(picture, name, words, end, more, width)
}

/// [`item`] with its logo, 24 px on the same tile, in place of the icon. The tile keeps its
/// background, so a dark logo shows in the dark theme too.
pub fn item_logo(
    logo: &limn::RasterImage,
    name: &str,
    words: &str,
    end: Option<Element>,
    more: Option<Element>,
    width: f32,
) -> Element {
    let picture = Element::raster_image(logo.clone()).size(Some(24.0), Some(24.0));
    item_with(picture, name, words, end, more, width)
}

/// The item over `picture`, centered on its 36 px tile.
fn item_with(
    picture: Element,
    name: &str,
    words: &str,
    end: Option<Element>,
    more: Option<Element>,
    width: f32,
) -> Element {
    let tile = Element::row(vec![picture.centered().flex(1.0)])
        .size(Some(36.0), Some(36.0))
        .background(look::customize_tile())
        .radius(8.0)
        .centered_y();
    let copy = Element::column(vec![
        Element::truncated_text(name.to_owned())
            .font_family(FontFamily::Sans)
            .font_size(15.0)
            .foreground(look::palette().text),
        Element::truncated_text(words.to_owned())
            .font_family(FontFamily::Sans)
            .font_size(13.0)
            .foreground(look::muted()),
    ])
    .gap(4.0)
    .flex(1.0)
    .centered_y();
    // The copy 16 px after its tile, as Desktop's.
    let mut row = vec![Element::row(vec![tile, copy]).gap(16.0).flex(1.0)];
    row.extend(end.map(|e| e.centered_y()));
    row.extend(more.map(|e| e.centered_y()));
    Element::column(vec![
        Element::row(row).gap(12.0).size(Some(width), Some(62.0)),
        Element::spacer()
            .background(look::customize_line())
            .size(Some(width), Some(1.0)),
    ])
}

/// An item's "⋯" opening its menu, 28 px square.
pub fn more(id: &str, entries: Vec<limn::DropdownMenuEntry>, open: bool) -> Element {
    let glyph = Element::icon(icons::of(icons::DOTS_HORIZONTAL))
        .foreground(look::palette().text)
        .size(Some(16.0), Some(16.0))
        .centered();
    let trigger = Element::clickable(
        id,
        Element::row(vec![glyph.flex(1.0)]).size(Some(28.0), Some(28.0)),
    )
    .hover_background(look::tool_hover())
    .radius(6.0);
    menu_button(trigger, id, entries, open)
}

/// Something being read: a spinner beside what it waits for.
pub fn loading(id: &str, words: &str) -> Element {
    Element::row(vec![
        limn::Spinner::new(id, look::muted())
            .size(16.0)
            .render()
            .centered_y(),
        muted(words).font_size(BODY).centered_y(),
    ])
    .gap(10.0)
    .size(None, Some(48.0))
}

/// What a section says when it holds nothing: an icon's tile beside muted words, as
/// Desktop's "No plugins installed for this project yet."
pub fn empty(icon: &'static str, words: &str, width: f32) -> Element {
    let tile = Element::row(vec![Element::icon(icons::of(icon))
        .foreground(look::muted())
        .size(Some(16.0), Some(16.0))
        .centered()
        .flex(1.0)])
    .size(Some(36.0), Some(36.0))
    .background(look::customize_tile())
    .radius(8.0)
    .centered_y();
    Element::row(vec![tile, muted(words).font_size(BODY).centered_y()])
        .gap(16.0)
        .size(Some(width), Some(48.0))
}
