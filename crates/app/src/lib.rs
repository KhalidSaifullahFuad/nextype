//! NexType core runtime. Follows the frontmost app's focused text field, asks the engine
//! for a continuation, shows it as ghost text at the caret, and types it on Tab / key above Tab.
//! The shell (Swift) owns `NSApplication` and calls `start` once on the main thread.

mod clipboard;
mod engine;
pub mod history;
mod keys;
mod overlay;
pub mod screen;
pub mod stats;
pub mod settings;

use std::cell::{OnceCell, RefCell};
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::time::Duration;

use dispatch2::DispatchQueue;
use engine::{LATEST, Request};
use keys::{GHOST_VISIBLE, Key};
use nextype_ax::AppObserver;
use nextype_core::prompt::Context;
use nextype_core::{FieldSnapshot, OFF_BY_DEFAULT, Rect, gate, next_word, prompt_title};
use objc2_foundation::{NSFullUserName, NSLocale};
use objc2::MainThreadMarker;
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFRetained, CFRunLoop, CFRunLoopTimer, kCFRunLoopCommonModes,
};
use overlay::Overlay;

/// Spec §5.1: wait 5 ms after an AX change before reading the field.
const DEBOUNCE_S: f64 = 0.005;
/// How often we check which app is frontmost (re-attach the observer on change).
const APP_POLL_S: f64 = 0.25;
/// Spec §12.2 `insertionConfirmationTTL`.
const INSERT_CONFIRM: Duration = Duration::from_secs(1);

/// The suggestion for the focused field: `anchor` (the text before the caret when we asked)
/// continued by `full`, of which the first `typed` bytes are already in the field.
struct Ghost {
    /// Engine request still feeding `full` (0 = none).
    id: u64,
    pid: i32,
    anchor: String,
    full: String,
    typed: usize,
    caret: Rect,
    font: Option<(String, f64)>,
    /// Text edges to wrap between (`FieldSnapshot::wrap_edges`).
    edges: Option<(f64, f64)>,
}

impl Ghost {
    fn shown(&self) -> &str {
        &self.full[self.typed..]
    }
}

thread_local! {
    static OBSERVER: RefCell<Option<AppObserver>> = const { RefCell::new(None) };
    static DEBOUNCE: RefCell<Option<CFRetained<CFRunLoopTimer>>> = const { RefCell::new(None) };
    static LAST_LOG: RefCell<String> = const { RefCell::new(String::new()) };
    static GHOST: RefCell<Option<Ghost>> = const { RefCell::new(None) };
    static OVERLAY: OnceCell<Overlay> = const { OnceCell::new() };
    static ENGINE: OnceCell<Sender<Request>> = const { OnceCell::new() };
}

/// Whether this process has the Accessibility permission; `prompt` shows the system dialog.
/// A grant made while we run is not seen until relaunch (spec §23 M1 lessons).
pub fn accessibility_trusted(prompt: bool) -> bool {
    nextype_ax::is_trusted(prompt)
}

/// Switch completions on or off. Main thread only.
/// Completions start off in this app until the user turns them on.
pub fn off_by_default(bundle_id: &str) -> bool {
    OFF_BY_DEFAULT.contains(&bundle_id)
}

/// Undo changes made to other apps; call before quitting.
pub fn stop() {
    history::flush();
    nextype_ax::reset_enhanced_ui();
}

pub fn set_enabled(on: bool) {
    settings::ENABLED.store(on, Ordering::Relaxed);
    if !on {
        clear_ghost();
    }
}

