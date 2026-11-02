//! The shell, which opens a page in the default browser and a folder in File Explorer.

/// Opens `target` as double-clicking it would.
#[expect(unsafe_code, reason = "ShellExecuteW reads the NUL-terminated strings it is given")]
pub(super) fn open(target: &str, _: bool) -> std::io::Result<()> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    /// Above this, `ShellExecuteW` succeeded; at or below, it is an error code.
    const OPENED: isize = 32;
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
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
