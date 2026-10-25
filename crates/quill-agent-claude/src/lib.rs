//! quill's adapter for the Claude Code CLI: stream-json in and out, plus its control
//! protocol (docs/engine.md §2–3). Shapes are the ones captured from CLI 2.1.310
//! (`fixtures/2.1.310/`, docs/engine.md §11.4).

use quill_agent::{
    AgentAdapter, AgentCapabilities, AgentCommand, AgentEvent, AgentId, CommandRun, Decision,
    Decoded, FileChange, LaunchPlan, ModelInfo, SessionSpec, Suggestion, Usage,
};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The PostToolUse hook quill registers: each call reports a tool's end with its prompt.
const POST_TOOL_HOOK: &str = "quill_post_tool";

/// Permission modes the CLI accepts at launch (`auto` stays out of the MVP, spec.md §6).
const MODES: [&str; 3] = ["default", "acceptEdits", "plan"];

/// Message types that carry nothing quill shows yet; not `Unknown`, not events.
const QUIET: [&str; 3] = ["rate_limit_event", "command_lifecycle", "auth_status"];

#[derive(Default)]
pub struct Claude {
    next_request: u64,
    initialize: Option<String>,
    interrupted: bool,
    /// `can_use_tool` waiting for a decision: its tool call and input, echoed back on allow.
    pending: HashMap<String, (Value, Value)>,
    /// Each tool call's name and input, to describe its result.
    calls: HashMap<String, (String, Value)>,
    /// The context the last answer was given, in tokens: what the conversation fills now.
    context: u64,
}

impl Claude {
    pub fn new() -> Self {
        Self::default()
    }

    fn request(&mut self, request: Value) -> String {
        self.next_request += 1;
        let id = format!("q_{}", self.next_request);
        json!({ "type": "control_request", "request_id": id, "request": request }).to_string()
    }

    fn last_request_id(&self) -> String {
        format!("q_{}", self.next_request)
    }

    fn decide(&mut self, id: String, decision: Decision) -> Vec<String> {
        let (call, input) = self.pending.remove(&id).unwrap_or_default();
        let response = match decision {
            Decision::Allow => {
                json!({ "behavior": "allow", "updatedInput": input, "toolUseID": call })
            }
            Decision::AllowAlways(suggestions) => json!({
                "behavior": "allow", "updatedInput": input, "toolUseID": call,
                "updatedPermissions": suggestions.into_iter().map(suggestion_value).collect::<Vec<_>>(),
            }),
            Decision::Deny { message } => {
                json!({ "behavior": "deny", "message": message, "toolUseID": call })
            }
            Decision::Answer(answers) => {
                let mut input = input;
                input["answers"] = answers;
                json!({ "behavior": "allow", "updatedInput": input, "toolUseID": call })
            }
        };
        vec![success(&Value::String(id), response)]
    }

    fn control_request(&mut self, id: Value, request: &Value, decoded: &mut Decoded) {
        let id_text = id.as_str().unwrap_or_default().to_owned();
        match request["subtype"].as_str() {
            Some("can_use_tool") => {
                let tool = text(&request["tool_name"]);
                let input = request["input"].clone();
                let call = request["tool_use_id"].clone();
                self.pending
                    .insert(id_text.clone(), (call.clone(), input.clone()));
                decoded.events.push(AgentEvent::PermissionRequest {
                    id: id_text,
                    call: call.as_str().map(str::to_owned),
                    interactive: request["requires_user_interaction"] == true
                        || tool == "AskUserQuestion",
                    title: request["title"].as_str().map(str::to_owned),
                    tool,
                    input,
                    suggestions: suggestions(&request["permission_suggestions"]),
                });
            }
            Some("hook_callback") => {
                decoded.events.push(AgentEvent::ToolDone {
                    call: text(&request["input"]["tool_use_id"]),
                    prompt: request["input"]["prompt_id"].as_str().map(str::to_owned),
                });
                decoded.replies.push(success(&id, json!({})));
            }
            // Elicitations, dialogs and token refreshes: quill offers none (engine.md §3.3).
            _ => {
                decoded.events.push(AgentEvent::Unknown(request.clone()));
                decoded.replies.push(
                    json!({ "type": "control_response", "response": {
                        "subtype": "error", "request_id": id, "error": "not supported by quill" } })
                    .to_string(),
                );
            }
        }
    }

    fn tool_result(&mut self, block: &Value, structured: &Value) -> AgentEvent {
        let call = text(&block["tool_use_id"]);
        let output = match &block["content"] {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            other => other.to_string(),
        };
        let (name, input) = self.calls.get(&call).cloned().unwrap_or_default();
        let files = file_changes(structured);
        let command = (name == "Bash").then(|| CommandRun {
            command: text(&input["command"]),
            stdout: text(&structured["stdout"]),
            stderr: text(&structured["stderr"]),
            interrupted: structured["interrupted"] == true,
        });
        AgentEvent::ToolResult {
            call,
            output,
            is_error: block["is_error"] == true,
            files,
            command,
        }
    }
}

