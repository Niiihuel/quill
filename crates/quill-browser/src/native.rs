//! Chromium inside quill's own window (docs/browser.md, "Native view"): each tab is a
//! window of its own, a child of quill's, that the person uses directly, with no frames in
//! between, as Desktop's tabs are `WebContentsView`s. The engine drives it
//! over the same DevTools stream it drives a headless Chromium with: a message for a page
//! goes to that browser's own DevTools, and the few browser-wide commands it sends
//! (contexts, tabs, downloads) are answered with CEF's own. Linux on X11 only for now: on
//! Wayland CEF cannot yet be embedded in another program's window (CEF issue #2804).
//!
//! The host decides where the tab shows: `place` every frame with the rectangle the pane
//! has, or `None` while something of quill's is drawn over it, which nothing can be over a
//! native window; the page's last frame is shown in its place then, as Desktop does.

use crate::frames::{decode, Frames};
use cef::*;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a covered page stays up waiting for its last frame before it is hidden anyway,
/// as Desktop holds its pane (3.6556.2 `cf550b864`, `holdMs` 250).
const HOLD: Duration = Duration::from_millis(250);
/// DevTools ids quill's own calls use, far above the engine's, which count from 1.
const OWN_IDS: i64 = 1 << 30;
/// quill's own chords, which reach quill rather than the page that has the keys: Windows key
/// code, Ctrl, Shift. Ctrl+N, B, K, comma and backtick; Ctrl+Shift+I, M, D and B; and the
/// browser's: Ctrl+L and F6, Ctrl+T, Ctrl+W, Ctrl+R and F5, Ctrl+(Shift+)Tab.
const CHORDS: [(i32, bool, bool); 17] = [
    (0x4E, true, false),
    (0x42, true, false),
    (0x4B, true, false),
    (0xBC, true, false),
    (0xC0, true, false),
    (0x49, true, true),
    (0x4D, true, true),
    (0x44, true, true),
    (0x42, true, true),
    (0x4C, true, false),
    (0x75, false, false),
    (0x54, true, false),
    (0x57, true, false),
    (0x52, true, false),
    (0x74, false, false),
    (0x09, true, false),
    (0x09, true, true),
];

/// The program run again as one of Chromium's own processes (`--type=renderer`…): it runs
/// that process and gives its exit code. Call it first thing in `main`.
pub fn subprocess() -> Option<i32> {
    if !std::env::args().any(|arg| arg.starts_with("--type=")) {
        return None;
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = cef::args::Args::new();
    Some(execute_process(
        Some(args.as_main_args()),
        None,
        std::ptr::null_mut(),
    ))
}

/// The `Xft.dpi` the X server's resources set, the scale the person gave X11 programs.
pub fn xft_dpi() -> Option<f32> {
    let lib = x11_dl::xlib::Xlib::open().ok()?;
    unsafe {
        let display = (lib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return None;
        }
        let resources = (lib.XResourceManagerString)(display);
        let dpi = (!resources.is_null())
            .then(|| {
                std::ffi::CStr::from_ptr(resources)
                    .to_string_lossy()
                    .into_owned()
            })
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("Xft.dpi:"))
                    .and_then(|value| value.trim().parse().ok())
            });
        (lib.XCloseDisplay)(display);
        dpi
    }
}

/// Chromium running in quill's process, and what it shows in quill's window.
#[derive(Clone)]
pub struct Native {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Native {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Native")
            .field("parent", &self.shared.parent)
            .finish_non_exhaustive()
    }
}

struct Shared {
    /// quill's window, the X11 parent of every tab's.
    parent: u64,
    scale: f32,
    frames: Frames,
    x: Mutex<X11>,
    pages: Mutex<Pages>,
    view: Mutex<View>,
    /// The engine's end of the DevTools stream, while one is open.
    stream: Mutex<Option<std::sync::mpsc::Sender<Vec<u8>>>>,
    own: AtomicI64,
    tokens: AtomicU64,
    /// The browser's chords pressed while a tab had the keys, for the host to carry out.
    keys: Mutex<Vec<Key>>,
    /// A tab was pressed and has the keys; else quill does.
    keys_on_page: AtomicBool,
}

/// A chord a tab passes on to quill: its Windows key code, Ctrl, Shift.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub code: i32,
    pub ctrl: bool,
    pub shift: bool,
}

