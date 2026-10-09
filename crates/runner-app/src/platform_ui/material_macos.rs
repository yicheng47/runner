use std::ptr::NonNull;

use block2::RcBlock;
use futures::{channel::mpsc, StreamExt};
use gpui::{App, Global};
use objc2::{msg_send, rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};

use crate::appearance::Appearance;

pub(crate) fn reduce_transparency() -> bool {
    unsafe {
        msg_send![
            &*NSWorkspace::sharedWorkspace(),
            accessibilityDisplayShouldReduceTransparency
        ]
    }
}

type NotificationObserver = (
    Retained<NSNotificationCenter>,
    Retained<ProtocolObject<dyn NSObjectProtocol>>,
);
struct Notifications(Vec<NotificationObserver>);

impl Global for Notifications {}

impl Drop for Notifications {
    fn drop(&mut self) {
        for (center, observer) in &self.0 {
            unsafe {
                center.removeObserver(AsRef::<objc2::runtime::AnyObject>::as_ref(&**observer))
            };
        }
    }
}

pub(crate) fn install(cx: &mut App) {
    cx.global_mut::<Appearance>().reduce_transparency = reduce_transparency();
    let (sender, mut receiver) = mpsc::unbounded();
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
        let _ = sender.unbounded_send(());
    });
    let name = objc2_foundation::NSString::from_str(
        "NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification",
    );
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(&name), None, None, &block)
    };
    cx.set_global(Notifications(vec![(center, observer)]));
    cx.spawn(async move |cx| {
        while receiver.next().await.is_some() {
            cx.update(|cx| {
                cx.global_mut::<Appearance>().reduce_transparency = reduce_transparency();
                let material = cx.global::<Appearance>().material;
                crate::appearance::configure(material, cx);
                cx.refresh_windows();
            });
        }
    })
    .detach();
}
