//! The engine driving a fake agent that replays real captures (crates/quill-agent-claude/fixtures).

use quill_engine::Engine;
use quill_model::{AgentEvent, Decision, Event, Intent, Lane, NewSession, SessionStatus};
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
        lane: Lane::Steer,
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
        lane: Lane::Steer,
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
        lane: Lane::Steer,
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
        lane: Lane::Steer,
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
    // The history is told before `Restored`, so the host notifies only what follows it.
    let mut told = replayed.clone();
    if !told.iter().any(|e| matches!(e, Event::Restored)) {
        told.extend(until(&rx, |e| matches!(e, Event::Restored)));
    }
    let at = |wanted: fn(&Event) -> bool| told.iter().position(wanted);
    assert!(
        at(|e| matches!(
            e,
            Event::Agent {
                event: AgentEvent::TurnFinished { .. },
                ..
            }
        )) < at(|e| matches!(e, Event::Restored))
    );
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
        lane: Lane::Steer,
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
        lane: Lane::Steer,
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
        lane: Lane::Steer,
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
        lane: Lane::Steer,
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
        lane: Lane::Steer,
    });
    engine.send(Intent::Delete { session: 1 });
    until(&rx, |e| matches!(e, Event::Deleted { session: 1 }));
    drop(engine);

    let (engine, rx) = start();
    new_session(&engine, "simple");
    // The settings and the end of the retelling come at every start; they are no session's.
    let events: Vec<Event> = until(&rx, |e| matches!(e, Event::SessionCreated { .. }))
        .into_iter()
        .filter(|e| !matches!(e, Event::Settings { .. } | Event::Restored))
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
        lane: Lane::Steer,
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
        lane: Lane::Steer,
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

/// Events until the steer capture's turn is under way: its agent said it is ready and the
/// prompt was written. The capture then holds the turn open at its steer line, until something
/// is written there, so a queue has a busy turn to wait behind.
fn busy_steer_session(engine: &Engine, rx: &Receiver<Event>) {
    new_session(engine, "steer");
    // The agent starts with the first message; its `Ready` says whether it steers.
    engine.send(Intent::Send {
        session: 1,
        text: "Count from 1 to 40".into(),
        mentions: vec![],
        images: vec![],
        lane: Lane::Steer,
    });
    until(rx, |e| {
        matches!(
            e,
            Event::Agent {
                event: AgentEvent::Ready { .. },
                ..
            }
        )
    });
}

fn queue_send(engine: &Engine, text: &str) {
    engine.send(Intent::Send {
        session: 1,
        text: text.into(),
        mentions: vec![],
        images: vec![],
        lane: Lane::Queue,
    });
}

/// The ids of the queue as the engine last said it, after `done` matched its change.
fn queue_ids(events: &[Event]) -> Vec<(String, String)> {
    let Some(Event::QueueChanged { entries, .. }) = events
        .iter()
        .rev()
        .find(|e| matches!(e, Event::QueueChanged { .. }))
    else {
        panic!("no QueueChanged in {events:#?}");
    };
    entries
        .iter()
        .map(|m| (m.id.clone(), m.text.clone()))
        .collect()
}

#[test]
fn a_queued_message_is_not_sent_mid_turn_and_goes_out_when_the_turn_ends() {
    let (engine, rx) = engine();
    busy_steer_session(&engine, &rx);
    queue_send(&engine, "later");
    let events = until(
        &rx,
        |e| matches!(e, Event::QueueChanged { entries, .. } if entries.len() == 1),
    );
    assert!(!events
        .iter()
        .any(|e| matches!(e, Event::Queued { text, .. } if text == "later")));
    engine.send(Intent::Interrupt { session: 1 });
    until(
        &rx,
        |e| matches!(e, Event::Queued { text, .. } if text == "later"),
    );
    // The queue is told it is empty once the message is out.
    until(
        &rx,
        |e| matches!(e, Event::QueueChanged { entries, .. } if entries.is_empty()),
    );
}

