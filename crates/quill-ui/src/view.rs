//! The window: sidebar of sessions, transcript and composer (features/code-*.md, docs/ui.md §2).

use crate::look::{self, BODY, CHAT, COLUMN, SMALL};
use crate::{Item, Pane, Session, Store, ToolResult, Ui};
use limn::{
    Button, ButtonGroup, ButtonSize, ButtonVariant, Collapsible, Component, ComponentPalette,
    Composer, ControlSize, DropdownOption, Element, FontFamily, FramedCard, Markdown, OutputBlock,
    Select, Size, TextShimmer,
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
pub const TERMINAL_TOGGLE: &str = "terminal:toggle";
/// The terminal pane; while it has the focus the host sends keys to the shell.
pub const TERMINAL: &str = "terminal";
/// The terminal's scroll area, which follows new output while it is at the end.
pub const TERMINAL_SCROLL: &str = "terminal:scroll";
pub const TERMINAL_NEW: &str = "terminal:new";
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
    let column = match panes.first() {
        Some(Pane::Diff) => (rest * 0.6).round(),
        Some(Pane::Terminal) => (rest / 3.0).round(),
        None => 0.0,
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

/// The height each pane of the column gets: the space under the top bar, shared evenly.
fn pane_height(layout: &Layout, window: Size) -> f32 {
    let below = window.height - look::TOP_BAR - 1.0;
    let n = layout.panes.len().max(1) as f32;
    ((below - (n - 1.0)) / n).floor()
}

/// The files of the request the diff pane shows.
fn diff_files<'a>(store: &'a Store, ui: &Ui) -> Option<&'a Vec<FileEffect>> {
    let (id, request) = ui.diff.as_ref()?;
    let session = store.session(ui.selected?).filter(|s| s.id == *id)?;
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
    let inner = Size::new(
        layout.column - 2.0 * TERMINAL_PADDING[0],
        pane_height(&layout, window) - look::PANE_HEADER - 1.0 - 2.0 * TERMINAL_PADDING[1],
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
                composer(session, ui, column, palette).centered_x(),
                Element::spacer().size(None, Some(12.0)),
            ])
            .size(Some(layout.main), Some(below)),
        );
    }
    if !layout.panes.is_empty() {
        let height = pane_height(&layout, window);
        let mut stack = vec![];
        for (i, pane) in layout.panes.iter().enumerate() {
            if i > 0 {
                stack.push(hline(layout.column, look::PANE_BORDER));
            }
            stack.push(match pane {
                Pane::Diff => diff_pane(store, ui, layout.column, height, palette),
                Pane::Terminal => terminal_pane(session, ui, layout.column, height, palette),
            });
        }
        if layout.main > 0.0 {
            row.push(vline(below));
        }
        row.push(Element::column(stack).size(Some(layout.column), Some(below)));
    }
    if let (Some(session), true) = (session, layout.panel > 0.0) {
        row.push(right_panel(session, layout.panel, below, palette));
    }
    let content = Element::column(vec![
        top_bar(session, ui, palette),
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
    let closing = ui.terminal.closing.and_then(|i| ui.terminal.tabs.get(i));
    limn::AlertDialog::new(
        root,
        closing.is_some(),
        "Close terminal?",
        closing.map_or(String::new(), |(name, _)| {
            format!("A command is still running in {name}. Closing the terminal stops it.")
        }),
        TERMINAL_CLOSE_CONFIRM,
        TERMINAL_CLOSE_CANCEL,
        // Cancel is filled with `raised`: Desktop's light gray.
        ComponentPalette {
            raised: look::CANCEL,
            ..palette
        },
    )
    .width(400.0)
    .confirm_label("Close terminal")
    .soft_destructive(look::DANGER_FILL, look::DANGER_BORDER, look::DANGER_INK)
    .cancel_variant(ButtonVariant::Secondary)
    .description_size(BODY)
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
    let lead = if tabs.len() <= 1 {
        let folder = session.map_or(&ui.cwd, |s| &s.cwd);
        Element::row(vec![
            text("Terminal").centered_y(),
            muted(folder_name(folder)).centered_y(),
            Element::spacer().size(Some(4.0), None),
            new,
        ])
        .gap(8.0)
    } else {
        let mut pills: Vec<Element> = tabs
            .iter()
            .enumerate()
            .map(|(i, (title, _))| {
                let pill = Element::row(vec![
                    hgap(8.0),
                    text(title.clone()).centered_y(),
                    icon_tool(
                        &format!("terminal:close:{i}"),
                        limn::Icon::Close,
                        "Close terminal",
                        20.0,
                        palette,
                    ),
                ])
                .gap(4.0)
                .size(None, Some(28.0))
                .radius(8.0);
                let pill = if i == ui.terminal.active {
                    pill.background(look::ACTIVE)
                } else {
                    pill
                };
                Element::clickable(format!("terminal:tab:{i}"), pill).centered_y()
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

/// A square icon button with the palette's hover and press, as limn's own tools draw them.
fn icon_tool(
    id: &str,
    icon: limn::Icon,
    label: &str,
    side: f32,
    palette: ComponentPalette,
) -> Element {
    palette
        .interactive_surface(Element::icon_button(id, icon, label))
        .foreground(palette.text)
        .size(Some(side), Some(side))
        .centered_y()
}

/// A top-bar toggle: its icon on the active fill while what it shows is open.
fn toggle(id: &str, icon: limn::Icon, label: &str, on: bool, palette: ComponentPalette) -> Element {
    let tool = icon_tool(id, icon, label, 26.0, palette);
    if on {
        tool.background(look::ACTIVE).radius(6.0)
    } else {
        tool
    }
}

/// The session's title, Stop while it works, and the toggles of the panel and the panes
///.
fn top_bar(session: Option<&Session>, ui: &Ui, palette: ComponentPalette) -> Element {
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
    if session.is_some_and(|s| matches!(s.status, SessionStatus::Running | SessionStatus::Awaiting))
    {
        row.push(
            Button::new(STOP, "Stop", palette)
                .variant(ButtonVariant::Ghost)
                .button_size(ButtonSize::Small)
                .render()
                .centered_y(),
        );
    }
    if session.is_some() {
        row.push(toggle(
            PANEL_TOGGLE,
            limn::Icon::PanelRight,
            "Toggle panel",
            ui.panel_open && ui.panes.is_empty(),
            palette,
        ));
    }
    row.push(toggle(
        TERMINAL_TOGGLE,
        limn::Icon::Terminal,
        if ui.pane_open(Pane::Terminal) {
            "Hide terminal"
        } else {
            "Show terminal"
        },
        ui.pane_open(Pane::Terminal),
        palette,
    ));
    if session.is_some() {
        row.push(toggle(
            DIFF_TOGGLE,
            limn::Icon::Code,
            "Toggle diff",
            ui.pane_open(Pane::Diff),
            palette,
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
        let header = Element::row(vec![
            hgap(10.0),
            muted(folder_name(folder)).font_size(SMALL).centered_y(),
            caret,
        ])
        .gap(6.0)
        .size(None, Some(28.0));
        rows.push(Element::clickable(
            format!("group:{}", folder.display()),
            header,
        ));
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
    let row = palette
        .interactive_surface(Element::clickable(id, row))
        .radius(8.0);
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

fn empty(ui: &Ui) -> Element {
    Element::column(vec![
        text("What's up next?").font_size(22.0),
        muted(format!(
            "A new session runs Claude Code in {}",
            folder_name(&ui.cwd)
        )),
    ])
    .gap(8.0)
    .centered()
}

fn transcript(session: &Session, ui: &Ui, column: f32, palette: ComponentPalette) -> Element {
    let mut items: Vec<Element> = session
        .items
        .iter()
        .map(|item| self::item(item, ui, column, palette))
        .collect();
    if let Some(status) = status_line(&session.status, palette) {
        items.push(status);
    }
    let content = Element::column(items)
        .gap(14.0)
        .size(Some(column), None)
        .padding(16.0)
        .centered_x();
    Element::scroll(format!("transcript:{}", session.id), content)
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
            // Up to 3/4 of the column, 16 px of side padding.
            let line = text(t.clone()).font_size(CHAT);
            let bubble = Element::panel(pad(line, 4.0, 0.0).size(Some(column * 0.75), None))
                .background(look::USER_BUBBLE)
                .radius(14.0)
                .padding(12.0);
            Element::row(vec![Element::spacer().flex(1.0), bubble])
        }
        Item::Assistant { text: t, streaming } => {
            Markdown::new(t, palette)
                .width(column)
                .font_size(CHAT)
                .streaming(*streaming)
                .render()
                .element
        }
        Item::Tool {
            call,
            name,
            input,
            result,
        } => tool(call, name, input, result.as_ref(), ui, column, palette),
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

/// What a tool's header says: verb and object, as Desktop's rows ("Ran", "Edited").
fn tool_title(name: &str, input: &Value) -> String {
    let file = || {
        input["file_path"]
            .as_str()
            .map(|p| p.rsplit('/').next().unwrap_or(p).to_owned())
            .unwrap_or_default()
    };
    match name {
        "Bash" => format!("Ran {}", input["command"].as_str().unwrap_or_default()),
        "Edit" | "MultiEdit" => format!("Edited {}", file()),
        "Write" => format!("Wrote {}", file()),
        "Read" => format!("Read {}", file()),
        "Grep" | "Glob" => format!("Searched {}", input["pattern"].as_str().unwrap_or_default()),
        other => other.to_owned(),
    }
}

fn tool(
    call: &str,
    name: &str,
    input: &Value,
    result: Option<&ToolResult>,
    ui: &Ui,
    column: f32,
    palette: ComponentPalette,
) -> Element {
    let open = ui.expanded.iter().any(|c| c == call);
    let content = match result {
        Some(r) if !r.output.is_empty() => {
            OutputBlock::new(format!("out:{call}"), r.output.clone(), palette).render()
        }
        _ => muted("No output"),
    };
    let mut row = Collapsible::new(
        format!("tool:{call}"),
        tool_title(name, input),
        content,
        palette,
    )
    .expanded(open)
    .loading(result.is_none());
    if let Some(r) = result {
        if r.added + r.removed > 0 {
            row = row.detail(format!("+{} −{}", r.added, r.removed));
        }
        if r.is_error {
            row = row.ink(look::REMOVED);
        }
    }
    row.render().size(Some(column), None)
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

fn composer(session: Option<&Session>, ui: &Ui, column: f32, palette: ComponentPalette) -> Element {
    let folder = session.map_or(&ui.cwd, |s| &s.cwd);
    let folder = folder_name(folder);
    let mode = session.map_or(ui.mode.as_str(), |s| s.mode.as_str());
    let modes = ["default", "acceptEdits", "plan"]
        .into_iter()
        .map(|m| DropdownOption::new(format!("mode:{m}"), mode_label(m)).selected(m == mode))
        .collect();
    let models: Vec<DropdownOption> = session
        .and_then(|s| s.capabilities.as_ref())
        .map(|c| {
            c.models
                .iter()
                .map(|m| {
                    DropdownOption::new(format!("model:{}", m.id), m.name.clone())
                        .selected(ui.model.as_deref() == Some(m.id.as_str()))
                })
                .collect()
        })
        .unwrap_or_default();
    let model_label = ui.model.clone().unwrap_or_else(|| "Default model".into());
    // Only a new session chooses its folder; a session's footer is mode and model.
    let mut footer = vec![];
    if session.is_none() {
        footer.push(
            Button::new(PICK_FOLDER, format!("▣ {folder}"), palette)
                .variant(ButtonVariant::Ghost)
                .button_size(ButtonSize::Small)
                .tooltip("The folder a new session works in")
                .render(),
        );
    }
    footer.extend([
        Select::new(
            MODE_MENU,
            mode_label(mode),
            modes,
            ui.open_menu == Some(MODE_MENU),
            format!("{MODE_MENU}:dismiss"),
            palette,
        )
        .size(ControlSize::Compact)
        .width(120.0)
        .render(),
        Element::spacer().flex(1.0),
        Select::new(
            MODEL_MENU,
            model_label,
            models,
            ui.open_menu == Some(MODEL_MENU),
            format!("{MODEL_MENU}:dismiss"),
            palette,
        )
        .size(ControlSize::Compact)
        .width(150.0)
        .render(),
    ]);
    let footer = Element::row(footer).gap(8.0);
    let placeholder = if session.is_some() {
        "Reply…"
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
    .auto_grow(44.0, 200.0)
    .footer(footer)
    .render()
}

fn mode_label(mode: &str) -> &'static str {
    match mode {
        "acceptEdits" => "Accept edits",
        "plan" => "Plan",
        _ => "Manual",
    }
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
    let header = pane_header(
        text("Changes").centered_y(),
        pane_tools(Pane::Diff, ui, palette),
        width,
    );
    let mut blocks = vec![];
    for (index, file) in files.iter().enumerate() {
        blocks.push(
            limn::DiffFileHeader::new(format!("diff:file:{index}"), file.path.clone(), palette)
                .stat(file.added as usize, file.removed as usize)
                .render(),
        );
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
        Element::column(blocks)
            .gap(4.0)
            .padding(8.0)
            .size(Some(width), None),
    );
    Element::column(vec![header, body.flex(1.0)])
        .background(look::THEME.background)
        .size(Some(width), Some(height))
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

/// One titled card of the right panel.
fn panel_card(title: &str, rows: Vec<Element>, width: f32, palette: ComponentPalette) -> Element {
    let mut content = vec![muted(title.to_owned())];
    content.extend(rows);
    Element::panel(Element::column(content).gap(6.0))
        .background(palette.surface)
        .border(look::DIVIDER, 1.0)
        .radius(8.0)
        .padding(10.0)
        .size(Some(width), None)
}

/// Where the session works, how full its context is and what it changed.
fn right_panel(session: &Session, width: f32, height: f32, palette: ComponentPalette) -> Element {
    let inner = width - 16.0;
    let mut environment = vec![
        text("This computer"),
        text(format!("▣ {}", folder_name(&session.cwd))),
    ];
    if let Some(branch) = &session.branch {
        environment.push(text(format!("⑂ {branch}")));
    }
    let context = if session.context_tokens == 0 {
        muted("No turns yet")
    } else {
        text(format!(
            "{:.1}k tokens",
            session.context_tokens as f64 / 1000.0
        ))
    };
    let mut context_rows = vec![context];
    if session.cost > 0.0 {
        context_rows.push(muted(format!("${:.4} this session", session.cost)));
    }
    let (added, removed, files) =
        session
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
            });
    let changes = if files == 0 {
        muted("Nothing changed yet")
    } else {
        Element::row(vec![
            text(if files == 1 {
                "1 file".to_string()
            } else {
                format!("{files} files")
            }),
            text(format!("+{added}")).foreground(look::ADDED),
            text(format!("−{removed}")).foreground(look::REMOVED),
        ])
        .gap(6.0)
    };
    Element::column(vec![
        panel_card("Environment", environment, inner, palette),
        panel_card("Context", context_rows, inner, palette),
        panel_card("Changes", vec![changes], inner, palette),
    ])
    .gap(8.0)
    .padding(8.0)
    .size(Some(width), Some(height))
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
