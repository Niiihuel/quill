//! The window: sidebar of sessions, transcript and composer (features/code-*.md, docs/ui.md §2).

use crate::icons;
use crate::look::{self, BODY, CHAT, COLUMN, SMALL};
use crate::{Item, Pane, Session, Store, ToolResult, Ui};
use limn::{
    Button, ButtonGroup, ButtonSize, ButtonVariant, Component, ComponentPalette, Composer,
    ControlSize, Element, FontFamily, FramedCard, Markdown, OutputBlock, Size, Spinner,
    TextShimmer,
};
use quill_model::{ChangeKind, FileEffect, SessionStatus};
use serde_json::Value;

pub const COMPOSER: &str = "composer";
pub const NEW_SESSION: &str = "new-session";
pub const STOP: &str = "stop";
pub const MODE_MENU: &str = "mode-menu";
pub const MODEL_MENU: &str = "model-menu";
pub const DIFF_TOGGLE: &str = "diff:toggle";
pub const DIFF_CLOSE: &str = "pane:diff:close";
pub const PANEL_TOGGLE: &str = "panel:toggle";
/// Asks the host for the platform folder picker; the host sets `Ui::cwd` with the answer.
pub const PICK_FOLDER: &str = "pick-folder";
pub const PLUS_MENU: &str = "plus-menu";
pub const FOLDER_MENU: &str = "folder-menu";
pub const SEND: &str = "send";
pub const USAGE_MENU: &str = "usage-menu";
pub const EFFORT_MENU: &str = "effort-menu";
/// "See all →": the plan's usage page, which the host opens in the browser.
pub const USAGE_ALL: &str = "usage:all";
pub const SCROLL_END: &str = "scroll-end";
/// The prefix of a code block's copy button; the code follows it.
pub const COPY_CODE: &str = "copy-code:";
/// "+" menu actions the host carries out: pickers for @-mentions, and the slash.
pub const ATTACH_FILES: &str = "attach:files";
pub const ATTACH_FOLDER: &str = "attach:folder";
pub const SLASH: &str = "attach:slash";
pub const TERMINAL_TOGGLE: &str = "terminal:toggle";
/// The terminal pane; while it has the focus the host sends keys to the shell.
pub const TERMINAL: &str = "terminal";
/// The terminal's scroll area, which follows new output while it is at the end.
pub const TERMINAL_SCROLL: &str = "terminal:scroll";
pub const TERMINAL_NEW: &str = "terminal:new";
/// The field a tab's pill turns into while the person renames it.
pub const TERMINAL_RENAME: &str = "terminal:rename";
pub const TERMINAL_CLOSE_CONFIRM: &str = "terminal:close-confirm";
pub const TERMINAL_CLOSE_CANCEL: &str = "terminal:close-cancel";
pub const SIDEBAR_TOGGLE: &str = "sidebar:toggle";
pub const SEARCH: &str = "search";
pub const SEARCH_FIELD: &str = "search:field";
pub const SEARCH_CLOSE: &str = "search:close";
pub const SEARCH_NEW: &str = "search:new";
/// The chevron beside the session's title and the menu it opens.
pub const TITLE_MENU: &str = "title-menu";
/// The field the title turns into while the person renames the session.
pub const TITLE_FIELD: &str = "title:field";
/// The branch bar's pull request, the menu that picks its kind, and its close button.
pub const PR_CREATE: &str = "pr:create";
pub const PR_MENU: &str = "pr-menu";
pub const BAR_CLOSE: &str = "branch-bar:close";
pub const FILTER_RESET: &str = "filter:reset";
pub const DELETE_CONFIRM: &str = "delete:confirm";
pub const DELETE_CANCEL: &str = "delete:cancel";
const TERMINAL_FONT: f32 = 12.0;
/// The shell's text starts 12 px in and 8 px under the pane's header.
const TERMINAL_PADDING: [f32; 2] = [12.0, 8.0];

fn text(s: impl Into<String>) -> Element {
    Element::text(s.into())
        .font_family(FontFamily::Sans)
        .font_size(BODY)
        .foreground(look::palette().text)
}

/// The last part of a path, as Desktop names a session's folder.
fn folder_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn muted(s: impl Into<String>) -> Element {
    text(s).foreground(look::muted())
}

/// How the space right of the sidebar is shared.
struct Layout {
    /// The conversation; zero while a pane is expanded over it.
    main: f32,
    /// The column of panes, and the panes in it from the top.
    column: f32,
    panes: Vec<Pane>,
    /// The right panel, shown only while no pane is open.
    panel: f32,
}

fn layout(store: &Store, ui: &Ui, window: Size) -> Layout {
    let session = ui.selected.and_then(|id| store.session(id));
    let rest = (window.width - sidebar_width(ui)).max(0.0);
    let panes: Vec<Pane> = ui
        .panes
        .iter()
        .copied()
        .filter(|p| *p != Pane::Diff || diff_files(store, ui).is_some())
        .collect();
    if let Some(pane) = ui.expanded_pane.filter(|p| panes.contains(p)) {
        return Layout {
            main: 0.0,
            column: rest,
            panes: vec![pane],
            panel: 0.0,
        };
    }
    // The first pane sets the column: the diff takes 3 of 5 parts, a terminal 1 of 3
    // (the 1× captures of 2026-10-25, see look.rs).
    let fraction = match panes.first() {
        Some(Pane::Diff) => 0.6,
        // A helper agent and a plan open at a third too.
        Some(Pane::Terminal | Pane::Agent | Pane::Plan) => 1.0 / 3.0,
        None => 0.0,
    };
    // Dragging the boundary sets the share, as in Desktop.
    let column = if panes.is_empty() {
        0.0
    } else {
        column_width(rest, ui.column_fraction.unwrap_or(fraction))
    };
    let panel = if panes.is_empty()
        && ui.panel_open
        && session.is_some()
        && rest - look::PANEL_WIDTH >= COLUMN
    {
        look::PANEL_WIDTH
    } else {
        0.0
    };
    Layout {
        main: rest - column - panel,
        column,
        panes,
        panel,
    }
}

/// The narrowest the conversation and the column of panes get when the boundary is dragged.
pub const MIN_CHAT: f32 = 360.0;
pub const MIN_COLUMN: f32 = 280.0;
pub const MIN_PANE: f32 = 120.0;
/// The boundaries the person drags: between the conversation and the panes, and between
/// two stacked panes. Each is 2 px, Desktop's hairline plus the blue it turns under the
/// pointer.
pub const COLUMN_HANDLE: &str = "split:column";
pub const STACK_HANDLE: &str = "split:stack";
const HANDLE: f32 = 2.0;

/// The column's width for a share of the space right of the sidebar, within the minimums.
/// The sidebar's width with its border, nothing while it is hidden.
pub fn sidebar_width(ui: &Ui) -> f32 {
    if ui.sidebar_open {
        look::SIDEBAR_WIDTH
    } else {
        0.0
    }
}

pub fn column_width(rest: f32, fraction: f32) -> f32 {
    (rest * fraction).round().clamp(
        MIN_COLUMN.min(rest),
        (rest - MIN_CHAT).max(MIN_COLUMN.min(rest)),
    )
}

/// The heights of the panes in the column, top first: one fills it; two share it, evenly
/// until the person drags their boundary.
fn pane_heights(layout: &Layout, ui: &Ui, window: Size) -> Vec<f32> {
    let below = window.height - look::TOP_BAR - 1.0;
    if layout.panes.len() < 2 {
        return vec![below];
    }
    let room = below - HANDLE;
    let top = (room * ui.stack_fraction.unwrap_or(0.5))
        .round()
        .clamp(MIN_PANE, (room - MIN_PANE).max(MIN_PANE));
    vec![top, room - top]
}

/// A boundary: Desktop's hairline at rest, its blue under the pointer and while dragged.
fn divider(id: &str, across: bool, length: f32, palette: ComponentPalette) -> Element {
    let blue = ComponentPalette {
        hover: look::handle(),
        selected: look::handle(),
        ..palette
    };
    if across {
        let line = Element::row(vec![vline(length), hgap(HANDLE - 1.0)]);
        let handle = limn::ResizeHandle::vertical(id, blue)
            .thickness(HANDLE)
            .render();
        Element::overlay(line, handle).size(Some(HANDLE), Some(length))
    } else {
        let line = Element::column(vec![
            hline(length, look::pane_border()),
            Element::spacer().size(None, Some(HANDLE - 1.0)),
        ]);
        let handle = limn::ResizeHandle::horizontal(id, blue)
            .thickness(HANDLE)
            .render();
        Element::overlay(line, handle).size(Some(length), Some(HANDLE))
    }
}

/// The panes' width: the column less the boundary beside the conversation.
fn pane_width(layout: &Layout) -> f32 {
    if layout.main > 0.0 {
        layout.column - HANDLE
    } else {
        layout.column
    }
}

/// The files of the request the diff pane shows.
fn diff_files<'a>(store: &'a Store, ui: &Ui) -> Option<&'a Vec<FileEffect>> {
    let session = store.session(ui.selected?)?;
    // Without a request, the pane shows what is uncommitted, as Desktop's does.
    let Some((id, request)) = ui.diff.as_ref() else {
        return session
            .working
            .as_ref()
            .filter(|_| ui.pane_open(Pane::Diff));
    };
    let session = Some(session).filter(|s| s.id == *id)?;
    session.items.iter().find_map(|i| match i {
        Item::Changes {
            request: r, files, ..
        } if r == request => Some(files),
        _ => None,
    })
}

/// The columns and rows the terminal pane holds in `window`, for the host's shell.
pub fn terminal_grid(store: &Store, ui: &Ui, window: Size) -> (u16, u16) {
    let layout = layout(store, ui, window);
    let at = layout
        .panes
        .iter()
        .position(|p| *p == Pane::Terminal)
        .unwrap_or(0);
    let height = pane_heights(&layout, ui, window)[at.min(layout.panes.len().saturating_sub(1))];
    let inner = Size::new(
        pane_width(&layout) - 2.0 * TERMINAL_PADDING[0],
        height - look::PANE_HEADER - 1.0 - 2.0 * TERMINAL_PADDING[1],
    );
    let (cols, rows) = limn::TerminalView::grid_size(inner, TERMINAL_FONT);
    (cols as u16, rows as u16)
}

/// `child` with `x` of space on its sides and `y` above and below it.
fn pad(child: Element, x: f32, y: f32) -> Element {
    let gap = |w: Option<f32>, h: Option<f32>| Element::spacer().size(w, h);
    Element::column(vec![
        gap(None, Some(y)),
        Element::row(vec![
            gap(Some(x), None),
            child.flex(1.0),
            gap(Some(x), None),
        ])
        .flex(1.0),
        gap(None, Some(y)),
    ])
}

fn hgap(width: f32) -> Element {
    Element::spacer().size(Some(width), None)
}

fn vline(height: f32) -> Element {
    Element::spacer()
        .background(look::divider())
        .size(Some(1.0), Some(height))
}

fn hline(width: f32, color: limn::Color) -> Element {
    Element::spacer()
        .background(color)
        .size(Some(width), Some(1.0))
}

pub fn view(store: &Store, ui: &Ui, window: Size) -> Element {
    let palette = look::palette();
    let session = ui.selected.and_then(|id| store.session(id));
    let layout = layout(store, ui, window);
    let rest = layout.main + layout.column + layout.panel;
    let below = window.height - look::TOP_BAR - 1.0;
    let mut row = vec![];
    if layout.main > 0.0 {
        let column = COLUMN.min(layout.main - 64.0).max(240.0);
        let body = match session {
            Some(session) => transcript(session, ui, column, palette),
            None => empty(ui, column),
        };
        row.push(
            Element::column(vec![
                body.flex(1.0),
                composer(store, session, ui, column, palette).centered_x(),
                Element::spacer().size(None, Some(12.0)),
            ])
            .size(Some(layout.main), Some(below)),
        );
    }
    if !layout.panes.is_empty() {
        let heights = pane_heights(&layout, ui, window);
        let width = pane_width(&layout);
        let mut stack = vec![];
        for (i, (pane, height)) in layout.panes.iter().zip(heights).enumerate() {
            if i > 0 {
                stack.push(divider(STACK_HANDLE, false, width, palette));
            }
            stack.push(match pane {
                Pane::Diff => diff_pane(store, ui, width, height, palette),
                Pane::Terminal => terminal_pane(session, ui, width, height, palette),
                Pane::Agent => agent_pane(store, session, ui, width, height, palette),
                Pane::Plan => plan_pane(session, ui, width, height),
            });
        }
        if layout.main > 0.0 {
            row.push(divider(COLUMN_HANDLE, true, below, palette));
        }
        row.push(Element::column(stack).size(Some(width), Some(below)));
    }
    if let (Some(session), true) = (session, layout.panel > 0.0) {
        row.push(right_panel(session, ui, layout.panel, below, palette));
    }
    let content = Element::column(vec![
        top_bar(session, ui),
        hline(rest, look::pane_border()),
        Element::row(row).size(Some(rest), Some(below)),
    ])
    .size(Some(rest), Some(window.height));
    let mut root = vec![];
    if ui.sidebar_open {
        root.push(sidebar(store, ui, window.height, palette));
        root.push(vline(window.height));
    }
    root.push(content);
    let root = Element::row(root)
        .background(look::theme().background)
        .size(Some(window.width), Some(window.height));
    let root = search_dialog(store, ui, root, palette);
    // Desktop's own wording for both closes (Ion `cccc2cf0a`,) and for a delete
    //, less its vendor's servers: quill forgets only its own record.
    let tabs = &ui.terminal.tabs;
    let deleting = ui.deleting.and_then(|id| store.session(id));
    let (title, description, confirm) = match (ui.terminal.closing, deleting) {
        (_, Some(session)) => (
            "Delete session?",
            format!(
                "\u{201c}{}\u{201d} will be removed from quill. The agent's own transcript of it stays on this computer.",
                session_title(session)
            ),
            "Delete",
        ),
        (Some(crate::Closing::Others(keep)), None) => {
            let busy = tabs
                .iter()
                .enumerate()
                .filter(|(i, t)| *i != keep && t.busy)
                .count();
            let description = if busy == 1 {
                "A command is still running in one of the other terminals. Closing them stops it."
                    .to_string()
            } else {
                format!("Commands are still running in {busy} of the other terminals. Closing them stops those commands.")
            };
            (
                "Close other terminals?",
                description,
                "Close other terminals",
            )
        }
        (Some(crate::Closing::One(tab)), None) => (
            "Close terminal?",
            format!(
                "A command is still running in {}. Closing the terminal stops it.",
                tabs.get(tab).map_or("the terminal", |t| t.title.as_str())
            ),
            "Close terminal",
        ),
        (None, None) => ("", String::new(), ""),
    };
    let (confirm_id, cancel_id) = if deleting.is_some() {
        (DELETE_CONFIRM, DELETE_CANCEL)
    } else {
        (TERMINAL_CLOSE_CONFIRM, TERMINAL_CLOSE_CANCEL)
    };
    limn::AlertDialog::new(
        root,
        ui.terminal.closing.is_some() || deleting.is_some(),
        title,
        description,
        confirm_id,
        cancel_id,
        // Cancel is filled with `raised`: Desktop's light gray.
        ComponentPalette {
            raised: look::cancel(),
            ..palette
        },
    )
    .width(400.0)
    .confirm_label(confirm)
    .soft_destructive(
        look::danger_fill(),
        look::danger_border(),
        look::danger_ink(),
    )
    .cancel_variant(ButtonVariant::Secondary)
    .description_size(BODY)
    .description_line_height(20.0)
    .closable(false)
    // Desktop's backdrop is lighter, its title closer, its buttons 30 px with 14 px labels.
    .scrim(look::scrim())
    .title_gap(1.0)
    .button_sizing(limn::ButtonSizing {
        height: 30.0,
        font_size: BODY,
        horizontal_padding: 12.0,
        ..ControlSize::Compact.button_sizing()
    })
    .render()
}

/// A pane's header: what it shows on the left, its tools on the right, 44 px over a
/// hairline.
fn pane_header(lead: Element, tools: Vec<Element>, width: f32) -> Element {
    let mut row = vec![hgap(12.0), lead, Element::spacer().flex(1.0)];
    row.extend(tools);
    row.push(hgap(4.0));
    Element::column(vec![
        Element::row(row)
            .gap(8.0)
            .size(Some(width), Some(look::PANE_HEADER)),
        hline(width, look::pane_border()),
    ])
}

/// Expand and close, the tools every pane carries.
fn pane_tools(pane: Pane, ui: &Ui, palette: ComponentPalette) -> Vec<Element> {
    let expanded = ui.expanded_pane == Some(pane);
    let key = pane.key();
    vec![
        icon_tool(
            &format!("pane:{key}:expand"),
            if expanded {
                icons::of(icons::COLLAPSE)
            } else {
                icons::of(icons::EXPAND)
            },
            if expanded { "Collapse" } else { "Expand" },
            28.0,
            palette,
        ),
        icon_tool(
            &format!("pane:{key}:close"),
            icons::of(icons::CLOSE),
            "Close",
            28.0,
            palette,
        ),
    ]
}

