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
    /// Keep `resume`'s conversation only up to this message of it, by the agent's own id.
    pub resume_at: Option<String>,
    /// The person allowed the agent's modes that skip every approval.
    pub allow_unsafe: bool,
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
    /// Modes that skip every approval, offered only once the person allows them.
    #[serde(default)]
    pub unsafe_modes: Vec<String>,
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
    /// The agent tells its MCP servers' state on request, and reconnects and turns them on
    /// and off ([`AgentCommand::McpStatus`]).
    #[serde(default)]
    pub mcp: bool,
}

/// One MCP server a session's agent loaded, as the agent reports it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct McpServer {
    /// The name the agent knows it by, which reconnecting and toggling it take.
    pub name: String,
    /// The name to show, without what the agent prefixes to tell servers apart.
    pub label: String,
    pub status: McpStatus,
    pub source: McpSource,
    /// The plugin that ships it, for [`McpSource::Plugin`].
    pub plugin: Option<String>,
    /// Why it failed, when it did.
    pub error: Option<String>,
    /// Its tools' names, once connected.
    pub tools: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum McpStatus {
    Connected,
    Pending,
    NeedsAuth,
    Failed,
    Disabled,
}

/// A plugin installed for the agent, as its program lists it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Plugin {
    /// The agent's own id of it, which switching it on and off takes.
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    /// Where it is installed, in the agent's words (`user`, `project`…).
    pub scope: String,
    pub enabled: bool,
}

/// A skill the agent offers, as it lists them.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Where it comes from, in the agent's words (`user`, `plugin`…).
    pub source: String,
    /// Whether the agent offers it at all; one that is off is listed and never used.
    pub enabled: bool,
}

/// Where a server's definition came from: the person's own settings, the project's, the
/// person's for this project only, the organization's, a plugin, the person's account, the
/// app running the agent (quill's own tools), or somewhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum McpSource {
    User,
    Project,
    Local,
    Managed,
    Plugin,
    Account,
    App,
    Other,
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

/// How the agent's context is spent, as it reports it on request.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContextUsage {
    /// Tokens in use and the context's size.
    pub used: u64,
    pub max: u64,
    /// What fills it, in the agent's own words, each with its share.
    pub categories: Vec<ContextCategory>,
    /// Lists that make up some categories (tools of each server, memory files…), each by
    /// name with its entries' tokens.
    pub groups: Vec<(String, Vec<(String, u64)>)>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContextCategory {
    pub name: String,
    pub tokens: u64,
    pub kind: ContextKind,
}

/// What a category's tokens are: in the context now, loaded only when needed, kept free
/// for the agent's own compaction, or not used yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContextKind {
    Used,
    Deferred,
    Buffer,
    Free,
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
        /// The agent's own id of the message: a fork can start from it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// When it came, in seconds since 1970; the engine sets it as it keeps it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<u64>,
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
    /// How much of the plan's usage windows is spent.
    Limits(PlanLimits),
    /// The answer to [`AgentCommand::ContextUsage`].
    ContextUsage(ContextUsage),
    /// The answer to [`AgentCommand::McpStatus`]: every server the session loaded.
    McpServers(Vec<McpServer>),
    /// The answer to [`AgentCommand::Skills`].
    Skills(Vec<Skill>),
    /// The agent calls one of quill's own tools, the ones that start and propose other
    /// sessions ([`HOST_TOOLS`]). The engine answers with [`AgentCommand::HostToolResult`].
    HostTool {
        id: String,
        name: String,
        input: Value,
    },
    /// A line the adapter does not understand, kept whole.
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
    /// Ask how the context is spent, answered by [`AgentEvent::ContextUsage`]; an agent
    /// that cannot tell sends nothing.
    ContextUsage,
    /// Ask for the MCP servers' state, answered by [`AgentEvent::McpServers`].
    McpStatus,
    /// Connect a server that failed or has not connected again, by its name.
    McpReconnect(String),
    /// Turn a server on or off, by its name.
    McpToggle {
        name: String,
        enabled: bool,
    },
    /// Ask for the skills the agent offers, answered by [`AgentEvent::Skills`].
    Skills,
    /// Load the plugins again, after one was switched on or off.
    ReloadPlugins,
    /// The answer to an [`AgentEvent::HostTool`] call.
    HostToolResult {
        id: String,
        content: Vec<ToolContent>,
        is_error: bool,
    },
}

