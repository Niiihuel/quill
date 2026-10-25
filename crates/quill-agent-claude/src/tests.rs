//! Every test replays a capture of the real CLI (`fixtures/2.1.310/`, docs/engine.md §11.4).

use super::*;
use std::path::Path;

/// Feed a fixture through a fresh adapter: lines quill sent become the same commands
/// (so the adapter's state matches), lines the CLI sent are decoded.
fn replay(name: &str) -> (Claude, Vec<AgentEvent>, Vec<String>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/2.1.310/{name}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut claude = Claude::new();
    let (mut events, mut replies) = (vec![], vec![]);
    for line in text.lines() {
        let entry: Value = serde_json::from_str(line).unwrap();
        let msg = &entry["msg"];
        match entry["dir"].as_str() {
            Some("out") if msg["type"] == "control_request" => {
                match msg["request"]["subtype"].as_str() {
                    Some("initialize") => drop(claude.encode(AgentCommand::Start)),
                    Some("interrupt") => drop(claude.encode(AgentCommand::Interrupt)),
                    _ => {}
                }
            }
            Some("in") => {
                let decoded = claude.decode(&msg.to_string());
                events.extend(decoded.events);
                replies.extend(decoded.replies);
            }
            _ => {}
        }
    }
    (claude, events, replies)
}

fn unknown(events: &[AgentEvent]) -> Vec<&Value> {
    events
        .iter()
        .filter_map(|e| {
            if let AgentEvent::Unknown(v) = e {
                Some(v)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn every_fixture_decodes_without_unknown_messages() {
    for name in [
        "simple",
        "edit-bash",
        "ask",
        "plan",
        "steer",
        "interrupt",
        "rewind",
        "resume",
        "resume-fork",
        "deny",
        "always",
    ] {
        let (_, events, _) = replay(name);
        assert!(
            unknown(&events).is_empty(),
            "{name}: {:?}",
            unknown(&events)
        );
    }
}

#[test]
fn initialize_reports_models_with_their_efforts() {
    let (_, events, _) = replay("simple");
    let Some(AgentEvent::Ready {
        version,
        capabilities,
        mode,
    }) = events
        .iter()
        .find(|e| matches!(e, AgentEvent::Ready { .. }))
    else {
        panic!("no Ready")
    };
    assert_eq!(version, "2.1.310");
    assert_eq!(mode.as_deref(), Some("default"));
    let opus = capabilities
        .models
        .iter()
        .find(|m| m.name == "Opus 5.5")
        .expect("Opus 5.5");
    assert!(opus.efforts.iter().any(|e| e == "low") && opus.efforts.iter().any(|e| e == "max"));
}

#[test]
fn a_simple_turn_streams_text_and_finishes() {
    let (_, events, _) = replay("simple");
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::Session { id } if !id.is_empty())));
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::TextDelta { .. })));
    let finished: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::TurnFinished { ok: true, .. }))
        .collect();
    assert_eq!(finished.len(), 1);
}

#[test]
fn edits_and_bash_carry_their_files_and_command() {
    let (_, events, replies) = replay("edit-bash");
    let results: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let AgentEvent::ToolResult { files, command, .. } = e {
                Some((files, command))
            } else {
                None
            }
        })
        .collect();
    // In this capture the model created the file with Bash, not Write: only a snapshot sees it.
    assert_eq!(results.len(), 3, "Bash, Edit, Bash");
    assert!(results[0].0.is_empty());
    assert!(results[0]
        .1
        .as_ref()
        .is_some_and(|c| c.command.starts_with("printf")));
    let edit = &results[1].0[0];
    assert_eq!(
        (edit.before.as_deref(), edit.after.as_deref()),
        (Some("hello\n"), Some("hello world\n"))
    );
    assert_eq!(
        results[2].1.as_ref().map(|c| c.command.as_str()),
        Some("mv notes.txt moved.txt && rm moved.txt")
    );
    // Each tool's end is reported by the hook and acknowledged at once, all under one prompt.
    let prompts: std::collections::HashSet<_> = events
        .iter()
        .filter_map(|e| {
            if let AgentEvent::ToolDone { prompt, .. } = e {
                prompt.clone()
            } else {
                None
            }
        })
        .collect();
    assert_eq!(prompts.len(), 1);
    assert_eq!(replies.len(), 3);
}

