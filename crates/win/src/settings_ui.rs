//! The Settings window: a WebView2 page (`assets/settings.html`) in a Win32 window with the
//! Windows 11 Mica backdrop, styled after the Windows settings pages (PowerToys). It runs on the
//! UI thread, inside the app's message loop. The page posts JSON messages; Rust answers by
//! calling `nx.load(state)` in the page.

use std::cell::RefCell;
use std::num::NonZeroIsize;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::json;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::Graphics::Dwm::{
    DWMWA_CAPTION_BUTTON_BOUNDS, DWMWINDOWATTRIBUTE, DwmDefWindowProc, DwmExtendFrameIntoClientArea, DwmGetWindowAttribute,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BLACK_BRUSH, ClientToScreen, CombineRgn, CreateRectRgn, DeleteObject, FillRect, GetStockObject, HBRUSH, HDC, RGN_DIFF, SetWindowRgn,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RegCloseKey, RegDeleteValueW, RegGetValueW, RegOpenKeyExW,
    RegSetValueExW,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use wry::{Rect, WebContext, WebView, WebViewBuilder};

use crate::settings::{self, Settings};

const RESHAPE_TIMER: usize = 1;
const PAGE: &str = include_str!("../assets/settings.html");
/// Size at 100% scaling.
const SIZE: (i32, i32) = (1040, 720);
const MIN_SIZE: (i32, i32) = (640, 480);

/// DWM attributes not named in this `windows` version's enum.
const DWMWA_USE_IMMERSIVE_DARK_MODE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(20);
const DWMWA_SYSTEMBACKDROP_TYPE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(38);
const DWMSBT_NONE: i32 = 1;
const DWMWA_CAPTION_COLOR: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(35);
/// Windows 11 window background (the page's --window): dark, light. COLORREF is 0x00BBGGRR.
const WINDOW_BG: (u32, u32) = (0x0020_2020, 0x00F3_F3F3);

struct Open {
    hwnd: HWND,
    /// The WebView's own child window (shaped around the caption buttons).
    child: HWND,
    webview: WebView,
    _context: WebContext,
}

thread_local! {
    static WINDOW: RefCell<Option<Open>> = const { RefCell::new(None) };
    /// Whether the window got the Mica backdrop.
    static MICA: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Apps seen in a text field this session (exe, name), newest first, for "Add an app".
static RECENT: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Remember an app the user typed in.
pub fn saw_app(exe: &str, name: &str) {
    let Ok(mut r) = RECENT.lock() else { return };
    if r.first().is_some_and(|(e, _)| e == exe) {
        return;
    }
    r.retain(|(e, _)| e != exe);
    r.insert(0, (exe.to_owned(), name.to_owned()));
    r.truncate(30);
}

/// Open the window, or bring it to the front.
pub fn open() {
    let existing = WINDOW.with(|w| w.borrow().as_ref().map(|o| o.hwnd));
    if let Some(hwnd) = existing {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    match create() {
        Ok(o) => WINDOW.with(|w| *w.borrow_mut() = Some(o)),
        Err(e) => eprintln!("settings window: {e}"),
    }
}

/// Send the current settings to the page, if it's open (after a change from the tray menu).
pub fn refresh() {
    WINDOW.with(|w| {
        if let Some(o) = w.borrow().as_ref() {
            push_state(&o.webview);
            push_frame(&o.webview, o.hwnd);
        }
    });
}

struct Parent(HWND);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let h = NonZeroIsize::new(self.0.0 as isize).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(h));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

fn create() -> Result<Open, String> {
    let hwnd = unsafe {
        let inst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = w!("NexTypeSettings");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: class,
            hIcon: LoadImageW(Some(inst.into()), PCWSTR(1 as *const u16), IMAGE_ICON, 0, 0, LR_DEFAULTSIZE)
                .map(|h| HICON(h.0))
                .unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc); // fails harmlessly when already registered
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("NexType Settings"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            SIZE.0,
            SIZE.1,
            None,
            None,
            Some(inst.into()),
            None,
        )
        .map_err(|e| e.to_string())?;
        // Scale to the monitor the window opened on, centred there.
        let scale = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
        let (w, h) = ((SIZE.0 as f64 * scale) as i32, (SIZE.1 as f64 * scale) as i32);
        let mut work = RECT::default();
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let (x, y) = (work.left + (work.right - work.left - w).max(0) / 2, work.top + (work.bottom - work.top - h).max(0) / 2);
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER);
        let dark: i32 = dark_mode() as i32;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        // No Mica: Windows paints the title-bar strip of a Mica window lighter than its body, which
        // showed as a band through the transparent page. The page paints the solid Windows 11
        // background instead, and the strip behind the caption buttons gets the same color.
        // The frame still extends over the client area so DWM draws its caption buttons there.
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_NONE as *const _ as _, 4);
        let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });
        set_caption_color(hwnd);
        let mica = false;
        MICA.with(|m| m.set(mica));
        // Apply WM_NCCALCSIZE below: the page draws its own title bar, like WinUI apps.
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
        hwnd
    };

    // WebView2 paints this before the page loads; transparent so Mica shows, not a white flash.
    unsafe { std::env::set_var("WEBVIEW2_DEFAULT_BACKGROUND_COLOR", "00000000") };
    let data = std::env::var("LOCALAPPDATA").map(|b| std::path::PathBuf::from(b).join("NexType").join("WebView2")).ok();
    let mut context = WebContext::new(data);
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_html(PAGE)
        .with_transparent(true)
        .with_background_color((0, 0, 0, 0))
        .with_devtools(cfg!(debug_assertions))
        .with_bounds(client_rect(hwnd))
        .with_ipc_handler(|req| on_message(req.body()))
        .build_as_child(&Parent(hwnd))
        .map_err(|e| e.to_string())?;
    unsafe {
        // Not ShowWindow: its first call in a process takes the launcher's show state, which is
        // "hidden" or "minimized" when NexType was started that way (login, a hidden launch).
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(hwnd);
    }
    let _ = webview.focus();
    // wry uses a newer `windows` crate; the handle is the same pointer.
    let child = HWND(wry::WebViewExtWindows::hwnd(&webview).0 as _);
    shape_child(hwnd, child);
    Ok(Open { hwnd, child, webview, _context: context })
}

