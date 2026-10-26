//! The contract between quill's UI and its engine (spec.md D7): the UI sends [`Intent`]s,
//! the engine answers with [`Event`]s. Neither side knows the other's crate.

use std::path::PathBuf;

pub use quill_agent::{
    AgentCapabilities, AgentEvent, ContextCategory, ContextKind, ContextUsage, Decision,
    FileChange, ModelInfo, PlanLimits, Suggestion, ToolContent, Usage, UsageWindow,
};

/// quill's own id of a session.
pub type SessionId = u64;
/// quill's own id of one message the person sent; the agent receives it as the prompt's id.
pub type RequestId = String;

/// What a new session starts with.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewSession {
    pub agent: String,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
}

/// How one file changed during a request.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
}

/// One file a request changed, whoever changed it (an edit tool or a shell command).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileEffect {
    /// Relative to the session's folder.
    pub path: String,
    pub kind: ChangeKind,
    pub added: u32,
    pub removed: u32,
    /// The unified diff of this file for the request, cut when very long.
    pub patch: String,
}

/// The repository a folder is in: its branch, where it pushes, and the branch pull requests
/// go into.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Repo {
    pub branch: String,
    /// `origin`'s URL, when there is one.
    pub remote: Option<String>,
    /// The branch `origin` names as its own, such as `main`.
    pub base: Option<String>,
}

/// What the person chose in Settings, as the engine keeps it in `settings.json`. A key it
/// does not know is dropped and a missing one takes its default, so an older or newer file
/// still reads.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    /// The size of the conversation's words.
    pub text_size: TextSize,
    /// The conversation in a wider column.
    pub wide: bool,
    pub motion: Motion,
    /// Keep the computer from idle sleep while an agent works.
    pub keep_awake: bool,
    /// Ask the desktop to draw attention to quill when a session finishes or waits on the
    /// person while quill is not in front.
    pub attention: bool,
    /// A pull request opens as a draft unless the person picks otherwise.
    pub pr_draft: bool,
    /// Start quill when the person logs in.
    pub open_at_login: bool,
    /// Sites the agents may use in the built-in browser without asking; local addresses
    /// never ask (docs/browser.md §5).
    pub browser_sites: Vec<String>,
    /// The Chromium the built-in browser runs; `None` takes `QUILL_CHROME` or the first on
    /// `PATH`.
    pub browser_program: Option<String>,
    /// Draw the browser's pages with Chromium in quill's own process, without the headless
    /// one's delay. From the next start.
    pub browser_native: bool,
    /// The editor files open in: the last chosen in Open in, "quill-ide" or an editor's URL
    /// scheme ("vscode", "cursor", "windsurf", "zed").
    pub editor: String,
    /// Each agent's own, by its id.
    pub agents: std::collections::BTreeMap<String, AgentSettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            text_size: TextSize::Medium,
            wide: false,
            motion: Motion::System,
            // Desktop keeps the computer awake by default and does not flash the app icon
            //; quill has no notifications yet, so drawing attention is
            // its only signal and starts on (inferred).
            keep_awake: true,
            attention: true,
            pr_draft: false,
            open_at_login: false,
            browser_sites: vec![],
            browser_program: None,
            browser_native: false,
            editor: "quill-ide".into(),
            agents: Default::default(),
        }
    }
}

impl Settings {
    pub fn agent(&self, agent: &str) -> AgentSettings {
        self.agents.get(agent).cloned().unwrap_or_default()
    }
}

/// What the person chose for one agent.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// The agent's program, when it is not the one on `PATH`.
    pub program: Option<String>,
    /// What a new session starts with; `None` leaves it to the agent.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
    /// Offer the agent's modes that skip every approval.
    pub allow_unsafe: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Motion {
    #[default]
    System,
    Reduced,
    Full,
}

/// An agent quill can run: its id, its name for people, and the versions of its program
/// quill was tested with.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub tested: Vec<String>,
}

