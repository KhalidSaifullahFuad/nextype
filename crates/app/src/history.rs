//! Past writing (spec §7.4, §13): what the user wrote in each field, kept in an encrypted
//! SQLCipher database (key in the Keychain), and a few examples of it for the prompt.
//! Main thread only.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nextype_core::FieldSnapshot;
use rusqlite::{Connection, params};

/// Shorter texts aren't worth keeping (spec §13.2).
const MIN_RECORD_CHARS: usize = 33;
/// Only the end of long fields is kept.
const MAX_RECORD_CHARS: usize = 4000;
/// Examples must be at least this long (spec §7.4).
const MIN_EXAMPLE_CHARS: usize = 100;
const WINDOW_SECS: f64 = 30.0 * 86_400.0;
const KEYCHAIN_SERVICE: &str = "dev.nextype.NexType.database";
const KEYCHAIN_ACCOUNT: &str = "history";

struct Session {
    bundle_id: String,
    domain: String,
    /// Identifies the field (see `field_key`).
    key: String,
    text: String,
}

thread_local! {
    /// None until the first use; Some(None) once opening failed (Keychain denied), so it isn't
    /// retried, and the Keychain prompt doesn't come back, on every keystroke.
    static DB: RefCell<Option<Option<Connection>>> = const { RefCell::new(None) };
    /// The field being typed in; saved when the user moves to another one.
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    /// Examples for the current field, fetched once per field so the prompt prefix stays stable.
    static EXAMPLES: RefCell<(String, Vec<String>)> = const { RefCell::new((String::new(), Vec::new())) };
}

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/NexType/history.db"))
}

/// The database key from the Keychain, created on first use.
fn key() -> Option<String> {
    use security_framework::passwords::{get_generic_password, set_generic_password};
    if let Ok(k) = get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
        return String::from_utf8(k).ok();
    }
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes)).ok()?;
    let k: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, k.as_bytes()).map_err(|e| eprintln!("history: keychain: {e}")).ok()?;
    Some(k)
}

fn open() -> rusqlite::Result<Connection> {
    let p = path().ok_or(rusqlite::Error::InvalidPath("no HOME".into()))?;
    let _ = std::fs::create_dir_all(p.parent().expect("has parent"));
    let c = Connection::open(&p)?;
    let k = key().ok_or(rusqlite::Error::InvalidQuery)?;
    // A raw 256-bit key, so SQLCipher skips its key derivation.
    c.pragma_update(None, "key", format!("x'{k}'"))?;
    c.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS user_inputs (
             id INTEGER PRIMARY KEY,
             created_at REAL NOT NULL,
             bundle_id TEXT NOT NULL,
             domain TEXT NOT NULL,
             text TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_user_inputs_bundle_created ON user_inputs (bundle_id, created_at);
         CREATE INDEX IF NOT EXISTS idx_user_inputs_created ON user_inputs (created_at);",
    )?;
    Ok(c)
}

fn with_db<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
    DB.with(|db| {
        let mut db = db.borrow_mut();
        let db = db.get_or_insert_with(|| open().map_err(|e| eprintln!("history: could not open: {e}")).ok());
        f(db.as_ref()?).map_err(|e| eprintln!("history: {e}")).ok()
    })
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// shortcut: app + window title, not the field: web fields move and grow while typing, so a
/// position in the key would save a text once per move. Two fields in one window share a session.
fn field_key(s: &FieldSnapshot) -> String {
    format!("{}|{}", s.bundle_id, s.window_title)
}

/// Note the field's current text; saves the previous field's text when the field changed.
pub fn observe(s: &FieldSnapshot) {
    let key = field_key(s);
    let text = s.text.trim();
    let start = text.char_indices().rev().nth(MAX_RECORD_CHARS - 1).map_or(0, |(i, _)| i);
    let text = text[start..].to_owned();
    let changed = SESSION.with(|cur| {
        let mut cur = cur.borrow_mut();
        if let Some(c) = cur.as_mut().filter(|c| c.key == key) {
            c.text = text.clone();
            return false;
        }
        let old = cur.replace(Session { bundle_id: s.bundle_id.clone(), domain: s.domain.clone(), key: key.clone(), text: text.clone() });
        if let Some(old) = old {
            save(old);
        }
        true
    });
    if changed {
        let ex = examples_from_db(&s.bundle_id, &text);
        EXAMPLES.with(|e| *e.borrow_mut() = (key, ex));
    }
}

