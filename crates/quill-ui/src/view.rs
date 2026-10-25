//! The window: sidebar of sessions, transcript and composer (features/code-*.md, docs/ui.md §2).

use crate::look::{self, BODY, COLUMN};
use crate::{Item, Session, Store, ToolResult, Ui};
use limn::{
    Button, ButtonGroup, ButtonSize, ButtonVariant, Collapsible, Component, ComponentPalette,
    Composer, ControlSize, DropdownOption, Element, FontFamily, FramedCard, ListRow, Markdown,
    OutputBlock, Select, Size, TextShimmer,
};
use quill_model::{ChangeKind, FileEffect, SessionStatus};
use serde_json::Value;

pub const COMPOSER: &str = "composer";
pub const NEW_SESSION: &str = "new-session";
pub const STOP: &str = "stop";
pub const MODE_MENU: &str = "mode-menu";
pub const MODEL_MENU: &str = "model-menu";
pub const DIFF_CLOSE: &str = "diff:close";
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
const TERMINAL_HEIGHT: f32 = 272.0;
const TERMINAL_TAB_WIDTH: f32 = 140.0;
const TERMINAL_FONT: f32 = 12.0;
const TERMINAL_PADDING: f32 = 8.0;

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

/// The widths of the diff panel, the right panel and the main column, in that order.
fn widths(store: &Store, ui: &Ui, window: Size) -> (f32, f32, f32) {
    let session = ui.selected.and_then(|id| store.session(id));
    let rest = (window.width - look::SIDEBAR_WIDTH).max(0.0);
    let diff_width = if diff_files(store, ui).is_some() {
        (rest * 0.6).min(760.0)
    } else {
        0.0
    };
    // The right panel stays only while it fits beside a full column.
    let panel_width = if diff_width == 0.0 && session.is_some() && rest >= 1100.0 {
        look::PANEL_WIDTH
    } else {
        0.0
    };
    (diff_width, panel_width, rest - diff_width - panel_width)
}

/// The files of the request the diff panel shows.
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
    let (_, _, main_width) = widths(store, ui, window);
    let inner = Size::new(
        main_width - 2.0 * TERMINAL_PADDING,
        TERMINAL_HEIGHT - limn::TabStrip::HEADER_HEIGHT - 2.0 * TERMINAL_PADDING,
    );
    let (cols, rows) = limn::TerminalView::grid_size(inner, TERMINAL_FONT);
    (cols as u16, rows as u16)
}

pub fn view(store: &Store, ui: &Ui, window: Size) -> Element {
    let palette = look::PALETTE;
    let session = ui.selected.and_then(|id| store.session(id));
    // The diff opens beside the conversation, taking 3 of 5 parts as Desktop's does.
    let files = diff_files(store, ui);
    let (diff_width, panel_width, main_width) = widths(store, ui, window);
    let column = COLUMN.min(main_width - 32.0).max(240.0);
    let body = match session {
        Some(session) => transcript(session, ui, column, palette),
        None => empty(ui),
    };
    let mut main = vec![
        top_bar(session, ui, palette),
        body.flex(1.0),
        composer(session, ui, column, palette).centered_x(),
        Element::spacer().size(None, Some(12.0)),
    ];
    // The session's shell sits under the conversation (features/terminal.md).
    if let (true, Some(screen)) = (ui.terminal.open, &ui.terminal.screen) {
        main.push(terminal_tabs(ui, main_width, palette));
        main.push(
            limn::TerminalView::new(TERMINAL, screen.clone(), palette)
                .font_size(TERMINAL_FONT)
                .background(look::THEME.background)
                .render()
                .padding(TERMINAL_PADDING)
                .size(
                    Some(main_width),
                    Some(TERMINAL_HEIGHT - limn::TabStrip::HEADER_HEIGHT),
                ),
        );
    }
    let main = Element::column(main).size(Some(main_width), Some(window.height));
    let mut row = vec![
        sidebar(store, ui, window.height, palette),
        Element::spacer()
            .background(look::DIVIDER)
            .size(Some(1.0), Some(window.height)),
        main,
    ];
    if let Some(files) = files {
        row.push(
            Element::spacer()
                .background(look::DIVIDER)
                .size(Some(1.0), Some(window.height)),
        );
        row.push(diff_panel(files, diff_width - 1.0, window.height, palette));
    }
    if let (Some(session), true) = (session, panel_width > 0.0) {
        row.push(right_panel(session, panel_width, window.height, palette));
    }
    let root = Element::row(row)
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
        palette,
    )
    .confirm_label("Close terminal")
    .render()
}

/// The pane's tabs, each with its close button, and "New terminal".
fn terminal_tabs(ui: &Ui, width: f32, palette: ComponentPalette) -> Element {
    let tabs = ui
        .terminal
        .tabs
        .iter()
        .enumerate()
        .map(|(i, (title, _))| {
            limn::TabItem::new(
                format!("terminal:tab:{i}"),
                format!("terminal:extent:{i}"),
                limn::Icon::Terminal,
                title,
                TERMINAL_TAB_WIDTH,
            )
            .active(i == ui.terminal.active)
            .close(icon_tool(
                &format!("terminal:close:{i}"),
                limn::Icon::Close,
                "Close terminal",
                limn::TabStrip::CLOSE_SIZE,
                palette,
            ))
        })
        .collect();
    let strip = limn::TabStrip::new("terminal:tabs", tabs, palette).render();
    let new = icon_tool(
        TERMINAL_NEW,
        limn::Icon::Add,
        "New terminal",
        limn::TabStrip::TOOL_SIZE,
        palette,
    );
    limn::TabStrip::header(strip, new, width, palette).render()
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
        .foreground(palette.muted)
        .size(Some(side), Some(side))
}

