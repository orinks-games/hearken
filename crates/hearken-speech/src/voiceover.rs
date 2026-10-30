//! VoiceOver on macOS, spoken as accessibility announcements.
//!
//! Prism's VoiceOver backend first tries `tell application "VoiceOver" to
//! output` through AppleScript, synchronously on the main thread. A
//! notarized game has no Apple Events entitlement, so macOS refuses it and
//! Prism falls back to an announcement, but while VoiceOver is starting
//! (its Welcome dialog is open) or has just quit, the AppleScript call
//! waits about two minutes first and the game shows as Not Responding.
//! This backend only posts the announcement: what Prism ends up doing
//! anyway, without the wait. Like Prism, lines spoken within 15 ms of each
//! other are joined, so a queued line does not cut off the one before it.

use std::ffi::{c_void, CString};
use std::sync::{Mutex, Once};

use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

use crate::backend::{SpeechError, VoiceBackend, VoiceFeatures};

/// The name Prism registers its VoiceOver backend under, which this
/// backend stands in for.
pub const REGISTRY_NAME: &str = "VoiceOver";

/// The backend's name: the same as Prism's, so selection and the startup
/// hold for VoiceOver treat it the same.
pub const NAME: &str = "VoiceOver (macOS)";

const COALESCE_NS: i64 = 15_000_000;
const NS_ACCESSIBILITY_PRIORITY_HIGH: isize = 90;
const DISPATCH_TIME_NOW: u64 = 0;

#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_time(when: u64, delta: i64) -> u64;
    fn dispatch_after_f(
        when: u64,
        queue: *const DispatchQueue,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn dispatch_async_f(
        queue: *const DispatchQueue,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSAccessibilityAnnouncementRequestedNotification: *mut AnyObject;
    static NSAccessibilityAnnouncementKey: *mut AnyObject;
    static NSAccessibilityPriorityKey: *mut AnyObject;
    static NSAccessibilityWindowCreatedNotification: *mut AnyObject;
    static NSAccessibilityFocusedWindowChangedNotification: *mut AnyObject;
    fn NSAccessibilityPostNotification(element: *mut AnyObject, notification: *mut AnyObject);
    fn NSAccessibilityPostNotificationWithUserInfo(
        element: *mut AnyObject,
        notification: *mut AnyObject,
        user_info: *mut AnyObject,
    );
}

/// Text waiting for the next announcement, and whether one is scheduled.
struct Pending {
    text: String,
    scheduled: bool,
}

static PENDING: Mutex<Pending> = Mutex::new(Pending {
    text: String::new(),
    scheduled: false,
});

static INTRODUCE_WINDOW: Once = Once::new();

/// True while VoiceOver is on.
pub fn voiceover_enabled() -> bool {
    autoreleasepool(|_| unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        !workspace.is_null() && msg_send![workspace, isVoiceOverEnabled]
    })
}

/// VoiceOver, reached by announcements only.
pub struct VoiceOverAnnouncer;

impl VoiceOverAnnouncer {
    /// Fails, as Prism's backend does, when VoiceOver is off.
    pub fn open() -> Result<VoiceOverAnnouncer, SpeechError> {
        if !voiceover_enabled() {
            return Err(prismer::Error::BackendNotAvailable);
        }
        INTRODUCE_WINDOW.call_once(|| unsafe {
            dispatch_async_f(&_dispatch_main_q, std::ptr::null_mut(), introduce_window);
        });
        Ok(VoiceOverAnnouncer)
    }
}

impl VoiceBackend for VoiceOverAnnouncer {
    fn name(&self) -> String {
        NAME.to_string()
    }

    fn features(&self) -> VoiceFeatures {
        VoiceFeatures {
            is_supported_at_runtime: voiceover_enabled(),
            supports_output: true,
            supports_speak: true,
            ..VoiceFeatures::default()
        }
    }

    fn output(&mut self, text: &str, interrupt: bool) -> Result<(), SpeechError> {
        self.speak(text, interrupt)
    }

