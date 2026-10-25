//! The contract between quill's engine and one coding-agent CLI (docs/agents.md).
//!
//! An adapter is pure: it never opens a process or a socket. The engine starts what
//! [`AgentAdapter::launch`] describes, writes what [`AgentAdapter::encode`] returns, and
//! hands every line it reads to [`AgentAdapter::decode`]. Nothing here names a vendor.

use serde_json::Value;
use std::path::PathBuf;

/// Which agent: `"claude"`, later `"codex"`, `"opencode"`…
pub type AgentId = &'static str;

/// How the engine starts the agent's process.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
    pub cwd: PathBuf,
}

/// What a new or resumed session asks of the agent.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionSpec {
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
    /// The agent's own id of a session to continue.
    pub resume: Option<String>,
    /// Continue `resume` as a new session, leaving the old one as it was.
    pub fork: bool,
}

/// One model the agent offers, with the efforts it accepts.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub efforts: Vec<String>,
}

/// What this agent can do; the UI offers only what is here.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentCapabilities {
    pub models: Vec<ModelInfo>,
    /// Permission modes, in the agent's own words (`default`, `acceptEdits`, `plan`…).
    pub modes: Vec<String>,
    pub interrupt: bool,
    pub resume: bool,
    pub fork: bool,
    /// The agent restores files to before a request by itself.
    pub rewind: bool,
    /// The agent reports each tool as it ends, tied to its request: per-tool attribution.
    pub tool_hooks: bool,
    /// Optional features the agent announced, as it names them.
    pub features: Vec<String>,
    /// The person's plan, as the agent names it ("max", "pro"), when it says.
    #[serde(default)]
    pub plan: Option<String>,
}

/// One file a tool changed, as the agent reported it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileChange {
    pub path: PathBuf,
    /// Content before the change; `None` when the tool created the file.
    pub before: Option<String>,
    pub after: Option<String>,
}

/// A shell command a tool ran.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CommandRun {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub interrupted: bool,
}

/// One permission the agent offers to remember, kept as the agent wrote it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Suggestion {
    /// Allow rules for later calls.
    Rules(Value),
    /// Switch the session's permission mode.
    Mode(String),
    /// Anything else (e.g. extra directories), passed back untouched if chosen.
    Other(Value),
}

/// The agent's own token use for a turn.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
    /// How much of the model's context the conversation fills after the turn.
    #[serde(default)]
    pub context_tokens: u64,
    /// The size of the model's context, when the agent says.
    #[serde(default)]
    pub context_window: u64,
}

/// One usage window of the person's plan.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UsageWindow {
    /// The share spent, 0 to 1.
    pub utilization: f32,
    /// When it starts again, in Unix seconds.
    pub resets_at: u64,
}

/// The plan's usage windows the agent reported: the session's five hours and the week.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanLimits {
    pub session: Option<UsageWindow>,
    pub weekly: Option<UsageWindow>,
}

/// Something the agent said, in quill's terms.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AgentEvent {
    /// The agent answered `Start`: who it is and what it can do.
    Ready {
        version: String,
        capabilities: AgentCapabilities,
        mode: Option<String>,
    },
    /// The agent's id for this session, once it is known.
    Session {
        id: String,
    },
    /// A request began running (`request` is the id quill sent with the prompt).
    Running {
        requests: Vec<String>,
    },
    TextDelta {
        text: String,
        parent: Option<String>,
    },
    Text {
        text: String,
        parent: Option<String>,
    },
    Thinking {
        parent: Option<String>,
    },
    /// The tokens the running turn has used so far, its helpers' included.
    Tokens {
        tokens: u64,
    },
    /// Work the agent runs beside its turn began: a helper agent, a command in the
    /// background. `call` is the tool call that started it.
    TaskStarted {
        id: String,
        call: Option<String>,
        description: String,
    },
    /// That work ended; `summary` is its own account of what it did.
    TaskEnded {
        id: String,
        call: Option<String>,
        summary: Option<String>,
        ok: bool,
    },
    ToolUse {
        call: String,
        name: String,
        input: Value,
        parent: Option<String>,
    },
    ToolResult {
        call: String,
        output: String,
        is_error: bool,
        files: Vec<FileChange>,
        command: Option<CommandRun>,
    },
    /// A tool ended. `prompt` is the agent's own id of the request that caused it, the
    /// same for every tool of that request; the engine ties it to quill's request id.
    ToolDone {
        call: String,
        prompt: Option<String>,
    },
    PermissionRequest {
        id: String,
        call: Option<String>,
        tool: String,
        input: Value,
        title: Option<String>,
        /// The person must answer something, not only allow or deny (a question).
        interactive: bool,
        suggestions: Vec<Suggestion>,
    },
    ModeChanged {
        mode: String,
    },
    TurnFinished {
        ok: bool,
        interrupted: bool,
        usage: Usage,
        error: Option<String>,
    },
    /// The answer to a command quill sent, by the id `encode` gave it.
    Reply {
        id: String,
        ok: bool,
        body: Value,
    },
    /// A line the adapter does not understand, kept whole.
    /// How much of the plan's usage windows is spent.
    Limits(PlanLimits),
    Unknown(Value),
}

/// The answer to a [`AgentEvent::PermissionRequest`].
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Decision {
    Allow,
    /// Allow and remember the chosen suggestions.
    AllowAlways(Vec<Suggestion>),
    Deny {
        message: String,
    },
    /// Allow with the person's answers to an interactive request.
    Answer(Value),
}

/// Something quill asks of the agent.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AgentCommand {
    Start,
    Prompt {
        request: String,
        text: String,
        priority: Option<String>,
    },
    Interrupt,
    Decide {
        id: String,
        decision: Decision,
    },
    SetMode(String),
    SetModel(String),
    SetEffort(String),
    /// Restore files to before `request`; `dry_run` only reports what would change.
    Rewind {
        request: String,
        dry_run: bool,
    },
}

/// What one line from the agent became.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Decoded {
    pub events: Vec<AgentEvent>,
    /// Lines to write back at once, without the engine deciding (e.g. hook acknowledgements).
    pub replies: Vec<String>,
}

/// One coding-agent CLI.
pub trait AgentAdapter: Send {
    fn id(&self) -> AgentId;
    fn launch(&self, spec: &SessionSpec) -> LaunchPlan;
    /// Lines for `command`. Commands the agent answers get an id, reported back as [`AgentEvent::Reply`].
    fn encode(&mut self, command: AgentCommand) -> Vec<String>;
    fn decode(&mut self, line: &str) -> Decoded;
}
