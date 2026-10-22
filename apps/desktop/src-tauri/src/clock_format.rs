//! The Mac's own hour cycle, for the "System" Time format setting.
//!
//! WKWebView's `Intl` reports the locale's default hour cycle but ignores the
//! user's System Settings → Date & Time → "24-hour time" override, so the
//! frontend asks here once per launch. Non-macOS returns `None` and the
//! frontend falls back to `Intl`.

/// `true` when an ICU skeleton-resolved hour pattern is 24-hour: it has no
/// 12-hour field (`h`/`K`) outside quoted literals.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn hour_pattern_is_24_hour(pattern: &str) -> bool {
    let mut quoted = false;
    for ch in pattern.chars() {
        match ch {
            '\'' => quoted = !quoted,
            'h' | 'K' if !quoted => return false,
            _ => {}
        }
    }
    true
}

#[tauri::command]
pub fn system_uses_24_hour_time() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        macos_hour_pattern().map(|pattern| hour_pattern_is_24_hour(&pattern))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// `[NSDateFormatter dateFormatFromTemplate:@"j" options:0 locale:NSLocale.currentLocale]`
/// — the "j" skeleton resolves to the preferred hour field for the current
/// locale *including* the user's 24-hour override.
#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn macos_hour_pattern() -> Option<String> {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::CStr;
    use std::os::raw::c_char;

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    cidre::ns::try_catch(|| unsafe {
        let locale: id = msg_send![class!(NSLocale), currentLocale];
        let template = NSString::alloc(nil).init_str("j");
        let pattern: id = msg_send![
            class!(NSDateFormatter),
            dateFormatFromTemplate: template
            options: 0usize
            locale: locale
        ];
        let _: () = msg_send![template, release];
        if pattern == nil {
            return None;
        }
        let raw: *const c_char = msg_send![pattern, UTF8String];
        (!raw.is_null()).then(|| CStr::from_ptr(raw).to_string_lossy().into_owned())
    })
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::hour_pattern_is_24_hour;

    #[test]
    fn classifies_hour_patterns() {
        assert!(!hour_pattern_is_24_hour("h a"));
        assert!(!hour_pattern_is_24_hour("a h時"));
        assert!(!hour_pattern_is_24_hour("K a"));
        assert!(hour_pattern_is_24_hour("HH"));
        assert!(hour_pattern_is_24_hour("H"));
        assert!(hour_pattern_is_24_hour("HH 'h'"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolves_on_this_mac() {
        assert!(super::system_uses_24_hour_time().is_some());
    }
}
