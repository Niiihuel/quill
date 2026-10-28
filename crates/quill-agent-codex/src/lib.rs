//! quill's adapter for the OpenAI Codex CLI: `codex app-server`, JSON-RPC over stdio, one
//! message a line (docs/codex.md). Shapes are the ones captured from codex-cli 0.163.0
//! (`fixtures/0.163.0/<scenario>.jsonl:<line>`); what no capture shows is marked as inferred.
//!
//! The adapter is a small state machine: it numbers its requests, remembers which answer each
//! one waits for, and writes the next request when an answer arrives (`Decoded::replies`).

use quill_agent::{
    AgentAdapter, AgentCapabilities, AgentCommand, AgentEvent, AgentId, AuthState, Choices,
    CommandRun, ContextCategory, ContextKind, ContextUsage, Decision, Decoded, FileChange,
    LaunchPlan, ModeWords, ModelInfo, PlanLimits, ProgramRun, PromptImage, SessionSpec, Suggestion,
    Usage, UsageWindow, WebSearchRun, WebSource,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

/// The permission presets, as Codex's own are (docs/codex.md "Modos de permiso"). Each is an
/// approval policy and a sandbox; quill always sends both, because a thread that is not told
/// takes the person's own Codex config, which may be `danger-full-access`
/// (resume.jsonl, first attempt in docs/codex.md "Fase 1").
const MODES: [&str; 2] = ["read-only", "auto"];
const FULL_ACCESS: &str = "full-access";
const DEFAULT_MODE: &str = "auto";

/// The id of the `model/list` request: a name, not a number, so that the numbers of the
/// requests every capture holds stay the ones the adapter writes (the fake agent replays them).
const MODELS_ID: &str = "quill-models";

/// Per-thread config (not `~/.codex/config.toml`): `update_plan` is off by default
/// (plan.jsonl, two attempts without it) and `request_user_input` only exists in Plan mode
/// unless this feature is on (ask.jsonl:9).
fn thread_config() -> Value {
    json!({
        "tools": { "update_plan": { "enabled": true } },
        "features": { "default_mode_request_user_input": true },
    })
}

/// Notifications that carry nothing quill shows; not `Unknown`, not events.
const QUIET: [&str; 13] = [
    "remoteControl/status/changed",
    "mcpServer/startupStatus/updated",
    "thread/status/changed",
    "thread/started",
    "thread/goal/cleared",
    "hook/started",
    "hook/completed",
    "warning",
    "deprecationNotice",
    "serverRequest/resolved",
    // The turn's accumulated git diff; quill attributes files itself (docs/codex.md).
    "turn/diff/updated",
    "item/commandExecution/outputDelta",
    "item/fileChange/outputDelta",
];

/// What a request waits for.
enum Pending {
    Init,
    Models,
    /// `thread/start`, `thread/resume` or `thread/fork`.
    Thread,
    /// `turn/start`.
    Turn,
    /// `turn/steer`, for the quill request it carries.
    Steer(String),
    /// Events to tell once the answer comes: the only way a command without its own answer
    /// can be told back (`encode` returns lines, not events).
    Ack(Vec<AgentEvent>),
    /// Nothing to do with the answer.
    Ignore,
}

/// A server request waiting for the person.
enum Approval {
    /// Answered with `{decision}`.
    Decide(Value),
    /// A question: answered with `{answers}`. Holds each question's text and Codex id.
    Ask(Value, Vec<(String, String)>),
}

#[derive(Default)]
pub struct Codex {
    /// What `launch` was given: Codex takes it in `thread/start`, not on the command line.
    spec: Mutex<SessionSpec>,
    next_id: u64,
    pending: HashMap<String, Pending>,
    version: String,
    thread: Option<String>,
    turn: Option<String>,
    /// The quill requests of the running turn, steers included.
    turn_requests: Vec<String>,
    /// Requests of `turn/start`s whose `turn/started` has not come.
    starting: Vec<String>,
    /// Prompts sent before the thread existed: (request, input).
    queued: Vec<(String, Vec<Value>)>,
    mode: String,
    model: Option<String>,
    effort: Option<String>,
    models: Vec<ModelInfo>,
    thread_model: Option<String>,
    plan: Option<String>,
    /// Each tool call's name and input, to ask about it and to close it.
    calls: HashMap<String, (String, Value)>,
    approvals: HashMap<String, Approval>,
    auth_failed: bool,
    plan_updates: u64,
    turn_in: u64,
    turn_out: u64,
    context: u64,
    window: u64,
}

impl Codex {
    pub fn new() -> Self {
        Self::default()
    }

    fn request(&mut self, method: &str, params: Value, pending: Pending) -> String {
        self.next_id += 1;
        self.pending.insert(self.next_id.to_string(), pending);
        json!({ "id": self.next_id, "method": method, "params": params }).to_string()
    }

    fn thread_params(&self) -> Value {
        let spec = self.spec.lock().unwrap();
        let (approval, sandbox, _) = preset(&self.mode);
        let mut params = json!({
            "cwd": spec.cwd.to_string_lossy(), "approvalPolicy": approval, "sandbox": sandbox,
            "config": thread_config(),
        });
        if let Some(model) = &self.model {
            params["model"] = json!(model);
        }
        params
    }

    /// `thread/start`, or `thread/resume` / `thread/fork` of the spec's session. A resumed
    /// thread asks for no turns: quill keeps its own history, and the full one is deprecated
    /// (resume.jsonl:41).
    fn open_thread(&mut self) -> String {
        let resume = self.spec.lock().unwrap().resume.clone();
        let mut params = self.thread_params();
        let Some(id) = resume else {
            return self.request("thread/start", params, Pending::Thread);
        };
        let fork = self.spec.lock().unwrap().fork;
        params["threadId"] = json!(id);
        params["excludeTurns"] = json!(true);
        // A rewind or a fork from a message: the copy keeps the conversation through that turn,
        // the original whole (rewind.jsonl:48,58-72).
        let at = self.spec.lock().unwrap().resume_at.clone();
        if let Some(turn) = at.filter(|_| fork) {
            params["lastTurnId"] = json!(turn);
        }
        let method = if fork { "thread/fork" } else { "thread/resume" };
        self.request(method, params, Pending::Thread)
    }

    fn turn_start(&mut self, request: String, input: Vec<Value>) -> String {
        let (approval, _, policy) = preset(&self.mode);
        // Mode, model and effort are sent with every turn, so a change applies to the next one.
        let mut params = json!({
            "threadId": self.thread, "input": input, "approvalPolicy": approval,
            "sandboxPolicy": { "type": policy }, "summary": "auto",
        });
        if let Some(model) = &self.model {
            params["model"] = json!(model);
        }
        if let Some(effort) = &self.effort {
            params["effort"] = json!(effort);
        }
        self.starting.push(request);
        self.request("turn/start", params, Pending::Turn)
    }

    fn decide(&mut self, id: String, decision: Decision) -> Vec<String> {
        let Some(approval) = self.approvals.remove(&id) else {
            return vec![];
        };
        let (rpc, result) = match (approval, decision) {
            (Approval::Decide(rpc), decision) => {
                // `acceptForSession` and `decline` work although `availableDecisions` does not
                // list them (always.jsonl:44, deny.jsonl:28).
                let word = match decision {
                    Decision::Allow | Decision::Answer(_) => "accept",
                    Decision::AllowAlways(_) => "acceptForSession",
                    Decision::Deny { .. } => "decline",
                };
                (rpc, json!({ "decision": word }))
            }
            (Approval::Ask(rpc, questions), Decision::Answer(answers)) => {
                let answers: serde_json::Map<String, Value> = questions
                    .into_iter()
                    .map(|(question, id)| {
                        let said = text(&answers[question.as_str()]);
                        // The card says "[No preference]" for a question left alone. A single
                        // choice: a typed "Other" is one answer, whatever it contains.
                        let labels = if said.is_empty() || said == "[No preference]" {
                            vec![]
                        } else {
                            vec![said]
                        };
                        (id, json!({ "answers": labels }))
                    })
                    .collect();
                (rpc, json!({ "answers": answers }))
            }
            (Approval::Ask(rpc, _), _) => (rpc, json!({ "answers": {} })),
        };
        vec![json!({ "id": rpc, "result": result }).to_string()]
    }

    fn ready(&self) -> AgentEvent {
        let mut models = self.models.clone();
        if models.is_empty() {
            // Until `model/list` answers (or if it cannot): the thread's own model.
            let model = self.model.clone().or(self.thread_model.clone());
            models = model
                .into_iter()
                .map(|id| ModelInfo {
                    name: id.clone(),
                    id,
                    efforts: ["low", "medium", "high", "xhigh"].map(String::from).into(),
                    modes: vec![],
                })
                .collect();
        }
        AgentEvent::Ready {
            version: self.version.clone(),
            capabilities: AgentCapabilities {
                models,
                modes: MODES.map(String::from).into(),
                unsafe_modes: vec![FULL_ACCESS.into()],
                interrupt: true,
                resume: true,
                fork: true,
                plan: self.plan.clone(),
                thinking_text: true,
                steer: true,
                ..Default::default()
            },
            mode: Some(self.mode.clone()),
        }
    }

    fn response(&mut self, id: &Value, msg: &Value, out: &mut Decoded) {
        let Some(pending) = self.pending.remove(&key(id)) else {
            return out.events.push(AgentEvent::Unknown(msg.clone()));
        };
        if msg.get("error").is_some() {
            return self.failed(pending, msg, out);
        }
        let result = &msg["result"];
        match pending {
            Pending::Init => {
                // "quill/0.163.0 (Linux; …)": the client's name, then the CLI's version
                // (simple.jsonl:2).
                self.version = text(&result["userAgent"])
                    .split_whitespace()
                    .next()
                    .and_then(|agent| agent.split_once('/'))
                    .map(|(_, version)| version.to_owned())
                    .unwrap_or_default();
                out.replies
                    .push(json!({ "method": "initialized" }).to_string());
                self.pending.insert(MODELS_ID.into(), Pending::Models);
                out.replies.push(
                    json!({ "id": MODELS_ID, "method": "model/list", "params": {} }).to_string(),
                );
                let thread = self.open_thread();
                out.replies.push(thread);
            }
            Pending::Models => {
                self.models = models(&result["data"]);
                if self.thread.is_some() {
                    out.events.push(self.ready());
                }
            }
            Pending::Thread => {
                let id = text(&result["thread"]["id"]);
                self.thread_model = result["model"].as_str().map(str::to_owned);
                self.thread = Some(id.clone());
                out.events.push(AgentEvent::Session { id: id.clone() });
                out.events.push(self.ready());
                for (request, input) in std::mem::take(&mut self.queued) {
                    out.replies.push(self.turn_start(request, input));
                }
            }
            Pending::Turn => {
                if self.turn.is_none() {
                    self.turn = result["turn"]["id"].as_str().map(str::to_owned);
                }
            }
            // The same turn takes it (steer.jsonl:32): its requests grow by one.
            Pending::Steer(request) => {
                self.turn_requests.push(request);
                out.events.push(AgentEvent::Running {
                    requests: self.turn_requests.clone(),
                });
            }
            Pending::Ack(events) => out.events.extend(events),
            Pending::Ignore => {}
        }
    }

    fn failed(&mut self, pending: Pending, msg: &Value, out: &mut Decoded) {
        let error = text(&msg["error"]["message"]);
        // Inferred from the schema (codexErrorInfo "unauthorized"): the error's words.
        if error.to_lowercase().contains("unauthorized") {
            out.events.push(AgentEvent::AuthRequired);
        }
        let turn = matches!(pending, Pending::Turn)
            || (matches!(pending, Pending::Thread) && !self.queued.is_empty());
        if turn {
            self.queued.clear();
            self.starting.clear();
            out.events
                .push(finished(false, false, Usage::default(), Some(error)));
        }
        out.events.push(AgentEvent::Unknown(msg.clone()));
    }

    fn notification(&mut self, method: &str, p: &Value, msg: &Value, out: &mut Decoded) {
        let events = &mut out.events;
        match method {
            "turn/started" => {
                self.turn = p["turn"]["id"].as_str().map(str::to_owned);
                self.turn_requests = std::mem::take(&mut self.starting);
                events.push(AgentEvent::Running {
                    requests: self.turn_requests.clone(),
                });
            }
            "turn/completed" => self.turn_completed(&p["turn"], out),
            "item/started" => self.item(&p["item"], false, out),
            "item/completed" => self.item(&p["item"], true, out),
            "item/agentMessage/delta" => events.push(AgentEvent::TextDelta {
                text: text(&p["delta"]),
                parent: None,
            }),
            // Only in the schema: `think.jsonl` got no reasoning from gpt-6-luna at low effort.
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                events.push(AgentEvent::ThinkingDelta {
                    text: text(&p["delta"]),
                    parent: None,
                })
            }
            // The whole list each time (plan.jsonl:52); the task list is drawn from TodoWrite.
            "turn/plan/updated" => {
                self.plan_updates += 1;
                let call = format!("plan-{}", self.plan_updates);
                let todos: Vec<Value> = (p["plan"].as_array().into_iter().flatten())
                    .map(|step| {
                        let status = match step["status"].as_str() {
                            Some("inProgress") => "in_progress",
                            Some("completed") => "completed",
                            _ => "pending",
                        };
                        json!({ "content": step["step"], "activeForm": step["step"], "status": status })
                    })
                    .collect();
                events.push(AgentEvent::ToolUse {
                    call: call.clone(),
                    name: "TodoWrite".into(),
                    input: json!({ "todos": todos }),
                    parent: None,
                });
                events.push(result(call, String::new(), false, vec![], None, None));
            }
            "thread/tokenUsage/updated" => {
                let last = &p["tokenUsage"]["last"];
                self.turn_in += number(&last["inputTokens"]);
                self.turn_out += number(&last["outputTokens"]);
                // The last request's size is what the conversation fills now (inferred).
                self.context = number(&last["totalTokens"]);
                self.window = number(&p["tokenUsage"]["modelContextWindow"]);
                // Output only: what the model wrote this turn (inferred; Codex has no turn total).
                events.push(AgentEvent::Tokens {
                    tokens: self.turn_out,
                });
                events.push(AgentEvent::ContextUsage(ContextUsage {
                    used: self.context,
                    max: self.window,
                    categories: context_categories(last, self.window),
                    ..Default::default()
                }));
            }
            "account/rateLimits/updated" => {
                let window = |w: &Value| {
                    Some(UsageWindow {
                        utilization: w["usedPercent"].as_f64()? as f32 / 100.0,
                        resets_at: w["resetsAt"].as_u64()?,
                    })
                };
                let limits = &p["rateLimits"];
                events.push(AgentEvent::Limits(PlanLimits {
                    session: window(&limits["primary"]),
                    weekly: window(&limits["secondary"]),
                }));
            }
            "account/updated" => self.plan = p["planType"].as_str().map(str::to_owned),
            "error" => {
                self.auth_failed |= p["error"]["codexErrorInfo"] == "unauthorized";
            }
            method if QUIET.contains(&method) => {}
            _ => events.push(AgentEvent::Unknown(msg.clone())),
        }
    }

    fn turn_completed(&mut self, turn: &Value, out: &mut Decoded) {
        let status = turn["status"].as_str().unwrap_or_default();
        if std::mem::take(&mut self.auth_failed)
            || turn["error"]["codexErrorInfo"] == "unauthorized"
        {
            out.events.push(AgentEvent::AuthRequired);
        }
        let usage = Usage {
            input_tokens: std::mem::take(&mut self.turn_in),
            output_tokens: std::mem::take(&mut self.turn_out),
            cost_usd: None,
            context_tokens: self.context,
            context_window: self.window,
        };
        let error = turn["error"]["message"].as_str().map(str::to_owned);
        out.events.push(finished(
            status == "completed",
            status == "interrupted",
            usage,
            error,
        ));
        self.turn = None;
        self.turn_requests.clear();
        self.calls.clear();
        self.approvals.clear();
    }

    /// A tool opens (`item/started`), unless it has nothing to show until it ends.
    fn tool_use(&mut self, call: String, name: &str, input: Value, out: &mut Decoded) {
        self.calls
            .insert(call.clone(), (name.to_owned(), input.clone()));
        out.events.push(AgentEvent::ToolUse {
            call,
            name: name.to_owned(),
            input,
            parent: None,
        });
    }

    /// A tool ends. A resumed thread may end one it never opened: it opens now.
    #[allow(clippy::too_many_arguments)]
    fn tool_end(
        &mut self,
        (call, name, input): (String, &str, Value),
        output: String,
        is_error: bool,
        files: Vec<FileChange>,
        command: Option<CommandRun>,
        web: Option<WebSearchRun>,
        out: &mut Decoded,
    ) {
        if !self.calls.contains_key(&call) {
            self.tool_use(call.clone(), name, input, out);
        }
        out.events
            .push(result(call, output, is_error, files, command, web));
    }

    fn item(&mut self, item: &Value, done: bool, out: &mut Decoded) {
        let id = text(&item["id"]);
        let status = text(&item["status"]);
        let failed = status == "failed" || status == "declined";
        match item["type"].as_str() {
            // quill has the person's words already, a steer's too.
            Some("userMessage") => {}
            Some("agentMessage") if done && !text(&item["text"]).is_empty() => {
                out.events.push(AgentEvent::Text {
                    text: text(&item["text"]),
                    parent: None,
                    // The turn, not the item: `thread/fork`'s `lastTurnId` takes a turn
                    // (rewind.jsonl:48).
                    id: self.turn.clone(),
                    at: None,
                })
            }
            Some("agentMessage") => {}
            Some("reasoning") if !done => out.events.push(AgentEvent::Thinking { parent: None }),
            Some("reasoning") => {
                let words = |key: &str| {
                    (item[key].as_array().into_iter().flatten())
                        .map(text)
                        .collect::<Vec<_>>()
                        .join("\n\n")
                };
                let said = Some(words("summary")).filter(|s| !s.is_empty());
                if let Some(text) = said.or(Some(words("content")).filter(|s| !s.is_empty())) {
                    out.events
                        .push(AgentEvent::ThinkingText { text, parent: None });
                }
            }
            Some("commandExecution") => {
                let (name, input) = command_tool(item);
                if !done {
                    return self.tool_use(id, name, input, out);
                }
                let output = text(&item["aggregatedOutput"]);
                let output = if output.is_empty() && status == "declined" {
                    "Declined".to_owned()
                } else {
                    output
                };
                let run = (name == "Bash").then(|| CommandRun {
                    command: text(&input["command"]),
                    stdout: text(&item["aggregatedOutput"]),
                    stderr: String::new(),
                    interrupted: false,
                });
                let error = failed || item["exitCode"].as_i64().is_some_and(|c| c != 0);
                self.tool_end((id, name, input), output, error, vec![], run, None, out);
            }
            Some("fileChange") => {
                for (call, name, input, change) in file_edits(item) {
                    if !done {
                        self.tool_use(call, name, input, out);
                        continue;
                    }
                    let output = match status.as_str() {
                        "declined" => "Declined",
                        "failed" => "Failed",
                        _ => "",
                    };
                    let files = if failed { vec![] } else { vec![change] };
                    self.tool_end(
                        (call, name, input),
                        output.into(),
                        failed,
                        files,
                        None,
                        None,
                        out,
                    );
                }
            }
            Some("mcpToolCall") => {
                let name = format!("mcp__{}__{}", text(&item["server"]), text(&item["tool"]));
                let input = item["arguments"].clone();
                if !done {
                    return self.tool_use(id, &name, input, out);
                }
                let output = match item["error"]["message"].as_str() {
                    Some(error) => error.to_owned(),
                    None => (item["result"]["content"].as_array().into_iter().flatten())
                        .filter_map(|part| part["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                self.tool_end(
                    (id, &name, input),
                    output,
                    failed || item["error"].is_object(),
                    vec![],
                    None,
                    None,
                    out,
                );
            }
            // Opens with an empty query (websearch.jsonl:44): it is drawn once it ends.
            Some("webSearch") if done => {
                let run = web_run(item);
                let output = (run.sources.iter())
                    .map(|s| format!("{} - {}", s.title, s.url))
                    .collect::<Vec<_>>()
                    .join("\n");
                let input = json!({ "query": run.query });
                self.tool_end(
                    (id, "WebSearch", input),
                    output,
                    false,
                    vec![],
                    None,
                    Some(run),
                    out,
                );
            }
            Some("webSearch") => {}
            _ => out
                .events
                .push(AgentEvent::Unknown(json!({ "item": item }))),
        }
    }

    fn server_request(
        &mut self,
        method: &str,
        id: &Value,
        p: &Value,
        msg: &Value,
        out: &mut Decoded,
    ) {
        let key = key(id);
        let always = vec![Suggestion::Other(json!("acceptForSession"))];
        let (tool, input, interactive, suggestions) = match method {
            // Same tool and input as Claude's card for a command (Bash {command}).
            "item/commandExecution/requestApproval" => {
                self.approvals
                    .insert(key.clone(), Approval::Decide(id.clone()));
                let input = json!({ "command": shell_line(&text(&p["command"])) });
                ("Bash".to_owned(), input, false, always)
            }
            // The request carries no files: the card shows the first the item announced.
            "item/fileChange/requestApproval" => {
                self.approvals
                    .insert(key.clone(), Approval::Decide(id.clone()));
                let (name, input) = self
                    .calls
                    .get(&text(&p["itemId"]))
                    .cloned()
                    .unwrap_or_default();
                (name, input, false, always)
            }
            // The card of AskUserQuestion: `{questions: [{question, header, options}]}`.
            "item/tool/requestUserInput" => {
                let asked: Vec<&Value> = p["questions"].as_array().into_iter().flatten().collect();
                let questions: Vec<Value> = asked
                    .iter()
                    .map(|q| {
                        json!({ "question": q["question"], "header": q["header"],
                            "options": q["options"], "multiSelect": false })
                    })
                    .collect();
                let ids = asked
                    .iter()
                    .map(|q| (text(&q["question"]), text(&q["id"])))
                    .collect();
                self.approvals
                    .insert(key.clone(), Approval::Ask(id.clone(), ids));
                (
                    "AskUserQuestion".to_owned(),
                    json!({ "questions": questions }),
                    true,
                    vec![],
                )
            }
            // Dialogs, permission grants and token refreshes: quill offers none, and Codex
            // waits for an answer.
            _ => {
                out.events.push(AgentEvent::Unknown(msg.clone()));
                return out.replies.push(
                    json!({ "id": id, "error": { "code": -32601, "message": "not supported by quill" } })
                        .to_string(),
                );
            }
        };
        out.events.push(AgentEvent::PermissionRequest {
            id: key,
            call: Some(text(&p["itemId"])).filter(|i| !i.is_empty()),
            tool,
            input,
            title: p["reason"].as_str().map(str::to_owned),
            interactive,
            suggestions,
        });
    }
}

impl AgentAdapter for Codex {
    fn id(&self) -> AgentId {
        "codex"
    }

    fn name(&self) -> &'static str {
        "Codex"
    }

    fn tested(&self) -> &'static [&'static str] {
        // The version the fixtures were captured from.
        &["0.163.0"]
    }

    fn launch(&self, spec: &SessionSpec) -> LaunchPlan {
        *self.spec.lock().unwrap() = spec.clone();
        LaunchPlan {
            program: "codex".into(),
            args: vec!["app-server".into()],
            env: vec![],
            env_remove: vec![],
            cwd: spec.cwd.clone(),
        }
    }

    fn mode_waives_prompts(&self, mode: &str) -> bool {
        mode == FULL_ACCESS
    }

    // The three presets of docs/codex.md "Modos de permiso", in Codex's own words for them.
    fn modes(&self) -> Vec<ModeWords> {
        use quill_agent::ModeTone::*;
        let w = |id: &str, label: &str, about: &str, tone, listed| ModeWords {
            id: id.into(),
            label: label.into(),
            about: about.into(),
            tone,
            listed,
        };
        vec![
            w(
                "read-only",
                "Read only",
                "Asks before anything but reading",
                Read,
                true,
            ),
            w(
                "auto",
                "Auto",
                "Works in this folder; asks to go beyond it",
                Auto,
                true,
            ),
            w(
                FULL_ACCESS,
                "Full access",
                "No sandbox, never asks",
                Unsafe,
                false,
            ),
        ]
    }

    fn default_mode(&self) -> &'static str {
        DEFAULT_MODE
    }

    // svgl.app's "Codex" icon, recolored to currentColor (the same source as Claude's).
    fn icon(&self) -> Option<&'static str> {
        Some(include_str!("../assets/codex.svg"))
    }

    // Sign-in is the CLI's own `codex login` (docs/codex.md, codex-rs/cli/src/login.rs): it
    // prints the page address on stderr and waits for the browser to come back to a local
    // server, so it never needs a code. `--device-auth` prints a code the person must type
    // on the page, which quill has no place to show, so it is not offered.
    fn sign_in_methods(&self) -> Choices {
        vec![("chatgpt".into(), "ChatGPT account".into())]
    }

    fn login_takes_code(&self) -> bool {
        false
    }

    fn auth_query(&self) -> Option<Vec<String>> {
        Some(["login", "status"].map(String::from).into())
    }

    /// `codex login status` writes its one line on stderr and exits 0, or 1 when signed
    /// out (docs/codex.md "Fase 1", 6); the engine hands over whichever stream has text.
    fn auth_state(&self, stdout: &str, exit: i32) -> AuthState {
        let said = stdout.trim();
        if said.starts_with("Not logged in") {
            return AuthState::SignedOut;
        }
        match said.strip_prefix("Logged in using ") {
            Some(method) => AuthState::SignedIn {
                method: method.to_lowercase(),
                // The email is not in this line; it is in `account/read`.
                email: None,
                org: None,
                plan: None,
            },
            None if said.starts_with("API key configured") => AuthState::SignedIn {
                method: "api key".into(),
                email: None,
                org: None,
                plan: None,
            },
            None => AuthState::Unknown(format!("codex login status exited {exit}: {said}")),
        }
    }

    fn login_run(&self, method: &str) -> Option<ProgramRun> {
        (method == "chatgpt").then(|| ProgramRun {
            args: vec!["login".into()],
            stdin: None,
            timeout: 600,
        })
    }

    /// The handshake and `model/list`, no thread: what account.jsonl:1-7 asked.
    fn models_query(&self) -> Option<Vec<String>> {
        Some(vec![
            json!({ "id": 1, "method": "initialize", "params": { "clientInfo": client() } })
                .to_string(),
            json!({ "method": "initialized" }).to_string(),
            json!({ "id": MODELS_ID, "method": "model/list", "params": {} }).to_string(),
        ])
    }

    fn models_answer(&self, line: &str) -> Option<Vec<ModelInfo>> {
        let answer: Value = serde_json::from_str(line).ok()?;
        (answer["id"] == MODELS_ID).then(|| models(&answer["result"]["data"]))
    }

    /// Its own line, after "navigate to this URL to authenticate:" (the line before it
    /// holds `http://localhost:<port>`, not the page).
    fn login_url(&self, line: &str) -> Option<String> {
        let line = line.trim();
        line.starts_with("https://").then(|| {
            line.split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned()
        })
    }

    fn logout_run(&self) -> Option<ProgramRun> {
        Some(ProgramRun {
            args: vec!["logout".into()],
            stdin: None,
            timeout: 60,
        })
    }

    fn encode(&mut self, command: AgentCommand) -> Vec<String> {
        match command {
            AgentCommand::Start => {
                // A new process: its requests count from 1 again (resume.jsonl:36).
                let spec = self.spec.lock().unwrap().clone();
                *self = Self {
                    spec: Mutex::new(spec.clone()),
                    mode: spec
                        .mode
                        .filter(|m| {
                            MODES.contains(&m.as_str()) || (m == FULL_ACCESS && spec.allow_unsafe)
                        })
                        .unwrap_or_else(|| DEFAULT_MODE.into()),
                    model: spec.model,
                    effort: spec.effort,
                    ..Self::default()
                };
                // quill says who it is (docs/codex.md); the answer starts the rest.
                vec![self.request(
                    "initialize",
                    json!({ "clientInfo": client() }),
                    Pending::Init,
                )]
            }
            AgentCommand::Prompt {
                request,
                text,
                steer,
                images,
            } => {
                let mut input: Vec<Value> = images.iter().map(image_input).collect();
                input.push(json!({ "type": "text", "text": text }));
                match (self.thread.clone(), self.turn.clone()) {
                    (None, _) => {
                        self.queued.push((request, input));
                        vec![]
                    }
                    (Some(thread), Some(turn)) if steer => vec![self.request(
                        "turn/steer",
                        json!({ "threadId": thread, "expectedTurnId": turn, "input": input }),
                        Pending::Steer(request),
                    )],
                    _ => vec![self.turn_start(request, input)],
                }
            }
            AgentCommand::Interrupt => match (self.thread.clone(), self.turn.clone()) {
                (Some(thread), Some(turn)) => vec![self.request(
                    "turn/interrupt",
                    json!({ "threadId": thread, "turnId": turn }),
                    Pending::Ignore,
                )],
                _ => vec![],
            },
            AgentCommand::Decide { id, decision } => self.decide(id, decision),
            // The mode, model and effort travel with the next `turn/start`. A mode is told back
            // with a request that costs nothing, because Codex has none that changes a thread.
            // Full access is the engine's to allow (Settings, asked at the change): Codex takes
            // any policy per turn, so a session started before it was allowed can switch too.
            AgentCommand::SetMode(mode) => {
                if !(MODES.contains(&mode.as_str()) || mode == FULL_ACCESS) {
                    return vec![];
                }
                self.mode = mode.clone();
                vec![self.request(
                    "account/read",
                    json!({}),
                    Pending::Ack(vec![AgentEvent::ModeChanged { mode }]),
                )]
            }
            AgentCommand::SetModel(model) => {
                self.model = Some(model);
                vec![]
            }
            AgentCommand::SetEffort(effort) => {
                self.effort = Some(effort);
                vec![]
            }
            // `Rewind` restores files, which Codex does not do (`thread/revert` leaves them): they
            // stay quill's snapshots, so `capabilities.rewind` is false. The conversation goes
            // back through `thread/fork` with `lastTurnId` (docs/codex.md). No context
            // breakdown, MCP control, skills list or host tools that quill drives yet.
            _ => vec![],
        }
    }

    fn decode(&mut self, line: &str) -> Decoded {
        let mut decoded = Decoded::default();
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            decoded
                .events
                .push(AgentEvent::Unknown(Value::String(line.into())));
            return decoded;
        };
        match (msg["method"].as_str(), msg.get("id")) {
            (Some(method), Some(id)) => {
                self.server_request(method, id, &msg["params"], &msg, &mut decoded)
            }
            (Some(method), None) => self.notification(method, &msg["params"], &msg, &mut decoded),
            (None, Some(id)) => self.response(id, &msg, &mut decoded),
            _ => decoded.events.push(AgentEvent::Unknown(msg)),
        }
        decoded
    }
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn number(value: &Value) -> u64 {
    value.as_u64().unwrap_or(0)
}

