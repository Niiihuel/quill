# quill

A native desktop app, in Rust with [limn](https://github.com/Niiihuel/limn), for working with coding agents: pick a coding-agent CLI (Claude Code first; Codex, OpenCode, Grok and others through adapters) and drive it in local sessions, with permissions, a per-request record of what changed, diffs and a terminal.

Status: specification. Read [`docs/spec.md`](docs/spec.md), then [`docs/plan.md`](docs/plan.md).

quill is not affiliated with Anthropic or any other agent vendor. It talks to the agent CLIs the user has installed and signed in to, and uses no code, fonts, icons or branding of their vendors.