/// What the UI asks of the engine.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Intent {
    NewSession(NewSession),
    Send {
        session: SessionId,
        text: String,
    },
    Decide {
        session: SessionId,
        permission: String,
        decision: Decision,
    },
    Interrupt {
        session: SessionId,
    },
    SetMode {
        session: SessionId,
        mode: String,
    },
    SetModel {
        session: SessionId,
        model: String,
    },
    /// How hard the agent thinks, one of the levels its model accepts.
    SetEffort {
        session: SessionId,
        effort: String,
    },
    /// How the session's context is spent, answered by the agent's `ContextUsage` while it
    /// runs; a session whose agent is not running gets no answer.
    ContextUsage {
        session: SessionId,
    },
    /// What is uncommitted in the session's folder, answered by `Event::WorkingTree`.
    WorkingTree {
        session: SessionId,
    },
    /// Put the session's files back as they were before `request` ran.
    Rewind {
        session: SessionId,
        request: RequestId,
    },
    /// The person's own name for the session, in place of the one its first message gave.
    Rename {
        session: SessionId,
        title: String,
    },
    /// Keep the session at the top of the list, or stop keeping it there.
    Pin {
        session: SessionId,
        pinned: bool,
    },
    /// Stop the session's agent and remove everything quill kept of it.
    Delete {
        session: SessionId,
    },
    /// Take the session out of the list, or put it back.
    Archive {
        session: SessionId,
        archived: bool,
    },
    /// Mark the session as having news the person has not seen, or as seen.
    MarkUnread {
        session: SessionId,
        unread: bool,
    },
    /// A new session that continues this one's conversation, leaving it as it was: all of
    /// it, or up to the message `at` (the agent's own id of it), as Desktop's "Fork from here".
    Fork {
        session: SessionId,
        #[serde(default)]
        at: Option<String>,
    },
    /// A file of the session's folder, by its path in it, for the document pane: answered by
    /// `Event::FileRead`.
    ReadFile {
        session: SessionId,
        path: String,
    },
    /// The entries of a folder of the session's, for the Files pane: answered by
    /// `Event::Listed`. Only a folder inside the session's is listed.
    ListDir {
        session: SessionId,
        dir: std::path::PathBuf,
    },
    /// A file operation in the session's folder, from the Files pane: answered by
    /// `Event::FileOpDone`.
    FileOp {
        session: SessionId,
        op: quill_files::Op,
    },
    /// Write what the person edited over a file of the session's folder.
    WriteFile {
        session: SessionId,
        path: String,
        text: String,
    },
    /// Keep these settings and act on them (start at login, keep awake…); answered by
    /// `Event::Settings`.
    SaveSettings(Settings),
    /// How much room quill's own data takes; answered by `Event::Storage`.
    MeasureStorage,
    /// Read a reply aloud with the system's voice, stopping any other; `None` stops.
    ReadAloud {
        session: SessionId,
        text: Option<String>,
    },
    /// A side question (`/btw`): a fork of the session, opened beside it, that answers without
    /// adding to its conversation.
    SideChat {
        session: SessionId,
        text: String,
    },
    /// The person's answer to a task the session proposed: start it, do it here, or not.
    AnswerProposal {
        session: SessionId,
        task: String,
        answer: ProposalAnswer,
    },
    /// Something the person does in the session's browser (docs/browser.md); answered by
    /// `Event::Browser`.
    Browser {
        session: SessionId,
        op: BrowserOp,
    },
}

/// The tool a site permission's card names: the built-in browser asks, not the agent, before
/// the agent opens or uses a site that is not local nor allowed (docs/browser.md §5).
pub const SITE_PERMISSION: &str = "quill.browser.site";

