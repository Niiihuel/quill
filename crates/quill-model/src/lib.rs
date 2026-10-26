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
    /// A new session that continues this one's conversation, leaving it as it was.
    Fork {
        session: SessionId,
    },
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
    /// What is uncommitted in the session's folder now, against its branch's last commit.
    WorkingTree {
        session: SessionId,
        files: Vec<FileEffect>,
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
    /// The session continues `from`'s conversation: what `from` showed until now is its start.
    Forked {
        session: SessionId,
        from: SessionId,
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
            | Event::Forked { session, .. } => *session,
        }
    }
}
