//! quill's engine: one Tokio actor per session, owning its agent's process and adapter
//! (spec.md D6, engine.md §1). The UI sends [`Intent`]s with [`Engine::send`]; the engine
//! reports [`Event`]s through the sink given to [`Engine::start`]. Nothing here blocks.

use quill_agent::{AgentAdapter, AgentCommand, AgentEvent, SessionSpec};
use quill_model::{
    Event, Intent, NewSession, ProposalAnswer, ProposalOutcome, RequestId, SessionId, SessionStatus,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

mod bridge;
mod servers;

pub use quill_browser::{native, Frame, Frames, Native};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// Where events go; the app wakes its window from here.
pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// Site permissions the browser waits on: by card, the site and where the answer goes.
/// Questions quill itself asks on a permission card: by card, what for (a site, whose
/// "Always" joins Settings, or a dev server) and where the answer goes.
type SiteWaits = Arc<
    Mutex<
        HashMap<
            String,
            (
                Option<String>,
                tokio::sync::oneshot::Sender<quill_browser::Grant>,
            ),
        >,
    >,
>;

/// Ask the person on a permission card of quill's own; `site` is the site a yes "Always"
/// keeps, if it is one.
pub(crate) type Ask = Arc<
    dyn Fn(
            SessionId,
            serde_json::Value,
            Option<String>,
        ) -> tokio::sync::oneshot::Receiver<quill_browser::Grant>
        + Send
        + Sync,
>;

/// The agents quill has adapters for, as Settings lists them.
const AGENTS: [&str; 1] = ["claude"];

/// The adapter for an agent id; `None` for agents quill does not have yet.
fn adapter(agent: &str) -> Option<Box<dyn AgentAdapter>> {
    match agent {
        "claude" => Some(Box::new(quill_agent_claude::Claude::new())),
        _ => None,
    }
}

pub struct Engine {
    intents: UnboundedSender<Intent>,
    _runtime: tokio::runtime::Runtime,
}

/// How an engine runs.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// Replaces an agent's executable (tests run a fake agent this way).
    pub programs: HashMap<String, String>,
    /// Where sessions are kept; `None` keeps them in memory only.
    pub database: Option<PathBuf>,
    /// Where the shadow repositories of session folders live; `None` attributes no files.
    pub snapshots: Option<PathBuf>,
    /// Where the person's settings are kept; `None` keeps them in memory only.
    pub settings: Option<PathBuf>,
    /// The built-in browser's Chromium profile; `None` puts one in the temporary folder.
    pub browser: Option<PathBuf>,
    /// Where the browser leaves its pages' frames for the window to present.
    pub frames: quill_browser::Frames,
    /// Chromium inside the window, when the host started it (`Native::start`).
    pub native: Option<quill_browser::Native>,
}

impl Config {
    /// Sessions kept in `$XDG_DATA_HOME/quill/quill.db` (`~/.local/share/quill` without it).
    pub fn default_database() -> Self {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(|| ".".into());
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| ".".into());
        Self {
            database: Some(data.join("quill/quill.db")),
            snapshots: Some(data.join("quill/snapshots")),
            settings: Some(config.join("quill/settings.json")),
            browser: Some(data.join("quill/browser")),
            ..Self::default()
        }
    }
}

impl Engine {
    pub fn start(sink: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self::start_with(sink, Config::default_database())
    }

    pub fn start_with(sink: impl Fn(Event) + Send + Sync + 'static, config: Config) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("quill-engine")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (intents, rx) = unbounded_channel();
        runtime.spawn(route(rx, Arc::new(sink), config));
        Self {
            intents,
            _runtime: runtime,
        }
    }

    pub fn send(&self, intent: Intent) {
        let _ = self.intents.send(intent);
    }
}

/// Opens the record, falling back to memory so a broken file never stops quill.
fn open_store(config: &Config, sink: &Sink) -> quill_store::Store {
    let opened = match &config.database {
        Some(path) => quill_store::Store::open(path),
        None => quill_store::Store::in_memory(),
    };
    opened.unwrap_or_else(|error| {
        sink(Event::Status {
            session: 0,
            status: SessionStatus::Failed(format!("sessions are not saved: {error}")),
        });
        quill_store::Store::in_memory().expect("in-memory store")
    })
}

/// What actors tell the router, apart from the events everyone sees.
enum Internal {
    /// The agent called one of quill's own tools.
    HostTool {
        session: SessionId,
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// A browser tool the agent called finished.
    HostAnswered {
        session: SessionId,
        id: String,
        content: Vec<quill_agent::ToolContent>,
        is_error: bool,
    },
    /// A turn began or the session stopped working.
    Busy { session: SessionId, busy: bool },
    /// A turn ended; `text` is the last thing the agent said in it.
    TurnEnded { session: SessionId, text: String },
}

/// Hands each intent to its session's actor, creating sessions as asked. Sessions kept
/// from earlier runs come back first, ready to continue their conversation.
async fn route(mut rx: UnboundedReceiver<Intent>, ui: Sink, config: Config) {
    let (internal, mut internal_rx) = unbounded_channel();
    let mut router = Router::new(ui, config, internal);
    let path = router.config.settings.clone();
    router.settings = tokio::task::spawn_blocking(move || read_settings(path.as_deref()))
        .await
        .unwrap_or_default();
    router.restore();
    router.announce_settings();
    loop {
        tokio::select! {
            intent = rx.recv() => match intent {
                Some(intent) => router.intent(intent).await,
                None => break,
            },
            Some(note) = internal_rx.recv() => router.internal(note).await,
        }
    }
}

/// The engine's own state: each session's actor and what quill knows of how sessions
/// relate (which started which, what each proposed, what waits to be reported).
struct Router {
    store: Arc<Mutex<quill_store::Store>>,
    ui: Sink,
    /// Every event is written before the UI sees it (spec.md D5).
    sink: Sink,
    config: Config,
    internal: UnboundedSender<Internal>,
    sessions: HashMap<SessionId, UnboundedSender<Input>>,
    /// Each session's folder, for what is uncommitted in it.
    folders: HashMap<SessionId, std::path::PathBuf>,
    /// How each session started, which a fork of it or a session it starts starts as too.
    started: HashMap<SessionId, NewSession>,
    /// Each session's own id at its agent, which a fork continues from.
    agent_ids: Arc<Mutex<HashMap<SessionId, String>>>,
    next: SessionId,
    /// The session that started each one, which it reports to.
    parents: HashMap<SessionId, SessionId>,
    /// A started session's name, for its reports.
    titles: HashMap<SessionId, String>,
    /// Proposals nobody answered yet, by the session that made them and their id.
    proposals: HashMap<(SessionId, String), quill_model::Proposal>,
    /// What started sessions said, waiting for their parent to be free to hear it.
    reports: HashMap<SessionId, Vec<quill_model::Report>>,
    /// Sessions in the middle of a turn.
    busy: std::collections::HashSet<SessionId>,
    /// The built-in browser, shared by every session (each in a context of its own).
    browser: quill_browser::Browser,
    /// Site permissions the browser waits on, by the card's id: the site and where the
    /// answer goes.
    site_waits: SiteWaits,
    /// Asks the person on a card of quill's own.
    ask: Ask,
    /// The dev servers agents started with preview_start.
    servers: Arc<servers::Servers>,
    /// What the agents' `$BROWSER` hands back; `None` when the person has a `BROWSER` of
    /// their own or there is no private folder for it.
    bridge: Option<bridge::Bridge>,
    /// The reply being read aloud: what stops it, and the task that ends once it stopped.
    reading: Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    )>,
    settings: quill_model::Settings,
    /// What keeps the computer from idle sleep while an agent works.
    awake: Option<Child>,
}

