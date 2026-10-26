use super::*;
use limn::{UiAction, UiTree};
use quill_model::{AgentEvent, Usage};

fn tab(title: &str, busy: bool) -> TerminalTab {
    TerminalTab {
        title: title.into(),
        busy,
        named: false,
    }
}

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
fn always_allow_applies_every_suggestion_and_leads_the_next_card() {
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
    assert_eq!(kept.len(), 2);
    assert_eq!(ui.last_allow, Allow::Always);
}

#[test]
fn ctrl_enter_answers_the_waiting_permission_from_anywhere() {
    let (mut store, ui) = store_with_session();
    assert_eq!(shortcut(&store, &ui, limn::Key::Enter, true, false), None);
    agent(
        &mut store,
        AgentEvent::PermissionRequest {
            id: "p1".into(),
            call: Some("t1".into()),
            tool: "Bash".into(),
            input: serde_json::json!({ "command": "touch a" }),
            title: None,
            interactive: false,
            suggestions: vec![],
        },
    );
    assert_eq!(
        shortcut(&store, &ui, limn::Key::Enter, true, false).as_deref(),
        Some("perm:p1:once")
    );
    assert_eq!(
        shortcut(&store, &ui, limn::Key::Enter, true, true).as_deref(),
        Some("perm:p1:always")
    );
    assert_eq!(shortcut(&store, &ui, limn::Key::Enter, false, false), None);
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
        vec![
            Intent::Send {
                session: 7,
                text: "hi".into()
            },
            Intent::WorkingTree { session: 7 }
        ]
    );
    assert_eq!(ui.selected, Some(7));
}