#[test]
fn queued_messages_go_out_as_one_message() {
    let (engine, rx) = engine();
    busy_steer_session(&engine, &rx);
    queue_send(&engine, "a");
    until(
        &rx,
        |e| matches!(e, Event::QueueChanged { entries, .. } if entries.len() == 1),
    );
    queue_send(&engine, "b");
    until(
        &rx,
        |e| matches!(e, Event::QueueChanged { entries, .. } if entries.len() == 2),
    );
    engine.send(Intent::Interrupt { session: 1 });
    let events = until(&rx, |e| matches!(e, Event::Queued { .. }));
    assert!(matches!(
        events.last(),
        Some(Event::Queued { text, .. }) if text == "a\n\nb"
    ));
}

#[test]
fn a_slash_command_in_the_queue_is_sent_alone() {
    let (engine, rx) = engine();
    busy_steer_session(&engine, &rx);
    queue_send(&engine, "/compact");
    until(
        &rx,
        |e| matches!(e, Event::QueueChanged { entries, .. } if entries.len() == 1),
    );
    queue_send(&engine, "c");
    until(
        &rx,
        |e| matches!(e, Event::QueueChanged { entries, .. } if entries.len() == 2),
    );
    engine.send(Intent::Interrupt { session: 1 });
    let events = until(
        &rx,
        |e| matches!(e, Event::Queued { text, .. } if text == "/compact"),
    );
    assert!(!events
        .iter()
        .any(|e| matches!(e, Event::Queued { text, .. } if text == "c")));
    until(
        &rx,
        |e| matches!(e, Event::Queued { text, .. } if text == "c"),
    );
}

#[test]
fn a_queued_message_can_be_removed_sent_now_or_taken_back() {
    let (engine, rx) = engine();
    busy_steer_session(&engine, &rx);
    for text in ["a", "b", "c"] {
        queue_send(&engine, text);
    }
    let events = until(
        &rx,
        |e| matches!(e, Event::QueueChanged { entries, .. } if entries.len() == 3),
    );
    let [(a, _), (b, _), (c, _)] = queue_ids(&events).try_into().unwrap();
    engine.send(Intent::QueueRemove { session: 1, id: a });
    until(
        &rx,
        |e| matches!(e, Event::QueueChanged { entries, .. } if entries.len() == 2),
    );
    engine.send(Intent::QueueSendNow { session: 1, id: b });
    // Sent now, as a steer: the capture reads it at its steer line and the turn goes on.
    until(
        &rx,
        |e| matches!(e, Event::Queued { text, .. } if text == "b"),
    );
    engine.send(Intent::QueueTake { session: 1, id: c });
    let events = until(&rx, |e| matches!(e, Event::QueueTaken { .. }));
    assert!(matches!(
        events.last(),
        Some(Event::QueueTaken { text, images, .. }) if text == "c" && images.is_empty()
    ));
    engine.send(Intent::Interrupt { session: 1 });
    let events = quiet(&rx, 800);
    assert!(!events
        .iter()
        .any(|e| matches!(e, Event::Queued { text, .. } if text == "c")));
}

#[test]
fn a_cleared_queue_sends_nothing_at_the_turn_end() {
    let (engine, rx) = engine();
    busy_steer_session(&engine, &rx);
    queue_send(&engine, "a");
    until(&rx, |e| matches!(e, Event::QueueChanged { .. }));
    engine.send(Intent::QueueClear { session: 1 });
    until(
        &rx,
        |e| matches!(e, Event::QueueChanged { entries, .. } if entries.is_empty()),
    );
    engine.send(Intent::Interrupt { session: 1 });
    let events = quiet(&rx, 800);
    assert!(!events.iter().any(|e| matches!(e, Event::Queued { .. })));
}

/// Events for `millis`, with nothing waited for: what the engine did in that time.
fn quiet(rx: &Receiver<Event>, millis: u64) -> Vec<Event> {
    let mut seen = vec![];
    while let Ok(event) = rx.recv_timeout(Duration::from_millis(millis)) {
        seen.push(event);
    }
    seen
}

