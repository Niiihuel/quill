//! quill's interface: the state the engine's events build ([`Store::apply`]), what the person
//! is doing ([`Ui`]), what their actions ask ([`update`]) and the window ([`view`]).
//! Layout and colors follow Claude Desktop's Code surface (docs/ui.md, features/code-*.md).

pub mod icons;
pub mod look;
pub mod view;

pub use view::{menu_digit, view};

use quill_model::{
    AgentCapabilities, AgentEvent, Decision, Event, FileEffect, Intent, NewSession, RequestId,
    SessionId, SessionStatus, Suggestion,
};
use serde_json::Value;
use std::path::PathBuf;

/// One entry of a session's transcript.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    User {
        request: RequestId,
        text: String,
    },
    Assistant {
        text: String,
        streaming: bool,
    },
    Tool {
        call: String,
        name: String,
        input: Value,
        result: Option<ToolResult>,
    },
    Permission {
        id: String,
        /// The tool call it asks about.
        call: Option<String>,
        tool: String,
        input: Value,
        /// Why the agent asks, when it says.
        title: Option<String>,
        interactive: bool,
        suggestions: Vec<Suggestion>,
        decided: Option<&'static str>,
    },
    TurnEnd {
        ok: bool,
        interrupted: bool,
        cost: Option<f64>,
        error: Option<String>,
    },
    /// What one request changed in the folder, and whether it was rewound since.
    Changes {
        request: RequestId,
        files: Vec<FileEffect>,
        rewound: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    /// Lines added and removed, from the files the agent reported.
    pub added: usize,
    pub removed: usize,
    /// The files it changed, before and after, for the diff its row opens.
    pub files: Vec<quill_model::FileChange>,
    /// It never finished, or the person denied it: Desktop reads "Stopped".
    pub stopped: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: SessionId,
    pub agent: String,
    pub cwd: PathBuf,
    pub status: SessionStatus,
    pub items: Vec<Item>,
    pub capabilities: Option<AgentCapabilities>,
    pub mode: String,
    /// The first message, as the sidebar's title until the agent names the session.
    pub title: String,
    pub branch: Option<String>,
    /// Tokens the conversation fills after its last turn, and what the session has cost.
    pub context_tokens: u64,
    /// The model's context size, once a turn reported it.
    pub context_window: u64,
    pub cost: f64,
    /// What is uncommitted in its folder, as the engine last said.
    pub working: Option<Vec<quill_model::FileEffect>>,
}

/// What the engine said, as the transcript and sidebar show it.
#[derive(Default)]
pub struct Store {
    pub sessions: Vec<Session>,
    /// The plan's usage windows, as the agent last reported them; they are the person's,
    /// not one session's.
    pub limits: Option<quill_model::PlanLimits>,
}

