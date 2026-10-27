//! The menu bar icon. `AppKit` runs it on the main thread, so the server runs on another one.

use anyhow::Result;
use dispatch2::DispatchQueue;
use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{NSObject, NSObjectProtocol, ProtocolObject},
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBox, NSBoxType, NSColor, NSEvent,
    NSEventModifierFlags, NSEventType, NSTitlePosition, NSWindowOrderingMode,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use parking_lot::Mutex;

use super::{DOT, Rgb, Row, SIDE, Switch, dot, icon, ink};

/// The page that logs in to the web UI, once the server serves it.
static LOGIN: Mutex<Option<String>> = Mutex::new(None);

/// Pixels per point: `build.rs` renders the icon for Retina displays.
const SCALE: f64 = 2.0;
/// The icon's side, and the dot's radius and side, in points, rather than as tall as the menu bar.
const ICON: f64 = SIDE as f64 / SCALE;
const RADIUS: f64 = DOT as f64 / SCALE;
const BADGE_SIDE: f64 = 2.0 * RADIUS;

thread_local! {
    /// The dot over the menu bar icon.
    static BADGE: std::cell::OnceCell<Retained<NSBox>> = const { std::cell::OnceCell::new() };
}

define_class!(
    /// Opens the web UI when the app is opened again from Finder or Spotlight, which macOS sends to the server
    /// that runs as the app rather than start another.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct Reopen;

    unsafe impl NSObjectProtocol for Reopen {}

    unsafe impl NSApplicationDelegate for Reopen {
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn reopened(&self, _app: &NSApplication, _visible: bool) -> bool {
            if let Some(login) = LOGIN.lock().clone() {
                _ = crate::server::open_on_desktop(login);
            }
            false
        }
    }
);

/// Hands the collections' health to the main thread, which shows it.
pub(in crate::server) struct Tray(());

/// Runs `serve` beside the menu bar icon when its user runs it in their desktop session; elsewhere, as
/// over SSH, runs it alone.
pub(in crate::server) fn beside(
    open: bool,
    serve: impl FnOnce(Option<Tray>) -> Result<()> + Send + 'static,
) -> Result<()> {
    let desktop = super::attended(open) && objc2_core_graphics::CGSessionCopyCurrentDictionary().is_some();
    let Some(mtm) = MainThreadMarker::new().filter(|_| desktop) else { return serve(None) };
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon nor menu of its own.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // Held while the app runs: it holds its delegate weakly.
    // SAFETY: `NSObject`'s `init`, on an instance just allocated.
    #[expect(unsafe_code, reason = "initializes the app's delegate")]
    let reopen: Retained<Reopen> = unsafe { msg_send![Reopen::alloc(mtm), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*reopen)));
    icon::open_clicked();
    let server = std::thread::spawn(move || {
        let served = serve(Some(Tray(())));
        DispatchQueue::main().exec_async(stop);
        served
    });
    app.run();
    server.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// Ends `run`, which returns once it handles an event.
fn stop() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    app.stop(None);
    let wake = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::ApplicationDefined,
        NSPoint::new(0.0, 0.0),
        NSEventModifierFlags::empty(),
        0.0,
        0,
        None,
        0,
        0,
        0,
    );
    if let Some(wake) = wake {
        app.postEvent_atStart(&wake, true);
    }
}

impl Tray {
    #[expect(clippy::unused_async, reason = "as on Linux, where showing waits for D-Bus")]
    pub(super) async fn show(&mut self, login: &str, rows: &[Row], switches: &[(Switch, Option<bool>)]) {
        let (login, rows, switches) = (login.to_owned(), rows.to_vec(), switches.to_vec());
        *LOGIN.lock() = Some(login.clone());
        DispatchQueue::main().exec_async(move || {
            icon::show(login, &rows, &switches, |tray, level| {
                // A template, which macOS draws in black or white to suit the menu bar, and the dot apart, in color.
                _ = tray.set_icon_templated(icon::image(level, ink(false), true));
                badge(tray, dot(level));
            });
        });
    }
}

/// Sizes the icon as the other icons of the menu bar, and shows the dot in `color` in its top right corner,
/// over the template that the system draws in black or white, or hides it.
fn badge(tray: &tray_icon::TrayIcon, color: Option<Rgb>) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(button) = tray.ns_status_item().and_then(|item| item.button(mtm)) else { return };
    let Some(image) = button.image() else { return };
    image.setSize(NSSize::new(ICON, ICON));
    button.setImage(Some(&image));
    BADGE.with(|badge| {
        let badge = badge.get_or_init(|| {
            let badge = NSBox::new(mtm);
            badge.setBoxType(NSBoxType::Custom);
            badge.setTitlePosition(NSTitlePosition::NoTitle);
            badge.setBorderWidth(0.0);
            // Below the view that `tray-icon` lays over the button for clicks.
            button.addSubview_positioned_relativeTo(&badge, NSWindowOrderingMode::Below, None);
            badge
        });
        let Some([red, green, blue]) = color else { return badge.setHidden(true) };
        let channel = |c: u8| f64::from(c) / f64::from(u8::MAX);
        badge.setFillColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
            channel(red),
            channel(green),
            channel(blue),
            1.0,
        ));
        // The image is centered in the button: its right and top edges.
        let bounds = button.bounds().size;
        let (right, top) = (f64::midpoint(bounds.width, ICON), f64::midpoint(bounds.height, ICON));
        let y = if button.isFlipped() { bounds.height - top } else { top - BADGE_SIDE };
        badge.setFrame(NSRect::new(NSPoint::new(right - BADGE_SIDE, y), NSSize::new(BADGE_SIDE, BADGE_SIDE)));
        badge.setCornerRadius(RADIUS);
        badge.setHidden(false);
    });
}
