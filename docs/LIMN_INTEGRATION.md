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
| — | `feat/switch-off-ink` | Settings' off switches on Desktop's pale gray track | local, not pushed |
| — | `feat/segmented-icons` | Settings' theme picker: 26 px icon segments and the chosen one outlined | local, not pushed |
| — | `feat/action-row-line-height` | Settings' descriptions with their lines 20 px apart | local, not pushed |
| — | `feat/focus-area` | the browser pane's input surface (`Element::focus_area` public) | local, not pushed |
| [#132](https://github.com/Niiihuel/limn/pull/132) | `feat/code-editor-host-look` | quill-ide's editor in quill's code inks (types and properties too), and its own mode in the footer (on #73) | open |
| [#133](https://github.com/Niiihuel/limn/pull/133) | `feat/context-menu-sizing` | context menus at the measures of quill's menus, in quill and quill-ide | open |
| [#134](https://github.com/Niiihuel/limn/pull/134) | `feat/code-editor-change-marks` | quill-ide's git marks in the editor's gutter | open |
| [#135](https://github.com/Niiihuel/limn/pull/135) | `fix/code-editor-line-width` | the editor's horizontal scroll without flicker | open |
| [#137](https://github.com/Niiihuel/limn/pull/137) | `feat/code-editor-scrollbar` | quill-ide's editor scrollbar in quill's rail with its arrows | open |
| [#138](https://github.com/Niiihuel/limn/pull/138) | `feat/menu-picture` | the installed editors' own icons in quill's "Open in" menu | open |
| — | `feat/surface-image` | the browser's native frames handed to egui as they are (`SurfaceFrame::from_image`) | local, not pushed |
| — | `feat/surface-texture` | the browser's GPU frames drawn from a texture quill registered with egui-wgpu (`present_surface_texture`) | local, not pushed |
| — | `feat/surface-paint` | the page presented beneath the window: quill paints the browser's surface itself, a transparent hole (`present_surface_paint`) | local, not pushed |
| — | `feat/drawn-over` | no longer used: native tabs now paint into their surface, so nothing has to step aside for quill (docs/browser.md §8) | local, not pushed; can be dropped |

Updating the branch: `git -C ../limn-quill-integration merge <branch>`; the only conflicts so
far were rows added side by side in `docs/components.md`, `CHANGELOG.md` and `lib.rs`, and
`site/src/data/pages.json`, regenerated with `cargo run -p limn-gallery -- --pages`.
