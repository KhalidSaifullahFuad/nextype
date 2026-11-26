# NexType tasks

The plan for NexType, the macOS local-AI inline autocomplete, in one place: what is done, what is next, and what comes later. The detail lives in `docs/RUST-CLONE-SPEC.md` (§23 milestone results, §2 feature list). `HANDOFF.md` holds session notes and test methods.

Updated: 2026-11-19.

## Done

### M0. Engine spike ✅
- [x] Load the Gemma 4 E2B GGUF on Metal, tokenize, decode greedily, cancel mid-run
- [x] Reuse the KV-cache prefix between keystrokes (p50 first token about 40 ms)

### M1. Reading text fields ✅
- [x] Accessibility observer per front app, with a debounce
- [x] Field snapshot: text, caret, caret rectangle, frame, role, window title
- [x] Gate: excluded apps, password fields, size, minimum text, caret at the end of a line

### M2. Ghost text and accepting it ✅
- [x] Overlay panel in the field's font, top-aligned to the caret line, wrapping like the field
- [x] Tab accepts the next word, the key above Tab accepts the whole suggestion, Escape hides it
- [x] Insertion with Unicode key events, confirmed by reading the field again after 1 s
- [x] Quick typed-prefix healing (no more words broken mid-word)
- [x] Infer-from-ghost: typing that matches the suggestion advances it without a model run
- [x] Sentence trim

### M3. Prompt v1 ✅
- [x] Gemma 4 chat layout in our own wording
- [x] Context lines: date and time, languages, name, app, window title, website
- [x] Token budgets; secret redaction (13 patterns); custom instructions

### M4. App shell ✅
- [x] SwiftUI menu bar app over the Rust core (uniffi)
- [x] Onboarding: Accessibility, then the model download (resume, sha256 check, retries, cancel, disk check)
- [x] Settings window: searchable sidebar with General, Suggestions, Keyboard, Context sources, Learn my style, Apps & websites, Permissions, Model and About; same look as the Launchpad project's settings. Reopening the app shows it
- [x] Model choice: Gemma 4 E4B/E2B, Gemma 3 4B/1B, Qwen 3 1.7B; recommended by memory, switched live, chat format read from the file
- [x] Menu: today's stats, on/off, pause 1 h, turn off in this app, this app's Tab shortcut, or this website
- [x] Stats file; paste fallback when typed insertion fails

### M5. Browsers and editors ✅
- [x] Chromium browsers (`AXEnhancedUserInterface`), switched back off when NexType quits
- [x] Electron apps (`AXManualAccessibility`): Teams, VS Code editor
- [x] Safari; caret from text markers when range bounds fail
- [x] Website detection and per-site overrides
- [x] Ghost trims: repeated punctuation, copies of the window title, no wrapping without room
- [x] Fallback to NSWorkspace when the system-wide "focused app" query fails
- [x] Chrome's non-breaking space after an accepted word no longer drops the suggestion
- [x] Off by default: launchers (incl. macOS 27 Spotlight `com.apple.campo`), terminals, code and design tools, Finder, Preview and similar; search fields always skipped

### M6. More context ✅
- [x] Copied text from the last 2 minutes (skips password-manager and transient items)
- [x] Screen text: the window above the field, read with Vision OCR (needs Screen Recording)
- [x] Past writing: encrypted SQLCipher store (key in the Keychain), up to 5 style examples per field, count and Delete All in Settings
- [x] All three are off by default, each with a toggle in Settings

## Next: M7. Suggestion quality

- [x] Mid-line suggestions: between words, right after typing; the prompt shows the line with a [GAP] marker, the ghost (≤ 5 words) covers the text after the caret and stops where it rejoins it (spec §8.6)
- [ ] Beams and ranking instead of one greedy path; stronger typed-prefix healing (§9.4)
- [ ] Typo hold-back: no suggestion while the last word looks misspelled; offer the correction (§10)
- [ ] Sentence trim that doesn't stop at abbreviations like "e.g."
- [ ] Tune the suggestion lengths (short / medium / long are guesses: 8 / 16 / 32 tokens)
- [ ] Prompt: budget redistribution and fill target, dialect line, scaffolding-marker demotion
- [ ] Measure the acceptance rate in daily use

## Later: M8. Polish

- [ ] Advanced KV reuse: splicing, parked sequences, prewarm on focus (§9.3); this would also make the first suggestion after a new screen read faster
- [ ] Keystroke speculation: start on key-down (§6.2)
- [ ] Pill list for fields with no room after the caret, and for right-to-left text
- [ ] Text mirror overlay for apps with bad caret positions
- [ ] Ghost that follows window moves and scrolling
- [ ] N-gram personalization booster from past writing (§13.3)
- [ ] Auto-update (Sparkle-style)
- [ ] Secure Input notice that names the app holding it
- [ ] Offer to turn off macOS's own inline prediction
- [ ] Battery mode, word alternatives, emoji shortcodes, URL scheme for settings, opt-in crash reports

## Open issues and gaps

Deferred by decision:
- [ ] **Google Docs**: text is drawn on a canvas, and the only AX route needs Docs' braille mode, which we won't ask users to turn on. Try OCR of the caret line.
- [ ] **VS Code editor font**: VS Code reports no font and gives no width measurement, so the ghost uses the system font. Fallback idea: use Menlo in VS Code's editor, sized from how far the caret moves per character.
- [ ] **VS Code Chat input**: not tested.

