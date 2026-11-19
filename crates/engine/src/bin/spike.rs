//! M0 spike: simulate typing `text` one character at a time and measure per-keystroke latency.
//! Usage: spike <model.gguf> "<text to type>" [max_tokens] [clipboard text]

use nextype_core::prompt::{Context, build};
use nextype_engine::{Completer, LlamaBackend, load_model};

fn main() -> Result<(), nextype_engine::Error> {
    let args: Vec<String> = std::env::args().collect();
    let [_, model_path, text, rest @ ..] = args.as_slice() else {
        return Err("usage: spike <model.gguf> \"<text>\" [max_tokens]".into());
    };
    let max_tokens: usize = rest.first().map_or(Ok(8), |s| s.parse())?;
    let ctx = Context { clipboard: rest.get(1).cloned().unwrap_or_default(), ..Context::default() };

    let mut backend = LlamaBackend::init()?;
    backend.void_logs();
    let model = load_model(&backend, model_path)?;
    let mut completer = Completer::new(&backend, &model, 4096)?;

    // Warmup: first full prefill (cold), not counted.
    let (_, cold) = completer.complete(&build("", &Context::default(), "", &|s| completer.count_tokens(s)).text, "", 1, |_| true)?;
    println!("cold prefill: {} tokens in {:.1} ms", cold.prefilled, cold.prefill_ms);

    let (mut lat, mut first) = (Vec::new(), Vec::new());
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for &(i, c) in &chars {
        let typed = &text[..i + c.len_utf8()];
        let p = build(typed, &ctx, "", &|s| completer.count_tokens(s));
        let (out, s) = completer.complete(&p.text, &p.heal, max_tokens, |_| true)?;
        let suggestion = out.strip_prefix(p.heal.as_str()).unwrap_or("");
        lat.push(s.total_ms);
        first.push(s.first_token_ms);
        println!(
            "{:>6.1} ms  (first token {:>5.1} ms, reused {:>3}, new {:>2}, gen {:>2})  {typed:?} -> {suggestion:?}",
            s.total_ms, s.first_token_ms, s.reused, s.prefilled, s.generated
        );
    }
    let pct = |v: &mut Vec<f64>, p: f64| {
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * p).round() as usize]
    };
    println!("keystrokes {}", lat.len());
    println!("first token  p50 {:.1} ms  p95 {:.1} ms", pct(&mut first, 0.5), pct(&mut first, 0.95));
    println!("full ({max_tokens} tok) p50 {:.1} ms  p95 {:.1} ms", pct(&mut lat, 0.5), pct(&mut lat, 0.95));
    Ok(())
}
