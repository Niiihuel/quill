//! quill's own icons: line drawings on a 20 × 20 grid with a 1.5 stroke, round caps and
//! joins and no fill, in the quiet style of Claude Desktop's icon font (private notes
//! `rea/features/iconography.md`,). They are drawn for quill, not
//! taken from that font, which is Anthropic's own. Each is an SVG in `currentColor`, which
//! limn tints like its own icons through `Icon::Custom`.

macro_rules! icon {
    ($body:expr) => {
        concat!(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20' fill='none' ",
            "stroke='currentColor' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'>",
            $body,
            "</svg>"
        )
    };
}

/// A drawing as limn takes it.
pub fn of(svg: &'static str) -> limn::Icon {
    limn::Icon::Custom(svg)
}

// Actions.
pub const ADD: &str = icon!("<path d='M10 4.5v11M4.5 10h11'/>");
pub const CLOSE: &str = icon!("<path d='M5.5 5.5l9 9M14.5 5.5l-9 9'/>");
pub const CHECK: &str = icon!("<path d='M4.5 10.5l3.5 3.5 7.5-8'/>");
pub const CHECK_CIRCLE: &str =
    icon!("<circle cx='10' cy='10' r='6.75'/><path d='M7.25 10.25l2 2 3.75-4.25'/>");
pub const COPY: &str = icon!(
    "<rect x='7' y='7' width='9' height='9' rx='1.75'/><path d='M13 4.75v-.25A1.5 1.5 0 0 0 11.5 3h-7A1.5 1.5 0 0 0 3 4.5v7A1.5 1.5 0 0 0 4.5 13h.25'/>"
);
pub const SEARCH: &str = icon!("<circle cx='8.75' cy='8.75' r='5'/><path d='M12.5 12.5l4 4'/>");
pub const FILTER: &str = icon!("<path d='M3.5 6h13M6 10h8M8.5 14h3'/>");
pub const SHARE: &str = icon!("<path d='M10 3.5v9M6.5 7L10 3.5 13.5 7M5 10.5v4.75A1.25 1.25 0 0 0 6.25 16.5h7.5A1.25 1.25 0 0 0 15 15.25V10.5'/>");
pub const PIN: &str = icon!("<path d='M8 3.5h4M9 3.5v4L6.5 10.5h7L11 7.5v-4M10 10.5v6'/>");
/// Angle brackets: a file of code.
pub const CODE: &str = icon!("<path d='M7.5 6L3.5 10l4 4M12.5 6l4 4-4 4'/>");
/// A path that parts in two going right: a new session from a point of this one.
pub const FORK: &str = icon!(
    "<path d='M3.5 10h3c2.5 0 3.5-4.5 6.5-4.5h3M6.5 10c2.5 0 3.5 4.5 6.5 4.5h3M14.25 3.25l2.25 2.25-2.25 2.25M14.25 12.25l2.25 2.25-2.25 2.25'/>"
);
/// A speaker and its sound: read aloud.
pub const SPEAKER: &str = icon!(
    "<path d='M3.5 8h2.75L10 5v10l-3.75-3H3.5zM13 7.75a3.25 3.25 0 0 1 0 4.5M15.25 5.5a6.25 6.25 0 0 1 0 9'/>"
);
pub const STOP: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20'><rect x='2.5' y='2.5' width='15' height='15' rx='3.75' fill='currentColor'/></svg>";
pub const STOP_CIRCLE: &str = icon!(
    "<circle cx='10' cy='10' r='6.75'/><rect x='7.75' y='7.75' width='4.5' height='4.5' rx='0.75' fill='currentColor' stroke='none'/>"
);
pub const DOTS_HORIZONTAL: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20' fill='currentColor'><circle cx='4.75' cy='10' r='1.25'/><circle cx='10' cy='10' r='1.25'/><circle cx='15.25' cy='10' r='1.25'/></svg>";
pub const DOTS_VERTICAL: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20' fill='currentColor'><circle cx='10' cy='4.75' r='1.25'/><circle cx='10' cy='10' r='1.25'/><circle cx='10' cy='15.25' r='1.25'/></svg>";

// Arrows and carets.
pub const ARROW_LEFT: &str = icon!("<path d='M16 10H4M8.5 5.5L4 10l4.5 4.5'/>");
pub const ARROW_RIGHT: &str = icon!("<path d='M4 10h12M11.5 5.5L16 10l-4.5 4.5'/>");
pub const ARROW_UP: &str = icon!("<path d='M10 16V4M5.5 8.5L10 4l4.5 4.5'/>");
pub const ARROW_DOWN: &str = icon!("<path d='M10 4v12M5.5 11.5L10 16l4.5-4.5'/>");
/// Return: what sends a message.
pub const RETURN: &str =
    icon!("<path d='M15.5 5v4.25A1.75 1.75 0 0 1 13.75 11H4.5M8 7.5L4.5 11 8 14.5'/>");
