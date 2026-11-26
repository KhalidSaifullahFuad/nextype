//! Platform-free NexType logic: field snapshots, the completion gate (spec §5.2), prompt
//! building (§8) and secret redaction (§7.5).

pub mod prompt;
pub mod redact;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// What the focused text field looks like right now (spec §4.3).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FieldSnapshot {
    /// Process that owns the field (where accepted text is typed).
    pub pid: i32,
    pub bundle_id: String,
    pub role: String,
    pub subrole: String,
    /// Localized name of the owning app, and the title of the field's window.
    pub app_name: String,
    pub window_title: String,
    /// Host of the web page the field is in (`mail.google.com`), empty outside web views.
    pub domain: String,
    pub text: String,
    /// Caret (selection start) in UTF-16 units, as Accessibility reports it.
    pub caret_utf16: usize,
    pub selection_len: usize,
    pub frame: Rect,
    pub caret_rect: Option<Rect>,
    /// Font of the character before the caret: (PostScript name, point size).
    pub font: Option<(String, f64)>,
    /// On-screen widths of the text before the caret, read only when the app reports no font.
    pub width_sample: Option<WidthSample>,
    /// x where the lines of the caret's paragraph start (its first character's left edge).
    pub text_left: Option<f64>,
}

/// The last few characters before the caret and how wide they are on screen (and their first
/// `head_chars` characters), so the overlay can pick a matching font when the app doesn't name one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WidthSample {
    pub text: String,
    pub width: f64,
    pub head_chars: usize,
    pub head_width: f64,
}

impl FieldSnapshot {
    pub fn before(&self) -> &str {
        &self.text[..utf16_to_byte(&self.text, self.caret_utf16)]
    }

    pub fn after(&self) -> &str {
        &self.text[utf16_to_byte(&self.text, self.caret_utf16 + self.selection_len)..]
    }

    /// Whether text follows the caret on its line (a mid-line completion, spec §8.6).
    pub fn mid_line(&self) -> bool {
        // Web editors often leave a trailing space or nbsp after the caret: that's still line end.
        !self.after().lines().next().unwrap_or("").trim().is_empty()
    }

    /// Left and right edges the ghost wraps between in a multi-line field (spec §11.2),
    /// taking the right inset equal to the left one. `None`: single-line field, unknown
    /// geometry, or no room after the caret.
    pub fn wrap_edges(&self) -> Option<(f64, f64)> {
        let (caret, f) = (self.caret_rect?, self.frame);
        if self.role != "AXTextArea" || f.w <= 0.0 {
            return None;
        }
        let left = self.text_left.unwrap_or(f.x).clamp(f.x, caret.x);
        let right = f.x + f.w - (left - f.x);
        // Too little room (or a field that hugs its text, like VS Code's line-sized editor area):
        // wrapping would push all but a letter or two to the next line. shortcut: one unwrapped
        // line instead; the pill list is M8.
        (right - caret.x >= MIN_WRAP_ROOM).then_some((left, right))
    }
}

/// Byte offset of a UTF-16 index, clamped to the string and snapped to a char boundary.
pub fn utf16_to_byte(s: &str, idx: usize) -> usize {
    let mut units = 0;
    for (byte, c) in s.char_indices() {
        if units >= idx {
            return byte;
        }
        units += c.len_utf16();
    }
    s.len()
}

/// Apps where we never complete or read text (spec §17.2): password managers and ourselves.
pub const EXCLUDED_APPS: &[&str] = &[
    "dev.nextype.NexType",
    "com.1password.1password",
    "com.agilebits.onepassword",
    "com.apple.Passwords",
    "com.lastpass.lastpassmacdesktop",
    "com.lastpass.LastPass",
    "com.dashlane.dashlanephonefinal",
    "com.dashlane.Dashlane",
    "com.bitwarden.desktop",
    "com.callpod.keepermac.lite",
    "com.keepersecurity.passwordmanager",
    "com.sibersystems.RoboFormMac",
    "com.nordsec.nordpass",
    "in.sinew.Enpass-Desktop",
    "com.ascendo.DataVaultMac",
    "me.proton.pass.electron",
    "com.mseven.msecuremac",
    "com.symantec.NortonPasswordManager.combined",
    "org.keepassx.keepassx",
    "org.keepassxc.keepassxc",
    "com.selznick.PasswordWallet",
    "com.outercorner.Secrets-setapp",
    "com.outercorner.Secrets",
    "com.outercorner.osx.Secrets",
    "com.outercorner.Secrets-safari",
];

