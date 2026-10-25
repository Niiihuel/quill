//! The window: sidebar of sessions, transcript and composer (features/code-*.md, docs/ui.md §2).

use crate::look::{self, BODY, CHAT, COLUMN, SMALL};
use crate::{Item, Pane, Session, Store, ToolResult, Ui};
use limn::{
    Button, ButtonGroup, ButtonSize, ButtonVariant, Component, ComponentPalette, Composer,
    ControlSize, Element, FontFamily, FramedCard, Markdown, OutputBlock, Size, TextShimmer,
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
const TERMINAL_FONT: f32 = 12.0;
/// The shell's text starts 12 px in and 8 px under the pane's header.
const TERMINAL_PADDING: [f32; 2] = [12.0, 8.0];

fn text(s: impl Into<String>) -> Element {
    Element::text(s.into())
        .font_family(FontFamily::Sans)
        .font_size(BODY)
        .foreground(look::PALETTE.text)
}

/// The last part of a path, as Desktop names a session's folder.
fn folder_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn muted(s: impl Into<String>) -> Element {
    text(s).foreground(look::MUTED)
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
    let rest = (window.width - look::SIDEBAR_WIDTH).max(0.0);
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
        Some(Pane::Terminal) => 1.0 / 3.0,
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
        hover: look::HANDLE,
        selected: look::HANDLE,
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
            hline(length, look::PANE_BORDER),
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
        .background(look::DIVIDER)
        .size(Some(1.0), Some(height))
}

fn hline(width: f32, color: limn::Color) -> Element {
    Element::spacer()
        .background(color)
        .size(Some(width), Some(1.0))
}

pub fn view(store: &Store, ui: &Ui, window: Size) -> Element {
    let palette = look::PALETTE;
    let session = ui.selected.and_then(|id| store.session(id));
    let layout = layout(store, ui, window);
    let rest = layout.main + layout.column + layout.panel;
    let below = window.height - look::TOP_BAR - 1.0;
    let mut row = vec![];
    if layout.main > 0.0 {
        let column = COLUMN.min(layout.main - 64.0).max(240.0);
        let body = match session {
            Some(session) => transcript(session, ui, column, palette),
            None => empty(ui),
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
        hline(rest, look::PANE_BORDER),
        Element::row(row).size(Some(rest), Some(below)),
    ])
    .size(Some(rest), Some(window.height));
    let root = Element::row(vec![
        sidebar(store, ui, window.height, palette),
        vline(window.height),
        content,
    ])
    .background(look::THEME.background)
    .size(Some(window.width), Some(window.height));
    // Desktop's own wording for both closes (Ion `cccc2cf0a`,).
    let tabs = &ui.terminal.tabs;
    let (title, description, confirm) = match ui.terminal.closing {
        Some(crate::Closing::Others(keep)) => {
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
        Some(crate::Closing::One(tab)) => (
            "Close terminal?",
            format!(
                "A command is still running in {}. Closing the terminal stops it.",
                tabs.get(tab).map_or("the terminal", |t| t.title.as_str())
            ),
            "Close terminal",
        ),
        None => ("", String::new(), ""),
    };
    limn::AlertDialog::new(
        root,
        ui.terminal.closing.is_some(),
        title,
        description,
        TERMINAL_CLOSE_CONFIRM,
        TERMINAL_CLOSE_CANCEL,
        // Cancel is filled with `raised`: Desktop's light gray.
        ComponentPalette {
            raised: look::CANCEL,
            ..palette
        },
    )
    .width(400.0)
    .confirm_label(confirm)
    .soft_destructive(look::DANGER_FILL, look::DANGER_BORDER, look::DANGER_INK)
    .cancel_variant(ButtonVariant::Secondary)
    .description_size(BODY)
    .description_line_height(20.0)
    .closable(false)
    // Desktop's backdrop is lighter, its title closer, its buttons 30 px with 14 px labels.
    .scrim(look::SCRIM)
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
        hline(width, look::PANE_BORDER),
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
                limn::Icon::Collapse
            } else {
                limn::Icon::Expand
            },
            if expanded { "Collapse" } else { "Expand" },
            28.0,
            palette,
        ),
        icon_tool(
            &format!("pane:{key}:close"),
            limn::Icon::Close,
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
    let new = icon_tool(TERMINAL_NEW, limn::Icon::Add, "New terminal", 28.0, palette);
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
            Some((0, ref name)) => rename_field(name, palette),
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
                        return rename_field(name, palette);
                    }
                }
                // "Terminal 2  work ×": a named tab drops the folder.
                let mut items = vec![hgap(8.0), text(tab.title.clone()).centered_y()];
                if !tab.named {
                    items.push(muted(folder.clone()).centered_y());
                }
                items.push(icon_tool(
                    &format!("terminal:close:{i}"),
                    limn::Icon::Close,
                    "Close terminal",
                    20.0,
                    palette,
                ));
                let pill = Element::row(items)
                    .gap(6.0)
                    .size(None, Some(28.0))
                    .radius(8.0);
                let pill = if i == ui.terminal.active {
                    pill.background(look::ACTIVE)
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
                    text: look::CURSOR,
                    ..palette
                },
            )
            .font_size(TERMINAL_FONT)
            .background(look::THEME.background)
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