#[derive(Default)]
struct Pages {
    contexts: HashMap<String, RequestContext>,
    next_context: u32,
    /// Each tab by its target id, which is its page's main frame's.
    open: HashMap<String, Page>,
    /// Browsers made and not yet told their target id, by the token their client carries.
    made: HashMap<u64, Made>,
    /// The target id of each browser's token, once known.
    targets: HashMap<u64, String>,
    own: HashMap<i64, Own>,
    downloads: Option<PathBuf>,
    finished: HashSet<u32>,
    /// `Browser.close` was asked: the stream ends once the last tab has closed.
    closing: bool,
}

struct Page {
    browser: Browser,
    xid: u64,
    context: String,
    url: String,
    title: String,
    _observer: Option<Registration>,
}

/// A browser on its way: what to answer, or announce, once its target id is known.
struct Made {
    reply: Option<i64>,
    opener: Option<String>,
    context: String,
    url: String,
    page: Option<Page>,
}

enum Own {
    TargetInfo { token: u64 },
    Freeze { surface: String },
    Nothing,
}

/// Tells quill, through a DevTools binding, that the person pressed the page: it has the keys
/// from then on, wherever the pointer goes.
const PRESSED: &str = "if (!window.__quillPressed) { window.__quillPressed = 1; \
    addEventListener('pointerdown', () => { try { quillPressed('') } catch (e) {} }, true) }";

#[derive(Default)]
struct View {
    target: Option<String>,
    surface: Option<String>,
    placed: Option<[i32; 4]>,
    mapped: bool,
    covered_since: Option<Instant>,
    frozen: bool,
}

struct X11 {
    lib: x11_dl::xlib::Xlib,
    display: *mut x11_dl::xlib::Display,
}

// The connection is used under the lock only.
unsafe impl Send for X11 {}

impl X11 {
    fn place(&self, window: u64, [x, y, width, height]: [i32; 4]) {
        unsafe {
            (self.lib.XMoveResizeWindow)(
                self.display,
                window,
                x,
                y,
                width.max(1) as u32,
                height.max(1) as u32,
            );
            (self.lib.XMapWindow)(self.display, window);
            (self.lib.XFlush)(self.display);
        }
    }

    /// Hand quill's window a key X11 gave a page under the pointer while quill has the keys,
    /// as the X server would have: its keycode, with CEF's modifiers as X11's. winit reads it
    /// with its own keyboard state, dead keys and compose included.
    fn resend(&self, window: u64, keycode: i32, press: bool, flags: u32) {
        use x11_dl::xlib;
        let state = x11_state(flags);
        unsafe {
            let root = (self.lib.XDefaultRootWindow)(self.display);
            let mut event: xlib::XEvent = std::mem::zeroed();
            event.key = xlib::XKeyEvent {
                type_: if press {
                    xlib::KeyPress
                } else {
                    xlib::KeyRelease
                },
                serial: 0,
                send_event: xlib::True,
                display: self.display,
                window,
                root,
                subwindow: 0,
                time: xlib::CurrentTime,
                x: 0,
                y: 0,
                x_root: 0,
                y_root: 0,
                state,
                keycode: keycode as u32,
                same_screen: xlib::True,
            };
            let mask = if press {
                xlib::KeyPressMask
            } else {
                xlib::KeyReleaseMask
            };
            (self.lib.XSendEvent)(self.display, window, xlib::False, mask, &mut event);
            (self.lib.XFlush)(self.display);
        }
    }

    /// Out of sight, beyond quill's window, which clips it: Chromium maps an unmapped tab
    /// again on its own, under a pointer dragging something of quill's.
    fn hide(&self, window: u64) {
        unsafe {
            (self.lib.XMoveWindow)(self.display, window, -30000, -30000);
            (self.lib.XFlush)(self.display);
        }
    }
}