/// Save the open field's text once the user is somewhere else: another window, even one where
/// completions don't run.
pub fn leave_if_moved(s: &FieldSnapshot) {
    let key = field_key(s);
    if SESSION.with(|c| c.borrow().as_ref().is_some_and(|c| c.key != key)) {
        flush();
    }
}

/// Save the open field's text (on quit, app switch, or `leave_if_moved`).
pub fn flush() {
    if let Some(s) = SESSION.with(|c| c.borrow_mut().take()) {
        save(s);
    }
}

fn save(s: Session) {
    if s.text.chars().count() < MIN_RECORD_CHARS {
        return;
    }
    eprintln!("history: saved {} chars from {}", s.text.chars().count(), s.bundle_id);
    with_db(|db| {
        // Coming back to the same field saves it again: drop the copies and drafts this text extends.
        db.execute(
            "DELETE FROM user_inputs WHERE bundle_id = ?1 AND substr(?2, 1, length(text)) = text",
            params![s.bundle_id, s.text],
        )?;
        db.execute(
            "INSERT INTO user_inputs (created_at, bundle_id, domain, text) VALUES (?1, ?2, ?3, ?4)",
            params![now(), s.bundle_id, s.domain, s.text],
        )
    });
}

/// Examples for the field `s` (fetched when the field was entered).
pub fn examples(s: &FieldSnapshot) -> Vec<String> {
    let key = field_key(s);
    EXAMPLES.with(|e| {
        let e = e.borrow();
        if e.0 == key { e.1.clone() } else { Vec::new() }
    })
}

/// Two longest and two newest from this app, one newest from another app (spec §7.4 mix).
fn examples_from_db(bundle_id: &str, current: &str) -> Vec<String> {
    let since = now() - WINDOW_SECS;
    let q = |sql: &str, n: usize| -> Vec<String> {
        with_db(|db| {
            let mut st = db.prepare_cached(sql)?;
            let rows = st.query_map(params![bundle_id, since, MIN_EXAMPLE_CHARS as i64, n as i64], |r| r.get(0))?;
            rows.collect()
        })
        .unwrap_or_default()
    };
    let base = "SELECT text FROM user_inputs WHERE created_at >= ?2 AND length(text) >= ?3 AND";
    let mut out: Vec<String> = Vec::new();
    for t in q(&format!("{base} bundle_id = ?1 ORDER BY length(text) DESC LIMIT ?4"), 2)
        .into_iter()
        .chain(q(&format!("{base} bundle_id = ?1 ORDER BY created_at DESC LIMIT ?4"), 3))
        .chain(q(&format!("{base} bundle_id != ?1 ORDER BY created_at DESC LIMIT ?4"), 1))
    {
        // Skip duplicates and the text the user is editing right now.
        if !out.contains(&t) && t != current && out.len() < 5 {
            out.push(t);
        }
    }
    out
}

/// Number of saved texts, for Settings.
pub fn count() -> u64 {
    // Don't create the database (and its Keychain key) just to say it's empty.
    if !path().is_some_and(|p| p.exists()) {
        return 0;
    }
    with_db(|db| db.query_row("SELECT count(*) FROM user_inputs", [], |r| r.get::<_, i64>(0))).map_or(0, |n| n as u64)
}

/// Delete all saved writing.
pub fn delete_all() {
    SESSION.with(|c| c.borrow_mut().take());
    EXAMPLES.with(|e| *e.borrow_mut() = (String::new(), Vec::new()));
    with_db(|db| db.execute_batch("DELETE FROM user_inputs; PRAGMA wal_checkpoint(TRUNCATE); VACUUM;"));
}
