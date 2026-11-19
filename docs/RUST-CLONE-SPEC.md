# Rust Clone: Build Specification

This is everything we need to build a Rust app for macOS that does what Cotypist does: system-wide inline autocomplete from a local LLM, using what is on screen as context. It has three inputs:

- **What Cotypist does.** Behaviour, parameters and data formats, taken from the shipped binary (build 85). The background and evidence are in [`HOW-COTYPIST-WORKS.md`](HOW-COTYPIST-WORKS.md).
- **How to do it in Rust.** The crate and API for each piece.
- **What we decide ourselves.** The items Cotypist doesn't settle for us, each with a recommended starting value.

Evidence tags as in the main doc: **[O]** observed in the binary or on disk, **[I]** inferred, **[U]** unknown. Unless noted, numbers are Cotypist's compiled-in defaults. Server flags on this Mac are empty (`ServerFlags_cachedPayload` → `{"v":1,"flags":{}}`), so **these defaults are the values actually in use** [O].

---

## 0. Ground rules for the clone

1. **Our own name, icon and wording.** Don't reuse Cotypist's name, art, UI text or onboarding videos.
   - Prompt *structure* is functional and can be reused.
   - Write our own instruction sentences. Cotypist's verbatim prompt text is kept in this spec as a reference only.
2. **Our own models.** Cotypist's `Q4_C` model files are encrypted and licensed to them. We use public GGUF files from Hugging Face (§16).
   - Qwen3 and Gemma 4 are both Apache 2.0, confirmed on 2026-11-18 via the HF API: `cardData.license = apache-2.0`, ungated. The Gemma 4 license page is ai.google.dev/gemma/docs/gemma_4_license. Bundle the LICENSE/NOTICE files and never use model names as our branding.
3. **Leave out what is about their business, not the feature:**
   - licensing, paywall, trial and daily caps
   - anti-tamper and string obfuscation
   - analytics upload and the newsletter
   - the RepliesSDK support form
4. **Target platform.** macOS 14+ on Apple Silicon (arm64), **not sandboxed**. Decided 2026-11-18:
   - **Name:** **NexType**, bundle id `dev.nextype.NexType`.
   - **No Mac App Store.** The sandbox would block Accessibility and event taps anyway.
   - **Signing for personal use:** a self-signed "NexType Dev" certificate (`scripts/make-dev-cert.sh`).
   - **Signing for other Macs:** only then add Developer ID signing and notarization, so Gatekeeper accepts it. Cotypist isn't sandboxed either; Accessibility and event taps require it.
5. **Local-first.** No text leaves the Mac. iCloud sync is optional and comes last (§19).

---

## 1. Feature parity checklist

Priority: **P0** = MVP (useful product), **P1** = v1 parity, **P2** = later or optional.

| # | Feature | Cotypist behaviour | Pri |
|---|---|---|---|
| 1 | Menu-bar agent | `LSUIElement`, status item with menu (§15) | P0 |
| 2 | Permission onboarding | Accessibility (required), Screen Recording (optional), Input Monitoring for the key tap | P0 |
| 3 | Focus tracking | AX observer per app + 1 s polling fallback (§4) | P0 |
| 4 | Read text around caret | AX value/selection; text markers for web views (§4) | P0 |
| 5 | Gating | Field size, chars, roles, excluded apps, Secure Input (§5) | P0 |
| 6 | Prompt builder | Per-family chat templates, pre-filled assistant turn (§8) | P0 |
| 7 | Local inference | llama.cpp + Metal, KV prefix reuse, cancellation (§9) | P0 |
| 8 | Ghost text overlay | Borderless click-through panel at the caret (§11) | P0 |
| 9 | Accept full / next word | Key above Tab = full, Tab = next word (§12) | P0 |
| 10 | Insertion | Unicode injection in chunks, paste fallback, AX confirmation (§12) | P0 |
| 11 | Model download and management | Catalog, download, checksum, select (§16) | P0 |
| 12 | Settings window | Global settings + per-app/domain overrides (§14) | P0 |
| 13 | Environment context | Date/time, languages, name, app, window title, domain (§7.1) | P0 |
| 14 | Secret redaction | 13 regexes (§7.5) | P0 |
| 15 | Mid-line completions | Split layout with text after caret (§8.6) | P1 |
| 16 | Screen OCR context | ScreenCaptureKit + Vision, cleanup, budget (§7.2) | P1 |
| 17 | Clipboard context | Recent only, skips concealed/transient types (§7.3) | P1 |
| 18 | Recorded writing + few-shot examples | Encrypted SQLite (`user_inputs`), selection rules (§7.4, §13) | P1 |
| 19 | Custom instructions | Global + per-app/domain | P1 |
| 20 | Browser support | Chromium/Electron AX enabling, domain detection, text markers | P1 |
| 21 | Editor-specific handling | Google Docs, VS Code/Cursor/Windsurf chat, CodeMirror, Mail fields (§4.4) | P1 |
| 22 | Terminal/TUI support | xterm.js, native terminals, AI-agent output section | P2 |
| 23 | Beams + ranking + typed-prefix healing | §9.4 | P1 |
| 24 | Advanced KV reuse | splice, parked sequences, prewarm (§9.3) | P2 |
| 25 | Keystroke speculation | Start on key-down from the tap (§6.2) | P2 |
| 26 | Typo hold-back + autocorrect offer | NSSpellChecker + logprob delta (§10) | P1 |
| 27 | Word alternatives / thesaurus | List after 2 s pause or shortcut | P2 |
| 28 | Emoji `:shortcode` completion | Trie over emoji data | P2 |
| 29 | Inline suggestion list (pill) | For RTL or when the ghost has no room | P2 |
| 30 | Text mirror overlay | For apps with bad caret geometry | P2 |
| 31 | Caret-line font matching | Screenshot + render matching (§11.4) | P2 |
| 32 | N-gram personalization booster | §13.3 | P2 |
| 33 | LoRA adapters | Load/apply | P2 |
| 34 | Battery mode | Disable auto-activation / shorter completions / smaller model | P2 |
| 35 | Stats | Words accepted per day, menu-bar count | P2 |
| 36 | Secure Input incident UI | Name the app holding Secure Input | P1 |
| 37 | Event-tap leak warning | `CGGetEventTapList` | P2 |
| 38 | Disable macOS inline prediction prompt | `NSAutomaticInlinePredictionEnabled` | P1 |
| 39 | URL scheme for settings | `app://setPreference`, `setOverride`, `showPreferencePane/…` | P2 |
| 40 | Auto-update | Sparkle-equivalent | P1 |
| 41 | Crash reporting | Optional, opt-in | P2 |
| 42 | iCloud E2E sync of recorded writing | CloudKit, own encryption envelope | P2 (maybe never) |

---

## 2. Architecture for the Rust app

```text
┌──────────────────────────── main thread (CFRunLoop / NSApplication) ────────────────────────────┐
│ StatusItem + menus · Settings window · Overlay panels (AppKit is main-thread only)               │
│ AXObserver run-loop sources · CGEventTap run-loop source · Carbon hot keys · NSWorkspace notices │
└──────────────┬──────────────────────────────────────────────────────────────────▲───────────────┘
               │ FieldEvent (focus/value/caret/key)                                │ Suggestion / Hide
        ┌──────▼───────────┐   snapshot   ┌───────────────────┐  PromptPlan  ┌─────┴─────────────┐
        │ Session manager  ├─────────────►│ Context gatherer  ├─────────────►│ Generation worker │
        │ (gating, debounce│              │ (env, OCR, clip,  │              │ owns llama model  │
        │  cancel tokens)  │◄─────────────┤  past writing)    │              │ + context + KV    │
        └──────┬───────────┘   AX reads   └─────────┬─────────┘              └─────────┬─────────┘
               │                                    │ SQLCipher (store thread)         │
               │ accept → Inserter (CGEvent / paste) ▼                                  │ tokens/logits
               └──────────────────────────── Store (user_inputs, overrides, stats) ◄────┘
```

**Cargo workspace layout** (suggested):

| Crate | Role | Platform |
|---|---|---|
| `core` | Types, settings model, gating rules, prompt builder, budgets, redaction, post-processing, ranking | pure Rust, fully unit-testable |
| `engine` | llama.cpp wrapper: load, tokenize, batch decode, KV ops, sampler, beams, caches | llama.cpp FFI |
| `mac-ax` | AXUIElement/AXObserver wrappers, text-marker helpers, app-specific readers | macOS |
| `mac-input` | Event tap, hot keys, CGEvent insertion, pasteboard, keyboard-layout helpers | macOS |
| `mac-overlay` | NSPanel ghost window, text drawing, inline list, text mirror | macOS |
| `mac-capture` | ScreenCaptureKit screenshot, Vision OCR, cleanup | macOS |
| `store` | SQLCipher DB, migrations, Keychain key, model files | mostly portable |
| `app` | Bootstrap, menu bar, settings UI, onboarding, updater, URL scheme | macOS |

**Threading rules:**
- All AppKit/AX observer work runs on the main thread.
- AX *reads* can block for up to the messaging timeout, so do them on a dedicated "AX reader" thread with `AXUIElementSetMessagingTimeout`. Cotypist uses that call [O]; the value is [U], so start at ~0.25 s.
- llama runs on exactly one worker thread that owns the context. Requests carry a cancel token, and the newest request wins. Cotypist's outcomes `cancelledInDebounce`/`taskCancelled`/`steerYielded` [O] map to this.

---

## 3. Permissions and onboarding (§1 #2)

| Permission | API to check / request | Needed for |
|---|---|---|
| Accessibility | `AXIsProcessTrustedWithOptions({kAXTrustedCheckOptionPrompt: true})` [O] | Everything. Pipeline must not start without it ("Pipeline not started: accessibility permission unavailable") [O] |
| Input Monitoring / event posting | `CGPreflightPostEventAccess` [O]; event-tap creation fails without listen access | Key tap (speculation, Escape), CGEvent posting |
| Screen Recording | `CGPreflightScreenCaptureAccess` / `CGRequestScreenCaptureAccess` [O] | OCR context, caret-line font matching |

Cotypist's onboarding steps (from localization comments, [O]):
1. Accessibility. Its video differs for Ventura vs macOS 27, where the System Settings pane is named "Device Control and Data Access".
2. Pick and download a model (recommended by RAM/chip, with a warning if suboptimal).
3. Learn the keys ("Continue by pressing Tab"; full-accept on the key above Tab).
4. Custom instructions.
5. Personalization opt-in (writing recording).
6. Offer to disable macOS inline prediction.
7. Optional Screen Recording.

Permissions are tied to the code signature. Dev builds must be signed with a **stable** identity, or every rebuild loses the grants (§21).

---

## 4. Focus and field tracking

### 4.1 AX notifications to observe [O]

Registered per app pid with `AXObserverCreate` + `AXObserverAddNotification`; the source goes on the main run loop:

- `AXFocusedUIElementChanged`, `AXFocusedWindowChanged`
- `AXValueChanged`, `AXSelectedTextChanged`
- `AXUIElementDestroyed`, `AXWindowMiniaturized`
- `AXApplicationDeactivated`, `AXApplicationHidden`
- `AXAnnouncementRequested` (from `NSAccessibilityAnnouncementRequestedNotification`)

Also listen to `NSWorkspace` app-activation notices to re-attach the observer to the new frontmost app. As a fallback, poll the focused element every **1.0 s** (`focusedElementPollingInterval`) [O].

