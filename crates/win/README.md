# NexType for Windows (first cut)

Win32 + UI Automation runtime that reuses `nextype-core` (gate, prompt, trims) and
`nextype-engine` (llama.cpp). The macOS crates (`ax`, `app`, `ffi`) don't build on Windows, so
always build this package by name.

## Build

Needs the MSVC toolchain, CMake and LLVM (bindgen needs `libclang.dll`):

```powershell
winget install LLVM.LLVM
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
cargo build --release -p nextype-win            # CPU
cargo build --release -p nextype-win -F vulkan  # GPU, needs the Vulkan SDK
cargo build --release -p nextype-win -F cuda    # NVIDIA, needs the CUDA toolkit
```

## Model

Same pinned model as the Mac app:

```powershell
$dir = "$env:LOCALAPPDATA\NexType\models"; mkdir -Force $dir
curl.exe -L -o "$dir\gemma-4-E2B-it-Q4_0.gguf" https://huggingface.co/ggml-org/gemma-4-E2B-it-GGUF/resolve/2a8538c2e2d4687953a86c422c7c623cfd022880/gemma-4-E2B-it-Q4_0.gguf
# sha256 8e30dff3ac4c8434c49a7036fa15564bdbb6044e42bf04550bf1a096ad7e6a52
```

## Run

```powershell
target\release\nextype.exe [path\to\model.gguf]
```

The console logs every field it sees and why it completes or skips. Tray icon → Quit to exit.
Tab accepts the next word, the key above Tab (`` ` ``) accepts the whole ghost, Escape hides it.

## What works / what doesn't yet

- Reads the focused Edit / Document control through UIA `TextPattern` (selection = caret).
- Skips password fields (`IsPassword`), search boxes, password managers, and the off-by-default
  apps (terminals, Explorer, Start search) listed in `nextype-core`.
- Ghost overlay: layered, click-through, topmost, per-pixel alpha, sized from the caret height.
  Single line only (no wrapping), Segoe UI rather than the field's font.
- Accept by pasting (`clip.rs`): text goes on the clipboard kept out of clipboard history,
  Ctrl+V, then the user's clipboard is restored. Typed Unicode events garbled fast input in
  Notepad. Can't paste into elevated (admin) windows.
- Polling (250 ms) + key-hook debounce instead of UIA event handlers.
- Not yet: settings window, per-app overrides, website detection, clipboard/screen/history
  context, open-at-login, installer.