/// A JSON-RPC id as the map key it is found by.
fn key(id: &Value) -> String {
    id.as_str().map_or_else(|| id.to_string(), str::to_owned)
}

/// (approval policy, sandbox, sandbox policy of a turn) of a mode. Read only asks before
/// anything but a read (`untrusted`): with `on-request` the sandbox fails a command without
/// asking (deny.jsonl and always.jsonl ask only under `untrusted`).
fn preset(mode: &str) -> (&'static str, &'static str, &'static str) {
    match mode {
        "read-only" => ("untrusted", "read-only", "readOnly"),
        FULL_ACCESS => ("never", "danger-full-access", "dangerFullAccess"),
        _ => ("on-request", "workspace-write", "workspaceWrite"),
    }
}

fn finished(ok: bool, interrupted: bool, usage: Usage, error: Option<String>) -> AgentEvent {
    AgentEvent::TurnFinished {
        ok,
        interrupted,
        usage,
        error,
    }
}

fn result(
    call: String,
    output: String,
    is_error: bool,
    files: Vec<FileChange>,
    command: Option<CommandRun>,
    web: Option<WebSearchRun>,
) -> AgentEvent {
    AgentEvent::ToolResult {
        call,
        output,
        is_error,
        files,
        command,
        web,
    }
}

