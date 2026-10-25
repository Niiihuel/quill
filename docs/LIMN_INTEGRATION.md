# limn integration

quill builds against `../limn-quill-integration`, a worktree of limn on the branch
`quill/integration`: `main` plus the pull requests quill needs that are not merged yet.
When they are all merged, point `Cargo.toml` back at `../lienzo` and drop the branch.

| PR | Branch | What quill uses it for | State |
|---|---|---|---|
| [#120](https://github.com/Niiihuel/limn/pull/120) | `feat/split-view` | chat \| diff and diff / terminal panes | open |
| [#121](https://github.com/Niiihuel/limn/pull/121) | `feat/code-diff` | word highlights, folded context, file headers in the changes panel | open |
| [#122](https://github.com/Niiihuel/limn/pull/122) | `feat/terminal` | the terminal panel | open |
| [#123](https://github.com/Niiihuel/limn/pull/123) | `feat/split-button` | "Always allow ▾" on the permission card | open |
| [#124](https://github.com/Niiihuel/limn/pull/124) | `feat/status-marks` | pulsing session dots, the context ring | open |

Updating the branch: `git -C ../limn-quill-integration merge <branch>`; the only conflicts so
far were rows added side by side in `docs/components.md`, `CHANGELOG.md` and `lib.rs`, and
`site/src/data/pages.json`, regenerated with `cargo run -p limn-gallery -- --pages`.