/// What the person does in a session's browser.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BrowserOp {
    /// Show the browser: its tabs, or a first one at `url` (a blank one without).
    Open {
        url: Option<String>,
    },
    Navigate {
        tab: u32,
        url: String,
    },
    NewTab {
        url: Option<String>,
    },
    CloseTab {
        tab: u32,
    },
    SelectTab {
        tab: u32,
    },
    Back {
        tab: u32,
    },
    Forward {
        tab: u32,
    },
    Reload {
        tab: u32,
    },
    Stop {
        tab: u32,
    },
    /// The tab on screen and the size of its view in logical pixels, or none on screen:
    /// only what is shown is drawn.
    Show {
        view: Option<BrowserView>,
    },
    Input {
        tab: u32,
        input: BrowserInput,
    },
    /// Copy what is selected in the page to the clipboard (`BrowserUpdate::Copied`).
    Copy {
        tab: u32,
    },
    /// Draw the page at `size` as a phone or a tablet would, or at the pane's size again
    /// (Desktop's Viewport menu: Responsive, Mobile, Tablet).
    Viewport {
        tab: u32,
        size: Option<(u32, u32)>,
    },
    /// Pick an element of the page to tell the agent about, or stop picking (Desktop's
    /// "Select element"): the pick comes back as `BrowserUpdate::Selected`.
    Select {
        tab: u32,
        on: bool,
    },
    /// The files the person chose for the page's file field `node`
    /// (`BrowserUpdate::ChooseFiles`); none leaves it as it was.
    Files {
        tab: u32,
        node: i64,
        paths: Vec<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BrowserView {
    pub tab: u32,
    pub width: u32,
    pub height: u32,
}

/// The pointer or the keyboard in a page, in logical pixels from the view's corner.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BrowserInput {
    Move {
        x: f32,
        y: f32,
    },
    Leave,
    Down {
        x: f32,
        y: f32,
        button: PointerButton,
        clicks: u8,
        keys: Modifiers,
    },
    Up {
        x: f32,
        y: f32,
        button: PointerButton,
        keys: Modifiers,
    },
    Wheel {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
    },
    /// A key pressed and released: the DOM's `key` and `code`, the Windows key code Chrome
    /// edits by, and the text it types, if any.
    Key {
        key: String,
        code: String,
        key_code: u32,
        text: String,
        keys: Modifiers,
    },
    /// Text typed or pasted, or committed by an input method.
    Text(String),
    /// What an input method is composing, shown in place until it commits.
    Composition(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// One tab of a session's browser.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BrowserTab {
    pub id: u32,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub back: bool,
    pub forward: bool,
    /// The viewport set for it, by the person or the agent, instead of the pane's size.
    #[serde(default)]
    pub emulated: Option<(u32, u32)>,
}

/// What changed in a session's browser.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BrowserUpdate {
    Tabs {
        tabs: Vec<BrowserTab>,
        active: Option<u32>,
    },
    /// The pointer's shape over the page, as CSS names it ("pointer", "text"…).
    Cursor(String),
    /// What was selected in the page, for the clipboard.
    Copied(String),
    /// Something the person should know: a dialog the page opened, a download saved.
    Notice(String),
    /// The browser could not start or stopped.
    Failed(String),
    /// Whether the page waits for the person to pick an element.
    Selecting(bool),
    /// The element the person picked, as words for the message being written.
    Selected(String),
    /// The addresses of the session's tabs changed: kept, so they open again after a restart
    /// (the only browser update that is stored).
    Kept(Vec<String>),
    /// The page asks for files: the host shows the desktop's file dialog and answers with
    /// `BrowserOp::Files`.
    ChooseFiles { tab: u32, node: i64, multiple: bool },
}

/// What the person does with a proposed task.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ProposalAnswer {
    /// In a session of its own, which reports back to the one that proposed it.
    Start,
    /// In the session that proposed it: its brief is sent there.
    Here,
    Dismiss,
}

/// A task a session proposed to run in a session of its own; nothing runs until the person
/// starts it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Proposal {
    pub id: String,
    pub title: String,
    /// The brief the new session starts from.
    pub prompt: String,
    /// The goal, decisions and constraints so far, for a session that starts fresh.
    pub background: Option<String>,
    /// The new session continues this whole conversation instead of starting fresh.
    pub fork: bool,
}

/// What became of a proposal.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ProposalOutcome {
    Started { session: SessionId },
    Kept,
    Dismissed,
}

/// What a session another one started said when its turn ended.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Report {
    pub from: SessionId,
    pub title: String,
    pub text: String,
}

/// Where a session's agent process is.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SessionStatus {
    Starting,
    Idle,
    Running,
    /// Waiting for the person to decide a permission or answer a question.
    Awaiting,
    /// The process ended; the next message starts it again, resuming the conversation.
    Exited,
    Failed(String),
}