impl AgentAdapter for Claude {
    fn id(&self) -> AgentId {
        "claude"
    }

    fn launch(&self, spec: &SessionSpec) -> LaunchPlan {
        let mut args: Vec<String> = [
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
            "--permission-prompt-tool",
            "stdio",
            "--include-partial-messages",
            "--setting-sources=user,project,local",
        ]
        .map(String::from)
        .into();
        args.extend([
            "--permission-mode".into(),
            spec.mode.clone().unwrap_or_else(|| "default".into()),
        ]);
        if let Some(model) = &spec.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if let Some(id) = &spec.resume {
            args.push(format!("--resume={id}"));
            if spec.fork {
                args.push("--fork-session".into());
            }
        }
        // Same switches Desktop sets; quill does not claim to be Desktop (spec.md D2).
        let env = [
            ("CLAUDE_CODE_ENABLE_ASK_USER_QUESTION_TOOL", "true"),
            ("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING", "true"),
            ("CLAUDE_CODE_EMIT_TOOL_USE_SUMMARIES", "false"),
            ("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1"),
            ("MCP_CONNECTION_NONBLOCKING", "true"),
        ];
        LaunchPlan {
            program: "claude".into(),
            args,
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            env_remove: vec!["NODE_OPTIONS".into(), "CLAUDE_CODE_ENTRYPOINT".into()],
            cwd: spec.cwd.clone(),
        }
    }

    fn encode(&mut self, command: AgentCommand) -> Vec<String> {
        match command {
            AgentCommand::Start => {
                let line = self.request(json!({
                    "subtype": "initialize",
                    "hooks": { "PostToolUse": [{ "hookCallbackIds": [POST_TOOL_HOOK] }] },
                }));
                self.initialize = Some(self.last_request_id());
                vec![line]
            }
            AgentCommand::Prompt {
                request,
                text,
                priority,
            } => {
                let mut msg = json!({
                    "type": "user", "uuid": request, "session_id": "", "parent_tool_use_id": null,
                    "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
                });
                if let Some(priority) = priority {
                    msg["priority"] = json!(priority);
                }
                vec![msg.to_string()]
            }
            AgentCommand::Interrupt => {
                self.interrupted = true;
                vec![self.request(json!({ "subtype": "interrupt" }))]
            }
            AgentCommand::Decide { id, decision } => self.decide(id, decision),
            AgentCommand::SetMode(mode) => {
                vec![self.request(json!({ "subtype": "set_permission_mode", "mode": mode }))]
            }
            AgentCommand::SetModel(model) => {
                vec![self.request(json!({ "subtype": "set_model", "model": model }))]
            }
            AgentCommand::Rewind { request, dry_run } => vec![self.request(json!({
                "subtype": "rewind_files", "user_message_id": request, "dry_run": dry_run,
            }))],
        }
    }