/// Start watching and completing. Needs the main thread, a running `NSApplication` (system-wide
/// AX queries fail from a bare run loop) and Accessibility. Returns false if either is missing;
/// calling it again after success is a no-op.
pub fn start(model_path: String) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    if OVERLAY.with(|o| o.get().is_some()) {
        return true;
    }
    if !nextype_ax::is_trusted(false) {
        return false;
    }
    eprintln!("NexType: watching focused text fields.");

    OVERLAY.with(|o| o.set(Overlay::new(mtm)).ok());
    ENGINE.with(|e| e.set(engine::spawn(model_path, on_engine_text)).ok());
    if !keys::install_tap(on_key) {
        eprintln!("NexType: could not install the keyboard tap; accept keys will not work.");
    }

    // Far-future repeating timer that we re-arm on each AX event: a cheap restartable debounce.
    // ~3 years out: CF misbehaves with absurd dates like f64::MAX.
    let debounce = timer(1.0e8, 1.0e8, on_debounce);
    let poll = timer(0.0, APP_POLL_S, on_app_poll);
    let rl = CFRunLoop::main().expect("main run loop");
    for t in [&debounce, &poll] {
        rl.add_timer(Some(t), unsafe { kCFRunLoopCommonModes });
    }
    DEBOUNCE.with(|d| *d.borrow_mut() = Some(debounce));
    true
}

fn timer(first_fire_in: f64, interval: f64, cb: unsafe extern "C-unwind" fn(*mut CFRunLoopTimer, *mut std::ffi::c_void)) -> CFRetained<CFRunLoopTimer> {
    let now = CFAbsoluteTimeGetCurrent();
    unsafe { CFRunLoopTimer::new(None, now + first_fire_in, interval, 0, 0, Some(cb), ptr::null_mut()) }
        .expect("CFRunLoopTimer")
}

fn schedule_read() {
    DEBOUNCE.with(|d| {
        if let Some(t) = d.borrow().as_ref() {
            t.set_next_fire_date(CFAbsoluteTimeGetCurrent() + DEBOUNCE_S);
        }
    });
}

fn on_ax_event(_notification: &str) {
    schedule_read();
}

unsafe extern "C-unwind" fn on_app_poll(_: *mut CFRunLoopTimer, _: *mut std::ffi::c_void) {
    clipboard::poll();
    let Some(pid) = nextype_ax::focused_app_pid() else { return };
    let changed = OBSERVER.with(|o| o.borrow().as_ref().map(|o| o.pid) != Some(pid));
    if changed {
        clear_ghost();
        history::flush();
        // Drop the old observer first so its run-loop source is removed.
        OBSERVER.with(|o| {
            o.borrow_mut().take();
            let obs = AppObserver::new(pid, on_ax_event);
            eprintln!("frontmost app pid {pid}: observer {}", if obs.is_some() { "attached" } else { "FAILED" });
            *o.borrow_mut() = obs;
        });
        schedule_read();
    }
}

unsafe extern "C-unwind" fn on_debounce(_: *mut CFRunLoopTimer, _: *mut std::ffi::c_void) {
    if !settings::ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let snap = nextype_ax::snapshot();
    log_snapshot(snap.as_ref());
    if let Ok(mut d) = settings::CURRENT_DOMAIN.lock() {
        *d = snap.as_ref().map(|s| s.domain.clone()).unwrap_or_default();
    }
    if let Some(s) = snap.as_ref().filter(|_| settings::HISTORY.load(Ordering::Relaxed)) {
        history::leave_if_moved(s);
    }
    let Some(s) = snap.filter(|s| allowed(s).is_ok()) else {
        screen::set_target(None);
        clear_ghost();
        return;
    };
    screen::set_target(Some(&s));
    if settings::HISTORY.load(Ordering::Relaxed) {
        history::observe(&s);
    }
    keys::TAB_ENABLED.store(!settings::override_for(&s).is_some_and(|o| o.tab_off), Ordering::Relaxed);
    let (before, caret) = (s.before(), s.caret_rect.unwrap_or_default());
    let kept = GHOST.with(|g| {
        let mut g = g.borrow_mut();
        let Some(gh) = g.as_mut().filter(|gh| gh.pid == s.pid) else { return false };
        let Some(t) = before.strip_prefix(gh.anchor.as_str()) else { return false };
        // Infer-from-ghost (spec §9.6): typing (or our own insertion) that matches the ghost
        // only advances it; no new model run until it is used up.
        if !(gh.full.starts_with(t) && (t.len() < gh.full.len() || t.is_empty())) {
            return false;
        }
        gh.typed = t.len();
        (gh.caret, gh.font, gh.edges) = (caret, field_font(&s), s.wrap_edges());
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
        let gh = Ghost {
            id,
            pid: s.pid,
            anchor: before.clone(),
            full: String::new(),
            typed: 0,
            caret,
            font: field_font(s),
            edges: s.wrap_edges(),
        };
        show(&gh);
        *g.borrow_mut() = Some(gh);
    });
    let mut instructions = settings::INSTRUCTIONS.lock().map(|s| s.trim().to_owned()).unwrap_or_default();
    if let Some(o) = settings::override_for(s).filter(|o| !o.instructions.trim().is_empty()) {
        if !instructions.is_empty() {
            instructions.push('\n');
        }
        instructions.push_str(o.instructions.trim());
    }
    ENGINE.with(|e| e.get().map(|tx| tx.send(Request { id, before, context: gather_context(s), instructions })));
}

