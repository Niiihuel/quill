//! quill's interface: the state the engine's events build ([`Store::apply`]), what the person
//! is doing ([`Ui`]), what their actions ask ([`update`]) and the window ([`view`]).
//! Layout and colors follow Claude Desktop's Code surface (docs/ui.md, features/code-*.md).

mod look;
pub mod view;

pub use view::view;

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
        tool: String,
        input: Value,
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
    pub cost: f64,
}

/// What the engine said, as the transcript and sidebar show it.
#[derive(Default)]
pub struct Store {
    pub sessions: Vec<Session>,
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
                cost: 0.0,
            }),
            Event::Status { session, status } => {
                if let Some(s) = self.session_mut(session) {
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
                        });
                        break;
                    }
                }
            }
        }
        AgentEvent::PermissionRequest {
            id,
            tool,
            input,
            interactive,
            suggestions,
            ..
        } => s.items.push(Item::Permission {
            id,
            tool,
            input,
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
    /// Answers picked on question cards, by permission and question index.
    pub answers: std::collections::HashMap<(String, usize), String>,
    /// The permission whose "Always allow" menu is open.
    pub perm_menu: Option<String>,
    pub terminal: TerminalPane,
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
    /// The folder's tabs: title, and whether a command runs in it.
    pub tabs: Vec<(String, bool)>,
    pub active: usize,
    pub screen: Option<std::sync::Arc<limn::TerminalScreen>>,
    /// The tab whose close waits for the person to confirm, as a command runs in it.
    pub closing: Option<usize>,
    /// What the host does with the folder's shells on the next frame.
    pub requests: Vec<TerminalRequest>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TerminalRequest {
    New,
    Select(usize),
    Close(usize),
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
            open_menu: None,
            expanded: vec![],
            waiting: None,
            diff: None,
            panes: vec![],
            expanded_pane: None,
            panel_open: true,
            collapsed: vec![],
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
        vec![]
    }
}

/// What one action of the person asks of the engine; the rest changes `ui` only.
pub fn update(store: &Store, ui: &mut Ui, action: &limn::UiAction) -> Vec<Intent> {
    use limn::UiAction;
    match action {
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
                        effort: None,
                        mode: Some(ui.mode.clone()),
                    })]
                }
            }
        }
        UiAction::Clicked { id } => clicked(store, ui, id),
        _ => vec![],
    }
}

/// The selected session's last request that changed files, which the diff toggle opens.
fn last_changes(store: &Store, ui: &Ui) -> Option<(SessionId, RequestId)> {
    let session = store.session(ui.selected?)?;
    session.items.iter().rev().find_map(|item| match item {
        Item::Changes { request, .. } => Some((session.id, request.clone())),
        _ => None,
    })
}

fn terminal_clicked(pane: &mut TerminalPane, id: &str) {
    let index = |prefix: &str| id.strip_prefix(prefix)?.parse::<usize>().ok();
    if id == view::TERMINAL_NEW {
        pane.requests.push(TerminalRequest::New);
    } else if id == view::TERMINAL_CLOSE_CONFIRM {
        pane.requests
            .extend(pane.closing.take().map(TerminalRequest::Close));
    } else if id == view::TERMINAL_CLOSE_CANCEL {
        pane.closing = None;
    } else if let Some(tab) = index("terminal:tab:") {
        pane.requests.push(TerminalRequest::Select(tab));
    } else if let Some(tab) = index("terminal:close:") {
        // A running command is stopped only once the person confirms.
        if pane.tabs.get(tab).is_some_and(|(_, busy)| *busy) {
            pane.closing = Some(tab);
        } else {
            pane.requests.push(TerminalRequest::Close(tab));
        }
    }
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
        if ui.pane_open(Pane::Diff) {
            ui.close_pane(Pane::Diff);
        } else if let Some((session, request)) = last_changes(store, ui) {
            ui.diff = Some((session, request));
            ui.open_pane(Pane::Diff);
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
    for menu in [view::MODE_MENU, view::MODEL_MENU] {
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
            // Rules only: a mode switch offered with them is the person's separate choice (engine.md §11.4).
            "always" => Decision::AllowAlways(
                suggestions
                    .iter()
                    .filter(|s| matches!(s, Suggestion::Rules(_)))
                    .cloned()
                    .collect(),
            ),
            _ => Decision::Allow,
        };
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
