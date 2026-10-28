//! Every test replays a capture of the real CLI (`fixtures/0.163.0/`, docs/codex.md "Fase 1").
//!
//! A capture is driven the way the engine would: the lines quill sent become the commands
//! that make the adapter write them, and the lines Codex sent are decoded. What the adapter
//! writes must be the capture's `out` lines, in number and order, because the fake agent
//! waits for one line per `out`.

use super::*;
use quill_agent::AgentEvent::*;
use std::path::{Path, PathBuf};

fn entries(name: &str) -> Vec<(String, Value)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/0.163.0/{name}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    (text.lines())
        .map(|line| {
            let entry: Value = serde_json::from_str(line).unwrap();
            (text_of(&entry["dir"]), entry["msg"].clone())
        })
        .collect()
}

fn text_of(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// One process of the adapter: what it wrote and what it told.
struct Run {
    codex: Codex,
    written: Vec<Value>,
    events: Vec<AgentEvent>,
    prompts: u32,
}

impl Run {
    fn new(spec: SessionSpec) -> Self {
        let codex = Codex::new();
        codex.launch(&SessionSpec {
            cwd: PathBuf::from("/work"),
            model: Some("gpt-6-luna".into()),
            effort: Some("low".into()),
            ..spec
        });
        Run {
            codex,
            written: vec![],
            events: vec![],
            prompts: 0,
        }
    }

    fn started(spec: SessionSpec) -> Self {
        let mut run = Self::new(spec);
        run.send(AgentCommand::Start);
        run
    }

    fn write(&mut self, lines: Vec<String>) {
        self.written.extend(
            lines
                .iter()
                .map(|l| serde_json::from_str::<Value>(l).unwrap()),
        );
    }

    fn send(&mut self, command: AgentCommand) {
        let lines = self.codex.encode(command);
        self.write(lines);
    }

    fn feed(&mut self, msg: &Value) {
        let decoded = self.codex.decode(&msg.to_string());
        self.events.extend(decoded.events);
        self.write(decoded.replies);
    }

    fn prompt(&mut self, text: &str, steer: bool) {
        self.prompts += 1;
        self.send(AgentCommand::Prompt {
            request: format!("r{}", self.prompts),
            text: text.into(),
            steer,
            images: vec![],
        });
    }

    /// The decision the capture answered a server request with.
    fn answer(&mut self, id: &Value, result: &Value) {
        let asked = self.events.iter().rev().find_map(|e| match e {
            PermissionRequest { id: p, input, .. } if *p == key(id) => Some(input.clone()),
            _ => None,
        });
        let decision = if let Some(answers) = result["answers"].as_object() {
            // One question, as the capture asked (ask.jsonl:30).
            let question = text_of(&asked.unwrap()["questions"][0]["question"]);
            let picked = answers.values().next().unwrap()["answers"][0].clone();
            Decision::Answer(json!({ question: picked }))
        } else {
            match result["decision"].as_str() {
                Some("acceptForSession") => Decision::AllowAlways(vec![]),
                Some("decline") => Decision::Deny {
                    message: String::new(),
                },
                _ => Decision::Allow,
            }
        };
        self.send(AgentCommand::Decide {
            id: key(id),
            decision,
        });
    }
}

/// Replay a capture of one process (up to an `exit`, from `from`); returns where it ended.
fn replay(run: &mut Run, entries: &[(String, Value)], from: usize) -> usize {
    let mut at = from;
    while at < entries.len() {
        let (dir, msg) = &entries[at];
        at += 1;
        match (dir.as_str(), msg["method"].as_str()) {
            ("exit", _) => return at,
            ("in", _) => run.feed(msg),
            ("out", Some("turn/start")) => {
                run.prompt(msg["params"]["input"][0]["text"].as_str().unwrap(), false)
            }
            ("out", Some("turn/steer")) => {
                run.prompt(msg["params"]["input"][0]["text"].as_str().unwrap(), true)
            }
            ("out", Some("turn/interrupt")) => run.send(AgentCommand::Interrupt),
            // A fork inside one process: quill forks by launching a new one (below).
            ("out", Some("thread/fork")) => {
                run.codex.next_id = msg["id"].as_u64().unwrap();
                run.codex.pending.insert(key(&msg["id"]), Pending::Thread);
            }
            // Start, `initialized`, the thread, the turn list: the adapter writes them itself.
            ("out", Some(_)) => {}
            ("out", None) => run.answer(&msg["id"], &msg["result"]),
            _ => {}
        }
    }
    at
}

fn methods(lines: &[Value]) -> Vec<String> {
    (lines.iter())
        .map(|l| l["method"].as_str().unwrap_or("response").to_owned())
        .collect()
}

/// The `out` lines of a capture, as the adapter must write them: the same methods, ids and
/// answers.
fn out_lines(entries: &[(String, Value)]) -> Vec<Value> {
    (entries.iter())
        .filter(|(dir, _)| dir == "out")
        .map(|(_, msg)| msg.clone())
        .collect()
}

/// Replay `name` as one process and check the adapter wrote the capture's `out` lines.
fn run_of(name: &str) -> Run {
    let entries = entries(name);
    let mut run = Run::started(SessionSpec::default());
    replay(&mut run, &entries, 0);
    // A fork asks the thread of the process it is made in: not a line of this run.
    let expected: Vec<Value> = out_lines(&entries)
        .into_iter()
        .filter(|l| l["method"] != "thread/fork")
        .collect();
    assert_eq!(
        methods(&run.written),
        methods(&expected),
        "{name}: lines written"
    );
    for (written, expected) in run.written.iter().zip(&expected) {
        assert_eq!(written["id"], expected["id"], "{name}: ids");
        // Only the request's identity and the answers are the capture's; the adapter sends more
        // parameters (policy, config) than the capture's driver did.
        if expected.get("result").is_some() {
            assert_eq!(written["result"], expected["result"], "{name}: answers");
        }
    }
    run
}

fn unknown(events: &[AgentEvent]) -> Vec<&Value> {
    (events.iter())
        .filter_map(|e| if let Unknown(v) = e { Some(v) } else { None })
        .collect()
}

fn calls(events: &[AgentEvent]) -> Vec<(&str, &Value)> {
    (events.iter())
        .filter_map(|e| match e {
            ToolUse { name, input, .. } => Some((name.as_str(), input)),
            _ => None,
        })
        .collect()
}

fn results(events: &[AgentEvent]) -> Vec<&AgentEvent> {
    events
        .iter()
        .filter(|e| matches!(e, ToolResult { .. }))
        .collect()
}

fn finished(events: &[AgentEvent]) -> Vec<(bool, bool)> {
    (events.iter())
        .filter_map(|e| match e {
            TurnFinished {
                ok, interrupted, ..
            } => Some((*ok, *interrupted)),
            _ => None,
        })
        .collect()
}

fn texts(events: &[AgentEvent]) -> Vec<&str> {
    (events.iter())
        .filter_map(|e| match e {
            Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

const SCENARIOS: [&str; 13] = [
    "simple",
    "edit-bash",
    "deny-patch",
    "deny",
    "always",
    "plan",
    "steer",
    "interrupt",
    "fork",
    "think",
    "websearch",
    "ask",
    "resume",
];

#[test]
fn every_fixture_writes_its_out_lines_and_decodes_without_unknown_messages() {
    for name in SCENARIOS {
        if matches!(name, "resume" | "fork") {
            continue; // two processes: their own tests
        }
        let run = run_of(name);
        assert_eq!(unknown(&run.events), Vec::<&Value>::new(), "{name}");
        assert_eq!(finished(&run.events).len(), 1, "{name}");
    }
}

#[test]
fn a_session_starts_with_its_id_and_what_codex_can_do() {
    let run = run_of("simple");
    assert!(
        matches!(&run.events[0], Session { id } if id == "01a1817c-3a5b-79d0-ae92-0f25db9b7c42")
    );
    let Ready {
        version,
        capabilities,
        mode,
    } = &run.events[1]
    else {
        panic!("{:?}", run.events[1]);
    };
    assert_eq!(version, "0.163.0");
    assert_eq!(mode.as_deref(), Some("auto"));
    assert_eq!(capabilities.modes, ["read-only", "auto"]);
    assert_eq!(capabilities.unsafe_modes, ["full-access"]);
    assert!(
        capabilities.steer && capabilities.resume && capabilities.fork && !capabilities.tool_hooks
    );
    // model/list's own order (account.jsonl), the thread's model among them.
    assert_eq!(capabilities.models.len(), 8);
    assert!(capabilities.models.iter().any(|m| m.id == "gpt-6-luna"));
    assert_eq!(texts(&run.events), ["ok"]);
    let streamed: Vec<_> = (run.events.iter())
        .filter_map(|e| {
            if let TextDelta { text, .. } = e {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(streamed, ["ok"]);
    assert!(run
        .events
        .iter()
        .any(|e| matches!(e, Running { requests } if requests == &["r1"])));
    assert!(run.events.iter().any(|e| matches!(e, Tokens { tokens: 5 })));
    assert!(run
        .events
        .iter()
        .any(|e| matches!(e, ContextUsage(c) if c.max == 258400 && c.used > 21000)));
    assert!(run
        .events
        .iter()
        .any(|e| matches!(e, Limits(l) if l.session.is_some() && l.weekly.is_some())));
    let Some(TurnFinished {
        ok: true,
        usage,
        error: None,
        ..
    }) = run.events.last()
    else {
        panic!("{:?}", run.events.last());
    };
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.context_window
        ),
        (21918, 5, 258400)
    );
}

#[test]
fn every_thread_is_opened_with_its_approval_policy_sandbox_and_config() {
    // The person's own Codex config may be danger-full-access: never inherit it.
    for (mode, approval, sandbox) in [
        (None, "on-request", "workspace-write"),
        (Some("read-only"), "untrusted", "read-only"),
        (Some("full-access"), "never", "danger-full-access"),
    ] {
        let allow = mode == Some("full-access");
        for (resume, fork) in [(None, false), (Some("t1"), false), (Some("t1"), true)] {
            let mut run = Run::started(SessionSpec {
                mode: mode.map(str::to_owned),
                resume: resume.map(str::to_owned),
                fork,
                allow_unsafe: allow,
                ..Default::default()
            });
            let init = entries("simple")[1].1.clone();
            run.feed(&init);
            let thread = run.written.last().unwrap();
            let want = match (resume, fork) {
                (None, _) => "thread/start",
                (_, false) => "thread/resume",
                (_, true) => "thread/fork",
            };
            assert_eq!(thread["method"], want);
            let p = &thread["params"];
            assert_eq!(
                (&p["approvalPolicy"], &p["sandbox"]),
                (&json!(approval), &json!(sandbox))
            );
            assert_eq!(p["cwd"], "/work");
            assert_eq!(p["config"]["tools"]["update_plan"]["enabled"], true);
            assert_eq!(
                p["config"]["features"]["default_mode_request_user_input"],
                true
            );
            assert_eq!(p.get("threadId").is_some(), resume.is_some());
        }
    }
    // Full access is offered only to someone who allowed it.
    let mut run = Run::started(SessionSpec {
        mode: Some("full-access".into()),
        ..Default::default()
    });
    run.feed(&entries("simple")[1].1);
    assert_eq!(
        run.written.last().unwrap()["params"]["sandbox"],
        "workspace-write"
    );
}

#[test]
fn a_prompt_before_the_thread_waits_for_it() {
    let mut run = Run::started(SessionSpec::default());
    run.prompt("hi", false);
    assert_eq!(methods(&run.written), ["initialize"]);
    run.feed(&entries("simple")[1].1);
    run.feed(&entries("simple")[7].1);
    assert_eq!(
        methods(&run.written),
        [
            "initialize",
            "initialized",
            "model/list",
            "thread/start",
            "turn/start"
        ]
    );
    let turn = run.written.last().unwrap();
    assert_eq!(turn["params"]["effort"], "low");
    assert_eq!(turn["params"]["model"], "gpt-6-luna");
    assert_eq!(turn["params"]["approvalPolicy"], "on-request");
    assert_eq!(turn["params"]["sandboxPolicy"]["type"], "workspaceWrite");
}

#[test]
fn a_mode_a_model_and_an_effort_apply_to_the_next_turn() {
    let mut run = run_of("simple");
    run.send(AgentCommand::SetMode("read-only".into()));
    run.send(AgentCommand::SetModel("gpt-6.1-sol".into()));
    run.send(AgentCommand::SetEffort("high".into()));
    // The mode is told back when Codex answers the request that carried it.
    assert_eq!(run.written.last().unwrap()["method"], "account/read");
    let id = run.written.last().unwrap()["id"].clone();
    run.feed(&json!({ "id": id, "result": {} }));
    assert_eq!(
        run.events.last(),
        Some(&ModeChanged {
            mode: "read-only".into()
        })
    );
    run.prompt("again", false);
    let turn = run.written.last().unwrap();
    assert_eq!(turn["params"]["sandboxPolicy"]["type"], "readOnly");
    assert_eq!(
        (&turn["params"]["model"], &turn["params"]["effort"]),
        (&json!("gpt-6.1-sol"), &json!("high"))
    );
    // Full access is not taken without the person's allowance.
    run.send(AgentCommand::SetMode("full-access".into()));
    assert_eq!(run.codex.mode, "read-only");
}

#[test]
fn commands_are_bash_reads_or_listings_and_files_are_edits_and_writes() {
    let run = run_of("edit-bash");
    // The commands of the capture, unwrapped from `bash -lc '…'` (edit-bash.jsonl:54, :80).
    let calls = calls(&run.events);
    assert_eq!(
        calls.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        ["Bash", "Write", "Edit", "Bash"]
    );
    assert_eq!(calls[0].1["command"], "cat /home/user/.codex/RTK.md && pwd");
    assert_eq!(
        calls[1].1,
        &json!({ "file_path": "/work/notes.txt", "content": "hello\n" })
    );
    assert_eq!(
        calls[2].1,
        &json!({ "file_path": "/work/notes.txt", "old_string": "hello\n", "new_string": "hello world\n" })
    );
    assert_eq!(calls[3].1["command"], "rtk cat notes.txt");
    let ends = results(&run.events);
    let ToolResult {
        files,
        output,
        is_error,
        command,
        ..
    } = ends[3]
    else {
        unreachable!()
    };
    assert_eq!((output.as_str(), *is_error), ("hello world\n", false));
    assert_eq!(command.as_ref().unwrap().command, "rtk cat notes.txt");
    assert!(files.is_empty());
    let ToolResult { files, .. } = ends[1] else {
        unreachable!()
    };
    assert_eq!(files[0].path, Path::new("/work/notes.txt"));
    assert_eq!(
        (files[0].before.as_deref(), files[0].after.as_deref()),
        (None, Some("hello\n"))
    );
    let ToolResult { files, .. } = ends[2] else {
        unreachable!()
    };
    assert_eq!(
        (files[0].before.as_deref(), files[0].after.as_deref()),
        (Some("hello\n"), Some("hello world\n"))
    );
    assert_eq!(texts(&run.events).len(), 2); // a commentary and the final answer
}

#[test]
fn a_lone_read_or_listing_is_the_tool_the_ui_draws() {
    let run = run_of("deny-patch");
    assert!(calls(&run.events)
        .iter()
        .any(|(n, i)| *n == "Read" && i["file_path"].is_string()));
    let run = run_of("plan");
    assert!(calls(&run.events)
        .iter()
        .any(|(n, i)| *n == "LS" && i["path"] == "."));
    // A search was never captured (docs/codex.md): shaped from the schema.
    let (name, input) = command_tool(&json!({
        "command": "bash -lc 'rg x'", "commandActions": [{ "type": "search", "command": "rg x", "query": "x", "path": null }],
    }));
    assert_eq!((name, input["pattern"].as_str()), ("Grep", Some("x")));
    assert_eq!(shell_line("/bin/bash -lc 'echo '\\''hi'\\'''"), "echo 'hi'");
    assert_eq!(shell_line("ls -la"), "ls -la");
}

#[test]
fn an_approval_asks_the_card_claude_asks_and_a_decline_closes_the_tool_as_an_error() {
    let run = run_of("edit-bash");
    let asked: Vec<_> = (run.events.iter())
        .filter_map(|e| match e {
            PermissionRequest {
                id,
                call,
                tool,
                input,
                interactive,
                suggestions,
                ..
            } => Some((
                id.as_str(),
                call.as_deref(),
                tool.as_str(),
                input,
                *interactive,
                suggestions.len(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0].0, "0");
    assert_eq!(
        asked[0].1,
        Some("exec-0a95eab0-d99f-4d36-bc69-f650244e32fd")
    );
    assert_eq!((asked[0].2, asked[1].2), ("Write", "Edit"));
    assert_eq!(asked[1].3["old_string"], "hello\n");
    assert_eq!((asked[0].4, asked[0].5), (false, 1));

    let run = run_of("deny");
    let PermissionRequest { tool, input, .. } = run
        .events
        .iter()
        .find(|e| matches!(e, PermissionRequest { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        (tool.as_str(), input["command"].as_str()),
        ("Bash", Some("touch denied.txt"))
    );
    let ToolResult {
        is_error, output, ..
    } = results(&run.events)[0]
    else {
        unreachable!()
    };
    assert_eq!((*is_error, output.as_str()), (true, "Declined"));
    assert_eq!(run.written.last().unwrap()["result"]["decision"], "decline");

    let run = run_of("deny-patch");
    let declined = results(&run.events)
        .into_iter()
        .filter(|e| matches!(e, ToolResult { is_error: true, files, .. } if files.is_empty()))
        .count();
    assert_eq!(declined, 1);
    assert_eq!(finished(&run.events), [(true, false)]);
}

#[test]
fn allowing_always_is_accepting_for_the_session() {
    let run = run_of("always");
    let decisions: Vec<_> = (run.written.iter())
        .filter_map(|l| l["result"]["decision"].as_str())
        .collect();
    assert_eq!(decisions, ["acceptForSession"]);
    // The second `touch` ran without a request of its own (always.jsonl:44 on).
    assert_eq!(results(&run.events).len(), 2);
}

#[test]
fn the_plan_is_a_task_list() {
    let run = run_of("plan");
    let lists: Vec<Vec<String>> = (calls(&run.events).into_iter())
        .filter(|(n, _)| *n == "TodoWrite")
        .map(|(_, i)| {
            i["todos"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| text_of(&t["status"]))
                .collect()
        })
        .collect();
    assert_eq!(lists.len(), 4);
    assert_eq!(lists[0], ["pending", "pending", "pending"]);
    assert_eq!(lists[1], ["completed", "in_progress", "pending"]);
    assert_eq!(lists[3], ["completed", "completed", "completed"]);
    let first = calls(&run.events)
        .into_iter()
        .find(|(n, _)| *n == "TodoWrite")
        .unwrap()
        .1;
    assert_eq!(first["todos"][0]["content"], "Create a.txt");
}

#[test]
fn a_prompt_during_a_turn_steers_it() {
    let run = run_of("steer");
    let steer = run
        .written
        .iter()
        .find(|l| l["method"] == "turn/steer")
        .unwrap();
    assert_eq!(
        steer["params"]["expectedTurnId"],
        "01a1817d-03bc-73c3-bbb9-80cec5061abb"
    );
    assert!(run
        .events
        .iter()
        .any(|e| matches!(e, Running { requests } if requests == &["r1", "r2"])));
    assert_eq!(texts(&run.events).last(), Some(&"1\n2\n3\n4\n5"));
}

#[test]
fn an_interrupt_ends_the_turn_interrupted() {
    let run = run_of("interrupt");
    assert_eq!(run.written.last().unwrap()["method"], "turn/interrupt");
    assert_eq!(finished(&run.events), [(false, true)]);
}

#[test]
fn a_question_is_the_ask_user_question_card_and_its_answer_goes_by_id() {
    let run = run_of("ask");
    let PermissionRequest {
        tool,
        input,
        interactive,
        call,
        ..
    } = run
        .events
        .iter()
        .find(|e| matches!(e, PermissionRequest { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!((tool.as_str(), *interactive), ("AskUserQuestion", true));
    assert_eq!(
        call.as_deref(),
        Some("call_ac2837bc8308424d94a79e98800ec25f")
    );
    let q = &input["questions"][0];
    assert_eq!(q["question"], "Which do you pick: red or blue?");
    assert_eq!(q["header"], "Color");
    assert_eq!(q["options"][1]["label"], "Blue");
    // The card answers by the question's words; Codex wants its id (ask.jsonl:31).
    let answer = run.written.last().unwrap();
    assert_eq!(
        answer["result"],
        json!({ "answers": { "color_choice": { "answers": ["Red"] } } })
    );
    assert_eq!(texts(&run.events), ["Red"]);
}

#[test]
fn a_web_search_is_drawn_with_its_sources() {
    let run = run_of("websearch");
    let ToolResult {
        web: Some(web),
        call,
        ..
    } = run
        .events
        .iter()
        .find(|e| matches!(e, ToolResult { web: Some(_), .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        web.query,
        "site:blog.rust-lang.org latest stable Rust release announcement"
    );
    assert_eq!(web.sources[0].url, "https://blog.rust-lang.org/releases/");
    assert!(call.starts_with("exec-"));
    let search = calls(&run.events)
        .into_iter()
        .find(|(n, _)| *n == "WebSearch")
        .unwrap();
    assert_eq!(search.1["query"], web.query);
}

#[test]
fn thinking_is_told_from_the_schema_shapes() {
    // `think.jsonl` holds none (docs/codex.md): the shapes come from the schema.
    let mut run = run_of("think");
    assert!(!run
        .events
        .iter()
        .any(|e| matches!(e, Thinking { .. } | ThinkingDelta { .. })));
    for (method, params) in [
        (
            "item/reasoning/summaryTextDelta",
            json!({ "delta": "Let me", "itemId": "r1", "summaryIndex": 0 }),
        ),
        (
            "item/started",
            json!({ "item": { "type": "reasoning", "id": "r1", "summary": [], "content": [] } }),
        ),
        (
            "item/completed",
            json!({ "item": { "type": "reasoning", "id": "r1", "summary": ["Let me think"], "content": [] } }),
        ),
    ] {
        run.feed(&json!({ "method": method, "params": params }));
    }
    let tail = &run.events[run.events.len() - 3..];
    assert_eq!(
        tail[0],
        ThinkingDelta {
            text: "Let me".into(),
            parent: None
        }
    );
    assert_eq!(tail[1], Thinking { parent: None });
    assert_eq!(
        tail[2],
        ThinkingText {
            text: "Let me think".into(),
            parent: None
        }
    );
}

#[test]
fn resume_is_a_second_process_that_starts_over_and_lists_its_turns() {
    let all = entries("resume");
    let mut first = Run::started(SessionSpec::default());
    let at = replay(&mut first, &all, 0);
    let Session { id } = first.events[0].clone() else {
        panic!()
    };
    // The engine keeps the session's id as `spec.resume` and starts the program again.
    let mut second = Run::started(SessionSpec {
        resume: Some(id),
        ..Default::default()
    });
    replay(&mut second, &all, at);
    let written: Vec<Value> = first
        .written
        .iter()
        .chain(&second.written)
        .cloned()
        .collect();
    assert_eq!(methods(&written), methods(&out_lines(&all)));
    for (w, e) in written.iter().zip(out_lines(&all)) {
        assert_eq!(w["id"], e["id"]);
    }
    assert_eq!(second.written[0]["params"]["clientInfo"]["name"], "quill");
    assert_eq!(
        second
            .written
            .iter()
            .find(|l| l["method"] == "thread/resume")
            .unwrap()["params"]["excludeTurns"],
        true
    );
    assert_eq!(texts(&second.events).len(), 1);
    assert!(unknown(&second.events).is_empty() && unknown(&first.events).is_empty());
    assert_eq!(finished(&second.events), [(true, false)]);
}

#[test]
fn a_fork_continues_a_copy_of_the_thread() {
    let all = entries("fork");
    // The capture forks inside the process; quill launches a new one with `fork` set, so the
    // second half is checked on its own, after the first.
    let mut first = Run::started(SessionSpec::default());
    replay(&mut first, &all[..37], 0);
    let Session { id } = first.events[0].clone() else {
        panic!()
    };
    assert_eq!(
        methods(&first.written),
        [
            "initialize",
            "initialized",
            "model/list",
            "thread/start",
            "turn/start"
        ]
    );
    let mut second = Run::started(SessionSpec {
        resume: Some(id.clone()),
        fork: true,
        ..Default::default()
    });
    // Ids as the capture has them from the fork on, where its process had counted four.
    second.feed(&all[1].1);
    assert_eq!(
        methods(&second.written),
        ["initialize", "initialized", "model/list", "thread/fork"]
    );
    assert_eq!(second.written.last().unwrap()["params"]["threadId"], id);
    second.feed(&json!({ "id": 2, "result": all[43].1["result"] }));
    assert!(
        matches!(&second.events[0], Session { id: forked } if forked == "01a18181-020f-72e0-bd68-fab8fef650f4")
    );
    second.prompt("What word did I ask you to remember? One word.", false);
    assert_eq!(
        second.written.last().unwrap()["params"]["threadId"],
        "01a18181-020f-72e0-bd68-fab8fef650f4"
    );
    // The capture's ids are those of a process that counted on; only its notifications apply.
    for (dir, msg) in &all[46..] {
        if dir == "in" && msg["method"].is_string() {
            second.feed(msg);
        }
    }
    assert_eq!(texts(&second.events).len(), 1);
}

#[test]
fn account_models_become_the_pickers_and_the_unauthorized_turn_asks_to_sign_in() {
    let account = entries("account");
    let mut run = Run::started(SessionSpec::default());
    run.feed(&account[1].1);
    // model/list answers under its own id, as the adapter asked it.
    let mut models = account[6].1.clone();
    models["id"] = json!(MODELS_ID);
    run.feed(&models);
    run.feed(&entries("simple")[7].1);
    let Ready { capabilities, .. } = run
        .events
        .iter()
        .rev()
        .find(|e| matches!(e, Ready { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    let first = &capabilities.models[0];
    assert_eq!(
        (first.id.as_str(), first.name.as_str()),
        ("gpt-6.1-sol", "GPT-6.1-Sol")
    );
    assert_eq!(
        first.efforts,
        ["low", "medium", "high", "xhigh", "max", "ultra"]
    );
    assert_eq!(capabilities.plan, None);
    // Models after the thread tell the session again.
    let mut late = Run::started(SessionSpec::default());
    late.feed(&account[1].1);
    late.feed(&entries("simple")[7].1);
    late.feed(&models);
    assert_eq!(
        late.events
            .iter()
            .filter(|e| matches!(e, Ready { .. }))
            .count(),
        2
    );

    // Inferred from the schema (docs/codex.md Fase 1, 5): no capture has a turn without account.
    let mut run = run_of("simple");
    run.feed(&json!({ "method": "error", "params": { "error": { "message": "x", "codexErrorInfo": "unauthorized" }, "willRetry": false } }));
    run.feed(&json!({ "method": "turn/completed", "params": { "turn": { "status": "failed", "error": { "message": "x", "codexErrorInfo": "unauthorized" } } } }));
    let tail = &run.events[run.events.len() - 2..];
    assert_eq!(tail[0], AuthRequired);
    assert!(
        matches!(&tail[1], TurnFinished { ok: false, interrupted: false, error: Some(e), .. } if e == "x")
    );
}

#[test]
fn what_codex_asks_that_quill_cannot_answer_is_refused_not_awaited() {
    let mut codex = Codex::new();
    let decoded = codex.decode(
        &json!({ "id": 7, "method": "item/permissions/requestApproval", "params": {} }).to_string(),
    );
    assert!(matches!(&decoded.events[..], [Unknown(_)]));
    let reply: Value = serde_json::from_str(&decoded.replies[0]).unwrap();
    assert_eq!(
        (reply["id"].clone(), reply["error"]["code"].clone()),
        (json!(7), json!(-32601))
    );
    for line in [
        "not json",
        r#"{"method":"a/new","params":{}}"#,
        r#"{"id":99,"result":{}}"#,
        "{}",
        "[]",
    ] {
        assert!(
            matches!(&codex.decode(line).events[..], [Unknown(_)]),
            "{line}"
        );
    }
    // Items quill does not draw yet stay whole.
    let item = json!({ "method": "item/started", "params": { "item": { "type": "contextCompaction", "id": "c" } } });
    assert!(matches!(
        &codex.decode(&item.to_string()).events[..],
        [Unknown(_)]
    ));
}

#[test]
fn a_diff_gives_the_hunk_old_and_new_lines_with_their_context() {
    assert_eq!(
        hunk_sides("@@ -1 +1 @@\n-hello\n+hello world\n"),
        ("hello\n".into(), "hello world\n".into())
    );
    let (old, new) = hunk_sides("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n a\n-b\n+c\n d\n\\ No newline at end of file\n");
    assert_eq!((old.as_str(), new.as_str()), ("a\nb\nd\n", "a\nc\nd\n"));
    let edits = file_edits(&json!({ "id": "i", "changes": [
        { "path": "/w/a", "kind": { "type": "add" }, "diff": "x\n" },
        { "path": "/w/b", "kind": { "type": "delete" }, "diff": "gone\n" },
    ] }));
    assert_eq!((edits[0].0.as_str(), edits[1].0.as_str()), ("i", "i#1"));
    assert_eq!(edits[1].3.before.as_deref(), Some("gone\n"));
    assert_eq!(edits[1].3.after, None);
}

#[test]
fn the_account_is_the_clis_own() {
    let codex = Codex::new();
    assert_eq!(codex.id(), "codex");
    assert_eq!(codex.name(), "Codex");
    assert_eq!(codex.tested(), ["0.163.0"]);
    let plan = codex.launch(&SessionSpec {
        cwd: "/p".into(),
        ..Default::default()
    });
    assert_eq!(
        (
            plan.program.as_str(),
            plan.args.as_slice(),
            plan.cwd.as_path()
        ),
        (
            "codex",
            ["app-server".to_owned()].as_slice(),
            Path::new("/p")
        )
    );
    assert_eq!(codex.auth_query().unwrap(), ["login", "status"]);
    // docs/codex.md "Fase 1", 6: the line is on stderr; the engine hands it over.
    assert!(
        matches!(codex.auth_state("Logged in using ChatGPT\n", 0), AuthState::SignedIn { method, .. } if method == "chatgpt")
    );
    assert!(matches!(
        codex.auth_state("Not logged in\n", 1),
        AuthState::SignedOut
    ));
    assert!(matches!(codex.auth_state("", 1), AuthState::Unknown(_)));
    assert_eq!(
        codex.sign_in_methods(),
        [("chatgpt".to_owned(), "ChatGPT account".to_owned())]
    );
    assert_eq!(codex.login_run("chatgpt").unwrap().args, ["login"]);
    assert!(codex.login_run("device").is_none());
    assert_eq!(codex.logout_run().unwrap().args, ["logout"]);
    // The page is the line on its own, not the local server's address (login.rs, upstream).
    assert_eq!(
        codex.login_url("Starting local login server on http://localhost:1455."),
        None
    );
    assert_eq!(
        codex
            .login_url("https://auth.openai.com/oauth/authorize?x=1\n")
            .as_deref(),
        Some("https://auth.openai.com/oauth/authorize?x=1")
    );
    assert!(codex.mode_waives_prompts("full-access") && !codex.mode_waives_prompts("auto"));
}

#[test]
fn the_mode_words_cover_the_modes_it_offers_and_the_default_is_one_of_them() {
    let codex = Codex::new();
    let words = codex.modes();
    let ids: Vec<&str> = words.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["read-only", "auto", "full-access"]);
    assert_eq!(codex.default_mode(), "auto");
    assert!(ids.contains(&codex.default_mode()));
    // Only the unsafe one waits for the person's say.
    let unlisted: Vec<&str> = words
        .iter()
        .filter(|w| !w.listed)
        .map(|w| w.id.as_str())
        .collect();
    assert_eq!(unlisted, ["full-access"]);
    assert!(codex.icon().is_some_and(|svg| svg.contains("currentColor")));
    assert!(!codex.login_takes_code());
}
