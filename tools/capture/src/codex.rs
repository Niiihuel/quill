//! The Codex driver of quill-capture: `quill-capture --agent codex <scenario>` records what
//! `codex app-server` says over stdio JSON-RPC (docs/codex.md, phase 1). Same capture format as
//! the Claude driver: `captures/<scenario>-<epoch>/session.jsonl`, one `{t_ms, dir, msg}` per
//! line (`out` = quill → CLI, `in` = CLI → quill, `stderr`, `exit`). Each run gets a fresh git repo.
//!
//! Always `gpt-6-luna` at effort `low`; never `danger-full-access` or approval policy `never`.
//! Scenarios: simple, edit-bash, deny, deny-patch, always, plan, steer, interrupt, resume, fork, think,
//! websearch, ask, account.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MODEL: &str = "gpt-6-luna";

enum Step {
    /// `turn/start`, then wait for `turn/completed`.
    Turn(&'static str),
    /// `turn/start`; at the first item after the user's own, `turn/steer`; wait for the end.
    Steer(&'static str, &'static str),
    /// `turn/start`; at the first item after the user's own, `turn/interrupt`; wait for the end.
    Interrupt(&'static str),
    /// Close the process, start another one, `thread/resume` and `thread/turns/list`.
    Restart,
    /// `thread/fork` of the thread; later turns run on the fork.
    Fork,
    /// Any request, no thread needed.
    Rpc(&'static str, Value),
}

struct Scenario {
    approval: &'static str,
    sandbox: &'static str,
    /// Answer to approvals; `decline_first` declines only the first one and accepts the rest.
    decision: &'static str,
    decline_first: bool,
    thread: bool,
    config: Value,
    steps: Vec<Step>,
}

fn scenario(name: &str) -> Option<Scenario> {
    use Step::*;
    const RO: (&str, &str) = ("on-request", "read-only");
    const UNTRUSTED: (&str, &str) = ("untrusted", "read-only");
    const AUTO: (&str, &str) = ("on-request", "workspace-write");
    const EDIT: &str =
        "Using apply_patch, create notes.txt containing the line hello. Then, with apply_patch, \
                        edit it so the line reads hello world. Then run exactly: cat notes.txt";
    let ((approval, sandbox), decision, steps) = match name {
        "simple" => (AUTO, "accept", vec![Turn("Reply with exactly the word: ok")]),
        "edit-bash" => (RO, "accept", vec![Turn(EDIT)]),
        "deny-patch" => (RO, "decline", vec![Turn(EDIT)]),
        // `on-request` never asks for a command the sandbox merely blocks, so these two ask for
        // everything not known to be safe.
        "deny" => (UNTRUSTED, "decline", vec![Turn("Use the shell tool to run exactly: touch denied.txt")]),
        "always" => (
            UNTRUSTED,
            "acceptForSession",
            vec![Turn(
                "Use the shell tool to run exactly: touch one.txt. Then use the shell tool again to run exactly: touch one.txt",
            )],
        ),
        "plan" => (
            AUTO,
            "accept",
            vec![Turn(
                "Call your update_plan tool with a 3-step plan: create a.txt, create b.txt, list the files. \
                 Then do each step with the shell, calling update_plan as each step completes.",
            )],
        ),
        "steer" => (
            AUTO,
            "accept",
            vec![Steer(
                "Count from 1 to 40, one number per line, nothing else.",
                "Change of plans: stop at 5.",
            )],
        ),
        "interrupt" => (AUTO, "accept", vec![Interrupt("Write a 500 word story about a quill.")]),
        "resume" => (
            AUTO,
            "accept",
            vec![
                Turn("Remember the word: lantern. Reply with exactly: ok"),
                Restart,
                Turn("What word did I ask you to remember? One word."),
            ],
        ),
        "fork" => (
            AUTO,
            "accept",
            vec![
                Turn("Remember the word: lantern. Reply with exactly: ok"),
                Fork,
                Turn("What word did I ask you to remember? One word."),
            ],
        ),
        "think" => (AUTO, "accept", vec![Turn("Which is larger, 17^23 or 23^17? Reason carefully, then answer in one line.")]),
        "websearch" => (
            AUTO,
            "accept",
            vec![Turn("Search the web for the latest stable release of the Rust compiler and tell me its version in one line.")],
        ),
        "ask" => (
            AUTO,
            "accept",
            vec![Turn(
                "Use the request_user_input tool to ask me to pick red or blue (multiple choice), then reply with my answer only.",
            )],
        ),
        "account" => (
            AUTO,
            "accept",
            vec![
                Rpc("model/list", json!({})),
                Rpc("account/read", json!({})),
                Rpc("account/rateLimits/read", json!({})),
                Rpc("mcpServerStatus/list", json!({})),
            ],
        ),
        _ => return None,
    };
    Some(Scenario {
        approval,
        sandbox,
        decision,
        decline_first: name == "deny",
        thread: name != "account",
        // Per-thread config overrides (not the user's config file): `update_plan` is off and
        // `request_user_input` is Plan-mode only unless set.
        config: match name {
            "plan" => json!({ "tools": { "update_plan": { "enabled": true } } }),
            "ask" => json!({ "features": { "default_mode_request_user_input": true } }),
            _ => Value::Null,
        },
        steps,
    })
}

struct Log {
    start: Instant,
    file: Mutex<std::fs::File>,
}

impl Log {
    fn write(&self, dir: &str, msg: Value) {
        let line =
            json!({ "t_ms": self.start.elapsed().as_millis() as u64, "dir": dir, "msg": msg });
        let _ = writeln!(self.file.lock().unwrap(), "{line}");
    }
}

/// One `codex app-server` process.
struct Proc {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    log: Arc<Log>,
    next_id: u64,
}

impl Proc {
    fn spawn(repo: &std::path::Path, log: &Arc<Log>) -> Result<Proc, String> {
        let mut child = Command::new("codex")
            .arg("app-server")
            .current_dir(repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn codex: {e}"))?;
        let (tx, rx) = mpsc::channel::<Value>();
        let stdout = child.stdout.take().unwrap();
        let out_log = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let msg = serde_json::from_str(&line).unwrap_or(Value::String(line));
                out_log.write("in", msg.clone());
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        let stderr = child.stderr.take().unwrap();
        let err_log = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                err_log.write("stderr", Value::String(line));
            }
        });
        Ok(Proc {
            stdin: child.stdin.take().unwrap(),
            child,
            rx,
            log: log.clone(),
            next_id: 0,
        })
    }

    fn send(&mut self, msg: Value) {
        self.log.write("out", msg.clone());
        writeln!(self.stdin, "{msg}").expect("codex stdin closed");
        self.stdin.flush().expect("codex stdin closed");
    }

    /// Close stdin, give the process a moment, then kill it by its own handle.
    fn close(self) {
        let Proc {
            mut child,
            stdin,
            log,
            ..
        } = self;
        drop(stdin);
        let deadline = Instant::now() + Duration::from_secs(5);
        let code = loop {
            if let Ok(Some(status)) = child.try_wait() {
                break status.code();
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        log.write("exit", json!({ "code": code }));
    }
}

/// What the scenario has seen so far.
#[derive(Default)]
struct State {
    thread_id: String,
    turn_id: String,
    turn_done: bool,
    /// Items started in the current turn other than the user's own message.
    work_started: bool,
    approvals: u32,
}

struct Driver {
    p: Proc,
    st: State,
    decision: &'static str,
    decline_first: bool,
    /// `turn/start.summary`: reasoning summaries are off unless asked for.
    summary: Option<&'static str>,
    /// Sent again on resume and fork, which otherwise take the user's own configured defaults.
    approval: &'static str,
    sandbox: &'static str,
}

impl Driver {
    /// Read until `done`, answering the server's own requests on the way.
    fn pump(
        &mut self,
        limit: Duration,
        done: impl Fn(&State, &Value) -> bool,
    ) -> Result<Value, String> {
        let deadline = Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = self
                .p
                .rx
                .recv_timeout(left)
                .map_err(|e| format!("waiting: {e}"))?;
            self.observe(&msg);
            if msg["method"].is_string() && !msg["id"].is_null() {
                let reply = self.answer(&msg);
                self.p.send(reply);
            }
            if done(&self.st, &msg) {
                return Ok(msg);
            }
        }
    }

    fn observe(&mut self, msg: &Value) {
        let (m, p) = (msg["method"].as_str().unwrap_or(""), &msg["params"]);
        match m {
            "turn/started" => {
                self.st.turn_id = p["turn"]["id"].as_str().unwrap_or("").into();
                self.st.turn_done = false;
                self.st.work_started = false;
            }
            "turn/completed" => self.st.turn_done = true,
            "item/started" if p["item"]["type"] != "userMessage" => self.st.work_started = true,
            _ => {}
        }
    }

    /// The JSON-RPC response to one server request.
    fn answer(&mut self, req: &Value) -> Value {
        let id = req["id"].clone();
        let result = match req["method"].as_str().unwrap_or("") {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                self.st.approvals += 1;
                let first_declined = self.decline_first && self.st.approvals == 1;
                let decision = if self.decline_first && !first_declined {
                    "accept"
                } else {
                    self.decision
                };
                json!({ "decision": decision })
            }
            "item/tool/requestUserInput" => {
                let answers: serde_json::Map<String, Value> = req["params"]["questions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|q| {
                        let label = q["options"][0]["label"].as_str()?;
                        Some((q["id"].as_str()?.to_owned(), json!({ "answers": [label] })))
                    })
                    .collect();
                json!({ "answers": answers })
            }
            // Grant nothing: quill-capture never widens a sandbox.
            "item/permissions/requestApproval" => json!({ "permissions": {} }),
            "mcpServer/elicitation/request" => json!({ "action": "decline" }),
            _ => {
                return json!({ "id": id, "error": { "code": -32601, "message": "not supported by quill-capture" } })
            }
        };
        json!({ "id": id, "result": result })
    }