Known gaps:
- [ ] Keyboard layouts: the key above Tab and ⌘V use ANSI key codes; ISO and Dvorak need a `UCKeyTranslate` lookup
- [ ] Per-app "always paste" option for apps where typed insertion fails
- [ ] Settings for screen text: excluded apps, full-window scope
- [ ] Past writing: language filter, recording off per app or website, a per-app history list, two texts in one window count as one
- [ ] Search websites (Google, Bing, DuckDuckGo) aren't off by default yet; this needs exact search hosts so Gmail and Docs stay on
- [ ] Search boxes that don't mark themselves as search fields still get suggestions
- [ ] Pause doesn't survive a relaunch
- [ ] Killing NexType (instead of quitting) leaves Chrome's accessibility mode on and loses the open past-writing session
- [ ] Not yet verified: open at login, the full 2.8 GB download through the UI, Escape hiding

## Future: beyond the Mac

- [ ] **Windows port.** Every feature already built for the Mac, and what Windows needs for it.

  **Reused as-is (platform-free Rust):**
  - `core`: the gate rules, prompt builder, budgets, redaction, ghost trims, healing, next-word split, website matching
  - `engine`: llama.cpp, using CUDA or Vulkan on a graphics card, or the CPU
  - stats file and override logic

  **Needs a Windows version:**

  | Mac feature (done) | Mac today | Windows needs |
  |---|---|---|
  | Read the focused field: text, caret, selection, role | Accessibility (`AXUIElement`) | UI Automation: `TextPattern` / `TextPattern2`, `GetCaretRange` |
  | Caret rectangle | `AXBoundsForRange`, text markers | `ITextRangeProvider::GetBoundingRectangles`; `GetGUIThreadInfo` caret as fallback |
  | Field font | `AXAttributedStringForRange` | UIA `FontName` / `FontSize` text attributes |
  | Watch focus and text changes | `AXObserver` | UIA focus-changed and text-changed event handlers |
  | Front app and its id | NSWorkspace, bundle id | `GetForegroundWindow` + process path / AppUserModelID as the app key |
  | Window title | `AXTitle` | `GetWindowText` |
  | Password fields skipped | `AXSecureTextField` | UIA `IsPassword` |
  | Search fields skipped | `AXSearchField` | UIA control type / localized type "search" (per app) |
  | Switch on browser and Electron accessibility | `AXEnhancedUserInterface`, `AXManualAccessibility` (reset on quit) | Chromium turns UIA on when a client asks; may need `--force-renderer-accessibility` checks per browser |
  | Website detection | `AXWebArea` `AXURL` | UIA document `Value` / address bar of the browser |
  | Ghost text overlay | NSPanel, never takes focus | Layered, click-through, topmost window (`WS_EX_LAYERED \| WS_EX_TRANSPARENT \| WS_EX_NOACTIVATE`), DPI-aware per monitor |
  | Tab / key-above-Tab / Escape while a ghost shows | CGEvent tap | `SetWindowsHookEx(WH_KEYBOARD_LL)`; the key above Tab is VK_OEM_3 on US layouts |
  | Insert the accepted text | Unicode key events | `SendInput` with `KEYEVENTF_UNICODE` |
  | Paste fallback | NSPasteboard + ⌘V | Win32 clipboard + Ctrl+V, restore after; skip clipboard-history formats |
  | Confirm insertion | AX re-read after 1 s | UIA re-read after 1 s |
  | Copied-text context | `changeCount`, nspasteboard.org privacy types | `GetClipboardSequenceNumber`; skip `ExcludeClipboardContentFromMonitorProcessing` / `CanIncludeInClipboardHistory = 0` |
  | Screen text | ScreenCaptureKit + Vision | Windows.Graphics.Capture + Windows.Media.Ocr |
  | Past-writing database | SQLCipher via CommonCrypto | SQLCipher with vendored OpenSSL |
  | Database key | Keychain | Windows Credential Manager or DPAPI |
  | Menu bar app | SwiftUI `MenuBarExtra` | Tray icon app (Tauri, WinUI or egui) |
  | Settings window, per-app/site overrides | SwiftUI | Same screens in the new shell |
  | Onboarding | Accessibility permission + download | No permission step (UIA needs none); model download step only |
  | Model download | URLSession, resume, sha256 | Same in Rust (`reqwest`) or the shell |
  | Open at login | `SMAppService` | Registry `Run` key or Startup task |
  | Off-by-default apps | bundle ids (Spotlight, terminals…) | Windows list: Start / Search, PowerToys Run, Windows Terminal, cmd, PowerShell, Explorer, password managers |
  | Excluded password managers | bundle ids | executable names of the same products |
  | Per-app quirks | Chrome, Safari, Teams, VS Code, Gmail tested | Test matrix again: Chrome, Edge, Firefox, Teams, VS Code, Outlook, Word, Slack, Notepad |
  | Signing and install | Self-signed dev identity, .app bundle | Code-signing certificate, MSIX or installer |
- [ ] iCloud end-to-end sync of past writing (maybe never)
- [ ] LoRA adapters
