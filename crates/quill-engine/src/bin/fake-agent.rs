//! A stand-in for an agent CLI in tests: replays a capture (`FAKE_AGENT_FIXTURE`).
//! Prints each line the real CLI sent, and where quill sent a line in the capture it
//! waits for one on stdin, so the engine's writes pace the replay as they did live.

use std::io::{BufRead, Write};

fn main() {
    let path = std::env::var("FAKE_AGENT_FIXTURE").expect("FAKE_AGENT_FIXTURE");
    let text = std::fs::read_to_string(path).expect("fixture");
    let stdin = std::io::stdin();
    let mut input = stdin.lock().lines();
    let mut out = std::io::stdout().lock();
    for line in text.lines() {
        let entry: serde_json::Value = serde_json::from_str(line).expect("fixture line");
        match entry["dir"].as_str() {
            Some("out") if input.next().is_none() => return,
            Some("in") => {
                writeln!(out, "{}", entry["msg"]).unwrap();
                out.flush().unwrap();
            }
            _ => {}
        }
    }
    // The real CLI stays until its stdin closes.
    for _ in input {}
}