/// Apps where completions start off; unlike `EXCLUDED_APPS` the user can turn them on (spec §17.2,
/// Cotypist's off-by-default set, plus launchers and terminals). Search boxes, shells and tools
/// where a sentence completion is in the way.
pub const OFF_BY_DEFAULT: &[&str] = &[
    // Launchers and search
    "com.apple.Spotlight",
    "com.apple.campo", // Spotlight's search panel on macOS 27
    "com.raycast.macos",
    "com.runningwithcrayons.Alfred",
    "at.obdev.LaunchBar",
    // System tools
    "com.apple.finder",
    "com.apple.Preview",
    "com.apple.systempreferences",
    "com.apple.iCal",
    // Terminals (their prompts aren't prose)
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "com.mitchellh.ghostty",
    "dev.warp.Warp-Stable",
    "com.madda.ghostex.host",
    // Code and design tools (VS Code stays on: users write prose in its chat)
    "com.apple.dt.Xcode",
    "dev.zed.Zed",
    "com.sublimetext.4",
    "com.figma.Desktop",
    "com.microsoft.Excel",
    "pl.maketheweb.cleanshotx",
];

/// Chromium browsers: they build their full AX tree (with text positions) only after
/// `AXEnhancedUserInterface` is set, and reject `AXManualAccessibility` (spec §4.2; tested on
/// Chrome 2026-11-19: Set returns -25208 yet takes effect). Not set elsewhere: it slows window
/// animations and window managers.
pub const NEEDS_ENHANCED_UI: &[&str] = &[
    "com.google.Chrome",
    "com.google.Chrome.beta",
    "com.google.Chrome.dev",
    "com.google.Chrome.canary",
    "org.chromium.Chromium",
    "com.brave.Browser",
    "com.microsoft.edgemac",
    "com.microsoft.edgemac.Beta",
    "com.microsoft.edgemac.Dev",
    "company.thebrowser.Browser",
    "company.thebrowser.dia",
    "com.vivaldi.Vivaldi",
    "com.operasoftware.Opera",
    "ai.perplexity.comet",
];

const TEXT_ROLES: &[&str] = &["AXTextArea", "AXTextField", "AXComboBox"];

/// Cotypist's field-size thresholds (spec §5.2); enough existing text bypasses them.
const MIN_W: f64 = 300.0;
const MIN_H: f64 = 32.0;
const MIN_AREA: f64 = 6400.0;
const SIZE_BYPASS_CHARS: usize = 30;
const MIN_CHARS: usize = 3;
/// Least space after the caret (points) worth wrapping the ghost into.
const MIN_WRAP_ROOM: f64 = 80.0;

/// `Ok` if a completion may be shown for this snapshot, else the reason it may not.
pub fn gate(s: &FieldSnapshot) -> Result<(), &'static str> {
    if EXCLUDED_APPS.contains(&s.bundle_id.as_str()) {
        return Err("excluded app");
    }
    if s.subrole == "AXSecureTextField" {
        return Err("secure field");
    }
    // Search boxes (Spotlight, app and web search fields) want queries, not sentences (spec §4.2).
    if s.subrole == "AXSearchField" {
        return Err("search field");
    }
    if !TEXT_ROLES.contains(&s.role.as_str()) {
        return Err("not a text field");
    }
    if s.selection_len > 0 {
        return Err("text selected");
    }
    if s.caret_rect.is_none() {
        return Err("no caret rect");
    }
    let f = s.frame;
    let big_enough = f.w >= MIN_W && f.h >= MIN_H && f.w * f.h >= MIN_AREA;
    if !big_enough && s.text.chars().count() < SIZE_BYPASS_CHARS {
        return Err("field too small");
    }
    if s.before().chars().count() < MIN_CHARS {
        return Err("too little text");
    }
    // Mid-line is fine between words (spec §8.6), but not inside one.
    if s.after().starts_with(char::is_alphanumeric) {
        return Err("caret mid-word");
    }
    Ok(())
}

/// Longest partial word we heal (spec §9.4 `maxPrefixLength`).
const MAX_HEAL_CHARS: usize = 20;

