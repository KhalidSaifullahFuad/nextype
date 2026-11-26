//! Prompt builder (spec §7.1, §8): Gemma 4 chat format, all context in the user turn, the model
//! turn pre-filled with the text before the caret so the model just continues it.
//! Platform-free: the caller supplies the context values and a token counter.

use crate::{heal_split, redact::redact};

/// Token budgets (spec §8.5).
pub const BEFORE_BUDGET: usize = 350;
pub const CONTEXT_BUDGET: usize = 150;
pub const INSTRUCTIONS_BUDGET: usize = 1000;
pub const CLIPBOARD_BUDGET: usize = 200;
pub const SCREEN_BUDGET: usize = 600;
pub const PAST_BUDGET: usize = 400;
pub const AFTER_BUDGET: usize = 200;
const WINDOW_TITLE_CHARS: usize = 100;

// Our own wording (spec §0: reference prompts are not copied).
const INTRO: &str = "I am writing a text at the end of this message, on my Mac. Some of the details below may not matter for it.";
const TASK: &str = "Continue the text I am writing from exactly where it stops. Keep its language, tone and style, write complete grammatical sentences, and reply with the continuation only: no preamble, no quotes, no placeholders.";
/// Mid-line (spec §8.6): the model sees the line with a gap marker and fills the gap. Gemma E2B
/// ignored the text after the caret when it was given as a separate section.
const GAP: &str = "[GAP]";
const MID_TASK: &str = "Continue my text from the cursor with only the words that fill [GAP], so the sentence reads naturally with what follows. Reply with those words only.";
/// The current line before the caret, repeated around the gap marker.
const GAP_LINE_BUDGET: usize = 100;

/// What the app knows about the moment of typing (spec §7.1). Empty values are left out.
#[derive(Debug, Default, Clone)]
pub struct Context {
    /// Local wall-clock time as seconds since 1970-01-01 00:00 (UTC offset already added).
    pub local_secs: i64,
    pub languages: Vec<String>,
    pub name: String,
    pub app: String,
    pub window_title: String,
    /// Host of the web page, if any.
    pub website: String,
    /// Earlier texts the user wrote (spec §7.4), best first.
    pub past: Vec<String>,
    /// Text read off the screen above the field (spec §7.2), top to bottom.
    pub screen: String,
    /// Text the user copied recently (spec §7.3), if that setting is on.
    pub clipboard: String,
    /// Text after the caret when it is mid-line (spec §8.6), else empty.
    pub after: String,
}

#[derive(Debug, PartialEq)]
pub struct Prompt {
    pub text: String,
    /// Typed partial word the model must spell out first (see `heal_split`).
    pub heal: String,
}