/// The session's shells: "Terminal" and the folder while there is one, pills once
/// there are more, "New terminal" after them.
fn terminal_pane(
    session: Option<&Session>,
    ui: &Ui,
    width: f32,
    height: f32,
    palette: ComponentPalette,
) -> Element {
    let new = icon_tool(
        TERMINAL_NEW,
        icons::of(icons::ADD),
        "New terminal",
        28.0,
        palette,
    );
    let tabs = &ui.terminal.tabs;
    let folder = folder_name(session.map_or(&ui.cwd, |s| &s.cwd));
    let lead = if tabs.len() <= 1 {
        let title = match tabs.first() {
            Some(tab) if tab.named => tab.title.clone(),
            _ => "Terminal".into(),
        };
        let label = Element::row(vec![
            text(title).centered_y(),
            muted(folder.clone()).centered_y(),
        ])
        .gap(8.0);
        let label = match ui.terminal.renaming {
            Some((0, ref name)) => rename_field(TERMINAL_RENAME, name, 160.0, palette),
            _ => tab_menu(0, 1, label, palette),
        };
        Element::row(vec![label, Element::spacer().size(Some(4.0), None), new]).gap(8.0)
    } else {
        let mut pills: Vec<Element> = tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                if let Some((at, name)) = &ui.terminal.renaming {
                    if *at == i {
                        return rename_field(TERMINAL_RENAME, name, 160.0, palette);
                    }
                }
                // "Terminal 2  work ×": a named tab drops the folder.
                let mut items = vec![hgap(8.0), text(tab.title.clone()).centered_y()];
                if !tab.named {
                    items.push(muted(folder.clone()).centered_y());
                }
                items.push(icon_tool(
                    &format!("terminal:close:{i}"),
                    icons::of(icons::CLOSE),
                    "Close terminal",
                    20.0,
                    palette,
                ));
                let pill = Element::row(items)
                    .gap(6.0)
                    .size(None, Some(28.0))
                    .radius(8.0);
                let pill = if i == ui.terminal.active {
                    pill.background(look::active())
                } else {
                    pill
                };
                let pill = Element::clickable(format!("terminal:tab:{i}"), pill).centered_y();
                tab_menu(i, tabs.len(), pill, palette)
            })
            .collect();
        pills.push(new);
        Element::row(pills).gap(4.0)
    };
    let header = pane_header(lead, pane_tools(Pane::Terminal, ui, palette), width);
    let body = match &ui.terminal.screen {
        Some(screen) => pad(
            // The view draws its cursor in the palette's text: Desktop's is blue.
            limn::TerminalView::new(
                TERMINAL,
                screen.clone(),
                ComponentPalette {
                    text: look::cursor(),
                    ..palette
                },
            )
            .font_size(TERMINAL_FONT)
            .background(look::theme().background)
            // Desktop's terminal marks its focus with the cursor alone.
            .focus_ring(false)
            .render(),
            TERMINAL_PADDING[0],
            TERMINAL_PADDING[1],
        ),
        None => Element::spacer(),
    };
    Element::column(vec![header, body.flex(1.0)]).size(Some(width), Some(height))
}

/// A tab's right-click menu, Desktop's four actions: rename, clear, close, and
/// close the others when there are others.
fn tab_menu(tab: usize, count: usize, child: Element, palette: ComponentPalette) -> Element {
    use limn::{ContextMenuEntry, ContextMenuItem};
    let item = |action: &str, label: &str| {
        ContextMenuEntry::Item(ContextMenuItem::new(
            format!("terminal:menu:{action}:{tab}"),
            label,
        ))
    };
    let mut entries = vec![
        item("rename", "Rename terminal"),
        item("clear", "Clear terminal"),
        ContextMenuEntry::Separator,
        item("close", "Close terminal"),
    ];
    if count > 1 {
        entries.push(item("others", "Close other terminals"));
    }
    limn::ContextMenu::new(format!("terminal:menu:{tab}"), child, entries, palette).render()
}

/// The name field a pill or the session's title turns into while the person renames it
///.
fn rename_field(id: &str, name: &str, width: f32, palette: ComponentPalette) -> Element {
    limn::TextField::new(id, name, "", palette)
        .size(ControlSize::Compact)
        .width(width)
        .render()
        .border(look::card_border(), 1.0)
        .focus_border(look::handle(), 1.0)
        .radius(8.0)
        .centered_y()
}

/// A square icon button with the palette's hover and press, as limn's own tools draw them.
fn icon_tool(
    id: &str,
    icon: limn::Icon,
    label: &str,
    side: f32,
    _palette: ComponentPalette,
) -> Element {
    tool_surface(id, icon, label, side, None, look::tool_hover(), None).centered_y()
}

/// An icon button as Desktop draws them: rounded 6, its icon in the text's ink,
/// `hover` under the pointer, an optional fill at rest, and a tooltip with its shortcut.
fn tool_surface(
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

/// A top-bar toggle: its icon on the active fill while what it shows is open.
fn toggle(id: &str, icon: limn::Icon, label: &str, on: bool, shortcut: Option<&str>) -> Element {
    let (fill, hover) = if on {
        (Some(look::active()), look::active_hover())
    } else {
        (None, look::tool_hover())
    };
    tool_surface(id, icon, label, 26.0, fill, hover, shortcut).centered_y()
}

/// A session's name: the person's, the agent's or its first message, else "New session".
fn session_title(session: &Session) -> String {
    if session.title.is_empty() {
        "New session".into()
    } else {
        session.title.clone()
    }
}

/// The sidebar's toggle, "Hide sidebar Ctrl+B"; it stays at the bar's start while the sidebar
/// is hidden, on the active fill.
fn sidebar_toggle(ui: &Ui) -> Element {
    toggle(
        SIDEBAR_TOGGLE,
        icons::of(icons::SIDEBAR),
        if ui.sidebar_open {
            "Hide sidebar"
        } else {
            "Show sidebar"
        },
        !ui.sidebar_open,
        Some("Ctrl+B"),
    )
}

/// "Open in ›": of Desktop's places, the one quill has, the desktop's
/// file manager showing the session's folder.
fn open_in(session: quill_model::SessionId) -> limn::DropdownMenuEntry {
    limn::DropdownMenuItem::new(format!("open-in:{session}"), "Open in")
        .submenu(vec![
            limn::DropdownMenuEntry::Heading("Open in".into()),
            limn::DropdownMenuItem::new(format!("open-in:files:{session}"), "File manager").into(),
        ])
        .into()
}

/// A session menu's last rows, Archive (or Unarchive) and the red Delete.
fn archive_and_delete(session: &Session) -> [limn::DropdownMenuEntry; 2] {
    [
        lettered(
            format!("archive:{}", session.id),
            if session.archived {
                "Unarchive"
            } else {
                "Archive"
            },
            "A",
        )
        .into(),
        lettered(format!("delete:{}", session.id), "Delete", "D")
            .destructive(look::danger_ink())
            .into(),
    ]
}

/// A menu row with Desktop's one-letter shortcut.
fn lettered(id: String, label: &str, key: &str) -> limn::DropdownMenuItem {
    limn::DropdownMenuItem::new(id, label).shortcut(key)
}

/// The session's title, Stop while it works, and the toggles of the panel and the panes
///. The title's chevron opens its menu, and Rename turns it into a field
///.
fn top_bar(session: Option<&Session>, ui: &Ui) -> Element {
    // An expanded pane leaves the bar empty, as Desktop's does.
    if ui.expanded_pane.is_some() {
        return Element::spacer().size(None, Some(look::TOP_BAR));
    }
    let mut row = if ui.sidebar_open {
        vec![hgap(16.0)]
    } else {
        vec![hgap(9.0), sidebar_toggle(ui), hgap(12.0)]
    };
    match (session, &ui.title_edit) {
        (Some(_), Some(value)) => {
            row.push(rename_field(TITLE_FIELD, value, 320.0, look::palette()))
        }
        (Some(s), None) => {
            let title = session_title(s);
            let open = ui.open_menu == Some(TITLE_MENU);
            let chevron = tool_surface(
                TITLE_MENU,
                icons::of(icons::CARET_DOWN),
                &format!("More options for {title}"),
                22.0,
                open.then(look::active),
                look::tool_hover(),
                None,
            );
            let mut entries = vec![open_in(s.id), limn::DropdownMenuEntry::Separator];
            entries.extend([
                lettered(format!("rename:{}", s.id), "Rename", "R").into(),
                lettered(format!("fork:{}", s.id), "Fork", "F").into(),
                limn::DropdownMenuEntry::Separator,
            ]);
            entries.extend(archive_and_delete(s));
            row.push(text(title).centered_y());
            row.push(
                menu(
                    chevron,
                    entries,
                    open,
                    TITLE_MENU,
                    160.0,
                    limn::PopoverPlacement::BottomStart,
                )
                .centered_y(),
            );
        }
        (None, _) => row.push(muted("New session").centered_y()),
    }
    row.push(Element::spacer().flex(1.0));
    // Stop lives in the composer, as in Desktop.
    if session.is_some() {
        let shown = ui.panel_open && ui.panes.is_empty();
        row.push(toggle(
            PANEL_TOGGLE,
            icons::of(icons::PANEL_RIGHT),
            if shown {
                "Hide session rail"
            } else {
                "Show session rail"
            },
            shown,
            None,
        ));
        // A hairline parts the rail's toggle from the panes'.
        row.push(vline(16.0).centered_y());
    }
    row.push(toggle(
        TERMINAL_TOGGLE,
        icons::of(icons::TERMINAL),
        "Terminal",
        ui.pane_open(Pane::Terminal),
        Some("Ctrl+`"),
    ));
    if session.is_some() {
        row.push(toggle(
            DIFF_TOGGLE,
            icons::of(icons::DIFF),
            "Changes",
            ui.pane_open(Pane::Diff),
            Some("Ctrl+Shift+D"),
        ));
    }
    row.push(hgap(8.0));
    Element::row(row).gap(4.0).size(None, Some(look::TOP_BAR))
}

/// Desktop's sidebar: its toggle, New and Search, the pinned
/// sessions, then the rest under a header per folder that folds them, the folder with the
/// newest session first.
fn sidebar(store: &Store, ui: &Ui, height: f32, palette: ComponentPalette) -> Element {
    let nav = |id: &str, icon: &'static str, label: &str, keys: &str, selected: bool| {
        side_row(
            id.into(),
            vec![
                Element::icon(icons::of(icon))
                    .foreground(look::palette().text)
                    .size(Some(16.0), Some(16.0))
                    .centered_y(),
                text(label).centered_y(),
                Element::spacer().flex(1.0),
                muted(keys)
                    .font_size(12.0)
                    .centered_y()
                    .reveal_on_hover(format!("{id}:keys")),
            ],
            selected,
            palette,
        )
    };
    let mut rows = vec![
        // Desktop's band holds its menu, this toggle and its history; quill keeps the toggle,
        // centered on the top bar's middle.
        Element::row(vec![hgap(5.0), sidebar_toggle(ui)]).size(None, Some(look::TOP_BAR - 16.0)),
        Element::spacer().size(None, Some(12.0)),
        // New is filled while its page shows, as Desktop's is.
        nav(
            NEW_SESSION,
            icons::ADD,
            "New",
            "Ctrl+N",
            ui.selected.is_none(),
        ),
        nav(SEARCH, icons::SEARCH, "Search", "Ctrl+K", false),
        Element::spacer().size(None, Some(16.0)),
    ];
    let listed = |pinned: bool| {
        sorted(
            store
                .sessions
                .iter()
                .filter(|s| s.pinned == pinned && !s.archived)
                .collect(),
            ui,
        )
    };
    let pinned = listed(true);
    if !pinned.is_empty() {
        // "Pinned" folds under the empty folder, which no session works in.
        let key = std::path::PathBuf::new();
        let open = !ui.collapsed.contains(&key);
        rows.push(group_header("Pinned", &key, open, None, ui, palette));
        if open {
            rows.extend(pinned.iter().map(|s| session_row(s, ui, palette)));
        }
        rows.push(Element::spacer().size(None, Some(8.0)));
    }
    // Each group in the order of its first session, so the one with the newest leads.
    let rest = listed(false);
    let mut groups: Vec<(String, std::path::PathBuf, Option<&std::path::Path>)> = vec![];
    for session in &rest {
        let group = match ui.group_by {
            crate::GroupBy::Folder => (
                folder_name(&session.cwd),
                session.cwd.clone(),
                Some(session.cwd.as_path()),
            ),
            crate::GroupBy::Status => {
                let label = status_group(session);
                (label.into(), format!("status:{label}").into(), None)
            }
            // One header still holds the filter menu (inferred: Desktop's was not captured).
            crate::GroupBy::None => ("Sessions".into(), "all:".into(), None),
        };
        if !groups.iter().any(|g| g.1 == group.1) {
            groups.push(group);
        }
    }
    if ui.group_by == crate::GroupBy::Status {
        groups.sort_by_key(|g| STATUS_GROUPS.iter().position(|s| g.0 == *s));
    }
    for (label, key, folder) in groups {
        let open = !ui.collapsed.contains(&key);
        rows.push(group_header(&label, &key, open, folder, ui, palette));
        if !open {
            continue;
        }
        rows.extend(
            rest.iter()
                .filter(|s| match ui.group_by {
                    crate::GroupBy::Folder => s.cwd == key,
                    crate::GroupBy::Status => status_group(s) == label,
                    crate::GroupBy::None => true,
                })
                .map(|s| session_row(s, ui, palette)),
        );
    }
    Element::column(rows)
        .gap(2.0)
        .padding(8.0)
        .background(look::sidebar())
        .size(Some(look::SIDEBAR_WIDTH - 1.0), Some(height))
}

/// Desktop's buckets when the list is grouped by status, in its order, less
/// "done", which quill has no way to mark.
const STATUS_GROUPS: [&str; 4] = ["Blocked", "Working", "Ready", "Idle"];

fn status_group(session: &Session) -> &'static str {
    match session.status {
        SessionStatus::Awaiting => STATUS_GROUPS[0],
        SessionStatus::Running | SessionStatus::Starting => STATUS_GROUPS[1],
        _ if session.unread => STATUS_GROUPS[2],
        _ => STATUS_GROUPS[3],
    }
}

/// The sessions in the order the filter menu asks: by last change, by when they were made,
/// or by name, newest or Z first unless ascending.
fn sorted<'a>(mut list: Vec<&'a Session>, ui: &Ui) -> Vec<&'a Session> {
    match ui.sort_by {
        crate::SortBy::Updated => list.sort_by_key(|s| (s.touched, s.id)),
        crate::SortBy::Created => list.sort_by_key(|s| s.id),
        crate::SortBy::Name => list.sort_by_key(|s| session_title(s).to_lowercase()),
    }
    if !ui.ascending {
        list.reverse();
    }
    list
}

/// The filter menu of a group's header: Group by, Sort by and Reset to
/// defaults, with the current choices checked. Desktop's Show, New section and View all,
/// and grouping by date or environment, are not here: quill keeps no dates or sections and
/// runs on this computer only.
fn filter_menu(key: &str, ui: &Ui) -> Element {
    let id = format!("filter-menu:{key}");
    let open = ui.filter_menu.as_deref() == Some(key);
    let button = tool_surface(
        &id,
        icons::of(icons::FILTER),
        "Group and sort",
        22.0,
        open.then(look::active),
        look::tool_hover(),
        None,
    )
    .centered_y();
    let button = if open {
        button
    } else {
        button.reveal_on_hover(format!("{id}:reveal"))
    };
    let check = |id: &str, label: &str, on: bool| {
        limn::DropdownMenuEntry::from(limn::DropdownMenuItem::new(id, label).selected(on))
    };
    use crate::{GroupBy, SortBy};
    let group = match ui.group_by {
        GroupBy::Folder => "Folder",
        GroupBy::Status => "Status",
        GroupBy::None => "None",
    };
    let sort = match ui.sort_by {
        SortBy::Updated => "Last updated",
        SortBy::Created => "Date created",
        SortBy::Name => "Name",
    };
    let entries = vec![
        limn::DropdownMenuItem::new("filter:group", "Group by")
            .annotation(group)
            .submenu(vec![
                check("group-by:status", "Status", ui.group_by == GroupBy::Status),
                check("group-by:folder", "Folder", ui.group_by == GroupBy::Folder),
                limn::DropdownMenuEntry::Separator,
                check("group-by:none", "None", ui.group_by == GroupBy::None),
            ])
            .into(),
        limn::DropdownMenuItem::new("filter:sort", "Sort by")
            .annotation(sort)
            .submenu(vec![
                check(
                    "sort-by:updated",
                    "Last updated",
                    ui.sort_by == SortBy::Updated,
                ),
                check(
                    "sort-by:created",
                    "Date created",
                    ui.sort_by == SortBy::Created,
                ),
                check("sort-by:name", "Name", ui.sort_by == SortBy::Name),
                limn::DropdownMenuEntry::Separator,
                check("sort-way:descending", "Descending", !ui.ascending),
                check("sort-way:ascending", "Ascending", ui.ascending),
            ])
            .into(),
        limn::DropdownMenuEntry::Separator,
        limn::DropdownMenuItem::new(FILTER_RESET, "Reset to defaults")
            .enabled((ui.group_by, ui.sort_by, ui.ascending) != Default::default())
            .into(),
    ];
    menu(
        button,
        entries,
        open,
        &id,
        220.0,
        // Its right edge under the button's, as Desktop's.
        limn::PopoverPlacement::BottomEnd,
    )
    .centered_y()
}