impl Native {
    /// Start Chromium for tabs inside the X11 window `parent`, drawn at `scale` device
    /// pixels per point, keeping what it must in `cache`. Call it on the main thread, once.
    pub fn start(parent: u64, scale: f32, cache: &Path, frames: Frames) -> Result<Self, String> {
        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
        let dir = resources();
        std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
        let args = cef::args::Args::new();
        let settings = Settings {
            multi_threaded_message_loop: 1,
            resources_dir_path: CefString::from(dir.to_string_lossy().as_ref()),
            locales_dir_path: CefString::from(dir.join("locales").to_string_lossy().as_ref()),
            root_cache_path: CefString::from(cache.to_string_lossy().as_ref()),
            log_file: CefString::from(cache.join("chromium.log").to_string_lossy().as_ref()),
            log_severity: LogSeverity::WARNING,
            no_sandbox: i32::from(std::env::var_os("QUILL_NO_SANDBOX").is_some()),
            ..Default::default()
        };
        let mut app = QuillApp::new(scale);
        if initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut(),
        ) != 1
        {
            return Err(format!(
                "Chromium did not start; see {}",
                cache.join("chromium.log").display()
            ));
        }
        let lib = x11_dl::xlib::Xlib::open().map_err(|e| e.to_string())?;
        let display = unsafe { (lib.XOpenDisplay)(std::ptr::null()) };
        if display.is_null() {
            return Err("no X11 display".into());
        }
        Ok(Self {
            shared: Arc::new(Shared {
                parent,
                scale,
                frames,
                x: Mutex::new(X11 { lib, display }),
                pages: Mutex::default(),
                view: Mutex::default(),
                stream: Mutex::default(),
                own: AtomicI64::new(OWN_IDS),
                tokens: AtomicU64::default(),
                keys: Mutex::default(),
                keys_on_page: AtomicBool::new(false),
            }),
        })
    }

    /// Show the visible tab at `at`, `[x, y, width, height]` in the window's pixels, or,
    /// with `None`, put its last frame in its place because something covers it: the page
    /// stays up until a fresh frame is in, or for `HOLD` at most, unless `now`, when it goes at
    /// once and the frame taken last stands in.
    /// Returns whether the page is still up waiting for its frame: the host paints again
    /// within `HOLD` then, as nothing else may wake it.
    pub fn place(&self, at: Option<[i32; 4]>, now: bool) -> bool {
        let shared = &self.shared;
        let mut view = shared.view.lock().unwrap();
        let Some(target) = view.target.clone() else {
            return false;
        };
        let Some(xid) = shared
            .pages
            .lock()
            .unwrap()
            .open
            .get(&target)
            .map(|p| p.xid)
        else {
            return false;
        };
        match at {
            Some(rect) => {
                view.covered_since = None;
                view.frozen = false;
                if !view.mapped || view.placed != Some(rect) {
                    shared.x.lock().unwrap().place(xid, rect);
                    view.mapped = true;
                    view.placed = Some(rect);
                }
                false
            }
            None if view.mapped => {
                let since = match view.covered_since {
                    Some(since) => since,
                    None => {
                        if let Some(surface) = view.surface.clone() {
                            shared.post(Job::Freeze { target, surface });
                        }
                        let now = Instant::now();
                        view.covered_since = Some(now);
                        now
                    }
                };
                if now || view.frozen || since.elapsed() >= HOLD {
                    shared.x.lock().unwrap().hide(xid);
                    view.mapped = false;
                }
                view.mapped
            }
            None => false,
        }
    }

    /// Keep the visible tab's frame at hand, for when something covers it or the panes move
    /// it: the host asks as the pointer leaves the page, which it does before anything of
    /// quill's is used, as Desktop keeps a warm frame (3.6556.2 `cf550b864` `warmFrame`).
    pub fn warm(&self) {
        let view = self.shared.view.lock().unwrap();
        if let (Some(target), Some(surface), true) =
            (view.target.clone(), view.surface.clone(), view.mapped)
        {
            self.shared.post(Job::Freeze { target, surface });
        }
    }

    /// The keys tabs passed on to quill since the last call, in order.
    pub fn take_keys(&self) -> Vec<Key> {
        std::mem::take(&mut self.shared.keys.lock().unwrap())
    }

    /// Whether a tab has the keys: it was pressed, and nothing of quill's since. Keys quill's
    /// window gets meanwhile are the page's, for the host to send it.
    pub fn keys_on_page(&self) -> bool {
        self.shared.keys_on_page.load(Ordering::SeqCst)
    }

    /// The person pressed somewhere of quill's own: the keys come back to quill, and the
    /// page's field lets go of them as a page does when its window loses the focus.
    pub fn blur(&self) {
        if self.shared.keys_on_page.swap(false, Ordering::SeqCst) {
            let target = self.shared.view.lock().unwrap().target.clone();
            if let Some(target) = target {
                self.shared.post(Job::Blur { target });
            }
        }
    }

    /// Close every tab and stop Chromium. Call it on the thread `start` ran on, after the
    /// window's loop ended.
    pub fn shutdown(&self) {
        self.shared.post(Job::CloseAll);
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && !self.shared.pages.lock().unwrap().open.is_empty() {
            std::thread::sleep(Duration::from_millis(20));
        }
        shutdown();
    }

    /// A fresh DevTools stream for the engine, as a launched Chromium's pipe would be.
    pub(crate) fn pipe(&self) -> (impl Read + Send + 'static, impl Write + Send + 'static) {
        let (to_engine, from_cef) = std::sync::mpsc::channel();
        *self.shared.stream.lock().unwrap() = Some(to_engine);
        self.shared.pages.lock().unwrap().closing = false;
        (
            Incoming {
                from: from_cef,
                left: Vec::new(),
                at: 0,
            },
            Outgoing {
                shared: self.shared.clone(),
                buffer: Vec::new(),
            },
        )
    }

    /// The tab on screen, and the surface its last frame is shown on; `None` shows none.
    pub(crate) fn show(&self, shown: Option<(String, String)>) {
        let shared = &self.shared;
        let mut view = shared.view.lock().unwrap();
        let (target, surface) = shown.unzip();
        if view.target == target {
            view.surface = surface;
            return;
        }
        if let Some(old) = view.target.take().filter(|_| view.mapped) {
            if let Some(page) = shared.pages.lock().unwrap().open.get(&old) {
                shared.x.lock().unwrap().hide(page.xid);
            }
        }
        *view = View {
            target,
            surface,
            ..View::default()
        };
    }
}

