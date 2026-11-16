use chrono::{DateTime, Utc};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Notify;

pub(super) const DEBOUNCE: Duration = Duration::from_millis(300);
pub(super) const MAX_DEBOUNCE: Duration = Duration::from_millis(1500);
pub(super) const CLOCK_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const SUSPEND_GAP: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(super) struct Trigger {
    seq: AtomicU64,
    notify: Notify,
}

impl Trigger {
    pub(super) fn fire(&self) {
        self.seq.fetch_add(1, Ordering::Release);
        self.notify.notify_one();
    }
}

pub(super) fn wall_gap_jumped(
    wall_before: DateTime<Utc>,
    wall_after: DateTime<Utc>,
    mono_before: tokio::time::Instant,
    mono_after: tokio::time::Instant,
    watch_clock: bool,
) -> bool {
    if !watch_clock {
        return false;
    }
    let mono_elapsed = mono_after.saturating_duration_since(mono_before);
    let wall_elapsed = wall_after.signed_duration_since(wall_before);
    let gap = chrono::Duration::from_std(SUSPEND_GAP).expect("valid constant duration");
    let mono_wall = chrono::Duration::from_std(mono_elapsed).unwrap_or(chrono::Duration::MAX);
    mono_wall
        .checked_add(&gap)
        .is_none_or(|limit| wall_elapsed > limit)
        || wall_elapsed < -gap
}

pub(super) fn clock_jumped(
    wall_before: DateTime<Utc>,
    wall_after: DateTime<Utc>,
    mono_before: tokio::time::Instant,
    mono_after: tokio::time::Instant,
    watch_clock: bool,
) -> bool {
    wall_gap_jumped(
        wall_before,
        wall_after,
        mono_before,
        mono_after,
        watch_clock,
    ) || (watch_clock
        && mono_after.saturating_duration_since(mono_before) >= CLOCK_CHECK_INTERVAL + SUSPEND_GAP)
}

pub(super) struct Watcher {
    trigger: Arc<Trigger>,
    seen: u64,
    burst_start: Option<tokio::time::Instant>,
    settle_at: Option<tokio::time::Instant>,
    _platform: Option<platform::PlatformWatcher>,
}

const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<Watcher>();
};

impl Watcher {
    pub(super) fn new_with_trigger(trigger: Arc<Trigger>, native: bool) -> Self {
        let seen = trigger.seq.load(Ordering::Acquire);
        let platform = native.then(|| platform::PlatformWatcher::new(trigger.clone()));
        Self {
            trigger,
            seen,
            burst_start: None,
            settle_at: None,
            _platform: platform,
        }
    }

    pub(super) fn fire(&self) {
        self.trigger.fire();
    }

