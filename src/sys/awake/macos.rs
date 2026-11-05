//! A power assertion of `IOKit`, as `caffeinate` takes.

/// Held until dropped: see the module.
pub(crate) struct Awake(u32);

#[expect(unsafe_code, reason = "IOKit's power assertions are C functions")]
pub(crate) fn stay_awake() -> anyhow::Result<Awake> {
    use std::ffi::{CString, c_char, c_void};
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(kind: *const c_void, level: u32, name: *const c_void, id: *mut u32) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(allocator: *const c_void, text: *const c_char, encoding: u32) -> *const c_void;
        fn CFRelease(object: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;
    const ON: u32 = 255;
    let string = |text: &str| {
        let text = CString::new(text).unwrap_or_default();
        // SAFETY: a C string that lives through the call, and the default allocator.
        unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) }
    };
    let (kind, name) = (string("PreventUserIdleSystemSleep"), string(&crate::i18n::t!("awake-why")));
    let mut id = 0;
    // SAFETY: two strings that the call copies, released after, and where it writes the assertion's id.
    let returned = unsafe {
        let returned = if kind.is_null() || name.is_null() {
            -1
        } else {
            IOPMAssertionCreateWithName(kind, ON, name, &raw mut id)
        };
        for s in [kind, name].into_iter().filter(|s| !s.is_null()) {
            CFRelease(s);
        }
        returned
    };
    match returned {
        0 => Ok(Awake(id)),
        code => Err(std::io::Error::other(format!("{code:#x}")).into()),
    }
}

#[expect(unsafe_code, reason = "IOKit's power assertions are C functions")]
impl Drop for Awake {
    fn drop(&mut self) {
        #[link(name = "IOKit", kind = "framework")]
        unsafe extern "C" {
            fn IOPMAssertionRelease(id: u32) -> i32;
        }
        // SAFETY: an assertion this took and releases once.
        unsafe { IOPMAssertionRelease(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::{super::unix::output, stay_awake};

    /// The idle sleep assertions that this process holds, as `pmset` lists them.
    fn assertions() -> Vec<String> {
        let all = output(&["/usr/bin/pmset", "-g", "assertions"]);
        let own = format!("pid {}(", std::process::id());
        all.lines()
            .filter(|l| l.contains(&own) && l.contains("PreventUserIdleSystemSleep"))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn iokit_holds_an_idle_sleep_assertion_until_dropped() {
        let awake = stay_awake().unwrap();
        let held = assertions();
        assert!(held.iter().any(|l| l.contains(&crate::i18n::t!("awake-why"))), "{held:?}");
        drop(awake);
        assert!(assertions().is_empty(), "the assertion outlived the run");
    }
}
