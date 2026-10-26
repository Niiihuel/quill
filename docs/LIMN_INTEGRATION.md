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
| [#128](https://github.com/Niiihuel/limn/pull/128) | `feat/spinner-dots` | the running line's spinner | open |
| [#129](https://github.com/Niiihuel/limn/pull/129) | `feat/choice-fill` | the accent of the chosen radio on question cards | open |
| [#130](https://github.com/Niiihuel/limn/pull/130) | `feat/stream-fade` | the reply fading in as it streams | open |
| — | `feat/rolling-number` | the branch bar's +N −M rolling to each new count | local, not pushed |
| — | `fix/submenu-gap` | the composer's submenus | local, not pushed |
| — | `feat/modal-flush-body` | Settings' modal, a body without padding | local, not pushed |
| — | `feat/select-plain` | Settings' plain selects | local, not pushed |
| — | `feat/focus-area` | the browser pane's input surface (`Element::focus_area` public) | local, not pushed |

Updating the branch: `git -C ../limn-quill-integration merge <branch>`; the only conflicts so
far were rows added side by side in `docs/components.md`, `CHANGELOG.md` and `lib.rs`, and
`site/src/data/pages.json`, regenerated with `cargo run -p limn-gallery -- --pages`.