Field-session reset reasons [O]: "new field session", "window changed", "app deactivated", "app hidden", "layout change", "scroll wheel" (global NSEvent monitors for scroll and mouse).

### 4.2 AX attributes and calls used [O] (from imports)

- **Elements:** `AXUIElementCreateSystemWide`, `AXUIElementCreateApplication`, `AXUIElementCopyElementAtPosition`, `AXUIElementGetPid`, `AXUIElementGetWindow`.
- **Attributes:**

  | Kind | Attributes |
  |---|---|
  | Focus and hierarchy | `Focused`, `FocusedUIElement`, `FocusedWindow`, `Parent`, `Children`, `Window`, `TitleUIElement`, `Rows` |
  | Text | `Value`, `SelectedText`, `SelectedTextRange`, `NumberOfCharacters`, `InsertionPointLineNumber`, `SharedCharacterRange`, `PlaceholderValue` |
  | Identity | `Role`, `Subrole`, `Identifier`, `Description`, `Help`, `Title` |
  | Document | `Document`, `URL` |
  | Geometry and chrome | `Position`, `Size`, `HorizontalScrollBar`, `VerticalScrollBar`, `MenuBar` |

- **Parameterized:** `StringForRange`, `AttributedStringForRange`, `BoundsForRange`, `LineForIndex`, `RangeForLine`.
- **Font and colour** from the attributed string: `kAXFontTextAttribute` (`FontName`/`FontFamily`/`FontSize`), `kAXForegroundColorTextAttribute`, `kAXListItemLevelTextAttribute`.
- **Web/WebKit text markers** (decoded strings) [O]:
  - `AXSelectedTextMarkerRange`, `AXTextMarkerRangeForUIElement`
  - `AXStringForTextMarkerRange`, `AXAttributedStringForTextMarkerRange`
  - `AXBoundsForTextMarkerRange`, `AXLengthForTextMarkerRange`
  - `AXIndexForTextMarker`, `AXTextMarkerForPosition`, `AXUIElementForTextMarker`
  - `AXNextTextMarkerForTextMarker`, `AXPreviousTextMarkerForTextMarker`
  - `AXNextLineEndTextMarkerForTextMarker`, `AXPreviousLineStartTextMarkerForTextMarker`, `AXPreviousParagraphStartTextMarkerForTextMarker`
  - `AXTextInputMarkedRange` (IME composition)
  - `AXDOMIdentifier`, `AXDOMClassList`, `AXLanguage`, `AXUserInputLabels`
  - `AXWebArea` and `AXWebApplication` roles

  Text-marker objects are built with the private-but-exported `AXTextMarkerRangeCreate`, `AXTextMarkerRangeCopyStartMarker`/`EndMarker`, `AXTextMarkerGetLength` and `AXTextMarkerGetBytePtr` [O].
- **Enabling AX in Chromium/Electron:** set `AXEnhancedUserInterface` = true (or `AXManualAccessibility` for Electron) on the app element. Cotypist does this only for listed apps or the per-app "Improve compatibility" toggle (`needsEnhancedUserInterface`) [O]. It waits for the AX tree to appear ("Force-activation waited for the accessibility tree of %@; ready %ld after %.1f s") [O].
- **Roles that count as text fields:**
  - Standard roles: `AXTextArea`, `AXTextField`, `AXComboBox`, `AXWebArea` content
  - Subroles excluded: `AXSecureTextField` (passwords) and `AXSearchField` [I]

### 4.3 Field snapshot (what to capture each cycle)

| Field | Source |
|---|---|
| `text_up_to_cursor`, `selected_text`, `text_after_cursor` | Value + SelectedTextRange (or text markers) |
| caret rect | `BoundsForRange` at the caret (or `AXBoundsForTextMarkerRange`), validated: reject "no cursor rect", "unsafe cursor rect", "cursor outside window", "cursor on no screen" [O] |
| font name/size, text colour | attributed string at the caret |
| role/subrole/identifier/DOM id/classes | element attributes |
| app bundle id, pid, window title, URL/domain | app + window + `AXURL`/`AXDocument` |
| placeholder | `AXPlaceholderValue` (stored in `environmentContext` as `placeholderValue`) [O] |
| text language | `NLLanguageRecognizer` (stored as `textLanguage`) [O] |

### 4.4 App and editor recognizers (DOM ids/classes) [O]

| Target | Signals |
|---|---|
| Google Docs | `kix-appview`, `kix-appview-editor`, `kix-appview-editor-container`, `kix-appview-clipped-ui-elements-container`, `kix-cursor`, `kix-current-user-cursor-caret`, `docs-editor`, `docs-editor-container`, `docs-a11y-aria-enable` (screen-reader support must be on). Caret poll limit **0.5 s**, retry **0.03 s** [O] |
| VS Code / Insiders / Windsurf / Cursor (`com.todesktop.230313mzl4w4u92`) | `native-edit-context`, `monaco-mouse-cursor-text`, `cursor-text`, `suggest-widget`, `workbench.panel.chat`, `workbench.parts.sessions`, `workbench.parts.editor`, `windsurf.cascadePanel`; chat inputs: `aislash-editor-input`, `ui-prompt-input-editor__input`, `composer-editor` |
| CKEditor / rich editors | `ck-content`, `persistent-editor-content` |
| xterm.js terminals | `xterm-screen`, `xterm-helper-textarea`, `xterm-accessibility`, `xterm-accessibility-tree` |
| Safari Mail-like address fields (exclude) | `Mail.toField`, `Mail.ccField`, `Mail.bccField`, `Mail.replyToField` |
| Remote-view-hosted fields | Detected via `detectRemoteViewHostedFields` (default **true**); insertion events go to the global session instead of the app pid [O] |

---

## 5. Triggers, debounce and gating

### 5.1 Triggers

- AX value or selection change → schedule a cycle after **5 ms** (`completionUpdateDebounceMs`) [O], or **30 ms** in terminals [O].
- Mouse click that moves the caret → wait **100 ms** [O].
- Key-down from the listen-only tap is used for speculation, Escape handling and "typing along with the ghost" (§6.2, §9.6).

### 5.2 Gate (all must pass) [O unless marked]

| Rule | Value |
|---|---|
| App not excluded | built-in list (§17) + user overrides `completionsDisabled` |
| Field role allowed | per-app `onlyAdmissibleRoles` (e.g. Mail: body only) |
| Not a secure/password field | `AXSecureTextField` subrole [I]; `IsSecureEventInputEnabled()` [O] |
| Field size | width ≥ **300 pt** and height ≥ **32 pt** and area ≥ **6400 pt²** … |
| … or enough text already | ≥ **30** chars bypasses the size check |
| Minimum text | ≥ **3** chars typed [I semantics] |
| Caret position | end of line, unless mid-line is enabled (`midLineCompletionsDisabled` override, `midLineRequireRecentTyping`) |
| RTL text | suppressed (`suppressCompletionsOnRTLText`) or shown in the pill list |
| User toggles | global off, per-app/domain off, temporary disable (short/long durations), Escape snooze |
| Private browser window | no recording ("shown instead of writing recording items while a private browser window is in front") [O] |
| Code editors | off unless `codeEditorCompletionsEnabled`; AI chat inputs always allowed |

### 5.3 Escape behaviour (picker, 3 options) [O]

1. Never intercept Escape.
2. A single Escape turns completions off in the current app until the user switches apps.
3. A single Escape briefly pauses completions (`escapeSnoozeDurationSeconds`, value [U]; suggest 30 s).

`captureEscapeOnlyWhileSuggestionShown` means Escape is only grabbed while a ghost is visible.

---

## 6. Input plumbing

### 6.1 Event tap [O]

```text
CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionListenOnly,
                 CGEventMaskBit(kCGEventKeyDown), callback, userInfo)
```

The tap is listen-only, so it never blocks typing. Re-enable it on `kCGEventTapDisabledByTimeout`.

Cotypist also checks `CGGetEventTapList` to warn when *another* app has leaked taps that slow the Mac [O].

### 6.2 Keystroke speculation (P2)

- Source: the tap (`Speculative_primarySource = "cgtap"`); debounce **5 ms** [O].
- On key-down, predict the new text (old text + typed char) and start generating before the AX value arrives. Adopt the result if the AX value matches; otherwise discard it ("running ahead" / "staged ahead" fallbacks) [I].
- Eager-hide the stale ghost on key-down (`Speculative_eagerHideEnabled`) [O].

### 6.3 Keyboard layout