/// The models `model/list` offers, the default first, hidden ones left out
/// (account.jsonl:7).
/// What fills the context, from the last request's token counts (edit-bash.jsonl:58). Codex
/// names no categories as Claude Code does; its input counts the cached part and its output
/// the reasoning, as OpenAI's usage does (inferred), so each is split off.
fn context_categories(last: &Value, window: u64) -> Vec<ContextCategory> {
    let (input, cached) = (
        number(&last["inputTokens"]),
        number(&last["cachedInputTokens"]),
    );
    let (output, reasoning) = (
        number(&last["outputTokens"]),
        number(&last["reasoningOutputTokens"]),
    );
    let used = |name: &str, tokens: u64| ContextCategory {
        name: name.into(),
        tokens,
        kind: ContextKind::Used,
    };
    let mut categories = vec![
        used("Cached input", cached),
        used("New input", input.saturating_sub(cached)),
        used("Reasoning", reasoning),
        used("Output", output.saturating_sub(reasoning)),
    ];
    categories.retain(|c| c.tokens > 0);
    let total = number(&last["totalTokens"]);
    if window > total {
        categories.push(ContextCategory {
            name: "Free space".into(),
            tokens: window - total,
            kind: ContextKind::Free,
        });
    }
    categories
}

/// Who quill says it is to Codex: itself, never another client (AGENTS.md).
fn client() -> Value {
    json!({ "name": "quill", "title": "quill", "version": env!("CARGO_PKG_VERSION") })
}