impl Store {
    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    fn session_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::SessionCreated {
                session,
                agent,
                cwd,
            } => self.sessions.push(Session {
                id: session,
                agent,
                cwd,
                status: SessionStatus::Starting,
                items: vec![],
                capabilities: None,
                mode: "default".into(),
                title: String::new(),
                branch: None,
                context_tokens: 0,
                context_window: 0,
                working: None,
                cost: 0.0,
            }),
            Event::Status { session, status } => {
                if let Some(s) = self.session_mut(session) {
                    // A process that ended (or a session read back) answers nothing it asked.
                    if matches!(status, SessionStatus::Exited | SessionStatus::Failed(_)) {
                        close_open_permissions(&mut s.items);
                    }
                    s.status = status;
                }
            }
            Event::Queued {
                session,
                request,
                text,
            } => {
                if let Some(s) = self.session_mut(session) {
                    if s.title.is_empty() {
                        s.title = text
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .chars()
                            .take(60)
                            .collect();
                    }
                    s.items.push(Item::User { request, text });
                }
            }
            Event::Agent {
                event: AgentEvent::Limits(limits),
                ..
            } => self.limits = Some(limits),
            Event::Agent { session, event, .. } => {
                if let Some(s) = self.session_mut(session) {
                    agent_event(s, event);
                }
            }
            // A request that changed nothing gets no card.
            Event::Changes {
                session,
                request,
                files,
                ..
            } if !files.is_empty() => {
                if let Some(s) = self.session_mut(session) {
                    s.items.push(Item::Changes {
                        request,
                        files,
                        rewound: false,
                    });
                }
            }
            Event::Changes { .. } => {}
            Event::WorkingTree { session, files } => {
                if let Some(s) = self.session_mut(session) {
                    s.working = Some(files);
                }
            }
            Event::Environment { session, branch } => {
                if let Some(s) = self.session_mut(session) {
                    s.branch = branch;
                }
            }
            Event::Decided {
                session,
                permission,
                decision,
            } => {
                if let Some(s) = self.session_mut(session) {
                    mark_decided(&mut s.items, &permission, &decision);
                }
            }
            Event::Rewound {
                session, request, ..
            } => {
                if let Some(s) = self.session_mut(session) {
                    for item in &mut s.items {
                        if let Item::Changes {
                            request: r,
                            rewound,
                            ..
                        } = item
                        {
                            if *r == request {
                                *rewound = true;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn agent_event(s: &mut Session, event: AgentEvent) {
    match event {
        AgentEvent::Ready {
            capabilities, mode, ..
        } => {
            s.capabilities = Some(capabilities);
            if let Some(mode) = mode {
                s.mode = mode;
            }
        }
        AgentEvent::ModeChanged { mode } => s.mode = mode,
        // Subagents' own text stays out of the main transcript for now.
        AgentEvent::TextDelta { text, parent: None } => match s.items.last_mut() {
            Some(Item::Assistant {
                text: shown,
                streaming: true,
            }) => shown.push_str(&text),
            _ => s.items.push(Item::Assistant {
                text,
                streaming: true,
            }),
        },
        AgentEvent::Text { text, parent: None } => match s.items.last_mut() {
            Some(Item::Assistant {
                text: shown,
                streaming,
            }) if *streaming => {
                *shown = text;
                *streaming = false;
            }
            _ => s.items.push(Item::Assistant {
                text,
                streaming: false,
            }),
        },
        AgentEvent::ToolUse {
            call,
            name,
            input,
            parent: None,
        } => s.items.push(Item::Tool {
            call,
            name,
            input,
            result: None,
        }),
        AgentEvent::ToolResult {
            call,
            output,
            is_error,
            files,
            ..
        } => {
            let (added, removed) = files
                .iter()
                .map(|f| line_changes(f.before.as_deref(), f.after.as_deref()))
                .fold((0, 0), |(a, r), (x, y)| (a + x, r + y));
            let denied = s.items.iter().any(|item| {
                matches!(item, Item::Permission { call: Some(c), decided: Some("Denied"), .. } if *c == call)
            });
            for item in s.items.iter_mut().rev() {
                if let Item::Tool {
                    call: c, result, ..
                } = item
                {
                    if *c == call {
                        *result = Some(ToolResult {
                            output,
                            is_error,
                            added,
                            removed,
                            files,
                            stopped: denied,
                        });
                        break;
                    }
                }
            }
        }
        AgentEvent::PermissionRequest {
            id,
            call,
            tool,
            input,
            title,
            interactive,
            suggestions,
        } => s.items.push(Item::Permission {
            id,
            call,
            tool,
            input,
            title,
            interactive,
            suggestions,
            decided: None,
        }),
        AgentEvent::TurnFinished {
            ok,
            interrupted,
            usage,
            error,
        } => {
            if let Some(Item::Assistant { streaming, .. }) = s.items.last_mut() {
                *streaming = false;
            }
            // A finished turn waits on nothing: a permission it left open (interrupted, or
            // answered before answers were kept) is closed.
            close_open_permissions(&mut s.items);
            if usage.context_window > 0 {
                s.context_window = usage.context_window;
            }
            if usage.context_tokens > 0 {
                s.context_tokens = usage.context_tokens;
            }
            s.cost += usage.cost_usd.unwrap_or(0.0);
            s.items.push(Item::TurnEnd {
                ok,
                interrupted,
                cost: usage.cost_usd,
                error,
            });
        }
        _ => {}
    }
}

/// Lines added and removed between two versions of a file, counted as unmatched lines
/// (a rewritten line counts once each way, as Desktop shows +1 −1).
fn line_changes(before: Option<&str>, after: Option<&str>) -> (usize, usize) {
    let mut left: Vec<&str> = before.unwrap_or_default().lines().collect();
    let mut added = 0;
    for line in after.unwrap_or_default().lines() {
        match left.iter().position(|l| *l == line) {
            Some(at) => drop(left.remove(at)),
            None => added += 1,
        }
    }
    (added, left.len())
}

/// The folder the terminal and new sessions work in: the selected session's, else the chosen one.
pub fn folder(store: &Store, ui: &Ui) -> PathBuf {
    ui.selected
        .and_then(|id| store.session(id))
        .map_or_else(|| ui.cwd.clone(), |s| s.cwd.clone())
}

/// What the person is doing, apart from what the engine knows.
pub struct Ui {
    pub selected: Option<SessionId>,
    pub prompt: String,
    /// Where a new session starts.
    pub cwd: PathBuf,
    pub agent: String,
    pub mode: String,
    pub model: Option<String>,
    /// The effort new sessions start with and the selected one runs at.
    pub effort: Option<String>,
    pub open_menu: Option<&'static str>,
    pub expanded: Vec<String>,
    /// A message written before its session existed, sent once it does.
    pub waiting: Option<String>,
    /// The request whose changes the diff pane shows.
    pub diff: Option<(SessionId, RequestId)>,
    /// The panes open beside the conversation, in the order they opened: the first sets the
    /// column's width and the rest stack under it, as Desktop's do.
    pub panes: Vec<Pane>,
    /// The pane that fills the space of the conversation, as Desktop's expand does.
    pub expanded_pane: Option<Pane>,
    /// Whether the right panel (Environment, Context, Changes) shows while no pane is open.
    pub panel_open: bool,
    /// Folders whose sessions the sidebar folds away.
    pub collapsed: Vec<PathBuf>,
    /// The person's first name for the new session's greeting, when the host knows it.
    pub name: Option<String>,
    /// The local time's distance from UTC in seconds, for the hours limits reset at.
    pub utc_offset: i64,
    /// How the person allowed last, which the next permission card offers first.
    pub last_allow: Allow,
    /// Whether the transcript shows its end, which the host sets every frame.
    pub at_end: bool,
    /// The window's size, which the host sets every frame; a dragged boundary needs it.
    pub window: limn::Size,
    /// The column of panes' share of the space right of the sidebar, once the person dragged
    /// its boundary; until then the first pane's own share.
    pub column_fraction: Option<f32>,
    /// The top pane's share of two stacked panes, once their boundary was dragged.
    pub stack_fraction: Option<f32>,
    /// Answers picked on question cards, by permission and question index.
    pub answers: std::collections::HashMap<(String, usize), String>,
    /// The permission whose "Always allow" menu is open.
    pub perm_menu: Option<String>,
    pub terminal: TerminalPane,
}

/// The two ways to allow a permission; the card's main button is the one used last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Allow {
    Once,
    Always,
}

/// The click a key press stands for: Ctrl+Enter allows the waiting permission once and
/// Ctrl+Shift+Enter always, wherever the focus is, as in Desktop.
pub fn shortcut(store: &Store, ui: &Ui, key: limn::Key, ctrl: bool, shift: bool) -> Option<String> {
    if key != limn::Key::Enter || !ctrl {
        return None;
    }
    let session = store.session(ui.selected?)?;
    let id = session.items.iter().rev().find_map(|item| match item {
        Item::Permission {
            id,
            interactive: false,
            decided: None,
            ..
        } => Some(id),
        _ => None,
    })?;
    Some(format!(
        "perm:{id}:{}",
        if shift { "always" } else { "once" }
    ))
}

/// What can open in the column beside the conversation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pane {
    Diff,
    Terminal,
}

impl Pane {
    pub fn key(self) -> &'static str {
        match self {
            Pane::Diff => "diff",
            Pane::Terminal => "terminal",
        }
    }
}

/// The terminal pane: what the host's shells last showed, and what the person asked of them.
#[derive(Default)]
pub struct TerminalPane {
    /// The folder's tabs, as the host's shells last were.
    pub tabs: Vec<TerminalTab>,
    pub active: usize,
    pub screen: Option<std::sync::Arc<limn::TerminalScreen>>,
    /// A close that waits for the person to confirm, as a command runs where it closes.
    pub closing: Option<Closing>,
    /// The tab whose pill is a name field, and what the field holds.
    pub renaming: Option<(usize, String)>,
    /// What the host does with the folder's shells on the next frame.
    pub requests: Vec<TerminalRequest>,
}

/// One terminal tab as the pane shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalTab {
    pub title: String,
    pub busy: bool,
    /// Named by the person: its pill drops the folder.
    pub named: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Closing {
    One(usize),
    /// Every tab but this one.
    Others(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TerminalRequest {
    New,
    Select(usize),
    Close(usize),
    CloseOthers(usize),
    Rename(usize, Option<String>),
    Clear(usize),
    Move { from: usize, to: usize },
}

impl Ui {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            selected: None,
            prompt: String::new(),
            cwd,
            agent: "claude".into(),
            mode: "default".into(),
            model: None,
            effort: None,
            open_menu: None,
            expanded: vec![],
            waiting: None,
            diff: None,
            panes: vec![],
            expanded_pane: None,
            panel_open: true,
            collapsed: vec![],
            last_allow: Allow::Once,
            name: None,
            utc_offset: 0,
            window: limn::Size::new(1408.0, 844.0),
            at_end: true,
            column_fraction: None,
            stack_fraction: None,
            answers: Default::default(),
            perm_menu: None,
            terminal: TerminalPane::default(),
        }
    }

    pub fn pane_open(&self, pane: Pane) -> bool {
        self.panes.contains(&pane)
    }

    pub fn open_pane(&mut self, pane: Pane) {
        if !self.pane_open(pane) {
            self.panes.push(pane);
        }
    }

    pub fn close_pane(&mut self, pane: Pane) {
        self.panes.retain(|p| *p != pane);
        if self.expanded_pane == Some(pane) {
            self.expanded_pane = None;
        }
        if pane == Pane::Diff {
            self.diff = None;
        }
    }

    /// After the engine reported `event`: select a session the person just created
    /// and send what they wrote while it was being made.
    pub fn applied(&mut self, event: &Event) -> Vec<Intent> {
        if let Event::SessionCreated { session, .. } = event {
            if let Some(text) = self.waiting.take() {
                self.selected = Some(*session);
                return vec![Intent::Send {
                    session: *session,
                    text,
                }];
            }
        }
        // A finished turn may have changed files: what is uncommitted is asked for again.
        if let Event::Agent {
            session,
            event: AgentEvent::TurnFinished { .. },
            ..
        } = event
        {
            if self.selected == Some(*session) {
                return vec![Intent::WorkingTree { session: *session }];
            }
        }
        vec![]
    }
}

/// What one action of the person asks of the engine; the rest changes `ui` only.
pub fn update(store: &Store, ui: &mut Ui, action: &limn::UiAction) -> Vec<Intent> {
    use limn::UiAction;
    match action {
        UiAction::InputChanged { id, value } if id == view::TERMINAL_RENAME => {
            if let Some((_, name)) = &mut ui.terminal.renaming {
                *name = value.clone();
            }
            vec![]
        }
        UiAction::Submitted { id, value } if id == view::TERMINAL_RENAME => {
            if let Some((tab, _)) = ui.terminal.renaming.take() {
                ui.terminal
                    .requests
                    .push(TerminalRequest::Rename(tab, Some(value.clone())));
            }
            vec![]
        }
        UiAction::InputChanged { id, value } if id == view::COMPOSER => {
            ui.prompt = value.clone();
            vec![]
        }
        UiAction::Submitted { id, value } if id == view::COMPOSER => {
            let text = value.trim().to_owned();
            if text.is_empty() {
                return vec![];
            }
            ui.prompt.clear();
            match ui.selected {
                Some(session) => vec![Intent::Send { session, text }],
                None => {
                    ui.waiting = Some(text);
                    vec![Intent::NewSession(NewSession {
                        agent: ui.agent.clone(),
                        cwd: ui.cwd.clone(),
                        model: ui.model.clone(),
                        effort: ui.effort.clone(),
                        mode: Some(ui.mode.clone()),
                    })]
                }
            }
        }
        UiAction::Clicked { id } => clicked(store, ui, id),
        UiAction::DragStarted { id, position } | UiAction::DragMoved { id, position, .. } => {
            dragged(ui, id, *position);
            vec![]
        }
        _ => vec![],
    }
}

/// A boundary followed the pointer to `position`: the column's left edge, or the top of the
/// lower stacked pane.
fn dragged(ui: &mut Ui, id: &str, position: limn::Point) {
    let rest = ui.window.width - look::SIDEBAR_WIDTH;
    if id == view::COLUMN_HANDLE && rest > 0.0 {
        let width = view::column_width(rest, (ui.window.width - position.x) / rest);
        ui.column_fraction = Some(width / rest);
    } else if id == view::STACK_HANDLE {
        let room = ui.window.height - look::TOP_BAR - 1.0 - 2.0;
        if room > 0.0 {
            let top = (position.y - look::TOP_BAR - 1.0)
                .clamp(view::MIN_PANE, (room - view::MIN_PANE).max(view::MIN_PANE));
            ui.stack_fraction = Some(top / room);
        }
    }
}

fn terminal_clicked(pane: &mut TerminalPane, id: &str) {
    let index = |prefix: &str| id.strip_prefix(prefix)?.parse::<usize>().ok();
    if id == view::TERMINAL_NEW {
        pane.requests.push(TerminalRequest::New);
    } else if id == view::TERMINAL_CLOSE_CONFIRM {
        pane.requests
            .extend(pane.closing.take().map(|closing| match closing {
                Closing::One(tab) => TerminalRequest::Close(tab),
                Closing::Others(tab) => TerminalRequest::CloseOthers(tab),
            }));
    } else if id == view::TERMINAL_CLOSE_CANCEL {
        pane.closing = None;
    } else if let Some(tab) = index("terminal:menu:rename:") {
        // Desktop's field starts empty.
        pane.renaming = Some((tab, String::new()));
    } else if let Some(tab) = index("terminal:menu:clear:") {
        pane.requests.push(TerminalRequest::Clear(tab));
    } else if let Some(tab) = index("terminal:menu:others:") {
        let busy = pane
            .tabs
            .iter()
            .enumerate()
            .any(|(i, t)| i != tab && t.busy);
        if busy {
            pane.closing = Some(Closing::Others(tab));
        } else {
            pane.requests.push(TerminalRequest::CloseOthers(tab));
        }
    } else if let Some(tab) = index("terminal:tab:") {
        pane.requests.push(TerminalRequest::Select(tab));
    } else if let Some(tab) = index("terminal:close:").or_else(|| index("terminal:menu:close:")) {
        // A running command is stopped only once the person confirms.
        if pane.tabs.get(tab).is_some_and(|t| t.busy) {
            pane.closing = Some(Closing::One(tab));
        } else {
            pane.requests.push(TerminalRequest::Close(tab));
        }
    }
}

/// Keys for the terminal's tabs, which the host passes before the window takes them: on a
/// focused pill Delete or Backspace closes it and Ctrl+Shift+arrows move it; in the name
/// field Escape gives up. Whether the key was taken.
pub fn terminal_key(
    ui: &mut Ui,
    focused: Option<&str>,
    key: limn::Key,
    ctrl: bool,
    shift: bool,
) -> bool {
    use limn::Key;
    let pane = &mut ui.terminal;
    if focused == Some(view::TERMINAL_RENAME) && key == Key::Escape {
        pane.renaming = None;
        return true;
    }
    let Some(tab) = focused
        .and_then(|f| f.strip_prefix("terminal:tab:"))
        .and_then(|i| i.parse::<usize>().ok())
    else {
        return false;
    };
    match key {
        Key::Delete | Key::Backspace => terminal_clicked(pane, &format!("terminal:close:{tab}")),
        Key::ArrowLeft | Key::ArrowRight if ctrl && shift => {
            let to = if key == Key::ArrowLeft {
                tab.checked_sub(1)
            } else {
                Some(tab + 1).filter(|to| *to < pane.tabs.len())
            };
            if let Some(to) = to {
                pane.requests.push(TerminalRequest::Move { from: tab, to });
            }
        }
        _ => return false,
    }
    true
}

fn clicked(store: &Store, ui: &mut Ui, id: &str) -> Vec<Intent> {
    if id == view::NEW_SESSION {
        ui.selected = None;
        return vec![];
    }
    if let Some(rest) = id.strip_prefix("ask:") {
        return answer(store, ui, rest);
    }
    if id == view::TERMINAL_TOGGLE {
        if ui.pane_open(Pane::Terminal) {
            ui.close_pane(Pane::Terminal);
        } else {
            ui.open_pane(Pane::Terminal);
        }
        return vec![];
    }
    if id.starts_with("terminal:") {
        terminal_clicked(&mut ui.terminal, id);
        return vec![];
    }
    if let Some(folder) = id.strip_prefix("group:") {
        let folder = PathBuf::from(folder);
        if let Some(at) = ui.collapsed.iter().position(|f| *f == folder) {
            ui.collapsed.remove(at);
        } else {
            ui.collapsed.push(folder);
        }
        return vec![];
    }
    if id == view::PANEL_TOGGLE {
        ui.panel_open = !ui.panel_open;
        return vec![];
    }
    if id == view::DIFF_TOGGLE {
        // Desktop's diff is what is uncommitted, "master → working tree".
        if ui.pane_open(Pane::Diff) {
            ui.close_pane(Pane::Diff);
        } else if let Some(session) = ui.selected {
            ui.diff = None;
            ui.open_pane(Pane::Diff);
            return vec![Intent::WorkingTree { session }];
        }
        return vec![];
    }
    if let Some(rest) = id.strip_prefix("pane:") {
        for pane in [Pane::Diff, Pane::Terminal] {
            if rest == format!("{}:close", pane.key()) {
                ui.close_pane(pane);
            } else if rest == format!("{}:expand", pane.key()) {
                ui.expanded_pane = (ui.expanded_pane != Some(pane)).then_some(pane);
            }
        }
        return vec![];
    }
    if let Some(rest) = id.strip_prefix("changes:") {
        let Some(session) = ui.selected else {
            return vec![];
        };
        if let Some(request) = rest.strip_suffix(":view") {
            ui.diff = Some((session, request.to_owned()));
            ui.open_pane(Pane::Diff);
        } else if let Some(request) = rest.strip_suffix(":undo") {
            return vec![Intent::Rewind {
                session,
                request: request.to_owned(),
            }];
        }
        return vec![];
    }
    if id == view::STOP {
        return ui
            .selected
            .map(|session| vec![Intent::Interrupt { session }])
            .unwrap_or_default();
    }
    if id == view::SEND {
        let value = ui.prompt.clone();
        return update(
            store,
            ui,
            &limn::UiAction::Submitted {
                id: view::COMPOSER.into(),
                value,
            },
        );
    }
    // The "+" actions are the host's (pickers, the editor's text); the menu closes.
    if [
        view::ATTACH_FILES,
        view::ATTACH_FOLDER,
        view::SLASH,
        view::PICK_FOLDER,
        view::USAGE_ALL,
    ]
    .contains(&id)
    {
        ui.open_menu = None;
        return vec![];
    }
    if let Some(folder) = id.strip_prefix("new-in:") {
        ui.selected = None;
        ui.cwd = PathBuf::from(folder);
        return vec![];
    }
    if let Some(folder) = id.strip_prefix("folder:") {
        ui.open_menu = None;
        ui.cwd = PathBuf::from(folder);
        return vec![];
    }
    if let Some(effort) = id.strip_prefix("effort:") {
        ui.open_menu = None;
        ui.effort = Some(effort.to_owned());
        return ui
            .selected
            .map(|session| {
                vec![Intent::SetEffort {
                    session,
                    effort: effort.to_owned(),
                }]
            })
            .unwrap_or_default();
    }
    if id == format!("{}:effort", view::MODEL_MENU) {
        return vec![];
    }
    for menu in [
        view::MODE_MENU,
        view::MODEL_MENU,
        view::PLUS_MENU,
        view::FOLDER_MENU,
        view::USAGE_MENU,
        view::EFFORT_MENU,
    ] {
        if id == menu {
            ui.open_menu = if ui.open_menu == Some(menu) {
                None
            } else {
                Some(menu)
            };
            return vec![];
        }
        if id == format!("{menu}:dismiss") {
            ui.open_menu = None;
            return vec![];
        }
    }
    if let Some(mode) = id.strip_prefix("mode:") {
        ui.open_menu = None;
        ui.mode = mode.to_owned();
        return ui
            .selected
            .map(|session| {
                vec![Intent::SetMode {
                    session,
                    mode: mode.to_owned(),
                }]
            })
            .unwrap_or_default();
    }
    if let Some(model) = id.strip_prefix("model:") {
        ui.open_menu = None;
        ui.model = Some(model.to_owned());
        return ui
            .selected
            .map(|session| {
                vec![Intent::SetModel {
                    session,
                    model: model.to_owned(),
                }]
            })
            .unwrap_or_default();
    }
    if let Some(session) = id.strip_prefix("session:").and_then(|s| s.parse().ok()) {
        ui.selected = Some(session);
        return vec![Intent::WorkingTree { session }];
    }
    if id.starts_with("steps:") || id.starts_with("panel:") {
        if let Some(at) = ui.expanded.iter().position(|c| c == id) {
            ui.expanded.remove(at);
        } else {
            ui.expanded.push(id.to_owned());
        }
        return vec![];
    }
    if let Some(call) = id.strip_prefix("tool:") {
        if let Some(at) = ui.expanded.iter().position(|c| c == call) {
            ui.expanded.remove(at);
        } else {
            ui.expanded.push(call.to_owned());
        }
        return vec![];
    }
    if let Some(rest) = id.strip_prefix("perm:") {
        if let Some(permission) = rest.strip_suffix(":menu") {
            ui.perm_menu =
                (ui.perm_menu.as_deref() != Some(permission)).then(|| permission.to_owned());
            return vec![];
        }
        if rest.ends_with(":menu-dismiss") {
            ui.perm_menu = None;
            return vec![];
        }
        ui.perm_menu = None;
        let Some((permission, choice)) = rest.rsplit_once(':') else {
            return vec![];
        };
        let Some(session) = ui.selected.and_then(|s| store.session(s)) else {
            return vec![];
        };
        let Some(Item::Permission { suggestions, .. }) = session
            .items
            .iter()
            .find(|i| matches!(i, Item::Permission { id, .. } if id == permission))
        else {
            return vec![];
        };
        let decision = match choice {
            "deny" => Decision::Deny {
                message: "The person denied this.".into(),
            },
            // Everything the agent suggests, a mode switch included: Desktop's "Always allow" on
            // a file-creating command left the session in Accept edits.
            "always" => Decision::AllowAlways(suggestions.clone()),
            _ => Decision::Allow,
        };
        // Desktop offers the last choice first on the next card.
        match choice {
            "once" => ui.last_allow = Allow::Once,
            "always" => ui.last_allow = Allow::Always,
            _ => {}
        }
        return vec![Intent::Decide {
            session: session.id,
            permission: permission.to_owned(),
            decision,
        }];
    }
    vec![]
}

/// Marks a permission as decided once the person chose, so its card closes at once.
pub fn decided(store: &mut Store, intent: &Intent) {
    if let Intent::Decide {
        session,
        permission,
        decision,
    } = intent
    {
        if let Some(s) = store.session_mut(*session) {
            mark_decided(&mut s.items, permission, decision);
        }
    }
}

/// What a turn left open once nothing can answer it: its permissions close, and a tool that
/// never reported stops spinning.
fn close_open_permissions(items: &mut [Item]) {
    for item in items {
        match item {
            Item::Permission {
                decided: decided @ None,
                ..
            } => *decided = Some("Closed"),
            Item::Tool {
                result: result @ None,
                ..
            } => {
                *result = Some(ToolResult {
                    output: String::new(),
                    is_error: false,
                    added: 0,
                    removed: 0,
                    files: vec![],
                    stopped: true,
                })
            }
            _ => {}
        }
    }
}

fn mark_decided(items: &mut [Item], permission: &str, decision: &Decision) {
    for item in items {
        if let Item::Permission { id, decided, .. } = item {
            if id == permission {
                *decided = Some(match decision {
                    Decision::Deny { .. } => "Denied",
                    Decision::AllowAlways(_) => "Always allowed",
                    _ => "Allowed",
                });
            }
        }
    }
}

#[cfg(test)]
mod tests;

/// A pick or a submit on a question card: `{permission}:{question}:{option}` or `{permission}:submit`.
fn answer(store: &Store, ui: &mut Ui, rest: &str) -> Vec<Intent> {
    let Some(session) = ui.selected.and_then(|s| store.session(s)) else {
        return vec![];
    };
    let find = |permission: &str| {
        session.items.iter().find_map(|i| match i {
            Item::Permission { id, input, .. } if id == permission => Some(input.clone()),
            _ => None,
        })
    };
    if let Some(permission) = rest.strip_suffix(":submit") {
        let Some(input) = find(permission) else {
            return vec![];
        };
        let answers: serde_json::Map<String, Value> = input["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(q, question)| {
                let picked = ui.answers.get(&(permission.to_owned(), q))?;
                Some((
                    question["question"].as_str()?.to_owned(),
                    Value::String(picked.clone()),
                ))
            })
            .collect();
        return vec![Intent::Decide {
            session: session.id,
            permission: permission.to_owned(),
            decision: Decision::Answer(Value::Object(answers)),
        }];
    }
    let mut parts = rest.rsplitn(3, ':');
    let (Some(option), Some(question), Some(permission)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return vec![];
    };
    let (Ok(q), Ok(o)) = (question.parse::<usize>(), option.parse::<usize>()) else {
        return vec![];
    };
    if let Some(label) = find(permission).and_then(|input| {
        input["questions"][q]["options"][o]["label"]
            .as_str()
            .map(str::to_owned)
    }) {
        ui.answers.insert((permission.to_owned(), q), label);
    }
    vec![]
}

/// The words of the reply a "copy:{session}:{item}" button stands for.
pub fn copied_text(store: &Store, id: &str) -> Option<String> {
    let (session, at) = id.strip_prefix("copy:")?.split_once(':')?;
    let session = store.session(session.parse().ok()?)?;
    match session.items.get(at.parse::<usize>().ok()?)? {
        Item::Assistant { text, .. } => Some(text.clone()),
        _ => None,
    }
}
