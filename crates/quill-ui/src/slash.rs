//! The "/" palette over the composer, as Desktop's slash menu (3.6556.2: `shared-5-Bg6ZMqvt.js`
//! for the trigger and the ranking, `c5cd1b4dc` for the rows). Its rows are quill's own commands
//! that map to what quill already does, then the agent's commands; a pick either inserts
//! `/name ` for the agent or runs the existing action.

use crate::{view, Session, Store, Ui};
use limn::{
    Component, ComponentPalette, Element, Key, SuggestionItem, SuggestionList, UiAction, UiEvent,
};
use quill_model::Intent;
use std::ops::Range;

/// Most characters a query may have; a longer one keeps the palette shut.
const MAX_QUERY: usize = 50;
/// The rows as the list shows them: the id a pick reports, and the item.
type Row = (String, SuggestionItem);

/// One of quill's own commands: its name and aliases, Desktop's words for it, and the action
/// the existing button or menu item reports.
struct Own {
    name: &'static str,
    aliases: &'static [&'static str],
    description: &'static str,
    action: String,
}

/// The `/` the caret is in, as its byte index and the query after it. The caret is taken to be
/// at the end of the prompt, since limn's composer does not report it. The `/` must start the
/// prompt or follow whitespace; the query has no whitespace and at most 50 characters.
pub fn slash_query(prompt: &str) -> Option<(usize, &str)> {
    let start = prompt
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(at, c)| at + c.len_utf8());
    let query = prompt[start..].strip_prefix('/')?;
    (query.chars().count() <= MAX_QUERY).then_some((start, query))
}

/// Desktop's rank (`BE`): 0 exact name, 1 exact alias, 2 name prefix, 3 alias prefix, 4 a
/// substring of the name; `None` drops the command. Case does not matter. The range is the
/// part of the name the query matched, for the list to draw bold.
fn rank<S: AsRef<str>>(
    query: &str,
    name: &str,
    aliases: &[S],
) -> Option<(u8, Option<Range<usize>>)> {
    if query.is_empty() {
        return Some((0, None));
    }
    let (q, n) = (query.to_ascii_lowercase(), name.to_ascii_lowercase());
    if n == q {
        return Some((0, Some(0..name.len())));
    }
    if aliases
        .iter()
        .any(|a| a.as_ref().eq_ignore_ascii_case(query))
    {
        return Some((1, None));
    }
    if n.starts_with(&q) {
        return Some((2, Some(0..q.len())));
    }
    if aliases
        .iter()
        .any(|a| a.as_ref().to_ascii_lowercase().starts_with(&q))
    {
        return Some((3, None));
    }
    n.find(&q).map(|at| (4, Some(at..at + q.len())))
}

/// quill's own commands whose action exists now: the ones that need a session only show once
/// one is selected, and plan only when the agent offers that mode.
fn own_commands(session: Option<&Session>, ui: &Ui) -> Vec<Own> {
    let own = |name, aliases, description, action: &str| Own {
        name,
        aliases,
        description,
        action: action.to_owned(),
    };
    let mut commands = vec![
        own(
            "model",
            &[],
            "Set the model for this session",
            view::MODEL_MENU,
        ),
        own(
            "effort",
            &[],
            "Set the effort level for this session",
            view::EFFORT_MENU,
        ),
        own(
            "permissions",
            &["allowed-tools"],
            "Change the permission mode",
            view::MODE_MENU,
        ),
    ];
    if view::offered_modes(session, ui).iter().any(|m| m == "plan") {
        commands.push(own("plan", &[], "Switch to plan mode", "mode:plan"));
    }
    commands.extend([
        own("mcp", &[], "Manage MCP connectors", view::CONNECTORS_MANAGE),
        own(
            "plugin",
            &["plugins", "marketplace"],
            "Manage plugins",
            view::PLUGINS_MANAGE,
        ),
        own(
            "skills",
            &[],
            "Browse and manage your skills",
            view::CUSTOMIZE_OPEN,
        ),
        own(
            "config",
            &["settings"],
            "Open settings",
            view::SETTINGS_OPEN,
        ),
        own(
            "usage",
            &["stats", "cost"],
            "Show plan usage and rate limits",
            view::USAGE_MENU,
        ),
    ]);
    if let Some(id) = ui.selected {
        commands.extend([
            own("tasks", &[], "Show this session's tasks", view::TASKS_OPEN),
            own(
                "diff",
                &[],
                "Show this session's changes",
                view::DIFF_TOGGLE,
            ),
            Own {
                name: "fork",
                aliases: &[],
                description: "Fork this session",
                action: format!("fork:{id}"),
            },
            Own {
                name: "rename",
                aliases: &[],
                description: "Rename this session",
                action: format!("rename:{id}"),
            },
        ]);
    }
    commands
}

