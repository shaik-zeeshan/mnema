//! The macOS preferred language, `navigator.language`'s source in the Tauri
//! WebView ("en-US", "de-DE"): Semantic Search recommends its Multilingual
//! tier when it isn't English.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;

/// `NSLocale.preferredLanguages.firstObject`, or "" when there is none.
pub fn preferred_language() -> String {
    // SAFETY: class methods on NSLocale / NSArray with no arguments; a nil
    // answer comes back as `None`.
    unsafe {
        let langs: Option<Retained<AnyObject>> = msg_send![class!(NSLocale), preferredLanguages];
        let Some(langs) = langs else {
            return String::new();
        };
        let first: Option<Retained<NSString>> = msg_send![&*langs, firstObject];
        first.map(|s| s.to_string()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_a_language_tag() {
        let tag = super::preferred_language();
        assert!(
            tag.is_empty() || tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "{tag}"
        );
    }
}