fn models(data: &Value) -> Vec<ModelInfo> {
    let mut rows: Vec<&Value> = data
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["hidden"] != true)
        .collect();
    rows.sort_by_key(|m| m["isDefault"] != true);
    rows.into_iter()
        .map(|m| ModelInfo {
            id: text(&m["model"]),
            name: m["displayName"]
                .as_str()
                .map_or_else(|| text(&m["model"]), str::to_owned),
            efforts: (m["supportedReasoningEfforts"]
                .as_array()
                .into_iter()
                .flatten())
            .map(|e| text(&e["reasoningEffort"]))
            .collect(),
            modes: vec![],
        })
        .collect()
}

/// An image as a data URL (inferred: the schema's `UserInput` image takes a `url`).
fn image_input(image: &PromptImage) -> Value {
    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(&image.data);
    json!({ "type": "image", "url": format!("data:{};base64,{data}", image.media_type) })
}

/// The command a person would type: Codex runs `<shell> -lc '<command>'` (deny.jsonl:26).
fn shell_line(command: &str) -> String {
    let Some((_, rest)) = command
        .split_once(" -lc ")
        .or_else(|| command.split_once(" -c "))
    else {
        return command.to_owned();
    };
    let rest = rest.trim();
    if let Some(quoted) = rest.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        return quoted.replace("'\\''", "'");
    }
    // A command holding single quotes comes double-quoted, a backslash before \, $, " and a
    // backtick (seen live, 2026-10-28: `"printf 'hello\\n' > hello.txt"`).
    let Some(quoted) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
        return rest.to_owned();
    };
    let mut line = String::with_capacity(quoted.len());
    let mut chars = quoted.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some(&next)) if matches!(next, '\\' | '$' | '"' | '`') => {
                line.push(next);
                chars.next();
            }
            _ => line.push(c),
        }
    }
    line
}