/// A group's header, which folds it: under the pointer it fills, and offers its filter menu
/// and, for a folder, a new session there.
fn group_header(
    label: &str,
    key: &std::path::Path,
    open: bool,
    folder: Option<&std::path::Path>,
    ui: &Ui,
    palette: ComponentPalette,
) -> Element {
    let folder_key = key;
    let caret = Element::icon(if open {
        icons::of(icons::CARET_DOWN)
    } else {
        icons::of(icons::CARET_RIGHT)
    })
    .foreground(look::muted())
    .size(Some(10.0), Some(10.0))
    .centered_y();
    let mut items = vec![
        hgap(4.0),
        muted(label).font_size(SMALL).centered_y(),
        caret,
        Element::spacer().flex(1.0),
    ];
    // Every header but Pinned's offers the list's filter menu.
    if !key.as_os_str().is_empty() {
        items.push(filter_menu(&key.display().to_string(), ui));
    }
    if let Some(folder) = folder {
        let id = format!("new-in:{}", folder.display());
        items.push(
            tool_surface(
                &id,
                icons::of(icons::ADD),
                "New session",
                22.0,
                None,
                look::tool_hover(),
                None,
            )
            .centered_y()
            .reveal_on_hover(format!("{id}:reveal")),
        );
    }
    items.push(hgap(4.0));
    let header = Element::row(items).gap(6.0).size(None, Some(28.0));
    ComponentPalette {
        hover: look::row_hover(),
        ..palette
    }
    .interactive_surface(Element::clickable(
        format!("group:{}", folder_key.display()),
        header,
    ))
    .radius(8.0)
    .hover_region()
}

/// A session's row: its title, its status, and under the pointer "···" with Desktop's
/// Pin, Rename and Delete.
fn session_row(session: &Session, ui: &Ui, palette: ComponentPalette) -> Element {
    let id = format!("row-menu:{}", session.id);
    let open = ui.row_menu == Some(session.id);
    let dots = tool_surface(
        &id,
        icons::of(icons::DOTS_HORIZONTAL),
        "More options",
        24.0,
        open.then(look::active),
        look::tool_hover(),
        None,
    );
    let mut entries = vec![
        open_in(session.id),
        limn::DropdownMenuEntry::Separator,
        lettered(
            format!("pin:{}", session.id),
            if session.pinned { "Unpin" } else { "Pin" },
            "P",
        )
        .into(),
        lettered(
            format!("unread:{}", session.id),
            if session.unread {
                "Mark as read"
            } else {
                "Mark as unread"
            },
            "U",
        )
        .into(),
        lettered(format!("rename:{}", session.id), "Rename", "R").into(),
        lettered(format!("fork:{}", session.id), "Fork", "F").into(),
        limn::DropdownMenuEntry::Separator,
    ];
    entries.extend(archive_and_delete(session));
    let dots = menu(
        dots,
        entries,
        open,
        &id,
        180.0,
        limn::PopoverPlacement::BottomEnd,
    )
    .centered_y();
    let dots = if open {
        dots
    } else {
        dots.reveal_on_hover(format!("{id}:reveal"))
    };
    let title = Element::truncated_text(session_title(session))
        .font_family(FontFamily::Sans)
        .font_size(BODY)
        .foreground(look::palette().text)
        .flex(1.0)
        .centered_y();
    side_row_with(
        format!("session:{}", session.id),
        vec![title, status_dot(session), dots],
        ui.selected == Some(session.id),
        Some(&id),
        palette,
    )
}

/// The sessions whose title holds `query`, as the search lists them: those that need the
/// person, those at work, then the rest, newest first.
pub fn found(store: &Store, query: &str) -> Vec<quill_model::SessionId> {
    let query = query.to_lowercase();
    let mut found: Vec<&Session> = store
        .sessions
        .iter()
        .rev()
        .filter(|s| session_title(s).to_lowercase().contains(&query))
        .collect();
    found.sort_by_key(|s| search_section(s));
    found.into_iter().map(|s| s.id).collect()
}

/// Where the search lists a session; an archived one is found only here, at the end.
fn search_section(session: &Session) -> usize {
    match session.status {
        _ if session.archived => 3,
        SessionStatus::Awaiting => 0,
        SessionStatus::Running | SessionStatus::Starting => 1,
        _ => 2,
    }
}

/// Desktop's search, Ctrl+K: a field, the sessions under "Needs
/// attention", "Active" and "Recents", and "New session"; Enter opens the first, marked ↵.
fn search_dialog(store: &Store, ui: &Ui, base: Element, palette: ComponentPalette) -> Element {
    let Some(query) = &ui.search else {
        return base;
    };
    let field = limn::TextField::new(SEARCH_FIELD, query.as_str(), "Search", palette)
        .render()
        .size(None, Some(36.0));
    let mut body = vec![field, Element::spacer().size(None, Some(6.0))];
    let heading = |label: &str| {
        pad(muted(label).font_size(SMALL), 12.0, 0.0)
            .centered_y()
            .size(None, Some(28.0))
    };
    let row = |id: String, label: String, first: bool| {
        let mut items = vec![
            hgap(12.0),
            text(label).centered_y(),
            Element::spacer().flex(1.0),
        ];
        if first {
            items.push(
                Element::icon(icons::of(icons::RETURN))
                    .foreground(look::muted())
                    .size(Some(14.0), Some(14.0))
                    .centered_y(),
            );
        }
        items.push(hgap(12.0));
        let row = ComponentPalette {
            hover: look::row_hover(),
            ..palette
        }
        .interactive_surface(Element::clickable(
            id,
            Element::row(items).gap(8.0).size(None, Some(36.0)),
        ))
        .radius(8.0);
        if first {
            row.background(look::selected())
        } else {
            row
        }
    };
    let found = found(store, query);
    let mut section = None;
    for (i, id) in found.iter().enumerate() {
        let Some(session) = store.session(*id) else {
            continue;
        };
        let at = search_section(session);
        if section != Some(at) {
            section = Some(at);
            body.push(heading(
                ["Needs attention", "Active", "Recents", "Archived"][at],
            ));
        }
        body.push(row(format!("search:{id}"), session_title(session), i == 0));
    }
    body.push(heading("Actions"));
    body.push(row(
        SEARCH_NEW.into(),
        "New session".into(),
        found.is_empty(),
    ));
    let footer = Element::row(vec![
        muted("Close").font_size(SMALL).centered_y(),
        limn::Kbd::new("Esc", palette).render().centered_y(),
    ])
    .gap(6.0);
    limn::Modal::new(base, true, "", "", SEARCH_CLOSE, palette)
        .width(672.0)
        .body(Element::column(body).gap(2.0))
        .footer(footer)
        .initial_focus(SEARCH_FIELD)
        .backdrop_dismiss(true)
        .scrim(look::scrim())
        .render()
}

/// A sidebar row: 32 px tall, 10 px in, rounded, filled while selected.
fn side_row(
    id: String,
    content: Vec<Element>,
    selected: bool,
    palette: ComponentPalette,
) -> Element {
    side_row_with(id, content, selected, None, palette)
}

/// A sidebar row that stays lit while the pointer is on `peer`, its "···", as Desktop's
/// keeps both the row and the button lit [the person, 2026-10-25].
fn side_row_with(
    id: String,
    content: Vec<Element>,
    selected: bool,
    peer: Option<&str>,
    palette: ComponentPalette,
) -> Element {
    // Empty ends: the row's own gap is the 10 px inset, as Desktop's.
    let mut items = vec![hgap(0.0)];
    items.extend(content);
    items.push(hgap(0.0));
    let row = Element::row(items).gap(10.0).size(None, Some(32.0));
    // Under the pointer a row turns #EDEDED, a selected one from #E0E0E0 to #D5D5D5
    //.
    let palette = ComponentPalette {
        hover: if selected {
            look::selected_hover()
        } else {
            look::row_hover()
        },
        ..palette
    };
    let row = palette.interactive_surface(Element::clickable(id, row));
    let row = match peer {
        Some(peer) => row.hover_with(peer),
        None => row,
    }
    .radius(8.0)
    .hover_region();
    if selected {
        row.background(look::selected())
    } else {
        row
    }
}

/// Running and awaiting sessions carry a dot in the sidebar, as Desktop's rows do.
fn status_dot(session: &Session) -> Element {
    let status = &session.status;
    let color = match status {
        SessionStatus::Running | SessionStatus::Starting => look::running(),
        SessionStatus::Awaiting => look::awaiting(),
        SessionStatus::Failed(_) => look::removed(),
        // Unread reads as a small gray dot.
        _ if session.unread => look::muted(),
        _ => return Element::spacer().size(Some(6.0), Some(6.0)),
    };
    // A running session's dot pulses, as Desktop's does.
    let running = matches!(status, SessionStatus::Running | SessionStatus::Starting);
    limn::StatusDot::new(color)
        .pulse(running)
        .size(6.0)
        .render()
        .centered_y()
}

/// A new session's page greets the person at the top, as their Desktop does
///: an orange spark and "What's up next, Name?", 20 px.
fn empty(ui: &Ui, column: f32) -> Element {
    let greeting = match &ui.name {
        Some(name) => format!("What's up next, {name}?"),
        None => "What's up next?".into(),
    };
    let line = Element::row(vec![
        Element::icon(icons::of(icons::SPARK))
            .foreground(look::spark())
            .size(Some(24.0), Some(24.0))
            .centered_y(),
        text(greeting).font_size(20.0).centered_y(),
    ])
    .gap(8.0)
    .size(Some(column), Some(30.0));
    Element::column(vec![
        Element::spacer().size(None, Some(16.0)),
        line.centered_x(),
    ])
}

fn transcript(session: &Session, ui: &Ui, column: f32, palette: ComponentPalette) -> Element {
    // Desktop opens a session with where it started.
    let mut items: Vec<Element> = vec![muted(format!(
        "Started on this computer · {}",
        folder_name(&session.cwd)
    ))
    .font_size(12.0)];
    let busy = matches!(
        session.status,
        SessionStatus::Starting | SessionStatus::Running | SessionStatus::Awaiting
    );
    let (entries, running) = entries(session, &session.items, busy, ui, column, palette);
    items.extend(entries);
    // While the turn runs, its task list sits over the running line; it goes when the
    // turn ends.
    let list = todos(&session.items);
    if busy && !list.is_empty() {
        items.push(task_card(&list, column, palette));
    }
    if let Some(status) = status_line(session, running.as_ref(), ui, column, palette) {
        items.push(status);
    }
    let content = Element::column(items)
        .gap(14.0)
        .size(Some(column), None)
        .padding(16.0)
        .centered_x();
    let scroll = Element::scroll(format!("transcript:{}", session.id), content);
    if ui.at_end {
        return scroll;
    }
    // Away from the end, Desktop floats a round "↓" over the conversation.
    let down = ComponentPalette {
        hover: look::tool_hover(),
        ..palette
    }
    .interactive_surface(Element::icon_button(
        SCROLL_END,
        icons::of(icons::ARROW_DOWN),
        "Scroll to bottom",
    ))
    .foreground(look::palette().text)
    .background(palette.surface)
    .border(look::card_border(), 1.0)
    .radius(16.0)
    .size(Some(32.0), Some(32.0));
    let float = Element::column(vec![
        Element::spacer().flex(1.0),
        Element::row(vec![
            Element::spacer().flex(1.0),
            down,
            Element::spacer().flex(1.0),
        ]),
        Element::spacer().size(None, Some(12.0)),
    ]);
    Element::overlay(scroll, float)
}

/// What a list of transcript items shows: messages, cards, and tool calls in a row folded
/// into one group, as Desktop shows a turn's work; an answered permission
/// between them is gone, a waiting one ends the group. While `busy`, a last step still
/// running leaves its group and is returned for the running line.
fn entries<'a>(
    session: &'a Session,
    list: &'a [Item],
    busy: bool,
    ui: &Ui,
    column: f32,
    palette: ComponentPalette,
) -> (Vec<Element>, Option<Step<'a>>) {
    let mut items: Vec<Element> = vec![];
    let mut steps: Vec<Step> = vec![];
    let flush = |steps: &mut Vec<Step>, items: &mut Vec<Element>| {
        if !steps.is_empty() {
            items.push(step_group(steps, ui, column, palette));
            steps.clear();
        }
    };
    let main = std::ptr::eq(list, session.items.as_slice());
    let mut waiting = None;
    for (at, item) in list.iter().enumerate() {
        match item {
            Item::Tool { name, .. } if is_task_tool(name) => {}
            Item::Tool {
                call,
                name,
                input,
                result,
            } => {
                let step = Step {
                    call,
                    name,
                    input,
                    result: result.as_ref(),
                    running: result.is_none() || session.task_running(call),
                };
                if stands_alone(name) {
                    flush(&mut steps, &mut items);
                    if let Some(element) = lone_step(&step, column, palette) {
                        items.push(element);
                    }
                } else {
                    steps.push(step);
                }
            }
            Item::Permission {
                decided: Some(_), ..
            } => {}
            other => {
                // A step that waits on this permission runs under its card
                //.
                if busy
                    && matches!(other, Item::Permission { decided: None, .. })
                    && steps.last().is_some_and(|s| s.result.is_none())
                {
                    waiting = steps.pop();
                }
                flush(&mut steps, &mut items);
                // Desktop sets each message apart: about 60 px between turns and
                // 39 px under the person's words; the gap adds to the column's own 14.
                let user = matches!(other, Item::User { .. });
                if user && !items.is_empty() {
                    items.push(Element::spacer().size(None, Some(32.0)));
                }
                let element = self::item(other, session, ui, column, palette);
                // Under a reply, its actions show while the pointer is on it.
                let element = match other {
                    Item::Assistant {
                        streaming: false, ..
                    } if main => {
                        let copy = format!("copy:{}:{at}", session.id);
                        Element::column(vec![
                            element,
                            tool_surface(
                                &copy,
                                icons::of(icons::COPY),
                                "Copy",
                                24.0,
                                None,
                                look::tool_hover(),
                                None,
                            )
                            .reveal_on_hover(format!("{copy}:reveal")),
                        ])
                        .gap(4.0)
                        .hover_region()
                    }
                    _ => element,
                };
                items.push(element);
                if user {
                    items.push(Element::spacer().size(None, Some(11.0)));
                }
            }
        }
    }
    let running = if busy && steps.last().is_some_and(|s| s.result.is_none()) {
        steps.pop()
    } else {
        waiting
    };
    flush(&mut steps, &mut items);
    (items, running)
}

/// The line under the turn: Desktop's orange dots, then the step running, or the
/// turn's clock, its tokens, the work running beside it and whether the model thinks, muted,
/// at the transcript's size. Waiting on the person it says so; with only background work
/// left, it counts it.
fn status_line(
    session: &Session,
    running: Option<&Step>,
    ui: &Ui,
    column: f32,
    palette: ComponentPalette,
) -> Option<Element> {
    let dots = || {
        Spinner::new("status:dots", look::spark())
            .variant(limn::SpinnerVariant::Dots)
            .size(16.0)
            .decorative()
            .render()
            .centered_y()
    };
    let tasks = session.tasks.iter().filter(|t| t.running).count();
    let tasks = match tasks {
        0 => None,
        1 => Some("1 running task".to_owned()),
        n => Some(format!("{n} running tasks")),
    };
    Some(match &session.status {
        // "Reading saludo.py 2s ▸": the verb muted, what it acts on in the text's ink, and
        // the step's own clock; under a permission it waits on,
        // the same.
        SessionStatus::Starting | SessionStatus::Running | SessionStatus::Awaiting
            if running.is_some() =>
        {
            let step = running?;
            let key = format!("tool:{}", step.call);
            let open = ui.expanded.iter().any(|c| c == step.call);
            let (verb, object) = step_label(step, false);
            let mut line = vec![dots(), muted(verb).font_size(CHAT).centered_y()];
            if !object.is_empty() {
                line.push(text(object).font_size(CHAT).centered_y());
            }
            let since = session.step_since.map_or(0, |t| t.elapsed().as_secs());
            line.push(muted(elapsed(since)).font_size(CHAT).centered_y());
            line.push(caret(open));
            let line = Element::clickable(key, Element::row(line).gap(8.0));
            if !open {
                return Some(line);
            }
            Element::column(vec![line, step_body(step, column, palette)]).gap(8.0)
        }
        SessionStatus::Starting | SessionStatus::Running => {
            // The clock shows from five seconds on.
            let secs = session.running_since.map_or(0, |t| t.elapsed().as_secs());
            let mut parts = vec![];
            if secs >= 5 {
                parts.push(elapsed(secs));
            }
            if session.tokens > 0 {
                parts.push(tokens(session.tokens));
            }
            parts.extend(tasks);
            if session.thinking {
                parts.push("Thinking".into());
            }
            Element::row(vec![
                dots(),
                muted(parts.join(" · ")).font_size(CHAT).centered_y(),
            ])
            .gap(12.0)
        }
        // A ringed orange dot and a white pill with an orange edge.
        SessionStatus::Awaiting => Element::row(vec![
            Element::panel(
                Element::spacer()
                    .background(look::awaiting())
                    .radius(4.0)
                    .size(Some(8.0), Some(8.0)),
            )
            .border(look::awaiting_edge(), 2.0)
            .radius(7.0)
            .padding(1.0)
            .centered_y(),
            Element::panel(Element::row(vec![
                hgap(5.0),
                text("Needs your input")
                    .font_size(12.0)
                    .foreground(look::manual()),
                hgap(5.0),
            ]))
            .border(look::awaiting_edge(), 1.0)
            .radius(10.0)
            .padding(2.0)
            .centered_y(),
        ])
        .gap(8.0),
        SessionStatus::Failed(error) => text(error.clone()).foreground(look::removed()),
        SessionStatus::Idle | SessionStatus::Exited => {
            let tasks = tasks?;
            TextShimmer::new("status:tasks", tasks, palette)
                .render()
                .font_size(CHAT)
        }
    })
}

