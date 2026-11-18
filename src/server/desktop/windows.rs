//! The shell, which opens a page in the default browser and a folder in File Explorer.

/// Opens `target` as double-clicking it would.
#[expect(unsafe_code, reason = "ShellExecuteW reads the NUL-terminated strings it is given")]
pub(super) fn open(target: &str, _: bool) -> std::io::Result<()> {
    use crate::sys::wide;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    /// Above this, `ShellExecuteW` succeeded; at or below, it is an error code.
    const OPENED: isize = 32;
    let (verb, file) = (wide("open"), wide(target));
    // SAFETY: strings that live through the call, and no window that owns it.
    let done = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if done as isize > OPENED { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// File Explorer's window of the folder, with `file` selected.
#[expect(clippy::unused_async, reason = "as Linux's, which asks over D-Bus")]
#[expect(unsafe_code, reason = "the shell's item lists are C structures")]
pub(super) async fn reveal(file: &std::path::Path) -> bool {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems},
    };
    let wide: Vec<u16> = file.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: COM set up for this call on this thread, a path that lives through it, and an item list freed once.
    unsafe {
        let set_up = CoInitializeEx(std::ptr::null(), u32::try_from(COINIT_APARTMENTTHREADED).unwrap_or_default()) >= 0;
        let item = ILCreateFromPathW(wide.as_ptr());
        let shown = !item.is_null() && SHOpenFolderAndSelectItems(item, 0, std::ptr::null(), 0) >= 0;
        if !item.is_null() {
            ILFree(item);
        }
        if set_up {
            CoUninitialize();
        }
        shown
    }
}
