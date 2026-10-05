use block2::RcBlock;
use objc2_app_kit::{NSWorkspace, NSWorkspaceDidWakeNotification};
use objc2_foundation::NSNotification;
use std::ptr::NonNull;
pub fn observe_wake<F: Fn() + Send + Sync + 'static>(on_wake: F) {
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
