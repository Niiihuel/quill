//! The contract between quill's UI and its engine (spec.md D7): the UI sends [`Intent`]s,
//! the engine answers with [`Event`]s. Neither side knows the other's crate.

use std::path::PathBuf;

pub use quill_agent::{
    AgentCapabilities, AgentEvent, Decision, FileChange, ModelInfo, PlanLimits, Suggestion, Usage,
    UsageWindow,
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
    /// Write what the person edited over a file of the session's folder.
    WriteFile {
        session: SessionId,
        path: String,
        text: String,
    },
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
    /// Writing a file the person edited failed, and why.
    FileWriteFailed {
        session: SessionId,
        path: String,
        error: String,
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
            | Event::FileRead { session, .. }
            | Event::FileWriteFailed { session, .. }
            | Event::Linked { session, .. }
            | Event::Proposed { session, .. }
            | Event::ProposalAnswered { session, .. }
            | Event::Reported { session, .. } => *session,
        }
    }
}