fn client_rect(hwnd: HWND) -> Rect {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    Rect {
        position: wry::dpi::PhysicalPosition::new(0, 0).into(),
        size: wry::dpi::PhysicalSize::new((rc.right - rc.left).max(1) as u32, (rc.bottom - rc.top).max(1) as u32).into(),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // The caption buttons are Windows' own, drawn by DWM in the corner the WebView leaves free
    // (`shape_child`); DWM handles their hover, clicks and the Snap Layouts flyout.
    let mut handled = LRESULT(0);
    if unsafe { DwmDefWindowProc(hwnd, msg, w, l, &mut handled) }.as_bool() {
        return handled;
    }
    match msg {
        // DWM missed it: when maximized it hit-tests its buttons 8px above where it draws them
        // (the frame that hangs off screen), so the lower part of each button got HTCLIENT and
        // clicks did nothing. Map the whole visible button strip to the buttons ourselves.
        WM_NCHITTEST => {
            if let Some(hit) = caption_hit(hwnd, l) {
                return LRESULT(hit as isize);
            }
            unsafe { DefWindowProcW(hwnd, msg, w, l) }
        }
        WM_NCCALCSIZE if w.0 != 0 => {
            // No caption: keep the side and bottom frame (resizing, shadow) and give the top to
            // the client area, where the page draws the title bar. A maximized window hangs over
            // the screen edge by its frame, so then the top keeps that much back.
            let params = unsafe { &mut *(l.0 as *mut NCCALCSIZE_PARAMS) };
            let top = params.rgrc[0].top;
            let r = unsafe { DefWindowProcW(hwnd, msg, w, l) };
            params.rgrc[0].top = top + if unsafe { IsZoomed(hwnd) }.as_bool() { frame_thickness(hwnd) } else { 0 };
            r
        }
        WM_SIZE => {
            // try_with: the window is still being created inside `create` (no WebView yet).
            let _ = WINDOW.try_with(|win| {
                if let Ok(b) = win.try_borrow() {
                    if let Some(o) = b.as_ref() {
                        let _ = o.webview.set_bounds(client_rect(hwnd));
                        shape_child(hwnd, o.child);
                        push_frame(&o.webview, hwnd);
                    }
                }
            });
            // DWM moves the caption buttons after WM_SIZE (maximize, restore); shape again once it has.
            unsafe { SetTimer(Some(hwnd), RESHAPE_TIMER, 100, None) };
            LRESULT(0)
        }
        WM_TIMER if w.0 == RESHAPE_TIMER => {
            unsafe { let _ = KillTimer(Some(hwnd), RESHAPE_TIMER); }
            let _ = WINDOW.try_with(|win| {
                if let Ok(b) = win.try_borrow() {
                    if let Some(o) = b.as_ref() {
                        shape_child(hwnd, o.child);
                        push_frame(&o.webview, hwnd);
                    }
                }
            });
            LRESULT(0)
        }
        WM_ACTIVATE => {
            let _ = WINDOW.try_with(|win| {
                if let Ok(b) = win.try_borrow() {
                    if let Some(o) = b.as_ref() {
                        push_frame(&o.webview, hwnd);
                        // Keyboard focus back into the page when the window is activated.
                        if w.0 & 0xFFFF != 0 {
                            let _ = o.webview.focus();
                        }
                    }
                }
            });
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            // Black is "transparent" over the extended frame: Mica shows wherever the WebView
            // hasn't drawn yet (opening, resizing).
            let mut rc = RECT::default();
            unsafe {
                let _ = GetClientRect(hwnd, &mut rc);
                FillRect(HDC(w.0 as _), &rc, HBRUSH(GetStockObject(BLACK_BRUSH).0));
            }
            LRESULT(1)
        }
        WM_GETMINMAXINFO => {
            let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f64 / 96.0;
            let info = unsafe { &mut *(l.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize.x = (MIN_SIZE.0 as f64 * scale) as i32;
            info.ptMinTrackSize.y = (MIN_SIZE.1 as f64 * scale) as i32;
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            // Light/dark switch while open: the title bar follows; the page follows by itself.
            let dark: i32 = dark_mode() as i32;
            unsafe {
                let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
            }
            set_caption_color(hwnd);
            refresh();
            LRESULT(0)
        }
        WM_CLOSE => {
            // Drop the WebView before its parent window goes.
            if let Some(o) = WINDOW.with(|win| win.borrow_mut().take()) {
                drop(o.webview);
            }
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Message {
    Ready,
    Save { settings: Settings },
    OpenFolder,
    /// Title bar actions: "drag", "resizeTop", "minimize", "maximize", "close", "menu".
    Window { action: String },
}

fn on_message(body: &str) {
    let msg = match serde_json::from_str::<Message>(body) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("settings window: bad message ({e}): {body}");
            return;
        }
    };
    match msg {
        Message::Ready => refresh_later(),
        Message::Save { settings: new } => {
            let old = settings::get();
            if new.open_at_login != old.open_at_login {
                set_open_at_login(new.open_at_login);
            }
            settings::replace(new);
            crate::settings_changed();
        }
        Message::Window { action } => window_action(&action),
        Message::OpenFolder => {
            let dir: Vec<u16> = settings::path().parent().map(|p| p.display().to_string()).unwrap_or_default().encode_utf16().chain([0]).collect();
            unsafe {
                ShellExecuteW(None, w!("open"), PCWSTR(dir.as_ptr()), None, None, SW_SHOWNORMAL);
            }
        }
    }
}

/// The IPC handler runs inside WebView2's callback, where the WebView is already borrowed;
/// answer from the message loop instead.
fn refresh_later() {
    let hwnd = WINDOW.with(|w| w.try_borrow().ok().and_then(|b| b.as_ref().map(|o| o.hwnd)));
    crate::post_refresh_settings(hwnd);
}

/// Title-bar color = the page background, so the caption buttons sit on the same surface.
fn set_caption_color(hwnd: HWND) {
    let color: u32 = if dark_mode() { WINDOW_BG.0 } else { WINDOW_BG.1 };
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, &color as *const _ as _, 4);
    }
}

/// Give the WebView's window every pixel of the client area except the caption buttons, which
/// DWM draws on the window itself (Mica shows through the black the parent paints there).
fn shape_child(hwnd: HWND, child: HWND) {
    unsafe {
        let mut buttons = RECT::default();
        if DwmGetWindowAttribute(hwnd, DWMWA_CAPTION_BUTTON_BOUNDS, &mut buttons as *mut _ as _, size_of::<RECT>() as u32).is_err() {
            return;
        }
        // Caption button bounds are in window coordinates; the child is in client coordinates.
        let (mut win, mut origin) = (RECT::default(), POINT::default());
        let _ = GetWindowRect(hwnd, &mut win);
        let _ = ClientToScreen(hwnd, &mut origin);
        let (dx, dy) = (origin.x - win.left, origin.y - win.top);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let all = CreateRectRgn(0, 0, client.right, client.bottom);
        // The hole runs from the buttons to the top-right corner of the client area: a maximized
        // window's buttons start above the client top, and a stale rectangle mid-resize must not
        // leave a strip where clicks land on the page instead of the buttons.
        let hole = CreateRectRgn(buttons.left - dx, 0, client.right, buttons.bottom - buttons.top);
        CombineRgn(Some(all), Some(all), Some(hole), RGN_DIFF);
        let _ = DeleteObject(hole.into());
        // The system owns the region from here on.
        SetWindowRgn(child, Some(all), true);
    }
}

/// Which caption button (HTMINBUTTON / HTMAXBUTTON / HTCLOSE) is under the cursor, measured on
/// the strip the buttons are drawn in: from the client top, as tall as DWM's button bounds.
fn caption_hit(hwnd: HWND, l: LPARAM) -> Option<u32> {
    unsafe {
        let mut buttons = RECT::default();
        DwmGetWindowAttribute(hwnd, DWMWA_CAPTION_BUTTON_BOUNDS, &mut buttons as *mut _ as _, size_of::<RECT>() as u32).ok()?;
        let (mut win, mut origin) = (RECT::default(), POINT::default());
        let _ = GetWindowRect(hwnd, &mut win);
        let _ = ClientToScreen(hwnd, &mut origin);
        let (x, y) = ((l.0 & 0xFFFF) as i16 as i32 - origin.x, ((l.0 >> 16) & 0xFFFF) as i16 as i32 - origin.y);
        let left = buttons.left - (origin.x - win.left);
        let width = buttons.right - buttons.left;
        if width <= 0 || x < left || x >= left + width || y < 0 || y >= buttons.bottom - buttons.top {
            return None;
        }
        Some(match (x - left) * 3 / width {
            0 => HTMINBUTTON,
            1 => HTMAXBUTTON,
            _ => HTCLOSE,
        })
    }
}

/// Frame thickness of a maximized window's overhang at the top.
fn frame_thickness(hwnd: HWND) -> i32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    unsafe { GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) }
}

