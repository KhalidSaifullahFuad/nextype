//! Completion worker thread (spec §9): owns the model; the newest request wins and older
//! ones are abandoned between tokens. Text is streamed back to the main thread.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};

use crate::settings;
use dispatch2::DispatchQueue;
use nextype_core::prompt::{Context, Template, build};
use nextype_core::{MID_LINE_WORDS, copy_cut, mid_line_ghost, repeat_cut, sentence_end, tail, word_cut};
use nextype_engine::{Completer, LlamaBackend, load_model};

/// Id of the newest request (0 = none). The main thread bumps it to cancel.
pub static LATEST: AtomicU64 = AtomicU64::new(0);

const N_CTX: u32 = 4096;
/// Text before the caret handed to the prompt builder, which trims it to its token budget.
const WINDOW_CHARS: usize = 4000;


/// `qos_class_t` value from <sys/qos.h>.
const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

pub struct Request {
    pub id: u64,
    pub before: String,
    pub context: Context,
    /// Global custom instructions plus the app's own.
    pub instructions: String,
}


/// Start the worker. `on_text(id, suggestion)` runs on the main thread each time the
/// suggestion (the text after what is already typed) changes.
pub fn spawn(model_path: String, on_text: fn(u64, String)) -> Sender<Request> {
    let (tx, rx) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        // Ghost latency is what the user feels while typing; a default-QoS thread in a
        // background agent gets throttled CPU and GPU time (first token 0.3–0.6 s instead of ~35 ms).
        unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
        let run = || -> Result<(), nextype_engine::Error> {
            let mut backend = LlamaBackend::init()?;
            backend.void_logs();
            let model = load_model(&backend, &model_path)?;
            let mut completer = Completer::new(&backend, &model, N_CTX)?;
            let arch = model.meta_val_str("general.architecture").unwrap_or_default();
            let template = Template::from_arch(&arch).unwrap_or_else(|| {
                eprintln!("engine: unknown architecture {arch:?}, using the Gemma 4 format");
                Template::default()
            });
            eprintln!("engine: model loaded ({arch})");
            // Ends when the sender is dropped (another model was picked), freeing this one.
            while let Ok(mut req) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    req = newer;
                }
                let id = req.id;
                if LATEST.load(Ordering::Relaxed) != id {
                    continue;
                }
                req.context.template = template;
                let prompt = build(tail(&req.before, WINDOW_CHARS), &req.context, &req.instructions, &|s| completer.count_tokens(s));
                let heal = prompt.heal.as_str();
                let after_line = req.context.after.lines().next().unwrap_or("");
                let (mut sent, mut sentence_done) = (String::new(), false);
                let result = completer.complete(&prompt.text, heal, settings::MAX_TOKENS.load(Ordering::Relaxed), |text| {
                    let Some(raw) = text.strip_prefix(heal) else { return LATEST.load(Ordering::Relaxed) == id };
                    let mid;
                    let mut s = raw;
                    if !after_line.is_empty() {
                        let rejoined;
                        (mid, rejoined) = mid_line_ghost(raw, after_line);
                        s = &mid;
                        sentence_done |= rejoined;
                    }
                    // Stop at a sentence end, or before a run like "....." the model got stuck on.
                    // …or before it starts copying the window title from the prompt's context.
                    let copied = copy_cut(s, &req.context.window_title, 8);
                    let short = if after_line.is_empty() { None } else { word_cut(s, MID_LINE_WORDS) };
                    if let Some(end) = [sentence_end(s), repeat_cut(s), copied, short].into_iter().flatten().min() {
                        (s, sentence_done) = (&s[..end], true);
                    }
                    // Any change, not just growth: a mid-line ghost shrinks when it turns out to
                    // repeat the text after the caret.
                    if s != sent {
                        sent = s.to_owned();
                        let s = s.to_owned();
                        DispatchQueue::main().exec_async(move || on_text(id, s));
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
    tx
}