    /// A request and its response (the `result`; an `error` becomes `Err`).
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.p.next_id += 1;
        let id = self.p.next_id;
        self.p
            .send(json!({ "id": id, "method": method, "params": params }));
        let msg = self.pump(Duration::from_secs(120), |_, m| {
            m["id"] == id && m["method"].is_null()
        })?;
        if msg["error"].is_object() {
            return Err(format!("{method}: {}", msg["error"]));
        }
        Ok(msg["result"].clone())
    }

    fn handshake(&mut self) -> Result<(), String> {
        self.call(
            "initialize",
            json!({ "clientInfo": { "name": "quill", "title": "quill", "version": "0.0.1" } }),
        )?;
        self.p.send(json!({ "method": "initialized" }));
        Ok(())
    }

    fn start_turn(&mut self, text: &str) -> Result<(), String> {
        self.st.turn_done = false;
        self.st.work_started = false;
        let mut params = json!({ "threadId": self.st.thread_id, "input": [{ "type": "text", "text": text }], "effort": "low" });
        if let Some(summary) = self.summary {
            params["summary"] = json!(summary);
        }
        let r = self.call("turn/start", params)?;
        if let Some(id) = r["turn"]["id"].as_str() {
            self.st.turn_id = id.into();
        }
        Ok(())
    }

    fn finish_turn(&mut self) -> Result<(), String> {
        if !self.st.turn_done {
            self.pump(Duration::from_secs(300), |_, m| {
                m["method"] == "turn/completed"
            })?;
        }
        Ok(())
    }

    fn wait_work(&mut self) -> Result<(), String> {
        if !self.st.work_started && !self.st.turn_done {
            self.pump(Duration::from_secs(300), |s, _| {
                s.work_started || s.turn_done
            })?;
        }
        Ok(())
    }
}