fn settings_hwnd() -> Option<HWND> {
    WINDOW.with(|w| w.try_borrow().ok().and_then(|b| b.as_ref().map(|o| o.hwnd)))
}

/// The page's title bar asks for a window action. Drag and resize hand the pressed mouse button
/// to Windows' own move/size loop, so snapping and Aero Shake work as with a normal title bar.
fn window_action(action: &str) {
    let Some(hwnd) = settings_hwnd() else { return };
    unsafe {
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let at = LPARAM(((p.y as u16 as isize) << 16) | p.x as u16 as isize);
        match action {
            "drag" | "resizeTop" => {
                let hit = if action == "drag" { HTCAPTION } else { HTTOP };
                let _ = ReleaseCapture();
                let _ = PostMessageW(Some(hwnd), WM_NCLBUTTONDOWN, WPARAM(hit as usize), at);
            }
            "minimize" => {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
            "maximize" => {
                let _ = ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
            }
            "close" => {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            "menu" => {
                // The window menu (Restore, Move, Size, Close) at the cursor, as on a right-click
                // of a normal title bar.
                let menu = GetSystemMenu(hwnd, false);
                let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, p.x, p.y, None, hwnd, None);
                if cmd.0 != 0 {
                    let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(cmd.0 as usize), LPARAM(0));
                }
            }
            _ => {}
        }
    }
}