/// A turn's running time as Desktop writes it: "14s", "9m 4s", "1h 2m 3s".
pub fn elapsed(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m {}s", secs / 3600, secs % 3600 / 60, secs % 60),
    }
}

/// A token count as the running line writes it: "140 tokens", "30.7k tokens"
///.
pub fn tokens(count: u64) -> String {
    match count {
        0..1000 => format!("{count} tokens"),
        // shortcut: the millions form is a guess; Desktop's turns were never seen that long.
        1000..1_000_000 => format!("{:.1}k tokens", count as f64 / 1000.0),
        _ => format!("{:.1}M tokens", count as f64 / 1_000_000.0),
    }
}

fn item(
    item: &Item,
    session: &Session,
    ui: &Ui,
    column: f32,
    palette: ComponentPalette,
) -> Element {
    match item {
        Item::User { text: t, .. } => {
            // As wide as its words, up to 3/4 of the column, 16 px of side padding.
            let line = text(t.clone())
                .font_size(CHAT)
                .max_width(column * 0.75 - 32.0);
            let bubble = Element::panel(Element::row(vec![hgap(4.0), line, hgap(4.0)]))
                .background(look::user_bubble())
                .radius(14.0)
                .padding(12.0);
            Element::row(vec![Element::spacer().flex(1.0), bubble])
        }
        Item::Assistant { text: t, streaming } => {
            // Desktop's code: blocks in a white bordered box as wide as their
            // code, no language line, a copy button, 13 px; inline code on a light fill.
            let code_palette = ComponentPalette {
                raised: look::palette().surface,
                border: look::divider(),
                ..palette
            };
            let colors = limn::MarkdownColors {
                code_ink: look::palette().text,
                code_fill: look::command_box(),
                ..limn::MarkdownColors::for_palette(code_palette)
            };
            Markdown::new(t, code_palette)
                .width(column)
                .font_size(CHAT)
                .colors(colors)
                .code_label(false)
                .code_fit_content(true)
                .code_copy_action_prefix(COPY_CODE)
                .code_font_size(SMALL)
                .inline_code_size(SMALL)
                .code_syntax_colors(limn::CodeSyntaxColors {
                    text: look::palette().text,
                    string: look::added(),
                    keyword: look::keyword(),
                    function: Some(look::cursor()),
                    interpolation: Some(look::removed()),
                    ..limn::CodeSyntaxColors::for_palette(code_palette)
                })
                .streaming(*streaming)
                // New words come in out of a fade.
                .streaming_fade(true)
                .render()
                .element
        }
        Item::Tool { .. } => Element::spacer().size(None, Some(0.0)),
        Item::Permission {
            id,
            input,
            interactive: true,
            decided: None,
            ..
        } if input["questions"].is_array() => question_card(id, input, ui, column, palette),
        Item::Permission {
            id,
            tool,
            decided: None,
            ..
        } if tool == "ExitPlanMode" => plan_card(id, Some(session), ui, column, palette),
        Item::Permission { .. } => permission(item, ui, column, palette),
        Item::TurnEnd {
            ok: false,
            interrupted: true,
            ..
        } => muted("Interrupted"),
        Item::TurnEnd {
            ok: false, error, ..
        } => text(error.clone().unwrap_or_else(|| "The turn failed".into()))
            .foreground(look::removed()),
        Item::TurnEnd { .. } => Element::spacer().size(None, Some(0.0)),
        Item::Changes {
            request,
            files,
            rewound,
        } => changes_card(request, files, *rewound, column, palette),
    }
}

/// One tool call of a turn, as its group lists it.
pub struct Step<'a> {
    pub call: &'a str,
    pub name: &'a str,
    pub input: &'a Value,
    pub result: Option<&'a ToolResult>,
    /// It has not answered yet, or the work it started still runs (a helper agent).
    pub running: bool,
}

/// The task tools keep the list shown over the running line; Desktop lists no step for them
///.
pub fn is_task_tool(name: &str) -> bool {
    matches!(
        name,
        "TodoWrite" | "TaskCreate" | "TaskUpdate" | "TaskGet" | "TaskList" | "TaskStop"
    )
}

/// Steps Desktop sets on their own line, out of any group
///.
fn stands_alone(name: &str) -> bool {
    matches!(name, "AskUserQuestion" | "ExitPlanMode")
}

/// One entry of the task list.
#[derive(Clone, Debug, PartialEq)]
pub struct Todo {
    pub id: String,
    pub subject: String,
    /// "pending", "in_progress" or "completed".
    pub status: String,
}

/// The task list as the task tools left it: `TaskCreate` adds, `TaskUpdate` changes or
/// deletes by id, `TodoWrite` writes the whole list.
pub fn todos(items: &[Item]) -> Vec<Todo> {
    let mut list: Vec<Todo> = vec![];
    for item in items {
        let Item::Tool {
            name,
            input,
            result,
            ..
        } = item
        else {
            continue;
        };
        match name.as_str() {
            "TaskCreate" => {
                // The CLI numbers its tasks and says so: "Task #1 created successfully: …".
                let id = result
                    .as_ref()
                    .and_then(|r| r.output.strip_prefix("Task #"))
                    .and_then(|rest| rest.split(' ').next())
                    .map(str::to_owned)
                    .unwrap_or_else(|| (list.len() + 1).to_string());
                list.push(Todo {
                    id,
                    subject: input["subject"].as_str().unwrap_or_default().to_owned(),
                    status: "pending".into(),
                });
            }
            "TaskUpdate" => {
                let id = input["taskId"].as_str().unwrap_or_default();
                if input["status"] == "deleted" {
                    list.retain(|t| t.id != id);
                } else if let Some(todo) = list.iter_mut().find(|t| t.id == id) {
                    if let Some(status) = input["status"].as_str() {
                        todo.status = status.to_owned();
                    }
                    if let Some(subject) = input["subject"].as_str() {
                        todo.subject = subject.to_owned();
                    }
                }
            }
            "TodoWrite" => {
                list = input["todos"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(i, t)| Todo {
                        id: (i + 1).to_string(),
                        subject: t["content"].as_str().unwrap_or_default().to_owned(),
                        status: t["status"].as_str().unwrap_or("pending").to_owned(),
                    })
                    .collect();
            }
            _ => {}
        }
    }
    list
}

/// The task list: a card as wide as the column, a 28 px row per
/// task, its mark (a filled check when done, a turning arc while in progress, a dashed ring
/// while waiting) and its words, gray once done.
fn task_card(list: &[Todo], column: f32, palette: ComponentPalette) -> Element {
    let rows = list
        .iter()
        .enumerate()
        .map(|(i, todo)| {
            let done = todo.status == "completed";
            let mark = match todo.status.as_str() {
                // The icon's own white would take the tint: the check goes over a dot.
                "completed" => Element::overlay(
                    Element::spacer()
                        .background(look::muted())
                        .radius(7.0)
                        .size(Some(14.0), Some(14.0)),
                    Element::icon(icons::of(icons::CHECK))
                        .foreground(palette.surface)
                        .size(Some(10.0), Some(10.0))
                        .centered(),
                )
                .size(Some(14.0), Some(14.0)),
                "in_progress" => Spinner::new(format!("task:{i}"), look::muted())
                    .size(14.0)
                    .decorative()
                    .render(),
                _ => Element::icon(icons::of(icons::TASK_PENDING))
                    .foreground(look::muted())
                    .size(Some(14.0), Some(14.0)),
            };
            let words = text(todo.subject.clone()).font_size(BODY);
            let words = if done {
                words.foreground(look::muted())
            } else {
                words
            };
            Element::row(vec![mark.centered_y(), words.centered_y()])
                .gap(6.0)
                .size(None, Some(28.0))
        })
        .collect();
    Element::panel(Element::column(rows))
        .background(palette.surface)
        .border(look::card_border(), 1.0)
        .radius(8.0)
        .padding(12.0)
        .size(Some(column), None)
}

/// A verb phrase put in the past, as Desktop names a finished step: "Run saludo.py" becomes
/// "Ran saludo.py", "Create atajo.txt…" "Created atajo.txt…".
pub fn past_tense(phrase: &str) -> String {
    inflect(phrase, |verb| {
        const IRREGULAR: &[(&str, &str)] = &[
            ("Run", "Ran"),
            ("Make", "Made"),
            ("Write", "Wrote"),
            ("Find", "Found"),
            ("Get", "Got"),
            ("Build", "Built"),
            ("Read", "Read"),
            ("Set", "Set"),
            ("Put", "Put"),
            ("Begin", "Began"),
        ];
        if let Some((_, past)) = IRREGULAR.iter().find(|(v, _)| *v == verb) {
            return (*past).to_owned();
        }
        if verb.ends_with('e') {
            format!("{verb}d")
        } else if verb.ends_with('y') && !verb.ends_with("ay") && !verb.ends_with("ey") {
            format!("{}ied", &verb[..verb.len() - 1])
        } else {
            format!("{verb}ed")
        }
    })
}

/// The same phrase while it runs: "Creating atajo.txt…".
pub fn progressive(phrase: &str) -> String {
    inflect(phrase, |verb| match verb {
        "Run" => "Running".to_owned(),
        "Set" => "Setting".to_owned(),
        "Get" => "Getting".to_owned(),
        "Put" => "Putting".to_owned(),
        v if v.ends_with("ie") => format!("{}ying", &v[..v.len() - 2]),
        v if v.ends_with('e') && !v.ends_with("ee") => format!("{}ing", &v[..v.len() - 1]),
        v => format!("{v}ing"),
    })
}

/// Change the phrase's first word when it reads as a verb (capitalized, letters only).
fn inflect(phrase: &str, verb: impl Fn(&str) -> String) -> String {
    let (first, rest) = phrase.split_once(' ').unwrap_or((phrase, ""));
    let is_verb = first.chars().next().is_some_and(char::is_uppercase)
        && first.chars().all(char::is_alphabetic);
    if !is_verb {
        return phrase.to_owned();
    }
    if rest.is_empty() {
        verb(first)
    } else {
        format!("{} {rest}", verb(first))
    }
}

fn file_of(input: &Value) -> String {
    input["file_path"]
        .as_str()
        .or(input["notebook_path"].as_str())
        .map(|p| p.rsplit('/').next().unwrap_or(p).to_owned())
        .unwrap_or_default()
}