impl Router {
    fn new(ui: Sink, config: Config, internal: UnboundedSender<Internal>) -> Self {
        let store = Arc::new(Mutex::new(open_store(&config, &ui)));
        let agent_ids: Arc<Mutex<HashMap<SessionId, String>>> = Arc::default();
        let sink: Sink = {
            let (store, ui, agent_ids) = (store.clone(), ui.clone(), agent_ids.clone());
            Arc::new(move |event: Event| {
                if let Event::Agent {
                    session,
                    event: AgentEvent::Session { id },
                    ..
                } = &event
                {
                    agent_ids.lock().unwrap().insert(*session, id.clone());
                }
                if let Err(error) = store.lock().unwrap().append(event.session(), &event) {
                    ui(Event::Status {
                        session: event.session(),
                        status: SessionStatus::Failed(format!("not saved: {error}")),
                    });
                }
                ui(event);
            })
        };
        let site_waits: SiteWaits = Arc::default();
        // What quill asks itself (a site, a dev server) is a card, as a permission is.
        let ask: Ask = {
            let (sink, ui, waits) = (sink.clone(), ui.clone(), site_waits.clone());
            Arc::new(move |session, input, site| {
                let (answer, heard) = tokio::sync::oneshot::channel();
                let id = format!("site-{}", uuid::Uuid::new_v4().simple());
                waits.lock().unwrap().insert(id.clone(), (site, answer));
                sink(Event::Agent {
                    session,
                    request: None,
                    event: AgentEvent::PermissionRequest {
                        id,
                        call: None,
                        tool: quill_model::SITE_PERMISSION.into(),
                        input,
                        title: None,
                        interactive: false,
                        suggestions: vec![],
                    },
                });
                ui(Event::Status {
                    session,
                    status: SessionStatus::Awaiting,
                });
                heard
            })
        };
        let permit: quill_browser::Permit = {
            let ask = ask.clone();
            Arc::new(move |session, site: quill_browser::SiteAsk| {
                let input = serde_json::json!({
                    "kind": "site", "site": site.host, "url": site.url, "opening": site.opening,
                });
                ask(session, input, Some(site.host))
            })
        };
        let browser = {
            let (ui, sink) = (ui.clone(), sink.clone());
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| ".".into());
            quill_browser::Browser::new(
                quill_browser::Options {
                    program: None,
                    profile: config.browser.clone().unwrap_or_else(|| {
                        std::env::temp_dir().join(format!("quill-browser-{}", std::process::id()))
                    }),
                    downloads: home.join("Downloads"),
                    frames: config.frames.clone(),
                    permit,
                    native: config.native.clone(),
                },
                // What the browser shows is the UI's only, but its tabs' addresses, which
                // are kept to open again.
                Arc::new(move |session, update| {
                    let keep = matches!(update, quill_model::BrowserUpdate::Kept(_));
                    let event = Event::Browser { session, update };
                    if keep {
                        sink(event)
                    } else {
                        ui(event)
                    }
                }),
            )
        };
        Self {
            store,
            ui,
            sink,
            config,
            internal,
            ask,
            servers: Arc::default(),
            bridge: bridge::Bridge::start(browser.clone()),
            browser,
            site_waits,
            sessions: HashMap::new(),
            folders: HashMap::new(),
            started: HashMap::new(),
            agent_ids,
            next: 0,
            parents: HashMap::new(),
            titles: HashMap::new(),
            proposals: HashMap::new(),
            reports: HashMap::new(),
            busy: Default::default(),
            reading: None,
            settings: Default::default(),
            awake: None,
        }
    }

    /// The settings in force and the agents quill can run, for the UI.
    fn announce_settings(&self) {
        self.browser
            .allow_sites(self.settings.browser_sites.clone());
        self.browser
            .use_program(self.settings.browser_program.clone().map(PathBuf::from));
        let agents = AGENTS
            .iter()
            .filter_map(|id| adapter(id))
            .map(|a| quill_model::AgentInfo {
                id: a.id().to_owned(),
                name: a.name().to_owned(),
                tested: a.tested().iter().map(|v| (*v).to_owned()).collect(),
            })
            .collect();
        (self.ui)(Event::Settings {
            settings: self.settings.clone(),
            agents,
        });
    }

    /// The program an agent runs as: the tests' stand-in, the person's choice, or its own.
    fn program(&self, agent: &str) -> Option<String> {
        self.config
            .programs
            .get(agent)
            .cloned()
            .or_else(|| self.settings.agent(agent).program)
    }

    /// Hold the computer awake while an agent works and the person wants it, as Desktop's
    /// "Keep computer awake while Claude works": idle sleep is held
    /// off, the display may still turn off and the lid still sleeps (logind ignores
    /// inhibitors for the lid by default).
    fn hold_awake(&mut self) {
        let want = self.settings.keep_awake && !self.busy.is_empty();
        match (want, self.awake.is_some()) {
            (true, false) => {
                self.awake = Command::new("systemd-inhibit")
                    .args([
                        "--what=sleep",
                        "--who=quill",
                        "--why=An agent is working",
                        "sleep",
                        "infinity",
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .ok();
            }
            (false, true) => self.awake = None,
            _ => {}
        }
    }

    fn restore(&mut self) {
        let kept = self.store.lock().unwrap().load().unwrap_or_default();
        for (session, events) in kept {
            self.next = self.next.max(session);
            let Some(restored) = Restored::from(&events) else {
                continue;
            };
            self.folders.insert(session, restored.new.cwd.clone());
            self.started.insert(session, restored.new.clone());
            if let Some(id) = &restored.resume {
                self.agent_ids.lock().unwrap().insert(session, id.clone());
            }
            if let Some(parent) = restored.parent {
                self.parents.insert(session, parent);
            }
            if let Some(title) = restored.title.clone() {
                self.titles.insert(session, title);
            }
            self.browser
                .remember(session, restored.browser_tabs.clone());
            for proposal in restored.proposals.clone() {
                self.proposals
                    .insert((session, proposal.id.clone()), proposal);
            }
            for event in events {
                (self.ui)(event);
            }
            let Some(adapter) = adapter(&restored.new.agent) else {
                continue;
            };
            let (tx, rx) = unbounded_channel();
            let program = self.program(&restored.new.agent);
            let allow_unsafe = self.settings.agent(&restored.new.agent).allow_unsafe;
            let mut actor = Actor::new(
                session,
                restored.new,
                adapter,
                program,
                tx.clone(),
                self.sink.clone(),
            );
            actor.spec.resume = restored.resume;
            actor.spec.allow_unsafe = allow_unsafe;
            actor.env = self
                .bridge
                .as_ref()
                .map(|b| b.env(session))
                .unwrap_or_default();
            // A fork that has said nothing yet still continues where it was forked from.
            if let (None, Some((from, at))) = (&actor.spec.resume, restored.forked) {
                let mut ids = self.agent_ids.lock().unwrap();
                actor.spec.resume = ids.get(&from).cloned();
                actor.spec.fork = actor.spec.resume.is_some();
                actor.spec.resume_at = at.filter(|_| actor.spec.fork);
                // Until it has its own, a fork of it continues the same conversation.
                if let Some(id) = &actor.spec.resume {
                    ids.insert(session, id.clone());
                }
            }
            actor.before = restored.before;
            actor.shadow_root = self.config.snapshots.clone();
            actor.route = Some(self.internal.clone());
            tokio::spawn(actor.run(rx));
            self.sessions.insert(session, tx);
            (self.ui)(Event::Status {
                session,
                status: SessionStatus::Exited,
            });
        }
    }

    /// Make a session and its actor. `about` are the events that describe it before it does
    /// anything (what it was forked or started from); `fork_of` continues that session's
    /// conversation.
    async fn spawn(
        &mut self,
        new: NewSession,
        about: impl FnOnce(SessionId) -> Vec<Event>,
        fork_of: Option<(SessionId, Option<String>)>,
    ) -> Option<SessionId> {
        self.next += 1;
        let id = self.next;
        let Some(adapter) = adapter(&new.agent) else {
            (self.sink)(Event::Status {
                session: id,
                status: SessionStatus::Failed(format!("no adapter for {}", new.agent)),
            });
            return None;
        };
        (self.sink)(Event::SessionCreated {
            session: id,
            agent: new.agent.clone(),
            cwd: new.cwd.clone(),
        });
        for event in about(id) {
            (self.sink)(event);
        }
        self.folders.insert(id, new.cwd.clone());
        self.started.insert(id, new.clone());
        let cwd = new.cwd.clone();
        let branch = tokio::task::spawn_blocking(move || quill_git::branch(&cwd))
            .await
            .ok()
            .flatten();
        (self.sink)(Event::Environment {
            session: id,
            branch,
        });
        let (tx, rx) = unbounded_channel();
        let program = self.program(&new.agent);
        let allow_unsafe = self.settings.agent(&new.agent).allow_unsafe;
        let mut actor = Actor::new(id, new, adapter, program, tx.clone(), self.sink.clone());
        actor.spec.allow_unsafe = allow_unsafe;
        actor.env = self.bridge.as_ref().map(|b| b.env(id)).unwrap_or_default();
        actor.shadow_root = self.config.snapshots.clone();
        actor.route = Some(self.internal.clone());
        if let Some((from, at)) = fork_of {
            // Without an id of its own yet, the parent has no conversation to continue.
            let mut ids = self.agent_ids.lock().unwrap();
            actor.spec.resume = ids.get(&from).cloned();
            actor.spec.fork = actor.spec.resume.is_some();
            actor.spec.resume_at = at.filter(|_| actor.spec.fork);
            // Until it has its own, a fork of it continues the same conversation.
            if let Some(agent) = &actor.spec.resume {
                ids.insert(id, agent.clone());
            }
        }
        tokio::spawn(actor.run(rx));
        self.sessions.insert(id, tx);
        Some(id)
    }

    fn tell(&self, session: SessionId, input: Input) {
        if let Some(tx) = self.sessions.get(&session) {
            let _ = tx.send(input);
        }
    }

    /// Start a session for `proposal` under `parent`, which it reports back to, and send it
    /// its brief: the prompt, with the background when it starts fresh.
    async fn start_child(
        &mut self,
        parent: SessionId,
        proposal: &quill_model::Proposal,
    ) -> Option<SessionId> {
        let new = self.started.get(&parent)?.clone();
        let title = proposal.title.clone();
        let child = self
            .spawn(
                new,
                |id| {
                    vec![
                        Event::Linked {
                            session: id,
                            parent,
                            side: false,
                        },
                        Event::Renamed {
                            session: id,
                            title: title.clone(),
                        },
                    ]
                },
                proposal.fork.then_some((parent, None)),
            )
            .await?;
        self.parents.insert(child, parent);
        self.titles.insert(child, title);
        let brief = match (&proposal.background, proposal.fork) {
            (Some(background), false) if !background.trim().is_empty() => {
                format!("{}\n\nBackground:\n{background}", proposal.prompt)
            }
            _ => proposal.prompt.clone(),
        };
        self.tell(child, Input::Send(brief));
        Some(child)
    }

    /// Keep `settings` and act on them; the UI hears what is in force either way.
    async fn save_settings(&mut self, settings: quill_model::Settings) {
        let path = self.config.settings.clone();
        let (kept, login) = (settings.clone(), settings.open_at_login);
        let saved = tokio::task::spawn_blocking(move || {
            write_settings(path.as_deref(), &kept)?;
            start_at_login(login)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        if let Err(error) = saved {
            (self.ui)(Event::Status {
                session: 0,
                status: SessionStatus::Failed(format!("settings not saved: {error}")),
            });
            // What is in force stays as it was, and the UI hears it again.
            self.announce_settings();
            return;
        }
        self.settings = settings;
        self.hold_awake();
        self.announce_settings();
    }

    /// The person answered a site permission the browser asked for: the answer is the
    /// browser's, not the agent's. "Always allow" keeps the site in Settings.
    async fn site_decided(
        &mut self,
        session: SessionId,
        permission: String,
        decision: quill_model::Decision,
    ) -> bool {
        let Some((site, answer)) = self.site_waits.lock().unwrap().remove(&permission) else {
            return false;
        };
        let grant = match &decision {
            quill_model::Decision::Allow => quill_browser::Grant::Once,
            quill_model::Decision::AllowAlways(_) => quill_browser::Grant::Always,
            _ => quill_browser::Grant::No,
        };
        (self.sink)(Event::Decided {
            session,
            permission,
            decision,
        });
        (self.ui)(Event::Status {
            session,
            status: SessionStatus::Running,
        });
        if let (quill_browser::Grant::Always, Some(host)) = (grant, site) {
            if !self.settings.browser_sites.contains(&host) {
                let mut settings = self.settings.clone();
                settings.browser_sites.push(host);
                self.save_settings(settings).await;
            }
        }
        let _ = answer.send(grant);
        true
    }

    async fn intent(&mut self, intent: Intent) {
        if let Intent::Decide {
            session,
            permission,
            decision,
        } = &intent
        {
            if self
                .site_decided(*session, permission.clone(), decision.clone())
                .await
            {
                return;
            }
        }
        let (session, input) = match intent {
            Intent::NewSession(new) => {
                self.spawn(new, |_| vec![], None).await;
                return;
            }
            Intent::Send { session, text } => (session, Input::Send(text)),
            Intent::Decide {
                session,
                permission,
                decision,
            } => (
                session,
                Input::Command(AgentCommand::Decide {
                    id: permission,
                    decision,
                }),
            ),
            Intent::Interrupt { session } => (session, Input::Command(AgentCommand::Interrupt)),
            Intent::SetMode { session, mode } => {
                (session, Input::Command(AgentCommand::SetMode(mode)))
            }
            Intent::SetModel { session, model } => {
                (session, Input::Command(AgentCommand::SetModel(model)))
            }
            Intent::SetEffort { session, effort } => {
                (session, Input::Command(AgentCommand::SetEffort(effort)))
            }
            Intent::ContextUsage { session } => {
                (session, Input::Command(AgentCommand::ContextUsage))
            }
            Intent::Rewind { session, request } => (session, Input::Rewind(request)),
            Intent::Rename { session, title } => {
                self.titles.insert(session, title.clone());
                (self.sink)(Event::Renamed { session, title });
                return;
            }
            Intent::Pin { session, pinned } => {
                (self.sink)(Event::Pinned { session, pinned });
                return;
            }
            Intent::Archive { session, archived } => {
                (self.sink)(Event::Archived { session, archived });
                if archived {
                    let browser = self.browser.clone();
                    tokio::spawn(async move { browser.forget(session).await });
                }
                return;
            }
            Intent::Browser { session, op } => {
                self.browser.send(session, op);
                return;
            }
            Intent::MarkUnread { session, unread } => {
                (self.sink)(Event::Unread { session, unread });
                return;
            }
            Intent::Fork { session: from, at } => {
                if let Some(new) = self.started.get(&from).cloned() {
                    let about = at.clone();
                    let fork = self
                        .spawn(
                            new,
                            |id| {
                                vec![Event::Forked {
                                    session: id,
                                    from,
                                    at: about,
                                }]
                            },
                            Some((from, at)),
                        )
                        .await;
                    // Its agent starts with the first message; until then it waits on the person.
                    if let Some(session) = fork {
                        (self.sink)(Event::Status {
                            session,
                            status: SessionStatus::Idle,
                        });
                    }
                }
                return;
            }
            Intent::ReadFile { session, path } => {
                if let Some(cwd) = self.folders.get(&session).cloned() {
                    let sink = self.sink.clone();
                    tokio::spawn(async move {
                        let at = path.clone();
                        let text = tokio::task::spawn_blocking(move || read_text(&cwd, &at))
                            .await
                            .ok()
                            .flatten();
                        sink(Event::FileRead {
                            session,
                            path,
                            text,
                        });
                    });
                }
                return;
            }
            Intent::WriteFile {
                session,
                path,
                text,
            } => {
                if let Some(cwd) = self.folders.get(&session).cloned() {
                    let sink = self.sink.clone();
                    tokio::spawn(async move {
                        let (at, written) = (path.clone(), text.clone());
                        let wrote =
                            tokio::task::spawn_blocking(move || write_text(&cwd, &at, &written))
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()));
                        sink(match wrote {
                            Ok(()) => Event::FileRead {
                                session,
                                path,
                                text: Some(text),
                            },
                            Err(error) => Event::FileWriteFailed {
                                session,
                                path,
                                error,
                            },
                        });
                    });
                }
                return;
            }
            Intent::SaveSettings(settings) => {
                self.save_settings(settings).await;
                return;
            }
            Intent::MeasureStorage => {
                let path = self
                    .config
                    .database
                    .as_ref()
                    .and_then(|db| db.parent())
                    .map(PathBuf::from);
                if let Some(path) = path {
                    let ui = self.ui.clone();
                    tokio::spawn(async move {
                        let at = path.clone();
                        let bytes = tokio::task::spawn_blocking(move || size_of(&at))
                            .await
                            .unwrap_or(0);
                        ui(Event::Storage { path, bytes });
                    });
                }
                return;
            }
            Intent::ReadAloud { session, text } => {
                // One voice at a time: a new reading, or none, ends the one under way first.
                if let Some((stop, done)) = self.reading.take() {
                    let _ = stop.send(());
                    let _ = done.await;
                }
                if let Some(text) = text {
                    let (stop, stopped) = tokio::sync::oneshot::channel();
                    let done = tokio::spawn(read_aloud(session, text, stopped, self.sink.clone()));
                    self.reading = Some((stop, done));
                }
                return;
            }
            Intent::AnswerProposal {
                session,
                task,
                answer,
            } => {
                self.answer(session, task, answer).await;
                return;
            }
            // Desktop's /btw forks the session as a side child (3.6556.2 `startBtwChild`,
            // `forkSession(..., {btw: true})`); it is not a session that reports back.
            Intent::SideChat { session, text } => {
                let Some(new) = self.started.get(&session).cloned() else {
                    return;
                };
                let title: String = text.chars().take(60).collect();
                let side = self
                    .spawn(
                        new,
                        |id| {
                            vec![
                                Event::Linked {
                                    session: id,
                                    parent: session,
                                    side: true,
                                },
                                Event::Renamed { session: id, title },
                            ]
                        },
                        Some((session, None)),
                    )
                    .await;
                if let Some(side) = side {
                    self.tell(side, Input::Send(text));
                }
                return;
            }
            // The agent is stopped first, so nothing it says lands after its record is gone.
            Intent::Delete { session } => {
                self.servers.forget(session);
                let browser = self.browser.clone();
                tokio::spawn(async move { browser.forget(session).await });
                self.folders.remove(&session);
                self.started.remove(&session);
                self.agent_ids.lock().unwrap().remove(&session);
                let (done, stopped) = tokio::sync::oneshot::channel();
                let stopping = self
                    .sessions
                    .remove(&session)
                    .is_some_and(|tx| tx.send(Input::Stop(done)).is_ok());
                let (store, ui) = (self.store.clone(), self.ui.clone());
                tokio::spawn(async move {
                    if stopping {
                        let _ = stopped.await;
                    }
                    match store.lock().unwrap().delete(session) {
                        Ok(()) => ui(Event::Deleted { session }),
                        Err(error) => ui(Event::Status {
                            session,
                            status: SessionStatus::Failed(format!("not deleted: {error}")),
                        }),
                    }
                });
                return;
            }
            Intent::WorkingTree { session } => {
                if let Some(cwd) = self.folders.get(&session).cloned() {
                    let sink = self.sink.clone();
                    tokio::spawn(async move {
                        let read = tokio::task::spawn_blocking(move || {
                            let files = quill_git::working_changes(&cwd).unwrap_or_default();
                            (files, quill_git::repo(&cwd))
                        });
                        let (files, repo) = read.await.unwrap_or_default();
                        sink(Event::WorkingTree {
                            session,
                            files,
                            repo,
                        });
                    });
                }
                return;
            }
        };
        self.tell(session, input);
    }

    /// The person answered a proposal on its card.
    async fn answer(&mut self, session: SessionId, task: String, answer: ProposalAnswer) {
        let Some(proposal) = self.proposals.remove(&(session, task.clone())) else {
            return;
        };
        let outcome = match answer {
            ProposalAnswer::Start => match self.start_child(session, &proposal).await {
                Some(child) => ProposalOutcome::Started { session: child },
                None => return,
            },
            ProposalAnswer::Here => {
                self.tell(session, Input::Send(proposal.prompt.clone()));
                ProposalOutcome::Kept
            }
            ProposalAnswer::Dismiss => ProposalOutcome::Dismissed,
        };
        (self.sink)(Event::ProposalAnswered {
            session,
            task,
            outcome,
        });
    }

    async fn internal(&mut self, note: Internal) {
        match note {
            // Dev servers are processes, the engine's own; they open in the browser.
            Internal::HostTool {
                session,
                id,
                name,
                input,
            } if name.starts_with("preview_") => {
                let place = servers::Place {
                    cwd: self.folders.get(&session).cloned(),
                    file: self
                        .started
                        .get(&session)
                        .and_then(|new| adapter(&new.agent))
                        .and_then(|a| a.launch_file()),
                };
                let (servers, browser, ask, internal) = (
                    self.servers.clone(),
                    self.browser.clone(),
                    self.ask.clone(),
                    self.internal.clone(),
                );
                tokio::spawn(async move {
                    let (content, is_error) =
                        servers::tool(&servers, &browser, &ask, session, place, &name, &input)
                            .await;
                    let _ = internal.send(Internal::HostAnswered {
                        session,
                        id,
                        content,
                        is_error,
                    });
                });
            }
            // The browser's tools wait on pages: they run beside the router, which hears
            // their answer back.
            Internal::HostTool {
                session,
                id,
                name,
                input,
            } if quill_agent::BROWSER_TOOLS
                .iter()
                .any(|(tool, ..)| *tool == name) =>
            {
                let (browser, internal) = (self.browser.clone(), self.internal.clone());
                tokio::spawn(async move {
                    let (content, is_error) = browser.tool(session, &name, input).await;
                    let _ = internal.send(Internal::HostAnswered {
                        session,
                        id,
                        content,
                        is_error,
                    });
                });
            }
            Internal::HostTool {
                session,
                id,
                name,
                input,
            } => {
                let (text, is_error) = self.host_tool(session, &name, &input).await;
                self.tell(
                    session,
                    Input::Command(AgentCommand::HostToolResult {
                        id,
                        content: quill_agent::ToolContent::text(text),
                        is_error,
                    }),
                );
            }
            Internal::HostAnswered {
                session,
                id,
                content,
                is_error,
            } => self.tell(
                session,
                Input::Command(AgentCommand::HostToolResult {
                    id,
                    content,
                    is_error,
                }),
            ),
            Internal::Busy { session, busy } => {
                if busy {
                    self.busy.insert(session);
                } else {
                    self.busy.remove(&session);
                    self.flush(session);
                }
                self.hold_awake();
            }
            Internal::TurnEnded { session, text } => {
                self.busy.remove(&session);
                self.hold_awake();
                if let Some(&parent) = self.parents.get(&session) {
                    let title = self.titles.get(&session).cloned().unwrap_or_default();
                    self.reports
                        .entry(parent)
                        .or_default()
                        .push(quill_model::Report {
                            from: session,
                            title,
                            text,
                        });
                    self.flush(parent);
                }
                self.flush(session);
            }
        }
    }

    /// Tell `parent` what the sessions it started said, once it is between turns and none of
    /// them is still working, as Desktop continues a session when its side sessions end
    /// (3.6556.2 `start_session`'s `wake_when_done`).
    fn flush(&mut self, parent: SessionId) {
        let working = self.busy.contains(&parent)
            || self
                .parents
                .iter()
                .any(|(child, p)| *p == parent && self.busy.contains(child));
        if working {
            return;
        }
        if let Some(reports) = self.reports.remove(&parent).filter(|r| !r.is_empty()) {
            self.busy.insert(parent);
            self.tell(parent, Input::Report(reports));
        }
    }

    /// One of quill's own tools, called by `session`'s agent ([`quill_agent::HOST_TOOLS`]):
    /// what to answer it, and whether that is an error.
    async fn host_tool(
        &mut self,
        session: SessionId,
        name: &str,
        input: &serde_json::Value,
    ) -> (String, bool) {
        let field = |key: &str| {
            input[key]
                .as_str()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        match name {
            "start_session" => {
                if let Some(task) = field("proposal_id") {
                    let Some(proposal) = self.proposals.get(&(session, task.clone())).cloned()
                    else {
                        return (
                            "That task is not waiting: the user already started or dismissed it."
                                .into(),
                            true,
                        );
                    };
                    self.answer(session, task, ProposalAnswer::Start).await;
                    return (
                        format!(
                            "Started the session \u{201c}{}\u{201d}. It reports back here when its turn ends.",
                            proposal.title
                        ),
                        false,
                    );
                }
                let (Some(title), Some(prompt)) = (field("title"), field("prompt")) else {
                    return ("start_session needs a title and a prompt.".into(), true);
                };
                let proposal = quill_model::Proposal {
                    id: format!("task-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
                    title,
                    prompt,
                    background: field("background"),
                    fork: field("context").as_deref() == Some("fork"),
                };
                match field("initiation").as_deref() {
                    Some("own_initiative") => {
                        let id = proposal.id.clone();
                        self.proposals.insert((session, id.clone()), proposal.clone());
                        (self.sink)(Event::Proposed { session, proposal });
                        (
                            format!("Proposed as task {id}. The user sees it as a card and starts it there; nothing runs until then. If it goes stale, withdraw it with dismiss_task."),
                            false,
                        )
                    }
                    Some("user_asked") => match self.start_child(session, &proposal).await {
                        Some(_) => (
                            format!(
                                "Started the session \u{201c}{}\u{201d}. It reports back here when its turn ends.",
                                proposal.title
                            ),
                            false,
                        ),
                        None => ("The session could not be started.".into(), true),
                    },
                    _ => (
                        "initiation must be \"user_asked\" or \"own_initiative\".".into(),
                        true,
                    ),
                }
            }
            "dismiss_task" => {
                let Some(task) = field("task_id") else {
                    return ("dismiss_task needs a task_id.".into(), true);
                };
                if self.proposals.remove(&(session, task.clone())).is_none() {
                    return (
                        "That task is not waiting: the user already started or dismissed it."
                            .into(),
                        false,
                    );
                }
                (self.sink)(Event::ProposalAnswered {
                    session,
                    task,
                    outcome: ProposalOutcome::Dismissed,
                });
                ("Withdrawn.".into(), false)
            }
            "send_to_session" => {
                let (Some(title), Some(message)) = (field("session"), field("message")) else {
                    return (
                        "send_to_session needs a session and a message.".into(),
                        true,
                    );
                };
                let mine: Vec<(SessionId, String)> = self
                    .parents
                    .iter()
                    .filter(|(_, parent)| **parent == session)
                    .filter_map(|(child, _)| Some((*child, self.titles.get(child)?.clone())))
                    .collect();
                let Some((child, name)) = mine
                    .iter()
                    .find(|(_, name)| name.eq_ignore_ascii_case(&title))
                    .cloned()
                else {
                    let names: Vec<String> = mine
                        .iter()
                        .map(|(_, n)| format!("\u{201c}{n}\u{201d}"))
                        .collect();
                    return (
                        if names.is_empty() {
                            "You have started no session to pass it to.".into()
                        } else {
                            format!(
                                "No session of yours is called that. Yours: {}.",
                                names.join(", ")
                            )
                        },
                        true,
                    );
                };
                (self.sink)(Event::Forwarded {
                    session: child,
                    from: session,
                });
                self.tell(child, Input::Send(message));
                (format!("Passed on to \u{201c}{name}\u{201d}."), false)
            }
            other => (format!("quill has no tool named {other}."), true),
        }
    }
}

/// What a kept session needs to continue: how it started and the agent's own id for it.
struct Restored {
    new: NewSession,
    resume: Option<String>,
    /// Each attributed request's snapshot from before it ran, for rewinds after a restart.
    before: HashMap<RequestId, String>,
    /// The session that started it, and its name.
    parent: Option<SessionId>,
    title: Option<String>,
    /// What it proposed that nobody answered.
    proposals: Vec<quill_model::Proposal>,
    /// The session it was forked from, and the message the fork starts after.
    forked: Option<(SessionId, Option<String>)>,
    /// Its browser's tabs when quill last ran.
    browser_tabs: Vec<String>,
}

impl Restored {
    fn from(events: &[Event]) -> Option<Self> {
        let Some(Event::SessionCreated { agent, cwd, .. }) = events.first() else {
            return None;
        };
        let mut new = NewSession {
            agent: agent.clone(),
            cwd: cwd.clone(),
            model: None,
            effort: None,
            mode: None,
        };
        let mut resume = None;
        let mut before = HashMap::new();
        let (mut parent, mut title, mut proposals, mut forked) = (None, None, vec![], None);
        let mut browser_tabs = vec![];
        for event in events {
            match event {
                Event::Linked {
                    parent: p,
                    side: false,
                    ..
                } => parent = Some(*p),
                Event::Renamed { title: t, .. } => title = Some(t.clone()),
                Event::Forked { from, at, .. } => forked = Some((*from, at.clone())),
                Event::Proposed { proposal, .. } => proposals.push(proposal.clone()),
                Event::ProposalAnswered { task, .. } => proposals.retain(|p| p.id != *task),
                Event::Agent {
                    event: AgentEvent::Session { id },
                    ..
                } => resume = Some(id.clone()),
                Event::Agent {
                    event: AgentEvent::ModeChanged { mode },
                    ..
                } => new.mode = Some(mode.clone()),
                Event::Changes {
                    request, snapshot, ..
                } => drop(before.insert(request.clone(), snapshot.clone())),
                Event::Browser {
                    update: quill_model::BrowserUpdate::Kept(urls),
                    ..
                } => browser_tabs = urls.clone(),
                _ => {}
            }
        }
        Some(Self {
            new,
            resume,
            before,
            parent,
            title,
            proposals,
            forked,
            browser_tabs,
        })
    }
}

/// What reaches a session's actor: the UI's wishes and its process's output.
enum Input {
    Send(String),
    /// What sessions it started said: sent as one message that begins a turn.
    Report(Vec<quill_model::Report>),
    Rewind(RequestId),
    Command(AgentCommand),
    Line {
        generation: u64,
        line: String,
    },
    Exited {
        generation: u64,
        code: Option<i32>,
    },
    /// End the actor and its agent, then say so.
    Stop(tokio::sync::oneshot::Sender<()>),
}

struct Actor {
    id: SessionId,
    spec: SessionSpec,
    adapter: Box<dyn AgentAdapter>,
    program: Option<String>,
    inbox: UnboundedSender<Input>,
    sink: Sink,
    process: Option<(Child, ChildStdin)>,
    /// Bumped per launch, so lines of a process that already ended are dropped.
    generation: u64,
    /// The request the agent is working on, from its own `Running` reports.
    running: Option<RequestId>,
    /// A prompt was sent and its turn has not finished.
    busy: bool,
    /// Where this folder's shadow repository lives, and the shadow once opened.
    shadow_root: Option<PathBuf>,
    shadow: Option<Arc<quill_git::Shadow>>,
    /// Requests sent and not finished, oldest first: each finished turn belongs to the first.
    sent: std::collections::VecDeque<RequestId>,
    /// The folder when the oldest of `sent` began.
    start: Option<String>,
    /// Each attributed request's snapshot from before it ran.
    before: HashMap<RequestId, String>,
    /// Where it tells the router of tool calls and turns.
    route: Option<UnboundedSender<Internal>>,
    /// The last thing the agent said in the turn under way, for its report.
    last_text: String,
    /// Added to the agent's environment: the `$BROWSER` bridge.
    env: Vec<(String, String)>,
}

impl Actor {
    fn new(
        id: SessionId,
        new: NewSession,
        adapter: Box<dyn AgentAdapter>,
        program: Option<String>,
        inbox: UnboundedSender<Input>,
        sink: Sink,
    ) -> Self {
        let spec = SessionSpec {
            cwd: new.cwd,
            model: new.model,
            effort: new.effort,
            mode: new.mode,
            resume: None,
            fork: false,
            resume_at: None,
            allow_unsafe: false,
        };
        Self {
            id,
            spec,
            adapter,
            program,
            inbox,
            sink,
            process: None,
            generation: 0,
            running: None,
            busy: false,
            shadow_root: None,
            shadow: None,
            sent: Default::default(),
            start: None,
            before: HashMap::new(),
            route: None,
            last_text: String::new(),
            env: Vec::new(),
        }
    }

    fn tell(&self, note: Internal) {
        if let Some(route) = &self.route {
            let _ = route.send(note);
        }
    }

    /// Send `text` as a new request; `reports` makes it the report of started sessions, which
    /// the transcript shows as such rather than as the person's message.
    async fn prompt(&mut self, text: String, reports: Option<Vec<quill_model::Report>>) {
        if let Err(error) = self.ensure_running().await {
            self.status(SessionStatus::Failed(error));
            self.tell(Internal::Busy {
                session: self.id,
                busy: false,
            });
            return;
        }
        let request = uuid::Uuid::new_v4().to_string();
        // The first request in line starts from the folder as it is now; the
        // ones queued behind it start where the turn before them ended.
        if self.sent.is_empty() {
            self.start = self.snapshot().await;
        }
        self.sent.push_back(request.clone());
        (self.sink)(match reports {
            Some(reports) => Event::Reported {
                session: self.id,
                request: request.clone(),
                reports,
            },
            None => Event::Queued {
                session: self.id,
                request: request.clone(),
                text: text.clone(),
            },
        });
        self.write(AgentCommand::Prompt {
            request,
            text,
            priority: None,
        })
        .await;
        self.busy = true;
        self.tell(Internal::Busy {
            session: self.id,
            busy: true,
        });
        self.status(SessionStatus::Running);
    }

    /// The folder as it is now, off the actor's thread; `None` when files are not attributed.
    async fn snapshot(&mut self) -> Option<String> {
        if self.shadow.is_none() {
            let (root, cwd) = (self.shadow_root.clone()?, self.spec.cwd.clone());
            let opened =
                tokio::task::spawn_blocking(move || quill_git::Shadow::open(&root, &cwd)).await;
            self.shadow = Some(Arc::new(opened.ok()?.ok()?));
        }
        let shadow = self.shadow.clone()?;
        tokio::task::spawn_blocking(move || shadow.snapshot().ok())
            .await
            .ok()
            .flatten()
    }

    /// A turn ended: what its request changed in the folder (engine.md §6, spec.md D4).
    async fn attribute(&mut self) {
        let request = self.sent.pop_front();
        let end = self.snapshot().await;
        if let (Some(request), Some(start), Some(end), Some(shadow)) = (
            request,
            self.start.clone(),
            end.clone(),
            self.shadow.clone(),
        ) {
            let (from, to) = (start.clone(), end.clone());
            let files =
                tokio::task::spawn_blocking(move || shadow.changes(&from, &to).unwrap_or_default())
                    .await
                    .unwrap_or_default();
            self.before.insert(request.clone(), start.clone());
            (self.sink)(Event::Changes {
                session: self.id,
                request,
                files,
                snapshot: start,
            });
        }
        self.start = end;
    }

    /// Put the folder back as it was before `request`, unless a turn is running.
    async fn rewind(&mut self, request: RequestId) {
        let (Some(target), false) = (self.before.get(&request).cloned(), self.busy) else {
            return;
        };
        let Some(now) = self.snapshot().await else {
            return;
        };
        let Some(shadow) = self.shadow.clone() else {
            return;
        };
        let restored = tokio::task::spawn_blocking(move || {
            let files = shadow.changes(&now, &target)?;
            shadow.restore(&target).map(|()| (files, target))
        })
        .await;
        match restored {
            Ok(Ok((files, target))) => {
                self.start = Some(target);
                (self.sink)(Event::Rewound {
                    session: self.id,
                    request,
                    files,
                });
            }
            Ok(Err(error)) => self.status(SessionStatus::Failed(format!("rewind failed: {error}"))),
            Err(_) => {}
        }
    }

    fn status(&self, status: SessionStatus) {
        (self.sink)(Event::Status {
            session: self.id,
            status,
        });
    }

    async fn run(mut self, mut rx: UnboundedReceiver<Input>) {
        let mut stopped = None;
        while let Some(input) = rx.recv().await {
            match input {
                Input::Stop(done) => {
                    stopped = Some(done);
                    break;
                }
                Input::Send(text) => self.prompt(text, None).await,
                Input::Report(reports) => {
                    let text = report_text(&reports);
                    self.prompt(text, Some(reports)).await
                }
                Input::Rewind(request) => self.rewind(request).await,
                Input::Command(command) => {
                    let decided = match &command {
                        AgentCommand::Decide { id, decision } => Some(Event::Decided {
                            session: self.id,
                            permission: id.clone(),
                            decision: decision.clone(),
                        }),
                        _ => None,
                    };
                    self.write(command).await;
                    if let Some(event) = decided {
                        (self.sink)(event);
                        self.status(SessionStatus::Running);
                    }
                }
                Input::Line { generation, line } if generation == self.generation => {
                    self.line(&line).await
                }
                Input::Exited { generation, code } if generation == self.generation => {
                    self.process = None;
                    self.running = None;
                    self.busy = false;
                    self.tell(Internal::Busy {
                        session: self.id,
                        busy: false,
                    });
                    // Turns the agent never finished are not attributed.
                    self.sent.clear();
                    self.status(match code {
                        Some(0) | None => SessionStatus::Exited,
                        Some(code) => {
                            SessionStatus::Failed(format!("the agent exited with code {code}"))
                        }
                    });
                }
                Input::Line { .. } | Input::Exited { .. } => {}
            }
        }
        if let Some((mut child, _)) = self.process.take() {
            let _ = child.kill().await;
        }
        if let Some(done) = stopped {
            let _ = done.send(());
        }
    }

    /// Start the agent if it is not running; after an exit it resumes the same conversation.
    async fn ensure_running(&mut self) -> Result<(), String> {
        if self.process.is_some() {
            return Ok(());
        }
        let plan = self.adapter.launch(&self.spec);
        let mut command = Command::new(self.program.as_deref().unwrap_or(&plan.program));
        command
            .args(&plan.args)
            .current_dir(&plan.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (key, value) in &plan.env {
            command.env(key, value);
        }
        for key in &plan.env_remove {
            command.env_remove(key);
        }
        command.envs(self.env.iter().map(|(k, v)| (k, v)));
        let mut child = command
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", plan.program))?;
        self.generation += 1;
        let generation = self.generation;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let stdin = child.stdin.take().expect("piped stdin");
        let inbox = self.inbox.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if inbox.send(Input::Line { generation, line }).is_err() {
                    return;
                }
            }
            let _ = inbox.send(Input::Exited {
                generation,
                code: None,
            });
        });
        // stderr is drained so the agent never blocks on a full pipe.
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
        self.process = Some((child, stdin));
        self.status(SessionStatus::Starting);
        self.write(AgentCommand::Start).await;
        Ok(())
    }

    async fn write(&mut self, command: AgentCommand) {
        let lines = self.adapter.encode(command);
        self.write_lines(lines).await;
    }

    async fn write_lines(&mut self, lines: Vec<String>) {
        let Some((_, stdin)) = self.process.as_mut() else {
            return;
        };
        for line in lines {
            if stdin
                .write_all(format!("{line}\n").as_bytes())
                .await
                .is_err()
            {
                return;
            }
        }
        let _ = stdin.flush().await;
    }

    async fn line(&mut self, line: &str) {
        let decoded = self.adapter.decode(line);
        self.write_lines(decoded.replies).await;
        for event in decoded.events {
            // quill's own tools are the router's to run; the call itself shows as its step.
            if let AgentEvent::HostTool { id, name, input } = event {
                self.tell(Internal::HostTool {
                    session: self.id,
                    id,
                    name,
                    input,
                });
                continue;
            }
            let mut event = event;
            if let AgentEvent::Text {
                text, parent, at, ..
            } = &mut event
            {
                *at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_secs());
                if parent.is_none() {
                    self.last_text = text.clone();
                }
            }
            match &event {
                AgentEvent::Session { id } => {
                    self.spec.resume = Some(id.clone());
                    // A fork has its own id now: what follows continues it, not its parent.
                    self.spec.fork = false;
                    self.spec.resume_at = None;
                }
                AgentEvent::Running { requests } => self.running = requests.last().cloned(),
                // A prompt sent while the agent was starting keeps the session running.
                AgentEvent::Ready { .. } if !self.busy => self.status(SessionStatus::Idle),
                AgentEvent::PermissionRequest { .. } => self.status(SessionStatus::Awaiting),
                _ => {}
            }
            let finished = matches!(event, AgentEvent::TurnFinished { .. });
            (self.sink)(Event::Agent {
                session: self.id,
                request: self.running.clone(),
                event,
            });
            if finished {
                self.attribute().await;
                self.running = None;
                self.busy = false;
                self.status(SessionStatus::Idle);
                let text = std::mem::take(&mut self.last_text);
                self.tell(Internal::TurnEnded {
                    session: self.id,
                    text,
                });
            }
        }
    }
}