fn top_bar(session: Option<&Session>, ui: &Ui, palette: ComponentPalette) -> Element {
    let title = session.map_or("New session".to_string(), |s| {
        if s.title.is_empty() {
            "New session".into()
        } else {
            s.title.clone()
        }
    });
    let mut row = vec![muted(title), Element::spacer().flex(1.0)];
    if session.is_some_and(|s| matches!(s.status, SessionStatus::Running | SessionStatus::Awaiting))
    {
        row.push(
            Button::new(STOP, "Stop", palette)
                .variant(ButtonVariant::Ghost)
                .button_size(ButtonSize::Small)
                .render(),
        );
    }
    let label = if ui.terminal.open {
        "Hide terminal"
    } else {
        "Show terminal"
    };
    row.push(
        Button::new(TERMINAL_TOGGLE, label, palette)
            .variant(ButtonVariant::Ghost)
            .button_size(ButtonSize::Small)
            .render(),
    );
    Element::row(row)
        .gap(4.0)
        .padding(12.0)
        .size(None, Some(44.0))
}

fn sidebar(store: &Store, ui: &Ui, height: f32, palette: ComponentPalette) -> Element {
    let mut rows = vec![Button::new(NEW_SESSION, "New session", palette)
        .variant(ButtonVariant::Subtle)
        .render()];
    if store.sessions.is_empty() {
        rows.push(muted("Sessions you start will show up here.").padding(8.0));
    }
    for session in store.sessions.iter().rev() {
        let title = if session.title.is_empty() {
            "New session".to_string()
        } else {
            session.title.clone()
        };
        let mut row = ListRow::new(format!("session:{}", session.id), title, palette)
            .subtitle(
                session
                    .cwd
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
            .trailing(status_dot(&session.status));
        if ui.selected == Some(session.id) {
            row = row.filled();
        }
        rows.push(row.render());
    }
    Element::column(rows)
        .gap(2.0)
        .padding(8.0)
        .background(look::SIDEBAR)
        .size(Some(look::SIDEBAR_WIDTH - 1.0), Some(height))
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
            let bubble =
                Element::panel(text(t.clone()).size(Some((column * 0.8).min(460.0)), None))
                    .background(look::USER_BUBBLE)
                    .radius(12.0)
                    .padding(10.0);
            Element::row(vec![Element::spacer().flex(1.0), bubble])
        }
        Item::Assistant { text: t, streaming } => {
            Markdown::new(t, palette)
                .width(column)
                .font_size(BODY)
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

/// The inline approval card: question, what it runs, Deny / Allow once / Always allow.
fn permission(item: &Item, ui: &Ui, column: f32, palette: ComponentPalette) -> Element {
    let Item::Permission {
        id,
        tool,
        input,
        interactive,
        decided,
        ..
    } = item
    else {
        return Element::spacer();
    };
    let (interactive, decided) = (*interactive, *decided);
    let menu_open = ui.perm_menu.as_deref() == Some(id.as_str());
    let what = input["command"]
        .as_str()
        .or(input["file_path"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            serde_json::to_string(input)
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect()
        });
    let mut card = FramedCard::new(palette)
        .width(column)
        .section(text(format!("Allow Claude to use {tool}?")).font_size(14.0))
        .section(
            Element::panel(text(what).font_family(FontFamily::Monospace))
                .background(look::COMMAND_BOX)
                .radius(6.0)
                .padding(8.0),
        );
    if let Some(decided) = decided {
        card = card.foot(muted(decided));
    } else if interactive {
        // Questions get their own card (plan Q3.7); until then they are allowed as asked.
        card = card.foot(Button::new(format!("perm:{id}:once"), "Continue", palette).render());
    } else {
        // Deny, then "Always allow" with "Allow once" in its menu.
        let once = limn::DropdownMenuItem::new(format!("perm:{id}:once"), "Allow once")
            .shortcut("Ctrl+Enter");
        let actions = ButtonGroup::new(vec![
            Button::new(format!("perm:{id}:deny"), "Deny", palette)
                .variant(ButtonVariant::Ghost)
                .button_size(ButtonSize::Small)
                .render(),
            limn::SplitButton::new(
                format!("perm:{id}:always"),
                "Always allow",
                format!("perm:{id}:menu"),
                palette,
            )
            .keys(&["Ctrl", "Shift", "⏎"])
            .menu(
                vec![once.into()],
                menu_open,
                format!("perm:{id}:menu-dismiss"),
            )
            .render(),
        ])
        .align_end()
        .render();
        card = card.foot(actions);
    }
    card.render()
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
    let footer = Element::row(vec![
        Button::new(PICK_FOLDER, format!("▣ {folder}"), palette)
            .variant(ButtonVariant::Ghost)
            .button_size(ButtonSize::Small)
            .enabled(session.is_none())
            .tooltip("The folder a new session works in")
            .render(),
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
    ])
    .gap(8.0);
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
fn diff_panel(files: &[FileEffect], width: f32, height: f32, palette: ComponentPalette) -> Element {
    let header = Element::row(vec![
        text("Changes"),
        Element::spacer().flex(1.0),
        Button::new(DIFF_CLOSE, "Close", palette)
            .variant(ButtonVariant::Ghost)
            .button_size(ButtonSize::Small)
            .render(),
    ])
    .padding(10.0)
    .size(Some(width), Some(44.0));
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
                .height((rows * 18.0 + 8.0).min(600.0))
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