#[test]
fn a_queue_kept_across_a_restart_comes_back_held_and_waits_for_the_person() {
    let database = std::env::temp_dir().join(format!("quill-test-queue-{}.db", uuid_like()));
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
    busy_steer_session(&engine, &rx);
    queue_send(&engine, "held one");
    until(
        &rx,
        |e| matches!(e, Event::QueueChanged { entries, .. } if entries.len() == 1),
    );
    drop(engine);

    let (engine, rx) = start();
    let events = until(&rx, |e| {
        matches!(e, Event::QueueChanged { entries, .. }
            if entries.len() == 1 && entries[0].held && entries[0].text == "held one")
    });
    let [(id, _)] = queue_ids(&events).try_into().unwrap();
    // A turn ending does not send it.
    engine.send(Intent::Send {
        session: 1,
        text: "Count from 1 to 40".into(),
        mentions: vec![],
        images: vec![],
        lane: Lane::Steer,
    });
    until(&rx, |e| {
        matches!(
            e,
            Event::Status {
                status: SessionStatus::Running,
                ..
            }
        )
    });
    engine.send(Intent::Interrupt { session: 1 });
    until(&rx, |e| {
        matches!(
            e,
            Event::Agent {
                event: AgentEvent::TurnFinished { .. },
                ..
            }
        )
    });
    let events = quiet(&rx, 800);
    assert!(!events
        .iter()
        .any(|e| matches!(e, Event::Queued { text, .. } if text == "held one")));
    // The person sends it, which goes out.
    engine.send(Intent::QueueSendNow { session: 1, id });
    until(
        &rx,
        |e| matches!(e, Event::Queued { text, .. } if text == "held one"),
    );
    let _ = std::fs::remove_file(&database);
}

/// An engine whose fake agent starts signed out, with the marker that flips it in a folder of
/// the test's own (the agent's environment setting reaches the account commands too).
fn signed_out_engine(name: &str) -> (Engine, Receiver<Event>, std::path::PathBuf) {
    let (engine, rx) = engine();
    let marker = workdir(name).join("marker");
    let mut settings = quill_model::Settings::default();
    settings.agents.insert(
        "claude".into(),
        quill_model::AgentSettings {
            env: vec![
                "QUILL_FAKE_AUTH=signed-out".into(),
                format!("QUILL_FAKE_AUTH_MARKER={}", marker.display()),
            ],
            ..Default::default()
        },
    );
    engine.send(Intent::SaveSettings(settings));
    (engine, rx, marker)
}

fn state(rx: &Receiver<Event>) -> quill_model::AuthState {
    let seen = until(rx, |e| matches!(e, Event::Auth { .. }));
    match seen.last() {
        Some(Event::Auth { state, .. }) => state.clone(),
        _ => unreachable!(),
    }
}

#[test]
fn signing_in_through_the_program_ends_signed_in_and_signing_out_ends_signed_out() {
    let (engine, rx, _) = signed_out_engine("auth-flow");
    let agent = "claude".to_string();
    engine.send(Intent::CheckSignIn {
        agent: agent.clone(),
    });
    assert_eq!(state(&rx), quill_model::AuthState::SignedOut);

    engine.send(Intent::SignIn {
        agent: agent.clone(),
        method: "claudeai".into(),
    });
    let seen = until(&rx, |e| matches!(e, Event::SignInUrl { .. }));
    // The OSC 8 wrapper is gone: a plain address.
    assert!(matches!(
        seen.last(),
        Some(Event::SignInUrl { url, .. }) if url.starts_with("https://claude.com/cai/oauth/authorize?")
            && !url.contains('\x1b') && !url.contains('\x07')
    ));
    engine.send(Intent::SubmitSignInCode {
        agent: agent.clone(),
        code: "abc#state".into(),
    });
    assert!(
        matches!(state(&rx), quill_model::AuthState::SignedIn { email: Some(e), .. } if e == "ada@example.com")
    );

    engine.send(Intent::SignOut { agent });
    assert_eq!(state(&rx), quill_model::AuthState::SignedOut);
}

#[test]
fn a_rejected_code_says_what_the_program_said_and_cancel_ends_the_login() {
    let (engine, rx, marker) = signed_out_engine("auth-fail");
    let agent = "claude".to_string();
    let sign_in = || Intent::SignIn {
        agent: "claude".into(),
        method: "console".into(),
    };
    engine.send(sign_in());
    until(&rx, |e| matches!(e, Event::SignInUrl { .. }));
    engine.send(Intent::SubmitSignInCode {
        agent: agent.clone(),
        code: "fail".into(),
    });
    let seen = until(&rx, |e| matches!(e, Event::SignInFailed { .. }));
    assert!(matches!(
        seen.last(),
        Some(Event::SignInFailed { message, .. }) if message == "Login failed: the code was not accepted"
    ));

    // Cancelled, the program is killed before it can accept a code: nothing follows.
    engine.send(sign_in());
    until(&rx, |e| matches!(e, Event::SignInUrl { .. }));
    engine.send(Intent::CancelSignIn {
        agent: agent.clone(),
    });
    engine.send(Intent::SubmitSignInCode {
        agent,
        code: "abc#state".into(),
    });
    assert!(rx.recv_timeout(Duration::from_millis(700)).is_err());
    assert!(!marker.exists());
}