/// One block of a host tool's answer: words, or a picture (a page's screenshot).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ToolContent {
    Text(String),
    Image {
        /// "image/jpeg", "image/png".
        mime: String,
        /// The picture, base64.
        data: String,
    },
}

impl ToolContent {
    pub fn text(words: impl Into<String>) -> Vec<Self> {
        vec![Self::Text(words.into())]
    }
}

/// quill's own tools, which an adapter offers its agent where the agent can take tools from
/// its host: a tool's name, what it is for, and the JSON schema of its input.
pub const HOST_TOOLS: [(&str, &str, &str); 3] = [
    (
        "start_session",
        "Start a separate session for a task of its own while you stay with the user, or propose one. `initiation` says who wants it: \"user_asked\" (the user asked in words for separate, parallel or background work) starts it now; \"own_initiative\" (follow-up work you noticed or that your own work turned up, which the user has not asked for) shows the user a card, and nothing runs until they start it there. Both need `prompt`: the brief, standing alone (the task, the files or area, what done looks like). context \"fresh\" (default) gives the new session your `prompt` and `background` only, so `background` is required: the goal, decisions and constraints so far, for someone who has not seen this conversation; \"fork\" gives it this whole conversation and your `prompt`. A session you start reports back to you when its turn ends. A session is for a task of its own that ends in something of its own; searching, independent parts of one change, and checking what you built stay here or go to a subagent. Start nothing the user did not ask for.",
        r#"{"type":"object","properties":{"initiation":{"type":"string","enum":["user_asked","own_initiative"]},"title":{"type":"string","description":"Under 60 characters; names the task on its row or card."},"prompt":{"type":"string","description":"The brief: the task, the files or area, what done looks like."},"context":{"type":"string","enum":["fresh","fork"]},"background":{"type":"string","description":"With fresh: the goal, decisions and constraints so far."},"proposal_id":{"type":"string","description":"Start a proposal the user told you to start, exactly as proposed: its task id and nothing else."}}}"#,
    ),
    (
        "dismiss_task",
        "Withdraw a proposal you made with start_session (initiation \"own_initiative\") that the user has not started, when it went stale: the work was done here after all, it turned out unnecessary, or you proposed a better one.",
        r#"{"type":"object","properties":{"task_id":{"type":"string"}},"required":["task_id"]}"#,
    ),
    (
        "send_to_session",
        "Pass a message from the user to a session you started, when it is about that session's task: a change of plan, a detail, an answer it waits for. The session gets `message` as from the user and carries on there; tell the user in a line that you passed it on. `session` is its title as you started it.",
        r#"{"type":"object","properties":{"session":{"type":"string","description":"The session's title, as you started it."},"message":{"type":"string","description":"What it should know or do, standing alone."}},"required":["session","message"]}"#,
    ),
];

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
    /// The agent's name for people, as Settings lists it.
    fn name(&self) -> &'static str;
    /// The versions of the agent's program quill was tested with.
    fn tested(&self) -> &'static [&'static str];
    fn launch(&self, spec: &SessionSpec) -> LaunchPlan;
    /// Where, in a session's folder, the agent keeps the dev servers `preview_start` runs
    /// ([`LaunchConfig`]); `None` when it keeps none.
    fn launch_file(&self) -> Option<&'static str> {
        None
    }
    /// The arguments that make the agent's program list its installed plugins, read by
    /// [`plugins`](Self::plugins); `None` when it has none.
    fn plugins_query(&self) -> Option<Vec<String>> {
        None
    }
    /// The plugins in what [`plugins_query`](Self::plugins_query) printed.
    fn plugins(&self, _stdout: &str) -> Vec<Plugin> {
        vec![]
    }
    /// The arguments that switch `plugin` on or off where it is installed.
    fn plugin_switch(&self, _plugin: &Plugin, _enabled: bool) -> Option<Vec<String>> {
        None
    }
    /// Lines for `command`. Commands the agent answers get an id, reported back as [`AgentEvent::Reply`].
    fn encode(&mut self, command: AgentCommand) -> Vec<String>;
    fn decode(&mut self, line: &str) -> Decoded;
}

