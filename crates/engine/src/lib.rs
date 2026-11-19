//! Local completion engine: one llama.cpp context, KV prefix reuse across keystrokes,
//! greedy continuation with typed-prefix healing, streamed and cancellable per token (spec §9).

use std::num::NonZeroU32;
use std::time::Instant;

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::token::LlamaToken;

pub use llama_cpp_2::llama_backend::LlamaBackend;
pub use llama_cpp_2::model::params::LlamaModelParams;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

const N_BATCH: usize = 512;

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    /// Prompt tokens served from the KV cache.
    pub reused: usize,
    /// Prompt tokens that had to be decoded.
    pub prefilled: usize,
    pub generated: usize,
    pub prefill_ms: f64,
    /// Time until the first continuation token is known (what the ghost text waits on).
    pub first_token_ms: f64,
    pub total_ms: f64,
    pub cancelled: bool,
}

pub struct Completer<'m> {
    model: &'m LlamaModel,
    ctx: LlamaContext<'m>,
    /// Prompt tokens currently in KV sequence 0 (generated tokens after them are dropped next call).
    cached: Vec<LlamaToken>,
    batch: LlamaBatch<'static>,
    /// Detokenized piece of every token id (spec §9.2), for typed-prefix matching.
    pieces: Vec<Vec<u8>>,
}

pub fn load_model(backend: &LlamaBackend, path: &str) -> Result<LlamaModel, Error> {
    let params = LlamaModelParams::default().with_n_gpu_layers(999);
    Ok(LlamaModel::load_from_file(backend, path, &params)?)
}

impl<'m> Completer<'m> {
    pub fn new(backend: &LlamaBackend, model: &'m LlamaModel, n_ctx: u32) -> Result<Self, Error> {
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            .with_n_batch(N_BATCH as u32)
            .with_n_ubatch(256)
            // Gemma 4 uses sliding-window attention; a full-size SWA cache lets us roll back the tail.
            .with_swa_full(true);
        let ctx = model.new_context(backend, params)?;
        let vocab = model.vocab();
        let pieces = (0..vocab.n_tokens()).map(|t| vocab.token_to_piece(LlamaToken(t), false, None)).collect();
        Ok(Self { model, ctx, cached: Vec::new(), batch: LlamaBatch::new(N_BATCH, 1), pieces })
    }

    /// Number of tokens `text` takes (no special tokens), for prompt budgets.
    pub fn count_tokens(&self, text: &str) -> usize {
        self.model.vocab().tokenize(text.as_bytes(), false, false).len()
    }

    pub fn tokenize(&self, prompt: &str) -> Vec<LlamaToken> {
        self.model.vocab().tokenize(prompt.as_bytes(), true, true)
    }

