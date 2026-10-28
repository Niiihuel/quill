//! A stand-in for an agent CLI in tests: replays the capture named in `.fake-agent` in its
//! working directory (each test session gets its own directory, so tests run side by side).
//! The file tools it replays (Write, Edit, Bash) really run there, so the folder changes as
//! it did live and the engine attribution can be tested.
//! Prints each line the real CLI sent, and where quill sent a line in the capture it
//! waits for one on stdin, so the engine's writes pace the replay as they did live.
//! `QUILL_FAKE_PACE=<factor>` also keeps the capture's time between lines, scaled, so a
//! person can watch a turn run.

use std::io::{BufRead, Write};

fn main() {
    if std::env::args().nth(1).as_deref() == Some("auth") {
        return auth();
    }
    let path =
        std::fs::read_to_string(".fake-agent").expect(".fake-agent in the working directory");
    let path = path.trim();
    let text = std::fs::read_to_string(path).expect("fixture");
    let stdin = std::io::stdin();
    let mut input = stdin.lock().lines();
    let mut out = std::io::stdout().lock();
    let pace: Option<f64> = std::env::var("QUILL_FAKE_PACE")
        .ok()
        .and_then(|p| p.parse().ok());
    let mut last = 0u64;
    for line in text.lines() {
        let entry: serde_json::Value = serde_json::from_str(line).expect("fixture line");
        let at = entry["t_ms"].as_u64().unwrap_or(last);
        if let Some(pace) = pace {
            let wait = at.saturating_sub(last) as f64 * pace;
            std::thread::sleep(std::time::Duration::from_millis(wait as u64));
        }
        last = at;
        match entry["dir"].as_str() {
            Some("out") if !next_line(&mut input, &mut out) => return,
            Some("in") => {
                run_tools(&entry["msg"]);
                writeln!(out, "{}", entry["msg"]).unwrap();
                out.flush().unwrap();
            }
            _ => {}
        }
    }
    // The real CLI stays until its stdin closes.
    for _ in input {}
}

/// The next line quill wrote that a capture accounts for. The Codex adapter also asks
/// `model/list` at start, which no capture holds: it is answered with no models, so the adapter
/// keeps its own, and does not count as the line the capture waits for.
fn next_line(
    input: &mut impl Iterator<Item = std::io::Result<String>>,
    out: &mut impl Write,
) -> bool {
    loop {
        let Some(Ok(line)) = input.next() else {
            return false;
        };
        let asked: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
        if asked["method"] != "model/list" {
            return true;
        }
        let answer =
            serde_json::json!({ "id": asked["id"], "result": { "data": [], "nextCursor": null } });
        writeln!(out, "{answer}").unwrap();
        out.flush().unwrap();
    }
}

/// `auth status|login|logout` as Claude Code 2.1.312 answers them (cli-auth.md §1-3). The
/// account starts as `QUILL_FAKE_AUTH` says (`signed-in`, the default, or `signed-out`); a login
/// or a logout flips it through a marker file (`QUILL_FAKE_AUTH_MARKER`, else `.fake-auth` in the
/// working directory), so the next `status` agrees.
fn auth() {
    let args: Vec<String> = std::env::args().collect();
    let marker = std::env::var("QUILL_FAKE_AUTH_MARKER").unwrap_or_else(|_| ".fake-auth".into());
    let base = std::env::var("QUILL_FAKE_AUTH").as_deref() != Ok("signed-out");
    let set = |signed_in: bool| {
        if signed_in == base {
            let _ = std::fs::remove_file(&marker);
        } else {
            let _ = std::fs::write(&marker, "");
        }
    };
    let signed_in = base != std::path::Path::new(&marker).exists();
    match args.get(2).map(String::as_str) {
        Some("status") if signed_in => {
            println!(
                r#"{{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"ada@example.com","orgName":"Analytical Engines","subscriptionType":"max"}}"#
            );
        }
        Some("status") => {
            println!(r#"{{"loggedIn":false,"authMethod":"none","apiProvider":"firstParty"}}"#);
            std::process::exit(1);
        }
        Some("login") => {
            // The real one wraps the address in an OSC 8 link even without a terminal, and
            // its last line has no newline.
            let url = "https://claude.com/cai/oauth/authorize?code=true&client_id=fake&state=fake";
            println!("Opening browser to sign in\u{2026}");
            println!("If the browser didn't open, visit: \x1b]8;;{url}\x07{url}\x1b]8;;\x07");
            print!("Paste code here if prompted > ");
            let _ = std::io::stdout().flush();
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                // `fail…` stands for a rejected code: the program gives up.
                if line.starts_with("fail") {
                    eprintln!("Login failed: the code was not accepted");
                    std::process::exit(1);
                }
                if line.contains('#') {
                    set(true);
                    println!("Login successful.");
                    return;
                }
                eprintln!("Invalid code. Please make sure the full code was copied.");
            }
            std::process::exit(130);
        }
        Some("logout") => {
            set(false);
            println!("Successfully logged out from your Anthropic account.");
        }
        _ => std::process::exit(2),
    }
}

/// Do what the captured tool calls did, in this directory. Paths under `/work/` (the
/// anonymized capture folder) are taken from here.
fn run_tools(msg: &serde_json::Value) {
    if msg["type"] != "assistant" {
        return;
    }
    let local = |path: &str| path.strip_prefix("/work/").unwrap_or(path).to_owned();
    let text = |value: &serde_json::Value| value.as_str().unwrap_or_default().to_owned();
    for block in msg["message"]["content"].as_array().into_iter().flatten() {
        if block["type"] != "tool_use" {
            continue;
        }
        let input = &block["input"];
        match block["name"].as_str() {
            Some("Write") => {
                let _ = std::fs::write(local(&text(&input["file_path"])), text(&input["content"]));
            }
            Some("Edit") => {
                let path = local(&text(&input["file_path"]));
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let edited = content.replacen(
                        &text(&input["old_string"]),
                        &text(&input["new_string"]),
                        1,
                    );
                    let _ = std::fs::write(&path, edited);
                }
            }
            Some("Bash") => {
                let command = text(&input["command"]).replace("/work/", "");
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .status();
            }
            _ => {}
        }
    }
}