/// The context lines of the prompt (spec §7.1), read on the main thread.
fn gather_context(s: &FieldSnapshot) -> Context {
    // SAFETY: plain libc time calls; `tm` is zero-initialised and filled by localtime_r.
    let local_secs = unsafe {
        let now = libc::time(ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        now as i64 + i64::from(tm.tm_gmtoff)
    };
    Context {
        local_secs,
        languages: NSLocale::preferredLanguages().iter().map(|l| l.to_string()).collect(),
        name: NSFullUserName().to_string(),
        app: s.app_name.clone(),
        // Editors prefix unsaved documents with a dot; it isn't part of the name. Titles that
        // repeat the user's text (an unsaved document named after its first line) are cut to the app.
        window_title: prompt_title(s.window_title.trim_start_matches(['●', '•', '*', ' ']), &s.text).to_owned(),
        website: s.domain.clone(),
        past: if settings::HISTORY.load(Ordering::Relaxed) { history::examples(s) } else { Vec::new() },
        screen: if settings::SCREEN.load(Ordering::Relaxed) { screen::text_for(s) } else { String::new() },
        clipboard: if settings::CLIPBOARD.load(Ordering::Relaxed) { clipboard::recent_text().unwrap_or_default() } else { String::new() },
    }
}

/// Engine output for request `id`, on the main thread.
fn on_engine_text(id: u64, text: String) {
    GHOST.with(|g| {
        if let Some(gh) = g.borrow_mut().as_mut().filter(|gh| gh.id == id && text.len() > gh.typed) {
            gh.full = text;
            show(gh);
            let c = gh.caret;
            eprintln!("ghost #{id} {:?} at ({:.0},{:.0} h{:.0}) font {:?}", gh.shown(), c.x, c.y, c.h, gh.font);
        }
    });
}

fn show(gh: &Ghost) {
    let text = gh.shown();
    OVERLAY.with(|o| {
        let Some(o) = o.get() else { return };
        if text.is_empty() { o.hide() } else { o.show(text, gh.caret, gh.font.as_ref(), gh.edges) }
    });
    GHOST_VISIBLE.store(!text.is_empty(), Ordering::Relaxed);
}

fn clear_ghost() {
    LATEST.fetch_add(1, Ordering::Relaxed); // abandon any running request
    GHOST.with(|g| g.borrow_mut().take());
    OVERLAY.with(|o| o.get().map(Overlay::hide));
    GHOST_VISIBLE.store(false, Ordering::Relaxed);
}

fn on_key(key: Key) {
    match key {
        Key::Other => schedule_read(),
        Key::Escape => GHOST.with(|g| {
            // Dismiss: re-anchor at the current text with an empty ghost, so the same text
            // doesn't ask again; the next real edit does.
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
                g.borrow().as_ref().map(|gh| {
                    let text = if matches!(key, Key::AcceptAll) { gh.shown() } else { next_word(gh.shown()) };
                    (gh.pid, text.to_owned(), format!("{}{}{text}", gh.anchor, &gh.full[..gh.typed]))
                })
            }) else {
                return;
            };
            if text.is_empty() {
                return;
            }
            // Test hook: skip typing so the paste fallback below runs.
            if std::env::var_os("NEXTYPE_TEST_SKIP_TYPING").is_none() {
                keys::insert(pid, &text);
            }
            stats::record(matches!(key, Key::AcceptWord), &text);
            eprintln!("accept {key:?}: typed {text:?}");
            // Spec §12.2: confirm the text landed; if it didn't, paste whatever is missing.
            let when = INSERT_CONFIRM.try_into().expect("dispatch time");
            let _ = DispatchQueue::main().after(when, move || {
                let Some(s) = nextype_ax::snapshot().filter(|s| s.pid == pid) else {
                    return eprintln!("insert NOT confirmed: focus moved, not pasting");
                };
                let base = &expect[..expect.len() - text.len()];
                match rest_to_paste(s.before(), base, &text) {
                    None if s.before().starts_with(&expect) => eprintln!("insert confirmed"),
                    Some(rest) => {
                        keys::paste(pid, rest);
                        eprintln!("insert NOT confirmed: pasted {} of {} chars", rest.chars().count(), text.chars().count());
                    }
                    None => eprintln!("insert NOT confirmed: field changed, not pasting"),
                }
            });
        }
    }
}

