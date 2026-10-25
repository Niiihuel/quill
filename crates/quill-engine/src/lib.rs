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

/// Hands each intent to its session's actor, creating sessions as asked. Sessions kept
/// from earlier runs come back first, ready to continue their conversation.
async fn route(mut rx: UnboundedReceiver<Intent>, ui: Sink, config: Config) {
    let store = Arc::new(Mutex::new(open_store(&config, &ui)));
    // Every event is written before the UI sees it (spec.md D5).
    let sink: Sink = {
        let (store, ui) = (store.clone(), ui.clone());
        Arc::new(move |event: Event| {
            if let Err(error) = store.lock().unwrap().append(event.session(), &event) {
                ui(Event::Status {
                    session: event.session(),
                    status: SessionStatus::Failed(format!("not saved: {error}")),
                });
            }
            ui(event);
        })
    };
    let mut sessions: HashMap<SessionId, UnboundedSender<Input>> = HashMap::new();
    let mut next: SessionId = 0;
    let kept = store.lock().unwrap().load().unwrap_or_default();
    for (session, events) in kept {
        next = next.max(session);
        let Some(restored) = Restored::from(&events) else {
            continue;
        };
        for event in events {
            ui(event);
        }
        let Some(adapter) = adapter(&restored.new.agent) else {
            continue;
        };
        let (tx, rx) = unbounded_channel();
        let program = config.programs.get(&restored.new.agent).cloned();
        let mut actor = Actor::new(
            session,
            restored.new,
            adapter,
            program,
            tx.clone(),
            sink.clone(),
        );
        actor.spec.resume = restored.resume;
        actor.before = restored.before;
        actor.shadow_root = config.snapshots.clone();
        tokio::spawn(actor.run(rx));
        sessions.insert(session, tx);
        ui(Event::Status {
            session,
            status: SessionStatus::Exited,
        });
    }
    while let Some(intent) = rx.recv().await {
        let (session, input) = match intent {
            Intent::NewSession(new) => {
                next += 1;
                let Some(adapter) = adapter(&new.agent) else {
                    sink(Event::Status {
                        session: next,
                        status: SessionStatus::Failed(format!("no adapter for {}", new.agent)),
                    });
                    continue;
                };
                let (tx, rx) = unbounded_channel();
                sink(Event::SessionCreated {
                    session: next,
                    agent: new.agent.clone(),
                    cwd: new.cwd.clone(),
                });
                let cwd = new.cwd.clone();
                let branch = tokio::task::spawn_blocking(move || quill_git::branch(&cwd))
                    .await
                    .ok()
                    .flatten();
                sink(Event::Environment {
                    session: next,
                    branch,
                });
                let program = config.programs.get(&new.agent).cloned();
                let mut actor = Actor::new(next, new, adapter, program, tx.clone(), sink.clone());
                actor.shadow_root = config.snapshots.clone();
                tokio::spawn(actor.run(rx));
                sessions.insert(next, tx);
                continue;
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
            Intent::Rewind { session, request } => (session, Input::Rewind(request)),
        };
        if let Some(tx) = sessions.get(&session) {
            let _ = tx.send(input);
        }
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
    Line { generation: u64, line: String },
    Exited { generation: u64, code: Option<i32> },
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
        while let Some(input) = rx.recv().await {
            match input {
                Input::Send(text) => {
                    if let Err(error) = self.ensure_running().await {
                        self.status(SessionStatus::Failed(error));
                        continue;
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
                AgentEvent::Session { id } => self.spec.resume = Some(id.clone()),
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