    pub(super) async fn next(&mut self) {
        loop {
            let notified = self.trigger.notify.notified();
            let current = self.trigger.seq.load(Ordering::Acquire);
            if current != self.seen {
                self.seen = current;
                let now = tokio::time::Instant::now();
                let start = *self.burst_start.get_or_insert(now);
                let cap = start + MAX_DEBOUNCE;
                self.settle_at = Some((now + DEBOUNCE).min(cap));
            }
            if let Some(settle) = self.settle_at {
                if tokio::time::Instant::now() >= settle {
                    self.burst_start = None;
                    self.settle_at = None;
                    return;
                }
                tokio::select! {
                    _ = tokio::time::sleep_until(settle) => {
                        let current = self.trigger.seq.load(Ordering::Acquire);
                        let now = tokio::time::Instant::now();
                        let cap = self.burst_start.unwrap_or(now) + MAX_DEBOUNCE;
                        if current != self.seen && now < cap {
                            self.seen = current;
                            self.settle_at = Some((now + DEBOUNCE).min(cap));
                            continue;
                        }
                        self.seen = current;
                        self.burst_start = None;
                        self.settle_at = None;
                        return;
                    }
                    _ = notified => continue,
                }
            } else {
                notified.await;
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Trigger;
    use std::{
        ffi::c_void,
        ptr::{null, null_mut},
        sync::{
            atomic::{AtomicBool, AtomicU32, Ordering},
            Arc,
        },
    };

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_IO_MESSAGE_CAN_SYSTEM_SLEEP: u32 = 0xe000_0270;
    const K_IO_MESSAGE_SYSTEM_WILL_SLEEP: u32 = 0xe000_0280;
    const K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON: u32 = 0xe000_0300;

    #[repr(C)]
    struct CFArrayCallBacks([usize; 5]);

    #[repr(C)]
    struct SCDynamicStoreContext {
        version: isize,
        info: *mut c_void,
        retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
        release: Option<unsafe extern "C" fn(*const c_void)>,
        copy_description: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    }

    type StoreCallback = Option<
        unsafe extern "C" fn(store: *mut c_void, changed_keys: *const c_void, info: *mut c_void),
    >;
    type PowerCallback = unsafe extern "C" fn(
        refcon: *mut c_void,
        service: u32,
        message_type: u32,
        message_argument: *mut c_void,
    );

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeArrayCallBacks: CFArrayCallBacks;
        fn CFStringCreateWithBytes(
            alloc: *const c_void,
            bytes: *const u8,
            num_bytes: isize,
            encoding: u32,
            is_external_representation: u8,
        ) -> *const c_void;
        fn CFArrayCreate(
            allocator: *const c_void,
            values: *const *const c_void,
            num_values: isize,
            callbacks: *const CFArrayCallBacks,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCDynamicStoreCreate(
            allocator: *const c_void,
            name: *const c_void,
            callout: StoreCallback,
            context: *mut SCDynamicStoreContext,
        ) -> *mut c_void;
        fn SCDynamicStoreSetNotificationKeys(
            store: *mut c_void,
            keys: *const c_void,
            patterns: *const c_void,
        ) -> u8;
        fn SCDynamicStoreSetDispatchQueue(store: *mut c_void, queue: *mut c_void) -> u8;
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IORegisterForSystemPower(
            refcon: *mut c_void,
            the_port_ref: *mut *mut c_void,
            callback: PowerCallback,
            notifier: *mut u32,
        ) -> u32;
        fn IONotificationPortSetDispatchQueue(notify: *mut c_void, queue: *mut c_void);
        fn IONotificationPortDestroy(notify: *mut c_void);
        fn IODeregisterForSystemPower(notifier: *mut u32) -> i32;
        fn IOAllowPowerChange(kernel_port: u32, notification_id: isize) -> i32;
        fn IOServiceClose(connect: u32) -> i32;
    }

    unsafe extern "C" {
        fn dispatch_queue_create(label: *const i8, attr: *const c_void) -> *mut c_void;
        fn dispatch_sync_f(
            queue: *mut c_void,
            context: *mut c_void,
            work: unsafe extern "C" fn(*mut c_void),
        );
        fn dispatch_release(object: *mut c_void);
    }

    struct CallbackContext {
        trigger: Arc<Trigger>,
        root_port: AtomicU32,
        active: AtomicBool,
    }

    struct PowerWatch {
        root_port: u32,
        notify_port: usize,
        notifier: u32,
        refcon: usize,
    }

    pub(super) struct PlatformWatcher {
        ctx: Arc<CallbackContext>,
        queue: usize,
        store: usize,
        power: Option<PowerWatch>,
    }

    impl PlatformWatcher {
        pub(super) fn new(trigger: Arc<Trigger>) -> Self {
            let ctx = Arc::new(CallbackContext {
                trigger,
                root_port: AtomicU32::new(0),
                active: AtomicBool::new(true),
            });
            let queue = unsafe {
                dispatch_queue_create(c"com.wycstudios.runner.remote-watch".as_ptr(), null())
            };
            if queue.is_null() {
                log::warn!("remote: macOS dispatch queue unavailable; using timer liveness");
                return Self {
                    ctx,
                    queue: 0,
                    store: 0,
                    power: None,
                };
            }

            let store = register_store(&ctx, queue);
            let power = register_power(&ctx, queue);

            Self {
                ctx,
                queue: queue as usize,
                store: store as usize,
                power,
            }
        }
    }

    impl Drop for PlatformWatcher {
        fn drop(&mut self) {
            self.ctx.active.store(false, Ordering::Release);
            let store = self.store as *mut c_void;
            if !store.is_null() {
                unsafe {
                    let _ = SCDynamicStoreSetDispatchQueue(store, null_mut());
                }
            }
            let power = self.power.take();
            if let Some(mut power) = power
                .as_ref()
                .map(|p| (p.root_port, p.notify_port, p.notifier))
            {
                unsafe {
                    if power.2 != 0 {
                        let _ = IODeregisterForSystemPower(&mut power.2);
                    }
                    if power.0 != 0 {
                        let _ = IOServiceClose(power.0);
                    }
                    if power.1 != 0 {
                        IONotificationPortDestroy(power.1 as *mut c_void);
                    }
                }
            }
            let queue = self.queue as *mut c_void;
            if !queue.is_null() {
                unsafe {
                    dispatch_sync_f(queue, null_mut(), noop_dispatch);
                }
            }
            if let Some(power) = power {
                unsafe {
                    drop(Arc::from_raw(power.refcon as *const CallbackContext));
                }
            }
            if !store.is_null() {
                unsafe {
                    CFRelease(store);
                }
            }
            if !queue.is_null() {
                unsafe {
                    dispatch_release(queue);
                }
            }
        }
    }

    struct CfRef(*const c_void);
    impl Drop for CfRef {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    fn cf_string(s: &str) -> Option<CfRef> {
        let ptr = unsafe {
            CFStringCreateWithBytes(
                null(),
                s.as_ptr(),
                s.len() as isize,
                K_CF_STRING_ENCODING_UTF8,
                0,
            )
        };
        (!ptr.is_null()).then_some(CfRef(ptr))
    }

    fn cf_array(strings: &[&str]) -> Option<CfRef> {
        let mut items = Vec::with_capacity(strings.len());
        for s in strings {
            items.push(cf_string(s)?);
        }
        let raw: Vec<*const c_void> = items.iter().map(|r| r.0).collect();
        let arr = unsafe {
            CFArrayCreate(
                null(),
                raw.as_ptr(),
                raw.len() as isize,
                &raw const kCFTypeArrayCallBacks,
            )
        };
        (!arr.is_null()).then_some(CfRef(arr))
    }

    fn register_store(ctx: &Arc<CallbackContext>, queue: *mut c_void) -> *mut c_void {
        let Some(name) = cf_string("com.wycstudios.runner.remote") else {
            log::warn!("remote: SCDynamicStore name allocation failed; using timer liveness");
            return null_mut();
        };
        let Some(keys) = cf_array(&["State:/Network/Global/IPv4", "State:/Network/Global/IPv6"])
        else {
            log::warn!("remote: SCDynamicStore key allocation failed; using timer liveness");
            return null_mut();
        };
        let Some(patterns) = cf_array(&[
            "State:/Network/Interface/[^/]+/Link",
            "State:/Network/Interface/[^/]+/IPv4",
            "State:/Network/Interface/[^/]+/IPv6",
        ]) else {
            log::warn!("remote: SCDynamicStore pattern allocation failed; using timer liveness");
            return null_mut();
        };
        let mut store_ctx = SCDynamicStoreContext {
            version: 0,
            info: Arc::as_ptr(ctx).cast_mut().cast(),
            retain: Some(retain_context),
            release: Some(release_context),
            copy_description: None,
        };
        let store =
            unsafe { SCDynamicStoreCreate(null(), name.0, Some(on_store_change), &mut store_ctx) };
        if store.is_null() {
            log::warn!("remote: SCDynamicStoreCreate unavailable; using timer liveness");
            return null_mut();
        }
        let configured =
            unsafe { SCDynamicStoreSetNotificationKeys(store, keys.0, patterns.0) != 0 };
        let scheduled = configured && unsafe { SCDynamicStoreSetDispatchQueue(store, queue) != 0 };
        if !scheduled {
            log::warn!("remote: SCDynamicStore notification setup failed; using timer liveness");
            unsafe { CFRelease(store) };
            return null_mut();
        }
        store
    }

    fn register_power(ctx: &Arc<CallbackContext>, queue: *mut c_void) -> Option<PowerWatch> {
        let refcon = Arc::into_raw(ctx.clone()) as *mut c_void;
        let mut notify_port: *mut c_void = null_mut();
        let mut notifier: u32 = 0;
        let root_port = unsafe {
            IORegisterForSystemPower(refcon, &mut notify_port, on_power_change, &mut notifier)
        };
        if root_port == 0 || notify_port.is_null() {
            log::warn!("remote: IORegisterForSystemPower unavailable; using timer liveness");
            unsafe {
                if notifier != 0 {
                    let _ = IODeregisterForSystemPower(&mut notifier);
                }
                if root_port != 0 {
                    let _ = IOServiceClose(root_port);
                }
                if !notify_port.is_null() {
                    IONotificationPortDestroy(notify_port);
                }
                drop(Arc::from_raw(refcon.cast::<CallbackContext>()));
            }
            return None;
        }
        ctx.root_port.store(root_port, Ordering::Release);
        unsafe {
            IONotificationPortSetDispatchQueue(notify_port, queue);
        }
        Some(PowerWatch {
            root_port,
            notify_port: notify_port as usize,
            notifier,
            refcon: refcon as usize,
        })
    }

    unsafe extern "C" fn retain_context(info: *const c_void) -> *const c_void {
        if !info.is_null() {
            unsafe { Arc::increment_strong_count(info.cast::<CallbackContext>()) };
        }
        info
    }

    unsafe extern "C" fn release_context(info: *const c_void) {
        if !info.is_null() {
            unsafe { Arc::decrement_strong_count(info.cast::<CallbackContext>()) };
        }
    }

    unsafe extern "C" fn on_store_change(
        _store: *mut c_void,
        _changed_keys: *const c_void,
        info: *mut c_void,
    ) {
        if info.is_null() {
            return;
        }
        let ctx = unsafe { &*info.cast::<CallbackContext>() };
        if ctx.active.load(Ordering::Acquire) {
            ctx.trigger.fire();
        }
    }

    unsafe extern "C" fn on_power_change(
        refcon: *mut c_void,
        _service: u32,
        message_type: u32,
        message_argument: *mut c_void,
    ) {
        if refcon.is_null() {
            return;
        }
        let ctx = unsafe { &*refcon.cast::<CallbackContext>() };
        match message_type {
            K_IO_MESSAGE_CAN_SYSTEM_SLEEP | K_IO_MESSAGE_SYSTEM_WILL_SLEEP => {
                let port = ctx.root_port.load(Ordering::Acquire);
                if port != 0 {
                    unsafe {
                        let _ = IOAllowPowerChange(port, message_argument as isize);
                    }
                }
            }
            K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON if ctx.active.load(Ordering::Acquire) => {
                ctx.trigger.fire();
            }
            _ => {}
        }
    }

    unsafe extern "C" fn noop_dispatch(_: *mut c_void) {}
}

#[cfg(windows)]
mod platform {
    use super::Trigger;
    use std::{
        ffi::c_void,
        ptr::null_mut,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_SUCCESS, HANDLE, NO_ERROR},
        NetworkManagement::IpHelper::{
            CancelMibChangeNotify2, MibInitialNotification, NotifyIpInterfaceChange,
            NotifyRouteChange2, NotifyUnicastIpAddressChange, MIB_IPFORWARD_ROW2,
            MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW,
        },
        Networking::WinSock::AF_UNSPEC,
        System::Power::{
            PowerRegisterSuspendResumeNotification, PowerUnregisterSuspendResumeNotification,
            DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY,
        },
        UI::WindowsAndMessaging::{
            DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
        },
    };

    struct CallbackContext {
        trigger: Arc<Trigger>,
        active: AtomicBool,
    }

    pub(super) struct PlatformWatcher {
        ctx: usize,
        iface_handle: usize,
        addr_handle: usize,
        route_handle: usize,
        power_handle: usize,
        power_params: usize,
    }

    impl PlatformWatcher {
        pub(super) fn new(trigger: Arc<Trigger>) -> Self {
            let ctx = Arc::into_raw(Arc::new(CallbackContext {
                trigger,
                active: AtomicBool::new(true),
            }));
            let caller_ctx = ctx.cast::<c_void>();

            let mut iface_handle: HANDLE = null_mut();
            let err = unsafe {
                NotifyIpInterfaceChange(
                    AF_UNSPEC,
                    Some(on_interface_change),
                    caller_ctx,
                    false,
                    &mut iface_handle,
                )
            };
            if err != NO_ERROR {
                log::warn!(
                    "remote: NotifyIpInterfaceChange unavailable ({err}); using timer liveness"
                );
                iface_handle = null_mut();
            }

            let mut addr_handle: HANDLE = null_mut();
            let err = unsafe {
                NotifyUnicastIpAddressChange(
                    AF_UNSPEC,
                    Some(on_address_change),
                    caller_ctx,
                    false,
                    &mut addr_handle,
                )
            };
            if err != NO_ERROR {
                log::warn!(
                    "remote: NotifyUnicastIpAddressChange unavailable ({err}); using timer liveness"
                );
                addr_handle = null_mut();
            }

            let mut route_handle: HANDLE = null_mut();
            let err = unsafe {
                NotifyRouteChange2(
                    AF_UNSPEC,
                    Some(on_route_change),
                    caller_ctx,
                    false,
                    &mut route_handle,
                )
            };
            if err != NO_ERROR {
                log::warn!("remote: NotifyRouteChange2 unavailable ({err}); using timer liveness");
                route_handle = null_mut();
            }

            let power_params = Box::into_raw(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
                Callback: Some(on_power_event),
                Context: caller_ctx.cast_mut(),
            }));
            let mut power_handle: *mut c_void = null_mut();
            let err = unsafe {
                PowerRegisterSuspendResumeNotification(
                    DEVICE_NOTIFY_CALLBACK,
                    power_params.cast::<c_void>(),
                    &mut power_handle,
                )
            };
            let kept_params = if err == ERROR_SUCCESS && !power_handle.is_null() {
                power_params as usize
            } else {
                log::warn!(
                    "remote: PowerRegisterSuspendResumeNotification unavailable ({err}); using timer liveness"
                );
                unsafe {
                    drop(Box::from_raw(power_params));
                }
                power_handle = null_mut();
                0
            };

            Self {
                ctx: ctx as usize,
                iface_handle: iface_handle as usize,
                addr_handle: addr_handle as usize,
                route_handle: route_handle as usize,
                power_handle: power_handle as usize,
                power_params: kept_params,
            }
        }
    }

    impl Drop for PlatformWatcher {
        fn drop(&mut self) {
            let ctx_ptr = self.ctx as *const CallbackContext;
            if !ctx_ptr.is_null() {
                unsafe { (*ctx_ptr).active.store(false, Ordering::Release) };
            }
            for handle in [self.iface_handle, self.addr_handle, self.route_handle] {
                if handle != 0 {
                    unsafe {
                        let _ = CancelMibChangeNotify2(handle as HANDLE);
                    }
                }
            }
            if self.power_handle != 0 {
                unsafe {
                    let _ =
                        PowerUnregisterSuspendResumeNotification(self.power_handle as HPOWERNOTIFY);
                }
            }
            if self.power_params != 0 {
                unsafe {
                    drop(Box::from_raw(
                        self.power_params as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
                    ));
                }
            }
            if !ctx_ptr.is_null() {
                unsafe {
                    drop(Arc::from_raw(ctx_ptr));
                }
            }
        }
    }

    fn notify_network(caller_context: *const c_void, notification_type: MIB_NOTIFICATION_TYPE) {
        if caller_context.is_null() || notification_type == MibInitialNotification {
            return;
        }
        let ctx = unsafe { &*caller_context.cast::<CallbackContext>() };
        if ctx.active.load(Ordering::Acquire) {
            ctx.trigger.fire();
        }
    }

    unsafe extern "system" fn on_interface_change(
        caller_context: *const c_void,
        _row: *const MIB_IPINTERFACE_ROW,
        notification_type: MIB_NOTIFICATION_TYPE,
    ) {
        notify_network(caller_context, notification_type);
    }

    unsafe extern "system" fn on_address_change(
        caller_context: *const c_void,
        _row: *const MIB_UNICASTIPADDRESS_ROW,
        notification_type: MIB_NOTIFICATION_TYPE,
    ) {
        notify_network(caller_context, notification_type);
    }

    unsafe extern "system" fn on_route_change(
        caller_context: *const c_void,
        _row: *const MIB_IPFORWARD_ROW2,
        notification_type: MIB_NOTIFICATION_TYPE,
    ) {
        notify_network(caller_context, notification_type);
    }

    unsafe extern "system" fn on_power_event(
        context: *const c_void,
        kind: u32,
        _setting: *const c_void,
    ) -> u32 {
        if !context.is_null() && matches!(kind, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND) {
            let ctx = unsafe { &*context.cast::<CallbackContext>() };
            if ctx.active.load(Ordering::Acquire) {
                ctx.trigger.fire();
            }
        }
        ERROR_SUCCESS
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::Trigger;
    use std::{
        os::fd::{AsRawFd, FromRawFd, OwnedFd},
        sync::Arc,
    };

    pub(super) struct PlatformWatcher {
        task: Option<tokio::task::JoinHandle<()>>,
    }

    impl PlatformWatcher {
        pub(super) fn new(trigger: Arc<Trigger>) -> Self {
            let Ok(runtime) = tokio::runtime::Handle::try_current() else {
                return Self { task: None };
            };
            let raw = unsafe {
                libc::socket(
                    libc::AF_NETLINK,
                    libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                    libc::NETLINK_ROUTE,
                )
            };
            if raw < 0 {
                log::warn!(
                    "remote: netlink socket unavailable ({}); using timer liveness",
                    std::io::Error::last_os_error()
                );
                return Self { task: None };
            }
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
            addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
            addr.nl_groups = (libc::RTMGRP_LINK
                | libc::RTMGRP_IPV4_IFADDR
                | libc::RTMGRP_IPV6_IFADDR
                | libc::RTMGRP_IPV4_ROUTE
                | libc::RTMGRP_IPV6_ROUTE) as u32;
            let bound = unsafe {
                libc::bind(
                    fd.as_raw_fd(),
                    (&raw const addr).cast::<libc::sockaddr>(),
                    std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
                )
            };
            if bound < 0 {
                log::warn!(
                    "remote: netlink bind unavailable ({}); using timer liveness",
                    std::io::Error::last_os_error()
                );
                return Self { task: None };
            }
            let async_fd = match tokio::io::unix::AsyncFd::new(fd) {
                Ok(async_fd) => async_fd,
                Err(err) => {
                    log::warn!(
                        "remote: netlink AsyncFd registration failed ({err}); using timer liveness"
                    );
                    return Self { task: None };
                }
            };
            let task = runtime.spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    let Ok(mut guard) = async_fd.readable().await else {
                        break;
                    };
                    let mut changed = false;
                    loop {
                        let n = unsafe {
                            libc::recv(
                                guard.get_inner().as_raw_fd(),
                                buf.as_mut_ptr().cast(),
                                buf.len(),
                                libc::MSG_DONTWAIT,
                            )
                        };
                        if n > 0 {
                            changed = true;
                        } else if n == 0 {
                            return;
                        } else {
                            let err = std::io::Error::last_os_error();
                            if err.kind() == std::io::ErrorKind::Interrupted {
                                continue;
                            }
                            if err.raw_os_error() == Some(libc::ENOBUFS) {
                                changed = true;
                                continue;
                            }
                            if err.kind() == std::io::ErrorKind::WouldBlock {
                                guard.clear_ready();
                                break;
                            }
                            return;
                        }
                    }
                    if changed {
                        trigger.fire();
                    }
                }
            });
            Self { task: Some(task) }
        }
    }

    impl Drop for PlatformWatcher {
        fn drop(&mut self) {
            if let Some(task) = self.task.take() {
                task.abort();
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod platform {
    use super::Trigger;
    use std::sync::Arc;

    pub(super) struct PlatformWatcher;

    impl PlatformWatcher {
        pub(super) fn new(_: Arc<Trigger>) -> Self {
            Self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_jump_detects_suspend_gaps_backwards_steps_and_overshoot() {
        let wall = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap();
        rt.block_on(async {
            let mono = tokio::time::Instant::now();
            assert!(!clock_jumped(
                wall,
                wall + chrono::Duration::seconds(2),
                mono,
                mono + Duration::from_secs(2),
                true,
            ));
            assert!(!clock_jumped(
                wall,
                wall + chrono::Duration::seconds(60),
                mono,
                mono + Duration::from_secs(2),
                false,
            ));
            assert!(clock_jumped(
                wall,
                wall + chrono::Duration::seconds(6),
                mono,
                mono + Duration::from_secs(2),
                true,
            ));
            assert!(clock_jumped(
                wall,
                wall - chrono::Duration::seconds(4),
                mono,
                mono + Duration::from_secs(1),
                true,
            ));
            assert!(clock_jumped(
                wall,
                wall + chrono::Duration::seconds(6),
                mono,
                mono + Duration::from_secs(6),
                true,
            ));
        });
    }

    #[tokio::test(start_paused = true)]
    async fn debouncer_coalesces_rapid_burst_survives_cancellation_and_caps_wait() {
        let trigger = Arc::new(Trigger::default());
        let mut watcher = Watcher::new_with_trigger(trigger.clone(), false);

        trigger.fire();
        tokio::select! {
            _ = watcher.next() => panic!("must wait for debounce window"),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
        trigger.fire();
        tokio::select! {
            _ = watcher.next() => panic!("cancelled select must preserve debounce and slide window"),
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
        tokio::time::timeout(Duration::from_millis(60), watcher.next())
            .await
            .expect("coalesced burst must fire once settle window completes");
        tokio::select! {
            _ = watcher.next() => panic!("coalesced burst must not fire a second time"),
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }

        let started = tokio::time::Instant::now();
        let noisy = trigger.clone();
        let spam = tokio::spawn(async move {
            for _ in 0..10 {
                noisy.fire();
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
        watcher.next().await;
        assert_eq!(tokio::time::Instant::now() - started, MAX_DEBOUNCE);
        spam.abort();
    }

    #[tokio::test]
    async fn platform_watcher_creates_and_drops_cleanly_repeatedly() {
        for _ in 0..32 {
            let trigger = Arc::new(Trigger::default());
            let watcher = Watcher::new_with_trigger(trigger, true);
            drop(watcher);
        }
    }
}