Use `TISCopyCurrentKeyboardLayoutInputSource` + `UCKeyTranslate` + `LMGetKbdType` [O] to find the **key above Tab** and its character (`keyAboveTabKeyCode` / `keyAboveTabCharacter`) [O].
- ANSI keyCode **50** (`` ` ``); ISO layouts differ.
- The onboarding has a variant for layouts where it can't be bound by default [O].

---

## 7. Context gathering

### 7.1 Environment block [O]

Built by `0x1001bb640`, in this code order:

```text
Today is <EEEE, MMMM d, yyyy>. The current time is <short time>     ← date rounded to 900 s
Operating system languages and locales: <Locale.preferredLanguages joined ", ">
My name: <full user name>
Active app: <app display name>
Window title: <title>
Context in which I am typing (e.g. domain/contact/channel/document): <typing context>
Domain: <host>
```

- The date format string `EEEE, MMMM d, yyyy` is [O] (`0x10029cf6c`). The time format is [U]; use the locale's short time.
- Lines are omitted when the value is missing [I].
- The environment JSON stored per recording has the keys `osUsername`, `name`, `windowTitle`, `url`, `placeholderValue` [O].
- Rounding the time to 15 minutes keeps the prompt prefix stable for KV reuse [I]. Keep this.
- The machine name is available via `SCDynamicStoreCopyComputerName` [O]; its use is [U].

### 7.2 Screen OCR

| Step | Cotypist | Clone |
|---|---|---|
| Permission | Screen Recording | same |
| Capture | ScreenCaptureKit `SCScreenshotManager captureImageWithFilter:configuration:completionHandler:` with an `SCContentFilter` for the window; legacy `CGWindowListCreateImage` as fallback [O] | SCK only (macOS 14+) |
| Scope | `aboveTextField` (`screenshot_space_above`) or `fullWindow`; default scope enum **1** [O]/[U] | Default: region above the field + the field's line, full window as an option |
| Mask own field | `maskTextFieldInScreenshots` [O] | Paint the field rect white before OCR |
| Size | long edge ≤ **2000 px**, optional grayscale [O] | same |
| OCR | `VNRecognizeTextRequest`, **`recognitionLevel = .accurate` (0)** [O], `usesLanguageCorrection` toggle [O] | same; languages from `Locale.preferredLanguages` |
| Concurrency | max **4** concurrent OCR jobs, timeout (value [U]) [O] | 1–2 jobs, ~400 ms timeout |
| Cleanup [O] | drop low-value observations; join adjacent ones; re-join wrapped lines; drop single-word lines | same, with confidence ≥ 0.3 and min height |
| Exclusions | `screenshotExcludedBundleIdentifiers` [O] | password managers + user list |
| Caching | Per window; refreshed on activation, not per keystroke [I] (`screenshotCaptureMs`, `ocrMs` are activation metrics [O]) | Capture on field focus, then every N s or on scroll |

### 7.3 Clipboard

- On for this user (`TextFieldContextCapture_pasteboardContextEnabled = true` in their preferences) [O]. The compiled-in default looks like **off** [I]; ship off and offer it in onboarding.
- Use only if recent: `pasteboardMaxAgeSeconds`, value [U]; suggest 120 s. Track `changeCount` with a timestamp.
- **Skip** items marked with the nspasteboard.org privacy types [O]:
  - `org.nspasteboard.ConcealedType`
  - `org.nspasteboard.TransientType`
  - `org.nspasteboard.AutoGeneratedType`

  Also check `org.nspasteboard.source`.
- The insert-by-paste path restores the previous clipboard after **0.4 s**, pasting after **0.01 s** [O]. It must not count its own write as user clipboard context.

### 7.4 Few-shot examples from recorded writing [O]

| Parameter | Value |
|---|---|
| Fetch candidates | **20** |
| Minimum length | **100** chars |
| Mix | longest-N + most-recent-N + across-apps-recent-N (counts [U]; suggest 2/2/1) |
| Budget | **400** tokens |
| Window | **30** days |
| Filters | same app only (option), language policy (same language as current text) + recheck, rank by content length |

Label: "Recent examples of my writing:".

### 7.5 Secret redaction (apply to every context string before the prompt) [O]

These are the exact regexes from the binary (`__cstring` `0x1012a7570`…`0x1012a7950`). Where a cheap pre-check pattern exists, run it first. The pairing of pre-checks to patterns is [I], and the replacement text is [U]; use `[REDACTED]`.

| Kind | Pattern | Pre-check |
|---|---|---|
| IBAN | `\b[A-Z]{2}\d{2}(?:[ ]?[A-Z0-9]{4}){2,7}(?:[ ]?[A-Z0-9]{1,3})?\b` | `(?-i:[A-Z]{2}\d{2}[ ]?[A-Z0-9]{4})` |
| Signed-URL params | `([?&][A-Za-z0-9_.\-]*sig(?:nature)?=)[^&\s"'<>()\[\]]{8,}` (keep group 1) | `[?&][A-Za-z0-9_.\-]*sig(?:nature)?=` |
| "password: x" (EN/DE) | `(\b(?:pass(?:word|wort|wd|phrase)|kennwort)[ \t]*[:=][ \t]*)(\S*[^\s.,;:!?])` (keep group 1) | `pass(?:word|wort|wd|phrase)|kennwort` |
| env-style assignment with secret-ish name | `^([ \t]*(?:export[ \t]+)?[A-Za-z_][A-Za-z0-9_]*[ \t]*=)[ \t]*(\S.*)$` where the name matches `secret|password|passwd|token|api[_-]?key|access[_-]?key|private[_-]?key|credential|client[_-]?secret|authorization` | `^[ \t]*(?:export[ \t]+)?[A-Z][A-Z0-9_]*=` |
| PEM private key | `-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----` | |
| AWS key id | `\b(?:AKIA|ASIA)[0-9A-Z]{16}` | |
| Stripe | `\b(?:sk|rk)_(?:live|test)_[0-9A-Za-z]{16,}` | |
| OpenAI | `\bsk-(?:proj-)?[0-9A-Za-z_-]{20,}` | |
| GitHub | `\bgh[pousr]_[0-9A-Za-z]{20,}` | |
| Google API | `\bAIza[0-9A-Za-z_-]{35}` | |
| Slack | `\bxox[baprs]-[0-9A-Za-z-]{10,}` | |
| JWT | `\beyJ[0-9A-Za-z_-]{8,}\.[0-9A-Za-z_-]{8,}\.[0-9A-Za-z_-]{8,}` | |
| URL with credentials | `[A-Za-z][A-Za-z0-9+.\-]*://[^\s/:@]+:[^\s/@]+@` | |

Rust: the `regex` crate supports all of these except the lazy `[\s\S]*?`, which is fine with `(?s)…*?`, and the inline `(?-i:…)` flag group, which is supported.

---

## 8. Prompt builder

### 8.1 Principle

All context and instructions go in the **user** turn. The **assistant** turn is opened and **pre-filled** with a lead-in plus the user's text, so the model continues the sentence [O].

### 8.2 Templates (Cotypist verbatim, for reference) [O]

| Family | User open | Assistant open (pre-filled after it) |
|---|---|---|
| Gemma 3 (and Gemma 4 in Cotypist) | `<start_of_turn>user\n` | `<end_of_turn>\n<start_of_turn>model\n` |
| **Gemma 4, official** (use in the clone) | `<bos><\|turn>user\n` | `<turn\|>\n<\|turn>model\n` |
| Qwen 3 | `<\|im_start\|>user\n/no_think\n` | `<\|im_end\|>\n<\|im_start\|>assistant\n<think>\n\n</think>\n` |
| Qwen 3 + system | `<\|im_start\|>system\n…<\|im_end\|>\n<\|im_start\|>user\n` | `<\|im_end\|>\n<\|im_start\|>assistant\n` |
| Llama 3.x | `<\|start_header_id\|>user<\|end_header_id\|>\n\n` | `<\|eot_id\|>\n<\|start_header_id\|>assistant<\|end_header_id\|>\n\n` |
| Phi-4 mini | `<\|user\|>` | `<\|end\|><\|assistant\|>` |
| Base models | (none) | `My writing:\n\n` / `The part of my writing I am currently typing:\n\n` |

> **Gemma 4: confirmed 2026-11-18.** Google's `chat_template.jinja` for `gemma-4-E2B-it` renders `<bos><|turn>user\n{text}<turn|>\n<|turn>model\n`. Thinking is **off by default**: without `enable_thinking` there is no system turn and no `<|think|>`.
>
> Special token ids in Gemma 4's `tokenizer.json`:
> - `<|turn>` = **105**, `<turn|>` = **106**. These are the same ids Gemma 3 gives `<start_of_turn>`/`<end_of_turn>`, and Gemma 4's vocabulary has no `<start_of_turn>` token. Cotypist's Gemma 3 markers therefore only work if its tokenizer maps them to ids 105/106 [I], which would explain why it never renamed them.
> - EOS ids from `generation_config.json`: `1` `<eos>`, `106` `<turn|>`, `50` `<|tool_response>`.
>
> **Clone:**
> - Use the official strings and tokenize with special-token parsing, so that 105/106 come out as single tokens.
> - Stop generation on 1/106.
> - Google's sampling defaults (temperature 1.0, top_k 64, top_p 0.95) are for chat. Keep greedy or beam decoding for completions.

### 8.3 Section header table (`0x10025c404`) [O]

Each section has (header, footer); the separator is `"\n\n"`:

| Slot | Header | Footer |
|---|---|---|
| environment | `""` | `"\n\n"` |
| (unnamed) | `""` | `""` |
| screen | `\n\nText visible on screen around where I am typing:\n` | `""` |
| past writing | `\n\nRecent examples of my writing:\n` | `""` |
| clipboard | `\n\nClipboard contents:\n` | `""` |
| what comes next | `\n\nWhat comes next:\n` | `""` |
| text before (mid-line) | `\n\nMy text so far, up to the part I am currently typing:\n` | `""` |
| text after (mid-line) | `\n\nWhat comes right after the part I am currently typing:\n` | `""` |

Other fixed parts:
- **Intro:** `I am typing the text at the end on my Mac. Additional context; some of it could be irrelevant:\n\n`
- **Master instruction (310 chars):** `\n\nNow, please complete the following text that I am typing. Write full, grammatically correct sentences in the language and style of the existing text. Avoid using placeholders. Do not mention that your task is to provide completions, or e.g. the fact that I am typing, etc.; just try to complete the sentence.`
- **Assistant lead-ins:** `Here is the entire text I am typing:\n` (whole field fits) / `Here is the part of the text I am currently typing:\n` (truncated)
- **Coding-agent section:** `Previous output of the AI coding agent:`
- **Dialect line:** `Please use British English (with -ise spellings) spelling and punctuation.` (modes `auto`, `british`, `canadian`, `disabled` [O])

### 8.4 Section order

The emission order is not fully recovered. `PromptCoordinator_sectionOrder` exists, but build 85 also reuses look-alike keys as hiding places for licensing state [O]. **Recommended clone order** follows the header table order and the "most stable first" rule for KV reuse:

```text
[user open]
intro
environment block
custom instructions (global + app/domain) [+ dialect line]
screen text
past-writing examples
clipboard
master instruction
(mid-line: "My text so far…" + before-text, "What comes right after…" + after-text)
[assistant open] lead-in + text before caret     ← generation starts here
```

Put the most volatile sections (screen, clipboard) **after** the stable ones. Splicing (§9.3) handles the rest.

### 8.5 Budgets (tokens) [O]

| Section | Budget |
|---|---|
| Text before caret | **350** (truncate from the start; snap to a word or sentence boundary) |
| Text after caret | **200** |
| Past writing | **400** |
| Custom-instruction reserve | **1000** |
| Screen / clipboard / environment caps | [U] → start at **600 / 200 / 150** |
| Fill target | **0.85** of the budget (`tokenTruncationPreferredUtilization`) |
| Redistribution | Hand budget left unused by empty sections to others in fixed steps (`tokenBudgetRedistributionStepSize`, value [U]; use 50) |
| Total prompt | [U]; aim ≤ 2,048 tokens and size `n_ctx` to 4,096 |

Count tokens with the model's tokenizer. Cache tokenizations of long, stable strings: Cotypist caches texts ≥ **200** chars, max **25** entries, evicting **5** [O].

### 8.6 Mid-line layout

When the caret is inside text, use the split layout. Helper regexes [O]:
- sentence end `.!?\n`
- empty bullet `(^|\n)\s*-\s?$`

Options: `midLineSplitLayout`, `midLineSplitAllCycles`, `midLineNearBeforeTokenBudget`, `maxNearAfterCursorTokens` [O]; values [U].

### 8.7 Scaffolding marker

Insert a unique marker string in the prompt, and demote tokens that would reproduce it while sampling (`shouldDemoteScaffoldingMarkerTokens`) [O]. This stops prompt structure from leaking into suggestions.

---

## 9. Inference engine

### 9.1 llama.cpp configuration

| Item | Cotypist | Clone start value |
|---|---|---|
| Library | private llama.cpp fork, static, Metal + CPU + Accelerate BLAS [O] | upstream llama.cpp via Rust bindings |
| GPU offload | Metal | `n_gpu_layers = 999` |
| Batch | **1024 or 512** depending on a model/device check; ubatch **256** on small configs [O] (`0x100779c10`, `0x100779d3c`) | `n_batch 512`, `n_ubatch 256` |
| Sequences | likely **8** slots [I] (`mov w8, #8` in engine init) | `n_seq_max 8` (beams + parked) |
| Context size | computed [U] | 4,096 |
| Attention | **flash attention on**: at runtime the fork's stderr shows `ggml-metal: fa-vec`/`fa-mm` kernels, dk 256/512 [O 2026-11-18] | `flash_attn = enabled` |
| Model shape (Gemma 4 E2B) | Runtime `ne00=1536`, `dk=256/512` and `kv=512` match the public `config.json`: hidden 1536, 35 layers, 8 heads, **1 KV head**, head_dim 256 (global layers 512), **sliding window 512**, 20 KV-shared layers, vocab 262,144, max context 131,072 [O] | Tiny KV cache, so a 4–8 k context costs little memory |
| Decode width | one decode call had `ne11=2` (2 rows at once) [O]. That suggests 2 sequences/beams per step [I] | start with 2–3 beams |
| Threads | `GenerationManager_threadCount` [O] | perf cores − 1 |
| Metal knobs | `GGML_METAL_NO_RESIDENCY`, `…RESIDENCY_KEEP_ALIVE_S`, `…TENSOR_DISABLE` env overrides [O] | expose as env |
| Unload | after **60 s** unused [O] | same |
| Startup | prewarm when a field gains focus (`extendedPrefillEnabled`) [O] | P2 |

### 9.2 Tokenizer

Use llama's tokenizer and detokenizer, with a cache:
- token → piece string, so typed-prefix matching (§9.4) avoids repeated detokenization
- text → tokens, keyed by a hash

Cotypist's own fast tokenizer (`FTE1`/`VTC1` caches) is an optimization only.

### 9.3 KV-cache reuse

| Mode | What it means | How in llama.cpp |
|---|---|---|
| `cached` | identical prompt | return the cached result or logits |
| `lcp` | longest common prefix | `llama_memory_seq_rm(seq, lcp, -1)`, decode the tail only |
| `splice` | middle changed, later chunk unchanged | remove the changed range; `seq_add` to shift the later chunk's positions; decode the new middle. Min extra tokens: `kvSplicingMinExtraTokens` [O]/[U] |
| parked sequences | keep recent alternative prompt states in spare sequence slots; reseed with `seq_cp` | count / max divergence / max age [U] |
| relocation | move the user-text KV block when the context before it changes length (`seq_add`) | |
| `cold` | full prefill | |

MVP needs only `lcp`. It already covers ~95 % of keystrokes, since only the tail changes.

### 9.4 Decoding

- **Beams.** Several candidates, decoded as parallel sequences in one batch; count [U], start with 3. Score = mean logprob with a length penalty: `score = Σlogp / len^α` (scaling factor and exponent [U]; α≈0.7). Keep the top 1 for the ghost and the rest for word alternatives.
- **Typed-prefix healing.** When the caret is mid-word:
  1. Drop the partial word's tokens.
  2. Restrict the first generated tokens to pieces consistent with the typed partial word.
  3. Expand at most `prefixExpansionLimit` / `expansionLimit` candidates. `maxPrefixLength` = **20** chars [O]. Include a space-only token in the search (`includeSpaceOnlyTokenInPrefixSearch`) [O].
- **Demote** (subtract a large constant, don't ban) [O]:
  - tokens forming invalid UTF-8
  - "problematic invisible characters" (zero-width, bidi controls)
  - scaffolding-marker tokens
- **Stop** at end of sentence or line for short mode. The max length setting (`maxCompletionLength`) has short / medium / long options ("We recommend sticking with a medium length") [O]; token counts [U] → 8 / 16 / 32.
- **Personalization bias.** Add α·boost from the n-gram profile to the logits (§13.3).
- **Greedy** within each beam (no temperature): deterministic, cacheable [I].

### 9.5 Caches

- **Logit cache:** pinned plus stale entries (`LogitCache_maxPinnedEntries` / `maxStaleEntries`) [O].
- **Completion cache on disk:** `completionCache.dat` [O]. Optional for the clone.
- **Stats file:** `completionStats.json` [O] = `{"perDay": {"YYYY-MM-DD": {"short"|"long": {characterCount, completionCount, wordCountAboveLengthThreshold, wordCountTotal}}}}`, where `short` = next-word accepts and `long` = full accepts. It feeds "Completions today" and the menu-bar word count.

### 9.6 Infer-from-ghost

If the user types characters equal to the ghost's next characters, advance the ghost and do **not** re-run the model ("infer-from-ghost") [O]. When the user commits a space at the end of a word that matches, keep going ("space-committed search") [O].

---

## 10. Post-processing

| Step | Rule |
|---|---|
| Trim | at sentence/line end per length mode; never leave a dangling partial UTF-8 character |
| Trailing punctuation | picker [O]: *neither* shortcut inserts trailing punctuation / *only full* / *both*. Default `trailingPunctuationMode` = 1 (likely "only with full completion") [I]. Punctuation sets `.,;:!?"'()[]{}«»-/~–—…“”‘’`, `.!?…,;:` [O] |
| Trailing space | next-word accept may include the trailing space (`includeTrailingSpaceIfAvailable`) [O] |
| Smart quotes | convert straight → curly per app, unless the last **50** typed chars contain straight quotes [O] |
| Typo hold-back | if the current word is misspelled (`NSSpellChecker checkSpellingOfString…`) and the model doesn't continue it, suppress the ghost. Offer a fix when logprob(fixed) − logprob(typed) ≥ `minLogprobDelta` ([U]; ~2.0 nats) [O]. Also run at sentence end |
| Language | `NLLanguageRecognizer`; English, German, French, Spanish, Italian, Portuguese and Dutch are special-cased [O] |
| RTL | skip, or use the pill list |

---

## 11. Overlay (ghost text)

### 11.1 Window [O]

| Property | Value |
|---|---|
| Class | `NSPanel` (`setFloatingPanel:`), `initWithContentRect:styleMask:0 (borderless) backing:2 (buffered) defer:NO` |
| Mouse | `setIgnoresMouseEvents: YES` |
| Collection behavior | `0x149` = canJoinAllSpaces \| transient \| ignoresCycle \| fullScreenAuxiliary |
| Look | `setOpaque: NO`, clear background, no shadow, `orderFrontRegardless` (never activates) |
| Level | `CompletionOverlayManager_overlayWindowLevel` [U]; use `NSStatusWindowLevel` (25) and check it stays above full-screen apps |
| Suggestion list / alternatives panels | `styleMask 0x80` (non-activating panel) at level **25** [O] |

### 11.2 Drawing

- Font: from the AX attributed string (`kAXFontNameKey`/`kAXFontSizeKey`), adjusted by the per-app factor and vertical offset. Bundled web fonts (Inter, Roboto, Open Sans, Lato, Nunito, Source Sans 3, Source Code Pro, JetBrains Mono) cover common web apps [O].
- Colour: a translucent version of the field's text colour (`kAXForegroundColorTextAttribute`), or neutral grey per app (`greySuggestionColorEnabled`) [O].
- Position: caret rect origin. Handle multi-line wrap. If the ghost doesn't fit, use the pill list (`InlineSuggestionsList_useWhenGhostHasNoRoom`) [O].
- Optional accessory button near the field (`accessoryButtonsEnabled`) [O]; purpose [U]; P2.

### 11.3 Text mirror (P2)

For apps whose caret geometry is unreliable, draw the current line plus the ghost in our own panel over the field (`textMirroringEnabled`) [O].

### 11.4 Caret-line font matching (P2) [O]

1. Screenshot a small rectangle around the caret line (`screenshotPadding`).
2. Render the line's text at candidate sizes.
3. Compare column brightness profiles.
4. Accept a size when R² ≥ `minRSquared` and confidence ≥ `minConfidence`, with a polarity margin/contrast check for dark vs light text.
5. Cache per field (`cacheMaxEntries`, `cacheMaxAge`); skip text shorter than `minTextLength`. Values [U].

---

## 12. Accept and insert

### 12.1 Default shortcuts (stored on this Mac; match the onboarding text) [O]

| Action | Default |
|---|---|
| Accept full completion | **key above Tab** (ANSI keyCode **50**, `` ` ``), no modifiers |
| Accept next word | **Tab** (keyCode **48**), no modifiers |
| Force-enable in this field | **⌥** + key above Tab (`ModifierFlags 524288`) |
| Temporary toggle | **⌃⌥⌘** + key above Tab (`1835008`) |
| Others (unbound by default) | alternate full/partial, toggle globally, reveal word alternatives, thesaurus |

Stored as `{KeyCode, ModifierFlags}`, MASShortcut style.

**Dynamic registration:** register Tab / key-above-Tab as Carbon hot keys **only while a ghost is visible**, and unregister on hide [I]. Otherwise Tab is stolen from every app. Option-Tab is a fallback when Tab can't be registered [O]. Per app, "Disable Tab key" removes the Tab binding [O].

### 12.2 Insertion methods [O]

| Method | Details |
|---|---|
| Unicode injection (default) | `CGEventCreateKeyboardEvent` + `CGEventKeyboardSetUnicodeString`, in chunks of **20** UTF-16 units; post to the app pid (`CGEventPostToPid`) or globally for remote-hosted fields |
| Per-app chunk override | 0 = paste, 1 = char-by-char, >1 = chunk, −1 = first word only |
| Paste | save the pasteboard → set the string → ⌘V after **0.01 s** → restore after **0.4 s** |
| Workarounds (per app) | NBSP for leading spaces; real Space key event for leading space; "Paste and Match Style" (⌥⇧⌘V); Backspace right after paste |
| Confirm | re-read AX within **1.0 s** (`insertionConfirmationTTL`); if the text didn't land, log it and try the fallback method |
| After accept | optional fade (default off; fallback 0.3 s); record acceptance stats; next cycle starts immediately |

---

## 13. Storage

### 13.1 Database (SQLCipher) [O]

- The file `cotypist.db` is GRDB 7.11.1 over SQLCipher 4.19.0. The key is kept in the Keychain (service/label `app.cotypist.Cotypist.database`; "Failed to store encryption key") [O]/[I].
- Pragmas, verbatim:

  ```sql
  PRAGMA journal_mode = WAL;            PRAGMA synchronous = FULL;
  PRAGMA foreign_keys = ON;             PRAGMA fullfsync = 1;
  PRAGMA checkpoint_fullfsync = 1;      PRAGMA wal_autocheckpoint = 1000;
  PRAGMA auto_vacuum = INCREMENTAL;     PRAGMA journal_size_limit = 0;
  PRAGMA cache_size = -10240;           PRAGMA mmap_size = 0;
  PRAGMA analysis_limit = 400;          -- integrity: PRAGMA quick_check(1)
  ```

- If the database can't be opened, it is set aside as `cotypist-unreadable-<yyyy-MM-dd-HHmmss>` and a new one is created [O].

**`user_inputs`** columns [O]. Column *types* are [I]; GRDB migrations were `createTable`, `addScreenshotData`, `addDomain`, `addTypingContext`, `addEnvironmentContext`.

| Column | Type (clone) | Notes |
|---|---|---|
| `id` | TEXT (UUID) PK | |
| `createdAt`, `updatedAt` | REAL/DATETIME | indexed |
| `cotypistVersion` | TEXT | app version |
| `appBundleIdentifier` | TEXT | indexed (+ with updatedAt) |
| `domain` | TEXT NULL | indexed with updatedAt |
| `typingContext` | TEXT NULL | indexed with updatedAt |
| `appProperties`, `textFieldProperties`, `propertyBagString` | TEXT (JSON) | role, ids, etc. |
| `textUpToCursor`, `selectedText`, `textAfterCursor` | TEXT | the recorded writing |
| `textLanguage` | TEXT | indexed |
| `screenshotText` | TEXT NULL | OCR text |
| `screenshotData` | BLOB NULL | `screenshotStorageFormat` |
| `environmentContext`, `environmentContextDescription` | TEXT (JSON / rendered) | keys `osUsername`, `name`, `windowTitle`, `url`, `placeholderValue` |
| `rev`, `originDeviceID`, `syncUnknownFields` | INTEGER / TEXT / BLOB | sync only |

Indexes [O]: `idx_user_inputs_{createdAt, updatedAt, textLanguage, appBundleIdentifier, appBundleIdentifier_updatedAt, domain_updatedAt, typingContext_updatedAt}`.

Other tables [O]:
- `AppOverrideRecord` (the §14.2 override fields)
- Sync tables: `sync_state`, `sync_dirty`, `sync_tombstones`, `sync_pending`, `sync_kv`, `sync_keys`, `sync_unsyncable`, `sync_recycle_bin`, `sync_suppress`, plus `sync_user_inputs_*` triggers. These are P2.

### 13.2 Recording rules [O]

- Record snapshots in eligible fields when recording is on (default on after opt-in), unless the app or domain disables it or the window is private.
- Minimum **33** chars; keep the **100** most recent snapshots in memory; retention window **3600 s** before a snapshot is persisted or merged [I].
- `storeInputsWithoutAcceptedCompletions` toggles whether texts with no accepted completion are kept [O].
- The "Typing History" UI shows counts per app and domain, delete per app/domain or all, and a "keeps until deleted" note.

### 13.3 Personalization n-gram booster (P2)

Settings [O]: `ngramBoosterEnabled`, `ngramBoosterAlpha`, `personalizationStrength`(V2), `smoothingFactor`, `minSamples`, `selectivity` + `selectivitySteepness`, `lengthBoostFactor`. The profile cache is per vocabulary (`extras/<family>-v<vocab>/profile.bin`).

Clone design [I]:
1. Build token n-gram counts (n = 1..4) over recorded texts.
2. At each step, boost[token] = α · f(P_user(token | last n−1 tokens) vs a background prior), smoothed, applied only if count ≥ minSamples.
3. Use a sigmoid selectivity so only confident n-grams fire.

### 13.4 Files

| Path | Content |
|---|---|
| `~/Library/Application Support/<bundle>/Models/*.gguf` | models |
| `…/<bundle>/<db>.db` | SQLCipher DB |
| `~/Library/Caches/<bundle>/tokenizer/`, `…/extras/` | caches |
| `~/Library/Preferences/<bundle>.plist` | settings (`NSUserDefaults` or our own TOML/JSON) |

---

## 14. Settings model

### 14.1 Global settings (from Cotypist's settings snapshot keys) [O]

| Group | Settings |
|---|---|
| Completions | `completionsEnabledByDefault`, `midLineCompletionsEnabledByDefault`, `maxCompletionLength` (short/medium/long), `includeTrailingSpaceInWordCompletions`, `includeTrailingPunctuationInWordCompletions`, `trailingPunctuationMode`, `escapeBehavior` |
| Word alternatives | `wordAlternativesMode`, `wordAlternativesPauseDelay` (2 s), `thesaurusMode`, `thesaurusAutoRevealDelay` |
| Autocorrect | `suppressCompletionsOnSuspectedTypo`, `displayAutocorrectFixes` |
| Context | `screenshotContextEnabled`, `pasteboardContextEnabled`, `verticalAlignmentEnabled`, custom instructions (`userPrompt`), British-English mode |
| Personalization | `trainingDataCollectionEnabled` (writing recording), `includePreviousUserInputsInPrompt`, `storeInputsWithoutAcceptedCompletions`, `ngramBoosterEnabled`, `ngramBoosterAlpha`, `personalizationStrength` |
| Model | selected model, recommended model, adapter |
| Emoji | enabled, skin tone, gender, include vanilla variants, shortcuts |
| Battery | `batteryDisableAutoActivation`, `batteryShorterCompletions`, `batteryModelOverrideName` |
| App | `launchAtLoginEnabled`, `statusItemVisible`, `accessoryButtonsEnabled`, `shouldShowCompletedWordCountInMenuBar`, `updateChannel`, `macOSInlinePredictionEnabled` (offer to turn it off), `shortTemporaryDisableDurationSeconds`, `longTemporaryDisableDurationSeconds` |
| Shortcuts | full, partial, full-alt, partial-alt, force-enable, temporary toggle, toggle globally, reveal word alternatives |
| Sync | `cloudSyncEnabled` (+ include OCR text) |

### 14.2 Per-app / per-domain override record (24 fields) [O]

Overrides are keyed by bundle id (`app:<id>`) or domain (`example.com`, `*.example.com`, `example.*`). Some app-only settings aren't available for domains: Tab key, smart quotes, text mirroring, size thresholds and display.

```text
completionsDisabled, midLineCompletionsDisabled, autocorrectDisabled, tabShortcutsDisabled,
smartQuotesDisabled, textMirroringEnabled, ignoreSizeThresholds, codeEditorCompletionsEnabled,
emojiCompletionsDisabled, emojiSearchDisabled, requiresNonBreakingSpaceWorkaround,
requiresSpaceKeyEventWorkaround, requiresPasteAndMatchStyleWorkaround,
requiresBackspaceRightAfterPaste, stringInjectionChunkSize, fontSizeAdjustmentFactor,
verticalAlignmentOffset, greySuggestionColorEnabled, needsEnhancedUserInterface,
proxyEditorSupportDisabled, onlyAdmissibleRoles, onlyAdmissibleRolesActivateEagerly,
trainingDataCollectionDisabled, customInstructions
```

Resolution order: user override → built-in override (§17) → global default. The UI shows "Default (on/off)", "Has built-in overrides" and "Remove Custom Overrides…" [O].

### 14.3 URL scheme (P2) [O]

- `…://setPreference/<key>[/<value>]` (asks the user before changing approval-gated keys)
- `…://setOverride`
- `…://showPreferencePane/{battery,context,personalization,shortcuts}`
- `…://contactSupport`

---

## 15. Menu-bar menu (status item) [O]

| Section | Items |
|---|---|
| Header | completions today / word count; model download progress |
| Problem items (when they apply) | missing Accessibility; no model downloaded; Secure Input held by `<app>` (affected key glyphs); leaked event taps by `<app>`; iCloud sync stopped |
| Enable / disable | Disable Completions Everywhere / for the next N minutes / forever; Disable or Enable in `<App>`; Disable or Enable on `<Domain>`; Enable temporarily disabled completions (global / app / domain) |
| Tab | Disable or Enable Tab Shortcuts (with an explanation for the app) |
| Recording | Writing recording in `<App>` / on `<Domain>`; Disable recording; Disable and erase. Replaced by a note in private browser windows |
| Display | Show/Hide Accessory Buttons; Show Word Count in Menu Bar |
| App | Open Preferences; Check for Updates; Contact Support; Visit Website; Quit |

---

## 16. Models for the clone

Cotypist's catalog (names, sources, quants) is in [`HOW-COTYPIST-WORKS.md` §7.2](HOW-COTYPIST-WORKS.md#72-models). For the clone we use **public GGUF** builds; §20 has the repos verified as available. Plan:

| Tier | Model | Why |
|---|---|---|
| Default (8–16 GB RAM) | Gemma 4 E2B-it Q4 | Cotypist's own pick on this Mac; 262k-token vocabulary, multilingual |
| Better (16 GB+) | Gemma 4 E4B-it Q4 / Qwen3-4B-Instruct Q4 | |
| Small / battery | Qwen3-1.7B or Gemma 3 1B Q4 | |
| Large (32 GB+) | Qwen3-8B, Qwen3-30B-A3B, Gemma 4 26B-A4B | |
| Base-model experiments | Qwen3-*-Base, gemma-3-*-pt | plain-text template |

Catalog entry fields: id, display name, family (template), HF repo, file, size, sha256, min RAM, tier, license notice.

The recommended model is picked from physical RAM and chip generation (Cotypist reports `chip`, `ramTier`) [O]. Show "This model may be slower than expected on your system" when the model is above the Mac's tier [O].

---

## 17. Built-in app policies (bundle-ID lists from the binary)

The lists come from two places [O]:
- **Static Swift `[String]` arrays** in `__DATA,__data` (`0x10141a788`…`0x10145dbe0`).
- **Two functions that use them:**
  - The built-in override table at `0x10002bd14`. Its keys look like `app:<bundle id>` / `domain:<host>`, e.g. `domain:github.com`, `app:com.apple.MobileSMS`, `app:com.microsoft.teams2`, `domain:*.atlassian.net`.
  - A migrator at `0x100029fec` that converts the old per-list preference keys into override records.

For the clone, ship all of this as one data file (`builtin-policies.toml`) that the override resolver reads (§14.2).

### 17.1 Defaults of the old per-list keys (migrator `0x100029fec`)

The migrator decodes each key and then loads one array. Each list is paired with the key decoded just **before** it [I from code order; the reverse pairing gives nonsense, such as the 43 code tools getting text mirroring]. Keys with no array after them have no built-in list here.

| Old key → override field | Built-in list [O] |
|---|---|
| `CompletionManager_applicationsInadmissibleForSmartQuotes` → `smartQuotesDisabled` | **43 code/terminal tools:** Xcode (`com.apple.dt.Xcode`, `com.apple.Xcode`), Simulator, Interface Builder, Editra, Terminal, iTerm2, Warp, Ghostty, Tincta, Kod, Sublime Text 2/3, Atom, BBEdit, VS Code, VS Code Insiders, Cursor (`com.todesktop.230313mzl4w4u92`), Windsurf, Android Studio (+EAP), IntelliJ (+CE, +EAP), AppCode, PhpStorm (+EAP), CLion (+EAP), PyCharm (+CE, +EAP), GoLand EAP, Rider (+EAP), RubyMine (+EAP), MATLAB, RStudio, TablePlus (+setapp) |
| `…applicationsAdmissibleForTextMirroring` → `textMirroringEnabled` | Arc (`company.thebrowser.Browser`, `…browser`), Dia (`company.thebrowser.dia`), Viber (`com.viber.osx`), Microsoft Word |
| `EmojiCompletionSupport_applicationsInadmissibleForEmojiCompletions` → `emojiCompletionsDisabled` | `domain:github.com` (GitHub has its own `:emoji:` popup) |
| `…InadmissibleForEmojiSearch` → `emojiSearchDisabled` | `app:` Slack, Notion, Discord, WhatsApp; `domain:` slack.com, app.slack.com, notion.so, discord.com, web.whatsapp.com (they have their own emoji pickers) |
| `CompletionAccessibilityMonitor_applicationsIgnoringSizeThresholds` → `ignoreSizeThresholds` | Messages (`com.apple.MobileSMS`) |
| `CompletionInserter_applicationsRequiringNonBreakingSpaceWorkaround` | Teams 2 (`com.microsoft.teams2`) |
| `…RequiringSpaceKeyEventWorkaround` | Teams, Teams helper, Teams 2, Teams 2 helper, Chrome |
| `…RequiringPasteAndMatchStyleWorkaround` | MailMaven (`com.smallcubed.mailmaven`), Outlook |
| `excludedApplications`, `applicationsWithTabShortcutsDisabled`, `TrainingDataCollector_applicationsWithCollectionDisabled`, `RequiringBackspaceRightAfterPaste` | none in the migrator. Exclusions come from the built-in table (17.2) |

### 17.2 Built-in table and other lists

Arrays the override table uses (`0x10002bd14`) [O]:
- **24 password managers:** 1Password (`com.1password.1password`, `com.agilebits.onepassword`), Apple Passwords, LastPass (×2), Dashlane (×2), Bitwarden, Keeper (×2), RoboForm, NordPass, Enpass (`in.sinew.Enpass-Desktop`), DataVault, Proton Pass, mSecure, Norton PM, KeePassX, KeePassXC, PasswordWallet, Secrets (×4).
- **The 43 code tools** from 17.1.
- Arc/Dia/Viber/Word.
- Vellum (`co.180g.Vellum`), ChatGPT (`com.openai.chat`), Gemini (`com.google.GeminiMacOS`), Cumbersome, WhatsApp.

Per-entry flags are set in code. Mapping each flag needs the decompiler [U]. Likely values for the clone [I]:

| Group | Clone default |
|---|---|
| Password managers | `completionsDisabled` + `trainingDataCollectionDisabled` + no screenshot/OCR |
| Code tools | completions off unless `codeEditorCompletionsEnabled`; smart quotes off |
| Arc/Dia/Viber/Word | text mirroring on |
| ChatGPT/Gemini/Vellum/Cumbersome/WhatsApp | [U]. Test and decide per app |
| `domain:*.atlassian.net` | [U]. Test Jira/Confluence editors |

Other lists:

| List (address) | Content | Use |
|---|---|---|
| Completions-off-by-default set (`0x100032340`) | password managers + code tools + Finder, Excel, Preview, CleanShot X, Serato DJ, Moleskine Journey, Figma (+Beta), Alfred, LaunchBar, System Settings, Calendar (`com.apple.iCal`), Datavyu, **our own app** (`NSBundle.mainBundle.bundleIdentifier`) | [O] content; "off by default" [I] |
| Compatibility notes (`0x10002b6e4`) | Warp: "doesn't provide the access … needs to read or insert text, so this toggle has no effect". Zed, Zed Preview/Nightly, Zoom (`us.zoom.xos`): "doesn't tell … when the text in it changes". The same table also has Ghostty (+debug), cmux, Sublime Text 4, OneNote, Anki (×2); their note text is [U] | Shown next to the per-app toggle [O] |
| Final Cut (`0x10141ada8`) | `com.apple.FinalCutApp`, `…FinalCut`, `…FinalCutTrial` | Likely excluded [I] |
| Search/design domains (`0x10141ae10`) | figma.com, google.*, google.co.uk/.co.jp/.co.in/.com.au/.com.br/.com.mx, bing.*, duckduckgo.com, kagi.com | Likely domains with completions off (search boxes) [I] |
| 53 web/Electron/Office apps (`0x10141b548`, no direct code reference found) | Chrome (+canary/beta/dev), Chromium, Vivaldi, Yandex, Opera, Brave, Ghost Browser, Edge (+Beta/Dev), Wavebox, Gener8, Helium, Thunderbird, Betterbird, Word, Outlook, Teams 2, Firefox (+Dev), Zen (×2), Sidekick, Arc (×2), Dia, Iridium, Naver Whale, Perplexity Comet, Viber, Pages/Keynote/Numbers (×2 ids each), Feishu (+iron, +helper), Obsidian, Slack, Discord, Notion, Linear, Typora, Bitrix24, Threema (×3), TheBrain | Probably the `applicationsNeedingEnhancedUserInterface` default (apps where AX must be switched on) [I, weak] |
| Firefox family (`0x10141ca50`) | Firefox, Firefox Dev, Zen (×2) | Firefox-specific text extraction (`0x100065bd8`) [O] |
| iWork (`0x1014212d8`) | Pages/Keynote/Numbers (old + new ids) | Special case in prompt assembly (`0x10015a0c8`) [O]; what it does is [U] |
| Terminals (`0x10141c9e0`, `0x10141c458`, `0x10141c4c0`, `0x10141c520`) | Terminal, iTerm2, Ghostty (+debug) | Terminal mode: 30 ms debounce, AI-agent output section [O] |
| VS Code by bundle id **or process name** (`0x10141c5b8`) | `com.microsoft.VSCode`/`Code`, `…VSCodeInsiders`/`Code - Insiders` | VS Code chat/editor detection (`0x1001db4bc`) [O] |
| VS Code family (`0x10141e940`) | VS Code, Insiders, Windsurf, Cursor | DOM-id rules (§4.4) |
| Single-app lists | Safari (+TP); Arc+Dia; Word; Obsidian (`0x1001e6900`); BBEdit (`0x10018db20`…); Fastmail web (`app.fastmail.com`); `dev.serafim.fast` | App-specific readers [O]; details [U] |
| Clipboard manager | `com.plumamazing.copypastepro` (`0x10026f8f0`) | Ignore its pasteboard writes [I] |
| 40 "popular apps" (`0x10145dbe0`) | Safari, Chrome, Firefox, Arc, Edge, Brave, Vivaldi, Orion, Terminal, iTerm2, Ghostty, Warp, VS Code, Cursor, Xcode, Zed, Sublime 4, BBEdit, TextEdit, Mail, Outlook, Mimestream, Spark, Slack, Discord, Teams 2, Telegram, WhatsApp, Signal, Messages, Notes, Notion, Obsidian, Bear, Craft, Pages, Word, Claude, ChatGPT, Codex | Probably the allow-list of app ids that may appear in analytics [I]. Not needed for the clone; useful as the **AX test matrix** (§22) |
| System helpers that are never "frontmost" | `com.apple.share-services`, `…widgetkit-extension`, `…ui-services`, `…usernotifications.service`, `…intents-service`, `com.apple.loginwindow` | Ignore on activation [O] |

The lists are reproduced by `research/tools/arrays.py`.

## 18. Other system integrations

| Item | Detail |
|---|---|
| Secure Input | Poll `IsSecureEventInputEnabled()`. Find the holder via the `CGSessionCopyCurrentDictionary` / `ioreg` "kCGSSessionSecureInputPID" approach [I]. Record incidents (start, last seen, holder, affected apps) and show a banner with the fix ("how to clear the condition") [O] |
| macOS inline prediction | Read and offer to turn off `NSAutomaticInlinePredictionEnabled` (global defaults) [O] |
| Screen lock / sleep | `com.apple.screenIsUnlocked`, `CGSSessionScreenIsLocked`, `CGDisplayIsAsleep` [O]: pause when locked |
| Thermal / battery | thermal state, low-power mode → battery settings [O] |
| Launch at login | `SMAppService.mainApp.register()` (`registerAndReturnError:`) [O] |
| Spell checker | `NSSpellChecker.sharedSpellChecker` `checkSpellingOfString…`, `guessesForWordRange…`, `checkString:range:types:…` [O] |
| Language | `NLLanguageRecognizer`, `NLTokenizer`, `NLTagger` with the lemma scheme (thesaurus) [O] |
| Hardware info | `IOPlatformUUID`, physical memory, CPU brand → model tier [O] |

---

## 19. Sync (P2, optional)

Cotypist syncs `user_inputs` through **CloudKit** (`iCloud.app.cotypist.Cotypist`) with its own E2E encryption envelope [O]:
- `sync_keys`
- `cotypist-usi-gcm-v1`, which suggests AES-GCM [I]
- per-record `ciphertext`, `envelope_version`, `key_id`
- dirty/tombstone/pending queues
- recycle bin

This needs the iCloud entitlement and a provisioning profile, which means a paid developer account and a CloudKit container. **Recommendation:** skip it for v1.

---

## 20. Rust stack

Versions are from the crates.io API on 2026-11-17. `llama-cpp-2`, `objc2-application-services`, `rusqlite`, `screencapturekit` and `global-hotkey` were checked by hand; the rest come from the research pass. **Pin exact versions**, because objc2 has an unreleased breaking change on `main`.

| Subsystem | Crate @ version | Notes and gotchas |
|---|---|---|
| Objective-C / Core Foundation base | `objc2` 0.6.x + `objc2-core-foundation` 0.3.2 | One binding layer for everything below |
| Accessibility | `objc2-application-services` 0.3.2 | Covers `AXUIElementCopy/Set*`, parameterized attributes, `SetMessagingTimeout`, `AXObserverCreateWithInfoCallback`, `AXTextMarker*`, `AXIsProcessTrustedWithOptions`. **The `kAX…` attribute and notification names are CFSTR macros and are not generated**, so define them ourselves as `CFString` constants (names in §4.2). Fallback: hand-written `extern "C"` from `AXUIElement.h` |
| Event tap, CGEvent posting | `objc2-core-graphics` 0.3.2 | `CGEventTapCreate`, `CGEventPostToPid`, `CGEventKeyboardSetUnicodeString`, `CGPreflight*Access`. Declare `IsSecureEventInputEnabled` (Carbon) ourselves. Never make AX calls inside the tap callback |
| Hot keys | `global-hotkey` 0.10.0 (Carbon `RegisterEventHotKey`) | Create on the main thread. Copy Cotypist: **register Tab and the key above Tab only while a ghost is visible**. Alternative: one *active* tap that swallows Tab only while a ghost shows. Simpler with no register race, but a slow callback delays all typing, and its permission needs are unverified. Spike both in M2 |
| Overlay, status item, menus, panels | `objc2-app-kit` 0.3.2 | `NSPanel` non-activating, click-through, collection behaviour `0x149`. AX coordinates are top-left based, so flip them for AppKit. **Not winit/tao**: they can't make non-activating panels and they want to own `NSApplication` |
| Screen capture | `screencapturekit` 11.1.2 (`macos_14_0` feature) | Its build compiles a Swift bridge (needs Xcode). `CGWindowListCreateImage` is obsolete on macOS 15, so don't add the legacy path |
| OCR | `objc2-vision` 0.3.2 | `VNRecognizeTextRequest`, `.accurate`, language correction |
| Language, spelling, pasteboard | `objc2-natural-language` 0.3.2, `objc2-app-kit` (`NSSpellChecker`, `NSPasteboard`) | **Not `arboard`**: it can't save and restore every pasteboard type for the paste-insert path |
| LLM runtime | `llama-cpp-2` 0.1.161 (+ `llama-cpp-sys-2`) | Vendors llama.cpp from 2026-10, with Metal on by default on arm64. Has the `llama_memory_*` sequence operations (Rust names still say `kv_cache_*`), multi-sequence batches, raw logits (`get_logits_ith`), tokenize/`token_to_piece`, LoRA, and saving per-sequence state. Knows `gemma4` and `qwen3`. Backend init happens **once per process**. Turn off default features (`common`, openmp) if unused. Fallback: our own bindgen against `llama.h` |
| Encrypted DB | `rusqlite` 0.43.0 with `bundled-sqlcipher` | On Apple targets SQLCipher builds against CommonCrypto, so no OpenSSL. Apply the §13.1 pragmas after `PRAGMA key` |
| Keychain | `security-framework` 3.7.0 | Store the DB key. The legacy keychain's access list is tied to the code signature (ad-hoc builds re-prompt) |
| Model download | `reqwest` + `sha2` (or `hf-hub` 1.0.0) | hf-hub 1.0 is a rewrite and resume isn't confirmed. Plain HTTP Range requests + sha256 against pinned `resolve/<commit>/<file>` URLs is about the same amount of code |
| Login item | `objc2-service-management` 0.3.2 | `SMAppService.mainApp` |
| Auto-update | Sparkle 2.10.1 via about 30 lines of `objc2` messaging | Embed `Sparkle.framework`, re-sign its XPC services, sign the appcast with EdDSA |
| Crash reports (opt-in) | `sentry` 0.49.6 with the `minidump` feature | Initialize first in `main`. P2 |
| iCloud sync | `objc2-cloud-kit` 0.3.2 (CKSyncEngine) | Needs restricted entitlements and a provisioning profile. **Dropped for v1** |
| Pure-Rust core | `regex`, `serde`/`toml`, `crossbeam-channel` or `tokio` (only for downloads), `tracing` | Prompt, budget, redaction, ranking: no platform code, fully unit-tested |
| Packaging | `cargo-packager` 0.11.8, or a script: `codesign --options runtime --timestamp` → `xcrun notarytool submit --wait` → `stapler` | `cargo-bundle` doesn't notarize |

**UI-layer decision (needs the user).**

| Option | Pros | Cons |
|---|---|---|
| (a) **All Rust** with `objc2-app-kit` for settings and onboarding | One language | Verbose; onboarding with videos and animation is a lot of AppKit code |
| (b) **Thin SwiftUI shell** (Settings scene + onboarding) over the Rust core via `uniffi` 0.32.2 | Native look; Xcode handles signing, entitlements, Sparkle and the login item | Two languages |

In both options the overlay, AX, input and engine stay in Rust. Tauri, egui, Slint and gpui are rejected: either not native-looking, they fight over `NSApplication`, or (Slint) licensing.

**Models for the engine (checked on Hugging Face, all ungated):**

| Model | Repositories |
|---|---|
| Gemma 4 E2B-it | `ggml-org/gemma-4-E2B-it-GGUF` (Q4_0 ≈ 2.84 GB), `google/gemma-4-E2B-it-qat-q4_0-gguf` (≈ 3.35 GB), `unsloth/…`, `bartowski/…` |
| Gemma 4 E4B-it | `ggml-org/…` Q4_0 ≈ 4.59 GB |
| Qwen3 0.6B / 1.7B / 4B / 8B | `unsloth/Qwen3-*-GGUF` Q4_K_M 0.40 / 1.11 / 2.50 / 5.03 GB. The official `Qwen/` repos for the small sizes are Q8_0 only. The 4B also has a non-thinking `Qwen3-4B-Instruct-2507` |
| Qwen 3.5 | **Avoid as the first model.** It is hybrid recurrent, so a partial `seq_rm` (prefix rollback) doesn't work |

Licenses: Qwen3 and Gemma 4 are both **Apache 2.0** (confirmed via the HF API 2026-11-18, ungated). Still read Google's prohibited-use page once before shipping. The `ggml-org` GGUF repos also ship `mtp-*.gguf` draft heads for speculative decoding; they need a newer llama.cpp (P2). Bundle LICENSE/NOTICE and don't use "Gemma" in our branding.

**Biggest practical risk:** TCC (Accessibility, Screen Recording) silently stops working when the signature changes. From the first build, sign dev builds with one stable Apple Development certificate and keep the bundle id and install path fixed (`tccutil reset Accessibility <id>` clears stale grants). Run as a `.app`, never as a bare binary.

## 21. Build, signing and permissions (TCC)

1. App bundle: `Info.plist` with `LSUIElement = YES`, `NSAppleEventsUsageDescription` if needed, a URL scheme, and a bundle id we own.
2. Sign every dev build with the **same** Apple Development / Developer ID identity. TCC keys Accessibility and Screen Recording grants on the code signature; ad-hoc signatures change on every build and lose the grants.
3. Release: hardened runtime + notarization. **No sandbox.** Entitlements only for what we use, e.g. `com.apple.security.cs.allow-jit` is not needed for Metal.
4. Metal: llama.cpp's `.metal` shaders are embedded or compiled at build time (bindings handle this; check).
5. Auto-update: Sparkle 2 appcast or a Rust updater (§20).

---

## 22. Testing and measuring

| Area | How |
|---|---|
| Prompt builder | Golden tests: fixed snapshot + context → exact prompt string per family |
| Redaction | Table tests for every regex (positives and negatives) |
| Budgets | Property tests: never exceed the budget; the caret end is always kept |
| Engine | Deterministic greedy outputs on a tiny model (e.g. Gemma 3 270M from `ggml-org`) in CI |
| AX readers | Manual matrix: TextEdit, Notes, Mail, Safari, Chrome, Slack, VS Code, Cursor, Google Docs, Terminal/iTerm2, Messages, Word |
| Latency | Mirror Cotypist's metrics [O]: `keyToAxMs`, `axToDisplayMs`, `keyToDisplayMs`, `promptLoadMs`, `beamDecodeMs`, `timeToFirstDisplayMs`, tokens decoded vs KV-reused. Target: key → ghost **< 60 ms** p50 on M1 with E2B |
| Insertion | Per-app check that the text arrived (AX re-read) |

---

## 23. Milestones

| # | Milestone | Contents | Exit test |
|---|---|---|---|
| M0 ✅ | Engine spike | Load GGUF, Metal, tokenize, `lcp` KV reuse, greedy decode, cancellation | CLI: continue text < 50 ms per keystroke after warmup |
| M1 ✅ | Text-field loop | AX observer + snapshot in TextEdit/Notes, debounce, gate | Logs show correct before/after text and caret rect |
| M2 ✅ | Ghost + accept | Overlay panel, dynamic hot keys, Unicode insertion, confirm | Type in TextEdit, see the ghost, accept with Tab / key above Tab |
| M3 ✅ | Prompt v1 | Templates, environment block, budgets, redaction, custom instructions | Golden tests pass |
| M4 ✅ | App shell | Menu bar, onboarding, model download, settings, per-app overrides | Fresh Mac → working in 5 minutes |
| M5 ✅ | Browsers + editors | Chromium AX enabling, text markers, domain, Google Docs, VS Code chat | Matrix in §22 passes |
| M6 ✅ | Context v2 | OCR, clipboard, recorded writing + few-shot (SQLCipher) | Context sections appear within budgets |
| M7 | Quality | Beams, typed-prefix healing, typo hold-back, infer-from-ghost, mid-line | Acceptance-rate dogfooding |
| M8 | Polish | Splicing/parked KV, speculation, pill list, text mirror, n-gram booster, stats, updater | |

---

**M0 result (2026-11-18, M3 16 GB, Gemma 4 E2B Q4_0 from `ggml-org`, Cotypist running at the same time).** Code: `crates/engine` (`nextype-engine`); run with `cargo run --release --bin spike -- models/gemma-4-E2B-it-Q4_0.gguf "<text>" 8`.

- **First token after each keystroke:** p50 **42.6 ms**, p95 50.9 ms. KV prefix reuse works: each keystroke decodes only 1–2 new tokens.
- **Each further token** costs ~18–25 ms, about what the M3's 100 GB/s memory bandwidth allows. 8 tokens therefore take p50 ~170–210 ms.
  - **Consequence:** stream tokens to the overlay (show the first word, then extend it), and keep completions short.
  - **Later speed-up:** the `mtp-gemma-4-*.gguf` draft heads for speculative decoding (P2, may need a newer llama.cpp).
- **Quality is good at word boundaries and broken mid-word** (`"sh"` → `"oud"`; empty output right after a trailing space). Typed-prefix healing (§9.4) is therefore **required**, not optional. Bring it forward from M7.

**M1 result (2026-11-18).** Crates `nextype-core` (snapshot, UTF-16 split, §5.2 gate), `nextype-ax` (AX reads + per-app observer), `nextype` (agent app); build with `scripts/bundle.sh`. Tested in TextEdit:
- caret at the start → "too little text"
- caret at the end → COMPLETE, and the caret x tracks typing (768 → 788 px)
- ← arrow → "caret mid-line"

Lessons for later milestones:
- **NexType must be a real `NSApplication`** (Accessory policy). From a bare CFRunLoop, system-wide AX queries return `kAXErrorCannotComplete` (-25204).
- **Timers and observer sources go in `kCFRunLoopCommonModes`.**
- **A grant made while the app runs isn't seen until relaunch** (`AXIsProcessTrusted` stayed false). Onboarding (M4) must offer "Relaunch NexType" after the user grants access.
- **Some apps (e.g. Ghostex) return -25204** to the system-wide `AXFocusedApplication` query. A fallback to `NSWorkspace.frontmostApplication` may be needed (M5 matrix).
- **The self-signed "NexType Dev" identity works:** the designated requirement is `identifier "dev.nextype.NexType" and certificate leaf = H"…"`, and the grant survived rebuilds.

**M2 result (2026-11-18).** The app now shows ghost text and accepts it. Code: `crates/app/src/{engine,overlay,keys}.rs`; quick typed-prefix healing in `nextype-engine` (`heal_argmax`) and `nextype-core` (`heal_split`, `next_word`, minimal `gemma4_prompt`). Tested in TextEdit with osascript keystrokes (Cotypist quit during the test):
- Typed `I think we should schedule a meeting next week to` → ghost ` discuss the proposal in more detail.`, streamed word by word, in the field's font (Helvetica 12 from `AXFont`), grey, on the caret's baseline (checked on a screenshot).
- **Tab** typed ` discuss ` (next word + trailing space), the ghost advanced without a model run, and **key above Tab** typed the rest. Both were confirmed by an AX re-read; neither key reached the document.
- With no ghost showing, Tab reaches the app normally (a tab character was inserted).
- **Latency at human typing speed (150 ms/char):** first token 25–37 ms, about the M0 spike's numbers. About 70 % of keystrokes needed no model run (infer-from-ghost, §9.6). Each full 16-token ghost takes ~350 ms, but it streams.
- **Healing works:** `"sh"` → `"ould explore…"`, `"sch"` → `"edule a meeting…"`, and a trailing space no longer gives an empty result.

What M2 built, in short:
- **Engine worker:** a thread owns the model. Requests go over an `mpsc` channel and the newest wins: a shared `AtomicU64` id is checked after every token. Text is streamed back to the main queue with `dispatch2`.
- **Healing (§9.4, quick version):** the partial word, with the space before it, is cut from the prompt (max 20 chars). The first tokens are restricted to pieces consistent with it, skipping empty and whitespace-only pieces. A single greedy path; no beams yet.
- **Overlay (§11.1):** borderless `NSPanel` + wrapping label, top-aligned to the caret line. The label spans the field's text width, from the left edge of the caret's paragraph to the mirrored right inset. Its first line is indented to the caret (`firstLineHeadIndent`, like Cotypist's `CompletionOverlayManager_maxHeadIndent`/`maxTailIndent` keys), so the ghost wraps like the field's text. Single-line fields and fields with no room draw one unwrapped line; the pill list is M8.
- **Sentence trim (§10):** the ghost ends at the first sentence end (`.!?…` plus closing quotes, after a word, followed by whitespace), and generation stops there. A user test showed a 16-token ghost running into a second sentence and past the window edge.
- **Accept keys:** an **active session event tap** swallows Tab / key above Tab only while a ghost is visible. Any other key-down clears the "visible" flag at once (so Tab never accepts a stale ghost) and re-arms the debounce. Escape hides the ghost and still reaches the app (§5.3 option 1).
- **Insertion (§12.2):** Unicode key events in chunks of ≤ 20 UTF-16 units (surrogate pairs kept whole), posted to the field's pid, confirmed by an AX re-read after 1 s. **The paste fallback is not built yet**; a failed insert is only logged.

Lessons for later milestones:
- **Set the engine thread's QoS to user-interactive.** At default QoS, first tokens took 0.3–0.6 s inside the agent app (~35 ms in the CLI spike). Cotypist also uses QoS dispatch queues (userInteractive / userInitiated) and doesn't call `beginActivity`.
- **`NSPanel` hides when its app is inactive** (`hidesOnDeactivate` defaults to YES), and NexType is never active. Set it to NO.
- **Zero-length `AXBoundsForRange` is wrong in TextEdit on macOS 27:** it reports y one line too high (173 vs 187). One-character ranges are right. Use the right edge of the character before the caret; use the zero-length range only at a line start.
- **Tab capture:** the active tap works with the Accessibility grant alone, and there's no register/unregister race. Carbon hot keys weren't spiked.
- **Testing:** run each osascript test (type → keys → read the document) as one script. Ghostex sometimes comes to the front between separate commands, and `screencapture` inside `do shell script` once moved focus away. Take screenshots from the shell right after osascript returns.

**M3 result (2026-11-18).** Code: `crates/core/src/{prompt,redact}.rs`; the worker builds the prompt per request (`crates/app/src/engine.rs`). 15 core tests, including a golden prompt.
- **Layout (our own wording):** Gemma 4 user turn = intro, context lines, optional "how I want you to write" block, task line; model turn = the text before the caret. The prompt is about 78 tokens with context; each keystroke still prefills 1 token (KV prefix reuse over 140+ tokens verified live), first token p50 29 ms in the spike.
- **Context lines (§7.1):** date and time (rounded down to 15 min, English names, 12-hour clock), preferred languages, full user name, app name, window title (AX `AXWindow` → `AXTitle`, cut to 100 chars). Empty values are left out. Domain and typing context come with browsers (M5).
- **Budgets (§8.5):** text before caret 350 tokens, context 150, instructions 1000. Counted with the model's tokenizer through a closure, so core has no llama dependency. Over budget: the context drops its least important lines first; the instructions are cut at the end; the before-text is cut at the start, at a sentence start if one is within the first quarter of what remains, else at a word start. The cut point only moves when the budget is exceeded, which keeps the KV prefix stable.
- **Redaction (§7.5):** all 13 patterns, replaced by `[REDACTED]` (labels kept), with one positive and one negative test per kind. Applied to the context block only: not to the text before the caret, and not to the user's own custom instructions.
- **Custom instructions:** one global file, `~/Library/Application Support/NexType/instructions.txt`, re-read on every request. Verified live ("write the word banana" shows up in ghosts). M4 replaces it with settings and per-app overrides.
- **Not built yet:** mid-line layout (§8.6), screen/clipboard/past-writing sections (M6), scaffolding-marker demotion (§8.7), budget redistribution and the 0.85 fill target, dialect line, the "Here is the text I am typing" lead-in.

Lessons:
- **A zero-sized tail must stay valid:** `fit_tail` returns an empty string when nothing fits; the gate (≥ 3 chars) keeps real text from hitting that.
- **AppleScript doc handling:** a new TextEdit document's name changes from "Untitled N" to "Untitled N.rtf" after typing, so `close d` by an old name fails with -1728 and leaves the doc open. The test script now looks it up by either name, and closes only the document it created (never "front document").

**M4 result (2026-11-19).** SwiftUI shell (`shell/`) over the Rust core via uniffi (`crates/ffi`), built and signed by `scripts/bundle.sh` into `target/NexType.app`.
- **Menu bar:** completions/words today, on/off, pause 1 h, turn completions or the Tab shortcut off in the last front app, Settings, Quit.
- **Onboarding window** at launch until Accessibility is granted (with Relaunch) and the model is present.
- **Model download** (§16): pinned `ggml-org/gemma-4-E2B-it-GGUF` commit, Range resume, sha256 check before install, cancel, free-disk check, 3 retries with backoff.
- **Settings:** General (open at login, length, custom instructions) and Apps (per-app completions off, Tab off, own instructions: a 3-field subset of §14.2).
- **Stats** (§9.5): `completionStats.json` updated on each accept.
- Verified by script in TextEdit: exclusion, Tab-off, per-app instructions, stats. Not yet verified: the full download through the UI, login item.

**M5 result (2026-11-19).** Closed with two items deferred and one skipped.
- **Chromium browsers** (Chrome, Brave, Edge, Arc, Dia, Vivaldi, Opera, Comet; `NEEDS_ENHANCED_UI`): NexType sets `AXEnhancedUserInterface` on the app. Chrome answers -25208 but applies it, and rejects `AXManualAccessibility`. Electron apps (Teams, VS Code) take `AXManualAccessibility`. The setting is switched back off in every browser on quit (`applicationWillTerminate` → ffi `stop()`); a kill or crash skips that.
- **Text markers:** when `AXBoundsForRange` fails, the caret comes from `AXSelectedTextMarkerRange` → `AXBoundsForTextMarkerRange` (rects of h ≥ 100 or w ≥ 50 are rejected).
- **Domain:** the outermost `AXWebArea` ancestor's `AXURL`, cached per (pid, window title). It feeds the prompt ("Website: …") and per-site overrides (`domain:<host>`, which matches subdomains and wins over the app override).
- **Paste fallback (§12.2):** when the AX re-read after 1 s shows the text missing, the missing part is pasted with ⌘V. The user's clipboard comes back after 0.5 s, marked transient while ours is on it.
- **Ghost trims:** repeated punctuation runs (`repeat_cut`, Gmail dots), copies of the window title (`copy_cut`, VS Code), and no wrapping with < 80 pt after the caret.
- **Front app:** the system-wide `AXFocusedApplication` query fails (-25204) after a fresh launch and in some apps; it now falls back to `NSWorkspace.frontmostApplication` (the M1 note).
- **Tested working:** Gmail in Chrome, Safari, Teams, the VS Code editor (with its "Screen Reader Optimized" mode on).
- **Deferred:** Google Docs draws text on a canvas, so AX can't read it (the caret is findable via `kix-cursor-caret`). The only AX route is Docs' braille/screen-reader mode, which normal users shouldn't have to turn on; OCR (M6) may unlock it. The VS Code editor font: VS Code reports no `AXFont`, and the width-sample font guess (`overlay::guess_font`, works in Chrome) gets no measurement from it, so the ghost uses the system font.
- **Skipped:** VS Code Chat input (not tested separately).

**M6 progress (2026-11-19).**
- **Clipboard (§7.3) done:** text copied in the last 120 s, up to 2,000 characters, goes in a "Text I copied recently" section after the custom instructions, redacted and cut to a 200-token budget. Items with the nspasteboard.org concealed, transient or auto-generated types are skipped, and so is our own paste fallback's restore. The copy time comes from `changeCount` checked on the 0.25 s app poll; a copy made before launch has no known age and isn't used. Off by default; Settings → General → "Use copied text". Engine check (`spike … 12 "<clipboard>"`): "…held at the" → " conference center on Tuesday, October 24th," without it, " Larkspur Hotel in Sylhet on December 12" with it.
- **Screen text (§7.2) done:** the Swift shell (`ScreenText.swift`) asks the core for the focused allowed field (ffi `screen_target`; none in excluded apps or secure fields), captures that window with `SCScreenshotManager` (long edge ≤ 2000 px), crops to the part above the field, and runs `VNRecognizeTextRequest` (`.accurate`, automatic language). It keeps observations with confidence ≥ 0.3 and ≥ 2 words, top to bottom, and hands the text back (`set_screen_text`, keyed by pid + window title). It re-reads when the window or field changes, at most every 30 s otherwise, from the shell's 2 s timer. Prompt: "Text on screen above where I am typing", after the instructions and before the clipboard, redacted, `fit_tail` to 600 tokens (keeps the bottom, nearest the field). Off by default (Settings → "Use text on screen"; needs Screen Recording). User test in Gmail: ~27 lines / 600 chars read, suggestions fit the email; the first request after a new read prefills ~240 tokens (~0.5 s first token), then KV reuse is back.
- **Fix found while testing:** Chrome turns a typed trailing space into U+00A0, so after Tab accepted "out " the ghost no longer matched and was replaced. `ax::snapshot` now maps U+00A0 to a space.
- **Past writing (§7.4, §13) built:** `crates/app/src/history.rs`. SQLCipher via `rusqlite` 0.43 `bundled-sqlcipher` (builds against CommonCrypto, no OpenSSL), raw 256-bit key from `/dev/urandom` kept in the Keychain (`security-framework`, service `dev.nextype.NexType.database`), file `~/Library/Application Support/NexType/history.db`, table `user_inputs (created_at, bundle_id, domain, text)`. A session is one app + window title (shortcut: web fields move while typing, so position can't be part of the key); its text (last 4,000 chars, ≥ 33) is saved when the user moves to another window (any snapshot with another key, even where completions are off), when the front app changes, and on quit (a first version saved only on entering another allowed field, so nothing was saved; user-tested fix). Examples are fetched once per session: 2 longest + 2 newest from the same app, 1 newest from another app, last 30 days, ≥ 100 chars, at most 5. Prompt: "Other texts I wrote earlier, to show my style", after the instructions and before the screen text, redacted, 400 tokens split evenly. Off by default (Settings → "Learn from my writing", with a count and Delete All). Checked: a save from TextEdit, and the file header is encrypted. Not built: language filter, per-app/domain recording off, the Typing History UI per app.
- **Next:** M6 exit check, then M7 (quality).

## 24. Open questions and optional experiments

| Question | Impact | How to settle |
|---|---|---|
| Exact section order and screenshot/clipboard/environment budgets | Low (we tune our own) | Needs a decompiler (Ghidra/IDA aren't installed here): decompile `0x10014af2c`. Otherwise tune |
| Context size, beam count, length-penalty values | Low | Tune empirically (§9.1 has the runtime hints) |
| ~~Gemma 4 template~~ | — | **Resolved** (§8.2) |
| Secure Input holder detection API | Low | Prototype |
| UI layer: all-Rust AppKit vs SwiftUI shell + UniFFI (§20) | High (affects project layout) | **User decides before M4** |
| ~~Tab capture: Carbon hot keys vs an active tap~~ | — | **Settled in M2:** active tap (§23 M2 result). Revisit only if a slow callback ever delays typing |
| Built-in flags for ChatGPT/Gemini/Vellum/Cumbersome/WhatsApp and `*.atlassian.net`; use of the 53-app list (§17.2) | Low | Decompile `0x10002bd14`, or test per app |
| Text-marker API behaviour on macOS 27 | Medium | Prototype in M5 |

**Runtime log experiment: done 2026-11-18, closed.** I set `Logging_fileTeeDirectory` (as a plain path and as a `file://` URL), `Logging_llamaFilterVerbose = false`, and `Logging_logLevel` = `debug`/`trace`/`verbose`/`all`/`0`/`1`, restarting Cotypist each time and focusing a TextEdit field. Results:
- **No app logs and no tee file.** These keys do nothing in release build 85.
- llama's own messages go through a muted callback.
- Only the fork's `ggml-metal:` lines reach stderr; they gave the §9.1 numbers.

All keys were deleted afterwards and Cotypist was relaunched normally. The remaining route is the app's support-diagnostics mode (`startDiagnostics` URL command: restarts and records for 10 minutes, meant to be sent to Cotypist support). Not needed.

---

## 25. Reference files in this repo

| File | Content |
|---|---|
| `docs/HOW-COTYPIST-WORKS.md` | How Cotypist works, with evidence |
| `docs/reference/decoded-strings.tsv` | 1,700+ strings recovered from build 85's encrypted literals (all tunable keys, prompt labels, DOM ids, override fields, settings keys) |
| `docs/reference/plain-app-strings.tsv` | App-owned plain strings (SQL, regexes, bundle-ID lists, UI copy, templates) |
| `docs/reference/imported-symbols.txt` | Every imported framework symbol (exact API surface) |
| `docs/reference/objc-selectors.txt` | Every Objective-C selector the app sends |
| `research/tools/*.py` | The static-analysis scripts used (`arrays.py` dumps the bundle-ID arrays in §17). Run `arm64_scan.py <binary> work/scan.pkl` first, then `q.py`, `deobf.py work`, `xref.py`, `getter.py`. Addresses are specific to build 85 (`xref.py` hard-codes the `__text` bounds) |

Vendor tokens, price IDs, developer paths and long key blobs are redacted in the reference files.
