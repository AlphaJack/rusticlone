//! The taskbar icon, on a thread of its own that runs the message loop its hidden window needs.

use std::sync::mpsc;

use anyhow::Result;
use windows_sys::{
    Win32::{
        Foundation::ERROR_SUCCESS,
        System::{
            Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_DWORD, RegGetValueW,
                RegNotifyChangeKeyValue, RegOpenKeyExW,
            },
            Threading::GetCurrentThreadId,
        },
        UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, TranslateMessage,
            WM_APP, WM_QUIT,
        },
    },
    w,
};

use super::{Row, Switch, icon, ink};

/// Where Windows says whether the taskbar is light or dark.
const PERSONALIZE: *const u16 = w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");

/// What the icon's thread shows: the web UI logged in, the collections and the switches.
type Shown = (String, Vec<Row>, Vec<(Switch, Option<bool>)>);

/// Hands the collections' health to the icon's thread, and wakes it.
pub(in crate::server) struct Tray {
    thread: u32,
    shown: mpsc::Sender<Shown>,
}

/// Runs `serve` with the icon when its user runs it; as the system service or under the tests, alone.
pub(in crate::server) fn beside(open: bool, serve: impl FnOnce(Option<Tray>) -> Result<()>) -> Result<()> {
    if !super::attended(open) {
        return serve(None);
    }
    let (shown, received) = mpsc::channel();
    let (started, thread) = mpsc::channel();
    let icon = std::thread::spawn(move || run(&started, &received));
    // Blocked until the process ends, as it waits for the registry.
    std::thread::spawn(watch_theme);
    let Ok(thread) = thread.recv() else { return serve(None) };
    let served = serve(Some(Tray { thread, shown }));
    // Ends the loop, so that the icon is removed rather than left until the pointer passes over it.
    post(thread, WM_QUIT);
    _ = icon.join();
    served
}

#[expect(unsafe_code, reason = "posts a message without parameters to a thread")]
fn post(thread: u32, message: u32) {
    // SAFETY: no pointers are passed.
    unsafe { PostThreadMessageW(thread, message, 0, 0) };
}

/// Shows what [`Tray::show`] sends, until [`WM_QUIT`]. The icon, a thread local, is removed as the thread
/// ends.
#[expect(unsafe_code, reason = "runs the thread's message loop")]
fn run(started: &mpsc::Sender<u32>, received: &mpsc::Receiver<Shown>) {
    // SAFETY: an all-zero MSG is valid, and every call is given a message to fill.
    let mut message: MSG = unsafe { std::mem::zeroed() };
    // Creates the thread's message queue, so that no message posted from now on is lost.
    unsafe { PeekMessageW(&raw mut message, std::ptr::null_mut(), WM_APP, WM_APP, PM_NOREMOVE) };
    if started.send(unsafe { GetCurrentThreadId() }).is_err() {
        return;
    }
    icon::open_clicked();
    while unsafe { GetMessageW(&raw mut message, std::ptr::null_mut(), 0, 0) } > 0 {
        if message.hwnd.is_null() && message.message == WM_APP {
            for (login, rows, switches) in received.try_iter() {
                icon::show(login, &rows, &switches, |tray, level| {
                    _ = tray.set_icon(icon::image(level, ink(dark()), false));
                });
            }
        }
        unsafe {
            TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
    }
}

/// Whether the taskbar is dark, as Windows draws it unless the user picks light for the system.
#[expect(unsafe_code, reason = "reads a value from the registry")]
fn dark() -> bool {
    let (mut light, mut size) = (0u32, u32::try_from(size_of::<u32>()).unwrap_or_default());
    // SAFETY: the value is a DWORD, read into a u32 of the size given.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PERSONALIZE,
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut light).cast(),
            &raw mut size,
        )
    };
    read != ERROR_SUCCESS || light == 0
}

/// Has the icon drawn again whenever the theme changes.
#[expect(unsafe_code, reason = "waits for changes to a registry key")]
fn watch_theme() {
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: the key is opened into a handle that outlives the waits.
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PERSONALIZE, 0, KEY_NOTIFY, &raw mut key) } != ERROR_SUCCESS {
        return;
    }
    // SAFETY: without an event, each call blocks until a value of the key changes.
    while unsafe { RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, std::ptr::null_mut(), 0) }
        == ERROR_SUCCESS
    {
        super::REDRAW.notify_one();
    }
}

impl Tray {
    #[expect(clippy::unused_async, reason = "as on Linux, where showing waits for D-Bus")]
    pub(super) async fn show(&mut self, login: &str, rows: &[Row], switches: &[(Switch, Option<bool>)]) {
        if self.shown.send((login.to_owned(), rows.to_vec(), switches.to_vec())).is_ok() {
            post(self.thread, WM_APP);
        }
    }
}
