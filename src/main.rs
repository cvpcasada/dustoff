//! Dustoff: swallows every keyboard, trackpad and mouse event so you can wipe them down with
//! the laptop on. A menu-bar item says it's active; holding the trackpad down shows a
//! countdown, and five seconds of holding quits (which gives input back).

use std::cell::OnceCell;
use std::ffi::c_void;
use std::ptr::{NonNull, null_mut};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::{ClassType, sel};
use objc2_app_kit::{NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventMask, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGEventType,
    CGRequestPostEventAccess,
};
use objc2_foundation::{MainThreadMarker, NSData, NSSize, NSString, ns_string};
use rune::tok::type_;
use rune::{
    App, AppHandle, AppSpec, Cx, DisplayTarget, El, OverlayHandle, OverlayOptions, OverlaySpec, ScreenEdge, Tween, Tx, Waker,
    col, el, oklch, row, text,
};

const HOLD: Duration = Duration::from_secs(5);
/// How long the "locked" notice shows once input is blocked.
const NOTICE: Duration = Duration::from_secs(3);
const FADE: Duration = Duration::from_millis(150);
const BAR_WIDTH: f32 = 240.0;
const WAITING: &str = "Dustoff: allow it in Accessibility to lock input";
const LOCKED: &str = "Dustoff: locked, hold click 5s to exit";

/// The countdown overlay's state, which also owns the menu-bar item (both live on the main thread).
struct Hud {
    item: Option<Retained<NSStatusItem>>,
    /// Showing the "locked" notice instead of the countdown: the menu-bar icon can be hidden.
    notice: bool,
    /// When the current trackpad press began.
    held: Option<Instant>,
    /// Seconds left as last drawn, so `poll` redraws once a second rather than every frame.
    left: u64,
    bar: Tween<[f32; 4]>,
    /// Quits the app. An overlay's own `Tx::quit` is dropped outside focus events, so `poll` uses this.
    app: AppHandle<()>,
}

impl Hud {
    /// Dims the menu-bar icon until input is actually blocked; the tooltip says why.
    fn status(&self, locked: bool) {
        let (Some(item), Some(mtm)) = (&self.item, MainThreadMarker::new()) else { return };
        if let Some(button) = unsafe { item.button(mtm) } {
            unsafe {
                button.setAppearsDisabled(!locked);
                button.setToolTip(Some(&NSString::from_str(if locked { LOCKED } else { WAITING })));
            }
        }
    }
}

/// The menu-bar icon, as a template image so AppKit tints it for light and dark menu bars.
fn icon() -> Retained<NSImage> {
    let svg = NSData::with_bytes(include_bytes!("../assets/menubar.svg"));
    let image = NSImage::initWithData(NSImage::alloc(), &svg).expect("menu-bar SVG decodes");
    unsafe {
        image.setSize(NSSize::new(18.0, 18.0));
        image.setTemplate(true);
    }
    image
}

/// A progress fill that sweeps the bar over the hold; a new one restarts when next shown.
fn sweep() -> Tween<[f32; 4]> {
    Tween::linear([0.0, 0.0, 0.0, 4.0], [0.0, 0.0, BAR_WIDTH, 4.0], HOLD)
}

fn seconds_left(held: Duration) -> u64 {
    HOLD.saturating_sub(held).as_secs_f64().ceil() as u64
}

fn hud(h: &Hud, _: &mut Cx<Hud>) -> El {
    let pill = col().key("pill").padding(16.0).radius(16.0).fill(oklch(0.2, 0.0, 0.0)).border(1.0, oklch(0.3, 0.0, 0.0));
    let pill = if h.notice {
        pill.gap(4.0).center()
            .child(text("Keyboard and trackpad locked").font(type_::TITLE).color(oklch(0.93, 0.0, 0.0)))
            .child(text("Hold click for 5 seconds to exit").font(type_::LABEL).color(oklch(0.72, 0.0, 0.0)))
    } else {
        pill.gap(12.0)
            .child(text(format!("Keep holding to exit · {}", h.left)).font(type_::LABEL).color(oklch(0.93, 0.0, 0.0)))
            .child(
                el().size(BAR_WIDTH, 4.0).radius(2.0).fill(oklch(0.35, 0.0, 0.0))
                    .child(el().key("fill").size(0.0, 4.0).radius(2.0).fill(oklch(0.75, 0.15, 150.0)).live_rect(&h.bar)),
            )
    };
    row().w_full().h_full().center().child(pill)
}

/// Ticks the countdown each second while held, and quits once the hold reaches `HOLD`.
fn poll(h: &mut Hud, _: f64, tx: &mut Tx) -> bool {
    let Some(at) = h.held else { return false };
    let held = at.elapsed();
    if held >= HOLD {
        h.app.quit();
        return false;
    }
    tx.wake_after(Duration::from_nanos(1_000_000_000 - u64::from(held.subsec_nanos())));
    let left = seconds_left(held);
    std::mem::replace(&mut h.left, left) != left
}