#[test]
fn the_window_lays_out_with_a_finished_turn_and_a_waiting_permission() {
    let (mut store, ui) = store_with_session();
    agent(
        &mut store,
        AgentEvent::TurnFinished {
            ok: true,
            interrupted: false,
            usage: Usage::default(),
            error: None,
        },
    );
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
            input: serde_json::json!({ "command": "ls", "description": "List files" }),
            title: None,
            interactive: false,
            suggestions: vec![Suggestion::Rules(serde_json::json!({ "type": "addRules" }))],
        },
    );
    let window = limn::Size::new(1408.0, 844.0);
    let mut tree = UiTree::new(view(&store, &ui, window));
    let frame = tree.layout(window);
    for id in [
        "perm:p1:deny",
        "perm:p1:menu",
        "perm:p1:once",
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

#[test]
fn the_terminal_toggles_and_takes_a_third_beside_the_conversation() {
    let store = Store::default();
    let mut ui = Ui::new("/tmp".into());
    let click = limn::UiAction::Clicked {
        id: view::TERMINAL_TOGGLE.into(),
    };
    update(&store, &mut ui, &click);
    assert!(ui.pane_open(Pane::Terminal));
    // Desktop at 1408×844: a 373 px column, its shell 12 px in from each side.
    let (cols, rows) = view::terminal_grid(&store, &ui, limn::Size::new(1408.0, 844.0));
    assert!((45..=50).contains(&cols) && rows > 40, "{cols}x{rows}");
    update(&store, &mut ui, &click);
    assert!(!ui.pane_open(Pane::Terminal));
}

#[test]
fn closing_a_terminal_with_a_running_command_asks_first() {
    let store = Store::default();
    let mut ui = Ui::new("/tmp".into());
    ui.terminal.tabs = vec![tab("Terminal 1", false), tab("Terminal 2", true)];
    let click = |ui: &mut Ui, id: &str| {
        update(&store, ui, &limn::UiAction::Clicked { id: id.into() });
    };
    click(&mut ui, "terminal:close:0");
    assert_eq!(ui.terminal.requests, [TerminalRequest::Close(0)]);
    ui.terminal.requests.clear();
    click(&mut ui, "terminal:close:1");
    assert_eq!(
        (ui.terminal.closing, ui.terminal.requests.len()),
        (Some(Closing::One(1)), 0)
    );
    click(&mut ui, view::TERMINAL_CLOSE_CANCEL);
    assert_eq!(ui.terminal.closing, None);
    click(&mut ui, "terminal:close:1");
    click(&mut ui, view::TERMINAL_CLOSE_CONFIRM);
    assert_eq!(ui.terminal.requests, [TerminalRequest::Close(1)]);
}

#[test]
fn the_terminal_pane_and_its_tabs_fit_in_the_window() {
    let store = Store::default();
    let mut ui = Ui::new("/tmp".into());
    ui.open_pane(Pane::Terminal);
    ui.terminal.tabs = vec![tab("Terminal 1", false), tab("Terminal 2", false)];
    ui.terminal.screen = Some(std::sync::Arc::new(limn::TerminalScreen {
        cols: 80,
        rows: 14,
        cells: vec![limn::TerminalCell::new('x', limn::Color::rgb(0, 0, 0)); 80 * 14],
        scrollback: vec![],
        cursor: None,
    }));
    let window = limn::Size::new(1400.0, 844.0);
    let mut tree = UiTree::new(view(&store, &ui, window));
    let frame = tree.layout(window);
    for id in [
        view::TERMINAL,
        view::TERMINAL_NEW,
        "terminal:close:0",
        "terminal:tab:1",
    ] {
        let region = frame
            .regions
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no {id}"));
        let r = region.rect;
        assert!(
            r.width > 0.0 && r.x + r.width <= 1400.0 && r.y + r.height <= 844.0,
            "{id} at {r:?}"
        );
    }
}

#[test]
fn one_terminal_shows_its_folder_and_more_show_as_pills() {
    let store = Store::default();
    let mut ui = Ui::new("/tmp/work".into());
    ui.open_pane(Pane::Terminal);
    ui.terminal.tabs = vec![tab("Terminal 1", false)];
    ui.terminal.screen = Some(std::sync::Arc::default());
    let window = limn::Size::new(1408.0, 844.0);
    let frame = UiTree::new(view(&store, &ui, window)).layout(window);
    assert!(!frame.regions.iter().any(|r| r.id == "terminal:close:0"));
    ui.terminal.tabs.push(tab("Terminal 2", false));
    ui.terminal.screen = Some(std::sync::Arc::default());
    let window = limn::Size::new(1400.0, 844.0);
    let frame = UiTree::new(view(&store, &ui, window)).layout(window);
    let close = frame
        .regions
        .iter()
        .find(|r| r.id == "terminal:close:0")
        .unwrap();
    assert!(close.rect.width <= 20.0, "{:?}", close.rect);
}

#[test]
fn a_permission_read_back_shows_its_answer() {
    // A session read back from the store has no intents, only events.
    let (mut store, _) = store_with_session();
    agent(
        &mut store,
        AgentEvent::PermissionRequest {
            id: "p1".into(),
            call: Some("t1".into()),
            tool: "Bash".into(),
            input: serde_json::json!({ "command": "touch a" }),
            title: None,
            interactive: false,
            suggestions: vec![],
        },
    );
    store.apply(Event::Decided {
        session: 1,
        permission: "p1".into(),
        decision: quill_model::Decision::Allow,
    });
    let decided = store
        .session(1)
        .unwrap()
        .items
        .iter()
        .find_map(|i| match i {
            Item::Permission { id, decided, .. } if id == "p1" => Some(*decided),
            _ => None,
        });
    assert_eq!(decided, Some(Some("Allowed")));
}

#[test]
fn the_sidebar_groups_sessions_by_folder_and_folds_a_group() {
    let mut store = Store::default();
    for (session, cwd) in [(1, "/work"), (2, "/other"), (3, "/work")] {
        store.apply(Event::SessionCreated {
            session,
            agent: "claude".into(),
            cwd: cwd.into(),
        });
    }
    let mut ui = Ui::new("/work".into());
    ui.group_by = GroupBy::Folder;
    let window = limn::Size::new(1408.0, 844.0);
    let ids = |store: &Store, ui: &Ui| -> Vec<String> {
        let frame = UiTree::new(view(store, ui, window)).layout(window);
        frame
            .regions
            .iter()
            .filter(|r| r.id.starts_with("group:") || r.id.starts_with("session:"))
            .map(|r| r.id.clone())
            .collect()
    };
    // The folder with the newest session comes first, its sessions newest first.
    assert_eq!(
        ids(&store, &ui),
        [
            "group:/work",
            "session:3",
            "session:1",
            "group:/other",
            "session:2"
        ]
    );
    update(
        &store,
        &mut ui,
        &UiAction::Clicked {
            id: "group:/work".into(),
        },
    );
    assert_eq!(
        ids(&store, &ui),
        ["group:/work", "group:/other", "session:2"]
    );
}

#[test]
fn a_finished_turn_closes_the_permissions_it_left_open() {
    let (mut store, ui) = store_with_session();
    agent(
        &mut store,
        AgentEvent::PermissionRequest {
            id: "p1".into(),
            call: Some("t1".into()),
            tool: "Bash".into(),
            input: serde_json::json!({ "command": "touch a" }),
            title: None,
            interactive: false,
            suggestions: vec![],
        },
    );
    agent(
        &mut store,
        AgentEvent::TurnFinished {
            ok: false,
            interrupted: true,
            usage: Usage::default(),
            error: None,
        },
    );
    assert_eq!(shortcut(&store, &ui, limn::Key::Enter, true, false), None);
}

#[test]
fn dragging_the_boundary_resizes_the_panes_within_their_minimums() {
    let store = Store::default();
    let mut ui = Ui::new("/tmp".into());
    ui.open_pane(Pane::Terminal);
    let drag = |ui: &mut Ui, x: f32| {
        update(
            &store,
            ui,
            &UiAction::DragMoved {
                id: view::COLUMN_HANDLE.into(),
                position: limn::Point::new(x, 400.0),
                over: None,
            },
        );
    };
    // From a third (373 px at 1408) to the boundary at x = 900: 508 px.
    drag(&mut ui, 900.0);
    let rest = 1408.0 - look::SIDEBAR_WIDTH;
    assert_eq!((ui.column_fraction.unwrap() * rest).round(), 508.0);
    // Never so far that the conversation gets under its minimum.
    drag(&mut ui, 300.0);
    assert_eq!(
        (ui.column_fraction.unwrap() * rest).round(),
        rest - view::MIN_CHAT
    );
}

#[test]
fn tab_menu_renames_closes_others_and_keys_move_tabs() {
    let store = Store::default();
    let mut ui = Ui::new("/tmp".into());
    ui.terminal.tabs = vec![tab("Terminal 1", true), tab("Terminal 2", false)];
    let click = |ui: &mut Ui, id: &str| {
        update(&store, ui, &UiAction::Clicked { id: id.into() });
    };
    click(&mut ui, "terminal:menu:rename:1");
    update(
        &store,
        &mut ui,
        &UiAction::Submitted {
            id: view::TERMINAL_RENAME.into(),
            value: "build".into(),
        },
    );
    assert_eq!(
        ui.terminal.requests,
        [TerminalRequest::Rename(1, Some("build".into()))]
    );
    ui.terminal.requests.clear();
    // Another tab runs a command: closing the others asks first, with Desktop's wording.
    click(&mut ui, "terminal:menu:others:1");
    assert_eq!(ui.terminal.closing, Some(Closing::Others(1)));
    click(&mut ui, view::TERMINAL_CLOSE_CONFIRM);
    assert_eq!(ui.terminal.requests, [TerminalRequest::CloseOthers(1)]);
    ui.terminal.requests.clear();
    assert!(terminal_key(
        &mut ui,
        Some("terminal:tab:1"),
        limn::Key::ArrowLeft,
        true,
        true
    ));
    assert_eq!(
        ui.terminal.requests,
        [TerminalRequest::Move { from: 1, to: 0 }]
    );
    assert!(!terminal_key(
        &mut ui,
        Some("composer"),
        limn::Key::Delete,
        false,
        false
    ));
}

#[test]
fn steps_are_named_in_desktops_words() {
    use view::{group_summary, line_diff, past_tense, progressive};
    // From Desktop's transcript.
    assert_eq!(
        past_tense("Run saludo.py to verify output"),
        "Ran saludo.py to verify output"
    );
    assert_eq!(
        past_tense("Create atajo.txt in /work and confirm it exists"),
        "Created atajo.txt in /work and confirm it exists"
    );
    assert_eq!(
        progressive("Create atajo2.txt in /work"),
        "Creating atajo2.txt in /work"
    );
    assert_eq!(past_tense("python3 x.py"), "python3 x.py");
    let done = ToolResult {
        output: String::new(),
        is_error: false,
        added: 1,
        removed: 1,
        files: vec![],
        stopped: false,
    };
    let read = serde_json::json!({ "file_path": "/work/saludo.py" });
    let bash =
        serde_json::json!({ "command": "python3 saludo.py", "description": "Run saludo.py" });
    let steps = [
        view::Step {
            call: "1",
            name: "Read",
            input: &read,
            result: Some(&done),
            running: false,
        },
        view::Step {
            call: "2",
            name: "Edit",
            input: &read,
            result: Some(&done),
            running: false,
        },
        view::Step {
            call: "3",
            name: "Bash",
            input: &bash,
            result: Some(&done),
            running: false,
        },
    ];
    assert_eq!(
        group_summary(&steps),
        "Read and edited saludo.py, ran a command"
    );
    assert_eq!(group_summary(&steps[2..]), "Ran saludo.py");
    let lines = line_diff("a\nb\nc\n", "a\nB\nc\n");
    let kinds: Vec<_> = lines.iter().map(|l| l.kind).collect();
    use limn::CodeDiffLineKind::*;
    assert_eq!(kinds, [Context, Added, Removed, Context]);
}

#[test]
fn limits_reset_as_desktop_says_it() {
    use view::resets_in;
    let now = 1_792_900_000;
    assert_eq!(
        resets_in(now + 4 * 3_600 + 53 * 60, now, 0),
        "Resets in 4 hr 53 min"
    );
    assert_eq!(resets_in(now + 300, now, 0), "Resets in 5 min");
    // 1793512800 is Sunday 2026-11-01 06:00 UTC: 3:00 AM at UTC-3.
    assert_eq!(
        resets_in(1_793_512_800, now, -3 * 3_600),
        "Resets Sun 3:00 AM"
    );
}

#[test]
fn running_clock_reads_as_desktop_writes_it() {
    assert_eq!(view::elapsed(14), "14s");
    assert_eq!(view::elapsed(9 * 60 + 4), "9m 4s");
}

#[test]
fn every_icon_is_a_complete_svg_on_the_20_grid() {
    let sheet = std::env::var_os("QUILL_ICON_SHEET");
    for (name, svg) in icons::ALL {
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"), "{name}");
        assert!(svg.contains("viewBox='0 0 20 20'"), "{name}");
        if let Some(dir) = &sheet {
            std::fs::write(std::path::Path::new(dir).join(format!("{name}.svg")), svg).unwrap();
        }
    }
}

fn tool(store: &mut Store, call: &str, name: &str, input: serde_json::Value, parent: Option<&str>) {
    agent(
        store,
        AgentEvent::ToolUse {
            call: call.into(),
            name: name.into(),
            input,
            parent: parent.map(str::to_owned),
        },
    );
}

fn result(store: &mut Store, call: &str, output: &str) {
    agent(
        store,
        AgentEvent::ToolResult {
            call: call.into(),
            output: output.into(),
            is_error: false,
            files: vec![],
            command: None,
        },
    );
}

#[test]
fn the_task_tools_keep_a_list_and_no_steps() {
    let (mut store, _) = store_with_session();
    for (call, subject) in [("c1", "Create a.txt"), ("c2", "List the files")] {
        tool(
            &mut store,
            call,
            "TaskCreate",
            serde_json::json!({ "subject": subject }),
            None,
        );
    }
    result(
        &mut store,
        "c1",
        "Task #1 created successfully: Create a.txt",
    );
    result(
        &mut store,
        "c2",
        "Task #2 created successfully: List the files",
    );
    tool(
        &mut store,
        "u1",
        "TaskUpdate",
        serde_json::json!({ "taskId": "1", "status": "completed" }),
        None,
    );
    tool(
        &mut store,
        "u2",
        "TaskUpdate",
        serde_json::json!({ "taskId": "2", "status": "in_progress" }),
        None,
    );
    let list = view::todos(&store.session(1).unwrap().items);
    let shown: Vec<_> = list
        .iter()
        .map(|t| (t.subject.as_str(), t.status.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            ("Create a.txt", "completed"),
            ("List the files", "in_progress")
        ]
    );
    assert!(view::is_task_tool("TaskUpdate") && !view::is_task_tool("Bash"));
}

#[test]
fn a_helper_agent_keeps_its_steps_and_counts_as_running_work() {
    let (mut store, _) = store_with_session();
    tool(
        &mut store,
        "a1",
        "Agent",
        serde_json::json!({ "description": "List files in folder" }),
        None,
    );
    agent(
        &mut store,
        AgentEvent::TaskStarted {
            id: "t1".into(),
            call: Some("a1".into()),
            description: "List files in folder".into(),
        },
    );
    tool(
        &mut store,
        "b1",
        "Bash",
        serde_json::json!({ "command": "ls" }),
        Some("a1"),
    );
    result(&mut store, "b1", "README.md");
    let session = store.session(1).unwrap();
    // The helper's step is its own, not the conversation's, and it got its result.
    assert!(!session
        .items
        .iter()
        .any(|i| matches!(i, Item::Tool { call, .. } if call == "b1")));
    assert!(matches!(
        &session.helpers["a1"][..],
        [Item::Tool {
            result: Some(_),
            ..
        }]
    ));
    assert!(session.task_running("a1"));
    let input = serde_json::json!({ "description": "List files in folder" });
    let step = view::Step {
        call: "a1",
        name: "Agent",
        input: &input,
        result: None,
        running: true,
    };
    assert_eq!(view::group_summary(&[step]), "Listing files in folder");
    agent(
        &mut store,
        AgentEvent::TaskEnded {
            id: "t1".into(),
            call: Some("a1".into()),
            summary: Some("Two files.".into()),
            ok: true,
        },
    );
    assert!(!store.session(1).unwrap().task_running("a1"));
}

#[test]
fn a_finished_group_counts_agents_and_other_tools() {
    let done = ToolResult {
        output: String::new(),
        is_error: false,
        added: 0,
        removed: 0,
        files: vec![],
        stopped: false,
    };
    let read = serde_json::json!({ "file_path": "/work/saludo.py" });
    let agent_input = serde_json::json!({ "description": "List files" });
    let other = serde_json::json!({});
    let step = |call, name, input| view::Step {
        call,
        name,
        input,
        result: Some(&done),
        running: false,
    };
    let steps = [
        step("1", "Read", &read),
        step("2", "Agent", &agent_input),
        step("3", "WebFetch", &other),
        step("4", "Skill", &other),
    ];
    assert_eq!(
        view::group_summary(&steps),
        "Read saludo.py, ran an agent, used 2 tools"
    );
}

#[test]
fn the_running_line_writes_time_and_tokens_as_desktop() {
    assert_eq!(view::elapsed(3_723), "1h 2m 3s");
    assert_eq!(view::tokens(140), "140 tokens");
    assert_eq!(view::tokens(30_712), "30.7k tokens");
}

#[test]
fn an_answer_is_read_back_from_the_tool_result() {
    let output = "The user answered: \"Pick red or blue?\"=\"blue\". Read the answers carefully.";
    assert_eq!(
        view::answered(output, "Pick red or blue?").as_deref(),
        Some("blue")
    );
    assert_eq!(view::answered(output, "Another?"), None);
}

#[test]
fn questions_page_take_several_choices_their_own_words_and_skips() {
    let (mut store, mut ui) = store_with_session();
    agent(
        &mut store,
        AgentEvent::PermissionRequest {
            id: "p1".into(),
            call: None,
            tool: "AskUserQuestion".into(),
            input: serde_json::json!({ "questions": [
                { "question": "Pets?", "multiSelect": true,
                  "options": [{ "label": "Cat" }, { "label": "Dog" }, { "label": "Fish" }] },
                { "question": "Color?", "options": [{ "label": "Red" }, { "label": "Blue" }] },
                { "question": "Size?", "options": [{ "label": "S" }] }] }),
            title: None,
            interactive: true,
            suggestions: vec![],
        },
    );
    let click = |ui: &mut Ui, id: &str| update(&store, ui, &UiAction::Clicked { id: id.into() });
    click(&mut ui, "ask:p1:0:0");
    click(&mut ui, "ask:p1:0:2");
    click(&mut ui, "ask:p1:0:1");
    click(&mut ui, "ask:p1:0:1");
    assert!(click(&mut ui, "ask:p1:next").is_empty());
    // "Other" is the option after the last, and takes what the person wrote.
    click(&mut ui, "ask:p1:1:2");
    update(
        &store,
        &mut ui,
        &UiAction::InputChanged {
            id: "ask-other:p1:1".into(),
            value: "Green".into(),
        },
    );
    let click = |ui: &mut Ui, id: &str| update(&store, ui, &UiAction::Clicked { id: id.into() });
    assert!(click(&mut ui, "ask:p1:next").is_empty());
    let intents = click(&mut ui, "ask:p1:skip");
    let [Intent::Decide {
        decision: Decision::Answer(answers),
        ..
    }] = intents.as_slice()
    else {
        panic!("{intents:?}")
    };
    assert_eq!(answers["Pets?"], "Cat, Fish");
    assert_eq!(answers["Color?"], "Green");
    assert_eq!(answers["Size?"], "[No preference]");
}

#[test]
fn accepting_a_plan_allows_it_and_sets_the_mode() {
    let (mut store, mut ui) = store_with_session();
    agent(
        &mut store,
        AgentEvent::PermissionRequest {
            id: "p2".into(),
            call: Some("x1".into()),
            tool: "ExitPlanMode".into(),
            input: serde_json::json!({ "plan": "# Plan" }),
            title: None,
            interactive: true,
            suggestions: vec![],
        },
    );
    assert_eq!(
        shortcut(&store, &ui, limn::Key::Enter, true, true).as_deref(),
        Some("planok:p2:default")
    );
    let intents = update(
        &store,
        &mut ui,
        &UiAction::Clicked {
            id: "planok:p2:acceptEdits".into(),
        },
    );
    assert!(matches!(
        intents.as_slice(),
        [
            Intent::Decide {
                decision: Decision::Allow,
                ..
            },
            Intent::SetMode { mode, .. }
        ] if mode == "acceptEdits"
    ));
    assert_eq!(
        view::plan_text(&store.session(1).unwrap().items, "p2").as_deref(),
        Some("# Plan")
    );
}

fn click(store: &Store, ui: &mut Ui, id: &str) -> Vec<Intent> {
    update(store, ui, &UiAction::Clicked { id: id.into() })
}

#[test]
fn a_session_is_renamed_pinned_and_deleted_from_its_menus() {
    let (mut store, mut ui) = store_with_session();
    // Rename starts from the current title, and Enter sends what the field holds.
    click(&store, &mut ui, "rename:1");
    assert_eq!(ui.title_edit.as_deref(), Some("Fix the bug"));
    let renamed = update(
        &store,
        &mut ui,
        &UiAction::Submitted {
            id: view::TITLE_FIELD.into(),
            value: " Bug hunt ".into(),
        },
    );
    assert_eq!(
        renamed,
        vec![Intent::Rename {
            session: 1,
            title: "Bug hunt".into()
        }]
    );
    assert!(ui.title_edit.is_none());
    store.apply(Event::Renamed {
        session: 1,
        title: "Bug hunt".into(),
    });
    assert_eq!(store.session(1).unwrap().title, "Bug hunt");

    assert_eq!(
        click(&store, &mut ui, "pin:1"),
        vec![Intent::Pin {
            session: 1,
            pinned: true
        }]
    );
    store.apply(Event::Pinned {
        session: 1,
        pinned: true,
    });
    assert_eq!(
        click(&store, &mut ui, "pin:1"),
        vec![Intent::Pin {
            session: 1,
            pinned: false
        }]
    );

    // A delete asks first; only the confirm reaches the engine.
    assert!(click(&store, &mut ui, "delete:1").is_empty());
    assert_eq!(ui.deleting, Some(1));
    assert!(click(&store, &mut ui, view::DELETE_CANCEL).is_empty());
    click(&store, &mut ui, "delete:1");
    assert_eq!(
        click(&store, &mut ui, view::DELETE_CONFIRM),
        vec![Intent::Delete { session: 1 }]
    );
    let deleted = Event::Deleted { session: 1 };
    store.apply(deleted.clone());
    ui.applied(&deleted);
    assert!(store.sessions.is_empty());
    assert_eq!(ui.selected, None);
}

#[test]
fn search_lists_waiting_sessions_first_and_enter_opens_the_first() {
    let mut store = Store::default();
    for (session, title, status) in [
        (1, "Fix login", SessionStatus::Idle),
        (2, "Fix logout", SessionStatus::Awaiting),
        (3, "Write docs", SessionStatus::Running),
    ] {
        store.apply(Event::SessionCreated {
            session,
            agent: "claude".into(),
            cwd: "/work".into(),
        });
        store.apply(Event::Renamed {
            session,
            title: title.into(),
        });
        store.apply(Event::Status { session, status });
    }
    assert_eq!(view::found(&store, ""), vec![2, 3, 1]);
    assert_eq!(view::found(&store, "FIX"), vec![2, 1]);
    let mut ui = Ui::new("/work".into());
    click(&store, &mut ui, view::SEARCH);
    update(
        &store,
        &mut ui,
        &UiAction::InputChanged {
            id: view::SEARCH_FIELD.into(),
            value: "login".into(),
        },
    );
    update(
        &store,
        &mut ui,
        &UiAction::Submitted {
            id: view::SEARCH_FIELD.into(),
            value: "login".into(),
        },
    );
    assert_eq!(ui.selected, Some(1));
    assert!(ui.search.is_none());
}

#[test]
fn a_hidden_sidebar_gives_its_width_to_the_conversation() {
    let (store, mut ui) = store_with_session();
    let window = limn::Size::new(1408.0, 844.0);
    let mut tree = UiTree::new(view(&store, &ui, window));
    tree.layout(window);
    click(&store, &mut ui, view::SIDEBAR_TOGGLE);
    assert!(!ui.sidebar_open);
    // It slides away on a spring and has nothing left in a second.
    ui.tick(&store);
    let later = std::time::Instant::now() + std::time::Duration::from_secs(1);
    assert_eq!(ui.sidebar_shown.target(), 0.0);
    assert_eq!(ui.sidebar_shown.value(later), 0.0);
    tree.set_root(view(&store, &ui, window));
    tree.layout(window);
}

#[test]
fn an_archived_session_leaves_the_list_and_the_search_still_finds_it() {
    let (mut store, mut ui) = store_with_session();
    assert_eq!(
        click(&store, &mut ui, "archive:1"),
        vec![Intent::Archive {
            session: 1,
            archived: true
        }]
    );
    store.apply(Event::Archived {
        session: 1,
        archived: true,
    });
    let window = limn::Size::new(1408.0, 844.0);
    let mut tree = UiTree::new(view(&store, &ui, window));
    let frame = tree.layout(window);
    assert!(!frame.regions.iter().any(|r| r.id == "session:1"));
    assert_eq!(view::found(&store, "bug"), vec![1]);
    assert_eq!(
        click(&store, &mut ui, "archive:1"),
        vec![Intent::Archive {
            session: 1,
            archived: false
        }]
    );
}

#[test]
fn a_turn_that_ends_unseen_leaves_the_session_unread_until_it_is_opened() {
    let (mut store, mut ui) = store_with_session();
    let finished = Event::Agent {
        session: 1,
        request: Some("r1".into()),
        event: AgentEvent::TurnFinished {
            ok: true,
            interrupted: false,
            usage: Usage::default(),
            error: None,
        },
    };
    ui.selected = None;
    // Read back at start, a finished turn is old news.
    assert!(ui.applied(&finished).is_empty());
    ui.applied(&Event::Status {
        session: 1,
        status: SessionStatus::Running,
    });
    assert_eq!(
        ui.applied(&finished),
        vec![Intent::MarkUnread {
            session: 1,
            unread: true
        }]
    );
    store.apply(Event::Unread {
        session: 1,
        unread: true,
    });
    let opened = click(&store, &mut ui, "session:1");
    assert!(opened.contains(&Intent::MarkUnread {
        session: 1,
        unread: false
    }));
}

#[test]
fn a_fork_starts_with_its_parents_conversation_and_opens() {
    let (mut store, mut ui) = store_with_session();
    assert_eq!(
        click(&store, &mut ui, "fork:1"),
        vec![Intent::Fork { session: 1 }]
    );
    let created = Event::SessionCreated {
        session: 2,
        agent: "claude".into(),
        cwd: "/work".into(),
    };
    assert_eq!(
        ui.applied(&created),
        vec![Intent::WorkingTree { session: 2 }]
    );
    assert_eq!(ui.selected, Some(2));
    store.apply(created);
    store.apply(Event::Forked {
        session: 2,
        from: 1,
    });
    let fork = store.session(2).unwrap();
    assert_eq!(fork.items, store.session(1).unwrap().items);
    assert_eq!(fork.title, "Fix the bug (fork)");
}

#[test]
fn the_list_groups_by_status_and_sorts_by_name() {
    let mut store = Store::default();
    for (session, title, status) in [
        (1, "beta", SessionStatus::Idle),
        (2, "alpha", SessionStatus::Awaiting),
        (3, "gamma", SessionStatus::Idle),
    ] {
        store.apply(Event::SessionCreated {
            session,
            agent: "claude".into(),
            cwd: "/work".into(),
        });
        store.apply(Event::Renamed {
            session,
            title: title.into(),
        });
        store.apply(Event::Status { session, status });
    }
    let mut ui = Ui::new("/work".into());
    click(&store, &mut ui, "group-by:status");
    click(&store, &mut ui, "sort-by:name");
    click(&store, &mut ui, "sort-way:ascending");
    let window = limn::Size::new(1408.0, 844.0);
    let mut tree = UiTree::new(view(&store, &ui, window));
    let frame = tree.layout(window);
    let y = |id: &str| frame.regions.iter().find(|r| r.id == id).unwrap().rect.y;
    // Needs input comes first, then Completed sorted A to Z.
    assert!(y("group:status:Needs input") < y("session:2"));
    assert!(y("session:2") < y("group:status:Completed"));
    assert!(y("session:1") < y("session:3"));
    click(&store, &mut ui, view::FILTER_RESET);
    assert_eq!(
        (ui.group_by, ui.sort_by, ui.ascending),
        (GroupBy::Status, SortBy::Updated, false)
    );
}

#[test]
fn the_branch_bar_writes_its_counts_with_commas() {
    assert_eq!(view::thousands(12), "12");
    assert_eq!(view::thousands(5318), "5,318");
    assert_eq!(view::thousands(1_234_567), "1,234,567");
}

#[test]
fn the_effort_thumb_follows_a_drag_and_springs_to_the_level_let_go_at() {
    let (mut store, mut ui) = store_with_session();
    agent(
        &mut store,
        AgentEvent::Ready {
            version: "2.1.310".into(),
            capabilities: quill_model::AgentCapabilities {
                models: vec![quill_model::ModelInfo {
                    id: "opus".into(),
                    name: "Opus 5.5".into(),
                    efforts: vec!["low".into(), "medium".into(), "high".into()],
                }],
                ..Default::default()
            },
            mode: None,
        },
    );
    ui.model = Some("opus".into());
    let slide = |ui: &mut Ui, fraction: f32, committed: bool| {
        update(
            &store,
            ui,
            &UiAction::SliderChanged {
                id: view::EFFORT_SLIDER.into(),
                fraction,
                committed,
            },
        )
    };
    // While dragged nothing is chosen: the thumb is where the pointer holds it.
    assert!(slide(&mut ui, 0.7, false).is_empty());
    assert!(ui.effort_dragging);
    let now = std::time::Instant::now();
    assert_eq!(
        ui.effort_thumb.unwrap().value(now),
        0.7 * view::EFFORT_TRAVEL
    );
    // Let go, the nearest level is chosen and the thumb travels on to it.
    assert_eq!(
        slide(&mut ui, 1.0, true),
        vec![Intent::SetEffort {
            session: 1,
            effort: "high".into()
        }]
    );
    assert!(!ui.effort_dragging);
    assert!(ui.animating());
    assert_eq!(ui.effort_thumb.unwrap().target(), view::EFFORT_TRAVEL);
}

#[test]
fn a_started_session_is_nested_under_its_parent_and_completed_shows_five_first() {
    let mut store = Store::default();
    for session in 1..=8 {
        store.apply(Event::SessionCreated {
            session,
            agent: "claude".into(),
            cwd: "/work".into(),
        });
        store.apply(Event::Status {
            session,
            status: SessionStatus::Idle,
        });
    }
    store.apply(Event::Linked {
        session: 8,
        parent: 1,
        side: false,
    });
    let mut ui = Ui::new("/work".into());
    let window = limn::Size::new(1408.0, 844.0);
    let regions = |store: &Store, ui: &Ui| {
        let mut tree = UiTree::new(view(store, ui, window));
        tree.layout(window)
            .regions
            .iter()
            .map(|r| (r.id.clone(), r.rect))
            .collect::<Vec<_>>()
    };
    let shown = regions(&store, &ui);
    let at = |id: &str| shown.iter().find(|r| r.0 == id).map(|r| r.1);
    // Seven top-level sessions: the five newest, then "Show more"; 1 is among those left.
    assert!(at("session:7").is_some() && at("session:3").is_some());
    assert!(at("session:2").is_none() && at("session:1").is_none());
    assert!(at(view::SHOW_MORE).is_some());
    click(&store, &mut ui, view::SHOW_MORE);
    let shown = regions(&store, &ui);
    let at = |id: &str| shown.iter().find(|r| r.0 == id).map(|r| r.1).unwrap();
    // The child sits right under its parent, further in.
    let (parent, child) = (at("session:1"), at("session:8"));
    assert!(child.y > parent.y && child.y < parent.y + 40.0);
    assert!(child.x > parent.x);
}

#[test]
fn proposals_share_a_card_that_starts_them_and_reports_read_back() {
    let (mut store, mut ui) = store_with_session();
    let proposal = |id: &str| quill_model::Proposal {
        id: id.into(),
        title: format!("Task {id}"),
        prompt: "Do it".into(),
        background: None,
        fork: false,
    };
    for id in ["a", "b"] {
        store.apply(Event::Proposed {
            session: 1,
            proposal: proposal(id),
        });
    }
    let cards: Vec<_> = store
        .session(1)
        .unwrap()
        .items
        .iter()
        .filter(|i| matches!(i, Item::Proposals { .. }))
        .collect();
    assert!(matches!(cards.as_slice(), [Item::Proposals { tasks }] if tasks.len() == 2));
    // Start all asks for each waiting task.
    assert_eq!(
        click(&store, &mut ui, "proposal-all:a,b"),
        ["a", "b"]
            .map(|task| Intent::AnswerProposal {
                session: 1,
                task: task.into(),
                answer: quill_model::ProposalAnswer::Start,
            })
            .to_vec()
    );
    store.apply(Event::ProposalAnswered {
        session: 1,
        task: "a".into(),
        outcome: quill_model::ProposalOutcome::Started { session: 2 },
    });
    store.apply(Event::Reported {
        session: 1,
        request: "r2".into(),
        reports: vec![quill_model::Report {
            from: 2,
            title: "Task a".into(),
            text: "All done".into(),
        }],
    });
    let window = limn::Size::new(1408.0, 844.0);
    let mut tree = UiTree::new(view(&store, &ui, window));
    let frame = tree.layout(window);
    // The started task follows its session; its report opens it and unfolds.
    assert!(frame.regions.iter().any(|r| r.id == "session:2"));
    assert!(frame.regions.iter().any(|r| r.id == "report:r2:0"));
    click(&store, &mut ui, "report:r2:0");
    assert!(ui.expanded.contains(&"report:r2:0".to_string()));
}

#[test]
fn btw_asks_beside_the_session_and_closing_puts_it_away() {
    let (mut store, mut ui) = store_with_session();
    let asked = update(
        &store,
        &mut ui,
        &UiAction::Submitted {
            id: view::COMPOSER.into(),
            value: "/btw what does it do?".into(),
        },
    );
    assert_eq!(
        asked,
        vec![Intent::SideChat {
            session: 1,
            text: "what does it do?".into()
        }]
    );
    let created = Event::SessionCreated {
        session: 2,
        agent: "claude".into(),
        cwd: "/work".into(),
    };
    // It opens in its pane; the session in view stays.
    assert!(ui.applied(&created).is_empty());
    store.apply(created);
    assert_eq!((ui.selected, ui.side_chat), (Some(1), Some(2)));
    assert!(ui.pane_open(Pane::Side));
    store.apply(Event::Status {
        session: 2,
        status: SessionStatus::Idle,
    });
    assert_eq!(
        click(&store, &mut ui, "pane:side:close"),
        vec![Intent::Archive {
            session: 2,
            archived: true
        }]
    );
    assert!(!ui.pane_open(Pane::Side));
}