/// The message that tells a session what the sessions it started said, in quill's words for
/// what Desktop sends (3.6556.2 `<side-sessions-event>`).
/// The settings kept at `path`; the defaults when there is none or it does not read.
fn read_settings(path: Option<&std::path::Path>) -> quill_model::Settings {
    path.and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Keep `settings` at `path`, whole or not at all: written beside it, then moved over it.
fn write_settings(
    path: Option<&std::path::Path>,
    settings: &quill_model::Settings,
) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    let next = path.with_extension("json.new");
    std::fs::write(&next, json).map_err(|e| e.to_string())?;
    std::fs::rename(&next, path).map_err(|e| e.to_string())
}

/// Start quill when the person logs in, or no longer: the desktop's autostart entry
/// (freedesktop Desktop Application Autostart), written for this program or removed.
fn start_at_login(on: bool) -> Result<(), String> {
    let Some(config) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    else {
        return Ok(());
    };
    let entry = config.join("autostart/quill.desktop");
    if !on {
        return match std::fs::remove_file(&entry) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let program = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(entry.parent().unwrap_or(&config)).map_err(|e| e.to_string())?;
    std::fs::write(
        &entry,
        format!(
            "[Desktop Entry]\nType=Application\nName=quill\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\n",
            program.display()
        ),
    )
    .map_err(|e| e.to_string())
}

/// The bytes the files under `path` take.
fn size_of(path: &std::path::Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| size_of(&entry.path()))
            .sum(),
        Ok(meta) => meta.len(),
        Err(_) => 0,
    }
}

