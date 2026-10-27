//! quill's own updates in the window: Settings' Updates group, the account menu's entry, the
//! toast once a version is ready, and the dialog a mandatory one puts up (the engine checks and
//! downloads; this shows what it reports).

use crate::view::ago;
use crate::Ui;
use limn::{Component, ComponentPalette, Element};
use quill_components::look::{self, BODY};
use quill_components::pieces::{hgap, keyed_button, muted};
use quill_components::settings::{setting, setting_switch, Setting};
use quill_model::{Event, Intent, UpdateState};

pub const UPDATE_CHECK: &str = "update:check";
pub const UPDATE_DOWNLOAD: &str = "update:download";
pub const UPDATE_RETRY: &str = "update:retry";
pub const UPDATE_RESTART: &str = "update:restart";
/// The Settings row's own button: not the toast's ID, which shares its draw state.
pub const UPDATE_RESTART_ROW: &str = "update:restart:row";
pub const UPDATE_TOAST_DISMISS: &str = "update:toast:dismiss";
/// The mandatory dialog's close: it has none, so a click here does nothing.
const UPDATE_HOLD: &str = "update:hold";

/// Keeps what the engine reports; a version's toast shows once, when it becomes ready (a
/// mandatory one is the dialog's, not a toast's).
pub(crate) fn heard(ui: &mut Ui, event: &Event) {
    let Event::Update(state) = event else {
        return;
    };
    if let UpdateState::Ready {
        version,
        mandatory: false,
    } = state
    {
        let same = matches!(&ui.update, UpdateState::Ready { version: now, .. } if now == version);
        if !same {
            ui.update_toast = Some(version.clone());
        }
    }
    ui.update = state.clone();
}

/// The clicks of this module, or `None` for another's.
pub(crate) fn clicked(ui: &mut Ui, id: &str) -> Option<Vec<Intent>> {
    match id {
        UPDATE_CHECK | UPDATE_RETRY => Some(vec![Intent::CheckUpdates]),
        UPDATE_DOWNLOAD => Some(vec![Intent::DownloadUpdate]),
        UPDATE_RESTART | UPDATE_RESTART_ROW => {
            ui.account_menu = false;
            ui.update_toast = None;
            Some(vec![Intent::InstallUpdate])
        }
        UPDATE_TOAST_DISMISS => {
            ui.update_toast = None;
            Some(vec![])
        }
        _ => None,
    }
}

/// The version and what it says about itself, for the state line.
fn state_line(state: &UpdateState) -> String {
    match state {
        UpdateState::Idle => "quill has not checked for updates yet.".into(),
        UpdateState::Checking => "Checking for updates\u{2026}".into(),
        UpdateState::UpToDate { checked_at } => {
            format!("quill is up to date \u{b7} Checked {}", ago(*checked_at))
        }
        UpdateState::Available { version, .. } => format!("Version {version} is available"),
        UpdateState::Downloading {
            version,
            done,
            total,
            ..
        } => {
            let percent = if *total == 0 { 0 } else { done * 100 / total };
            format!("Downloading {version} \u{b7} {percent}%")
        }
        UpdateState::Ready { version, .. } => format!("Version {version} is ready to install"),
        UpdateState::Installed { version, .. } => format!("Installing {version}\u{2026}"),
        UpdateState::Failed { reason } => reason.clone(),
        UpdateState::FromSource => "Built from source \u{2014} update with git pull".into(),
        UpdateState::NotInstalled => "Run quill --install to keep it up to date".into(),
        UpdateState::NeedsAccess => {
            "Updates need access to the private repository (set QUILL_UPDATE_TOKEN or sign in with gh)"
                .into()
        }
    }
}