    /// Continue `prompt` greedily for up to `max_tokens`, stopping at end-of-generation or a newline.
    /// `heal` is typed text cut from the end of the prompt: the first tokens are forced to spell it
    /// out again (spec §9.4), so the returned text starts with `heal` unless generation stopped early.
    /// `on_text` gets the text so far after every token; returning `false` cancels.
    pub fn complete(
        &mut self,
        prompt: &str,
        heal: &str,
        max_tokens: usize,
        mut on_text: impl FnMut(&str) -> bool,
    ) -> Result<(String, Stats), Error> {
        let start = Instant::now();
        let tokens = self.tokenize(prompt);
        if tokens.is_empty() || tokens.len() + max_tokens > self.ctx.n_ctx() as usize {
            return Err("prompt empty or longer than the context".into());
        }
        // Always re-decode at least the last prompt token so its logits are fresh.
        let mut keep = common_prefix(&self.cached, &tokens).min(tokens.len() - 1);
        if !self.ctx.clear_kv_cache_seq(Some(0), Some(keep as u32), None)? {
            self.ctx.clear_kv_cache_seq(None, None, None)?;
            keep = 0;
        }
        self.cached.clear();

        for (chunk_start, chunk) in tokens[keep..].chunks(N_BATCH).enumerate().map(|(i, c)| (keep + i * N_BATCH, c)) {
            self.batch.clear();
            for (i, &t) in chunk.iter().enumerate() {
                let pos = chunk_start + i;
                self.batch.add(t, pos as i32, &[0], pos == tokens.len() - 1)?;
            }
            self.ctx.decode(&mut self.batch)?;
        }
        self.cached = tokens.clone();
        let mut stats = Stats {
            reused: keep,
            prefilled: tokens.len() - keep,
            prefill_ms: ms(start),
            ..Stats::default()
        };

        let vocab = self.model.vocab();
        let mut out = Vec::new();
        let mut logits_idx = self.batch.n_tokens() - 1;
        let mut pos = tokens.len();
        while stats.generated < max_tokens {
            let logits = self.ctx.get_logits_ith(logits_idx);
            let next = match heal.as_bytes().get(out.len()..).filter(|rest| !rest.is_empty()) {
                Some(rest) => match heal_argmax(logits, &self.pieces, rest) {
                    Some(t) => t,
                    None => break,
                },
                None => argmax(logits),
            };
            if stats.generated == 0 {
                stats.first_token_ms = ms(start);
            }
            if vocab.is_eog(next) {
                break;
            }
            let piece = &self.pieces[next.0 as usize];
            if piece.contains(&b'\n') {
                break;
            }
            out.extend_from_slice(piece);
            stats.generated += 1;
            if !on_text(utf8_prefix(&out)) {
                stats.cancelled = true;
                break;
            }
            if stats.generated == max_tokens {
                break;
            }
            self.batch.clear();
            self.batch.add(next, pos as i32, &[0], true)?;
            self.ctx.decode(&mut self.batch)?;
            logits_idx = 0;
            pos += 1;
        }
        stats.total_ms = ms(start);
        Ok((utf8_prefix(&out).to_owned(), stats))
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn argmax(logits: &[f32]) -> LlamaToken {
    let (i, _) = logits
        .iter()
        .enumerate()
        .fold((0, f32::NEG_INFINITY), |best, (i, &l)| if l > best.1 { (i, l) } else { best });
    LlamaToken(i as i32)
}

/// Best token whose piece agrees with the typed bytes still to spell (`rest`): the piece is a
/// prefix of `rest`, or `rest` is a prefix of the piece. Empty and whitespace-only pieces are
/// skipped; a lone space token followed by a word is an unnatural split for SentencePiece.
fn heal_argmax(logits: &[f32], pieces: &[Vec<u8>], rest: &[u8]) -> Option<LlamaToken> {
    pieces
        .iter()
        .zip(logits)
        .enumerate()
        .filter(|(_, (p, _))| !p.iter().all(u8::is_ascii_whitespace) && (rest.starts_with(p) || p.starts_with(rest)))
        .max_by(|a, b| a.1.1.total_cmp(b.1.1))
        .map(|(i, _)| LlamaToken(i as i32))
}

/// Longest valid UTF-8 prefix (never show half a character).
fn utf8_prefix(b: &[u8]) -> &str {
    std::str::from_utf8(b).unwrap_or_else(|e| std::str::from_utf8(&b[..e.valid_up_to()]).unwrap_or_default())
}

pub fn common_prefix<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_prefix_counts_shared_head() {
        assert_eq!(common_prefix(&[1, 2, 3], &[1, 2, 4, 5]), 2);
        assert_eq!(common_prefix::<i32>(&[], &[1]), 0);
        assert_eq!(common_prefix(&[1, 2], &[1, 2]), 2);
    }

    #[test]
    fn argmax_picks_largest() {
        assert_eq!(argmax(&[0.1, 3.0, -1.0, 2.9]), LlamaToken(1));
    }

    #[test]
    fn heal_argmax_only_consistent_pieces() {
        let pieces: Vec<Vec<u8>> = [" sch", " s", "oud", " ", " schedule", "", " sc"].map(|p| p.as_bytes().to_vec()).into();
        let logits = [1.0, 2.0, 9.0, 8.0, 3.0, 7.0, 0.5];
        // "oud", " " and "" are excluded; " schedule" (rest is its prefix) wins.
        assert_eq!(heal_argmax(&logits, &pieces, b" sch"), Some(LlamaToken(4)));
        assert_eq!(heal_argmax(&logits, &pieces, b"xyz"), None);
        assert_eq!(utf8_prefix("aé".as_bytes().split_last().unwrap().1), "a");
    }
}
