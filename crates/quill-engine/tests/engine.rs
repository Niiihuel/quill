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
    let config = quill_engine::Config {
        programs,
        database: None,
        snapshots: None,
        settings: None,
        ..Default::default()
    };
    let engine = Engine::start_with(move |event| drop(tx.send(event)), config);
    (engine, rx)
}

/// A working directory whose `.fake-agent` names the capture the fake agent replays.
fn workdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "quill-engine-{}-{name}-{}",
        std::process::id(),
        uuid_like()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".fake-agent"), fixture(name)).unwrap();
    dir
}

fn uuid_like() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn new_session(engine: &Engine, name: &str) {
    engine.send(Intent::NewSession(NewSession {
        agent: "claude".into(),
        cwd: workdir(name),
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

#[test]
fn a_message_keeps_its_images_and_a_text_file_among_them_is_left_out() {
    let (engine, rx) = engine();
    let dir = workdir("simple");
    engine.send(Intent::NewSession(NewSession {
        agent: "claude".into(),
        cwd: dir.clone(),
        model: None,
        effort: None,
        mode: None,
    }));
    let picture = dir.join("picture.png");
    image::RgbImage::new(4, 4).save(&picture).unwrap();
    let notes = dir.join("notes.txt");
    std::fs::write(&notes, "words").unwrap();
    engine.send(Intent::Send {
        session: 1,
        text: "look".into(),
        mentions: vec![],
        images: vec![picture.clone(), notes.clone()],
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
    let queued = events.iter().find_map(|e| match e {
        Event::Queued { images, .. } => Some(images.clone()),
        _ => None,
    });
    assert_eq!(queued, Some(vec![picture, notes]));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Notice { text, .. }
            if text == "Only PNG, JPEG, GIF, and WebP images are supported."
    )));
}

#[test]
fn sessions_stream_turns_and_wait_for_decisions() {
    let (engine, rx) = engine();

    new_session(&engine, "simple");
    engine.send(Intent::Send {
        session: 1,
        text: "Reply with exactly the word: ok".into(),
        mentions: vec![],
        images: vec![],
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
        mentions: vec![],
        images: vec![],
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

#[test]
fn a_kept_session_comes_back_and_continues() {
    let database = std::env::temp_dir().join(format!("quill-test-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&database);
    let start = || {
        let (tx, rx) = channel();
        let programs = HashMap::from([(
            "claude".to_string(),
            env!("CARGO_BIN_EXE_fake-agent").to_string(),
        )]);
        let config = quill_engine::Config {
            programs,
            database: Some(database.clone()),
            snapshots: None,
            settings: None,
            ..Default::default()
        };
        (
            Engine::start_with(move |event| drop(tx.send(event)), config),
            rx,
        )
    };
    let cwd = workdir("simple");

    let (engine, rx) = start();
    engine.send(Intent::NewSession(NewSession {
        agent: "claude".into(),
        cwd: cwd.clone(),
        model: None,
        effort: None,
        mode: None,
    }));
    engine.send(Intent::Send {
        session: 1,
        text: "Reply with exactly the word: ok".into(),
        mentions: vec![],
        images: vec![],
    });
    until(&rx, |e| {
        matches!(
            e,
            Event::Agent {
                event: AgentEvent::TurnFinished { .. },
                ..
            }
        )
    });
    drop(engine);

    let (engine, rx) = start();
    let replayed = until(&rx, |e| {
        matches!(
            e,
            Event::Status {
                session: 1,
                status: SessionStatus::Exited
            }
        )
    });
    assert!(matches!(
        replayed.first(),
        Some(Event::SessionCreated { session: 1, .. })
    ));
    assert!(replayed
        .iter()
        .any(|e| matches!(e, Event::Queued { text, .. } if text.contains("ok"))));
    assert!(replayed.iter().any(|e| matches!(
        e,
        Event::Agent {
            event: AgentEvent::Text { .. },
            ..
        }
    )));
    assert!(!replayed.iter().any(|e| matches!(
        e,
        Event::Agent {
            event: AgentEvent::TextDelta { .. },
            ..
        }
    )));
    // The kept session takes a new message and runs again (resuming its conversation).
    engine.send(Intent::Send {
        session: 1,
        text: "again".into(),
        mentions: vec![],
        images: vec![],
    });
    until(&rx, |e| {
        matches!(
            e,
            Event::Agent {
                session: 1,
                event: AgentEvent::TurnFinished { .. },
                ..
            }
        )
    });
    let _ = std::fs::remove_file(&database);
}

#[test]
fn a_request_is_attributed_and_its_undo_puts_files_back() {
    let (tx, rx) = channel();
    let programs = HashMap::from([(
        "claude".to_string(),
        env!("CARGO_BIN_EXE_fake-agent").to_string(),
    )]);
    let snapshots = std::env::temp_dir().join(format!("quill-snapshots-{}", uuid_like()));
    let config = quill_engine::Config {
        programs,
        database: None,
        snapshots: Some(snapshots.clone()),
        settings: None,
        ..Default::default()
    };
    let engine = Engine::start_with(move |event| drop(tx.send(event)), config);
    let cwd = workdir("rewind");
    engine.send(Intent::NewSession(NewSession {
        agent: "claude".into(),
        cwd: cwd.clone(),
        model: None,
        effort: None,
        mode: None,
    }));
    engine.send(Intent::Send {
        session: 1,
        text: "Create rewind.txt containing the line one.".into(),
        mentions: vec![],
        images: vec![],
    });

    let events = until(&rx, |e| matches!(e, Event::Changes { .. }));
    let Some(Event::Changes { request, files, .. }) = events.last() else {
        unreachable!()
    };
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(
        (files[0].path.as_str(), &files[0].kind),
        ("rewind.txt", &quill_model::ChangeKind::Added)
    );
    assert!(cwd.join("rewind.txt").exists());

    engine.send(Intent::Rewind {
        session: 1,
        request: request.clone(),
    });
    let events = until(&rx, |e| matches!(e, Event::Rewound { .. }));
    assert!(matches!(events.last(), Some(Event::Rewound { files, .. }) if files.len() == 1));
    assert!(!cwd.join("rewind.txt").exists());
    // The marker the fake agent reads stays: the rewind undoes only what the request did.
    assert!(cwd.join(".fake-agent").exists());
    let _ = std::fs::remove_dir_all(&snapshots);
}

#[test]
fn a_name_and_a_pin_are_kept_and_a_deleted_session_is_gone() {
    let database = std::env::temp_dir().join(format!("quill-test-delete-{}.db", uuid_like()));
    let start = || {
        let (tx, rx) = channel();
        let programs = HashMap::from([(
            "claude".to_string(),
            env!("CARGO_BIN_EXE_fake-agent").to_string(),
        )]);
        let config = quill_engine::Config {
            programs,
            database: Some(database.clone()),
            snapshots: None,
            settings: None,
            ..Default::default()
        };
        (
            Engine::start_with(move |event| drop(tx.send(event)), config),
            rx,
        )
    };

    let (engine, rx) = start();
    new_session(&engine, "simple");
    engine.send(Intent::Send {
        session: 1,
        text: "Reply with exactly the word: ok".into(),
        mentions: vec![],
        images: vec![],
    });
    until(&rx, |e| {
        matches!(
            e,
            Event::Agent {
                event: AgentEvent::TurnFinished { .. },
                ..
            }
        )
    });
    engine.send(Intent::Rename {
        session: 1,
        title: "Mine".into(),
    });
    engine.send(Intent::Pin {
        session: 1,
        pinned: true,
    });
    engine.send(Intent::Archive {
        session: 1,
        archived: true,
    });
    engine.send(Intent::MarkUnread {
        session: 1,
        unread: true,
    });
    until(&rx, |e| matches!(e, Event::Unread { .. }));
    // A fork continues the conversation as a new session that runs on its own.
    engine.send(Intent::Fork {
        session: 1,
        at: None,
    });
    let forked = until(&rx, |e| matches!(e, Event::Forked { .. }));
    assert!(forked
        .iter()
        .any(|e| matches!(e, Event::SessionCreated { session: 2, .. })));
    assert!(matches!(
        forked.last(),
        Some(Event::Forked {
            session: 2,
            from: 1,
            at: None
        })
    ));
    engine.send(Intent::Send {
        session: 2,
        text: "again".into(),
        mentions: vec![],
        images: vec![],
    });
    until(&rx, |e| {
        matches!(
            e,
            Event::Agent {
                session: 2,
                event: AgentEvent::TurnFinished { .. },
                ..
            }
        )
    });
    engine.send(Intent::Delete { session: 2 });
    until(&rx, |e| matches!(e, Event::Deleted { session: 2 }));
    drop(engine);

    let (engine, rx) = start();
    let replayed = until(&rx, |e| matches!(e, Event::Status { session: 1, .. }));
    assert!(replayed
        .iter()
        .any(|e| matches!(e, Event::Renamed { title, .. } if title == "Mine")));
    assert!(replayed
        .iter()
        .any(|e| matches!(e, Event::Pinned { pinned: true, .. })));
    assert!(replayed
        .iter()
        .any(|e| matches!(e, Event::Archived { archived: true, .. })));
    assert!(replayed
        .iter()
        .any(|e| matches!(e, Event::Unread { unread: true, .. })));
    // A running agent is stopped and its record removed.
    engine.send(Intent::Send {
        session: 1,
        text: "again".into(),
        mentions: vec![],
        images: vec![],
    });
    engine.send(Intent::Delete { session: 1 });
    until(&rx, |e| matches!(e, Event::Deleted { session: 1 }));
    drop(engine);

    let (engine, rx) = start();
    new_session(&engine, "simple");
    // The settings come first at every start; they are no session's.
    let events: Vec<Event> = until(&rx, |e| matches!(e, Event::SessionCreated { .. }))
        .into_iter()
        .filter(|e| !matches!(e, Event::Settings { .. }))
        .collect();
    assert_eq!(
        events.len(),
        1,
        "nothing of the deleted session came back: {events:#?}"
    );
    drop(engine);
    let _ = std::fs::remove_file(&database);
}

#[test]
fn a_proposed_task_starts_a_linked_session_that_reports_back() {
    let (engine, rx) = engine();
    new_session(&engine, "sessions");
    engine.send(Intent::Send {
        session: 1,
        text: "Propose a README task".into(),
        mentions: vec![],
        images: vec![],
    });
    // The agent's start_session call becomes a proposal; nothing runs yet.
    let events = until(&rx, |e| matches!(e, Event::Proposed { session: 1, .. }));
    let Some(Event::Proposed { proposal, .. }) = events.last() else {
        unreachable!()
    };
    assert_eq!(proposal.title, "Add a README");
    let task = proposal.id.clone();
    until(&rx, |e| {
        matches!(
            e,
            Event::Agent {
                session: 1,
                event: AgentEvent::TurnFinished { .. },
                ..
            }
        )
    });
    // Started, it is a session of its own under the one that proposed it.
    engine.send(Intent::AnswerProposal {
        session: 1,
        task: task.clone(),
        answer: quill_model::ProposalAnswer::Start,
    });
    let events = until(&rx, |e| matches!(e, Event::ProposalAnswered { .. }));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Linked {
            session: 2,
            parent: 1,
            side: false
        }
    )));
    assert!(matches!(
        events.last(),
        Some(Event::ProposalAnswered {
            session: 1,
            outcome: quill_model::ProposalOutcome::Started { session: 2 },
            ..
        })
    ));
    // When its turn ends, what it said goes back to the one that started it.
    let events = until(&rx, |e| matches!(e, Event::Reported { session: 1, .. }));
    let Some(Event::Reported { reports, .. }) = events.last() else {
        unreachable!()
    };
    assert_eq!(reports[0].from, 2);
    assert_eq!(reports[0].title, "Add a README");
    assert_eq!(reports[0].text, "done");
}

#[test]
fn a_side_question_forks_the_session_beside_it() {
    let (engine, rx) = engine();
    new_session(&engine, "simple");
    engine.send(Intent::Send {
        session: 1,
        text: "Reply with exactly the word: ok".into(),
        mentions: vec![],
        images: vec![],
    });
    until(&rx, |e| {
        matches!(
            e,
            Event::Agent {
                session: 1,
                event: AgentEvent::TurnFinished { .. },
                ..
            }
        )
    });
    engine.send(Intent::SideChat {
        session: 1,
        text: "what is this?".into(),
    });
    let events = until(&rx, |e| matches!(e, Event::Queued { session: 2, .. }));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Linked {
            session: 2,
            parent: 1,
            side: true
        }
    )));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Renamed { session: 2, title } if title == "what is this?")));
}