pub fn run(name: &str) -> Result<(), String> {
    let sc = scenario(name).ok_or_else(|| format!("unknown codex scenario {name}"))?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let dir = std::path::PathBuf::from(format!("captures/{name}-{epoch}"));
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).map_err(|e| e.to_string())?;
    std::fs::write(repo.join("README.md"), "# capture\n").map_err(|e| e.to_string())?;
    for args in [
        &["init", "-q"][..],
        &["add", "-A"],
        &[
            "-c",
            "user.name=quill",
            "-c",
            "user.email=quill@localhost",
            "commit",
            "-qm",
            "init",
        ],
    ] {
        Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .map_err(|e| e.to_string())?;
    }
    let repo = repo.canonicalize().map_err(|e| e.to_string())?;
    let log = Arc::new(Log {
        start: Instant::now(),
        file: Mutex::new(
            std::fs::File::create(dir.join("session.jsonl")).map_err(|e| e.to_string())?,
        ),
    });
    let version = Command::new("codex")
        .arg("--version")
        .output()
        .map_err(|e| e.to_string())?;
    let meta = json!({
        "scenario": name, "cli_version": String::from_utf8_lossy(&version.stdout).trim(),
        "model": MODEL, "approval": sc.approval, "sandbox": sc.sandbox, "epoch": epoch,
        "repo": repo,
    });
    std::fs::write(
        dir.join("meta.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let mut d = Driver {
        p: Proc::spawn(&repo, &log)?,
        st: State::default(),
        decision: sc.decision,
        decline_first: sc.decline_first,
        summary: (name == "think").then_some("detailed"),
        approval: sc.approval,
        sandbox: sc.sandbox,
    };
    // A failed step still ends the run cleanly, so the capture of what happened is kept.
    let outcome = (|| -> Result<(), String> {
        d.handshake()?;
        if sc.thread {
            let mut params = json!({
                "cwd": repo, "model": MODEL, "approvalPolicy": sc.approval, "sandbox": sc.sandbox,
            });
            if !sc.config.is_null() {
                params["config"] = sc.config.clone();
            }
            let r = d.call("thread/start", params)?;
            d.st.thread_id = r["thread"]["id"]
                .as_str()
                .ok_or("thread/start: no thread id")?
                .into();
        }
        for step in &sc.steps {
            match step {
                Step::Turn(text) => {
                    d.start_turn(text)?;
                    d.finish_turn()?;
                }
                Step::Steer(text, then) => {
                    d.start_turn(text)?;
                    d.wait_work()?;
                    let (thread, turn) = (d.st.thread_id.clone(), d.st.turn_id.clone());
                    d.call(
                        "turn/steer",
                        json!({ "threadId": thread, "expectedTurnId": turn, "input": [{ "type": "text", "text": then }] }),
                    )?;
                    d.finish_turn()?;
                }
                Step::Interrupt(text) => {
                    d.start_turn(text)?;
                    d.wait_work()?;
                    let (thread, turn) = (d.st.thread_id.clone(), d.st.turn_id.clone());
                    d.call(
                        "turn/interrupt",
                        json!({ "threadId": thread, "turnId": turn }),
                    )?;
                    d.finish_turn()?;
                }
                Step::Restart => {
                    let old = std::mem::replace(&mut d.p, Proc::spawn(&repo, &log)?);
                    old.close();
                    d.p.next_id = 0;
                    d.handshake()?;
                    let thread = d.st.thread_id.clone();
                    let params = json!({ "threadId": thread, "approvalPolicy": d.approval, "sandbox": d.sandbox });
                    d.call("thread/resume", params)?;
                    // Whether resume repeats the items or they must be read on their own.
                    let _ = d.call("thread/turns/list", json!({ "threadId": thread }));
                }
                Step::Fork => {
                    let thread = d.st.thread_id.clone();
                    let params = json!({ "threadId": thread, "approvalPolicy": d.approval, "sandbox": d.sandbox });
                    let r = d.call("thread/fork", params)?;
                    d.st.thread_id = r["thread"]["id"]
                        .as_str()
                        .ok_or("thread/fork: no thread id")?
                        .into();
                }
                Step::Rpc(method, params) => {
                    d.call(method, params.clone())?;
                }
            }
        }
        Ok(())
    })();
    d.p.close();
    println!("{}", dir.display());
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scenario_exists_and_none_widens_the_sandbox() {
        for name in [
            "simple",
            "edit-bash",
            "deny",
            "deny-patch",
            "always",
            "plan",
            "steer",
            "interrupt",
            "resume",
            "fork",
            "think",
            "websearch",
            "ask",
            "account",
        ] {
            let sc = scenario(name).expect(name);
            assert_ne!(sc.sandbox, "danger-full-access", "{name}");
            assert_ne!(sc.approval, "never", "{name}");
        }
    }
}