/// A step's verb and what it acted on, done or still running: ("Edited", "saludo.py"),
/// ("Reading", "saludo.py"); a command keeps its own words.
/// The verbs are Desktop's.
fn step_label(step: &Step, done: bool) -> (String, String) {
    let input = step.input;
    let said = |key: &str| input[key].as_str().unwrap_or_default().to_owned();
    let (past, present, object) = match step.name {
        "Bash" => {
            let said = input["description"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Run {}", said("command")));
            // Running it reads "Creating a.txt…"; stopped or failed, in its own words
            //.
            let phrase = match step.result {
                None => progressive(&said),
                Some(r) if r.stopped || r.is_error => said,
                Some(_) if done => past_tense(&said),
                Some(_) => said,
            };
            return (phrase, String::new());
        }
        "Read" => ("Read", "Reading", file_of(input)),
        "Edit" | "MultiEdit" | "NotebookEdit" => ("Edited", "Editing", file_of(input)),
        "Write" => {
            let created = step
                .result
                .and_then(|r| r.files.first())
                .is_none_or(|f| f.before.is_none());
            let past = if created { "Created" } else { "Updated" };
            (past, "Writing", file_of(input))
        }
        "Grep" | "Glob" => ("Searched", "Searching", said("pattern")),
        "LS" => ("Listed", "Listing", said("path")),
        "WebFetch" => ("Fetched", "Fetching", said("url")),
        "WebSearch" => ("Searched web", "Searching web", said("query")),
        "Agent" | "Task" => ("Ran agent", "Running agent", said("description")),
        "Skill" => ("Ran skill", "Running skill", said("skill")),
        "ExitPlanMode" => ("Proposed plan", "Proposing plan", String::new()),
        "AskUserQuestion" => (
            "Asked",
            "Asking",
            input["questions"][0]["header"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        ),
        // A connector's tool by its own name: "mcp__server__tool" is "tool".
        name => (
            "Used",
            "Using",
            name.rsplit("__").next().unwrap_or(name).to_owned(),
        ),
    };
    let verb = if done { past } else { present };
    (verb.to_owned(), object)
}

/// The line a group of steps folds into, as Desktop words it: one command says
/// what it did ("Ran saludo.py"); otherwise each file with what was done to it and how many
/// commands ran: "Read and edited saludo.py, ran a command". While a step runs, the group
/// reads as that step: "Listing files in folder".
pub fn group_summary(steps: &[Step]) -> String {
    if let Some(step) = steps.iter().rev().find(|s| s.running) {
        return match step.name {
            "Bash" | "Agent" | "Task" => progressive(
                step.input["description"]
                    .as_str()
                    .unwrap_or("Run a command"),
            ),
            _ => {
                let (verb, object) = step_label(step, false);
                format!("{verb} {object}").trim_end().to_owned()
            }
        };
    }
    if let [step] = steps {
        if matches!(step.name, "Bash" | "Agent" | "Task") {
            let said = step.input["description"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| "Run a command".into());
            return if step.result.is_some_and(|r| r.stopped || r.is_error) {
                said
            } else {
                past_tense(&said)
            };
        }
    }
    let mut files: Vec<(String, Vec<String>)> = vec![];
    let (mut commands, mut searches, mut agents, mut others) = (0, 0, 0, 0);
    for step in steps {
        match step.name {
            "Bash" => commands += 1,
            "Grep" | "Glob" => searches += 1,
            "Agent" | "Task" => agents += 1,
            "Read" | "Edit" | "MultiEdit" | "NotebookEdit" | "Write" => {
                let (verb, file) = step_label(step, true);
                let verb = verb.to_lowercase();
                match files.iter_mut().find(|(f, _)| *f == file) {
                    Some((_, verbs)) if !verbs.contains(&verb) => verbs.push(verb),
                    Some(_) => {}
                    None => files.push((file, vec![verb])),
                }
            }
            _ => others += 1,
        }
    }
    let mut clauses: Vec<String> = files
        .into_iter()
        .map(|(file, verbs)| format!("{} {file}", verbs.join(" and ")))
        .collect();
    if searches > 0 {
        clauses.push("searched the code".into());
    }
    // "ran a command", "ran 2 commands"; "ran an agent" and
    // "used a tool" the same way.
    let count = |n: usize, verb: &str, one: &str, thing: &str| match n {
        0 => None,
        1 => Some(format!("{verb} {one} {thing}")),
        n => Some(format!("{verb} {n} {thing}s")),
    };
    clauses.extend(count(commands, "ran", "a", "command"));
    clauses.extend(count(agents, "ran", "an", "agent"));
    clauses.extend(count(others, "used", "a", "tool"));
    let mut summary = clauses.join(", ");
    if let Some(first) = summary.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    summary
}

/// The lines of `before` and `after` as a diff: an LCS walk, enough for the files a tool
/// edits. shortcut: quadratic in lines, capped at 400 a side; a real diff when files grow.
pub fn line_diff(before: &str, after: &str) -> Vec<limn::CodeDiffLine> {
    use limn::{CodeDiffLine, CodeDiffLineKind};
    let a: Vec<&str> = before.lines().take(400).collect();
    let b: Vec<&str> = after.lines().take(400).collect();
    let mut lcs = vec![vec![0u16; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut lines) = (0, 0, vec![]);
    let line = |kind, old: Option<usize>, new: Option<usize>, text: &str| CodeDiffLine {
        kind,
        old_line: old.map(|n| n + 1),
        new_line: new.map(|n| n + 1),
        text: text.to_owned(),
    };
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            lines.push(line(CodeDiffLineKind::Context, Some(i), Some(j), a[i]));
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            lines.push(line(CodeDiffLineKind::Added, None, Some(j), b[j]));
            j += 1;
        } else {
            lines.push(line(CodeDiffLineKind::Removed, Some(i), None, a[i]));
            i += 1;
        }
    }
    lines
}

/// A run of steps as Desktop shows it: one muted line with the files' added and
/// removed lines, which unfolds into a card with a row per step; a row unfolds what it did.
fn step_group(steps: &[Step], ui: &Ui, column: f32, palette: ComponentPalette) -> Element {
    let key = format!("steps:{}", steps[0].call);
    let open = ui.expanded.contains(&key);
    let running = steps.iter().any(|s| s.running);
    let stopped = !running
        && steps
            .last()
            .is_some_and(|s| s.result.is_some_and(|r| r.stopped));
    let (added, removed) = steps
        .iter()
        .filter_map(|s| s.result)
        .fold((0, 0), |(a, r), res| (a + res.added, r + res.removed));
    let summary = group_summary(steps);
    let mut head = vec![if running {
        TextShimmer::new(format!("{key}:shimmer"), summary, palette)
            .render()
            .font_size(CHAT)
            .centered_y()
    } else {
        muted(summary).font_size(CHAT).centered_y()
    }];
    // A command left running in the background offers its output.
    if steps
        .iter()
        .any(|s| s.running && s.name == "Bash" && s.result.is_some())
    {
        head.push(text("View live output").font_size(CHAT).centered_y());
    }
    if stopped {
        head.push(muted("Stopped").font_size(CHAT).centered_y());
    } else if !running
        && steps
            .last()
            .is_some_and(|s| s.result.is_some_and(|r| r.is_error))
    {
        // Desktop marks a run whose last step failed.
        head.push(
            text("Failed")
                .font_size(CHAT)
                .foreground(look::failed())
                .centered_y(),
        );
    }
    if added + removed > 0 {
        head.push(
            text(format!("+{added}"))
                .font_size(CHAT)
                .foreground(look::added())
                .centered_y(),
        );
        head.push(
            text(format!("−{removed}"))
                .font_size(CHAT)
                .foreground(look::removed())
                .centered_y(),
        );
    }
    head.push(caret(open));
    let head = Element::clickable(key, Element::row(head).gap(8.0).size(None, Some(24.0)));
    if !open {
        return head;
    }
    let mut rows = vec![];
    for (i, step) in steps.iter().enumerate() {
        if i > 0 {
            rows.push(hline(column, look::card_border()));
        }
        rows.push(step_row(step, ui, column, palette));
    }
    let card = Element::panel(Element::column(rows))
        .background(palette.surface)
        .border(look::card_border(), 1.0)
        .radius(8.0)
        .size(Some(column), None);
    Element::column(vec![head, card]).gap(8.0)
}

/// The small triangle after a group or a step: right while folded, down while open.
fn caret(open: bool) -> Element {
    text(if open { "▾" } else { "▸" })
        .font_size(10.0)
        .foreground(look::muted())
        .centered_y()
}

/// One step in its group's card: "Read saludo.py", "Edited saludo.py +1 −1",
/// "Ran saludo.py to verify output", each opening what it did. A helper agent's row has no
/// caret: it opens the agent's own pane.
fn step_row(step: &Step, ui: &Ui, column: f32, palette: ComponentPalette) -> Element {
    let agent = matches!(step.name, "Agent" | "Task");
    let open = !agent && ui.expanded.iter().any(|c| c == step.call);
    let done = !step.running && step.result.is_some_and(|r| !r.stopped);
    let (verb, object) = step_label(step, done);
    let mut line = vec![hgap(10.0), muted(verb).centered_y()];
    if !object.is_empty() {
        line.push(text(object).centered_y());
    }
    if let Some(r) = step.result {
        if r.added + r.removed > 0 {
            line.push(
                text(format!("+{}", r.added))
                    .foreground(look::added())
                    .centered_y(),
            );
            line.push(
                text(format!("−{}", r.removed))
                    .foreground(look::removed())
                    .centered_y(),
            );
        }
        if r.stopped {
            line.push(muted("Stopped").centered_y());
        }
    }
    if !agent {
        line.push(caret(open));
    }
    let row_palette = ComponentPalette {
        hover: look::row_hover(),
        ..palette
    };
    let id = if agent {
        format!("agent:{}", step.call)
    } else {
        format!("tool:{}", step.call)
    };
    let row = row_palette.interactive_surface(Element::clickable(
        id,
        Element::row(line).gap(6.0).size(Some(column), Some(31.0)),
    ));
    if !open {
        return row;
    }
    Element::column(vec![
        row,
        pad(step_body(step, column - 24.0, palette), 12.0, 4.0),
    ])
}

/// What a step did, as its row unfolds it: a command and its output, a file's diff, or the
/// tool's output.
fn step_body(step: &Step, inner: f32, palette: ComponentPalette) -> Element {
    let mut body = vec![];
    let result = step.result;
    match step.name {
        "Bash" => {
            body.push(muted("Bash").font_size(SMALL));
            let command = step.input["command"].as_str().unwrap_or_default();
            body.push(
                Element::panel(
                    Element::row(vec![
                        mono("$").foreground(look::muted()),
                        mono(command).size(Some(inner - 40.0), None),
                    ])
                    .gap(8.0),
                )
                .background(palette.surface)
                .border(look::card_border(), 1.0)
                .radius(6.0)
                .padding(10.0)
                .size(Some(inner), None),
            );
            if let Some(r) = result.filter(|r| !r.output.trim().is_empty()) {
                body.push(
                    mono(r.output.trim_end())
                        .foreground(look::muted())
                        .size(Some(inner), None),
                );
            }
        }
        _ => {
            let files = result.map(|r| r.files.as_slice()).unwrap_or_default();
            for file in files {
                body.push(muted(file.path.display().to_string()).font_size(SMALL));
                body.push(diff_box(
                    &format!("diff:{}:{}", step.call, file.path.display()),
                    file.before.as_deref().unwrap_or_default(),
                    file.after.as_deref().unwrap_or_default(),
                    inner,
                    palette,
                ));
            }
            if files.is_empty() {
                if let Some(r) = result.filter(|r| !r.output.trim().is_empty()) {
                    body.push(
                        OutputBlock::new(format!("out:{}", step.call), r.output.clone(), palette)
                            .render()
                            .size(Some(inner), None),
                    );
                }
            }
        }
    }
    Element::column(body).gap(8.0)
}

/// Two versions of a file as a bordered diff, up to 480 px high.
fn diff_box(id: &str, before: &str, after: &str, width: f32, palette: ComponentPalette) -> Element {
    let lines = line_diff(before, after);
    let rows = lines.len().max(1) as f32;
    let words = word_ranges(&lines);
    Element::panel(
        limn::CodeDiff::new(id, lines)
            .word_ranges(words)
            .palette(palette)
            .font_size(12.0)
            .height((rows * 19.0 + 16.0).min(480.0))
            .render(),
    )
    .border(look::card_border(), 1.0)
    .radius(6.0)
    .size(Some(width), None)
}

/// A step Desktop sets on its own line: a proposed plan as "Proposed plan ▸",
/// which opens the plan, and answered questions as a card of
/// each question and its answer. Until they answer, their cards
/// are the permission's.
fn lone_step(step: &Step, column: f32, palette: ComponentPalette) -> Option<Element> {
    let result = step.result.filter(|r| !r.stopped && !r.is_error)?;
    if step.name == "ExitPlanMode" {
        let line = Element::row(vec![
            muted("Proposed plan").font_size(CHAT).centered_y(),
            caret(false),
        ])
        .gap(8.0)
        .size(None, Some(24.0));
        return Some(Element::clickable(format!("plan:{}", step.call), line));
    }
    let questions = step.input["questions"].as_array()?;
    let rows: Vec<Element> = questions
        .iter()
        .enumerate()
        .flat_map(|(i, q)| {
            let question = q["question"].as_str().unwrap_or_default();
            let answer = answered(&result.output, question).unwrap_or_else(|| "Skipped".into());
            let row = Element::column(vec![
                text(question.to_owned()).font_size(BODY),
                muted(answer).font_size(BODY),
            ])
            .gap(4.0)
            .padding(16.0);
            let line = (i > 0).then(|| hline(column.min(480.0), look::card_border()));
            line.into_iter().chain([row])
        })
        .collect();
    Some(
        Element::panel(Element::column(rows))
            .background(palette.surface)
            .border(look::card_border(), 1.0)
            .radius(8.0)
            .size(Some(column.min(480.0)), None),
    )
}

/// The answer the agent was given to `question`, as the CLI words it:
/// `The user answered: "Pick red or blue?"="blue". …` ([F:ask]).
/// shortcut: an answer holding a quote is cut there; parse the tool's structured result when
/// one reaches the UI.
pub fn answered(output: &str, question: &str) -> Option<String> {
    let start = output.find(&format!("\"{question}\"=\""))? + question.len() + 4;
    let rest = &output[start..];
    Some(rest[..rest.find('"')?].to_owned())
}

fn mono(s: impl Into<String>) -> Element {
    Element::text(s.into())
        .font_family(FontFamily::Monospace)
        .font_size(SMALL)
        .foreground(look::palette().text)
}

/// The inline approval card, as Desktop 3.6556.2 draws it: a 480 px card with what
/// Claude wants to run, Deny, and the way the person allowed last as its main button with
/// the other in its menu. An edit names its file and shows its diff
///. Once answered it goes, leaving the tool's row.
fn permission(item: &Item, ui: &Ui, column: f32, palette: ComponentPalette) -> Element {
    let Item::Permission {
        id,
        tool,
        input,
        title,
        interactive,
        decided,
        suggestions,
        ..
    } = item
    else {
        return Element::spacer();
    };
    if decided.is_some() {
        return Element::spacer().size(None, Some(0.0));
    }
    let width = column.min(480.0);
    let inner = width - 32.0;
    let wrapped = |e: Element| e.size(Some(inner), None);
    let command = input["command"].as_str();
    // Bash says what it runs in its own words: "Run saludo.py".
    let what = input["description"].as_str();
    let path = input["file_path"]
        .as_str()
        .or(input["notebook_path"].as_str());
    let file = file_of(input);
    let heading = match (tool.as_str(), what) {
        ("Bash", Some(what)) => format!("Allow Claude to run {what}?"),
        ("Edit" | "MultiEdit" | "NotebookEdit" | "Write", _) if path.is_some() => {
            format!("Allow Claude to edit {file}?")
        }
        _ => format!("Allow Claude to use {tool}?"),
    };
    let mut rows = vec![wrapped(text(heading).font_family(FontFamily::SansBold))];
    if let Some(what) = what {
        rows.push(wrapped(muted(what.to_owned())));
    } else if path.is_some() {
        rows.push(wrapped(muted(file)));
    }
    if let Some(title) = title {
        rows.push(wrapped(muted(title.clone())));
    }
    let shown = command.or(path).map(str::to_owned).unwrap_or_else(|| {
        serde_json::to_string(input)
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect()
    });
    rows.push(
        Element::panel(
            text(shown)
                .font_family(FontFamily::Monospace)
                .font_size(SMALL)
                .foreground(look::command_ink())
                .size(Some(inner - 32.0), None),
        )
        .background(look::command_box())
        .radius(6.0)
        .padding(14.0),
    );
    // What the edit changes, before the person allows it.
    let change = match tool.as_str() {
        "Edit" => Some((
            input["old_string"].as_str().unwrap_or_default(),
            input["new_string"].as_str().unwrap_or_default(),
        )),
        "Write" => Some(("", input["content"].as_str().unwrap_or_default())),
        _ => None,
    };
    if let Some((before, after)) = change {
        rows.push(diff_box(
            &format!("perm-diff:{id}"),
            before,
            after,
            inner,
            palette,
        ));
    }
    if *interactive {
        rows.push(Button::new(format!("perm:{id}:once"), "Continue", palette).render());
    } else {
        rows.push(permission_actions(id, !suggestions.is_empty(), ui, palette));
    }
    Element::panel(Element::column(rows).gap(10.0))
        .background(palette.surface)
        .border(look::card_border(), 1.0)
        .radius(12.0)
        .padding(16.0)
        .size(Some(width), None)
}

/// A button with its keys in caps after the label, Desktop's "Revise… Esc" and
/// "Next Ctrl ⏎": the light fill, or the near-black one when
/// `primary`.
fn keyed_button(
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

/// The plan Claude proposed, before the person decides: "Claude
/// proposed a plan" and "Open plan"; Reject, "Revise…", "Accept", and the main button
/// accepting into the most trusting mode offered, the other modes in its menu
///.
fn plan_card(
    id: &str,
    session: Option<&Session>,
    ui: &Ui,
    column: f32,
    palette: ComponentPalette,
) -> Element {
    let inner = column - 32.0;
    let head = Element::row(vec![
        text("Claude proposed a plan")
            .font_family(FontFamily::SansBold)
            .centered_y(),
        Element::spacer().flex(1.0),
        limn::Link::new(format!("plan:{id}"), "Open plan", palette)
            .ink(look::handle())
            .font_size(BODY)
            .render()
            .centered_y(),
    ])
    .size(Some(inner), None);
    let modes = offered_modes(session);
    let trusting = if modes.iter().any(|m| m == "auto") {
        ("auto", "Accept and auto mode")
    } else {
        ("acceptEdits", "Accept and allow edits")
    };
    let menu: Vec<limn::DropdownMenuEntry> = [
        ("acceptEdits", "Accept and allow edits"),
        ("auto", "Accept and auto mode"),
    ]
    .into_iter()
    .filter(|(mode, _)| modes.iter().any(|m| m == mode))
    .map(|(mode, label)| limn::DropdownMenuItem::new(format!("planok:{id}:{mode}"), label).into())
    .collect();
    let accept = limn::SplitButton::new(
        format!("planok:{id}:{}", trusting.0),
        trusting.1,
        format!("perm:{id}:menu"),
        palette,
    )
    .keys(&["Ctrl", "⏎"])
    .menu(
        menu,
        ui.perm_menu.as_deref() == Some(id),
        format!("perm:{id}:menu-dismiss"),
    )
    .render();
    let mut rows = vec![
        head,
        Element::row(vec![
            keyed_button(
                &format!("perm:{id}:deny"),
                "Reject",
                "",
                false,
                true,
                palette,
            ),
            keyed_button(
                &format!("revise:{id}"),
                "Revise…",
                "Esc",
                false,
                true,
                palette,
            ),
            hgap(12.0),
            keyed_button(
                &format!("planok:{id}:default"),
                "Accept",
                "Ctrl+Shift+⏎",
                false,
                true,
                palette,
            ),
            accept.centered_y(),
        ])
        .gap(8.0)
        // In a narrow column the buttons go on a second line.
        .wrap()
        .size(Some(inner), None),
    ];
    if ui.revising.as_deref() == Some(id) {
        rows.push(
            limn::TextField::new(
                format!("revise-text:{id}"),
                ui.revision.as_str(),
                "What should change? (optional)",
                palette,
            )
            .width(inner)
            .render(),
        );
    }
    Element::panel(Element::column(rows).gap(14.0))
        .background(palette.surface)
        .border(look::card_border(), 1.0)
        .radius(12.0)
        .padding(16.0)
        .size(Some(column), None)
}

/// Deny, then the last way of allowing as the main button with its keys, and the other way
/// in its menu.
fn permission_actions(id: &str, can_always: bool, ui: &Ui, palette: ComponentPalette) -> Element {
    let menu_open = ui.perm_menu.as_deref() == Some(id);
    let always_first = can_always && ui.last_allow == crate::Allow::Always;
    let once =
        limn::DropdownMenuItem::new(format!("perm:{id}:once"), "Allow once").shortcut("Ctrl+Enter");
    let always = limn::DropdownMenuItem::new(format!("perm:{id}:always"), "Always allow")
        .description("Saves to local project settings")
        .shortcut("Ctrl+Shift+Enter");
    let (main, label, keys, others): (_, _, &[&str], Vec<_>) = if always_first {
        (
            "always",
            "Always allow",
            &["Ctrl", "Shift", "⏎"],
            vec![once.into()],
        )
    } else if can_always {
        ("once", "Allow once", &["Ctrl", "⏎"], vec![always.into()])
    } else {
        ("once", "Allow once", &["Ctrl", "⏎"], vec![])
    };
    let mut split = limn::SplitButton::new(
        format!("perm:{id}:{main}"),
        label,
        format!("perm:{id}:menu"),
        palette,
    )
    .keys(keys);
    if !others.is_empty() {
        split = split.menu(others, menu_open, format!("perm:{id}:menu-dismiss"));
    }
    let deny = Button::new(
        format!("perm:{id}:deny"),
        "Deny",
        ComponentPalette {
            raised: look::cancel(),
            ..palette
        },
    )
    .variant(ButtonVariant::Secondary)
    .button_size(ButtonSize::Small)
    .render();
    ButtonGroup::new(vec![deny, split.render()])
        .align_end()
        .render()
}

/// Desktop's composer, as the person's Desktop 2.9939.4 lays it out: a new
/// session's place and folder as filled chips over a 44 px box that holds only the message
/// and its return arrow; under the box "+" and the mode on the left, the model, its effort
/// and the plan's usage on the right.
fn composer(
    store: &Store,
    session: Option<&Session>,
    ui: &Ui,
    column: f32,
    palette: ComponentPalette,
) -> Element {
    let running = session.is_some_and(|s| {
        matches!(
            s.status,
            SessionStatus::Running | SessionStatus::Starting | SessionStatus::Awaiting
        )
    });
    let trailing = if running && ui.prompt.trim().is_empty() {
        composer_button(STOP, icons::of(icons::STOP), "Stop", false, 28.0)
    } else {
        send_button(!ui.prompt.trim().is_empty())
    };
    // While a plan waits, its card is the way to answer: Desktop hides the box and keeps
    // the footer under it.
    let plan_waiting = session.is_some_and(|s| {
        s.items.iter().any(
            |i| matches!(i, Item::Permission { tool, decided: None, .. } if tool == "ExitPlanMode"),
        )
    });
    if plan_waiting {
        return composer_footer(store, session, ui, palette).size(Some(column), None);
    }
    let placeholder = if session.is_some() {
        "Type / for commands"
    } else {
        "Describe a task or ask a question"
    };
    let box_ = Composer::new(
        COMPOSER,
        ui.prompt.clone(),
        placeholder,
        column,
        look::composer_palette(),
    )
    .enter_submits(true)
    .auto_grow(28.0, 184.0)
    .editor_font_size(15.0)
    .pill_controls(Element::spacer().size(Some(0.0), None), trailing)
    .inline_controls(true)
    .border_color(look::composer_border())
    .shadow_color(look::composer_shadow())
    .card_radius(14.0)
    .context(composer_footer(store, session, ui, palette))
    .render();
    if let Some(session) = session {
        return match branch_bar(session, ui, column, palette) {
            Some(bar) => Element::column(vec![bar, box_]).gap(8.0),
            None => box_,
        };
    }
    // quill runs sessions on this computer only, so "Local" is a label, not a menu.
    let local = Element::row(vec![
        hgap(8.0),
        chip_icon(icons::of(icons::LAPTOP)),
        chip_text("Local", look::palette().text),
        hgap(8.0),
    ])
    .gap(6.0)
    .background(look::cancel())
    .radius(6.0)
    .size(None, Some(24.0));
    Element::column(vec![
        Element::row(vec![local, folder_menu(store, ui)]).gap(8.0),
        box_,
    ])
    .gap(13.0)
}

/// A count with commas between its thousands, as the branch bar writes them.
pub(crate) fn thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Why the branch bar's pull request cannot be opened now, in Desktop's words.
pub fn pr_blocked(session: &Session) -> Option<&'static str> {
    let repo = session.repo.as_ref()?;
    if repo.remote.is_none() {
        Some("Add a GitHub remote to create a pull request")
    } else if repo.base.as_ref() == Some(&repo.branch) {
        Some("Already on the base branch \u{2014} check out a feature branch first")
    } else if matches!(
        session.status,
        SessionStatus::Running | SessionStatus::Starting | SessionStatus::Awaiting
    ) {
        Some("Claude is working \u{2014} wait for the turn to finish")
    } else {
        None
    }
}

/// A chip of the branch bar: 28 px, the bar's lighter fill.
fn bar_chip(content: Element) -> Element {
    Element::row(vec![content])
        .size(None, Some(28.0))
        .background(look::branch_chip())
        .border(look::branch_chip_edge(), 1.0)
        .radius(6.0)
        .clip_children()
        .centered_y()
}

/// The bar over a session's composer, as the person's Desktop keeps it
///: the folder and its branch, the lines added and
/// removed since the last commit rolling to each new count, the pull
/// request in the kind last chosen, and a close. It shows while the folder is a repository.
fn branch_bar(
    session: &Session,
    ui: &Ui,
    column: f32,
    palette: ComponentPalette,
) -> Option<Element> {
    let repo = session.repo.as_ref()?;
    if ui.bar_closed.contains(&session.id) {
        return None;
    }
    let mut row = vec![
        hgap(5.0),
        muted(folder_name(&session.cwd)).centered_y(),
        muted(repo.branch.clone())
            .font_family(FontFamily::Monospace)
            .centered_y(),
        Element::spacer().flex(1.0),
    ];
    let (added, removed) = session
        .working
        .iter()
        .flatten()
        .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
    if added + removed > 0 {
        let count = |key: &str, text: String, color: limn::Color| {
            limn::RollingNumber::new(format!("bar:{}:{key}", session.id), text, palette)
                .color(color)
                .font_size(BODY)
                .render()
                .centered_y()
        };
        row.push(bar_chip(
            Element::row(vec![
                hgap(6.0),
                count("added", format!("+{}", thousands(added)), look::added()),
                count(
                    "removed",
                    format!("\u{2212}{}", thousands(removed)),
                    look::removed(),
                ),
                hgap(6.0),
            ])
            .gap(4.0),
        ));
    }
    let label = if ui.pr_draft {
        "Create draft PR"
    } else {
        "Create PR"
    };
    let blocked = pr_blocked(session);
    // Space at the sides only: the label sits in the middle of the 28 px chip.
    let main = Element::row(vec![
        hgap(10.0),
        text(label)
            .foreground(if blocked.is_some() {
                look::muted()
            } else {
                look::palette().text
            })
            .centered_y(),
        hgap(10.0),
    ])
    .size(None, Some(28.0));
    let hover = ComponentPalette {
        hover: look::branch_chip_hover(),
        ..palette
    };
    let main = match blocked {
        Some(reason) => limn::Tooltip::new(main, "pr:why", reason, look::palette()).render(),
        None => hover
            .interactive_surface(Element::clickable(PR_CREATE, main))
            .size(None, Some(28.0)),
    };
    let open = ui.open_menu == Some(PR_MENU);
    let chevron = hover
        .interactive_surface(Element::clickable(
            PR_MENU,
            Element::icon(icons::of(icons::CARET_DOWN))
                .foreground(look::palette().text)
                .size(Some(14.0), Some(14.0))
                .centered(),
        ))
        .accessibility_label("Pull request options")
        .size(Some(28.0), Some(28.0));
    let chevron = if open {
        chevron.background(look::branch_chip_hover())
    } else {
        chevron
    };
    let kinds = vec![
        limn::DropdownMenuItem::new("pr-kind:ready", "Create PR")
            .radio(!ui.pr_draft)
            .into(),
        limn::DropdownMenuItem::new("pr-kind:draft", "Create draft PR")
            .radio(ui.pr_draft)
            .into(),
    ];
    let chevron = menu(
        chevron,
        kinds,
        open,
        PR_MENU,
        200.0,
        limn::PopoverPlacement::TopEnd,
    );
    row.push(bar_chip(Element::row(vec![
        main,
        Element::spacer()
            .background(look::branch_bar())
            .size(Some(1.0), Some(28.0)),
        chevron,
    ])));
    row.push(icon_tool(
        BAR_CLOSE,
        icons::of(icons::CLOSE),
        "Close",
        28.0,
        palette,
    ));
    Some(
        Element::row(row)
            .gap(8.0)
            .padding(8.0)
            .size(Some(column), Some(45.0))
            .background(look::branch_bar())
            .radius(12.0),
    )
}

/// A round-cornered 34 px icon button inside the composer, gray under the pointer and while
/// its menu is open.
fn composer_button(id: &str, icon: limn::Icon, label: &str, open: bool, side: f32) -> Element {
    let palette = ComponentPalette {
        hover: look::composer_hover(),
        selected: look::composer_open(),
        ..look::palette()
    };
    let glyph = Element::icon(icon)
        .foreground(look::palette().text)
        .size(Some(16.0), Some(16.0))
        .centered();
    let button = palette
        .interactive_surface(Element::clickable(id, glyph).accessibility_label(label))
        .radius(8.0)
        .size(Some(side), Some(side));
    let button = if open {
        button.background(look::composer_open())
    } else {
        button
    };
    limn::Tooltip::new(button, id, label, look::palette()).render()
}

/// Send, drawn as Desktop's return arrow; limn has no such icon, so it is the character.
fn send_button(ready: bool) -> Element {
    let palette = ComponentPalette {
        hover: look::composer_hover(),
        selected: look::composer_open(),
        ..look::palette()
    };
    // Muted while there is nothing to send, as Desktop's.
    let ink = if ready {
        look::palette().text
    } else {
        look::muted()
    };
    let glyph = Element::icon(icons::of(icons::RETURN))
        .foreground(ink)
        .size(Some(22.0), Some(22.0))
        .centered();
    let button = palette
        .interactive_surface(Element::clickable(SEND, glyph))
        .radius(8.0)
        // 28 px: limn centers it in the first line's 28 px, so it sits mid-box.
        .size(Some(28.0), Some(28.0));
    limn::Tooltip::new(button, SEND, "Send", look::palette()).render()
}

/// "+": what can go into the message, as far as quill can: files and folders
/// as @-mentions the agent reads, and the slash that opens its commands.
fn plus_menu(ui: &Ui) -> Element {
    let open = ui.open_menu == Some(PLUS_MENU);
    let anchor = composer_button(PLUS_MENU, icons::of(icons::ADD), "Add", open, 28.0);
    let entries = vec![
        limn::DropdownMenuItem::new(ATTACH_FILES, "Add files or photos")
            .icon(icons::of(icons::PAPERCLIP))
            .shortcut("Ctrl+U")
            .into(),
        limn::DropdownMenuItem::new(ATTACH_FOLDER, "Add folder")
            .icon(icons::of(icons::FOLDER))
            .into(),
        limn::DropdownMenuItem::new(SLASH, "Slash commands")
            .icon(icons::of(icons::SLASH))
            .into(),
    ];
    menu(
        anchor,
        entries,
        open,
        PLUS_MENU,
        320.0,
        limn::PopoverPlacement::TopStart,
    )
}

/// A menu in Desktop's style: white, rounded, above the composer.
fn menu(
    anchor: Element,
    entries: Vec<limn::DropdownMenuEntry>,
    open: bool,
    id: &str,
    width: f32,
    placement: limn::PopoverPlacement,
) -> Element {
    // Desktop's rows are 28 px with 14 px labels and 13 px captions.
    let sizing = limn::SelectorSizing {
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
    };
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

/// A footer chip: an icon and words, gray under the pointer and darker while its menu is
/// open, 24 px tall.
fn chip(
    id: &str,
    content: Vec<Element>,
    open: bool,
    tooltip: &str,
    shortcut: Option<&str>,
) -> Element {
    let palette = ComponentPalette {
        hover: look::chip_hover(),
        selected: look::chip_open(),
        ..look::palette()
    };
    let mut items = vec![hgap(6.0)];
    items.extend(content);
    items.push(hgap(6.0));
    let row = Element::row(items).gap(6.0).size(None, Some(24.0));
    let chip = palette
        .interactive_surface(Element::clickable(id, row))
        .radius(6.0);
    // Above the composer the chips are filled at rest.
    let chip = if open {
        chip.background(look::chip_open())
    } else if id == FOLDER_MENU {
        chip.background(look::cancel())
    } else {
        chip
    };
    let tip = limn::Tooltip::new(chip, id, tooltip, look::palette());
    match shortcut {
        Some(keys) => tip.shortcut(keys),
        None => tip,
    }
    .render()
}

fn chip_text(s: impl Into<String>, ink: limn::Color) -> Element {
    text(s).font_size(SMALL).foreground(ink).centered_y()
}

fn chip_icon(icon: limn::Icon) -> Element {
    Element::icon(icon)
        .foreground(look::palette().text)
        .size(Some(14.0), Some(14.0))
        .centered_y()
}

/// The row under the composer: "+" and the mode on the left; the model, its
/// effort and the plan's usage ring on the right.
fn composer_footer(
    store: &Store,
    session: Option<&Session>,
    ui: &Ui,
    palette: ComponentPalette,
) -> Element {
    let row = Element::row(vec![
        plus_menu(ui),
        mode_menu(session, ui),
        Element::spacer().flex(1.0),
        model_menu(store, session, ui, palette),
        effort_menu(store, session, ui),
        usage_menu(store, session, ui),
    ])
    .gap(2.0)
    .size(None, Some(28.0));
    // 20 px under the box, as Desktop's, so the row's tooltips clear the message.
    Element::column(vec![Element::spacer().size(None, Some(6.0)), row])
}

/// When a window starts again, as Desktop says it: "Resets in 4 hr 53 min"
/// within a day, else the local day and hour, "Resets Sun 3:00 AM". `offset` is the local
/// time's distance from UTC in seconds.
pub fn resets_in(resets_at: u64, now: u64, offset: i64) -> String {
    let left = resets_at.saturating_sub(now);
    if left < 86_400 {
        let (hours, minutes) = (left / 3_600, left % 3_600 / 60);
        return if hours == 0 {
            format!("Resets in {minutes} min")
        } else {
            format!("Resets in {hours} hr {minutes} min")
        };
    }
    let local = resets_at as i64 + offset;
    // 1 January 1970 was a Thursday.
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    let day = DAYS[local.div_euclid(86_400).rem_euclid(7) as usize];
    let seconds = local.rem_euclid(86_400);
    let (hour, minute) = (seconds / 3_600, seconds % 3_600 / 60);
    let (twelve, half) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("Resets {day} {twelve}:{minute:02} {half}")
}

/// A 4 px bar, gray, filled blue to `share`.
fn usage_bar(share: f32, width: f32) -> Element {
    let fill = (width * share.clamp(0.0, 1.0)).max(if share > 0.0 { 4.0 } else { 0.0 });
    Element::row(vec![Element::spacer()
        .background(look::handle())
        .radius(2.0)
        .size(Some(fill), Some(4.0))])
    .background(look::active())
    .radius(2.0)
    .size(Some(width), Some(4.0))
}

/// One labelled row of the usage popover, over its bar.
fn usage_row(label: &str, detail: String, share: f32, width: f32, muted_label: bool) -> Element {
    let label = if muted_label {
        muted(label.to_owned())
    } else {
        text(label.to_owned())
    };
    Element::column(vec![
        Element::row(vec![
            label.font_size(SMALL),
            Element::spacer().flex(1.0),
            muted(detail).font_size(SMALL),
        ])
        .size(Some(width), Some(20.0)),
        usage_bar(share, width),
    ])
    .gap(8.0)
}

/// The usage ring and its popover: the context window, then the plan's session
/// and weekly limits with when they reset, from what the agent reports.
fn usage_menu(store: &Store, session: Option<&Session>, ui: &Ui) -> Element {
    let open = ui.open_menu == Some(USAGE_MENU);
    let limits = store.limits.unwrap_or_default();
    let spent = limits.session.map_or(0.0, |w| w.utilization);
    // A 14 px ring, blue over gray, as Desktop's.
    let ring_palette = ComponentPalette {
        text: look::handle(),
        // Seen on the hover's #E5E5E5 too; Desktop darkens it there.
        selected: look::ring_track(),
        ..look::palette()
    };
    let ring = limn::Progress::new(spent, ring_palette)
        .ring(14.0)
        .render()
        .centered();
    let palette = ComponentPalette {
        hover: look::tool_hover(),
        ..look::palette()
    };
    let anchor = palette
        .interactive_surface(Element::clickable(USAGE_MENU, ring))
        .radius(6.0)
        .size(Some(28.0), Some(28.0));
    let anchor = if open {
        anchor.background(look::tool_hover())
    } else {
        anchor
    };
    let width = 328.0;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (tokens, share) = session.map_or((0, 0.0), |s| {
        let share = if s.context_window > 0 {
            s.context_tokens as f32 / s.context_window as f32
        } else {
            0.0
        };
        (s.context_tokens, share)
    });
    let tokens = if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    };
    let plan = session
        .and_then(|s| s.capabilities.as_ref())
        .or_else(|| {
            store
                .sessions
                .iter()
                .rev()
                .find_map(|s| s.capabilities.as_ref())
        })
        .and_then(|c| c.plan.clone())
        // The agent says "Claude Max"; Desktop's popover says "Max".
        .map(|p| effort_label(p.trim_start_matches("Claude ").trim()));
    let mut rows = vec![
        usage_row("Context window", tokens.clone(), share, width, true),
        hline(width, look::divider()),
        Element::row(vec![
            muted(match &plan {
                Some(plan) => format!("Plan usage limits · {plan}"),
                None => "Plan usage limits".into(),
            })
            .font_size(SMALL),
            Element::spacer().flex(1.0),
            Element::clickable(USAGE_ALL, text("See all →").font_size(SMALL)),
        ])
        .size(Some(width), Some(20.0)),
    ];
    for (label, window) in [
        ("Session limit", limits.session),
        ("Weekly · all models", limits.weekly),
    ] {
        if let Some(w) = window {
            rows.push(usage_row(
                label,
                format!(
                    "{}  {}%",
                    resets_in(w.resets_at, now, ui.utc_offset),
                    (w.utilization * 100.0).round() as u32
                ),
                w.utilization,
                width,
                false,
            ));
        }
    }
    let card = Element::column(rows).gap(14.0).padding(8.0);
    // Its hint sums it up, as Desktop's: "Context 30.0k", the week's share and reset.
    let mut tip = limn::Tooltip::new(
        anchor,
        USAGE_MENU,
        format!("Context {tokens}"),
        look::palette(),
    )
    // At the window's right edge it opens to the left, to stay whole.
    .placement(limn::TooltipPlacement::Left);
    if let Some(w) = limits.weekly {
        tip = tip.description(format!(
            "Weekly · all models: {}% · {}",
            (w.utilization * 100.0).round() as u32,
            resets_in(w.resets_at, now, ui.utc_offset)
        ));
    }
    let anchor = tip.render();
    menu(
        anchor,
        vec![limn::DropdownMenuEntry::Custom(Box::new(card))],
        open,
        USAGE_MENU,
        width + 32.0,
        limn::PopoverPlacement::TopEnd,
    )
}

/// The folder a new session works in: the recent ones and "Open folder…".
fn folder_menu(store: &Store, ui: &Ui) -> Element {
    let open = ui.open_menu == Some(FOLDER_MENU);
    let anchor = chip(
        FOLDER_MENU,
        vec![
            chip_icon(icons::of(icons::FOLDER)),
            chip_text(folder_name(&ui.cwd), look::palette().text),
        ],
        open,
        "Folder",
        None,
    );
    let mut recent: Vec<&std::path::PathBuf> = vec![&ui.cwd];
    for session in store.sessions.iter().rev() {
        if !recent.contains(&&session.cwd) {
            recent.push(&session.cwd);
        }
    }
    let mut entries = vec![limn::DropdownMenuEntry::Heading("Recent".into())];
    entries.extend(recent.into_iter().take(6).map(|folder| {
        limn::DropdownMenuItem::new(format!("folder:{}", folder.display()), folder_name(folder))
            .selected(*folder == ui.cwd)
            .into()
    }));
    entries.push(limn::DropdownMenuEntry::Separator);
    entries.push(limn::DropdownMenuItem::new(PICK_FOLDER, "Open folder…").into());
    menu(
        anchor,
        entries,
        open,
        FOLDER_MENU,
        220.0,
        limn::PopoverPlacement::TopStart,
    )
}

/// The modes the menu lists, in its order: those the agent offers that quill can name.
fn offered_modes(session: Option<&Session>) -> Vec<String> {
    session
        .and_then(|s| s.capabilities.as_ref())
        .map(|c| c.modes.clone())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| vec!["default".into(), "acceptEdits".into(), "plan".into()])
        .into_iter()
        .filter(|m| mode_entry(m).is_some())
        .collect()
}