/// X11's modifier state for CEF's flags. CEF: Caps Lock 1, Shift 2, Ctrl 4, Alt 8, Num Lock
/// 256, AltGr 4096; X11: Shift 1, Lock 2, Control 4, Mod1 8, Mod2 16, Mod5 128.
fn x11_state(flags: u32) -> u32 {
    use x11_dl::xlib;
    [
        (2, xlib::ShiftMask),
        (1, xlib::LockMask),
        (4, xlib::ControlMask),
        (8, xlib::Mod1Mask),
        (256, xlib::Mod2Mask),
        (4096, xlib::Mod5Mask),
    ]
    .iter()
    .filter(|(cef, _)| flags & cef != 0)
    .fold(0, |state, (_, x)| state | x)
}

/// Where CEF's resources are: `QUILL_CEF_DIR`, else the CEF quill was built with.
fn resources() -> PathBuf {
    std::env::var_os("QUILL_CEF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("QUILL_CEF_DIR")))
}

/// Work for CEF's UI thread, where its browsers must be driven.
#[derive(Clone)]
enum Job {
    Deliver(String),
    Freeze { target: String, surface: String },
    Blur { target: String },
    CloseAll,
}

wrap_task! {
    struct Run {
        shared: Arc<Shared>,
        job: Job,
    }

    impl Task {
        fn execute(&self) {
            self.shared.run(self.job.clone());
        }
    }
}

impl Shared {
    fn post(self: &Arc<Self>, job: Job) {
        let mut task = Run::new(self.clone(), job);
        post_task(ThreadId::UI, Some(&mut task));
    }

    fn run(self: &Arc<Self>, job: Job) {
        match job {
            Job::Deliver(message) => {
                if let Ok(message) = serde_json::from_str::<Value>(&message) {
                    self.deliver(message);
                }
            }
            Job::Freeze { target, surface } => {
                self.own(
                    &target,
                    Own::Freeze { surface },
                    "Page.captureScreenshot",
                    json!({ "format": "jpeg", "quality": 85 }),
                );
            }
            // Not CEF's focus: without it the page would no longer hand quill the keys X11
            // gives it while the pointer is over it.
            Job::Blur { target } => {
                self.own(
                    &target,
                    Own::Nothing,
                    "Runtime.evaluate",
                    json!({ "expression": "document.activeElement?.blur()" }),
                );
            }
            Job::CloseAll => {
                let mut pages = self.pages.lock().unwrap();
                pages.closing = true;
                let hosts: Vec<BrowserHost> = pages
                    .open
                    .values()
                    .filter_map(|p| p.browser.host())
                    .collect();
                let empty = pages.open.is_empty();
                drop(pages);
                for host in hosts {
                    host.close_browser(1);
                }
                if empty {
                    self.stream.lock().unwrap().take();
                }
            }
        }
    }

    fn host(&self, target: &str) -> Option<BrowserHost> {
        self.pages
            .lock()
            .unwrap()
            .open
            .get(target)
            .and_then(|p| p.browser.host())
    }

    /// Send the engine one message of the stream.
    fn emit(&self, message: Value) {
        if let Some(stream) = self.stream.lock().unwrap().as_ref() {
            let mut bytes = message.to_string().into_bytes();
            bytes.push(0);
            let _ = stream.send(bytes);
        }
    }

    fn reply(&self, id: i64, result: Result<Value, String>) {
        self.emit(match result {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(message) => json!({ "id": id, "error": { "code": -32000, "message": message } }),
        });
    }

    fn event(&self, method: &str, params: Value) {
        self.emit(json!({ "method": method, "params": params }));
    }

    /// One message from the engine: for a page, to its DevTools; for the browser, answered here.
    fn deliver(self: &Arc<Self>, message: Value) {
        let id = message["id"].as_i64().unwrap_or_default();
        let method = message["method"].as_str().unwrap_or_default();
        let params = message["params"].clone();
        match message["sessionId"].as_str() {
            Some(target) => match self.host(target) {
                Some(host) => {
                    let mut call = json!({ "id": id, "method": method });
                    if params.is_object() {
                        call["params"] = params;
                    }
                    host.send_dev_tools_message(Some(call.to_string().as_bytes()));
                }
                None => self.reply(id, Err("No session with given id".into())),
            },
            None => self.browser(id, method, params),
        }
    }

    /// The browser-wide commands the engine sends, in CEF's terms.
    fn browser(self: &Arc<Self>, id: i64, method: &str, params: Value) {
        let text = |key: &str| params[key].as_str().unwrap_or_default().to_owned();
        match method {
            "Target.setDiscoverTargets" => self.reply(id, Ok(json!({}))),
            "Target.createBrowserContext" => {
                let mut pages = self.pages.lock().unwrap();
                pages.next_context += 1;
                let name = format!("context-{}", pages.next_context);
                // No cache path: cookies and storage live in memory, one session's apart.
                if let Some(context) =
                    request_context_create_context(Some(&RequestContextSettings::default()), None)
                {
                    pages.contexts.insert(name.clone(), context);
                }
                drop(pages);
                self.reply(id, Ok(json!({ "browserContextId": name })));
            }
            "Target.disposeBrowserContext" => {
                let context = text("browserContextId");
                let mut pages = self.pages.lock().unwrap();
                pages.contexts.remove(&context);
                let hosts: Vec<BrowserHost> = pages
                    .open
                    .values()
                    .filter(|p| p.context == context)
                    .filter_map(|p| p.browser.host())
                    .collect();
                drop(pages);
                for host in hosts {
                    host.close_browser(1);
                }
                self.reply(id, Ok(json!({})));
            }
            "Target.createTarget" => {
                self.make(Some(id), None, text("browserContextId"), text("url"));
            }
            "Target.attachToTarget" => {
                let target = text("targetId");
                let known = self.pages.lock().unwrap().open.contains_key(&target);
                self.reply(
                    id,
                    if known {
                        Ok(json!({ "sessionId": target }))
                    } else {
                        Err("No target with given id found".into())
                    },
                );
            }
            "Target.closeTarget" => {
                if let Some(host) = self.host(&text("targetId")) {
                    host.close_browser(1);
                }
                self.reply(id, Ok(json!({ "success": true })));
            }
            "Browser.setDownloadBehavior" => {
                let path = text("downloadPath");
                self.pages.lock().unwrap().downloads = (!path.is_empty()).then(|| path.into());
                self.reply(id, Ok(json!({})));
            }
            "Browser.close" => {
                self.reply(id, Ok(json!({})));
                self.run(Job::CloseAll);
            }
            other => self.reply(id, Err(format!("'{other}' wasn't found"))),
        }
    }

    /// Make a tab in `context` showing `url`: answered to `reply`, or announced as opened by
    /// `opener`, once its target id is known.
    fn make(
        self: &Arc<Self>,
        reply: Option<i64>,
        opener: Option<String>,
        context: String,
        url: String,
    ) {
        let token = self.tokens.fetch_add(1, Ordering::Relaxed) + 1;
        let mut request = {
            let mut pages = self.pages.lock().unwrap();
            pages.made.insert(
                token,
                Made {
                    reply,
                    opener,
                    context: context.clone(),
                    url: url.clone(),
                    page: None,
                },
            );
            pages.contexts.get(&context).cloned()
        };
        // Made a pixel in the corner and hidden once made; `place` shows it where the pane is.
        // Alloy: the page alone, without Chrome's tab and window machinery, which takes a
        // child window for a tab of its own and fails on it.
        let info = WindowInfo {
            runtime_style: RuntimeStyle::ALLOY,
            ..Default::default()
        }
        .set_as_child(
            self.parent,
            &Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
        );
        let mut client = PageClient::new(self.clone(), token);
        let url = if url.is_empty() {
            "about:blank".to_owned()
        } else {
            url
        };
        browser_host_create_browser(
            Some(&info),
            Some(&mut client),
            Some(&CefString::from(url.as_str())),
            Some(&BrowserSettings::default()),
            None,
            request.as_mut(),
        );
    }

    /// Call `method` on `target`'s page for quill's own sake.
    fn own(self: &Arc<Self>, target: &str, purpose: Own, method: &str, params: Value) {
        let Some(host) = self.host(target) else {
            return;
        };
        let id = self.own.fetch_add(1, Ordering::Relaxed);
        self.pages.lock().unwrap().own.insert(id, purpose);
        let call = json!({ "id": id, "method": method, "params": params });
        host.send_dev_tools_message(Some(call.to_string().as_bytes()));
    }

    /// A browser was made: it is hidden until shown, and asked for its target id.
    fn made(self: &Arc<Self>, token: u64, browser: Browser) {
        let Some(host) = browser.host() else {
            return;
        };
        let xid = host.window_handle();
        self.x.lock().unwrap().hide(xid);
        let mut observer = Observer::new(self.clone(), token);
        let registration = host.add_dev_tools_message_observer(Some(&mut observer));
        let mut pages = self.pages.lock().unwrap();
        let Some(made) = pages.made.get_mut(&token) else {
            return;
        };
        made.page = Some(Page {
            browser,
            xid,
            context: made.context.clone(),
            url: made.url.clone(),
            title: String::new(),
            _observer: registration,
        });
        let id = self.own.fetch_add(1, Ordering::Relaxed);
        pages.own.insert(id, Own::TargetInfo { token });
        drop(pages);
        let call = json!({ "id": id, "method": "Target.getTargetInfo" });
        host.send_dev_tools_message(Some(call.to_string().as_bytes()));
    }

    /// What a page's DevTools sent: quill's own answers are kept, the rest goes to the engine
    /// as from the page's session.
    fn heard(self: &Arc<Self>, token: u64, raw: &[u8]) {
        let Ok(mut message) = serde_json::from_slice::<Value>(raw) else {
            return;
        };
        if let Some(id) = message["id"].as_i64().filter(|id| *id >= OWN_IDS) {
            let purpose = self.pages.lock().unwrap().own.remove(&id);
            match purpose {
                Some(Own::TargetInfo { token }) => {
                    let target = message["result"]["targetInfo"]["targetId"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                    self.named(token, target);
                }
                Some(Own::Freeze { surface }) => self.frozen(surface, &message),
                Some(Own::Nothing) | None => {}
            }
            return;
        }
        if message["method"] == "Runtime.bindingCalled"
            && message["params"]["name"] == "quillPressed"
        {
            self.keys_on_page.store(true, Ordering::SeqCst);
            return;
        }
        let target = self.pages.lock().unwrap().targets.get(&token).cloned();
        if let Some(target) = target {
            message["sessionId"] = json!(target);
            self.emit(message);
        }
    }

    /// The browser of `token` is the page `target`: a tab now, answered or announced.
    fn named(self: &Arc<Self>, token: u64, target: String) {
        let mut pages = self.pages.lock().unwrap();
        let Some(mut made) = pages.made.remove(&token) else {
            return;
        };
        let Some(page) = made.page.take() else {
            return;
        };
        let (url, context) = (page.url.clone(), page.context.clone());
        pages.targets.insert(token, target.clone());
        pages.open.insert(target.clone(), page);
        drop(pages);
        // A press in the page, in this document and every next one, reaches quill.
        self.own(
            &target,
            Own::Nothing,
            "Runtime.addBinding",
            json!({ "name": "quillPressed" }),
        );
        self.own(
            &target,
            Own::Nothing,
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": PRESSED }),
        );
        self.own(
            &target,
            Own::Nothing,
            "Runtime.evaluate",
            json!({ "expression": PRESSED }),
        );
        if let Some(reply) = made.reply {
            self.reply(reply, Ok(json!({ "targetId": target })));
        }
        if let Some(opener) = made.opener {
            self.event(
                "Target.targetCreated",
                json!({ "targetInfo": { "targetId": target, "type": "page", "url": url, "title": "",
                         "attached": false, "openerId": opener, "browserContextId": context } }),
            );
        }
    }

    /// A page's last frame is in: shown in its surface, and the page may go.
    /// A page's last frame is in, or could not be taken: either way the page may go now.
    fn frozen(self: &Arc<Self>, surface: String, message: &Value) {
        use base64::Engine;
        let jpeg = message["result"]["data"]
            .as_str()
            .and_then(|data| base64::engine::general_purpose::STANDARD.decode(data).ok());
        let shared = self.clone();
        std::thread::spawn(move || {
            if let Some(frame) = jpeg.and_then(|jpeg| decode(&jpeg, shared.scale)) {
                shared.frames.publish(surface, frame);
            }
            shared.view.lock().unwrap().frozen = true;
            shared.frames.wake();
        });
    }

    fn target_of(&self, browser: &Browser) -> Option<String> {
        let id = browser.identifier();
        let pages = self.pages.lock().unwrap();
        pages
            .open
            .iter()
            .find(|(_, page)| page.browser.identifier() == id)
            .map(|(target, _)| target.clone())
    }

    /// A tab's address or title changed, as Chromium reports a target's.
    fn changed(&self, browser: &Browser, url: Option<String>, title: Option<String>) {
        let Some(target) = self.target_of(browser) else {
            return;
        };
        let mut pages = self.pages.lock().unwrap();
        let Some(page) = pages.open.get_mut(&target) else {
            return;
        };
        if let Some(url) = url {
            page.url = url;
        }
        if let Some(title) = title {
            page.title = title;
        }
        let info = json!({ "targetInfo": { "targetId": target, "type": "page", "url": page.url,
                           "title": page.title, "attached": true, "browserContextId": page.context } });
        drop(pages);
        self.event("Target.targetInfoChanged", info);
    }

    fn closed(&self, browser: &Browser) {
        let Some(target) = self.target_of(browser) else {
            return;
        };
        let mut pages = self.pages.lock().unwrap();
        pages.open.remove(&target);
        pages.targets.retain(|_, t| *t != target);
        let last = pages.closing && pages.open.is_empty();
        drop(pages);
        {
            let mut view = self.view.lock().unwrap();
            if view.target.as_deref() == Some(target.as_str()) {
                *view = View::default();
            }
        }
        self.event(
            "Target.detachedFromTarget",
            json!({ "sessionId": target, "targetId": target }),
        );
        self.event("Target.targetDestroyed", json!({ "targetId": target }));
        if last {
            self.stream.lock().unwrap().take();
        }
    }
}

/// The engine's side of the stream: what CEF answers and reports, NUL-delimited.
struct Incoming {
    from: std::sync::mpsc::Receiver<Vec<u8>>,
    left: Vec<u8>,
    at: usize,
}

impl Read for Incoming {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.at >= self.left.len() {
            match self.from.recv() {
                Ok(next) => {
                    self.left = next;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.left.len() - self.at);
        out[..n].copy_from_slice(&self.left[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// What the engine writes, a message at a NUL, carried to CEF's UI thread.
struct Outgoing {
    shared: Arc<Shared>,
    buffer: Vec<u8>,
}

impl Write for Outgoing {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(data);
        while let Some(end) = self.buffer.iter().position(|b| *b == 0) {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            if let Ok(message) = std::str::from_utf8(&line[..end]) {
                self.shared.post(Job::Deliver(message.to_owned()));
            }
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

wrap_app! {
    struct QuillApp {
        scale: f32,
    }

    impl App {
        fn on_before_command_line_processing(&self, kind: Option<&CefString>, line: Option<&mut CommandLine>) {
            // The browser process's switches reach the others.
            let (None, Some(line)) = (kind.map(|k| k.to_string()).filter(|k| !k.is_empty()), line) else {
                return;
            };
            let switch = |name: &str, value: &str| {
                line.append_switch_with_value(Some(&CefString::from(name)), Some(&CefString::from(value)));
            };
            // Tabs are children of quill's X11 window, even in a Wayland session (XWayland).
            switch("ozone-platform", "x11");
            switch("force-device-scale-factor", &self.scale.to_string());
            // As the headless Chromium runs: no keyring prompt, no background chatter.
            switch("password-store", "basic");
            switch("disable-features", "Translate,MediaRouter,OptimizationHints");
        }
    }
}

wrap_client! {
    struct PageClient {
        shared: Arc<Shared>,
        token: u64,
    }

    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(Life::new(self.shared.clone(), self.token))
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(Display::new(self.shared.clone()))
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(Downloads::new(self.shared.clone()))
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(Keys::new(self.shared.clone()))
        }
    }
}

wrap_keyboard_handler! {
    struct Keys {
        shared: Arc<Shared>,
    }

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut cef::sys::XEvent>,
            _is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let Some(event) = event else {
                return 0;
            };
            let down = event.type_ == KeyEventType::RAWKEYDOWN || event.type_ == KeyEventType::KEYDOWN;
            // X11 hands keys to the window under the pointer while the focus is on one of its
            // ancestors, and quill's window keeps the focus: a page under the pointer takes
            // what is typed into quill. Keys never pass above the focused window, so the page
            // hands them back, to the window they were typed in.
            if !self.shared.keys_on_page.load(Ordering::SeqCst) {
                if down || event.type_ == KeyEventType::KEYUP {
                    let x = self.shared.x.lock().unwrap();
                    x.resend(self.shared.parent, event.native_key_code, down, event.modifiers);
                }
                return 1;
            }
            // CEF's flags: Shift 1 << 1, Ctrl 1 << 2.
            let key = Key { code: event.windows_key_code, ctrl: event.modifiers & 4 != 0, shift: event.modifiers & 2 != 0 };
            if !down || !CHORDS.contains(&(key.code, key.ctrl, key.shift)) {
                return 0;
            }
            self.shared.keys.lock().unwrap().push(key);
            self.shared.frames.wake();
            1
        }
    }
}

wrap_life_span_handler! {
    struct Life {
        shared: Arc<Shared>,
        token: u64,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                self.shared.made(self.token, browser.clone());
            }
        }

        // A page opening another opens a tab of the same session, as Chromium's targets do.
        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let opener = browser.and_then(|b| self.shared.target_of(b));
            if let Some(opener) = opener {
                let context = self.shared.pages.lock().unwrap().open.get(&opener).map(|p| p.context.clone());
                let url = target_url.map(|u| u.to_string()).unwrap_or_default();
                if let Some(context) = context {
                    self.shared.make(None, Some(opener), context, url);
                }
            }
            1
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                self.shared.closed(browser);
            }
        }
    }
}

wrap_display_handler! {
    struct Display {
        shared: Arc<Shared>,
    }

    impl DisplayHandler {
        fn on_address_change(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if let (Some(browser), Some(frame)) = (browser, frame) {
                if frame.is_main() != 0 {
                    self.shared.changed(browser, url.map(|u| u.to_string()), None);
                }
            }
        }

        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            if let Some(browser) = browser {
                self.shared.changed(browser, None, title.map(|t| t.to_string()));
            }
        }
    }
}