/// A command as the tool quill already draws: a lone read, listing or search is a step of its
/// own; anything else, or a chain of actions, is a Bash step (plan.jsonl:69, deny-patch.jsonl:53).
fn command_tool(item: &Value) -> (&'static str, Value) {
    let actions = item["commandActions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if let [action] = actions {
        let path = |key: &str| action[key].as_str().map(str::to_owned);
        match action["type"].as_str() {
            Some("read") => return ("Read", json!({ "file_path": path("path") })),
            Some("listFiles") => {
                return (
                    "LS",
                    json!({ "path": path("path").unwrap_or_else(|| ".".into()) }),
                )
            }
            // Inferred: no capture has a search.
            Some("search") => {
                let pattern = path("query").or_else(|| path("command"));
                return ("Grep", json!({ "pattern": pattern, "path": path("path") }));
            }
            _ => {}
        }
    }
    (
        "Bash",
        json!({ "command": shell_line(&text(&item["command"])) }),
    )
}

/// One edit a `fileChange` item holds per file: (call, tool, input, change). The first keeps
/// the item's id (the approval names it); the rest add their position.
fn file_edits(item: &Value) -> Vec<(String, &'static str, Value, FileChange)> {
    let id = text(&item["id"]);
    (item["changes"].as_array().into_iter().flatten())
        .enumerate()
        .map(|(i, change)| {
            let call = if i == 0 {
                id.clone()
            } else {
                format!("{id}#{i}")
            };
            let path = text(&change["path"]);
            let diff = text(&change["diff"]);
            // An added file's diff is its whole content (edit-bash.jsonl:58); an update's is a
            // hunk without the rest of the file (:69). Codex says nothing of a delete's (inferred).
            let (name, input, before, after) = match change["kind"]["type"].as_str() {
                Some("add") => (
                    "Write",
                    json!({ "file_path": path, "content": diff }),
                    None,
                    Some(diff),
                ),
                Some("delete") => {
                    let (old, _) = hunk_sides(&diff);
                    let old = if diff.contains("@@") { old } else { diff };
                    (
                        "Edit",
                        json!({ "file_path": path, "old_string": old, "new_string": "" }),
                        Some(old),
                        None,
                    )
                }
                _ => {
                    let (old, new) = hunk_sides(&diff);
                    (
                        "Edit",
                        json!({ "file_path": path, "old_string": old, "new_string": new }),
                        Some(old),
                        Some(new),
                    )
                }
            };
            (
                call,
                name,
                input,
                FileChange {
                    path: path.into(),
                    before,
                    after,
                },
            )
        })
        .collect()
}