/// The models the menu lists: the session's agent's, or for a new session the last
/// session's.
fn offered_models<'a>(
    store: &'a Store,
    session: Option<&'a Session>,
) -> &'a [quill_model::ModelInfo] {
    session
        .and_then(|s| s.capabilities.as_ref())
        .or_else(|| {
            store
                .sessions
                .iter()
                .rev()
                .find_map(|s| s.capabilities.as_ref())
        })
        .map(|c| c.models.as_slice())
        .unwrap_or_default()
}

/// The models Desktop lists by number, the newest of each family, and the rest it keeps
/// under "More models". The agent lists each family newest first.
fn split_models(
    models: &[quill_model::ModelInfo],
) -> (Vec<&quill_model::ModelInfo>, Vec<&quill_model::ModelInfo>) {
    let mut families: Vec<&str> = vec![];
    let (mut latest, mut more) = (vec![], vec![]);
    for model in models {
        let family = model.name.split_whitespace().next().unwrap_or_default();
        if family == "Default" || families.contains(&family) {
            more.push(model);
        } else {
            families.push(family);
            latest.push(model);
        }
    }
    (latest, more)
}

/// The row a typed digit picks in the open mode or model menu, as Desktop's 1–4 do, or a
/// typed letter in a session's menu, as its P, R and D.
pub fn menu_digit(store: &Store, ui: &Ui, typed: &str) -> Option<String> {
    let menu_session = ui
        .row_menu
        .or(ui.selected.filter(|_| ui.open_menu == Some(TITLE_MENU)));
    if let Some(session) = menu_session {
        let action = match typed {
            "p" | "P" if ui.row_menu.is_some() => "pin",
            "u" | "U" if ui.row_menu.is_some() => "unread",
            "r" | "R" => "rename",
            "f" | "F" => "fork",
            "a" | "A" => "archive",
            "d" | "D" => "delete",
            _ => return None,
        };
        return Some(format!("{action}:{session}"));
    }
    let n: usize = typed.parse().ok().filter(|n| (1..=9).contains(n))?;
    let session = ui.selected.and_then(|id| store.session(id));
    match ui.open_menu? {
        MODE_MENU => offered_modes(session)
            .get(n - 1)
            .map(|m| format!("mode:{m}")),
        MODEL_MENU => split_models(offered_models(store, session))
            .0
            .get(n - 1)
            .map(|m| format!("model:{}", m.id)),
        _ => None,
    }
}

