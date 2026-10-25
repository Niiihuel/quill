//! The engine driving a fake agent that replays real captures (crates/quill-agent-claude/fixtures).

use quill_engine::Engine;
use quill_model::{AgentEvent, Decision, Event, Intent, NewSession, SessionStatus};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

fn fixture(name: &str) -> String {
    format!(
        "{}/../quill-agent-claude/fixtures/2.1.310/{name}.jsonl",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn engine() -> (Engine, Receiver<Event>) {
    let (tx, rx) = channel();
    let programs = HashMap::from([(
        "claude".to_string(),
        env!("CARGO_BIN_EXE_fake-agent").to_string(),
    )]);
    let engine = Engine::start_with(move |event| drop(tx.send(event)), programs);
    (engine, rx)
}

fn new_session(engine: &Engine, name: &str) {
    // Read by the fake agent when it starts, which happens on the first message.
    std::env::set_var("FAKE_AGENT_FIXTURE", fixture(name));
    engine.send(Intent::NewSession(NewSession {
        agent: "claude".into(),
        cwd: std::env::temp_dir(),
        model: None,
        effort: None,
        mode: None,
    }));
}

/// Collect events until `done` matches one, failing after a few seconds.
fn until(rx: &Receiver<Event>, mut done: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let mut seen = vec![];
    loop {
        let event = rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("timed out after {seen:#?}"));
        let stop = done(&event);
        seen.push(event);
        if stop {
            return seen;
        }
    }
}

// One test: the fixture reaches the fake agent through the environment, shared by the process.
#[test]
fn sessions_stream_turns_and_wait_for_decisions() {
    let (engine, rx) = engine();

    new_session(&engine, "simple");
    engine.send(Intent::Send {
        session: 1,
        text: "Reply with exactly the word: ok".into(),
    });
    let events = until(&rx, |e| {
        matches!(
            e,
            Event::Agent {
                event: AgentEvent::TurnFinished { .. },
                ..
            }
        )
    });
    let request = events.iter().find_map(|e| {
        if let Event::Queued { request, .. } = e {
            Some(request.clone())
        } else {
            None
        }
    });
    assert!(request.is_some());
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Status {
            status: SessionStatus::Running,
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Agent {
            event: AgentEvent::TextDelta { .. },
            request: Some(_),
            ..
        }
    )));
    until(&rx, |e| {
        matches!(
            e,
            Event::Status {
                session: 1,
                status: SessionStatus::Idle
            }
        )
    });

    new_session(&engine, "deny");
    engine.send(Intent::Send {
        session: 2,
        text: "Use the Bash tool to run exactly: touch denied.txt".into(),
    });
    let events = until(&rx, |e| {
        matches!(
            e,
            Event::Agent {
                event: AgentEvent::PermissionRequest { .. },
                ..
            }
        )
    });
    let Some(Event::Agent {
        event: AgentEvent::PermissionRequest { id, .. },
        ..
    }) = events.last()
    else {
        unreachable!()
    };
    // The session already waits for the person when the card arrives.
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Status {
            session: 2,
            status: SessionStatus::Awaiting
        }
    )));
    engine.send(Intent::Decide {
        session: 2,
        permission: id.clone(),
        decision: Decision::Deny {
            message: "no".into(),
        },
    });
    let events = until(&rx, |e| {
        matches!(
            e,
            Event::Agent {
                session: 2,
                event: AgentEvent::TurnFinished { .. },
                ..
            }
        )
    });
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Agent {
            event: AgentEvent::ToolResult { is_error: true, .. },
            ..
        }
    )));
}