    fn speak(&mut self, text: &str, interrupt: bool) -> Result<(), SpeechError> {
        if text.is_empty() {
            return Err(prismer::Error::InvalidParam);
        }
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        if interrupt || pending.text.is_empty() {
            pending.text = text.to_string();
        } else {
            pending.text.push_str(" . ");
            pending.text.push_str(text);
        }
        if !pending.scheduled {
            pending.scheduled = true;
            unsafe {
                dispatch_after_f(
                    dispatch_time(DISPATCH_TIME_NOW, COALESCE_NS),
                    &_dispatch_main_q,
                    std::ptr::null_mut(),
                    announce_pending,
                );
            }
        }
        Ok(())
    }

    fn braille(&mut self, _text: &str) -> Result<(), SpeechError> {
        Err(prismer::Error::NotImplemented)
    }

    fn stop(&mut self) -> Result<(), SpeechError> {
        Err(prismer::Error::NotImplemented)
    }

    fn set_rate(&mut self, _rate: f64) -> Result<(), SpeechError> {
        Err(prismer::Error::NotImplemented)
    }

    fn set_pitch(&mut self, _pitch: f64) -> Result<(), SpeechError> {
        Err(prismer::Error::NotImplemented)
    }

    fn set_volume(&mut self, _volume: f64) -> Result<(), SpeechError> {
        Err(prismer::Error::NotImplemented)
    }

    fn voices_count(&self) -> Result<usize, SpeechError> {
        Err(prismer::Error::NotImplemented)
    }

    fn voice_name(&self, _index: usize) -> Result<String, SpeechError> {
        Err(prismer::Error::NotImplemented)
    }

    fn set_voice(&mut self, _index: usize) -> Result<(), SpeechError> {
        Err(prismer::Error::NotImplemented)
    }
}

/// The window announcements are posted on: the key window, else the main
/// one, else the first. Main thread only.
unsafe fn game_window() -> *mut AnyObject {
    let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
    if app.is_null() {
        return app;
    }
    let key: *mut AnyObject = msg_send![app, keyWindow];
    if !key.is_null() {
        return key;
    }
    let main: *mut AnyObject = msg_send![app, mainWindow];
    if !main.is_null() {
        return main;
    }
    let windows: *mut AnyObject = msg_send![app, windows];
    if windows.is_null() {
        return windows;
    }
    msg_send![windows, firstObject]
}

/// Tells VoiceOver the game's window is there, as Prism does on start.
extern "C" fn introduce_window(_: *mut c_void) {
    autoreleasepool(|_| unsafe {
        let window = game_window();
        if !window.is_null() {
            NSAccessibilityPostNotification(window, NSAccessibilityWindowCreatedNotification);
            NSAccessibilityPostNotification(
                window,
                NSAccessibilityFocusedWindowChangedNotification,
            );
        }
    });
}

/// Posts the pending text as one announcement. Runs on the main queue.
extern "C" fn announce_pending(_: *mut c_void) {
    let text = {
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        pending.scheduled = false;
        std::mem::take(&mut pending.text)
    };
    if text.is_empty() || !voiceover_enabled() {
        return;
    }
    let Ok(text) = CString::new(text.replace('\0', " ")) else {
        return;
    };
    autoreleasepool(|_| unsafe {
        let window = game_window();
        if window.is_null() {
            return;
        }
        let string: *mut AnyObject =
            msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()];
        let priority: *mut AnyObject = msg_send![
            class!(NSNumber),
            numberWithInteger: NS_ACCESSIBILITY_PRIORITY_HIGH
        ];
        if string.is_null() || priority.is_null() {
            return;
        }
        let keys = [NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey];
        let values = [string, priority];
        let info: *mut AnyObject = msg_send![
            class!(NSDictionary),
            dictionaryWithObjects: values.as_ptr(),
            forKeys: keys.as_ptr(),
            count: keys.len()
        ];
        NSAccessibilityPostNotificationWithUserInfo(
            window,
            NSAccessibilityAnnouncementRequestedNotification,
            info,
        );
    });
}