/// The name field a pill turns into while the person renames it.
fn rename_field(name: &str, palette: ComponentPalette) -> Element {
    limn::TextField::new(TERMINAL_RENAME, name, "", palette)
        .size(ControlSize::Compact)
        .width(160.0)
        .render()
        .border(look::CARD_BORDER, 1.0)
        .focus_border(look::HANDLE, 1.0)
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
    tool_surface(id, icon, label, side, None, look::TOOL_HOVER, None).centered_y()
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
        ..look::PALETTE
    };
    let button = palette
        .interactive_surface(Element::icon_button(id, icon, label))
        .foreground(look::PALETTE.text)
        .radius(6.0)
        .size(Some(side), Some(side));
    let button = match fill {
        Some(fill) => button.background(fill),
        None => button,
    };
    let tip = limn::Tooltip::new(button, id, label, look::PALETTE);
    match shortcut {
        Some(keys) => tip.shortcut(keys),
        None => tip,
    }
    .render()
}

/// A top-bar toggle: its icon on the active fill while what it shows is open.
fn toggle(id: &str, icon: limn::Icon, label: &str, on: bool, shortcut: Option<&str>) -> Element {
    let (fill, hover) = if on {
        (Some(look::ACTIVE), look::ACTIVE_HOVER)
    } else {
        (None, look::TOOL_HOVER)
    };
    tool_surface(id, icon, label, 26.0, fill, hover, shortcut).centered_y()
}

/// The session's title, Stop while it works, and the toggles of the panel and the panes
///.
fn top_bar(session: Option<&Session>, ui: &Ui) -> Element {
    // An expanded pane leaves the bar empty, as Desktop's does.
    if ui.expanded_pane.is_some() {
        return Element::spacer().size(None, Some(look::TOP_BAR));
    }
    let title = session.map_or("New session".to_string(), |s| {
        if s.title.is_empty() {
            "New session".into()
        } else {
            s.title.clone()
        }
    });
    let title = if session.is_some() {
        text(title)
    } else {
        muted(title)
    };
    let mut row = vec![title.centered_y(), Element::spacer().flex(1.0)];
    // Stop lives in the composer, as in Desktop.
    if session.is_some() {
        let shown = ui.panel_open && ui.panes.is_empty();
        row.push(toggle(
            PANEL_TOGGLE,
            limn::Icon::PanelRight,
            if shown {
                "Hide session rail"
            } else {
                "Show session rail"
            },
            shown,
            None,
        ));
    }
    row.push(toggle(
        TERMINAL_TOGGLE,
        limn::Icon::Terminal,
        "Terminal",
        ui.pane_open(Pane::Terminal),
        Some("Ctrl+`"),
    ));
    if session.is_some() {
        row.push(toggle(
            DIFF_TOGGLE,
            limn::Icon::Code,
            "Diff",
            ui.pane_open(Pane::Diff),
            Some("Ctrl+Shift+D"),
        ));
    }
    row.insert(0, hgap(20.0));
    row.push(hgap(8.0));
    Element::row(row).gap(4.0).size(None, Some(look::TOP_BAR))
}