#[test]
fn a_permission_request_is_answered_with_its_call_and_input() {
    let (mut claude, events, _) = replay("deny");
    let Some(AgentEvent::PermissionRequest { id, tool, call, .. }) = events
        .iter()
        .find(|e| matches!(e, AgentEvent::PermissionRequest { .. }))
    else {
        panic!("no permission request")
    };
    assert_eq!(tool, "Bash");
    // The fixture already denied it; ask again as if it were still pending.
    claude.pending.insert(
        id.clone(),
        (json!(call), json!({ "command": "touch denied.txt" })),
    );
    let line = &claude.encode(AgentCommand::Decide {
        id: id.clone(),
        decision: Decision::Allow,
    })[0];
    let reply: Value = serde_json::from_str(line).unwrap();
    assert_eq!(reply["response"]["response"]["behavior"], "allow");
    assert_eq!(
        reply["response"]["response"]["updatedInput"]["command"],
        "touch denied.txt"
    );
    assert_eq!(reply["response"]["response"]["toolUseID"], json!(call));
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolResult { is_error: true, .. })));
}

#[test]
fn always_allow_returns_only_the_rules() {
    let (mut claude, events, _) = replay("always");
    let Some(AgentEvent::PermissionRequest {
        id, suggestions, ..
    }) = events
        .iter()
        .find(|e| matches!(e, AgentEvent::PermissionRequest { .. }))
    else {
        panic!("no permission request")
    };
    assert!(suggestions
        .iter()
        .any(|s| matches!(s, Suggestion::Mode(m) if m == "acceptEdits")));
    let rules: Vec<_> = suggestions
        .iter()
        .filter(|s| matches!(s, Suggestion::Rules(_)))
        .cloned()
        .collect();
    claude.pending.insert(id.clone(), (json!("t"), json!({})));
    let line = &claude.encode(AgentCommand::Decide {
        id: id.clone(),
        decision: Decision::AllowAlways(rules),
    })[0];
    let reply: Value = serde_json::from_str(line).unwrap();
    let kept = reply["response"]["response"]["updatedPermissions"]
        .as_array()
        .unwrap();
    assert!(kept.iter().all(|p| p["type"] == "addRules"), "{kept:?}");
}

#[test]
fn a_question_is_interactive_and_answered_with_answers() {
    let (mut claude, events, _) = replay("ask");
    let Some(AgentEvent::PermissionRequest {
        id,
        interactive,
        input,
        ..
    }) = events
        .iter()
        .find(|e| matches!(e, AgentEvent::PermissionRequest { .. }))
    else {
        panic!("no question")
    };
    assert!(interactive);
    claude
        .pending
        .insert(id.clone(), (json!("t"), input.clone()));
    let answers = json!({ "Pick red or blue.": "blue" });
    let line = &claude.encode(AgentCommand::Decide {
        id: id.clone(),
        decision: Decision::Answer(answers.clone()),
    })[0];
    let reply: Value = serde_json::from_str(line).unwrap();
    assert_eq!(
        reply["response"]["response"]["updatedInput"]["answers"],
        answers
    );
    assert!(reply["response"]["response"]["updatedInput"]["questions"].is_array());
}

#[test]
fn an_interrupted_turn_finishes_interrupted() {
    let (_, events, _) = replay("interrupt");
    assert!(events.iter().any(|e| matches!(
        e,
        AgentEvent::TurnFinished {
            ok: false,
            interrupted: true,
            ..
        }
    )));
    assert!(events.iter().any(
        |e| matches!(e, AgentEvent::Reply { ok: true, body, .. } if body["still_queued"].is_array())
    ));
}

#[test]
fn rewind_dry_run_reports_the_files() {
    let (_, events, _) = replay("rewind");
    // A Write reports the new file whole, with nothing before it.
    assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolResult { files, .. }
        if files.len() == 1 && files[0].before.is_none() && files[0].after.as_deref() == Some("one\n"))));
    let reply = events
        .iter()
        .find_map(|e| {
            if let AgentEvent::Reply { body, .. } = e {
                body.get("canRewind").map(|_| body)
            } else {
                None
            }
        })
        .expect("rewind reply");
    assert_eq!(reply["filesChanged"].as_array().map(Vec::len), Some(1));
}

#[test]
fn the_running_request_and_mode_changes_are_reported() {
    let (_, events, _) = replay("always");
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::Running { requests } if requests.len() == 1)));
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::ModeChanged { mode } if mode == "acceptEdits")));
}

#[test]
fn launch_never_claims_to_be_desktop() {
    let plan = Claude::new().launch(&SessionSpec {
        cwd: "/work".into(),
        resume: Some("s".into()),
        fork: true,
        ..Default::default()
    });
    assert!(
        plan.args.contains(&"--resume=s".to_string())
            && plan.args.contains(&"--fork-session".to_string())
    );
    assert!(plan
        .env_remove
        .contains(&"CLAUDE_CODE_ENTRYPOINT".to_string()));
    assert!(!plan.env.iter().any(|(_, v)| v.contains("desktop")));
}
