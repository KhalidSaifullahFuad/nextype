//! Secret redaction for every context string that goes into a prompt (spec §7.5).
//! Never applied to the text before the caret: the model must continue that exactly.

use std::sync::OnceLock;

use regex::Regex;

const MASK: &str = "[REDACTED]";

/// (pattern, replacement). Patterns are the ones found in Cotypist's binary; `${1}` keeps the
/// label. shortcut: no cheap pre-check pass, the regex crate is fast enough on prompt-sized text.
const RULES: &[(&str, &str)] = &[
    (r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----", MASK),
    (r"([A-Za-z][A-Za-z0-9+.\-]*://)[^\s/:@]+:[^\s/@]+@", "${1}[REDACTED]@"),
    (r"(?i)([?&][A-Za-z0-9_.\-]*sig(?:nature)?=)[^&\s\x22'<>()\[\]]{8,}", "${1}[REDACTED]"),
    (r"(?i)(\b(?:pass(?:word|wort|wd|phrase)|kennwort)[ \t]*[:=][ \t]*)(\S*[^\s.,;:!?])", "${1}[REDACTED]"),
    (
        r"(?mi)^([ \t]*(?:export[ \t]+)?[A-Za-z_][A-Za-z0-9_]*?(?:secret|password|passwd|token|api[_-]?key|access[_-]?key|private[_-]?key|credential|client[_-]?secret|authorization)[A-Za-z0-9_]*[ \t]*=)[ \t]*(\S.*)$",
        "${1} [REDACTED]",
    ),
    (r"\b(?:AKIA|ASIA)[0-9A-Z]{16}", MASK),
    (r"\b(?:sk|rk)_(?:live|test)_[0-9A-Za-z]{16,}", MASK),
    (r"\bsk-(?:proj-)?[0-9A-Za-z_-]{20,}", MASK),
    (r"\bgh[pousr]_[0-9A-Za-z]{20,}", MASK),
    (r"\bAIza[0-9A-Za-z_-]{35}", MASK),
    (r"\bxox[baprs]-[0-9A-Za-z-]{10,}", MASK),
    (r"\beyJ[0-9A-Za-z_-]{8,}\.[0-9A-Za-z_-]{8,}\.[0-9A-Za-z_-]{8,}", MASK),
    (r"\b[A-Z]{2}\d{2}(?:[ ]?[A-Z0-9]{4}){2,7}(?:[ ]?[A-Z0-9]{1,3})?\b", MASK),
];

fn rules() -> &'static [(Regex, &'static str)] {
    static R: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    R.get_or_init(|| RULES.iter().map(|&(p, r)| (Regex::new(p).expect("redaction pattern"), r)).collect())
}

pub fn redact(s: &str) -> String {
    rules().iter().fold(s.to_owned(), |acc, (re, rep)| re.replace_all(&acc, *rep).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (input, expected output): one positive per rule.
    #[test]
    fn redacts_each_kind() {
        let cases = [
            ("k\n-----BEGIN RSA PRIVATE KEY-----\nabc\ndef\n-----END RSA PRIVATE KEY-----\nz", "k\n[REDACTED]\nz"),
            ("see postgres://admin:hunter2@db.local/x", "see postgres://[REDACTED]@db.local/x"),
            ("https://x.io/f?id=1&Signature=abcdef123456&b=2", "https://x.io/f?id=1&Signature=[REDACTED]&b=2"),
            ("My password: Tr0ub4dor&3.", "My password: [REDACTED]."),
            ("Kennwort = geheim123", "Kennwort = [REDACTED]"),
            ("export DB_TOKEN=abc.def\nPATH=/bin", "export DB_TOKEN= [REDACTED]\nPATH=/bin"),
            ("key AKIAIOSFODNN7EXAMPLE end", "key [REDACTED] end"),
            ("sk_live_0123456789abcdefABCD", "[REDACTED]"),
            ("sk-proj-abcdefghij0123456789", "[REDACTED]"),
            ("ghp_0123456789abcdefghijABCD", "[REDACTED]"),
            ("AIzaSyA-0123456789abcdefghijklmnopqrstu", "[REDACTED]"),
            ("xoxb-1234567890-abcdef", "[REDACTED]"),
            ("eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4", "[REDACTED]"),
            ("pay DE89 3704 0044 0532 0130 00 now", "pay [REDACTED] now"),
        ];
        for (input, want) in cases {
            assert_eq!(redact(input), want, "input: {input}");
        }
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        for s in [
            "Meet at 5pm, ask about the password policy.",
            "git clone https://github.com/a/b",
            "https://x.io/f?sig=short",
            "PATH=/usr/bin\nHOME=/Users/me",
            "The sky is blue. Token ring networks are old.",
            "Order AB12 is ready",
            "sk-short",
            "Call 555 0100 today",
        ] {
            assert_eq!(redact(s), s);
        }
    }
}
