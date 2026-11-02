//! Launch Services, through `NSWorkspace`.

/// Opens a `page` in the default browser, or a folder in Finder.
pub(super) fn open(target: &str, page: bool) -> std::io::Result<()> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    let text = NSString::from_str(target);
    let url = if page { NSURL::URLWithString(&text) } else { Some(NSURL::fileURLWithPath(&text)) };
    let opened = url.is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url));
    if opened { Ok(()) } else { Err(std::io::Error::other(target.to_owned())) }
}

/// Finder's window of the folder, with `file` selected.
#[expect(clippy::unused_async, reason = "as Linux's, which asks over D-Bus")]
pub(super) async fn reveal(file: &std::path::Path) -> bool {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSArray, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.display().to_string()));
    NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
    true
}