/// The built-in browser's tools (docs/browser.md §4), served like [`HOST_TOOLS`] by a server of
/// their own, `browser`. Their meaning follows Claude Desktop 3.6556.2's browser tools (ficha
///; the words are quill's.
/// One dev server of a project's launch file (Claude Desktop's format, notes):
/// `{"version": "0.0.1", "configurations": [{"name", "runtimeExecutable", "runtimeArgs",
/// "port", "url"?}]}`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchConfig {
    pub name: String,
    pub runtime_executable: String,
    #[serde(default)]
    pub runtime_args: Vec<String>,
    pub port: u16,
    /// Where the preview opens instead of http://localhost:<port>.
    #[serde(default)]
    pub url: Option<String>,
}

pub const BROWSER_TOOLS: [(&str, &str, &str); 19] = [
    (
        "navigate",
        "Open a URL in a tab of the built-in browser, which the user sees beside the conversation, or go \"back\" or \"forward\" in its history. Waits for the page to load. Local addresses (localhost, 127.0.0.1) are always allowed; other sites may need the user's permission.",
        r#"{"type":"object","properties":{"url":{"type":"string","description":"The address to open, or \"back\" / \"forward\"."},"tabId":{"type":"integer","description":"The tab to use; the one on screen when left out."}},"required":["url"]}"#,
    ),
    (
        "read_page",
        "The page as a tree of its elements, one per line: role, name, and a ref (ref_N) that other tools take to act on it. filter \"interactive\" keeps only the controls visible on screen. Start from ref_id to read part of a page.",
        r#"{"type":"object","properties":{"filter":{"type":"string","enum":["all","interactive"]},"ref_id":{"type":"string"},"depth":{"type":"integer","description":"How deep to go, 15 when left out."},"max_chars":{"type":"integer","description":"50000 when left out."},"tabId":{"type":"integer"}}}"#,
    ),
    (
        "find",
        "Look for elements whose role or name contains the words given, with the refs to act on them; at most 20.",
        r#"{"type":"object","properties":{"query":{"type":"string"},"tabId":{"type":"integer"}},"required":["query"]}"#,
    ),
    (
        "get_page_text",
        "The page's readable text: its main content when it has one, else the whole body.",
        r#"{"type":"object","properties":{"tabId":{"type":"integer"}}}"#,
    ),
    (
        "form_input",
        "Set the value of a form field by its ref: the text of an input or text area, the option of a select (by value or label), whether a checkbox or radio is checked (true/false).",
        r#"{"type":"object","properties":{"ref":{"type":"string"},"value":{"type":["string","boolean","number"]},"tabId":{"type":"integer"}},"required":["ref","value"]}"#,
    ),
    (
        "computer",
        "Look at and use the page as the user would. Actions: screenshot (a picture of the tab, at most 800 px wide; coordinates in later actions are in its pixels), left_click, right_click, double_click, triple_click (at coordinate, or at an element's ref), type (text into what has the focus), key (keys such as \"Enter\", \"Tab\", \"ctrl+a\"), scroll (scroll_direction up/down/left/right and scroll_amount ticks at coordinate), scroll_to (ref), hover, left_click_drag (from start_coordinate to coordinate), wait (duration seconds, at most 10).",
        r#"{"type":"object","properties":{"action":{"type":"string","enum":["screenshot","left_click","right_click","double_click","triple_click","type","key","scroll","scroll_to","hover","left_click_drag","wait"]},"coordinate":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"start_coordinate":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"ref":{"type":"string"},"text":{"type":"string"},"scroll_direction":{"type":"string","enum":["up","down","left","right"]},"scroll_amount":{"type":"integer"},"duration":{"type":"number"},"tabId":{"type":"integer"}},"required":["action"]}"#,
    ),
    (
        "javascript_tool",
        "Run JavaScript in the page and get its value (awaited when it is a promise). The last expression is the value.",
        r#"{"type":"object","properties":{"text":{"type":"string","description":"The script."},"tabId":{"type":"integer"}},"required":["text"]}"#,
    ),
    (
        "read_console_messages",
        "What the page logged to its console since it opened, with uncaught errors; pattern keeps the lines that match it, clear forgets what was read.",
        r#"{"type":"object","properties":{"pattern":{"type":"string"},"only_errors":{"type":"boolean"},"clear":{"type":"boolean"},"tabId":{"type":"integer"}}}"#,
    ),
    (
        "read_network_requests",
        "The requests the page made, with their method and status; url_pattern keeps those whose address contains it, clear forgets what was read.",
        r#"{"type":"object","properties":{"url_pattern":{"type":"string"},"clear":{"type":"boolean"},"tabId":{"type":"integer"}}}"#,
    ),
    (
        "resize_window",
        "Size the tab's viewport, to check a layout at another width (a phone is 375 x 812, a tablet 768 x 1024). Width 0 gives the pane's size back.",
        r#"{"type":"object","properties":{"width":{"type":"integer"},"height":{"type":"integer"},"tabId":{"type":"integer"}},"required":["width","height"]}"#,
    ),
    (
        "tabs_context",
        "The browser's tabs: their ids, titles and addresses, and which is on screen.",
        r#"{"type":"object","properties":{}}"#,
    ),
    (
        "tabs_create",
        "Open a new tab, at url or blank, and put it on screen.",
        r#"{"type":"object","properties":{"url":{"type":"string"}}}"#,
    ),
    (
        "tabs_select",
        "Put a tab on screen.",
        r#"{"type":"object","properties":{"tabId":{"type":"integer"}},"required":["tabId"]}"#,
    ),
    (
        "tabs_close",
        "Close a tab.",
        r#"{"type":"object","properties":{"tabId":{"type":"integer"}},"required":["tabId"]}"#,
    ),
    (
        "preview_start",
        "Start a dev server of this project by its name in the project's launch file and open it in a tab of the built-in browser, or open url in a tab. Reuses the server if it already runs; the user approves a server the first time. Use this instead of Bash to run dev servers.",
        r#"{"type":"object","properties":{"name":{"type":"string","description":"The server's name in the launch file."},"url":{"type":"string","description":"An address to open instead of starting a server."}}}"#,
    ),
    (
        "preview_stop",
        "Stop a dev server started with preview_start.",
        r#"{"type":"object","properties":{"serverId":{"type":"string"}},"required":["serverId"]}"#,
    ),
    (
        "preview_list",
        "The dev servers started with preview_start: their ids, names, addresses and whether they run.",
        r#"{"type":"object","properties":{}}"#,
    ),
    (
        "preview_logs",
        "A dev server's output, its standard output and errors: level \"error\" keeps the error lines, search the lines that contain it.",
        r#"{"type":"object","properties":{"serverId":{"type":"string"},"level":{"type":"string","enum":["all","error"]},"search":{"type":"string"}},"required":["serverId"]}"#,
    ),
    (
        "browser_batch",
        "Run up to 25 of these tools one after another, stopping at the first that fails: a list of {\"tool\": name, \"input\": {...}}. For steps whose outcome you do not need to see in between.",
        r#"{"type":"object","properties":{"actions":{"type":"array","maxItems":25,"items":{"type":"object","properties":{"tool":{"type":"string"},"input":{"type":"object"}},"required":["tool"]}}},"required":["actions"]}"#,
    ),
];

/// What an agent is told of the built-in browser, in words of quill's own.
pub const BROWSER_GUIDE: &str = "# Built-in browser\n\nYou have a browser built into this app, with the browser tools (navigate, read_page, find, get_page_text, form_input, computer, javascript_tool, read_console_messages, read_network_requests, tabs_*). The user sees its tab beside the conversation and can use it too. Use it to check what you build for the web: start the dev server, open it, read the console and the page, act on it, and take a screenshot to show the result instead of asking the user to look. Prefer read_page and find to pick elements by ref; use screenshots to see layout. Local addresses are always allowed; other sites may ask the user first. Do not use it for what a plain fetch answers.";
