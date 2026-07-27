//! macOS system-wake bridge (impl 0037).
//!
//! The app has no sleep/wake signal of its own. `RunEvent::Resumed` looks
//! like one and is not: tauri-runtime-wry maps it from
//! `Event::NewEvents(StartCause::Poll)`, and tao's macOS event loop only
//! produces `StartCause::Poll` under `ControlFlow::Poll` — which
//! tauri-runtime-wry overwrites with `ControlFlow::Wait` on every
//! iteration. So `app/resumed` never fires on macOS at all, let alone on
//! wake. tao's own `Event::Resumed` is emitted from the iOS and Android
//! backends only.
//!
//! `NSWorkspace.didWakeNotification` is the real thing: posted by the
//! window server when the machine comes back from sleep. The frontend
//! consumes the resulting `app/woke` event to invalidate WebGL texture
//! atlases, whose glyphs were rasterized against a display/GPU state that
//! no longer holds (#360).

use std::ptr::NonNull;

use block2::RcBlock;
use objc2_app_kit::{NSWorkspace, NSWorkspaceDidWakeNotification};
use objc2_foundation::NSNotification;
use tauri::{AppHandle, Emitter, Runtime};

/// Frontend event name. Follows the `<domain>/<verb>` convention used by
/// the rest of the backend's emits.
pub const WOKE_EVENT: &str = "app/woke";

/// Broadcast `app/woke` to every webview each time the machine wakes.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    observe_wake(move || {
        if let Err(e) = app.emit(WOKE_EVENT, ()) {
            log::error!("emit {WOKE_EVENT} failed: {e}");
        }
    });
}

/// Register `on_wake` against `NSWorkspaceDidWakeNotification` for the
/// remaining life of the process.
///
/// `Send + Sync` is the binding's price of admission, not decoration:
/// Foundation declares the block `NS_SWIFT_SENDABLE` and the objc2 method
/// is `unsafe` on exactly that condition. A `nil` queue means it runs
/// synchronously on whichever thread posts — the main thread for a real
/// wake, but nothing in the API promises that.
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