/// The largest file the document pane opens.
const DOCUMENT_LIMIT: u64 = 2 << 20;

/// Where `path` is inside `cwd`, or `None` when it would lead out of it: a document of a
/// session is a file of its own folder.
fn inside(cwd: &std::path::Path, path: &str) -> Option<std::path::PathBuf> {
    let relative = std::path::Path::new(path);
    let plain = relative
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)));
    if !plain {
        return None;
    }
    let root = cwd.canonicalize().ok()?;
    let full = root.join(relative);
    // A link inside the folder may point out of it.
    match full.canonicalize() {
        Ok(real) => real.starts_with(&root).then_some(real),
        Err(_) => full
            .parent()?
            .canonicalize()
            .ok()?
            .starts_with(&root)
            .then_some(full),
    }
}

fn read_text(cwd: &std::path::Path, path: &str) -> Option<String> {
    let file = inside(cwd, path)?;
    if std::fs::metadata(&file).ok()?.len() > DOCUMENT_LIMIT {
        return None;
    }
    String::from_utf8(std::fs::read(file).ok()?).ok()
}

fn write_text(cwd: &std::path::Path, path: &str, text: &str) -> Result<(), String> {
    let file = inside(cwd, path).ok_or("The file is not in the session's folder.")?;
    std::fs::write(file, text).map_err(|e| e.to_string())
}