/// Desktop's sidebar: New, then the sessions under a header per folder that folds
/// them, the folder with the newest session first.
fn sidebar(store: &Store, ui: &Ui, height: f32, palette: ComponentPalette) -> Element {
    let mut rows = vec![
        // Desktop keeps this band for its window controls; quill's window has its own.
        Element::spacer().size(None, Some(look::TOP_BAR)),
        side_row(
            NEW_SESSION.into(),
            vec![
                Element::icon(limn::Icon::Add)
                    .foreground(look::PALETTE.text)
                    .size(Some(16.0), Some(16.0))
                    .centered_y(),
                text("New").centered_y(),
                Element::spacer().flex(1.0),
                muted("Ctrl+N")
                    .font_size(12.0)
                    .centered_y()
                    .reveal_on_hover("new-session:keys"),
            ],
            false,
            palette,
        ),
        Element::spacer().size(None, Some(16.0)),
    ];
    let mut folders: Vec<&std::path::PathBuf> = vec![];
    for session in store.sessions.iter().rev() {
        if !folders.contains(&&session.cwd) {
            folders.push(&session.cwd);
        }
    }
    for folder in folders {
        let open = !ui.collapsed.contains(folder);
        let caret = Element::icon(if open {
            limn::Icon::ChevronDown
        } else {
            limn::Icon::ChevronRight
        })
        .foreground(look::MUTED)
        .size(Some(10.0), Some(10.0))
        .centered_y();
        // Under the pointer the header fills and offers a new session there.
        let new_here = format!("new-in:{}", folder.display());
        let header = Element::row(vec![
            hgap(10.0),
            muted(folder_name(folder)).font_size(SMALL).centered_y(),
            caret,
            Element::spacer().flex(1.0),
            tool_surface(
                &new_here,
                limn::Icon::Add,
                "New session",
                22.0,
                None,
                look::TOOL_HOVER,
                None,
            )
            .centered_y()
            .reveal_on_hover(format!("{new_here}:reveal")),
            hgap(4.0),
        ])
        .gap(6.0)
        .size(None, Some(28.0));
        let header_palette = ComponentPalette {
            hover: look::ROW_HOVER,
            ..palette
        };
        rows.push(
            header_palette
                .interactive_surface(Element::clickable(
                    format!("group:{}", folder.display()),
                    header,
                ))
                .radius(8.0)
                .hover_region(),
        );
        if !open {
            continue;
        }
        for session in store.sessions.iter().rev().filter(|s| &s.cwd == folder) {
            let title = if session.title.is_empty() {
                "New session".to_string()
            } else {
                session.title.clone()
            };
            rows.push(side_row(
                format!("session:{}", session.id),
                vec![
                    Element::truncated_text(title)
                        .font_family(FontFamily::Sans)
                        .font_size(BODY)
                        .foreground(look::PALETTE.text)
                        .flex(1.0)
                        .centered_y(),
                    status_dot(&session.status),
                ],
                ui.selected == Some(session.id),
                palette,
            ));
        }
    }
    Element::column(rows)
        .gap(2.0)
        .padding(8.0)
        .background(look::SIDEBAR)
        .size(Some(look::SIDEBAR_WIDTH - 1.0), Some(height))
}

/// A sidebar row: 32 px tall, 10 px in, rounded, filled while selected.
fn side_row(
    id: String,
    content: Vec<Element>,
    selected: bool,
    palette: ComponentPalette,
) -> Element {
    let mut items = vec![hgap(10.0)];
    items.extend(content);
    items.push(hgap(10.0));
    let row = Element::row(items).gap(10.0).size(None, Some(32.0));
    // Under the pointer a row turns #EDEDED, a selected one from #E0E0E0 to #D5D5D5
    //.
    let palette = ComponentPalette {
        hover: if selected {
            look::SELECTED_HOVER
        } else {
            look::ROW_HOVER
        },
        ..palette
    };
    let row = palette
        .interactive_surface(Element::clickable(id, row))
        .radius(8.0)
        .hover_region();
    if selected {
        row.background(look::SELECTED)
    } else {
        row
    }
}

