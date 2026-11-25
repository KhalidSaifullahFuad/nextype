# NexType beta

AI autocomplete in any text field on your Mac, run on your Mac. Nothing you type leaves it.

**Needs:** a Mac with Apple silicon (M1 or later), macOS 14 or later, about 3 GB free for the model.

## Install

1. Download `NexType-<version>.dmg` (or the `.zip`), open it, and drag **NexType** into **Applications**.
2. Open NexType. macOS says it can't check the app, because this beta isn't notarized by Apple. Click **Done**.
3. Open **System Settings → Privacy & Security**, scroll down, and click **Open Anyway** next to NexType. Confirm with your password or Touch ID.

   Or, in Terminal: `xattr -dr com.apple.quarantine /Applications/NexType.app`, then open it.
4. Follow the setup: allow **Accessibility** (NexType reads the field you type in), then relaunch NexType when it asks.
5. Let it download the model (2.8 GB, once).

Start typing in Mail, Notes, Messages, Slack or a browser. Grey text appears ahead of the cursor:

- **Tab**: accept the next word
- **`** (the key above Tab): accept the whole suggestion
- **Esc**: hide it

The menu bar icon turns suggestions off, pauses them or turns them off for one app. Open NexType again to see Settings.

## Updating

Quit NexType, replace the app in Applications, open it. Accessibility stays allowed.

## Known limits

- Apple silicon only.
- Suggestions only appear at the end of a line, not in the middle of text.
- Google Docs isn't supported.
- Keyboards other than US layout may accept with the wrong key.

## Feedback

Tell Khalid what worked, what didn't, and in which app.