wrap_download_handler! {
    struct Downloads {
        shared: Arc<Shared>,
    }

    impl DownloadHandler {
        fn on_before_download(
            &self,
            browser: Option<&mut Browser>,
            item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(browser), Some(item), Some(callback)) = (browser, item, callback) else {
                return 0;
            };
            let name = suggested_name.map(|n| n.to_string()).unwrap_or_else(|| "download".into());
            let folder = self.shared.pages.lock().unwrap().downloads.clone();
            let Some(folder) = folder else {
                return 0;
            };
            let path = folder.join(&name);
            callback.cont(Some(&CefString::from(path.to_string_lossy().as_ref())), 0);
            let frame = self.shared.target_of(browser).unwrap_or_default();
            let url = CefString::from(&item.url()).to_string();
            self.shared.event(
                "Browser.downloadWillBegin",
                json!({ "frameId": frame, "guid": item.id().to_string(), "url": url, "suggestedFilename": name }),
            );
            1
        }

        fn on_download_updated(&self, _browser: Option<&mut Browser>, item: Option<&mut DownloadItem>, _callback: Option<&mut DownloadItemCallback>) {
            let Some(item) = item else {
                return;
            };
            let state = if item.is_complete() != 0 {
                "completed"
            } else if item.is_canceled() != 0 || item.is_interrupted() != 0 {
                "canceled"
            } else {
                return;
            };
            if !self.shared.pages.lock().unwrap().finished.insert(item.id()) {
                return;
            }
            self.shared.event(
                "Browser.downloadProgress",
                json!({ "guid": item.id().to_string(), "state": state,
                        "totalBytes": item.total_bytes(), "receivedBytes": item.received_bytes() }),
            );
        }
    }
}