/// The part of `text` still missing when the field reads `before` after we typed `text` at the
/// end of `base`: `None` if all of it landed, or if the field changed in a way we can't
/// account for (the user kept typing, autocorrect, …), where pasting could double text.
fn rest_to_paste<'a>(before: &str, base: &str, text: &'a str) -> Option<&'a str> {
    let landed = before.strip_prefix(base)?;
    let rest = text.strip_prefix(landed)?;
    (!rest.is_empty()).then_some(rest)
}

/// The field's font, or a guess from measured text widths when the app doesn't report one.
fn field_font(s: &FieldSnapshot) -> Option<(String, f64)> {
    s.font.clone().or_else(|| s.width_sample.as_ref().and_then(overlay::guess_font))
}

/// Built-in gate (spec §5.2) plus the apps the user turned off.
fn allowed(s: &FieldSnapshot) -> Result<(), &'static str> {
    let off = settings::override_for(s).map_or(OFF_BY_DEFAULT.contains(&s.bundle_id.as_str()), |o| o.completions_off);
    if off {
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
            let head: String = s.after().chars().take(20).collect();
            let caret = s.caret_rect.map_or("none".into(), |r| format!("({:.0},{:.0} {:.0}x{:.0})", r.x, r.y, r.w, r.h));
            format!(
                "[{}] {}{} {decision} | caret {caret} field {:.0}x{:.0}{} | …{tail:?} ⎀ {head:?}…",
                s.bundle_id,
                s.role,
                if s.subrole.is_empty() { String::new() } else { format!("/{}", s.subrole) },
                s.frame.w,
                s.frame.h,
                if s.domain.is_empty() { String::new() } else { format!(" site {}", s.domain) },
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

#[cfg(test)]
mod tests {
    use super::rest_to_paste;

    #[test]
    fn pastes_only_what_is_missing() {
        assert_eq!(rest_to_paste("Hi", "Hi", " there"), Some(" there")); // nothing landed
        assert_eq!(rest_to_paste("Hi th", "Hi", " there"), Some("ere")); // part landed
        assert_eq!(rest_to_paste("Hi there", "Hi", " there"), None); // all landed
        assert_eq!(rest_to_paste("Hi thx", "Hi", " there"), None); // user typed on
        assert_eq!(rest_to_paste("Ho", "Hi", " there"), None); // field rewritten
    }
}
