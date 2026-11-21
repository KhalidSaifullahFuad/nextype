//! UI Automation events: focus changes anywhere, plus text and caret/selection changes in the
//! focused text control. Each one posts `WM_FIELD` to the UI thread, which debounces it into a
//! field read, so NexType reacts when something happens instead of polling the field.
//!
//! UIA calls event handlers on its own MTA threads, and a handler must not add or remove
//! handlers itself. So handlers only post messages; a watcher thread (MTA, with its own
//! `IUIAutomation`) moves the text handlers to each newly focused element.

use std::sync::mpsc::{Receiver, Sender, channel};

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
use windows::core::{Ref, Result, implement};

use crate::WM_FIELD;

/// `WM_FIELD` wParam: focus moved (maybe to another app).
pub const FOCUS: usize = 0;
/// `WM_FIELD` wParam: the focused field's text or caret changed.
pub const EDIT: usize = 1;

/// Text-control events we watch on the focused element.
const EDIT_EVENTS: [UIA_EVENT_ID; 2] = [UIA_Text_TextChangedEventId, UIA_Text_TextSelectionChangedEventId];

/// An element handed from UIA's handler thread to the watcher thread. Both are MTA threads,
/// where COM interface pointers may be used from any thread.
struct Element(IUIAutomationElement);
unsafe impl Send for Element {}

fn post(ui_thread: u32, kind: usize) {
    unsafe {
        let _ = PostThreadMessageW(ui_thread, WM_FIELD, WPARAM(kind), LPARAM(0));
    }
}

#[implement(IUIAutomationFocusChangedEventHandler)]
struct FocusHandler {
    ui_thread: u32,
    own_pid: i32,
    watcher: Sender<Element>,
}

impl IUIAutomationFocusChangedEventHandler_Impl for FocusHandler_Impl {
    fn HandleFocusChangedEvent(&self, sender: Ref<'_, IUIAutomationElement>) -> Result<()> {
        let el = sender.ok()?;
        // Our own tray menu taking focus is not a field change.
        if unsafe { el.CurrentProcessId() }.is_ok_and(|p| p == self.own_pid) {
            return Ok(());
        }
        let _ = self.watcher.send(Element(el.clone()));
        post(self.ui_thread, FOCUS);
        Ok(())
    }
}

#[implement(IUIAutomationEventHandler)]
struct EditHandler {
    ui_thread: u32,
}

impl IUIAutomationEventHandler_Impl for EditHandler_Impl {
    fn HandleAutomationEvent(&self, _sender: Ref<'_, IUIAutomationElement>, _id: UIA_EVENT_ID) -> Result<()> {
        post(self.ui_thread, EDIT);
        Ok(())
    }
}

/// Start watching on a background thread. False if UI Automation events are unavailable;
/// the caller's poll then has to do all the work.
pub fn spawn(ui_thread: u32) -> bool {
    let (ready_tx, ready_rx) = channel();
    std::thread::Builder::new()
        .name("uia-events".into())
        .spawn(move || {
            let started = unsafe { start(ui_thread) };
            let _ = ready_tx.send(started.is_ok());
            match started {
                Ok((auto, rx)) => watch(&auto, ui_thread, rx),
                Err(e) => eprintln!("UI Automation events unavailable: {e}"),
            }
        })
        .is_ok()
        && ready_rx.recv().unwrap_or(false)
}

unsafe fn start(ui_thread: u32) -> Result<(IUIAutomation, Receiver<Element>)> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let auto: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let (tx, rx) = channel();
        let focus: IUIAutomationFocusChangedEventHandler =
            FocusHandler { ui_thread, own_pid: GetCurrentProcessId() as i32, watcher: tx.clone() }.into();
        auto.AddFocusChangedEventHandler(None, &focus)?;
        // Watch the field that already has focus when NexType starts.
        if let Ok(el) = auto.GetFocusedElement() {
            let _ = tx.send(Element(el));
        }
        Ok((auto, rx))
    }
}

/// Keep the text handlers on the focused text control, for as long as the process runs.
fn watch(auto: &IUIAutomation, ui_thread: u32, rx: Receiver<Element>) {
    let edit: IUIAutomationEventHandler = EditHandler { ui_thread }.into();
    let mut current: Option<IUIAutomationElement> = None;
    while let Ok(Element(mut el)) = rx.recv() {
        // Focus can hop several times in a burst (app switch); only the last one matters.
        while let Ok(Element(next)) = rx.try_recv() {
            el = next;
        }
        if let Some(old) = current.take() {
            if unsafe { auto.CompareElements(&old, &el) }.is_ok_and(|same| same.as_bool()) {
                current = Some(old);
                continue;
            }
            for id in EDIT_EVENTS {
                let _ = unsafe { auto.RemoveAutomationEventHandler(id, &old, &edit) };
            }
        }
        if !is_text_control(&el) {
            continue;
        }
        let mut ok = true;
        for id in EDIT_EVENTS {
            // Subtree: rich editors (Word, web editors) raise text events on child elements.
            ok &= unsafe { auto.AddAutomationEventHandler(id, &el, TreeScope_Subtree, None, &edit) }.is_ok();
        }
        if !ok {
            eprintln!("could not watch text events in the focused field; the slow poll covers it");
        }
        current = Some(el);
    }
}

fn is_text_control(el: &IUIAutomationElement) -> bool {
    let ct = unsafe { el.CurrentControlType() }.unwrap_or_default();
    ct == UIA_EditControlTypeId || ct == UIA_DocumentControlTypeId || ct == UIA_ComboBoxControlTypeId
}