/// The old and the new lines of a unified diff's hunks, context in both.
// shortcut: hunks only, not whole files: Codex sends no content for an update, so
// `before`/`after` hold only the lines the hunks show; read the file from the shadow repo to
// give a full-file diff.
fn hunk_sides(diff: &str) -> (String, String) {
    let (mut old, mut new) = (String::new(), String::new());
    // Anything before the first hunk is a header (`diff --git`, `---`, `+++`).
    for line in diff.lines().skip_while(|l| !l.starts_with("@@")) {
        let push = |s: &mut String, rest: &str| {
            s.push_str(rest);
            s.push('\n');
        };
        match line.chars().next() {
            Some('-') => push(&mut old, &line[1..]),
            Some('+') => push(&mut new, &line[1..]),
            Some(' ') => {
                push(&mut old, &line[1..]);
                push(&mut new, &line[1..]);
            }
            Some('@') | Some('\\') => {}
            _ => {
                push(&mut old, "");
                push(&mut new, "");
            }
        }
    }
    (old, new)
}

fn web_run(item: &Value) -> WebSearchRun {
    let query = [&item["query"], &item["action"]["query"]]
        .iter()
        .map(|q| text(q))
        .find(|q| !q.is_empty())
        .unwrap_or_default();
    let mut sources: Vec<WebSource> = vec![];
    for r in item["results"].as_array().into_iter().flatten() {
        let url = text(&r["url"]);
        if !url.is_empty() && !sources.iter().any(|s| s.url == url) {
            sources.push(WebSource {
                title: text(&r["title"]),
                url,
            });
        }
    }
    WebSearchRun { query, sources }
}

#[cfg(test)]
mod tests;