/// A trackpad press: restart the countdown and the bar's sweep.
fn press(h: &mut Hud, tx: &mut Tx) {
    h.notice = false;
    h.held = Some(Instant::now());
    h.left = seconds_left(Duration::ZERO);
    h.bar = sweep();
    tx.wake_after(Duration::from_secs(1));
}

/// Runs on the main thread as the event loop starts: puts the indicator in the menu bar.
fn setup(h: &mut Hud, _: Waker) {
    let mtm = MainThreadMarker::new().expect("overlay setup runs on the main thread");
    unsafe {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        // Only reachable before Accessibility is granted: once input is blocked nothing can click it.
        let menu = NSMenu::new(mtm);
        menu.addItem(&NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(), ns_string!("Quit Dustoff"), Some(sel!(terminate:)), ns_string!("q"),
        ));
        item.setMenu(Some(&menu));
        if let Some(button) = item.button(mtm) { button.setImage(Some(&icon())) }
        h.item = Some(item);
    }
    h.status(false);
}

/// What the event tap's callback reaches through its `user_info` pointer.
struct Tap {
    overlay: OverlayHandle<Hud>,
    port: OnceCell<CFRetained<CFMachPort>>,
}

/// Swallows every input event; a left button press starts the exit countdown, its release
/// cancels it. Runs on the tap thread's run loop.
unsafe extern "C-unwind" fn on_event(_: CGEventTapProxy, ty: CGEventType, event: NonNull<CGEvent>, tap: *mut c_void) -> *mut CGEvent {
    let tap = unsafe { &*tap.cast::<Tap>() };
    match ty {
        // macOS turns off a tap it thinks stalled; turn it straight back on.
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
            if let Some(port) = tap.port.get() { CGEvent::tap_enable(port, true) }
            return event.as_ptr();
        }
        CGEventType::LeftMouseDown => {
            tap.overlay.update(press);
            tap.overlay.show();
        }
        CGEventType::LeftMouseUp => {
            tap.overlay.update(|h, _| h.held = None);
            tap.overlay.fade_out(FADE);
        }
        _ => {}
    }
    null_mut()
}

/// Installs a HID-level event tap that drops all input, asking for Accessibility access
/// (and waiting for it) first if needed. Never returns.
fn block_input(overlay: OverlayHandle<Hud>) {
    let tap: &'static Tap = Box::leak(Box::new(Tap { overlay, port: OnceCell::new() }));
    let mut asked = false;
    let port = loop {
        let port = unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::HIDEventTap, CGEventTapPlacement::HeadInsertEventTap, CGEventTapOptions::Default,
                CGEventMask::MAX, Some(on_event), std::ptr::from_ref(tap).cast_mut().cast(),
            )
        };
        if let Some(port) = port { break port }
        // The tap needs Accessibility; macOS shows its prompt once, then we poll until it's granted.
        if !std::mem::replace(&mut asked, true) { CGRequestPostEventAccess(); }
        std::thread::sleep(Duration::from_secs(1));
    };
    let source = CFMachPort::new_run_loop_source(None, Some(&port), 0).expect("event tap run loop source");
    CFRunLoop::current().expect("tap thread run loop").add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
    let _ = tap.port.set(port);
    announce_lock(&tap.overlay);
    CFRunLoop::run();
}

/// Input is blocked: light the menu-bar icon and show the "locked" notice for `NOTICE`.
fn announce_lock(overlay: &OverlayHandle<Hud>) {
    overlay.update(|h, _| {
        h.status(true);
        h.notice = true;
    });
    overlay.show();
    let overlay = overlay.clone();
    std::thread::spawn(move || {
        std::thread::sleep(NOTICE);
        // A press since then replaced the notice with the countdown, which stays up.
        let hide = overlay.clone();
        overlay.update(move |h, _| if std::mem::take(&mut h.notice) { hide.fade_out(FADE) });
    });
}

fn main() {
    let mut app = App::new(AppSpec::new("Dustoff", (1.0, 1.0), || (), |_, _| el())).windowless().accessory();
    let overlay = app.overlay(
        OverlaySpec { edge: ScreenEdge::Bottom, margin: 120.0, ..OverlaySpec::new("Dustoff", (300.0, 90.0), hud) }
            // Prewarmed, a reveal refreshes the panel first; otherwise macOS shows the frame it had
            // when last hidden, a stale bar, for ~150ms.
            .options(OverlayOptions { display: DisplayTarget::UnderPointer, prewarm: true, ..Default::default() })
            .setup(setup)
            .poll(poll),
        Hud { item: None, notice: false, held: None, left: 0, bar: sweep(), app: app.handle() },
    );
    std::thread::spawn(move || block_input(overlay));
    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_bar_icon_decodes() {
        assert!(unsafe { icon().isValid() });
    }

    #[test]
    fn countdown_rounds_up_and_ends_at_zero() {
        assert_eq!(seconds_left(Duration::ZERO), 5);
        assert_eq!(seconds_left(Duration::from_millis(4_001)), 1);
        assert_eq!(seconds_left(HOLD), 0);
        assert_eq!(seconds_left(Duration::from_secs(9)), 0);
    }
}