    fn decode(&mut self, line: &str) -> Decoded {
        let mut decoded = Decoded::default();
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            decoded
                .events
                .push(AgentEvent::Unknown(Value::String(line.to_owned())));
            return decoded;
        };
        let parent = msg["parent_tool_use_id"].as_str().map(str::to_owned);
        match (msg["type"].as_str().unwrap_or(""), msg["subtype"].as_str()) {
            ("control_response", _) => {
                let response = &msg["response"];
                let id = text(&response["request_id"]);
                let ok = response["subtype"] == "success";
                let body = response["response"].clone();
                if ok && self.initialize.as_deref() == Some(id.as_str()) {
                    decoded.events.push(ready(&body));
                }
                decoded.events.push(AgentEvent::Reply { id, ok, body });
            }
            ("control_request", _) => {
                self.control_request(msg["request_id"].clone(), &msg["request"], &mut decoded)
            }
            ("control_cancel_request", _) => {
                // The CLI no longer waits for this decision; the engine drops its card.
                self.pending
                    .remove(msg["request_id"].as_str().unwrap_or_default());
                decoded.events.push(AgentEvent::Unknown(msg));
            }
            ("system", Some("init")) => decoded.events.push(AgentEvent::Session {
                id: text(&msg["session_id"]),
            }),
            ("system", Some("status")) => {
                if let Some(requests) = msg["user_message_uuids"].as_array() {
                    let requests = requests
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect();
                    decoded.events.push(AgentEvent::Running { requests });
                }
                if let Some(mode) = msg["permissionMode"].as_str() {
                    decoded.events.push(AgentEvent::ModeChanged {
                        mode: mode.to_owned(),
                    });
                }
            }
            ("system", _) => {}
            ("stream_event", _) => {
                if msg["event"]["delta"]["type"] == "text_delta" {
                    let text = text(&msg["event"]["delta"]["text"]);
                    decoded.events.push(AgentEvent::TextDelta { text, parent });
                }
            }
            ("assistant", _) => {
                let usage = &msg["message"]["usage"];
                if parent.is_none() && usage.is_object() {
                    self.context = [
                        "input_tokens",
                        "cache_read_input_tokens",
                        "cache_creation_input_tokens",
                    ]
                    .iter()
                    .filter_map(|k| usage[*k].as_u64())
                    .sum();
                }
                for block in msg["message"]["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => decoded.events.push(AgentEvent::Text {
                            text: text(&block["text"]),
                            parent: parent.clone(),
                        }),
                        Some("thinking") => decoded.events.push(AgentEvent::Thinking {
                            parent: parent.clone(),
                        }),
                        Some("tool_use") => {
                            let call = text(&block["id"]);
                            let name = text(&block["name"]);
                            self.calls
                                .insert(call.clone(), (name.clone(), block["input"].clone()));
                            decoded.events.push(AgentEvent::ToolUse {
                                call,
                                name,
                                input: block["input"].clone(),
                                parent: parent.clone(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            ("user", _) => {
                // Echoes of quill's own prompts carry no tool result and are skipped.
                for block in msg["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_result" {
                        let event = self.tool_result(block, &msg["tool_use_result"]);
                        decoded.events.push(event);
                    }
                }
            }
            ("result", _) if parent.is_none() => {
                let interrupted = std::mem::take(&mut self.interrupted)
                    && msg["subtype"] == "error_during_execution";
                let errors = msg["errors"]
                    .as_array()
                    .map(|e| e.iter().map(text).collect::<Vec<_>>().join("\n"));
                decoded.events.push(AgentEvent::TurnFinished {
                    ok: msg["is_error"] != true,
                    interrupted,
                    usage: Usage {
                        input_tokens: msg["usage"]["input_tokens"].as_u64().unwrap_or(0),
                        output_tokens: msg["usage"]["output_tokens"].as_u64().unwrap_or(0),
                        cost_usd: msg["total_cost_usd"].as_f64(),
                        context_tokens: std::mem::take(&mut self.context),
                    },
                    error: errors.filter(|e| !e.is_empty()),
                });
            }
            ("result", _) => {}
            (kind, _) if QUIET.contains(&kind) => {}
            _ => decoded.events.push(AgentEvent::Unknown(msg)),
        }
        decoded
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn success(id: &Value, response: Value) -> String {
    json!({ "type": "control_response", "response": { "subtype": "success", "request_id": id, "response": response } })
        .to_string()
}

/// The answer to `initialize` describes the agent ([F:simple]).
fn ready(body: &Value) -> AgentEvent {
    let models = body["models"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| ModelInfo {
            // `value` is what `--model` and `set_model` take; older entries may lack it.
            id: m["value"]
                .as_str()
                .map_or_else(|| text(&m["resolvedModel"]), str::to_owned),
            name: text(&m["displayName"]),
            efforts: m["supportedEffortLevels"]
                .as_array()
                .into_iter()
                .flatten()
                .map(text)
                .collect(),
        });
    AgentEvent::Ready {
        version: text(&body["claude_code_version"]),
        capabilities: AgentCapabilities {
            models: models.collect(),
            modes: MODES.map(String::from).into(),
            interrupt: true,
            resume: true,
            fork: true,
            rewind: true,
            tool_hooks: true,
            features: body["capabilities"]
                .as_array()
                .into_iter()
                .flatten()
                .map(text)
                .collect(),
        },
        mode: body["current_permission_mode"].as_str().map(str::to_owned),
    }
}

/// Rules and mode switches arrive mixed ([F:always]); quill keeps them apart so
/// "Always allow" never changes the mode by accident (engine.md §11.4).
fn suggestions(value: &Value) -> Vec<Suggestion> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| match s["type"].as_str() {
            Some("addRules") => Suggestion::Rules(s.clone()),
            Some("setMode") => Suggestion::Mode(text(&s["mode"])),
            _ => Suggestion::Other(s.clone()),
        })
        .collect()
}

fn suggestion_value(suggestion: Suggestion) -> Value {
    match suggestion {
        Suggestion::Rules(v) | Suggestion::Other(v) => v,
        Suggestion::Mode(mode) => {
            json!({ "type": "setMode", "mode": mode, "destination": "session" })
        }
    }
}

/// Write and Edit report the file before and after ([F:edit-bash]); other tools report none.
fn file_changes(structured: &Value) -> Vec<FileChange> {
    let Some(path) = structured["filePath"].as_str() else {
        return vec![];
    };
    let before = structured["originalFile"].as_str().map(str::to_owned);
    let after = if let Some(content) = structured["content"].as_str() {
        Some(content.to_owned())
    } else if let (Some(old), Some(new), Some(original)) = (
        structured["oldString"].as_str(),
        structured["newString"].as_str(),
        before.as_deref(),
    ) {
        Some(if structured["replaceAll"] == true {
            original.replace(old, new)
        } else {
            original.replacen(old, new, 1)
        })
    } else {
        None
    };
    vec![FileChange {
        path: path.into(),
        before,
        after,
    }]
}

#[cfg(test)]
mod tests;