/// Codex's account through the fake agent's `login status` / `login` / `logout`: the address
/// is the line on stderr, the browser coming back signs in by itself (no code), and the state
/// is asked again after each.
#[test]
fn codex_signs_in_and_out_through_its_own_commands() {
    let (tx, rx) = channel();
    let fake = env!("CARGO_BIN_EXE_fake-agent").to_string();
    let programs = HashMap::from([("codex".to_string(), fake)]);
    let config = quill_engine::Config {
        programs,
        database: None,
        snapshots: None,
        settings: None,
        ..Default::default()
    };
    let engine = Engine::start_with(move |event| drop(tx.send(event)), config);
    let marker = workdir("codex-auth").join("marker");
    let mut settings = quill_model::Settings::default();
    settings.agents.insert(
        "codex".into(),
        quill_model::AgentSettings {
            env: vec![
                "QUILL_FAKE_AUTH=signed-out".into(),
                "QUILL_FAKE_LOGIN_MS=100".into(),
                format!("QUILL_FAKE_AUTH_MARKER={}", marker.display()),
            ],
            ..Default::default()
        },
    );
    engine.send(Intent::SaveSettings(settings));
    let agent = "codex".to_string();
    engine.send(Intent::CheckSignIn {
        agent: agent.clone(),
    });
    assert_eq!(state(&rx), quill_model::AuthState::SignedOut);
    engine.send(Intent::SignIn {
        agent: agent.clone(),
        method: "chatgpt".into(),
    });
    let seen = until(&rx, |e| matches!(e, Event::SignInUrl { .. }));
    assert!(matches!(
        seen.last(),
        Some(Event::SignInUrl { url, .. }) if url.starts_with("https://auth.openai.com/oauth/authorize?")
    ));
    assert!(matches!(
        state(&rx),
        quill_model::AuthState::SignedIn { .. }
    ));
    engine.send(Intent::SignOut { agent });
    assert_eq!(state(&rx), quill_model::AuthState::SignedOut);
}

/// Codex's captures through the same fake agent: the adapter's lines pace the replay, one per
/// `out` of the capture (its `model/list` is answered by the fake agent itself).
#[test]
fn codex_captures_replay_through_the_engine() {
    let (tx, rx) = channel();
    let programs = HashMap::from([(
        "codex".to_string(),
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
    let session = |name: &str, id: u64, text: &str| {
        let dir = workdir("simple");
        let capture = format!(
            "{}/../quill-agent-codex/fixtures/0.163.0/{name}.jsonl",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::write(dir.join(".fake-agent"), capture).unwrap();
        engine.send(Intent::NewSession(NewSession {
            agent: "codex".into(),
            cwd: dir,
            model: Some("gpt-6-luna".into()),
            effort: Some("low".into()),
            mode: None,
        }));
        engine.send(Intent::Send {
            session: id,
            text: text.into(),
            mentions: vec![],
            images: vec![],
            lane: Lane::Steer,
        });
    };
    let finished = |e: &Event| {
        matches!(
            e,
            Event::Agent {
                event: AgentEvent::TurnFinished { ok: true, .. },
                ..
            }
        )
    };

    session("simple", 1, "Reply with exactly the word: ok");
    let events = until(&rx, finished);
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Agent { event: AgentEvent::Text { text, .. }, .. } if text == "ok"
    )));

    session(
        "deny",
        2,
        "Use the shell tool to run exactly: touch denied.txt",
    );
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
        event: AgentEvent::PermissionRequest { id, tool, .. },
        ..
    }) = events.last()
    else {
        unreachable!()
    };
    assert_eq!(tool, "Bash");
    engine.send(Intent::Decide {
        session: 2,
        permission: id.clone(),
        decision: Decision::Deny {
            message: "no".into(),
        },
    });
    until(&rx, finished);
}
