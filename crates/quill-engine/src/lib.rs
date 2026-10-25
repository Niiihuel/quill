//! quill's engine: one Tokio actor per session, owning its agent's process and adapter
//! (spec.md D6, engine.md §1). The UI sends [`Intent`]s with [`Engine::send`]; the engine
//! reports [`Event`]s through the sink given to [`Engine::start`]. Nothing here blocks.

use quill_agent::{AgentAdapter, AgentCommand, AgentEvent, SessionSpec};
use quill_model::{Event, Intent, NewSession, RequestId, SessionId, SessionStatus};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
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

impl Engine {
    pub fn start(sink: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self::start_with(sink, HashMap::new())
    }

    /// `programs` replaces an agent's executable (tests run a fake agent this way).
    pub fn start_with(
        sink: impl Fn(Event) + Send + Sync + 'static,
        programs: HashMap<String, String>,
    ) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("quill-engine")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (intents, rx) = unbounded_channel();
        runtime.spawn(route(rx, Arc::new(sink), programs));
        Self {
            intents,
            _runtime: runtime,
        }
    }

    pub fn send(&self, intent: Intent) {
        let _ = self.intents.send(intent);
    }
}

/// Hands each intent to its session's actor, creating sessions as asked.
async fn route(mut rx: UnboundedReceiver<Intent>, sink: Sink, programs: HashMap<String, String>) {
    let mut sessions: HashMap<SessionId, UnboundedSender<Input>> = HashMap::new();
    let mut next: SessionId = 0;
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
                let program = programs.get(&new.agent).cloned();
                tokio::spawn(
                    Actor::new(next, new, adapter, program, tx.clone(), sink.clone()).run(rx),
                );
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
        };
        if let Some(tx) = sessions.get(&session) {
            let _ = tx.send(input);
        }
    }
}

/// What reaches a session's actor: the UI's wishes and its process's output.
enum Input {
    Send(String),
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
                Input::Command(command) => {
                    let resumes = matches!(command, AgentCommand::Decide { .. });
                    self.write(command).await;
                    if resumes {
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
                self.running = None;
                self.busy = false;
                self.status(SessionStatus::Idle);
            }
        }
    }
}