/// Running and awaiting sessions carry a dot in the sidebar, as Desktop's rows do.
fn status_dot(status: &SessionStatus) -> Element {
    let color = match status {
        SessionStatus::Running | SessionStatus::Starting => look::RUNNING,
        SessionStatus::Awaiting => look::AWAITING,
        SessionStatus::Failed(_) => look::REMOVED,
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

/// A new session's page is empty above the composer, as Desktop 3.6556.2's.
fn empty(_ui: &Ui) -> Element {
    Element::spacer()
}

fn transcript(session: &Session, ui: &Ui, column: f32, palette: ComponentPalette) -> Element {
    // Tool calls in a row fold into one group, as Desktop shows a turn's work;
    // an answered permission between them is gone, a waiting one ends the group.
    // Desktop opens a session with where it started.
    let mut items: Vec<Element> = vec![muted(format!(
        "Started on this computer · {}",
        folder_name(&session.cwd)
    ))
    .font_size(12.0)];
    let mut steps: Vec<Step> = vec![];
    for item in &session.items {
        match item {
            Item::Tool {
                call,
                name,
                input,
                result,
            } => steps.push(Step {
                call,
                name,
                input,
                result: result.as_ref(),
            }),
            Item::Permission {
                decided: Some(_), ..
            } => {}
            other => {
                if !steps.is_empty() {
                    items.push(step_group(&steps, ui, column, palette));
                    steps.clear();
                }
                // Desktop sets each message apart: about 60 px between turns and
                // 39 px under the person's words; the gap adds to the column's own 14.
                let user = matches!(other, Item::User { .. });
                if user && items.len() > 1 {
                    items.push(Element::spacer().size(None, Some(32.0)));
                }
                let element = self::item(other, ui, column, palette);
                // Under a reply, its actions show while the pointer is on it.
                let element = match other {
                    Item::Assistant {
                        streaming: false, ..
                    } => {
                        let at = session
                            .items
                            .iter()
                            .position(|i| std::ptr::eq(i, other))
                            .unwrap_or_default();
                        let copy = format!("copy:{}:{at}", session.id);
                        Element::column(vec![
                            element,
                            tool_surface(
                                &copy,
                                limn::Icon::Copy,
                                "Copy",
                                24.0,
                                None,
                                look::TOOL_HOVER,
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
    if !steps.is_empty() {
        items.push(step_group(&steps, ui, column, palette));
    }
    if let Some(status) = status_line(&session.status, palette) {
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
        hover: look::TOOL_HOVER,
        ..palette
    }
    .interactive_surface(Element::icon_button(
        SCROLL_END,
        limn::Icon::ArrowDown,
        "Scroll to bottom",
    ))
    .foreground(look::PALETTE.text)
    .background(palette.surface)
    .border(look::CARD_BORDER, 1.0)
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

fn status_line(status: &SessionStatus, palette: ComponentPalette) -> Option<Element> {
    Some(match status {
        SessionStatus::Starting => TextShimmer::new("status", "Starting…", palette).render(),
        SessionStatus::Running => TextShimmer::new("status", "Working…", palette).render(),
        SessionStatus::Awaiting => Element::row(vec![
            Element::spacer()
                .background(look::AWAITING)
                .radius(4.0)
                .size(Some(8.0), Some(8.0))
                .centered_y(),
            muted("Needs your input"),
        ])
        .gap(8.0),
        SessionStatus::Failed(error) => text(error.clone()).foreground(look::REMOVED),
        SessionStatus::Idle | SessionStatus::Exited => return None,
    })
}

fn item(item: &Item, ui: &Ui, column: f32, palette: ComponentPalette) -> Element {
    match item {
        Item::User { text: t, .. } => {
            // As wide as its words, up to 3/4 of the column, 16 px of side padding.
            let line = text(t.clone())
                .font_size(CHAT)
                .max_width(column * 0.75 - 32.0);
            let bubble = Element::panel(Element::row(vec![hgap(4.0), line, hgap(4.0)]))
                .background(look::USER_BUBBLE)
                .radius(14.0)
                .padding(12.0);
            Element::row(vec![Element::spacer().flex(1.0), bubble])
        }
        Item::Assistant { text: t, streaming } => {
            // Desktop's code: blocks in a white bordered box as wide as their
            // code, no language line, a copy button, 13 px; inline code on a light fill.
            let code_palette = ComponentPalette {
                raised: look::PALETTE.surface,
                border: look::DIVIDER,
                ..palette
            };
            let colors = limn::MarkdownColors {
                code_ink: look::PALETTE.text,
                code_fill: look::COMMAND_BOX,
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
                .streaming(*streaming)
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
        Item::Permission { .. } => permission(item, ui, column, palette),
        Item::TurnEnd {
            ok: false,
            interrupted: true,
            ..
        } => muted("Interrupted"),
        Item::TurnEnd {
            ok: false, error, ..
        } => text(error.clone().unwrap_or_else(|| "The turn failed".into()))
            .foreground(look::REMOVED),
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

/// A step's verb and what it acted on: ("Edited", "saludo.py"); a command keeps its own
/// words, past or present.
fn step_label(step: &Step, done: bool) -> (String, String) {
    match step.name {
        "Bash" => {
            let said = step.input["description"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    format!("Run {}", step.input["command"].as_str().unwrap_or_default())
                });
            let phrase = if done { past_tense(&said) } else { said };
            (phrase, String::new())
        }
        "Read" => ("Read".into(), file_of(step.input)),
        "Edit" | "MultiEdit" | "NotebookEdit" => ("Edited".into(), file_of(step.input)),
        "Write" => ("Wrote".into(), file_of(step.input)),
        "Grep" | "Glob" => (
            "Searched".into(),
            step.input["pattern"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        ),
        other => (other.to_owned(), String::new()),
    }
}

/// The line a group of steps folds into, as Desktop words it: one command says
/// what it did ("Ran saludo.py"); otherwise each file with what was done to it and how many
/// commands ran: "Read and edited saludo.py, ran a command".
pub fn group_summary(steps: &[Step]) -> String {
    let running = steps.iter().any(|s| s.result.is_none());
    if let [step] = steps {
        if step.name == "Bash" {
            let said = step.input["description"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| "Run a command".into());
            return if running {
                progressive(&said)
            } else if step.result.is_some_and(|r| r.stopped) {
                said
            } else {
                past_tense(&said)
            };
        }
    }
    let mut files: Vec<(String, Vec<String>)> = vec![];
    let (mut commands, mut searches, mut others) = (0, 0, vec![]);
    for step in steps {
        match step.name {
            "Bash" => commands += 1,
            "Grep" | "Glob" => searches += 1,
            "Read" | "Edit" | "MultiEdit" | "NotebookEdit" | "Write" => {
                let (verb, file) = step_label(step, true);
                let verb = verb.to_lowercase();
                match files.iter_mut().find(|(f, _)| *f == file) {
                    Some((_, verbs)) if !verbs.contains(&verb) => verbs.push(verb),
                    Some(_) => {}
                    None => files.push((file, vec![verb])),
                }
            }
            other => others.push(other.to_owned()),
        }
    }
    let mut clauses: Vec<String> = files
        .into_iter()
        .map(|(file, verbs)| format!("{} {file}", verbs.join(" and ")))
        .collect();
    if searches > 0 {
        clauses.push("searched the code".into());
    }
    match commands {
        0 => {}
        1 => clauses.push("ran a command".into()),
        n => clauses.push(format!("ran {n} commands")),
    }
    clauses.extend(others);
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
    let running = steps.iter().any(|s| s.result.is_none());
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
            .centered_y()
    } else {
        muted(summary).font_size(CHAT).centered_y()
    }];
    if stopped {
        head.push(muted("Stopped").font_size(CHAT).centered_y());
    }
    if added + removed > 0 {
        head.push(
            text(format!("+{added}"))
                .font_size(CHAT)
                .foreground(look::ADDED)
                .centered_y(),
        );
        head.push(
            text(format!("−{removed}"))
                .font_size(CHAT)
                .foreground(look::REMOVED)
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
            rows.push(hline(column, look::CARD_BORDER));
        }
        rows.push(step_row(step, ui, column, palette));
    }
    let card = Element::panel(Element::column(rows))
        .background(palette.surface)
        .border(look::CARD_BORDER, 1.0)
        .radius(8.0)
        .size(Some(column), None);
    Element::column(vec![head, card]).gap(8.0)
}

/// The small triangle after a group or a step: right while folded, down while open.
fn caret(open: bool) -> Element {
    text(if open { "▾" } else { "▸" })
        .font_size(10.0)
        .foreground(look::MUTED)
        .centered_y()
}

/// One step in its group's card: "Read saludo.py", "Edited saludo.py +1 −1",
/// "Ran saludo.py to verify output", each opening what it did.
fn step_row(step: &Step, ui: &Ui, column: f32, palette: ComponentPalette) -> Element {
    let open = ui.expanded.iter().any(|c| c == step.call);
    let done = step.result.is_some_and(|r| !r.stopped);
    let (verb, object) = step_label(step, done);
    let mut line = vec![hgap(10.0), muted(verb).centered_y()];
    if !object.is_empty() {
        line.push(text(object).centered_y());
    }
    if let Some(r) = step.result {
        if r.added + r.removed > 0 {
            line.push(
                text(format!("+{}", r.added))
                    .foreground(look::ADDED)
                    .centered_y(),
            );
            line.push(
                text(format!("−{}", r.removed))
                    .foreground(look::REMOVED)
                    .centered_y(),
            );
        }
        if r.stopped {
            line.push(muted("Stopped").centered_y());
        }
    }
    line.push(caret(open));
    let row_palette = ComponentPalette {
        hover: look::ROW_HOVER,
        ..palette
    };
    let row = row_palette.interactive_surface(Element::clickable(
        format!("tool:{}", step.call),
        Element::row(line).gap(6.0).size(Some(column), Some(31.0)),
    ));
    if !open {
        return row;
    }
    let inner = column - 24.0;
    let mut body = vec![];
    let result = step.result;
    match step.name {
        "Bash" => {
            body.push(muted("Bash").font_size(SMALL));
            let command = step.input["command"].as_str().unwrap_or_default();
            body.push(
                Element::panel(
                    Element::row(vec![
                        mono("$").foreground(look::MUTED),
                        mono(command).size(Some(inner - 40.0), None),
                    ])
                    .gap(8.0),
                )
                .background(palette.surface)
                .border(look::CARD_BORDER, 1.0)
                .radius(6.0)
                .padding(10.0)
                .size(Some(inner), None),
            );
            if let Some(r) = result.filter(|r| !r.output.trim().is_empty()) {
                body.push(
                    mono(r.output.trim_end())
                        .foreground(look::MUTED)
                        .size(Some(inner), None),
                );
            }
        }
        _ => {
            let files = result.map(|r| r.files.as_slice()).unwrap_or_default();
            for file in files {
                body.push(muted(file.path.display().to_string()).font_size(SMALL));
                let lines = line_diff(
                    file.before.as_deref().unwrap_or_default(),
                    file.after.as_deref().unwrap_or_default(),
                );
                let rows = lines.len().max(1) as f32;
                let words = word_ranges(&lines);
                body.push(
                    Element::panel(
                        limn::CodeDiff::new(
                            format!("diff:{}:{}", step.call, file.path.display()),
                            lines,
                        )
                        .word_ranges(words)
                        .palette(palette)
                        .font_size(12.0)
                        .height((rows * 19.0 + 16.0).min(480.0))
                        .render(),
                    )
                    .border(look::CARD_BORDER, 1.0)
                    .radius(6.0)
                    .size(Some(inner), None),
                );
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
    Element::column(vec![row, pad(Element::column(body).gap(8.0), 12.0, 4.0)])
}

fn mono(s: impl Into<String>) -> Element {
    Element::text(s.into())
        .font_family(FontFamily::Monospace)
        .font_size(SMALL)
        .foreground(look::PALETTE.text)
}

/// The inline approval card, as Desktop 3.6556.2 draws it: a 480 px card with what
/// Claude wants to run, Deny, and the way the person allowed last as its main button with
/// the other in its menu. Once answered it goes, leaving the tool's row.
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
    let heading = match (tool.as_str(), what) {
        ("Bash", Some(what)) => format!("Allow Claude to run {what}?"),
        _ => format!("Allow Claude to use {tool}?"),
    };
    let mut rows = vec![wrapped(text(heading).font_family(FontFamily::SansBold))];
    if let Some(what) = what {
        rows.push(wrapped(muted(what.to_owned())));
    }
    if let Some(title) = title {
        rows.push(wrapped(muted(title.clone())));
    }
    let shown = command
        .or(input["file_path"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| {
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
                .foreground(look::COMMAND_INK)
                .size(Some(inner - 32.0), None),
        )
        .background(look::COMMAND_BOX)
        .radius(6.0)
        .padding(14.0),
    );
    if *interactive {
        // Questions get their own card (plan Q3.7); until then they are allowed as asked.
        rows.push(Button::new(format!("perm:{id}:once"), "Continue", palette).render());
    } else {
        rows.push(permission_actions(id, !suggestions.is_empty(), ui, palette));
    }
    Element::panel(Element::column(rows).gap(10.0))
        .background(palette.surface)
        .border(look::CARD_BORDER, 1.0)
        .radius(12.0)
        .padding(16.0)
        .size(Some(width), None)
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
            raised: look::CANCEL,
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

/// Desktop's composer at 1×: a 44 px box with "+" on the left and send (or stop)
/// on the right, growing upward as the message does, over a row of chips outside it: the
/// folder and mode on the left, the model and effort on the right.
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
        composer_button(STOP, limn::Icon::StopFill, "Stop", false)
    } else if !ui.prompt.trim().is_empty() {
        send_button()
    } else {
        Element::spacer().size(Some(34.0), Some(34.0))
    };
    let placeholder = if session.is_some() {
        "Type / for commands"
    } else {
        "Describe a task or ask a question"
    };
    Composer::new(
        COMPOSER,
        ui.prompt.clone(),
        placeholder,
        column,
        look::COMPOSER_PALETTE,
    )
    .enter_submits(true)
    .auto_grow(28.0, 184.0)
    .editor_font_size(15.0)
    .pill_controls(plus_menu(ui), trailing)
    .inline_controls(true)
    .controls_at_last_line(true)
    .border_color(look::COMPOSER_BORDER)
    .shadow_color(look::COMPOSER_SHADOW)
    .card_radius(14.0)
    .context(composer_footer(store, session, ui, palette))
    .render()
}

/// A round-cornered 34 px icon button inside the composer, gray under the pointer and while
/// its menu is open.
fn composer_button(id: &str, icon: limn::Icon, label: &str, open: bool) -> Element {
    let palette = ComponentPalette {
        hover: look::COMPOSER_HOVER,
        selected: look::COMPOSER_OPEN,
        ..look::PALETTE
    };
    let button = palette
        .interactive_surface(Element::icon_button(id, icon, label))
        .foreground(look::PALETTE.text)
        .radius(8.0)
        .size(Some(34.0), Some(34.0));
    let button = if open {
        button.background(look::COMPOSER_OPEN)
    } else {
        button
    };
    limn::Tooltip::new(button, id, label, look::PALETTE).render()
}

/// Send, drawn as Desktop's return arrow; limn has no such icon, so it is the character.
fn send_button() -> Element {
    let palette = ComponentPalette {
        hover: look::COMPOSER_HOVER,
        selected: look::COMPOSER_OPEN,
        ..look::PALETTE
    };
    let glyph = text("↵").font_size(18.0).center_content();
    let button = palette
        .interactive_surface(Element::clickable(SEND, glyph))
        .radius(8.0)
        .size(Some(34.0), Some(34.0));
    limn::Tooltip::new(button, SEND, "Send", look::PALETTE).render()
}

/// "+": what can go into the message, as far as quill can: files and folders
/// as @-mentions the agent reads, and the slash that opens its commands.
fn plus_menu(ui: &Ui) -> Element {
    let open = ui.open_menu == Some(PLUS_MENU);
    let anchor = composer_button(PLUS_MENU, limn::Icon::Add, "Add", open);
    let entries = vec![
        limn::DropdownMenuItem::new(ATTACH_FILES, "Add files or photos")
            .icon(limn::Icon::Paperclip)
            .shortcut("Ctrl+U")
            .into(),
        limn::DropdownMenuItem::new(ATTACH_FOLDER, "Add folder")
            .icon(limn::Icon::Folder)
            .into(),
        limn::DropdownMenuItem::new(SLASH, "Slash commands")
            .icon(limn::Icon::Hash)
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
        look::MENU_PALETTE,
    )
    .sizing(sizing)
    // The current row has only Desktop's blue check, no fill (limn #127).
    .check_ink(look::HANDLE)
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
        hover: look::CHIP_HOVER,
        selected: look::CHIP_OPEN,
        ..look::PALETTE
    };
    let mut items = vec![hgap(6.0)];
    items.extend(content);
    items.push(hgap(6.0));
    let row = Element::row(items).gap(6.0).size(None, Some(24.0));
    let chip = palette
        .interactive_surface(Element::clickable(id, row))
        .radius(6.0);
    let chip = if open {
        chip.background(look::CHIP_OPEN)
    } else {
        chip
    };
    let tip = limn::Tooltip::new(chip, id, tooltip, look::PALETTE);
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
        .foreground(look::PALETTE.text)
        .size(Some(14.0), Some(14.0))
        .centered_y()
}

/// The chips under the composer: a new session's place and folder, then the mode;
/// the model and its effort on the right.
fn composer_footer(
    store: &Store,
    session: Option<&Session>,
    ui: &Ui,
    palette: ComponentPalette,
) -> Element {
    let mut left = vec![];
    if session.is_none() {
        // quill runs sessions on this computer only, so the place is a label, not a menu.
        left.push(
            Element::row(vec![
                hgap(6.0),
                chip_icon(limn::Icon::Laptop),
                chip_text("This computer", look::PALETTE.text),
            ])
            .gap(6.0)
            .size(None, Some(24.0)),
        );
        left.push(folder_menu(store, ui));
    }
    left.push(mode_menu(session, ui));
    let mut row = left;
    row.push(Element::spacer().flex(1.0));
    row.push(model_menu(store, session, ui, palette));
    Element::row(row).gap(2.0).size(None, Some(28.0))
}

/// The folder a new session works in: the recent ones and "Open folder…".
fn folder_menu(store: &Store, ui: &Ui) -> Element {
    let open = ui.open_menu == Some(FOLDER_MENU);
    let anchor = chip(
        FOLDER_MENU,
        vec![
            chip_icon(limn::Icon::Folder),
            chip_text(folder_name(&ui.cwd), look::PALETTE.text),
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

/// The row a typed digit picks in the open mode or model menu, as Desktop's 1–4 do.
pub fn menu_digit(store: &Store, ui: &Ui, typed: &str) -> Option<String> {
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
        look::MANUAL
    } else {
        look::PALETTE.text
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
    let efforts = current.map(|m| m.efforts.as_slice()).unwrap_or_default();
    let effort = ui.effort.clone().or_else(|| efforts.first().cloned());
    let mut content = vec![chip_text(name, look::PALETTE.text)];
    if let Some(effort) = &effort {
        content.push(chip_text(effort_label(effort), look::MUTED));
    }
    let anchor = chip(MODEL_MENU, content, open, "Model", Some("Ctrl+Shift+I"));
    let mut entries = vec![limn::DropdownMenuEntry::Heading(
        "Model · this session".into(),
    )];
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
    if !efforts.is_empty() {
        entries.push(limn::DropdownMenuEntry::Separator);
        let levels = efforts
            .iter()
            .map(|e| {
                limn::DropdownMenuItem::new(format!("effort:{e}"), effort_label(e))
                    .selected(effort.as_ref() == Some(e))
                    .into()
            })
            .collect();
        let mut effort_item =
            limn::DropdownMenuItem::new(format!("{MODEL_MENU}:effort"), "Effort").submenu(levels);
        if let Some(effort) = &effort {
            effort_item = effort_item.annotation(effort_label(effort));
        }
        entries.push(effort_item.into());
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
        text(format!("+{added}")).foreground(look::ADDED),
        text(format!("−{removed}")).foreground(look::REMOVED),
    ])
    .gap(6.0);
    let mut card = FramedCard::new(palette).width(column).section(head);
    let rows = files
        .iter()
        .map(|f| {
            let (mark, color) = match &f.kind {
                ChangeKind::Added => ("A".to_string(), look::ADDED),
                ChangeKind::Deleted => ("D".to_string(), look::REMOVED),
                ChangeKind::Renamed { from } => (format!("R ← {from}"), look::MUTED),
                ChangeKind::Modified => ("M".to_string(), look::MUTED),
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
        .background(look::THEME.background)
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
            .foreground(look::MUTED)
            .centered_y(),
        text("‹›")
            .font_size(SMALL)
            .foreground(look::MUTED)
            .centered_y(),
        text(name).foreground(look::COMMAND_INK).centered_y(),
        Element::spacer().flex(1.0),
        text(format!("+{}", file.added))
            .foreground(look::ADDED)
            .centered_y(),
        text(format!("−{}", file.removed))
            .foreground(look::REMOVED)
            .centered_y(),
        hgap(16.0),
    ])
    .gap(6.0)
    .background(look::CANCEL)
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

/// A question the agent asks (AskUserQuestion): one block per question with its options,
/// and Submit once each has an answer, as Desktop's question card.
fn question_card(
    id: &str,
    input: &Value,
    ui: &Ui,
    column: f32,
    palette: ComponentPalette,
) -> Element {
    let questions = input["questions"].as_array().cloned().unwrap_or_default();
    let mut card = FramedCard::new(palette).width(column);
    for (q, question) in questions.iter().enumerate() {
        let chosen = ui.answers.get(&(id.to_owned(), q));
        let options = question["options"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(o, option)| {
                let label = option["label"].as_str().unwrap_or_default();
                let picked = chosen.is_some_and(|c| c == label);
                let mut button = Button::new(format!("ask:{id}:{q}:{o}"), label, palette)
                    .variant(if picked {
                        ButtonVariant::Default
                    } else {
                        ButtonVariant::Outline
                    })
                    .button_size(ButtonSize::Small);
                if let Some(description) = option["description"].as_str() {
                    button = button.tooltip(description);
                }
                button.render()
            })
            .collect();
        card = card.section(
            Element::column(vec![
                muted(question["header"].as_str().unwrap_or_default().to_owned()),
                text(question["question"].as_str().unwrap_or_default().to_owned()),
                Element::row(options).gap(6.0).wrap(),
            ])
            .gap(6.0),
        );
    }
    let ready = (0..questions.len()).all(|q| ui.answers.contains_key(&(id.to_owned(), q)));
    card.foot(
        ButtonGroup::new(vec![
            Button::new(format!("perm:{id}:deny"), "Skip", palette)
                .variant(ButtonVariant::Ghost)
                .button_size(ButtonSize::Small)
                .render(),
            Button::new(format!("ask:{id}:submit"), "Submit", palette)
                .button_size(ButtonSize::Small)
                .enabled(ready)
                .render(),
        ])
        .align_end()
        .render(),
    )
    .render()
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
        .background(look::PALETTE.surface)
        .border(look::CARD_BORDER, 1.0)
        .radius(12.0)
        .padding(18.0)
        .size(Some(width), None)
}

/// A row of a panel card: a 16 px icon and its words.
fn panel_row(icon: limn::Icon, words: Element) -> Element {
    Element::row(vec![
        Element::icon(icon)
            .foreground(look::PALETTE.text)
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
        panel_row(limn::Icon::Laptop, text("This computer")),
        panel_row(limn::Icon::Folder, text(folder_name(&session.cwd))),
    ];
    if let Some(branch) = &session.branch {
        environment.push(panel_row(limn::Icon::Tree, text(branch.clone())));
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
                Element::icon(limn::Icon::Code)
                    .foreground(look::PALETTE.text)
                    .size(Some(16.0), Some(16.0))
                    .centered_y(),
                text(format!("+{added}"))
                    .foreground(look::ADDED)
                    .font_size(SMALL)
                    .centered_y(),
                text(format!("−{removed}"))
                    .foreground(look::REMOVED)
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
        .background(look::HANDLE)
        .radius(3.0)
        .size(Some(fill), Some(6.0))])
    .background(look::ACTIVE)
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