/// `count` is the model's token counter; it must grow with the length of its argument.
pub fn build(before: &str, ctx: &Context, instructions: &str, count: &dyn Fn(&str) -> usize) -> Prompt {
    let (kept, heal) = heal_split(fit_tail(before, BEFORE_BUDGET, count));
    let mut user = String::from(INTRO);
    let block = context_block(ctx, count);
    if !block.is_empty() {
        user.push_str("\n\n");
        user.push_str(&redact(&block));
    }
    // shortcut: custom instructions are the user's own text, so they are not redacted.
    let instructions = fit_head(instructions.trim(), INSTRUCTIONS_BUDGET, count);
    if !instructions.is_empty() {
        user.push_str("\n\nHow I want you to write:\n");
        user.push_str(instructions);
    }
    // Stable per field, so before the screen text. Each example gets an equal share of the budget.
    let past: Vec<&str> = ctx.past.iter().map(|t| fit_tail(t.trim(), PAST_BUDGET / ctx.past.len(), count)).filter(|t| !t.is_empty()).collect();
    if !past.is_empty() {
        user.push_str("\n\nOther texts I wrote earlier, to show my style:\n");
        user.push_str(&redact(&past.join("\n---\n")));
    }
    // The end is nearest the field, so a long screen keeps its bottom.
    let screen = fit_tail(ctx.screen.trim(), SCREEN_BUDGET, count);
    if !screen.is_empty() {
        user.push_str("\n\nText on screen above where I am typing:\n");
        user.push_str(&redact(screen));
    }
    // Volatile, so after the stable sections (spec §8.4); copied text can hold secrets.
    let clipboard = fit_head(ctx.clipboard.trim(), CLIPBOARD_BUDGET, count);
    if !clipboard.is_empty() {
        user.push_str("\n\nText I copied recently:\n");
        user.push_str(&redact(clipboard));
    }
    // Mid-line: right before the task, nearest what it explains. shortcut: this section changes
    // with each mid-line keystroke, so the model turn after it is prefilled again (~250 ms);
    // move the gap line after the before-text if that gets noticeable.
    let after = fit_head(ctx.after.trim_end(), AFTER_BUDGET, count);
    if !after.is_empty() {
        let line = before.rsplit('\n').next().unwrap_or("");
        let line = fit_tail(line, GAP_LINE_BUDGET, count);
        user.push_str("\n\nMy text, with [GAP] where my cursor is:\n");
        user.push_str(&redact(&format!("{line}{GAP}{after}")));
    }
    user.push_str("\n\n");
    user.push_str(if after.is_empty() { TASK } else { MID_TASK });
    Prompt { text: format!("<|turn>user\n{user}<turn|>\n<|turn>model\n{kept}"), heal: heal.to_owned() }
}

/// Context lines in order of importance; the least important are dropped to fit the budget.
fn context_block(c: &Context, count: &dyn Fn(&str) -> usize) -> String {
    let mut lines = Vec::new();
    if c.local_secs != 0 {
        let (date, time) = format_local(c.local_secs);
        lines.push(format!("Date and time: {date}, {time}"));
    }
    let title: String = c.window_title.trim().chars().take(WINDOW_TITLE_CHARS).collect();
    for (label, value) in [
        ("My languages", c.languages.join(", ")),
        ("My name", c.name.trim().to_owned()),
        ("App I am typing in", c.app.trim().to_owned()),
        ("Website", c.website.trim().to_owned()),
        ("Window", title),
    ] {
        if !value.is_empty() {
            lines.push(format!("{label}: {value}"));
        }
    }
    while !lines.is_empty() && count(&lines.join("\n")) > CONTEXT_BUDGET {
        lines.pop();
    }
    lines.join("\n")
}

/// Longest suffix of `s` within `budget` tokens, starting at a sentence start if one is near the
/// cut, else at a word start. The start only moves when the budget is exceeded, so the prompt
/// prefix (and the KV cache) stays stable while typing.
pub fn fit_tail<'a>(s: &'a str, budget: usize, count: &dyn Fn(&str) -> usize) -> &'a str {
    if count(s) <= budget {
        return s;
    }
    let starts: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    let (mut lo, mut hi) = (0, starts.len()); // the empty suffix always fits
    while lo < hi {
        let mid = (lo + hi) / 2;
        if count(&s[starts[mid]..]) <= budget { hi = mid } else { lo = mid + 1 }
    }
    snap_forward(s, starts.get(lo).copied().unwrap_or(s.len()))
}

/// Longest prefix of `s` within `budget` tokens.
fn fit_head<'a>(s: &'a str, budget: usize, count: &dyn Fn(&str) -> usize) -> &'a str {
    if count(s) <= budget {
        return s;
    }
    let ends: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    let (mut lo, mut hi) = (0, ends.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if count(&s[..ends[mid - 1]]) <= budget { lo = mid } else { hi = mid - 1 }
    }
    s[..ends.get(lo).copied().unwrap_or(s.len())].trim_end()
}