/// Window state the page's title bar shows: maximize vs restore glyph, dimmed when inactive.
fn push_frame(webview: &WebView, hwnd: HWND) {
    let (max, active) = unsafe { (IsZoomed(hwnd).as_bool(), GetForegroundWindow() == hwnd) };
    let inset = caption_inset(hwnd);
    let _ = webview.evaluate_script(&format!("window.nx && nx.frame({{ maximized: {max}, active: {active}, inset: {inset} }})"));
}

/// Width the caption buttons take from the right of the client area, in CSS pixels (like
/// AppWindow.TitleBar.RightInset), so the title bar content stops short of them.
fn caption_inset(hwnd: HWND) -> f64 {
    unsafe {
        let mut buttons = RECT::default();
        if DwmGetWindowAttribute(hwnd, DWMWA_CAPTION_BUTTON_BOUNDS, &mut buttons as *mut _ as _, size_of::<RECT>() as u32).is_err() {
            return 138.0;
        }
        let (mut win, mut origin, mut client) = (RECT::default(), POINT::default(), RECT::default());
        let _ = GetWindowRect(hwnd, &mut win);
        let _ = ClientToScreen(hwnd, &mut origin);
        let _ = GetClientRect(hwnd, &mut client);
        let left = buttons.left - (origin.x - win.left);
        let scale = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
        ((client.right - left).max(0) as f64 / scale).round()
    }
}

