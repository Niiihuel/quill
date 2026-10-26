# quill

A native desktop app, in Rust with [limn](https://github.com/Niiihuel/limn), for working with coding agents: pick a coding-agent CLI (Claude Code first; Codex, OpenCode, Grok and others through adapters) and drive it in local sessions, with permissions, a per-request record of what changed, diffs and a terminal.

Status: specification. Read [`docs/spec.md`](docs/spec.md), then [`docs/plan.md`](docs/plan.md).

quill is not affiliated with Anthropic or any other agent vendor. It talks to the agent CLIs the user has installed and signed in to, and uses no code, fonts, icons or branding of their vendors.

## quill-ide

`apps/quill-ide` is a second, separate binary: a code reader and editor in quill's look (`crates/quill-components`), keyboard first, with limn's Vim mode. `cargo run -p quill-ide -- [folder | file]`. Files open in Vim's Normal mode, to read; `i` writes and Ctrl+S saves. What others write to an open file (an agent) shows within a second, unless it has changes of yours, which are kept until you choose. Keys: Ctrl+Shift+E shows and hides the files, Ctrl+PageDown and Ctrl+PageUp go through the open ones, Ctrl+W closes one.
