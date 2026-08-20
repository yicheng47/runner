//! macOS system-wake bridge (impl 0037).
//!
//! The app has no sleep/wake signal of its own. Tauri's `RunEvent::Resumed`
//! looked like one and was not: tauri-runtime-wry mapped it from
//! `Event::NewEvents(StartCause::Poll)`, while tao's macOS event loop only
//! produced `StartCause::Poll` under `ControlFlow::Poll`. tao's own
//! `Event::Resumed` is emitted from the iOS and Android backends only.
//!
//! `NSWorkspace.didWakeNotification` is the real thing: posted by the
//! window server when the machine comes back from sleep. The backend keeps
//! ownership of the resulting `app/woke` event through `EventChannel`; the
//! GPUI process installs this observer from its main-thread app-init callback.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2_app_kit::{NSWorkspace, NSWorkspaceDidWakeNotification};
use objc2_foundation::NSNotification;

use crate::events::EventChannel;

/// Frontend event name. Follows the `<domain>/<verb>` convention used by
/// the rest of the backend's emits.
pub const WOKE_EVENT: &str = "app/woke";

/// Broadcast `app/woke` each time the machine wakes.
///
/// Main's Tauri implementation retained an `AppHandle` and emitted directly
/// to webviews. The GPUI seam retains the core's `EventChannel` instead; the
/// observer and its process-lifetime ownership are otherwise unchanged.
pub fn install(events: &EventChannel) {
    let events = events.clone();
    observe_wake(move || {
        events.emit(WOKE_EVENT, &());
    });
}

/// Register `on_wake` against `NSWorkspaceDidWakeNotification` for the
/// remaining life of the process.
///
/// `Send + Sync` is the binding's price of admission, not decoration:
/// Foundation declares the block `NS_SWIFT_SENDABLE` and the objc2 method
/// is `unsafe` on exactly that condition. A `nil` queue means it runs
/// synchronously on whichever thread posts — the main thread for a real
/// wake, but nothing in the API promises that. `EventChannel` is safe to
/// emit from either case.
///
/// Nothing is kept from the call. The center strongly holds both the
/// copied block and the returned observer token until an explicit
/// `removeObserver:`, so dropping the local handles here deregisters
/// nothing — which is what a process-lifetime observer wants. The test
/// below is what keeps that claim honest: it registers, lets both handles
/// drop, and only then posts.
fn observe_wake<F: Fn() + Send + Sync + 'static>(on_wake: F) {
    let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        on_wake();
    });
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: the name is AppKit's own constant, the object filter is nil,
    // and `F: Send + Sync` makes the block sendable as the method requires.
    let _observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidWakeNotification),
            None,
            None,
            &block,
        )
    };
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    /// Stand in for the window server's post. The observer registration
    /// and the notification name are the parts worth pinning; a real
    /// sleep/wake cycle exercises the same delivery path.
    ///
    /// Every post here happens after `observe_wake` has returned and
    /// dropped its block and observer handles, so a passing test is also
    /// the evidence that the center's own strong references are what keep
    /// the registration alive.
    fn post_wake_notification() {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: nil object is valid for a notification with no sender.
        unsafe { center.postNotificationName_object(NSWorkspaceDidWakeNotification, None) };
    }

    #[test]
    fn wake_notification_reaches_the_observer() {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        observe_wake(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        post_wake_notification();
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        post_wake_notification();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }
}