/// The rows for `query`, in Desktop's order: quill's commands, then the agent's, each kept in
/// that order among equal ranks.
fn rows(store: &Store, ui: &Ui, query: &str) -> Vec<Row> {
    let session = ui.selected.and_then(|id| store.session(id));
    let agent = session
        .and_then(|s| s.capabilities.as_ref())
        .map(|c| c.commands.as_slice())
        .unwrap_or_default();
    let own = own_commands(session, ui);
    let mut found: Vec<(u8, Row)> = Vec::new();
    for command in &own {
        if let Some((rank, matched)) = rank(query, command.name, command.aliases) {
            let id = format!("slash:quill:{}", command.action);
            let item = SuggestionItem::new(id.clone(), command.name)
                .description(command.description)
                .matched(matched.into_iter().collect());
            found.push((rank, (id, item)));
        }
    }
    for command in agent {
        if let Some((rank, matched)) = rank(query, &command.name, command.aliases.as_slice()) {
            // A name quill also has goes to the agent, and says so; Desktop's rows show no
            // argument hints (the person's capture).
            let custom = own.iter().any(|o| o.name == command.name);
            let id = format!("slash:agent:{}", command.name);
            let detail = if custom { "Custom command" } else { "" };
            let item = SuggestionItem::new(id.clone(), command.name.as_str())
                .description(command.description.as_str())
                .detail(detail)
                .matched(matched.into_iter().collect());
            found.push((rank, (id, item)));
        }
    }
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, row)| row).collect()
}

/// The palette while it is open: the `/`'s byte index and its rows. Shut when the `/` was
/// dismissed, or when nothing matches.
pub(crate) fn open(store: &Store, ui: &Ui) -> Option<(usize, Vec<Row>)> {
    let (at, query) = slash_query(&ui.prompt)?;
    if ui.slash_dismissed == Some(at) {
        return None;
    }
    let rows = rows(store, ui, query);
    (!rows.is_empty()).then_some((at, rows))
}

/// The composer `anchor` with the palette above it while it is open. Rows and width are
/// Desktop's: compact, about 320 px as the person's capture (its CSS allows 240–512), in the
/// composer's text size.
pub fn list(
    store: &Store,
    ui: &Ui,
    anchor: Element,
    width: f32,
    palette: ComponentPalette,
) -> Element {
    let Some((_, rows)) = open(store, ui) else {
        return anchor;
    };
    let selected = ui.slash_selected.min(rows.len() - 1);
    let items = rows.into_iter().map(|(_, item)| item).collect();
    let mut list = SuggestionList::new(
        anchor,
        "slash",
        items,
        selected,
        true,
        "slash:dismiss",
        palette,
    )
    .compact(true)
    .sizing(quill_components::pieces::menu_sizing(320.0))
    .max_width(320.0)
    .width(width)
    .placement(limn::PopoverPlacement::TopStart);
    // Until the session runs, the agent's commands are not known.
    if ui
        .selected
        .and_then(|id| store.session(id))
        .is_none_or(|s| s.capabilities.is_none())
    {
        list = list.title("More commands available once the session is running");
    }
    list.render()
}

/// A key the palette takes before limn while it is open over the focused composer: the arrows
/// move (no wrap), Enter and Tab pick, Escape shuts it for this `/`. `None` otherwise, so
/// Enter sends and the rest reaches the composer.
pub fn key(
    store: &Store,
    ui: &mut Ui,
    focused: Option<&str>,
    event: &UiEvent,
) -> Option<Vec<Intent>> {
    if focused != Some(view::COMPOSER) {
        return None;
    }
    let UiEvent::Key {
        key,
        ctrl: false,
        shift,
    } = event
    else {
        return None;
    };
    let (at, rows) = open(store, ui)?;
    let last = rows.len() - 1;
    let selected = ui.slash_selected.min(last);
    match key {
        Key::ArrowDown => ui.slash_selected = (selected + 1).min(last),
        Key::ArrowUp => ui.slash_selected = selected.saturating_sub(1),
        Key::Enter if !shift => return Some(pick(store, ui, &rows[selected].0)),
        Key::Tab => return Some(pick(store, ui, &rows[selected].0)),
        Key::Escape => ui.slash_dismissed = Some(at),
        _ => return None,
    }
    Some(vec![])
}

/// The palette's clicks: a row picked, or a press outside it, which shuts it for this `/`.
pub fn clicked(store: &Store, ui: &mut Ui, id: &str) -> Option<Vec<Intent>> {
    if id == "slash:dismiss" {
        ui.slash_dismissed = slash_query(&ui.prompt).map(|(at, _)| at);
        return Some(vec![]);
    }
    (id.starts_with("slash:agent:") || id.starts_with("slash:quill:")).then(|| pick(store, ui, id))
}

/// Picks row `id` in place of the `/query`. An agent command becomes `/name ` for the agent to
/// read; a quill command leaves the prompt without the token and runs its action, as its
/// button or menu item would.
fn pick(store: &Store, ui: &mut Ui, id: &str) -> Vec<Intent> {
    let Some((at, _)) = slash_query(&ui.prompt) else {
        return vec![];
    };
    let head = ui.prompt[..at].to_owned();
    ui.slash_selected = 0;
    ui.slash_dismissed = None;
    if let Some(name) = id.strip_prefix("slash:agent:") {
        ui.restore_prompt = Some(format!("{head}/{name} "));
        return vec![];
    }
    let Some(action) = id.strip_prefix("slash:quill:") else {
        return vec![];
    };
    ui.restore_prompt = Some(head.trim_end().to_owned());
    crate::update(
        store,
        ui,
        &UiAction::Clicked {
            id: action.to_owned(),
        },
    )
}