fn snap_forward(s: &str, start: usize) -> &str {
    let rest = &s[start..];
    // A sentence or line start within the first quarter of what remains.
    let window = rest.len() / 4;
    let mut prev_end = false;
    for (i, c) in rest.char_indices().take_while(|&(i, _)| i <= window) {
        if prev_end && c.is_whitespace() {
            return rest[i + c.len_utf8()..].trim_start();
        }
        prev_end = matches!(c, '.' | '!' | '?' | '\n');
    }
    if start == 0 || s[..start].ends_with(char::is_whitespace) {
        return rest;
    }
    rest.find(char::is_whitespace).map_or(rest, |i| rest[i..].trim_start())
}

/// "Wednesday, November 18, 2026" and "3:45 PM" for local seconds, rounded down to 15 minutes
/// so the prompt prefix stays stable for KV reuse. shortcut: English names and a 12-hour clock.
pub fn format_local(local_secs: i64) -> (String, String) {
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
        "November", "December",
    ];
    let secs = local_secs - local_secs.rem_euclid(900);
    let days = secs.div_euclid(86_400);
    let minutes = secs.rem_euclid(86_400) / 60;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (h, m) = (minutes / 60, minutes % 60);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    (
        format!("{}, {} {day}, {year}", DAYS[(days + 4).rem_euclid(7) as usize], MONTHS[month as usize - 1]),
        format!("{h12}:{m:02} {}", if h < 12 { "AM" } else { "PM" }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> usize {
        s.split_whitespace().count()
    }

    fn sample() -> Context {
        Context {
            local_secs: 1_795_016_850, // 2026-11-18 15:47:30
            languages: vec!["en-US".into(), "de-DE".into()],
            name: "Sam Example".into(),
            app: "TextEdit".into(),
            window_title: "Notes.txt".into(),
            website: String::new(),
            past: Vec::new(),
            screen: String::new(),
            clipboard: String::new(),
            after: String::new(),
        }
    }

    #[test]
    fn date_is_rounded_and_named() {
        assert_eq!(format_local(1_795_016_850), ("Wednesday, November 18, 2026".into(), "3:45 PM".into()));
        assert_eq!(format_local(0), ("Thursday, January 1, 1970".into(), "12:00 AM".into()));
        assert_eq!(format_local(951_868_799).0, "Tuesday, February 29, 2000");
        assert_eq!(format_local(951_868_800).0, "Wednesday, March 1, 2000");
    }

    #[test]
    fn golden_prompt() {
        let p = build("We should sch", &sample(), "Be concise.", &words);
        assert_eq!(p.heal, " sch");
        assert_eq!(
            p.text,
            "<|turn>user\n\
             I am writing a text at the end of this message, on my Mac. Some of the details below may not matter for it.\n\n\
             Date and time: Wednesday, November 18, 2026, 3:45 PM\n\
             My languages: en-US, de-DE\n\
             My name: Sam Example\n\
             App I am typing in: TextEdit\n\
             Window: Notes.txt\n\n\
             How I want you to write:\n\
             Be concise.\n\n\
             Continue the text I am writing from exactly where it stops. Keep its language, tone and style, write complete grammatical sentences, and reply with the continuation only: no preamble, no quotes, no placeholders.\
             <turn|>\n<|turn>model\nWe should"
        );
    }

    #[test]
    fn empty_context_leaves_out_sections() {
        let p = build("Hello there ", &Context::default(), "  ", &words);
        assert!(!p.text.contains("Date and time") && !p.text.contains("How I want"));
        assert!(p.text.ends_with("<|turn>model\nHello there"));
        assert_eq!(p.heal, " ");
    }

    #[test]
    fn context_is_redacted_and_budgeted() {
        let mut c = sample();
        c.window_title = "token=abc password: hunter2".into();
        let p = build("Hi all", &c, "", &words);
        assert!(p.text.contains("Window: token=abc password: [REDACTED]"));
        c.window_title = "w ".repeat(200); // cut to 100 chars, still fits
        assert!(build("Hi all", &c, "", &words).text.contains("App I am typing in: TextEdit"));
        c.website = "mail.google.com".into();
        assert!(build("Hi all", &c, "", &words).text.contains("App I am typing in: TextEdit\nWebsite: mail.google.com\n"));
        c.name = "n ".repeat(300); // over budget: the least important lines go first
        let p = build("Hi all", &c, "", &words);
        assert!(!p.text.contains("App I am typing in"));
    }

    #[test]
    fn mid_line_adds_the_text_after_and_its_task() {
        let c = Context { after: " tomorrow at noon.".into(), ..Default::default() };
        let p = build("Let us meet", &c, "", &words);
        assert!(p.text.contains("where my cursor is:\nLet us meet[GAP] tomorrow at noon.\n\nContinue my text from the cursor"));
        assert!(p.text.ends_with("<|turn>model\nLet us"));
        assert!(!build("Let us meet", &Context::default(), "", &words).text.contains("[GAP]"));
    }

    #[test]
    fn before_text_keeps_its_end_within_budget() {
        let text = (1..=1000).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
        let cut = fit_tail(&text, BEFORE_BUDGET, &words);
        assert!(words(cut) <= BEFORE_BUDGET && words(cut) > BEFORE_BUDGET - 5);
        assert!(cut.ends_with("w1000") && cut.starts_with('w'));
        assert_eq!(fit_tail("short text", 10, &words), "short text");
        assert_eq!(fit_tail("", 3, &words), "");
        assert_eq!(fit_tail("aaaa bbbb", 0, &words), "");
    }

    #[test]
    fn cut_prefers_a_sentence_start() {
        let text = "One two three. Four five six seven eight nine ten eleven twelve";
        assert_eq!(fit_tail(text, 11, &words), "Four five six seven eight nine ten eleven twelve");
        // No sentence end near the cut: starts at a word, never mid-word.
        assert_eq!(fit_tail("aa bb cc dd ee ff", 3, &words), "dd ee ff");
    }

    #[test]
    fn clipboard_comes_after_instructions_redacted_and_cut() {
        let c = Context { clipboard: format!("password: hunter2 {}", "word ".repeat(500)), ..Context::default() };
        let p = build("Hi all", &c, "Be brief.", &words);
        let (i, k) = (p.text.find("Be brief.").unwrap(), p.text.find("Text I copied recently:").unwrap());
        assert!(i < k && k < p.text.find(TASK).unwrap());
        assert!(!p.text.contains("hunter2"));
        assert!(words(&p.text) < CLIPBOARD_BUDGET + 120);
        assert!(!build("Hi all", &Context::default(), "", &words).text.contains("copied"));
    }

    #[test]
    fn screen_keeps_its_bottom_and_comes_before_clipboard() {
        let screen = format!("{} Last line near the field.", "early ".repeat(800));
        let c = Context { screen, clipboard: "copied".into(), ..Context::default() };
        let p = build("Hi all", &c, "", &words);
        let (s, k) = (p.text.find("Text on screen above").unwrap(), p.text.find("Text I copied").unwrap());
        assert!(s < k && p.text.contains("Last line near the field."));
        assert!(words(&p.text) < SCREEN_BUDGET + 120);
    }

    #[test]
    fn past_writing_shares_its_budget_and_comes_before_screen() {
        let long = "word ".repeat(1000);
        let c = Context { past: vec![long.clone(), long, "Short one.".into()], screen: "On screen now.".into(), ..Context::default() };
        let p = build("Hi all", &c, "", &words);
        let (e, s) = (p.text.find("Other texts I wrote").unwrap(), p.text.find("Text on screen above").unwrap());
        assert!(e < s && p.text.contains("Short one."));
        assert!(words(&p.text) < PAST_BUDGET + 120);
    }

    #[test]
    fn instructions_are_cut_to_budget() {
        let long = "word ".repeat(2000);
        let p = build("Hi all", &Context::default(), &long, &words);
        assert_eq!(p.text.matches("word").count(), INSTRUCTIONS_BUDGET);
    }
}
