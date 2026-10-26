//! quill's engine: one Tokio actor per session, owning its agent's process and adapter
//! (spec.md D6, engine.md §1). The UI sends [`Intent`]s with [`Engine::send`]; the engine
//! reports [`Event`]s through the sink given to [`Engine::start`]. Nothing here blocks.

use quill_agent::{AgentAdapter, AgentCommand, AgentEvent, SessionSpec};
use quill_model::{Event, Intent, NewSession, RequestId, SessionId, SessionStatus};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// Where events go; the app wakes its window from here.
pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

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
        Self {
            database: Some(data.join("quill/quill.db")),
            snapshots: Some(data.join("quill/snapshots")),
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

async fn route(mut rx: UnboundedReceiver<Intent>, ui: Sink, config: Config) {
    let mut router = Router::new(ui, config);
    router.restore();
    while let Some(intent) = rx.recv().await {
        router.intent(intent).await;
    }
}

/// The engine's own state: each session's actor and how it started.
struct Router {
    store: Arc<Mutex<quill_store::Store>>,
    ui: Sink,
    /// Every event is written before the UI sees it (spec.md D5).
    sink: Sink,
    config: Config,
    sessions: HashMap<SessionId, UnboundedSender<Input>>,
    /// Each session's folder, for what is uncommitted in it.
    folders: HashMap<SessionId, std::path::PathBuf>,
    /// How each session started, which a fork of it or a session it starts starts as too.
    started: HashMap<SessionId, NewSession>,
    /// Each session's own id at its agent, which a fork continues from.
    agent_ids: Arc<Mutex<HashMap<SessionId, String>>>,
    next: SessionId,
}

impl Router {
    fn new(ui: Sink, config: Config) -> Self {
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
        Self {
            store,
            ui,
            sink,
            config,
            sessions: HashMap::new(),
            folders: HashMap::new(),
            started: HashMap::new(),
            agent_ids,
            next: 0,
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
            for event in events {
                (self.ui)(event);
            }
            let Some(adapter) = adapter(&restored.new.agent) else {
                continue;
            };
            let (tx, rx) = unbounded_channel();
            let program = self.config.programs.get(&restored.new.agent).cloned();
            let mut actor = Actor::new(
                session,
                restored.new,
                adapter,
                program,
                tx.clone(),
                self.sink.clone(),
            );
            actor.spec.resume = restored.resume;
            actor.before = restored.before;
            actor.shadow_root = self.config.snapshots.clone();
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
        fork_of: Option<SessionId>,
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
        let program = self.config.programs.get(&new.agent).cloned();
        let mut actor = Actor::new(id, new, adapter, program, tx.clone(), self.sink.clone());
        actor.shadow_root = self.config.snapshots.clone();
        if let Some(from) = fork_of {
            // Without an id of its own yet, the parent has no conversation to continue.
            actor.spec.resume = self.agent_ids.lock().unwrap().get(&from).cloned();
            actor.spec.fork = actor.spec.resume.is_some();
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

    async fn intent(&mut self, intent: Intent) {
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
            Intent::Rewind { session, request } => (session, Input::Rewind(request)),
            Intent::Rename { session, title } => {
                (self.sink)(Event::Renamed { session, title });
                return;
            }
            Intent::Pin { session, pinned } => {
                (self.sink)(Event::Pinned { session, pinned });
                return;
            }
            Intent::Archive { session, archived } => {
                (self.sink)(Event::Archived { session, archived });
                return;
            }
            Intent::MarkUnread { session, unread } => {
                (self.sink)(Event::Unread { session, unread });
                return;
            }
            Intent::Fork { session: from } => {
                if let Some(new) = self.started.get(&from).cloned() {
                    self.spawn(
                        new,
                        |id| vec![Event::Forked { session: id, from }],
                        Some(from),
                    )
                    .await;
                }
                return;
            }
            // The agent is stopped first, so nothing it says lands after its record is gone.
            Intent::Delete { session } => {
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
}

/// What a kept session needs to continue: how it started and the agent's own id for it.
struct Restored {
    new: NewSession,
    resume: Option<String>,
    /// Each attributed request's snapshot from before it ran, for rewinds after a restart.
    before: HashMap<RequestId, String>,
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
        for event in events {
            match event {
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
                _ => {}
            }
        }
        Some(Self {
            new,
            resume,
            before,
        })
    }
}

/// What reaches a session's actor: the UI's wishes and its process's output.
enum Input {
    Send(String),
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
        }
    }

    /// Send `text` as a new request.
    async fn prompt(&mut self, text: String) {
        if let Err(error) = self.ensure_running().await {
            self.status(SessionStatus::Failed(error));
            return;
        }
        let request = uuid::Uuid::new_v4().to_string();
        // The first request in line starts from the folder as it is now; the
        // ones queued behind it start where the turn before them ended.
        if self.sent.is_empty() {
            self.start = self.snapshot().await;
        }
        self.sent.push_back(request.clone());
        (self.sink)(Event::Queued {
            session: self.id,
            request: request.clone(),
            text: text.clone(),
        });
        self.write(AgentCommand::Prompt {
            request,
            text,
            priority: None,
        })
        .await;
        self.busy = true;
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
                Input::Send(text) => self.prompt(text).await,
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
            match &event {
                AgentEvent::Session { id } => {
                    self.spec.resume = Some(id.clone());
                    // A fork has its own id now: what follows continues it, not its parent.
                    self.spec.fork = false;
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
            }
        }
    }
}
