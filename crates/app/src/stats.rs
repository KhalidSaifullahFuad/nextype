//! Accept stats (spec §9.5): `completionStats.json` in Application Support,
//! `{"perDay": {"YYYY-MM-DD": {"short"|"long": {characterCount, completionCount, wordCountTotal}}}}`.
//! `short` = next-word accepts, `long` = whole-ghost accepts.
//! shortcut: no `wordCountAboveLengthThreshold` (its threshold is unknown); add it if a feature needs it.

use std::path::PathBuf;

use serde_json::{Value, json};

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/NexType/completionStats.json"))
}

fn load() -> Value {
    path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "perDay": {} }))
}

/// Local date as YYYY-MM-DD.
fn today_key() -> String {
    // SAFETY: plain libc time calls; `tm` is zero-initialised and filled by localtime_r.
    let tm = unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm
    };
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

/// Add one accept to today's counts. Errors are logged, never fatal.
pub fn record(short: bool, text: &str) {
    let Some(p) = path() else { return };
    let mut v = load();
    add(&mut v, &today_key(), short, text);
    let tmp = p.with_extension("json.tmp");
    let res = std::fs::create_dir_all(p.parent().expect("has parent"))
        .and_then(|_| std::fs::write(&tmp, v.to_string()))
        .and_then(|_| std::fs::rename(&tmp, &p));
    if let Err(e) = res {
        eprintln!("stats: could not save: {e}");
    }
}

fn add(v: &mut Value, day: &str, short: bool, text: &str) {
    let e = &mut v["perDay"][day][if short { "short" } else { "long" }];
    for (k, n) in [
        ("characterCount", text.chars().count() as u64),
        ("completionCount", 1),
        ("wordCountTotal", text.split_whitespace().count() as u64),
    ] {
        e[k] = json!(e[k].as_u64().unwrap_or(0) + n);
    }
}

/// (completions, words) accepted today.
pub fn today() -> (u64, u64) {
    sum(&load(), &today_key())
}

/// (YYYY-MM-DD, completions, words, characters) for every day with accepts, oldest first.
pub fn per_day() -> Vec<(String, u64, u64, u64)> {
    days(&load())
}

fn days(v: &Value) -> Vec<(String, u64, u64, u64)> {
    let mut out: Vec<_> = v["perDay"]
        .as_object()
        .map(|m| m.keys().map(|d| { let (c, w) = sum(v, d); (d.clone(), c, w, total(v, d, "characterCount")) }).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn sum(v: &Value, day: &str) -> (u64, u64) {
    (total(v, day, "completionCount"), total(v, day, "wordCountTotal"))
}

/// One counter for a day, short and long accepts together.
fn total(v: &Value, day: &str, k: &str) -> u64 {
    let d = &v["perDay"][day];
    ["short", "long"].iter().map(|t| d[t][k].as_u64().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_add_up_per_day() {
        let mut v = json!({ "perDay": {} });
        add(&mut v, "2026-11-19", true, "hello ");
        add(&mut v, "2026-11-19", false, "big brown fox.");
        add(&mut v, "2026-11-18", false, "old");
        assert_eq!(sum(&v, "2026-11-19"), (2, 4));
        assert_eq!(v["perDay"]["2026-11-19"]["short"]["characterCount"], 6);
        assert_eq!(sum(&v, "2026-11-20"), (0, 0));
        assert_eq!(days(&v), vec![("2026-11-18".into(), 1, 1, 3), ("2026-11-19".into(), 2, 4, 20)]);
    }
}