/// Read `text` with speech-dispatcher's voice until it is done or `stop` fires, saying when it
/// starts and ends. Markdown's marks are left out, so the voice does not read them.
async fn read_aloud(
    session: SessionId,
    text: String,
    stop: tokio::sync::oneshot::Receiver<()>,
    sink: Sink,
) {
    let spoken: String = text
        .chars()
        .filter(|c| !matches!(c, '*' | '`' | '#'))
        .collect();
    sink(Event::Reading {
        session,
        reading: true,
    });
    let voice = Command::new("spd-say")
        .args(["-w", "-m", "none", "--", &spoken])
        .kill_on_drop(true)
        .spawn();
    if let Ok(mut voice) = voice {
        tokio::select! {
            _ = voice.wait() => {}
            _ = stop => {
                // The server goes on with what it was given unless told to cancel it.
                let _ = voice.kill().await;
                let _ = Command::new("spd-say").arg("-C").status().await;
            }
        }
    }
    sink(Event::Reading {
        session,
        reading: false,
    });
}

fn report_text(reports: &[quill_model::Report]) -> String {
    let mut text =
        String::from("<sessions-report>\nSessions you started have finished their turn.\n");
    for report in reports {
        text.push_str(&format!(
            "\n## {} (session {})\n{}\n",
            report.title, report.from, report.text
        ));
    }
    text.push_str("\nIf what the user asked you here has steps these results unblock, carry on with them; otherwise tell the user the outcome in a line or two.\n</sessions-report>");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_kept_whole_and_an_unknown_key_reads_as_defaults() {
        let dir = std::env::temp_dir().join(format!("quill-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        assert_eq!(read_settings(Some(&path)), quill_model::Settings::default());
        let mut settings = quill_model::Settings {
            wide: true,
            ..Default::default()
        };
        settings.agents.insert(
            "claude".into(),
            quill_model::AgentSettings {
                model: Some("haiku".into()),
                ..Default::default()
            },
        );
        write_settings(Some(&path), &settings).unwrap();
        assert_eq!(read_settings(Some(&path)), settings);
        std::fs::write(&path, r#"{"wide": true, "someday": 1}"#).unwrap();
        assert!(read_settings(Some(&path)).wide);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_document_stays_inside_its_sessions_folder() {
        let root = std::env::temp_dir().join(format!("quill-doc-{}", std::process::id()));
        std::fs::create_dir_all(root.join("docs")).unwrap();
        write_text(&root, "docs/notes.md", "# Notes\n").unwrap();
        assert_eq!(
            read_text(&root, "docs/notes.md").as_deref(),
            Some("# Notes\n")
        );
        for outside in ["../escape.md", "/etc/passwd", "docs/../../escape.md"] {
            assert_eq!(read_text(&root, outside), None, "{outside}");
            assert!(write_text(&root, outside, "x").is_err(), "{outside}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