/// Split `before` for typed-prefix healing (spec §9.4): the trailing partial word, with the
/// space in front of it, is cut from the prompt and the model must spell it out again.
/// Returns `(prompt_text, heal)`; `heal` is `" "` after a trailing space, `""` after a newline.
pub fn heal_split(before: &str) -> (&str, &str) {
    let word_start = before.trim_end_matches(|c: char| !c.is_whitespace()).len();
    let start = if before[..word_start].ends_with(' ') { word_start - 1 } else { word_start };
    if before[start..].chars().count() > MAX_HEAL_CHARS {
        return (before, "");
    }
    before.split_at(start)
}

/// Last `max_chars` characters of `s`.
pub fn tail(s: &str, max_chars: usize) -> &str {
    let start = s.char_indices().rev().nth(max_chars.saturating_sub(1)).map_or(0, |(i, _)| i);
    if max_chars == 0 { "" } else { &s[start..] }
}

/// Spec §10: a ghost ends with its sentence. Byte length of `s` through the first sentence end
/// (`.!?…` plus closing quotes/brackets) that follows a word and is followed by whitespace;
/// `None` while the sentence is still open. A leading `. ` (closing the user's sentence) is kept.
pub fn sentence_end(s: &str) -> Option<usize> {
    let mut seen_word = false;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_alphanumeric() {
            seen_word = true;
        } else if seen_word && matches!(c, '.' | '!' | '?' | '…') {
            let mut end = i + c.len_utf8();
            while let Some(&(j, q)) = chars.peek().filter(|(_, q)| ".!?…\"')]”’".contains(*q)) {
                end = j + q.len_utf8();
                chars.next();
            }
            if chars.peek().is_some_and(|(_, w)| w.is_whitespace()) {
                return Some(end);
            }
        }
    }
    None
}

/// Host part of a URL (`https://mail.google.com/x` → `mail.google.com`), lowercased; empty for
/// URLs without one (`file:`, `about:`, `data:`).
pub fn host_of(url: &str) -> String {
    let Some((_, rest)) = url.split_once("://") else { return String::new() };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = if host.starts_with('[') { host.split(']').next().map_or("", |h| &h[1..]) } else { host.split(':').next().unwrap_or("") };
    host.to_ascii_lowercase()
}

/// Whether an override for `key` (`google.com`) applies on `host` (`mail.google.com`):
/// the same host or a subdomain of it.
pub fn domain_matches(key: &str, host: &str) -> bool {
    !key.is_empty() && (host == key || host.strip_suffix(key).is_some_and(|h| h.ends_with('.')))
}

/// Where `ghost` starts copying `source` (a context line such as the window title): the start of
/// the first run of `min`+ characters that also appears in `source`. Small models sometimes
/// continue the text with a context line instead of the user's sentence.
pub fn copy_cut(ghost: &str, source: &str, min: usize) -> Option<usize> {
    if source.chars().count() < min {
        return None;
    }
    let starts: Vec<usize> = ghost.char_indices().map(|(i, _)| i).chain([ghost.len()]).collect();
    (0..starts.len().saturating_sub(min)).map(|k| (starts[k], starts[k + min])).find_map(|(a, b)| {
        let chunk = &ghost[a..b];
        (!chunk.trim().is_empty() && source.contains(chunk)).then_some(a)
    })
    // Also drop the separators the copy came with (" • Untitled…" → nothing left).
    .map(|a| ghost[..a].trim_end_matches(|c: char| !c.is_alphanumeric()).len())
}

/// Mid-line (spec §8.6): `ghost` with the model's gap markup (`**word**`, `[word]`) removed, cut where it runs into
/// `after` (the rest of the caret's line), and with any tail that could be the start of `after`
/// held back, so a rejoin never flashes on screen. `done` once the rejoin is certain.
pub fn mid_line_ghost(ghost: &str, after: &str) -> (String, bool) {
    let g = ghost.replace(['*', '[', ']'], "");
    if let Some(end) = copy_cut(&g, after, 6) {
        return (g[..end].trim_end().to_owned(), true);
    }
    let hold = (1..=g.len().min(after.len()))
        .rev()
        .find(|&k| g.is_char_boundary(g.len() - k) && after.is_char_boundary(k) && g.ends_with(&after[..k]))
        .unwrap_or(0);
    let mut kept = g[..g.len() - hold].to_owned();
    // The field already has the space before the next word.
    if after.starts_with(char::is_whitespace) {
        kept.truncate(kept.trim_end().len());
    }
    (kept, false)
}

/// Most words a mid-line ghost may have (spec §8.6 leaves the length open): it fills a gap, and
/// small models drift into chatter after a few words.
pub const MID_LINE_WORDS: usize = 5;