fn push_state(webview: &WebView) {
    let s = settings::get();
    let recent: Vec<_> = RECENT.lock().map(|r| r.iter().map(|(exe, name)| json!({ "exe": exe, "name": name })).collect()).unwrap_or_default();
    let state = json!({
        "settings": s,
        "recent": recent,
        "offByDefault": nextype_core::OFF_BY_DEFAULT,
        "accent": accent_colors(),
        "dark": dark_mode(),
        "mica": MICA.with(|m| m.get()),
        "version": env!("CARGO_PKG_VERSION"),
        "folder": settings::path().parent().map(|p| p.display().to_string()),
        "model": crate::model_path(),
        "now": settings::now(),
    });
    let _ = webview.evaluate_script(&format!("window.nx && nx.load({state})"));
}

/// The Windows accent as (shade for dark theme, shade for light theme), `#rrggbb`. Windows 11
/// apps take these from the accent palette: "AccentLight2" on dark, "AccentDark1" on light.
/// `AccentPalette` holds 8 RGBA colors from lightest to darkest, the accent itself 4th.
fn accent_colors() -> (String, String) {
    let mut pal = [0u8; 32];
    let mut size = pal.len() as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent"),
            w!("AccentPalette"),
            RRF_RT_REG_BINARY,
            None,
            Some(pal.as_mut_ptr() as _),
            Some(&mut size),
        )
    }
    .is_ok()
        && size == 32;
    if !ok {
        return ("#4cc2ff".into(), "#0067c0".into());
    }
    let hex = |i: usize| format!("#{:02x}{:02x}{:02x}", pal[i * 4], pal[i * 4 + 1], pal[i * 4 + 2]);
    (hex(1), hex(4))
}
/// Whether apps use the dark theme (Settings → Personalization → Colors).
pub fn dark_mode() -> bool {
    let mut value = 1u32;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as _),
            Some(&mut size),
        )
    };
    ok.is_ok() && value == 0
}

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");

/// Add or remove NexType in the current user's Run key.
pub fn set_open_at_login(on: bool) {
    unsafe {
        let mut key = Default::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_SET_VALUE, &mut key).is_err() {
            eprintln!("open at login: can't open the Run key");
            return;
        }
        if on {
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            let cmd: Vec<u16> = format!("\"{exe}\"").encode_utf16().chain([0]).collect();
            let bytes = std::slice::from_raw_parts(cmd.as_ptr() as *const u8, cmd.len() * 2);
            let _ = RegSetValueExW(key, w!("NexType"), None, REG_SZ, Some(bytes));
        } else {
            let _ = RegDeleteValueW(key, w!("NexType"));
        }
        let _ = RegCloseKey(key);
    }
}