/// What the engine tells the UI.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Event {
    SessionCreated {
        session: SessionId,
        agent: String,
        cwd: PathBuf,
    },
    Status {
        session: SessionId,
        status: SessionStatus,
    },
    /// The engine took a message and gave it its id; it is sent to the agent at once.
    Queued {
        session: SessionId,
        request: RequestId,
        text: String,
    },
    /// Something the agent said, tied to the request it answers when known.
    Agent {
        session: SessionId,
        request: Option<RequestId>,
        event: AgentEvent,
    },
    /// What files a finished request created, changed, removed or moved.
    Changes {
        session: SessionId,
        request: RequestId,
        files: Vec<FileEffect>,
        /// The folder before the request, as the engine's snapshot id: what a rewind restores.
        snapshot: String,
    },
    /// Where the session works, as the right panel shows it.
    Environment {
        session: SessionId,
        branch: Option<String>,
    },
    /// What is uncommitted in the session's folder now, against its branch's last commit,
    /// and the repository it is in, if it is in one.
    WorkingTree {
        session: SessionId,
        files: Vec<FileEffect>,
        repo: Option<Repo>,
    },
    /// The person answered a permission; kept so a session read back shows it answered.
    Decided {
        session: SessionId,
        permission: String,
        decision: Decision,
    },
    /// The session's files were put back as they were before `request`.
    Rewound {
        session: SessionId,
        request: RequestId,
        files: Vec<FileEffect>,
    },
    Renamed {
        session: SessionId,
        title: String,
    },
    Pinned {
        session: SessionId,
        pinned: bool,
    },
    /// Nothing of the session is kept any more.
    Deleted {
        session: SessionId,
    },
    Archived {
        session: SessionId,
        archived: bool,
    },
    Unread {
        session: SessionId,
        unread: bool,
    },
    /// The session continues `from`'s conversation: what `from` showed until now, or up to
    /// its message `at`, is its start.
    Forked {
        session: SessionId,
        from: SessionId,
        #[serde(default)]
        at: Option<String>,
    },
    /// A file of the session's folder as it is now on disk; `None` when it cannot be read as
    /// text, or is outside the folder.
    FileRead {
        session: SessionId,
        path: String,
        text: Option<String>,
    },
    /// A folder of the session's as it is now on disk, and when it last changed.
    Listed {
        session: SessionId,
        dir: std::path::PathBuf,
        entries: Vec<quill_files::Entry>,
        stamp: quill_files::Stamp,
    },
    /// A file operation done, with what it made (paths in the session's folder), or why not.
    FileOpDone {
        session: SessionId,
        op: quill_files::Op,
        result: Result<Vec<std::path::PathBuf>, String>,
    },
    /// Writing a file the person edited failed, and why.
    FileWriteFailed {
        session: SessionId,
        path: String,
        error: String,
    },
    /// The settings in force, and the agents quill can run. Not a session's: it is sent at
    /// start and after each save.
    Settings {
        settings: Settings,
        agents: Vec<AgentInfo>,
    },
    /// Where quill keeps its data and how many bytes it takes.
    Storage {
        path: PathBuf,
        bytes: u64,
    },
    /// What the session hears next was passed on by the session `from`, which started it.
    Forwarded {
        session: SessionId,
        from: SessionId,
    },
    /// The system's voice started or stopped reading one of the session's replies.
    Reading {
        session: SessionId,
        reading: bool,
    },
    /// The session was started by `parent`, which it reports back to: the sidebar nests it
    /// there. A side chat (`/btw`) reports nothing: it is the person's own aside.
    Linked {
        session: SessionId,
        parent: SessionId,
        #[serde(default)]
        side: bool,
    },
    /// The agent proposed a task for a session of its own.
    Proposed {
        session: SessionId,
        proposal: Proposal,
    },
    ProposalAnswered {
        session: SessionId,
        task: String,
        outcome: ProposalOutcome,
    },
    /// Sessions it started finished their turn; what they said is sent to the session as
    /// `request`, which begins a turn of its own.
    Reported {
        session: SessionId,
        request: RequestId,
        reports: Vec<Report>,
    },
    /// The session's browser changed; never stored.
    Browser {
        session: SessionId,
        update: BrowserUpdate,
    },
}

impl Event {
    pub fn session(&self) -> SessionId {
        match self {
            Event::SessionCreated { session, .. }
            | Event::Status { session, .. }
            | Event::Queued { session, .. }
            | Event::Agent { session, .. }
            | Event::Changes { session, .. }
            | Event::Rewound { session, .. }
            | Event::Decided { session, .. }
            | Event::WorkingTree { session, .. }
            | Event::Environment { session, .. }
            | Event::Renamed { session, .. }
            | Event::Pinned { session, .. }
            | Event::Deleted { session }
            | Event::Archived { session, .. }
            | Event::Unread { session, .. }
            | Event::Forked { session, .. }
            | Event::Reading { session, .. }
            | Event::Forwarded { session, .. }
            | Event::FileRead { session, .. }
            | Event::FileWriteFailed { session, .. }
            | Event::Listed { session, .. }
            | Event::FileOpDone { session, .. }
            | Event::Linked { session, .. }
            | Event::Proposed { session, .. }
            | Event::ProposalAnswered { session, .. }
            | Event::Reported { session, .. }
            | Event::Browser { session, .. } => *session,
            // quill's own, not a session's: no session has id 0.
            Event::Settings { .. } | Event::Storage { .. } => 0,
        }
    }
}