/// Byte length of the first `n` words of `s` (with their leading spaces), `None` if it has fewer.
pub fn word_cut(s: &str, n: usize) -> Option<usize> {
    let mut words = 0;
    let mut in_word = false;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            words += 1;
            if words > n {
                return Some(s[..i].trim_end().len());
            }
        }
    }
    None
}

/// Start of the first run of 4+ identical non-alphanumeric characters (`....`, `----`, blank
/// lines): small models sometimes loop on one, and the ghost must end before it.
pub fn repeat_cut(s: &str) -> Option<usize> {
    let mut run = (0, '\0', 0); // (start, char, length)
    for (i, c) in s.char_indices() {
        run = if c == run.1 { (run.0, c, run.2 + 1) } else { (i, c, 1) };
        if run.2 == 4 && !c.is_alphanumeric() {
            return Some(run.0);
        }
    }
    None
}

/// Next-word accept (spec §12.1): leading whitespace, the word, and one trailing space if present.
pub fn next_word(s: &str) -> &str {
    let lead = s.len() - s.trim_start().len();
    let end = s[lead..].find(char::is_whitespace).map_or(s.len(), |i| lead + i);
    if s[end..].starts_with(' ') { &s[..end + 1] } else { &s[..end] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str, caret: usize) -> FieldSnapshot {
        FieldSnapshot {
            bundle_id: "com.apple.TextEdit".into(),
            role: "AXTextArea".into(),
            text: text.into(),
            caret_utf16: caret,
            frame: Rect { x: 0.0, y: 0.0, w: 600.0, h: 400.0 },
            caret_rect: Some(Rect::default()),
            ..Default::default()
        }
    }

    #[test]
    fn utf16_split_handles_non_bmp() {
        // "a😀b": 😀 is 2 UTF-16 units, 4 bytes.
        assert_eq!(utf16_to_byte("a😀b", 1), 1);
        assert_eq!(utf16_to_byte("a😀b", 3), 5);
        assert_eq!(utf16_to_byte("a😀b", 99), 6);
        let f = field("a😀b", 3);
        assert_eq!((f.before(), f.after()), ("a😀", "b"));
    }

    #[test]
    fn mid_line_ghost_stops_at_the_rejoin() {
        let after = " by Friday.";
        assert_eq!(mid_line_ghost(" **rep", after), (" rep".into(), false));
        assert_eq!(mid_line_ghost(" **report** by", after), (" report".into(), false));
        assert_eq!(mid_line_ghost(" **report** by Friday.", after), (" report".into(), true));
        assert_eq!(mid_line_ghost(" to discuss it.", " to discuss it."), ("".into(), true));
        assert_eq!(mid_line_ghost(" great", " and more"), (" great".into(), false));
        assert_eq!(mid_line_ghost(" [option]", " is too"), (" option".into(), false));
    }

    #[test]
    fn word_cut_keeps_n_words() {
        assert_eq!(word_cut(" sales report for this week", 2), Some(" sales report".len()));
        assert_eq!(word_cut(" sales report", 2), None);
        assert_eq!(word_cut("", 1), None);
    }

    #[test]
    fn gate_rules() {
        assert_eq!(gate(&field("Hello there", 11)), Ok(()));
        assert_eq!(gate(&field("Hello there\nnext", 11)), Ok(()));
        assert_eq!(gate(&field("Hello there", 5)), Ok(()));
        assert_eq!(gate(&field("Hello there", 3)), Err("caret mid-word"));
        assert!(field("Hello there", 5).mid_line());
        assert!(!field("Hello there\nnext", 11).mid_line());
        assert!(!field("Hello there \u{a0}", 11).mid_line());
        assert_eq!(gate(&field("Hi", 2)), Err("too little text"));

        let mut f = field("Hello there", 11);
        f.subrole = "AXSecureTextField".into();
        assert_eq!(gate(&f), Err("secure field"));
        f.subrole = "AXSearchField".into();
        assert_eq!(gate(&f), Err("search field"));

        let mut f = field("Hello there", 11);
        f.bundle_id = "com.1password.1password".into();
        assert_eq!(gate(&f), Err("excluded app"));

        let mut f = field("Hello there", 11);
        f.frame = Rect { x: 0.0, y: 0.0, w: 200.0, h: 22.0 };
        assert_eq!(gate(&f), Err("field too small"));
        f.text = "x".repeat(40);
        f.caret_utf16 = 40;
        assert_eq!(gate(&f), Ok(()));
    }

    #[test]
    fn heal_split_cases() {
        assert_eq!(heal_split("we should sch"), ("we should", " sch"));
        assert_eq!(heal_split("we should "), ("we should", " "));
        assert_eq!(heal_split("we should"), ("we", " should"));
        assert_eq!(heal_split("Hello,\n"), ("Hello,\n", ""));
        assert_eq!(heal_split("Hello,\nWor"), ("Hello,\n", "Wor"));
        assert_eq!(heal_split("x"), ("", "x"));
        let long = format!("see {}", "a".repeat(25));
        assert_eq!(heal_split(&long), (long.as_str(), ""));
    }

    #[test]
    fn sentence_end_cases() {
        let s = "need to consider this. It could";
        assert_eq!(sentence_end(s).map(|n| &s[..n]), Some("need to consider this."));
        let s = ". How are you? Fine";
        assert_eq!(sentence_end(s).map(|n| &s[..n]), Some(". How are you?"));
        let s = "he said \"yes.\" Then";
        assert_eq!(sentence_end(s).map(|n| &s[..n]), Some("he said \"yes.\""));
        assert_eq!(sentence_end(" in more detail."), None);
        assert_eq!(sentence_end("costs 3.5 dollars"), None);
    }

    #[test]
    fn wrap_edges_inside_field() {
        let mut f = field("Hello there", 11);
        f.frame = Rect { x: 182.0, y: 155.0, w: 586.0, h: 382.0 };
        f.caret_rect = Some(Rect { x: 334.0, y: 187.0, w: 0.0, h: 14.0 });
        f.text_left = Some(187.0);
        assert_eq!(f.wrap_edges(), Some((187.0, 763.0)));
        f.caret_rect = Some(Rect { x: 770.0, ..f.caret_rect.unwrap() });
        assert_eq!(f.wrap_edges(), None);
        // VS Code: the field is exactly as wide as its line, caret at its right edge.
        f.frame = Rect { x: 352.0, y: -1012.0, w: 333.0, h: 18.0 };
        f.caret_rect = Some(Rect { x: 683.0, y: -1010.0, w: 0.0, h: 14.0 });
        f.text_left = Some(352.0);
        assert_eq!(f.wrap_edges(), None);
        f.role = "AXTextField".into();
        assert_eq!(f.wrap_edges(), None);
    }

    #[test]
    fn tail_and_next_word() {
        assert_eq!(tail("héllo", 3), "llo");
        assert_eq!(tail("hi", 10), "hi");
        assert_eq!(tail("hi", 0), "");
        assert_eq!(next_word(" discuss the plan"), " discuss ");
        assert_eq!(next_word("cuss the"), "cuss ");
        assert_eq!(next_word(" end."), " end.");
        assert_eq!(next_word("a\nb"), "a");
    }

    #[test]
    fn repeat_cut_stops_loops() {
        assert_eq!(repeat_cut(" today....... more"), Some(6));
        assert_eq!(repeat_cut("................"), Some(0));
        assert_eq!(repeat_cut(" well... ok"), None); // an ellipsis is fine
        assert_eq!(repeat_cut(" hmmmm"), None); // letters are left alone
        assert_eq!(repeat_cut("a\n\n\n\nb"), Some(1));
    }

    #[test]
    fn hosts_and_domain_matching() {
        assert_eq!(host_of("https://Mail.Google.com/mail/u/0/#inbox"), "mail.google.com");
        assert_eq!(host_of("http://user:pw@localhost:8000/x?y"), "localhost");
        assert_eq!(host_of("http://[::1]:8000/"), "::1");
        assert_eq!(host_of("file:///tmp/a.html"), "");
        assert_eq!(host_of("about:blank"), "");
        assert!(domain_matches("google.com", "mail.google.com"));
        assert!(domain_matches("google.com", "google.com"));
        assert!(!domain_matches("google.com", "notgoogle.com"));
        assert!(!domain_matches("", "google.com"));
    }

    #[test]
    fn copy_cut_finds_context_copies() {
        let title = "Untitled-1 — GTM-Express-React";
        assert_eq!(copy_cut(" • Untitled-1 — GTM", title, 8), Some(0));
        assert_eq!(copy_cut(" ok. Untitled-1 — GTM", title, 8), Some(3));
        assert_eq!(copy_cut(" and then we left", title, 8), None);
        assert_eq!(copy_cut(" React is nice", title, 8), None); // short overlaps are fine
        assert_eq!(copy_cut(" anything", "short", 8), None);
    }
}
