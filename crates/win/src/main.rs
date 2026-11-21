//! NexType for Windows: follows the focused text field through UI Automation, asks the engine
//! for a continuation, shows it as ghost text at the caret, and types it on Tab / key above Tab.
//! Mirrors `crates/app/src/lib.rs` (the macOS runtime); the platform-free parts come from
//! `nextype-core` and `nextype-engine`.
//!
//! Usage: `nextype [model.gguf]` (default: %LOCALAPPDATA%\NexType\models\gemma-4-E2B-it-Q4_0.gguf)

#[cfg(not(windows))]
fn main() {
    eprintln!("nextype-win runs on Windows only; the macOS app lives in shell/ and crates/app.");
}

#[cfg(windows)]
mod clip;
#[cfg(windows)]
mod keys;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod uia;
#[cfg(windows)]
mod worker;

#[cfg(windows)]
pub use app::{WM_ENGINE_TEXT, WM_KEY};

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(windows)]
mod app {
    use std::cell::{OnceCell, RefCell};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, Sender};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use nextype_core::prompt::Context;
    use nextype_core::{FieldSnapshot, OFF_BY_DEFAULT, Rect, gate, next_word, prompt_title};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Globalization::GetUserDefaultLocaleName;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
    use windows::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW};
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::w;

    use crate::keys::{self, GHOST_VISIBLE, Key, TAB_ENABLED};
    use crate::overlay::Overlay;
    use crate::uia::Uia;
    use crate::worker::{self, LATEST, Request};

    pub const WM_KEY: u32 = WM_APP + 1;
    pub const WM_ENGINE_TEXT: u32 = WM_APP + 2;
    const WM_TRAY: u32 = WM_APP + 3;

    const TIMER_READ: usize = 1;
    const TIMER_POLL: usize = 2;
    const TIMER_CONFIRM: usize = 3;
    /// Wait after a key before reading the field: the app must handle the key first.
    const DEBOUNCE_MS: u32 = 40;
    /// Re-read the field this often (caret moves by mouse, focus changes, app switches).
    const POLL_MS: u32 = 250;
    /// Spec §12.2 `insertionConfirmationTTL`.
    const CONFIRM_MS: u32 = 1000;
    /// How long after an accept a read may still show the field without it (paste in flight).
    const ACCEPT_LAG: Duration = Duration::from_millis(800);

    const MENU_ENABLED: usize = 1;
    const MENU_QUIT: usize = 2;
    const MENU_SPACES: usize = 3;

    /// `%LOCALAPPDATA%\NexType\settings.txt`, one `key=value` per line.
    fn settings_path() -> std::path::PathBuf {
        let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
        std::path::Path::new(&base).join("NexType").join("settings.txt")
    }

    fn load_settings() {
        let Ok(s) = std::fs::read_to_string(settings_path()) else { return };
        for (k, v) in s.lines().filter_map(|l| l.split_once('=')) {
            if k.trim() == "type_spaces_separately" {
                keys::SPLIT_SPACES.store(v.trim() == "true", Ordering::Relaxed);
            }
        }
    }

    fn save_settings() {
        let path = settings_path();
        let body = format!("type_spaces_separately={}\n", keys::SPLIT_SPACES.load(Ordering::Relaxed));
        let ok = path.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&path, body).is_ok();
        if !ok {
            eprintln!("could not save settings to {}", path.display());
        }
    }

    static ENABLED: AtomicBool = AtomicBool::new(true);

    struct Ghost {
        id: u64,
        pid: i32,
        anchor: String,
        full: String,
        typed: usize,
        caret: Rect,
        font: Option<(String, f64)>,
        /// Text edges to wrap between (`FieldSnapshot::wrap_edges`).
        edges: Option<(f64, f64)>,
        /// Field's right edge: single-line ghosts are cut here.
        right: f64,
        /// Bytes of `rest()` the overlay drew; accepting never takes more than this.
        visible: usize,
        /// Last accept: until the field shows it, a read that lags behind it is ignored.
        accepted_at: Option<Instant>,
    }

    impl Ghost {
        /// The suggestion still to type, drawn or not.
        fn rest(&self) -> &str {
            &self.full[self.typed..]
        }

        /// The part of `rest()` the user can see.
        fn shown(&self) -> &str {
            let rest = self.rest();
            &rest[..self.visible.min(rest.len())]
        }
    }

    thread_local! {
        static UIA: OnceCell<Uia> = const { OnceCell::new() };
        static OVERLAY: OnceCell<Overlay> = const { OnceCell::new() };
        static ENGINE: OnceCell<(Sender<Request>, Receiver<(u64, String)>)> = const { OnceCell::new() };
        static HWND_MAIN: OnceCell<HWND> = const { OnceCell::new() };
        static GHOST: RefCell<Option<Ghost>> = const { RefCell::new(None) };
        static FRONT_PID: RefCell<Option<i32>> = const { RefCell::new(None) };
        static LAST_LOG: RefCell<String> = const { RefCell::new(String::new()) };
        /// (pid, text the field should start with, text we typed) awaiting confirmation.
        static CONFIRM: RefCell<Option<(i32, String, String)>> = const { RefCell::new(None) };
    }

    fn default_model() -> String {
        let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
        format!(r"{base}\NexType\models\gemma-4-E2B-it-Q4_0.gguf")
    }

    pub fn run() {
        let model = std::env::args().nth(1).unwrap_or_else(default_model);
        if !std::path::Path::new(&model).is_file() {
            eprintln!("NexType: model not found at {model}\nDownload it (see crates/win/README.md) or pass its path.");
            std::process::exit(1);
        }
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().expect("COM");
        }
        load_settings();
        let tid = unsafe { GetCurrentThreadId() };
        UIA.with(|u| u.set(Uia::new().expect("UI Automation")).ok());
        OVERLAY.with(|o| o.set(Overlay::new().expect("overlay window")).ok());
        ENGINE.with(|e| e.set(worker::spawn(model, tid)).ok());
        let hwnd = main_window().expect("main window");
        HWND_MAIN.with(|h| h.set(hwnd).ok());
        add_tray(hwnd);
        if !keys::install_hook(tid) {
            eprintln!("NexType: could not install the keyboard hook; accept keys will not work.");
        }
        unsafe { SetTimer(Some(hwnd), TIMER_POLL, POLL_MS, None) };
        eprintln!("NexType: watching focused text fields. Tray icon → Quit to exit.");

        let mut msg = MSG::default();
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
            if msg.hwnd.is_invalid() {
                match msg.message {
                    WM_KEY => on_key(Key::from_wparam(msg.wParam.0)),
                    WM_ENGINE_TEXT => drain_engine(),
                    _ => {}
                }
                continue;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        remove_tray(hwnd);
    }

    fn main_window() -> windows::core::Result<HWND> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            let class = w!("NexTypeMain");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("NexType"), WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None)
        }
    }

    fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = tray_icon(ENABLED.load(Ordering::Relaxed));
        let tip = if ENABLED.load(Ordering::Relaxed) { "NexType" } else { "NexType (paused)" };
        for (d, s) in nid.szTip.iter_mut().zip(tip.encode_utf16()) {
            *d = s;
        }
        nid
    }

    /// Icon resource 1 (on) or 2 (paused) from `assets/nextype.rc`, at the small-icon size
    /// for the current DPI; the system's generic icon if the resource is missing.
    fn tray_icon(on: bool) -> HICON {
        unsafe {
            let id = if on { 1 } else { 2 };
            let (cx, cy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
            GetModuleHandleW(None)
                .ok()
                .and_then(|m| LoadImageW(Some(m.into()), windows::core::PCWSTR(id as *const u16), IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR).ok())
                .map(|h| HICON(h.0))
                .or_else(|| LoadIconW(None, IDI_APPLICATION).ok())
                .unwrap_or_default()
        }
    }

    fn update_tray(hwnd: HWND) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &tray_data(hwnd));
        }
    }

    fn add_tray(hwnd: HWND) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &tray_data(hwnd));
        }
    }

    fn remove_tray(hwnd: HWND) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &tray_data(hwnd));
        }
    }

    fn tray_menu(hwnd: HWND) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let on = ENABLED.load(Ordering::Relaxed);
            let _ = AppendMenuW(menu, if on { MF_CHECKED } else { MF_UNCHECKED }, MENU_ENABLED, w!("Completions on"));
            let split = keys::SPLIT_SPACES.load(Ordering::Relaxed);
            let _ = AppendMenuW(
                menu,
                if split { MF_CHECKED } else { MF_UNCHECKED },
                MENU_SPACES,
                w!("Type spaces separately (for apps that drop pasted spaces)"),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("Quit NexType"));
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            // Needed so the menu closes when the user clicks elsewhere.
            let _ = SetForegroundWindow(hwnd);
            let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, p.x, p.y, None, hwnd, None);
            let _ = DestroyMenu(menu);
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_TIMER => {
                match w.0 {
                    TIMER_READ => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_READ);
                        }
                        on_read();
                    }
                    TIMER_POLL => on_poll(),
                    TIMER_CONFIRM => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_CONFIRM);
                        }
                        confirm_insert();
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_TRAY => {
                let ev = (l.0 & 0xFFFF) as u32;
                if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
                    tray_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                match w.0 & 0xFFFF {
                    MENU_ENABLED => {
                        let on = !ENABLED.load(Ordering::Relaxed);
                        ENABLED.store(on, Ordering::Relaxed);
                        if !on {
                            clear_ghost();
                        }
                        update_tray(hwnd);
                        eprintln!("completions {}", if on { "on" } else { "off" });
                    }
                    MENU_SPACES => {
                        let on = !keys::SPLIT_SPACES.load(Ordering::Relaxed);
                        keys::SPLIT_SPACES.store(on, Ordering::Relaxed);
                        save_settings();
                        eprintln!("type spaces separately {}", if on { "on" } else { "off" });
                    }
                    MENU_QUIT => unsafe { PostQuitMessage(0) },
                    _ => {}
                }
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
        }
    }

    fn set_timer(id: usize, ms: u32) {
        HWND_MAIN.with(|h| {
            if let Some(h) = h.get() {
                unsafe { SetTimer(Some(*h), id, ms, None) };
            }
        });
    }

    fn schedule_read() {
        set_timer(TIMER_READ, DEBOUNCE_MS);
    }

    fn on_poll() {
        let pid = UIA.with(|u| u.get().and_then(Uia::foreground_pid));
        let changed = FRONT_PID.with(|f| {
            let mut f = f.borrow_mut();
            let changed = *f != pid;
            *f = pid;
            changed
        });
        if changed {
            clear_ghost();
            if let Some(pid) = pid {
                eprintln!("foreground app pid {pid}");
            }
        }
        on_read();
    }

    fn snapshot() -> Option<FieldSnapshot> {
        UIA.with(|u| u.get().and_then(Uia::snapshot))
    }

    fn on_read() {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        let snap = snapshot();
        log_snapshot(snap.as_ref());
        let Some(s) = snap.filter(|s| allowed(s).is_ok()) else {
            clear_ghost();
            return;
        };
        TAB_ENABLED.store(true, Ordering::Relaxed);
        let (before, caret) = (s.before(), s.caret_rect.unwrap_or_default());
        let kept = GHOST.with(|g| {
            let mut g = g.borrow_mut();
            let Some(gh) = g.as_mut().filter(|gh| gh.pid == s.pid) else { return false };
            let Some(t) = before.strip_prefix(gh.anchor.as_str()) else { return false };
            // Infer-from-ghost (spec §9.6): typing that matches the ghost only advances it.
            if !(gh.full.starts_with(t) && (t.len() < gh.full.len() || t.is_empty())) {
                return false;
            }
            // A paste still on its way: the field hasn't caught up with the accept yet. Keep the
            // ghost where the accept put it (hidden) instead of offering the same word again.
            if t.len() < gh.typed && gh.accepted_at.is_some_and(|a| a.elapsed() < ACCEPT_LAG) {
                return true;
            }
            gh.typed = t.len();
            (gh.caret, gh.font, gh.edges, gh.right) = (caret, s.font.clone(), s.wrap_edges(), s.frame.x + s.frame.w);
            show(gh);
            true
        });
        if !kept {
            request(&s, caret);
        }
    }

    fn request(s: &FieldSnapshot, caret: Rect) {
        let id = LATEST.fetch_add(1, Ordering::Relaxed) + 1;
        let before = s.before().to_owned();
        GHOST.with(|g| {
            let mut gh = Ghost {
                id,
                pid: s.pid,
                anchor: before.clone(),
                full: String::new(),
                typed: 0,
                caret,
                font: s.font.clone(),
                edges: s.wrap_edges(),
                right: s.frame.x + s.frame.w,
                visible: 0,
                accepted_at: None,
            };
            show(&mut gh);
            *g.borrow_mut() = Some(gh);
        });
        let context = gather_context(s);
        ENGINE.with(|e| e.get().map(|(tx, _)| tx.send(Request { id, before, context, instructions: String::new() })));
    }

    fn gather_context(s: &FieldSnapshot) -> Context {
        let utc = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let mut tzi = TIME_ZONE_INFORMATION::default();
        let mode = unsafe { GetTimeZoneInformation(&mut tzi) };
        // TIME_ZONE_ID_DAYLIGHT = 2. Bias is minutes to add to local time to get UTC.
        let bias = tzi.Bias + if mode == 2 { tzi.DaylightBias } else { tzi.StandardBias };
        let mut buf = [0u16; 85];
        let n = unsafe { GetUserDefaultLocaleName(&mut buf) };
        let locale = String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize]);
        Context {
            local_secs: utc - i64::from(bias) * 60,
            languages: if locale.is_empty() { Vec::new() } else { vec![locale] },
            name: std::env::var("USERNAME").unwrap_or_default(),
            app: s.app_name.clone(),
            window_title: prompt_title(s.window_title.trim_start_matches(['●', '•', '*', ' ']), &s.text).to_owned(),
            website: s.domain.clone(),
            ..Context::default()
        }
    }

    fn drain_engine() {
        let items: Vec<(u64, String)> = ENGINE.with(|e| e.get().map(|(_, rx)| rx.try_iter().collect()).unwrap_or_default());
        for (id, text) in items {
            GHOST.with(|g| {
                if let Some(gh) = g.borrow_mut().as_mut().filter(|gh| gh.id == id && text.len() > gh.typed) {
                    gh.full = text;
                    // Right after an accept the caret has moved; the next read draws it there.
                    if !gh.accepted_at.is_some_and(|a| a.elapsed() < ACCEPT_LAG) {
                        show(gh);
                    }
                }
            });
        }
    }

    fn show(gh: &mut Ghost) {
        let text = gh.rest();
        gh.visible = OVERLAY.with(|o| {
            let Some(o) = o.get() else { return 0 };
            if text.is_empty() {
                o.hide();
                0
            } else {
                o.show(text, gh.caret, gh.font.as_ref(), gh.edges, gh.right)
            }
        });
        GHOST_VISIBLE.store(gh.visible > 0, Ordering::Relaxed);
    }

    fn clear_ghost() {
        LATEST.fetch_add(1, Ordering::Relaxed);
        GHOST.with(|g| g.borrow_mut().take());
        OVERLAY.with(|o| o.get().map(Overlay::hide));
        GHOST_VISIBLE.store(false, Ordering::Relaxed);
    }

    fn on_key(key: Key) {
        match key {
            Key::Other => schedule_read(),
            Key::Escape => GHOST.with(|g| {
                if let Some(gh) = g.borrow_mut().as_mut() {
                    LATEST.fetch_add(1, Ordering::Relaxed);
                    gh.anchor.push_str(&gh.full[..gh.typed]);
                    (gh.id, gh.typed) = (0, 0);
                    gh.full.clear();
                    show(gh);
                }
            }),
            Key::AcceptAll | Key::AcceptWord => {
                let Some((pid, text, expect)) = GHOST.with(|g| {
                    g.borrow_mut().as_mut().and_then(|gh| {
                        let text = if key == Key::AcceptAll { gh.shown() } else { next_word(gh.shown()) }.to_owned();
                        if text.is_empty() {
                            return None;
                        }
                        let expect = format!("{}{}{text}", gh.anchor, &gh.full[..gh.typed]);
                        // Advance now: a quick second Tab takes the next word, not this one again.
                        // The ghost hides until a read shows where the caret went.
                        gh.typed += text.len();
                        gh.visible = gh.visible.saturating_sub(text.len());
                        gh.accepted_at = Some(Instant::now());
                        OVERLAY.with(|o| o.get().map(Overlay::hide));
                        GHOST_VISIBLE.store(gh.visible > 0, Ordering::Relaxed);
                        Some((gh.pid, text, expect))
                    })
                }) else {
                    return;
                };
                keys::insert(&text);
                eprintln!("accept {key:?}: typed {text:?}");
                CONFIRM.with(|c| *c.borrow_mut() = Some((pid, expect, text)));
                set_timer(TIMER_CONFIRM, CONFIRM_MS);
                schedule_read();
            }
        }
    }

    /// Spec §12.2: check the accepted text landed. shortcut: log only; the clipboard paste
    /// fallback of the Mac runtime is not ported yet.
    fn confirm_insert() {
        let Some((pid, expect, text)) = CONFIRM.with(|c| c.borrow_mut().take()) else { return };
        match snapshot().filter(|s| s.pid == pid) {
            None => eprintln!("insert NOT confirmed: focus moved"),
            Some(s) if s.before().starts_with(&expect) => eprintln!("insert confirmed"),
            // The paste was lost (nothing changed before the caret): send it once more.
            Some(s) if Some(s.before()) == expect.strip_suffix(text.as_str()) => {
                eprintln!("insert lost: pasting {text:?} again");
                keys::insert(&text);
                schedule_read();
            }
            Some(_) => eprintln!("insert NOT confirmed: field differs"),
        }
    }

    fn allowed(s: &FieldSnapshot) -> Result<(), &'static str> {
        if OFF_BY_DEFAULT.contains(&s.bundle_id.as_str()) {
            return Err("turned off for this app");
        }
        gate(s)
    }

    fn log_snapshot(snap: Option<&FieldSnapshot>) {
        let line = match snap {
            None => "no text field focused".to_string(),
            Some(s) => {
                let decision = match allowed(s) {
                    Ok(()) => "COMPLETE".to_string(),
                    Err(why) => format!("skip: {why}"),
                };
                let tail: String = s.before().chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
                let caret = s.caret_rect.map_or("none".into(), |r| format!("({:.0},{:.0} {:.0}x{:.0})", r.x, r.y, r.w, r.h));
                format!(
                    "[{}] {}{} {decision} | caret {caret} field {:.0}x{:.0} | …{tail:?}",
                    s.bundle_id,
                    s.role,
                    if s.subrole.is_empty() { String::new() } else { format!("/{}", s.subrole) },
                    s.frame.w,
                    s.frame.h,
                )
            }
        };
        LAST_LOG.with(|last| {
            if *last.borrow() != line {
                eprintln!("{line}");
                *last.borrow_mut() = line;
            }
        });
    }
}