wrap_dev_tools_message_observer! {
    struct Observer {
        shared: Arc<Shared>,
        token: u64,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_message(&self, _browser: Option<&mut Browser>, message: Option<&[u8]>) -> ::std::os::raw::c_int {
            if let Some(message) = message {
                self.shared.heard(self.token, message);
            }
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_keeps_its_modifiers_as_x11_names_them() {
        use x11_dl::xlib;
        assert_eq!(x11_state(2 | 4), xlib::ShiftMask | xlib::ControlMask);
        assert_eq!(x11_state(4096 | 256), xlib::Mod5Mask | xlib::Mod2Mask);
        assert_eq!(x11_state(0), 0);
    }

    #[test]
    fn the_stream_reads_what_cef_sent_in_order_and_ends_with_it() {
        let (to_engine, from_cef) = std::sync::mpsc::channel();
        let mut incoming = Incoming {
            from: from_cef,
            left: Vec::new(),
            at: 0,
        };
        to_engine.send(b"{\"id\":1}\0".to_vec()).unwrap();
        to_engine.send(b"{\"id\":2}\0".to_vec()).unwrap();
        drop(to_engine);
        let mut read = Vec::new();
        let mut chunk = [0; 3];
        loop {
            match incoming.read(&mut chunk).unwrap() {
                0 => break,
                n => read.extend_from_slice(&chunk[..n]),
            }
        }
        assert_eq!(read, b"{\"id\":1}\0{\"id\":2}\0");
    }
}