pub const ARROW_OUT: &str = icon!(
    "<path d='M11.5 4h4.5v4.5M16 4l-6.5 6.5M14 11.5v3.25A1.25 1.25 0 0 1 12.75 16h-7.5A1.25 1.25 0 0 1 4 14.75v-7.5A1.25 1.25 0 0 1 5.25 6H8.5'/>"
);
pub const CARET_RIGHT: &str = icon!("<path d='M8.25 6l4 4-4 4'/>");
pub const CARET_LEFT: &str = icon!("<path d='M11.75 6l-4 4 4 4'/>");
pub const CARET_DOWN: &str = icon!("<path d='M6 8.25l4 4 4-4'/>");
pub const CARET_UP: &str = icon!("<path d='M6 11.75l4-4 4 4'/>");
pub const EXPAND: &str = icon!("<path d='M12 4h4v4M16 4l-5 5M8 16H4v-4M4 16l5-5'/>");
pub const COLLAPSE: &str =
    icon!("<path d='M15.5 8.5h-4v-4M11.5 8.5l5-5M4.5 11.5h4v4M8.5 11.5l-5 5'/>");

// Places and things.
pub const FOLDER: &str = icon!(
    "<path d='M3 6.25A1.75 1.75 0 0 1 4.75 4.5h3.1l1.75 2h5.65A1.75 1.75 0 0 1 17 8.25v6A1.75 1.75 0 0 1 15.25 16H4.75A1.75 1.75 0 0 1 3 14.25z'/>"
);
pub const LAPTOP: &str =
    icon!("<rect x='4.25' y='4.75' width='11.5' height='8' rx='1.25'/><path d='M2.75 15.5h14.5'/>");
pub const CLOUD: &str = icon!(
    "<path d='M6 15.5a3.25 3.25 0 0 1-.35-6.48A4.5 4.5 0 0 1 14.4 8a3.75 3.75 0 0 1-.4 7.5z'/>"
);
pub const GLOBE: &str = icon!(
    "<circle cx='10' cy='10' r='6.75'/><path d='M3.25 10h13.5M10 3.25c1.9 1.9 2.75 4.1 2.75 6.75S11.9 14.85 10 16.75C8.1 14.85 7.25 12.65 7.25 10S8.1 5.15 10 3.25z'/>"
);
pub const EYE: &str = icon!(
    "<path d='M2.75 10S5.5 4.75 10 4.75 17.25 10 17.25 10 14.5 15.25 10 15.25 2.75 10 2.75 10z'/><circle cx='10' cy='10' r='2.25'/>"
);
pub const MICROPHONE: &str =
    icon!("<rect x='7.5' y='3' width='5' height='9' rx='2.5'/><path d='M5 9.75a5 5 0 0 0 10 0M10 14.75V17'/>");
pub const PAPERCLIP: &str = icon!(
    "<path d='M15.5 9.5l-5.6 5.6a3.25 3.25 0 0 1-4.6-4.6l6-6a2.25 2.25 0 0 1 3.2 3.2l-6 6a1.25 1.25 0 0 1-1.8-1.8l5.3-5.3'/>"
);
pub const PLUG: &str =
    icon!("<path d='M7.5 3.5v3M12.5 3.5v3M5.5 6.5h9v2.25a4.5 4.5 0 0 1-9 0zM10 13.25v3.25'/>");
pub const CONNECTORS: &str = icon!(
    "<rect x='3.5' y='3.5' width='5' height='5' rx='1.25'/><rect x='11.5' y='3.5' width='5' height='5' rx='1.25'/><rect x='3.5' y='11.5' width='5' height='5' rx='1.25'/><path d='M14 11.5v5M11.5 14h5'/>"
);
pub const CURSOR: &str = icon!("<path d='M5 3.75l10 5.5-4.25 1.25-2 4.25z'/>");
pub const SETTINGS: &str = icon!(
    "<circle cx='10' cy='10' r='2.25'/><path d='M10 2.75v2M10 15.25v2M2.75 10h2M15.25 10h2M4.9 4.9l1.4 1.4M13.7 13.7l1.4 1.4M4.9 15.1l1.4-1.4M13.7 6.3l1.4-1.4'/>"
);
pub const LIST: &str = icon!(
    "<path d='M8 6h8M8 10h8M8 14h8'/><circle cx='4.5' cy='6' r='0.6' fill='currentColor'/><circle cx='4.5' cy='10' r='0.6' fill='currentColor'/><circle cx='4.5' cy='14' r='0.6' fill='currentColor'/>"
);
pub const MENU: &str = icon!("<path d='M3.5 6h13M3.5 10h13M3.5 14h8'/>");

