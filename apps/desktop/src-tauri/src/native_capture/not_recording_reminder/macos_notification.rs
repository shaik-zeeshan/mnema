//! The native macOS half of the not-recording reminder, on
//! `UNUserNotificationCenter` through the `objc`/`block` crates the app already
//! links (cidre's `un` binding lacks posting/authorization).
//!
//! Authorization is requested lazily on the first post; a denial is logged once
//! and the in-app row carries on alone. `UNUserNotificationCenter` raises an
//! Objective-C exception in a process that isn't a real `.app` bundle (a plain
//! `tauri dev` binary), so every entry point is gated on that first.
#![allow(deprecated, unexpected_cfgs)]

use block::{Block, ConcreteBlock};
use cocoa::base::{id, nil};
use cocoa::foundation::{NSArray, NSString};
use objc::runtime::{Object, Sel, BOOL, NO};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

const NOTIFICATION_ID: &str = "day.mnema.not-recording-reminder";
const CATEGORY_ID: &str = "day.mnema.not-recording";
const START_ACTION_ID: &str = "start";
const SNOOZE_ACTION_ID: &str = "snooze-today";
/// `UNNotificationDefaultActionIdentifier`: the banner body was clicked.
const DEFAULT_ACTION_ID: &str = "com.apple.UNNotificationDefaultActionIdentifier";

static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
static DENIAL_LOGGED: AtomicBool = AtomicBool::new(false);
/// Set once we've touched the center, so `remove` never wakes it up for nothing.
static CENTER_READY: AtomicBool = AtomicBool::new(false);

pub(super) fn post(app_handle: &tauri::AppHandle) {
    if !running_from_app_bundle() {
        super::super::debug_log::log("native reminder skipped: not running from an .app bundle");
        return;
    }
    let _ = APP_HANDLE.set(app_handle.clone());
    let _pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    unsafe {
        let center = center_with_delegate();
        // Prompts only the first time; afterwards it answers with the stored choice.
        let on_answer = ConcreteBlock::new(|granted: BOOL, _error: id| {
            if granted == NO {
                if !DENIAL_LOGGED.swap(true, Ordering::Relaxed) {
                    super::super::debug_log::log_warn(
                        "notifications not authorized; the not-recording reminder stays in-app only",
                    );
                }
                return;
            }
            add_request();
        })
        .copy();
        const ALERT_AND_SOUND: usize = (1 << 2) | (1 << 1);
        let _: () = msg_send![
            center,
            requestAuthorizationWithOptions: ALERT_AND_SOUND
            completionHandler: &*on_answer
        ];
    }
}

/// Withdraws the banner (delivered or pending) once recording starts or the
/// reminder is snoozed.
pub(super) fn remove() {
    if !CENTER_READY.load(Ordering::Relaxed) {
        return;
    }
    let _pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    unsafe {
        let center: id = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let ids = NSArray::arrayWithObjects(nil, &[ns_string(NOTIFICATION_ID)]);
        let _: () = msg_send![center, removeDeliveredNotificationsWithIdentifiers: ids];
        let _: () = msg_send![center, removePendingNotificationRequestsWithIdentifiers: ids];
    }
}

fn running_from_app_bundle() -> bool {
    unsafe {
        let bundle: id = msg_send![class!(NSBundle), mainBundle];
        let path: id = msg_send![bundle, bundlePath];
        let identifier: id = msg_send![bundle, bundleIdentifier];
        identifier != nil && path != nil && rust_string(path).ends_with(".app")
    }
}

unsafe fn add_request() {
    let _pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    let content: id = msg_send![class!(UNMutableNotificationContent), new];
    let _: () = msg_send![content, setTitle: ns_string("Mnema isn't recording")];
    let _: () =
        msg_send![content, setBody: ns_string("Recording has been off for over 30 minutes.")];
    let _: () = msg_send![content, setCategoryIdentifier: ns_string(CATEGORY_ID)];
    let request: id = msg_send![
        class!(UNNotificationRequest),
        requestWithIdentifier: ns_string(NOTIFICATION_ID)
        content: content
        trigger: nil
    ];
    let _: () = msg_send![content, release];
    let center: id = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
    let on_added = ConcreteBlock::new(|error: id| {
        if error != nil {
            super::super::debug_log::log_warn("failed to post the not-recording notification");
        }
    })
    .copy();
    let _: () =
        msg_send![center, addNotificationRequest: request withCompletionHandler: &*on_added];
}