/// Settings' Updates group: the state and its buttons, the release notes when there are any,
/// and the switch that downloads on its own.
pub(crate) fn settings(ui: &Ui, auto_update: bool, palette: ComponentPalette) -> Vec<Setting> {
    let mut buttons = vec![];
    match &ui.update {
        UpdateState::Ready { .. } => buttons.push(keyed_button(
            UPDATE_RESTART_ROW,
            "Restart to update",
            "",
            true,
            true,
            palette,
        )),
        UpdateState::Available {
            mandatory: false, ..
        } => buttons.push(keyed_button(
            UPDATE_DOWNLOAD,
            "Download",
            "",
            false,
            true,
            palette,
        )),
        UpdateState::Failed { .. } => buttons.push(keyed_button(
            UPDATE_RETRY,
            "Retry",
            "",
            false,
            true,
            palette,
        )),
        _ => {}
    }
    if !matches!(
        ui.update,
        UpdateState::Checking | UpdateState::FromSource | UpdateState::NotInstalled
    ) {
        buttons.push(keyed_button(
            UPDATE_CHECK,
            "Check for updates",
            "",
            false,
            true,
            palette,
        ));
    }
    let mut out = vec![setting(
        "Updates",
        &state_line(&ui.update),
        Element::row(buttons).gap(8.0),
    )];
    if let UpdateState::Available { notes, .. } = &ui.update {
        if !notes.trim().is_empty() {
            out.push(setting(
                "Release notes",
                "",
                limn::Markdown::new(notes, palette)
                    .width(460.0)
                    .font_size(BODY)
                    .render()
                    .element,
            ));
        }
    }
    out.push(setting(
        "Install updates automatically",
        "Download new versions in the background. Installing always waits for you to restart.",
        setting_switch("auto_update", auto_update, palette),
    ));
    out
}

/// The account menu's entry while a version waits to be installed: a dot, and the restart.
pub(crate) fn account_entries(ui: &Ui) -> Vec<limn::DropdownMenuEntry> {
    match &ui.update {
        UpdateState::Ready { .. } => {
            vec![
                limn::DropdownMenuItem::new(UPDATE_RESTART, "Restart to update")
                    .annotation("\u{25cf}")
                    .into(),
            ]
        }
        _ => vec![],
    }
}

/// The toast once a version is ready, and the dialog a mandatory one puts up over `base`.
pub(crate) fn dialogs(ui: &Ui, base: Element, palette: ComponentPalette) -> Element {
    let base = match (&ui.update, &ui.update_toast) {
        (UpdateState::Ready { version, .. }, Some(shown)) if version == shown => {
            let toast =
                limn::Toast::new(format!("quill {version} is ready \u{b7} Restart"), palette)
                    .id("update:toast")
                    .action(UPDATE_RESTART)
                    .dismissible(UPDATE_TOAST_DISMISS)
                    .render();
            limn::Toast::layer(base, vec![toast], 12.0)
        }
        _ => base,
    };
    let version = match &ui.update {
        UpdateState::Available {
            version,
            mandatory: true,
            ..
        }
        | UpdateState::Downloading {
            version,
            mandatory: true,
            ..
        }
        | UpdateState::Ready {
            version,
            mandatory: true,
        } => version,
        _ => return base,
    };
    let (description, body, footer): (String, Element, Option<Element>) = match &ui.update {
        UpdateState::Downloading { done, total, .. } => {
            let fraction = if *total == 0 {
                0.0
            } else {
                *done as f32 / *total as f32
            };
            (
                "quill needs this version before it opens. It is downloading now.".to_owned(),
                limn::Progress::new(fraction, palette).render(),
                None,
            )
        }
        _ => (
            "Restart quill to finish updating. It opens in the new version.".to_owned(),
            hgap(0.0),
            Some(Element::row(vec![
                Element::spacer().flex(1.0),
                keyed_button(UPDATE_RESTART, "Restart to update", "", true, true, palette),
            ])),
        ),
    };
    let modal = limn::Modal::new(
        base,
        true,
        format!("quill {version} is required"),
        description,
        UPDATE_HOLD,
        palette,
    )
    .width(440.0)
    .body(Element::column(vec![body, muted(state_line(&ui.update)).font_size(BODY)]).gap(8.0))
    .closable(false)
    .backdrop_dismiss(false)
    .scrim(look::scrim());
    match footer {
        Some(footer) => modal.footer(footer).render(),
        None => modal.render(),
    }
}