/// What each mode does, in Desktop's words.
fn mode_entry(mode: &str) -> Option<(&'static str, &'static str)> {
    Some(match mode {
        "auto" => ("Auto", "Claude handles permission decisions"),
        "default" => ("Manual", "Always ask before making changes"),
        "acceptEdits" => ("Accept edits", "Automatically accept all file edits"),
        "plan" => ("Plan", "Create a plan before making changes"),
        _ => return None,
    })
}

/// The permission mode: Manual reads amber, as Desktop's chip does.
fn mode_menu(session: Option<&Session>, ui: &Ui) -> Element {
    let open = ui.open_menu == Some(MODE_MENU);
    let current = session.map_or(ui.mode.as_str(), |s| s.mode.as_str());
    let label = mode_entry(current).map_or("Manual", |(label, _)| label);
    let ink = if current == "default" {
        look::manual()
    } else {
        look::palette().text
    };
    // Desktop's tooltip says what the mode does.
    let about = mode_entry(current).map_or("", |(_, about)| about);
    let anchor = chip(
        MODE_MENU,
        vec![chip_text(label, ink)],
        open,
        about,
        Some("Ctrl+Shift+M"),
    );
    let mut entries = vec![limn::DropdownMenuEntry::Heading("Mode".into())];
    for (i, mode) in offered_modes(session).iter().enumerate() {
        let (label, about) = mode_entry(mode).unwrap_or(("", ""));
        let n = i + 1;
        entries.push(
            limn::DropdownMenuItem::new(format!("mode:{mode}"), label)
                .description(about)
                .shortcut(n.to_string())
                .selected(mode == current)
                .into(),
        );
    }
    menu(
        anchor,
        entries,
        open,
        MODE_MENU,
        256.0,
        limn::PopoverPlacement::TopStart,
    )
}

/// Desktop's names for effort levels.
fn effort_label(effort: &str) -> String {
    match effort {
        "xhigh" => "Extra".into(),
        "medium" => "Medium".into(),
        other => {
            let mut s = other.to_owned();
            if let Some(first) = s.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            s
        }
    }
}

/// The model and its effort: the models numbered, the current one checked, then
/// Effort with the levels that model accepts.
fn model_menu(
    store: &Store,
    session: Option<&Session>,
    ui: &Ui,
    _palette: ComponentPalette,
) -> Element {
    let open = ui.open_menu == Some(MODEL_MENU);
    let models = offered_models(store, session);
    let current = models.iter().find(|m| {
        ui.model
            .as_deref()
            .is_some_and(|id| m.id == id || m.name.to_lowercase().contains(id))
    });
    let name = current.map_or_else(
        || ui.model.clone().unwrap_or_else(|| "Default".into()),
        |m| m.name.clone(),
    );
    let content = vec![chip_text(name, look::palette().text)];
    let anchor = chip(MODEL_MENU, content, open, "Model", Some("Ctrl+Shift+I"));
    // The person's Desktop lists the models bare; effort has its own chip.
    let mut entries = vec![];
    let (latest, more) = split_models(models);
    let item = |model: &quill_model::ModelInfo| {
        limn::DropdownMenuItem::new(format!("model:{}", model.id), model.name.clone())
            .selected(current.is_some_and(|c| c.id == model.id))
    };
    for (n, model) in latest.iter().enumerate() {
        // The current model shows its check in place of its number.
        let row = item(model);
        let row = if current.is_some_and(|c| c.id == model.id) {
            row
        } else {
            row.shortcut((n + 1).to_string())
        };
        entries.push(row.into());
    }
    if !more.is_empty() {
        entries.push(limn::DropdownMenuEntry::Separator);
        let rest = more.iter().map(|m| item(m).into()).collect();
        entries.push(
            limn::DropdownMenuItem::new(format!("{MODEL_MENU}:more"), "More models")
                .submenu(rest)
                .into(),
        );
    }
    menu(
        anchor,
        entries,
        open,
        MODEL_MENU,
        200.0,
        limn::PopoverPlacement::TopEnd,
    )
}

/// The model the composer runs: the one picked, matched by id or by name.
fn current_model<'a>(
    store: &'a Store,
    session: Option<&'a Session>,
    ui: &Ui,
) -> Option<&'a quill_model::ModelInfo> {
    offered_models(store, session).iter().find(|m| {
        ui.model
            .as_deref()
            .is_some_and(|id| m.id == id || m.name.to_lowercase().contains(id))
    })
}

/// The effort chip and its popover, as the person's Desktop has them:
/// "Effort  Low", "Faster … Smarter" over a slider with a stop per level the model
/// accepts, and "Recommended" under medium.
fn effort_menu(store: &Store, session: Option<&Session>, ui: &Ui) -> Element {
    let efforts = current_model(store, session, ui)
        .map(|m| m.efforts.clone())
        .unwrap_or_default();
    if efforts.is_empty() {
        return Element::spacer().size(Some(0.0), Some(0.0));
    }
    let open = ui.open_menu == Some(EFFORT_MENU);
    let effort = ui
        .effort
        .clone()
        .filter(|e| efforts.contains(e))
        .unwrap_or_else(|| efforts[0].clone());
    let anchor = chip(
        EFFORT_MENU,
        vec![chip_text(effort_label(&effort), look::palette().text)],
        open,
        "Effort",
        None,
    );
    let width = 188.0;
    let recommended = efforts.iter().position(|e| e == "medium");
    // Desktop's slider: a 23 px #E6E6E6 pill, a white 20 px thumb on the level,
    // a tick on the recommended one and dots on the rest; each stop is a click away.
    // The ends sit 11.5 px in, a thumb's half and a hair; the stops between share the rest.
    let n = efforts.len();
    let step = if n > 1 {
        (width - 23.0) / (n - 1) as f32
    } else {
        0.0
    };
    let stops: Vec<Element> = efforts
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let (mark, side) = if *e == effort {
                (
                    Element::spacer()
                        .background(look::palette().surface)
                        .border(look::composer_border(), 1.0)
                        .radius(10.0)
                        .size(Some(20.0), Some(20.0)),
                    20.0,
                )
            } else if Some(i) == recommended {
                (
                    Element::spacer()
                        .background(look::slider_mark())
                        .radius(1.0)
                        .size(Some(2.0), Some(9.0)),
                    2.0,
                )
            } else {
                (
                    Element::spacer()
                        .background(look::slider_mark())
                        .radius(1.5)
                        .size(Some(3.0), Some(3.0)),
                    3.0,
                )
            };
            let inset = 11.5 - side / 2.0;
            let end = 11.5 + step / 2.0;
            let (cell, content) = if n == 1 {
                (width, mark.centered())
            } else if i == 0 {
                (end, Element::row(vec![hgap(inset), mark]).centered_y())
            } else if i == n - 1 {
                (
                    end,
                    Element::row(vec![hgap(end - inset - side), mark]).centered_y(),
                )
            } else {
                (step, mark.centered())
            };
            Element::clickable(format!("effort:{e}"), content).size(Some(cell), Some(23.0))
        })
        .collect();
    let track = Element::row(stops)
        .background(look::slider_track())
        .radius(11.5)
        .size(Some(width), Some(23.0));
    let mut rows = vec![
        Element::row(vec![
            muted("Effort").font_size(BODY),
            text(effort_label(&effort)).font_size(BODY),
            Element::spacer().flex(1.0),
            limn::Tooltip::new(
                Element::icon(icons::of(icons::QUESTION))
                    .foreground(look::muted())
                    .size(Some(16.0), Some(16.0)),
                "effort:help",
                "How long Claude thinks before it answers",
                look::palette(),
            )
            .render(),
        ])
        .gap(10.0)
        .size(Some(width), None),
        Element::spacer().size(None, Some(6.0)),
        Element::row(vec![
            muted("Faster").font_size(SMALL),
            Element::spacer().flex(1.0),
            muted("Smarter").font_size(SMALL),
        ])
        .size(Some(width), None),
        track,
    ];
    if let Some(recommended) = recommended {
        // Under its stop, as Desktop labels the recommended level.
        let x = (11.5 + step * recommended as f32 - 42.0).max(0.0);
        rows.push(Element::row(vec![
            hgap(x),
            muted("Recommended").font_size(SMALL),
        ]));
    }
    // The menu adds 8 px of its own: 16 in all, as Desktop's.
    let card = Element::column(rows).gap(10.0).padding(8.0);
    menu(
        anchor,
        vec![limn::DropdownMenuEntry::Custom(Box::new(card))],
        open,
        EFFORT_MENU,
        width + 32.0,
        limn::PopoverPlacement::TopEnd,
    )
}

/// What one request changed, under its turn: every file with how it changed,
/// and a real undo (Desktop's asks the agent instead, engine.md §6).
fn changes_card(
    request: &str,
    files: &[FileEffect],
    rewound: bool,
    column: f32,
    palette: ComponentPalette,
) -> Element {
    let (added, removed) = files
        .iter()
        .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
    let count = if files.len() == 1 {
        "1 file".to_string()
    } else {
        format!("{} files", files.len())
    };
    let head = Element::row(vec![
        text(format!("Changed {count}")),
        text(format!("+{added}")).foreground(look::added()),
        text(format!("−{removed}")).foreground(look::removed()),
    ])
    .gap(6.0);
    let mut card = FramedCard::new(palette).width(column).section(head);
    let rows = files
        .iter()
        .map(|f| {
            let (mark, color) = match &f.kind {
                ChangeKind::Added => ("A".to_string(), look::added()),
                ChangeKind::Deleted => ("D".to_string(), look::removed()),
                ChangeKind::Renamed { from } => (format!("R ← {from}"), look::muted()),
                ChangeKind::Modified => ("M".to_string(), look::muted()),
            };
            Element::row(vec![
                text(f.path.clone())
                    .font_family(FontFamily::Monospace)
                    .flex(1.0),
                text(mark).foreground(color),
                muted(format!("+{} −{}", f.added, f.removed)),
            ])
            .gap(10.0)
        })
        .collect();
    card = card.section(Element::column(rows).gap(4.0));
    let foot = if rewound {
        muted("Files restored to before this request")
    } else {
        ButtonGroup::new(vec![
            Button::new(format!("changes:{request}:undo"), "Undo", palette)
                .variant(ButtonVariant::Ghost)
                .button_size(ButtonSize::Small)
                .render(),
            Button::new(format!("changes:{request}:view"), "View changes", palette)
                .variant(ButtonVariant::Outline)
                .button_size(ButtonSize::Small)
                .render(),
        ])
        .align_end()
        .render()
    };
    card.foot(foot).render()
}

/// The changes of one request, file by file, as unified diffs.
fn diff_pane(
    store: &Store,
    ui: &Ui,
    width: f32,
    height: f32,
    palette: ComponentPalette,
) -> Element {
    let files = diff_files(store, ui).map_or(&[][..], Vec::as_slice);
    // "master → working tree" for what is uncommitted, as Desktop's; a request's
    // own changes when the person opened them from its card.
    let title = if ui.diff.is_some() {
        text("Changes in this request").centered_y()
    } else {
        let branch = ui
            .selected
            .and_then(|id| store.session(id))
            .and_then(|s| s.branch.clone())
            .unwrap_or_else(|| "HEAD".into());
        Element::row(vec![
            text(branch).centered_y(),
            muted("→").centered_y(),
            text("working tree").centered_y(),
        ])
        .gap(8.0)
    };
    let header = pane_header(title, pane_tools(Pane::Diff, ui, palette), width);
    let mut blocks = vec![];
    if files.is_empty() {
        blocks.push(pad(muted("No uncommitted changes"), 8.0, 8.0));
    }
    for (index, file) in files.iter().enumerate() {
        blocks.push(diff_file_band(file, width));
        let lines = parse_patch(&file.patch);
        let rows = lines.len().max(1) as f32;
        let words = word_ranges(&lines);
        blocks.push(
            limn::CodeDiff::new(format!("diff:{index}"), lines)
                .word_ranges(words)
                .palette(palette)
                .font_size(12.0)
                .height((rows * 19.0 + 16.0).min(600.0))
                .render(),
        );
    }
    let body = Element::scroll(
        "diff:scroll",
        Element::column(blocks).size(Some(width), None),
    );
    Element::column(vec![header, body.flex(1.0)])
        .background(look::theme().background)
        .size(Some(width), Some(height))
}

/// A file's band over its diff, as Desktop's: #F2F2F2, the name in #616161 after
/// a caret and the code mark, then its added and removed lines.
fn diff_file_band(file: &FileEffect, width: f32) -> Element {
    let name = file
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&file.path)
        .to_owned();
    Element::row(vec![
        hgap(16.0),
        text("▾")
            .font_size(10.0)
            .foreground(look::muted())
            .centered_y(),
        text("‹›")
            .font_size(SMALL)
            .foreground(look::muted())
            .centered_y(),
        text(name).foreground(look::command_ink()).centered_y(),
        Element::spacer().flex(1.0),
        text(format!("+{}", file.added))
            .foreground(look::added())
            .centered_y(),
        text(format!("−{}", file.removed))
            .foreground(look::removed())
            .centered_y(),
        hgap(16.0),
    ])
    .gap(6.0)
    .background(look::cancel())
    .size(Some(width), Some(32.0))
}