/// The center, with our delegate and Start / Snooze actions registered once.
unsafe fn center_with_delegate() -> id {
    let center: id = msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
    if !CENTER_READY.swap(true, Ordering::Relaxed) {
        // The center holds its delegate weakly; this one lives for the process.
        let delegate: id = msg_send![delegate_class(), new];
        let _: () = msg_send![center, setDelegate: delegate];

        let start: id = msg_send![
            class!(UNNotificationAction),
            actionWithIdentifier: ns_string(START_ACTION_ID)
            title: ns_string("Start")
            options: 0usize
        ];
        let snooze: id = msg_send![
            class!(UNNotificationAction),
            actionWithIdentifier: ns_string(SNOOZE_ACTION_ID)
            title: ns_string("Snooze for today")
            options: 0usize
        ];
        let actions = NSArray::arrayWithObjects(nil, &[start, snooze]);
        let no_intents = NSArray::arrayWithObjects(nil, &[]);
        let category: id = msg_send![
            class!(UNNotificationCategory),
            categoryWithIdentifier: ns_string(CATEGORY_ID)
            actions: actions
            intentIdentifiers: no_intents
            options: 0usize
        ];
        let categories: id = msg_send![class!(NSSet), setWithObject: category];
        let _: () = msg_send![center, setNotificationCategories: categories];
    }
    center
}

fn delegate_class() -> &'static objc::runtime::Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    let ptr = *CLASS.get_or_init(|| {
        let mut decl =
            objc::declare::ClassDecl::new("MnemaReminderNotificationDelegate", class!(NSObject))
                .expect("failed to declare MnemaReminderNotificationDelegate");
        unsafe {
            decl.add_method(
                sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:),
                did_receive_response as extern "C" fn(&Object, Sel, id, id, *mut c_void),
            );
            decl.add_method(
                sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:),
                will_present as extern "C" fn(&Object, Sel, id, id, *mut c_void),
            );
        }
        decl.register() as *const objc::runtime::Class as usize
    });
    unsafe { &*(ptr as *const objc::runtime::Class) }
}

extern "C" fn did_receive_response(
    _this: &Object,
    _cmd: Sel,
    _center: id,
    response: id,
    completion: *mut c_void,
) {
    let action = unsafe {
        let action: id = msg_send![response, actionIdentifier];
        rust_string(action)
    };
    if let Some(app_handle) = APP_HANDLE.get().cloned() {
        // Off the delegate queue: a start opens capture streams.
        std::thread::spawn(move || match action.as_str() {
            START_ACTION_ID => super::start_from_reminder(&app_handle),
            SNOOZE_ACTION_ID => super::snooze_until(&app_handle, next_local_midnight_ms()),
            DEFAULT_ACTION_ID => super::show_reminder_in_app(&app_handle),
            _ => {}
        });
    }
    unsafe { (*(completion as *const Block<(), ()>)).call(()) };
}

/// Show the banner even while Mnema is frontmost (the default hides it).
extern "C" fn will_present(
    _this: &Object,
    _cmd: Sel,
    _center: id,
    _notification: id,
    completion: *mut c_void,
) {
    const BANNER_AND_LIST: usize = (1 << 4) | (1 << 3);
    unsafe { (*(completion as *const Block<(usize,), ()>)).call((BANNER_AND_LIST,)) };
}

/// Unix ms of the next local midnight, from the user's calendar and time zone.
fn next_local_midnight_ms() -> u64 {
    const NS_CALENDAR_UNIT_DAY: usize = 1 << 4;
    let _pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    unsafe {
        let calendar: id = msg_send![class!(NSCalendar), currentCalendar];
        let now: id = msg_send![class!(NSDate), date];
        let today: id = msg_send![calendar, startOfDayForDate: now];
        let tomorrow: id = msg_send![
            calendar,
            dateByAddingUnit: NS_CALENDAR_UNIT_DAY
            value: 1isize
            toDate: today
            options: 0usize
        ];
        let seconds: f64 = msg_send![tomorrow, timeIntervalSince1970];
        (seconds * 1000.0) as u64
    }
}

unsafe fn ns_string(value: &str) -> id {
    let string = NSString::alloc(nil).init_str(value);
    msg_send![string, autorelease]
}

unsafe fn rust_string(value: id) -> String {
    if value == nil {
        return String::new();
    }
    let utf8: *const std::os::raw::c_char = msg_send![value, UTF8String];
    if utf8.is_null() {
        return String::new();
    }
    CStr::from_ptr(utf8).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn next_local_midnight_is_within_a_day_ahead() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let midnight = super::next_local_midnight_ms();
        assert!(midnight > now_ms && midnight <= now_ms + 25 * 60 * 60 * 1000);
    }
}
