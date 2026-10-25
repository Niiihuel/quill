use super::*;
use limn::{UiAction, UiTree};
use quill_model::{AgentEvent, Usage};

fn store_with_session() -> (Store, Ui) {
    let mut store = Store::default();
    store.apply(Event::SessionCreated {
        session: 1,
        agent: "claude".into(),
        cwd: "/work".into(),
    });
    store.apply(Event::Queued {
        session: 1,
        request: "r1".into(),
        text: "Fix the bug".into(),
    });
    let mut ui = Ui::new("/work".into());
    ui.selected = Some(1);
    (store, ui)
}

fn agent(store: &mut Store, event: AgentEvent) {
    store.apply(Event::Agent {
        session: 1,
        request: Some("r1".into()),
        event,
    });
}

#[test]
fn streamed_text_becomes_one_message() {
    let (mut store, _) = store_with_session();
    agent(
        &mut store,
        AgentEvent::TextDelta {
            text: "Hel".into(),
            parent: None,
        },
    );
    agent(
        &mut store,
        AgentEvent::TextDelta {
            text: "lo".into(),
            parent: None,
        },
    );
    agent(
        &mut store,
        AgentEvent::Text {
            text: "Hello".into(),
            parent: None,
        },
    );
    let items = &store.session(1).unwrap().items;
    assert_eq!(
        items.last(),
        Some(&Item::Assistant {
            text: "Hello".into(),
            streaming: false
        })
    );
    assert_eq!(store.session(1).unwrap().title, "Fix the bug");
}

#[test]
fn an_edit_counts_one_line_each_way() {
    assert_eq!(line_changes(Some("hello\n"), Some("hello world\n")), (1, 1));
    assert_eq!(line_changes(None, Some("a\nb\n")), (2, 0));
}

#[test]
fn always_allow_sends_only_rules() {
    let (mut store, mut ui) = store_with_session();
    agent(
        &mut store,
        AgentEvent::PermissionRequest {
            id: "p1".into(),
            call: Some("t1".into()),
            tool: "Bash".into(),
            input: serde_json::json!({ "command": "touch a" }),
            title: None,
            interactive: false,
            suggestions: vec![
                Suggestion::Rules(serde_json::json!({ "type": "addRules" })),
                Suggestion::Mode("acceptEdits".into()),
            ],
        },
    );
    let intents = update(
        &store,
        &mut ui,
        &UiAction::Clicked {
            id: "perm:p1:always".into(),
        },
    );
    let [Intent::Decide {
        decision: Decision::AllowAlways(kept),
        ..
    }] = intents.as_slice()
    else {
        panic!("{intents:?}")
    };
    assert_eq!(kept.len(), 1);
    assert!(matches!(kept[0], Suggestion::Rules(_)));
}

#[test]
fn the_first_message_creates_a_session_then_sends_it() {
    let store = Store::default();
    let mut ui = Ui::new("/work".into());
    let intents = update(
        &store,
        &mut ui,
        &UiAction::Submitted {
            id: view::COMPOSER.into(),
            value: "hi".into(),
        },
    );
    assert!(matches!(intents.as_slice(), [Intent::NewSession(_)]));
    let then = ui.applied(&Event::SessionCreated {
        session: 7,
        agent: "claude".into(),
        cwd: "/work".into(),
    });
    assert_eq!(
        then,
        vec![Intent::Send {
            session: 7,
            text: "hi".into()
        }]
    );
    assert_eq!(ui.selected, Some(7));
}

#[test]
fn the_window_lays_out_with_a_permission_and_a_finished_turn() {
    let (mut store, ui) = store_with_session();
    agent(
        &mut store,
        AgentEvent::ToolUse {
            call: "t1".into(),
            name: "Bash".into(),
            input: serde_json::json!({ "command": "ls" }),
            parent: None,
        },
    );
    agent(
        &mut store,
        AgentEvent::PermissionRequest {
            id: "p1".into(),
            call: Some("t1".into()),
            tool: "Bash".into(),
            input: serde_json::json!({ "command": "ls" }),
            title: None,
            interactive: false,
            suggestions: vec![],
        },
    );
    agent(
        &mut store,
        AgentEvent::TurnFinished {
            ok: true,
            interrupted: false,
            usage: Usage::default(),
            error: None,
        },
    );
    let window = limn::Size::new(1408.0, 844.0);
    let mut tree = UiTree::new(view(&store, &ui, window));
    let frame = tree.layout(window);
    for id in [
        "perm:p1:deny",
        "perm:p1:menu",
        "perm:p1:always",
        view::COMPOSER,
        "session:1",
    ] {
        let region = frame
            .regions
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no {id}"));
        assert!(
            region.rect.x + region.rect.width <= 1408.0,
            "{id} outside the window"
        );
    }
}

#[test]
fn a_question_is_answered_with_the_picked_option() {
    let (mut store, mut ui) = store_with_session();
    agent(
        &mut store,
        AgentEvent::PermissionRequest {
            id: "p9".into(),
            call: None,
            tool: "AskUserQuestion".into(),
            input: serde_json::json!({ "questions": [{ "question": "Red or blue?", "header": "Color",
                "options": [{ "label": "Red" }, { "label": "Blue" }] }] }),
            title: None,
            interactive: true,
            suggestions: vec![],
        },
    );
    assert!(update(
        &store,
        &mut ui,
        &UiAction::Clicked {
            id: "ask:p9:0:1".into()
        }
    )
    .is_empty());
    let intents = update(
        &store,
        &mut ui,
        &UiAction::Clicked {
            id: "ask:p9:submit".into(),
        },
    );
    let [Intent::Decide {
        decision: Decision::Answer(answers),
        ..
    }] = intents.as_slice()
    else {
        panic!("{intents:?}")
    };
    assert_eq!(answers["Red or blue?"], "Blue");
}

#[test]
fn a_rewritten_line_marks_only_what_changed() {
    let lines = view::parse_patch("@@ -1,2 +1,2 @@\n-    print(\"Hola \" + nombre)\n+    print(f\"Hola, {nombre}!\")\n context\n");
    let words = view::word_ranges(&lines);
    assert_eq!(lines.len(), 3);
    // Common start "    print(" (10) and common end ")" (1).
    assert_eq!(words[0], vec![10..26]);
    assert_eq!(words[1], vec![10..28]);
    assert!(words[2].is_empty());
}