/// Lines of a unified diff, numbered from its hunk headers.
pub fn parse_patch(patch: &str) -> Vec<limn::CodeDiffLine> {
    use limn::{CodeDiffLine, CodeDiffLineKind};
    let (mut old, mut new) = (0usize, 0usize);
    let mut lines = vec![];
    let mut in_hunk = false;
    for line in patch.lines() {
        if let Some(header) = line.strip_prefix("@@ ") {
            // "@@ -a,b +c,d @@": the first line numbers of the hunk.
            let mut parts = header.split_whitespace();
            let start = |p: Option<&str>| {
                p.and_then(|p| p[1..].split(',').next())
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(1)
            };
            old = start(parts.next());
            new = start(parts.next());
            in_hunk = true;
            continue;
        }
        if !in_hunk || line.starts_with("\\ ") {
            continue;
        }
        let (kind, text) = match line.chars().next() {
            Some('+') => (CodeDiffLineKind::Added, &line[1..]),
            Some('-') => (CodeDiffLineKind::Removed, &line[1..]),
            Some(' ') => (CodeDiffLineKind::Context, &line[1..]),
            _ => (CodeDiffLineKind::Context, line),
        };
        let (old_line, new_line) = match kind {
            CodeDiffLineKind::Added => (None, Some(new)),
            CodeDiffLineKind::Removed => (Some(old), None),
            // The parser makes no folded rows; they are the panel's own (plan Q5.5).
            CodeDiffLineKind::Context | CodeDiffLineKind::Folded => (Some(old), Some(new)),
        };
        if old_line.is_some() {
            old += 1;
        }
        if new_line.is_some() {
            new += 1;
        }
        lines.push(CodeDiffLine {
            kind,
            old_line,
            new_line,
            text: text.to_owned(),
        });
    }
    lines
}

/// A question Claude asks, one at a time: a 480 px card with the
/// question on a gray band and "‹ 1 of 2 ›" when there are more; each option a row with its
/// mark (a radio, or a box when several may be chosen), label and muted description, lines
/// between; "Other" with a field for the person's own words; Skip, and Next or
/// Submit with Ctrl ⏎. A single choice starts on its first option, as Desktop's did.
fn question_card(
    id: &str,
    input: &Value,
    ui: &Ui,
    column: f32,
    palette: ComponentPalette,
) -> Element {
    let questions = input["questions"].as_array().cloned().unwrap_or_default();
    let width = column.min(480.0);
    let count = questions.len().max(1);
    let page = ui.question.get(id).copied().unwrap_or(0).min(count - 1);
    let question = &questions.get(page).cloned().unwrap_or_default();
    let multi = question["multiSelect"] == true;
    let chosen = picked(ui, id, page, question);
    let mut band = vec![
        text(question["question"].as_str().unwrap_or_default().to_owned())
            .font_size(BODY)
            .size(Some(width - 140.0), None)
            .centered_y(),
        Element::spacer().flex(1.0),
    ];
    if count > 1 {
        let arrow = |dir: &str, icon: &'static str, label: &str, on: bool| {
            let glyph = Element::icon(icons::of(icon))
                .foreground(if on {
                    look::palette().text
                } else {
                    look::muted()
                })
                .size(Some(16.0), Some(16.0))
                .centered();
            let button = Element::panel(glyph)
                .size(Some(24.0), Some(24.0))
                .centered_y();
            if on {
                Element::clickable(format!("ask:{id}:{dir}"), button).accessibility_label(label)
            } else {
                button
            }
        };
        band.push(arrow("prev", icons::CARET_LEFT, "Back", page > 0));
        band.push(
            muted(format!("{} of {count}", page + 1))
                .font_size(12.0)
                .centered_y(),
        );
        band.push(arrow("next", icons::CARET_RIGHT, "Next", page + 1 < count));
    }
    let mut rows = vec![
        Element::panel(Element::row(band).gap(6.0).size(None, Some(24.0)))
            .background(look::user_bubble())
            // Inside the card's rounded edge.
            .corner_radii(limn::CornerRadii {
                top_left: 11.0,
                top_right: 11.0,
                ..Default::default()
            })
            .padding(10.0)
            .size(Some(width), None),
    ];
    let mut options: Vec<(String, String)> = question["options"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|o| {
            (
                o["label"].as_str().unwrap_or_default().to_owned(),
                o["description"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    options.push(("Other".into(), String::new()));
    for (o, (label, description)) in options.iter().enumerate() {
        if o > 0 {
            rows.push(hline(width, look::card_border()));
        }
        let action = format!("ask:{id}:{page}:{o}");
        let on = chosen.contains(label);
        let mark = if multi {
            limn::Checkbox::new(&action, on, palette)
                .label(label.clone())
                // Desktop's chosen marks are its blue.
                .accent(look::handle())
                .render()
        } else {
            let option =
                limn::RadioOption::new(&action, label.clone()).description(description.clone());
            limn::RadioGroup::new(
                format!("{action}:group"),
                vec![option],
                on.then_some(action.as_str()),
                palette,
            )
            .accent(look::handle())
            .render()
        };
        let mut row = vec![mark];
        if multi && !description.is_empty() {
            row.push(pad(muted(description.clone()).font_size(BODY), 26.0, 0.0));
        }
        if label == "Other" && on {
            let typed = ui
                .other
                .get(&(id.to_owned(), page))
                .map_or("", String::as_str);
            row.push(pad(
                limn::TextField::new(
                    format!("ask-other:{id}:{page}"),
                    typed,
                    "Type your own answer here",
                    palette,
                )
                .width(width - 74.0)
                .render(),
                26.0,
                0.0,
            ));
        }
        rows.push(
            Element::column(row)
                .gap(6.0)
                .padding(16.0)
                .size(Some(width), None),
        );
    }
    let last = page + 1 == count;
    let (go, label) = if last {
        ("submit", "Submit")
    } else {
        ("next", "Next")
    };
    rows.push(
        Element::row(vec![
            Element::spacer().flex(1.0),
            keyed_button(&format!("ask:{id}:skip"), "Skip", "", false, true, palette),
            keyed_button(
                &format!("ask:{id}:{go}"),
                label,
                "Ctrl+⏎",
                true,
                !chosen.is_empty(),
                palette,
            ),
        ])
        .gap(8.0)
        .padding(16.0)
        .size(Some(width), None),
    );
    Element::panel(Element::column(rows))
        .background(palette.surface)
        .border(look::card_border(), 1.0)
        .radius(12.0)
        .size(Some(width), None)
}

/// The labels the person chose for question `page` of `permission`; a single choice not yet
/// touched is its first option.
pub fn picked(ui: &Ui, permission: &str, page: usize, question: &Value) -> Vec<String> {
    match ui.answers.get(&(permission.to_owned(), page)) {
        Some(labels) => labels.clone(),
        None if question["multiSelect"] != true => question["options"][0]["label"]
            .as_str()
            .map(|l| vec![l.to_owned()])
            .unwrap_or_default(),
        None => vec![],
    }
}

/// One card of the right panel, as Desktop's: a 280 px white card with a #EBEBEB
/// edge, radius 12 and 18 px in, a 13 px title with its caret, and what it holds.
fn panel_card(
    title: &str,
    id: &str,
    open: bool,
    trailing: Vec<Element>,
    rows: Vec<Element>,
    width: f32,
) -> Element {
    let mut head = vec![text(title).font_size(SMALL).centered_y(), caret(open)];
    head.push(Element::spacer().flex(1.0));
    head.extend(trailing);
    let head = Element::clickable(id, Element::row(head).gap(6.0).size(None, Some(20.0)));
    let mut content = vec![head];
    if open {
        content.extend(rows);
    }
    Element::panel(Element::column(content).gap(14.0))
        .background(look::palette().surface)
        .border(look::card_border(), 1.0)
        .radius(12.0)
        .padding(18.0)
        .size(Some(width), None)
}

/// A row of a panel card: a 16 px icon and its words.
fn panel_row(icon: limn::Icon, words: Element) -> Element {
    Element::row(vec![
        Element::icon(icon)
            .foreground(look::palette().text)
            .size(Some(16.0), Some(16.0))
            .centered_y(),
        words.centered_y(),
    ])
    .gap(10.0)
    .size(None, Some(20.0))
}

/// Where the session works, how full its context is and what it changed.
fn right_panel(
    session: &Session,
    ui: &Ui,
    width: f32,
    height: f32,
    _palette: ComponentPalette,
) -> Element {
    let inner = width - 16.0;
    let open = |key: &str, default: bool| ui.expanded.iter().any(|k| k == key) != default;
    let mut environment = vec![
        panel_row(icons::of(icons::LAPTOP), text("This computer")),
        panel_row(icons::of(icons::FOLDER), text(folder_name(&session.cwd))),
    ];
    if let Some(branch) = &session.branch {
        environment.push(panel_row(
            icons::of(icons::GIT_BRANCH),
            text(branch.clone()),
        ));
    }
    // How full the context is, as Desktop's percentage and bar: the window comes from the
    // agent's own report of its model.
    let share = (session.context_window > 0)
        .then(|| session.context_tokens as f32 / session.context_window as f32);
    let mut context_head = vec![];
    if let Some(share) = share {
        context_head.push(
            muted(format!("{}%", (share * 100.0).round() as u32))
                .font_size(SMALL)
                .centered_y(),
        );
        context_head.push(context_bar(share));
    }
    let mut context_rows = vec![muted(if session.context_tokens == 0 {
        "No turns yet".to_string()
    } else if session.context_window > 0 {
        format!(
            "{:.1}k of {}k tokens",
            session.context_tokens as f64 / 1000.0,
            session.context_window / 1000
        )
    } else {
        format!("{:.1}k tokens", session.context_tokens as f64 / 1000.0)
    })];
    if session.cost > 0.0 {
        context_rows.push(muted(format!("${:.4} this session", session.cost)));
    }
    let (added, removed, files) = match &session.working {
        Some(working) => (
            working.iter().map(|f| f.added).sum::<u32>(),
            working.iter().map(|f| f.removed).sum::<u32>(),
            working.len(),
        ),
        None => session
            .items
            .iter()
            .fold((0, 0, 0), |(a, r, n), item| match item {
                Item::Changes {
                    files,
                    rewound: false,
                    ..
                } => (
                    a + files.iter().map(|f| f.added).sum::<u32>(),
                    r + files.iter().map(|f| f.removed).sum::<u32>(),
                    n + files.len(),
                ),
                _ => (a, r, n),
            }),
    };
    let changes = if files == 0 {
        muted("Nothing changed yet")
    } else {
        // The row opens the diff, as Desktop's does.
        Element::clickable(
            DIFF_TOGGLE,
            Element::row(vec![
                Element::icon(icons::of(icons::DIFF))
                    .foreground(look::palette().text)
                    .size(Some(16.0), Some(16.0))
                    .centered_y(),
                text(format!("+{added}"))
                    .foreground(look::added())
                    .font_size(SMALL)
                    .centered_y(),
                text(format!("−{removed}"))
                    .foreground(look::removed())
                    .font_size(SMALL)
                    .centered_y(),
            ])
            .gap(8.0)
            .size(None, Some(24.0)),
        )
    };
    Element::column(vec![
        panel_card(
            "Environment",
            "panel:environment",
            open("panel:environment", true),
            vec![],
            environment,
            inner,
        ),
        panel_card(
            "Context",
            "panel:context",
            open("panel:context", false),
            context_head,
            context_rows,
            inner,
        ),
        panel_card(
            "Changes",
            "panel:changes",
            open("panel:changes", true),
            vec![],
            vec![changes],
            inner,
        ),
    ])
    .gap(8.0)
    .padding(8.0)
    .size(Some(width), Some(height))
}

/// Desktop's small context bar: a 24 × 6 pill filled blue to the share used.
fn context_bar(share: f32) -> Element {
    let fill = (24.0 * share.clamp(0.0, 1.0)).max(3.0);
    Element::row(vec![Element::spacer()
        .background(look::handle())
        .radius(3.0)
        .size(Some(fill), Some(6.0))])
    .background(look::active())
    .radius(3.0)
    .size(Some(24.0), Some(6.0))
    .centered_y()
}

/// What changed inside each rewritten line, for the word highlights of the diff:
/// each run of removed lines is paired with the added run after it, line by line, and the
/// characters between their common start and common end are marked.
pub fn word_ranges(lines: &[limn::CodeDiffLine]) -> Vec<Vec<std::ops::Range<usize>>> {
    use limn::CodeDiffLineKind::{Added, Removed};
    let mut ranges = vec![Vec::new(); lines.len()];
    let mut i = 0;
    while i < lines.len() {
        let removed: Vec<usize> = (i..lines.len())
            .take_while(|&j| lines[j].kind == Removed)
            .collect();
        let after = i + removed.len();
        let added: Vec<usize> = (after..lines.len())
            .take_while(|&j| lines[j].kind == Added)
            .collect();
        for (&r, &a) in removed.iter().zip(&added) {
            let old: Vec<char> = lines[r].text.chars().collect();
            let new: Vec<char> = lines[a].text.chars().collect();
            let start = old.iter().zip(&new).take_while(|(x, y)| x == y).count();
            let end = old[start..]
                .iter()
                .rev()
                .zip(new[start..].iter().rev())
                .take_while(|(x, y)| x == y)
                .count();
            if start + end < old.len() {
                ranges[r].push(start..old.len() - end);
            }
            if start + end < new.len() {
                ranges[a].push(start..new.len() - end);
            }
        }
        i = (after + added.len()).max(i + 1);
    }
    ranges
}

/// A helper agent's own pane: "‹" and what it was asked to do,
/// then its prompt as the person's bubble, the model it ran on, and its steps and words.
fn agent_pane(
    store: &Store,
    session: Option<&Session>,
    ui: &Ui,
    width: f32,
    height: f32,
    palette: ComponentPalette,
) -> Element {
    let found = session.zip(ui.helper.as_deref()).and_then(|(s, call)| {
        s.items.iter().find_map(|i| match i {
            Item::Tool { call: c, input, .. } if c == call => Some((s, c.as_str(), input)),
            _ => None,
        })
    });
    let title = found.map_or("", |(_, _, input)| {
        input["description"].as_str().unwrap_or_default()
    });
    let back = icon_tool(
        "pane:agent:close",
        icons::of(icons::CARET_LEFT),
        "Back",
        28.0,
        palette,
    );
    let lead = Element::row(vec![back, text(title.to_owned()).centered_y()]).gap(4.0);
    let header = pane_header(lead, pane_tools(Pane::Agent, ui, palette), width);
    let Some((session, call, input)) = found else {
        return Element::column(vec![header]).size(Some(width), Some(height));
    };
    let inner = width - 48.0;
    let mut body = vec![Element::panel(
        text(input["prompt"].as_str().unwrap_or_default().to_owned())
            .font_size(BODY)
            .size(Some(inner - 32.0), None),
    )
    .background(look::user_bubble())
    .radius(14.0)
    .padding(16.0)];
    // "Model Haiku 5.5": the model it was given, else the session's.
    let models = offered_models(store, Some(session));
    let model = input["model"]
        .as_str()
        .and_then(|id| {
            models
                .iter()
                .find(|m| m.id == id || m.name.to_lowercase().contains(id))
        })
        .or_else(|| current_model(store, Some(session), ui));
    body.push(Element::spacer().size(None, Some(16.0)));
    if let Some(model) = model {
        body.push(muted(format!("Model {}", model.name)).font_size(SMALL));
    }
    let running = session.task_running(call);
    let helper = session.helpers.get(call).map_or(&[][..], Vec::as_slice);
    body.extend(entries(session, helper, running, ui, inner, palette).0);
    if running {
        body.push(
            Element::row(vec![
                Spinner::new("agent:dots", look::spark())
                    .variant(limn::SpinnerVariant::Dots)
                    .size(16.0)
                    .decorative()
                    .render()
                    .centered_y(),
                muted("Working").font_size(BODY).centered_y(),
            ])
            .gap(12.0),
        );
    }
    let content = Element::column(body).gap(12.0).padding(24.0);
    let scroll = Element::scroll(format!("agent:{call}:scroll"), content);
    Element::column(vec![header, scroll.flex(1.0)]).size(Some(width), Some(height))
}

/// The plan Claude proposed, in its own pane: "Plan" over the
/// plan's markdown.
fn plan_pane(session: Option<&Session>, ui: &Ui, width: f32, height: f32) -> Element {
    let palette = look::palette();
    let header = pane_header(
        text("Plan").centered_y(),
        pane_tools(Pane::Plan, ui, palette),
        width,
    );
    let plan = session
        .zip(ui.plan.as_deref())
        .and_then(|(s, id)| plan_text(&s.items, id))
        .unwrap_or_default();
    let inner = width - 48.0;
    let content = Markdown::new(&plan, palette)
        .width(inner)
        .font_size(BODY)
        .render()
        .element
        .padding(24.0);
    let scroll = Element::scroll("plan:scroll", content);
    Element::column(vec![header, scroll.flex(1.0)]).size(Some(width), Some(height))
}

/// The plan a permission or a tool call `id` carries.
pub fn plan_text(items: &[Item], id: &str) -> Option<String> {
    items.iter().find_map(|i| match i {
        Item::Permission { id: p, input, .. } | Item::Tool { call: p, input, .. } if p == id => {
            input["plan"].as_str().map(str::to_owned)
        }
        _ => None,
    })
}