/// Settings' theme: the system's, light, dark.
pub const MONITOR: &str = icon!(
    "<rect x='3.5' y='4' width='13' height='9' rx='1.5'/><path d='M10 13v2.75M7.25 15.75h5.5'/>"
);
pub const SUN: &str = icon!(
    "<circle cx='10' cy='10' r='2.75'/><path d='M10 3.5v1.5M10 15v1.5M3.5 10H5M15 10h1.5M5.4 5.4l1.05 1.05M13.55 13.55l1.05 1.05M5.4 14.6l1.05-1.05M13.55 6.45l1.05-1.05'/>"
);
pub const MOON: &str = icon!("<path d='M15.5 12A6 6 0 0 1 8 4.5a6.25 6.25 0 1 0 7.5 7.5z'/>");

// Code.
pub const TERMINAL: &str =
    icon!("<rect x='3' y='4' width='14' height='12' rx='2'/><path d='M6.5 8.25l2 1.75-2 1.75M10.5 12.25h3'/>");
pub const DIFF: &str = icon!(
    "<rect x='3.5' y='3.5' width='13' height='13' rx='2.25'/><path d='M10 6.25v5M7.5 8.75h5M7.5 13.5h5'/>"
);
pub const GIT_BRANCH: &str = icon!(
    "<circle cx='6' cy='5' r='1.5'/><circle cx='6' cy='15' r='1.5'/><circle cx='14' cy='6' r='1.5'/><path d='M6 6.5v7M14 7.5c0 3.25-8 2.5-8 6'/>"
);
pub const SLASH: &str = icon!(
    "<rect x='3.5' y='3.5' width='13' height='13' rx='2.25'/><path d='M11.75 6.75l-3.5 6.5'/>"
);
pub const SIDEBAR: &str =
    icon!("<rect x='3' y='4' width='14' height='12' rx='2'/><path d='M8 4v12'/>");
pub const PANEL_RIGHT: &str =
    icon!("<rect x='3' y='4' width='14' height='12' rx='2'/><path d='M12 4v12'/>");

pub const QUESTION: &str = icon!(
    "<circle cx='10' cy='10' r='6.75'/><path d='M8.1 8.1a2 2 0 0 1 3.85.65c0 1.35-1.95 1.75-1.95 2.75'/><circle cx='10' cy='13.6' r='0.55' fill='currentColor'/>"
);

/// A session that needs the person: a triangle with a mark [the person's Desktop, 2026-10-25
/// videos].
pub const WARNING: &str = icon!(
    "<path d='M10 3.75L17 16H3z' stroke-linejoin='round'/><path d='M10 8.5v3.5'/><circle cx='10' cy='14' r='0.55' fill='currentColor'/>"
);

/// A waiting task's dashed ring.
pub const TASK_PENDING: &str =
    icon!("<circle cx='10' cy='10' r='6.5' stroke-dasharray='2.2 2.1'/>");

/// The greeting's mark: an eight-ray spark, quill's own.
pub const SPARK: &str = icon!(
    "<path d='M10 2.5v15M2.5 10h15M4.7 4.7l10.6 10.6M15.3 4.7L4.7 15.3' stroke-width='1.6'/>"
);

/// Every icon by name, for the preview sheet and its test.
pub const ALL: &[(&str, &str)] = &[
    ("add", ADD),
    ("close", CLOSE),
    ("check", CHECK),
    ("check_circle", CHECK_CIRCLE),
    ("copy", COPY),
    ("search", SEARCH),
    ("filter", FILTER),
    ("share", SHARE),
    ("pin", PIN),
    ("code", CODE),
    ("fork", FORK),
    ("speaker", SPEAKER),
    ("monitor", MONITOR),
    ("sun", SUN),
    ("moon", MOON),
    ("stop", STOP),
    ("stop_circle", STOP_CIRCLE),
    ("dots_horizontal", DOTS_HORIZONTAL),
    ("dots_vertical", DOTS_VERTICAL),
    ("arrow_left", ARROW_LEFT),
    ("arrow_right", ARROW_RIGHT),
    ("arrow_up", ARROW_UP),
    ("arrow_down", ARROW_DOWN),
    ("return", RETURN),
    ("arrow_out", ARROW_OUT),
    ("caret_right", CARET_RIGHT),
    ("caret_left", CARET_LEFT),
    ("caret_down", CARET_DOWN),
    ("caret_up", CARET_UP),
    ("expand", EXPAND),
    ("collapse", COLLAPSE),
    ("folder", FOLDER),
    ("laptop", LAPTOP),
    ("cloud", CLOUD),
    ("globe", GLOBE),
    ("eye", EYE),
    ("microphone", MICROPHONE),
    ("paperclip", PAPERCLIP),
    ("plug", PLUG),
    ("connectors", CONNECTORS),
    ("cursor", CURSOR),
    ("settings", SETTINGS),
    ("list", LIST),
    ("menu", MENU),
    ("terminal", TERMINAL),
    ("diff", DIFF),
    ("git_branch", GIT_BRANCH),
    ("slash", SLASH),
    ("sidebar", SIDEBAR),
    ("panel_right", PANEL_RIGHT),
    ("question", QUESTION),
    ("task_pending", TASK_PENDING),
    ("warning", WARNING),
    ("spark", SPARK),
];
