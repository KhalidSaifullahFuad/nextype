//! Completion worker thread (spec §9), Windows twin of `crates/app/src/engine.rs`: owns the
//! model; the newest request wins and older ones are abandoned between tokens. Text goes back
//! to the UI thread through a channel plus a thread message that wakes its message loop.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use nextype_core::prompt::{Context, build};
use nextype_core::{copy_cut, repeat_cut, sentence_end, tail};
use nextype_engine::{Completer, LlamaBackend, load_model};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL};
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use crate::WM_ENGINE_TEXT;

/// Id of the newest request (0 = none). The UI thread bumps it to cancel.
pub static LATEST: AtomicU64 = AtomicU64::new(0);

const N_CTX: u32 = 4096;
/// Text before the caret handed to the prompt builder, which trims it to its token budget.
const WINDOW_CHARS: usize = 4000;

pub struct Request {
    pub id: u64,
    pub before: String,
    pub context: Context,
    pub instructions: String,
    /// Longest ghost in tokens (`settings::Length`).
    pub max_tokens: usize,
}

/// Start the worker. Each time a suggestion grows, `(id, text)` is queued on the returned
/// receiver and `WM_ENGINE_TEXT` is posted to `ui_thread`.
pub fn spawn(model_path: String, ui_thread: u32) -> (Sender<Request>, Receiver<(u64, String)>) {
    let (tx, rx) = mpsc::channel::<Request>();
    let (out_tx, out_rx) = mpsc::channel::<(u64, String)>();
    std::thread::spawn(move || {
        // Ghost latency is what the user feels while typing.
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
        }
        let send = |id: u64, s: String| {
            if out_tx.send((id, s)).is_ok() {
                unsafe {
                    let _ = PostThreadMessageW(ui_thread, WM_ENGINE_TEXT, WPARAM(0), LPARAM(0));
                }
            }
        };
        let run = || -> Result<(), nextype_engine::Error> {
            let mut backend = LlamaBackend::init()?;
            backend.void_logs();
            let model = load_model(&backend, &model_path)?;
            let mut completer = Completer::new(&backend, &model, N_CTX)?;
            eprintln!("engine: model loaded");
            while let Ok(mut req) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    req = newer;
                }
                let id = req.id;
                if LATEST.load(Ordering::Relaxed) != id {
                    continue;
                }
                let prompt = build(tail(&req.before, WINDOW_CHARS), &req.context, &req.instructions, &|s| completer.count_tokens(s));
                let heal = prompt.heal.as_str();
                let (mut shown, mut sentence_done) = (0, false);
                let result = completer.complete(&prompt.text, heal, req.max_tokens, |text| {
                    let Some(mut s) = text.strip_prefix(heal) else { return LATEST.load(Ordering::Relaxed) == id };
                    let copied = copy_cut(s, &req.context.window_title, 8);
                    if let Some(end) = [sentence_end(s), repeat_cut(s), copied].into_iter().flatten().min() {
                        (s, sentence_done) = (&s[..end], true);
                    }
                    if s.len() > shown {
                        shown = s.len();
                        send(id, s.to_owned());
                    }
                    !sentence_done && LATEST.load(Ordering::Relaxed) == id
                });
                match result {
                    Ok((_, st)) => eprintln!(
                        "engine #{id}: first token {:.0} ms, total {:.0} ms, reused {}, new {}, gen {}{}",
                        st.first_token_ms, st.total_ms, st.reused, st.prefilled, st.generated,
                        if sentence_done { " (sentence end)" } else if st.cancelled { " (cancelled)" } else { "" }
                    ),
                    Err(e) => eprintln!("engine #{id}: {e}"),
                }
            }
            Ok(())
        };
        if let Err(e) = run() {
            eprintln!("engine failed: {e}");
        }
    });
    (tx, out_rx)
}
