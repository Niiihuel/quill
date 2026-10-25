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
            Some("out") if input.next().is_none() => return,
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
