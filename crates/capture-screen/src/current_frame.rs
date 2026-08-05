//! One-shot "what is on screen right now" screenshot (round-4 decision G1).
//!
//! Deliberately NOT the recording pipeline's newest frame: that frame is stale,
//! and it is absent exactly when the feature is most wanted (while not
//! recording). This takes a live ScreenCaptureKit screenshot at invoke time
//! through a content filter that excludes
//!
//!   (a) Mnema's own windows — so the Quick Access bar (and, later, the reading
//!       outline) never appear in their own shot; self-exclusion in the FILTER
//!       is the mechanism, not capture-timing tricks, and
//!   (b) the privacy-listed apps — the same list the recording content filter
//!       and the system-audio tap exclude (the standing parity rule).
//!
//! An excluded app that is actually on screen is **blanked and named**, never
//! silently dropped and never a refusal: the caller surfaces the names in the
//! context chip ("1Password excluded").
//!
//! No session, no writer, no frame index, no DB row — one JPEG at a caller-chosen
//! path.

use capture_types::CaptureErrorResponse;
use std::collections::BTreeSet;
use std::path::Path;

/// A running app as ScreenCaptureKit reports it, reduced to the fields the plan
/// needs. Exists so [`plan_current_frame_filter`] is a pure function that can be
/// unit-tested without a display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentFrameApp {
    pub pid: i32,
    pub bundle_id: String,
    pub app_name: String,
}

/// A window as ScreenCaptureKit reports it, front-to-back in the order SCK
/// returns them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentFrameWindow {
    pub owner_pid: i32,
    pub title: Option<String>,
    pub layer: i64,
    pub on_screen: bool,
}

/// What the filter must contain, plus the metadata the chip and the non-vision
/// text fallback need.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CurrentFrameFilterPlan {
    /// Every app blanked out of the shot: Mnema itself plus the privacy list.
    pub excluded_pids: Vec<i32>,
    /// The privacy-listed apps that actually had a window on screen, named for
    /// the context chip. Mnema's own exclusion is never named — it is plumbing,
    /// not a privacy event the user needs told about.
    pub excluded_app_names: Vec<String>,
    /// The app owning the frontmost non-excluded window.
    pub app_name: Option<String>,
    /// That window's title.
    pub window_title: Option<String>,
}

/// Decide what the one-shot content filter excludes, and what the shot is "of".
///
/// Pure so the exclude-list → filter-contents rule is testable headlessly; the
/// ScreenCaptureKit call around it is not.
pub fn plan_current_frame_filter(
    excluded_bundle_ids: &[String],
    own_pid: i32,
    own_bundle_id: &str,
    apps: &[CurrentFrameApp],
    windows: &[CurrentFrameWindow],
) -> CurrentFrameFilterPlan {
    let privacy_listed: BTreeSet<&str> = excluded_bundle_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .collect();
    let own_bundle_id = own_bundle_id.trim();
    let on_screen_pids: BTreeSet<i32> = windows
        .iter()
        .filter(|window| window.on_screen)
        .map(|window| window.owner_pid)
        .collect();

    let mut excluded_pids = Vec::new();
    // Deduped: one app can run several processes, and the chip names apps.
    let mut excluded_app_names = BTreeSet::new();
    for app in apps {
        let bundle_id = app.bundle_id.trim();
        let is_self =
            app.pid == own_pid || (!own_bundle_id.is_empty() && bundle_id == own_bundle_id);
        let is_privacy_listed = privacy_listed.contains(bundle_id);
        if !is_self && !is_privacy_listed {
            continue;
        }
        excluded_pids.push(app.pid);
        // Named only when it is a privacy exclusion the user would otherwise see
        // vanish, and only when it really was on screen — naming an app that was
        // not visible would be noise, not disclosure.
        if is_privacy_listed && !is_self && on_screen_pids.contains(&app.pid) {
            let name = app.app_name.trim();
            if !name.is_empty() {
                excluded_app_names.insert(name.to_string());
            }
        }
    }

    let excluded: BTreeSet<i32> = excluded_pids.iter().copied().collect();
    // Frontmost = the first on-screen normal-layer window SCK lists that we did
    // not blank. Skipping the excluded ones matters: Mnema's own panel is key
    // when this runs, so without the skip every shot would report "Mnema".
    //
    // ponytail: trusts SCK's documented front-to-back window order; if that ever
    // stops holding, sort by layer then by window id.
    let front = windows.iter().find(|window| {
        window.on_screen && window.layer == 0 && !excluded.contains(&window.owner_pid)
    });
    let app_name = front.and_then(|window| {
        apps.iter()
            .find(|app| app.pid == window.owner_pid)
            .map(|app| app.app_name.trim().to_string())
            .filter(|name| !name.is_empty())
    });
    let window_title = front
        .and_then(|window| window.title.clone())
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());

    CurrentFrameFilterPlan {
        excluded_pids,
        excluded_app_names: excluded_app_names.into_iter().collect(),
        app_name,
        window_title,
    }
}

/// Long edge of the written JPEG. Enough for OCR and for a vision model to read
/// UI text, small enough that the image is not the dominant cost of the turn.
const CURRENT_FRAME_MAX_EDGE: f64 = 1600.0;

/// Scale a display's point size down so the long edge fits
/// [`CURRENT_FRAME_MAX_EDGE`]. Never scales up.
fn current_frame_capture_size(display_width: f64, display_height: f64) -> (usize, usize) {
    let longest = display_width.max(display_height);
    if longest <= 0.0 {
        return (0, 0);
    }
    let scale = (CURRENT_FRAME_MAX_EDGE / longest).min(1.0);
    (
        (display_width * scale).round().max(1.0) as usize,
        (display_height * scale).round().max(1.0) as usize,
    )
}

/// Take the live screenshot and write it to `output_path` as JPEG.
///
/// Returns the plan that was applied, so the caller can name the blanked apps
/// and stamp the frame with the app/window it is actually of.
#[cfg(target_os = "macos")]
pub fn capture_current_frame_jpeg(
    excluded_bundle_ids: &[String],
    own_pid: i32,
    own_bundle_id: &str,
    output_path: &Path,
) -> Result<CurrentFrameFilterPlan, CaptureErrorResponse> {
    use cidre::{blocks, cg, ns, sc};
    use std::sync::mpsc;
    use std::time::Duration;

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();

    let (content_tx, content_rx) = mpsc::channel();
    sc::ShareableContent::current_with_ch(move |content, error| {
        let result = match (content, error) {
            (Some(content), None) => Ok(content.retained()),
            (_, Some(error)) => Err(current_frame_error(format!(
                "Failed to query ScreenCaptureKit shareable content: {error}"
            ))),
            _ => Err(current_frame_error(
                "No ScreenCaptureKit shareable content available".to_string(),
            )),
        };
        let _ = content_tx.send(result);
    });
    let content = content_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| {
            current_frame_error("Timed out querying ScreenCaptureKit shareable content".to_string())
        })??;

    let displays = content.displays();
    let display = displays
        .first()
        .ok_or_else(|| current_frame_error("No ScreenCaptureKit display available".to_string()))?;

    let sc_apps = content.apps();
    let apps: Vec<CurrentFrameApp> = sc_apps
        .iter()
        .map(|app| CurrentFrameApp {
            pid: app.process_id(),
            bundle_id: app.bundle_id().to_string(),
            app_name: app.app_name().to_string(),
        })
        .collect();
    let sc_windows = content.windows();
    let windows: Vec<CurrentFrameWindow> = sc_windows
        .iter()
        .map(|window| CurrentFrameWindow {
            owner_pid: window
                .owning_app()
                .map(|app| app.process_id())
                .unwrap_or_default(),
            title: window.title().map(|title| title.to_string()),
            layer: window.window_layer() as i64,
            on_screen: window.is_on_screen(),
        })
        .collect();

    let plan =
        plan_current_frame_filter(excluded_bundle_ids, own_pid, own_bundle_id, &apps, &windows);

    let excluded: BTreeSet<i32> = plan.excluded_pids.iter().copied().collect();
    let filter = if excluded.is_empty() {
        sc::ContentFilter::with_display_excluding_windows(display, &ns::Array::new())
    } else {
        let excluded_apps: Vec<_> = sc_apps
            .iter()
            .filter(|app| excluded.contains(&app.process_id()))
            .map(|app| app.retained())
            .collect();
        sc::ContentFilter::with_display_excluding_apps_excepting_windows(
            display,
            &ns::Array::from_slice_retained(&excluded_apps),
            &ns::Array::new(),
        )
    };

    let (width, height) =
        current_frame_capture_size(display.width() as f64, display.height() as f64);
    let mut cfg = sc::StreamCfg::new();
    cfg.set_width(width);
    cfg.set_height(height);
    cfg.set_shows_cursor(false);
    cfg.set_captures_audio(false);
    cfg.set_capture_mic(false);

    let (image_tx, image_rx) = mpsc::channel();
    let mut handler = blocks::ResultCh::<cg::Image>::new2(
        move |image: Option<&cg::Image>, error: Option<&ns::Error>| {
            let result = match (image, error) {
                (Some(image), None) => Ok(image.retained()),
                (_, Some(error)) => Err(current_frame_error(format!(
                    "ScreenCaptureKit screenshot failed: {error}"
                ))),
                _ => Err(current_frame_error(
                    "ScreenCaptureKit returned no screenshot".to_string(),
                )),
            };
            let _ = image_tx.send(result);
        },
    );
    sc::ScreenshotManager::capture_image_ch(&filter, &cfg, Some(&mut handler));
    let image = image_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| current_frame_error("Timed out taking the screenshot".to_string()))??;

    crate::save_cg_image_as_jpeg(&image, output_path)?;

    Ok(plan)
}

#[cfg(not(target_os = "macos"))]
pub fn capture_current_frame_jpeg(
    _excluded_bundle_ids: &[String],
    _own_pid: i32,
    _own_bundle_id: &str,
    _output_path: &Path,
) -> Result<CurrentFrameFilterPlan, CaptureErrorResponse> {
    Err(current_frame_error(
        "Current-frame capture requires macOS ScreenCaptureKit".to_string(),
    ))
}

fn current_frame_error(message: String) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "current_frame_capture_failed".to_string(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(pid: i32, bundle_id: &str, app_name: &str) -> CurrentFrameApp {
        CurrentFrameApp {
            pid,
            bundle_id: bundle_id.to_string(),
            app_name: app_name.to_string(),
        }
    }

    fn window(owner_pid: i32, title: &str) -> CurrentFrameWindow {
        CurrentFrameWindow {
            owner_pid,
            title: Some(title.to_string()),
            layer: 0,
            on_screen: true,
        }
    }

    #[test]
    fn excludes_mnema_itself_without_naming_it() {
        let plan = plan_current_frame_filter(
            &[],
            501,
            "day.mnema",
            &[
                app(501, "day.mnema", "Mnema"),
                app(700, "com.apple.Safari", "Safari"),
            ],
            &[window(501, "Quick Access"), window(700, "Docs")],
        );

        assert_eq!(plan.excluded_pids, vec![501]);
        assert!(plan.excluded_app_names.is_empty());
        // Mnema's panel is key when this runs; the shot is still "of" Safari.
        assert_eq!(plan.app_name.as_deref(), Some("Safari"));
        assert_eq!(plan.window_title.as_deref(), Some("Docs"));
    }

    #[test]
    fn excludes_the_reading_outline_overlay_along_with_mnemas_other_windows() {
        // The "Mnema is reading this screen" outline (round-4 G3) is a full-screen
        // window of this very process. The filter excludes whole APPS by pid and
        // bundle id, so the overlay is blanked without opting in anywhere — the
        // invariant that lets capture stay implicit while indication is explicit.
        let plan = plan_current_frame_filter(
            &[],
            501,
            "day.mnema",
            &[
                app(501, "day.mnema", "Mnema"),
                app(700, "com.apple.Safari", "Safari"),
            ],
            &[
                // Frontmost while the bar is collapsed: the outline, then the bar.
                window(501, "mnema · reading this screen"),
                window(501, "Quick Access"),
                window(700, "Docs"),
            ],
        );

        assert_eq!(plan.excluded_pids, vec![501]);
        assert!(plan.excluded_app_names.is_empty());
        // The shot is of what is BEHIND the indication, never of the indication.
        assert_eq!(plan.app_name.as_deref(), Some("Safari"));
        assert_eq!(plan.window_title.as_deref(), Some("Docs"));
    }

    #[test]
    fn excludes_mnema_by_bundle_id_when_the_pid_differs() {
        let plan = plan_current_frame_filter(
            &[],
            501,
            "day.mnema",
            &[app(999, "day.mnema", "Mnema")],
            &[window(999, "Quick Access")],
        );

        assert_eq!(plan.excluded_pids, vec![999]);
        assert!(plan.excluded_app_names.is_empty());
    }

    #[test]
    fn blanks_and_names_a_privacy_listed_app_on_screen() {
        let excluded = vec!["com.1password.1password".to_string()];
        let plan = plan_current_frame_filter(
            &excluded,
            501,
            "day.mnema",
            &[
                app(501, "day.mnema", "Mnema"),
                app(800, "com.1password.1password", "1Password"),
                app(700, "com.apple.Safari", "Safari"),
            ],
            // 1Password frontmost — the case the decision calls out by name.
            &[window(800, "Vault"), window(700, "Docs")],
        );

        assert!(plan.excluded_pids.contains(&800));
        assert_eq!(plan.excluded_app_names, vec!["1Password".to_string()]);
        // Never a refusal: the shot still happens, of whatever was behind it.
        assert_eq!(plan.app_name.as_deref(), Some("Safari"));
    }

    #[test]
    fn does_not_name_a_privacy_listed_app_that_is_not_on_screen() {
        let excluded = vec!["com.1password.1password".to_string()];
        let plan = plan_current_frame_filter(
            &excluded,
            501,
            "day.mnema",
            &[
                app(800, "com.1password.1password", "1Password"),
                app(700, "com.apple.Safari", "Safari"),
            ],
            &[window(700, "Docs")],
        );

        assert!(plan.excluded_pids.contains(&800));
        assert!(plan.excluded_app_names.is_empty());
    }

    #[test]
    fn names_a_multi_process_app_once() {
        let excluded = vec!["com.brave.Browser".to_string()];
        let plan = plan_current_frame_filter(
            &excluded,
            501,
            "day.mnema",
            &[
                app(810, "com.brave.Browser", "Brave Browser"),
                app(811, "com.brave.Browser", "Brave Browser"),
            ],
            &[window(810, "Tab"), window(811, "Helper")],
        );

        assert_eq!(plan.excluded_pids, vec![810, 811]);
        assert_eq!(plan.excluded_app_names, vec!["Brave Browser".to_string()]);
    }

    #[test]
    fn ignores_blank_and_unknown_bundle_ids() {
        let excluded = vec!["  ".to_string(), "com.nowhere.app".to_string()];
        let plan = plan_current_frame_filter(
            &excluded,
            501,
            "day.mnema",
            &[app(700, "com.apple.Safari", "Safari")],
            &[window(700, "Docs")],
        );

        assert!(plan.excluded_pids.is_empty());
        assert!(plan.excluded_app_names.is_empty());
    }

    #[test]
    fn skips_non_normal_layer_and_offscreen_windows_when_picking_the_frame() {
        let plan = plan_current_frame_filter(
            &[],
            501,
            "day.mnema",
            &[app(700, "com.apple.Safari", "Safari")],
            &[
                CurrentFrameWindow {
                    owner_pid: 700,
                    title: Some("Menu bar".to_string()),
                    layer: 25,
                    on_screen: true,
                },
                CurrentFrameWindow {
                    owner_pid: 700,
                    title: Some("Minimized".to_string()),
                    layer: 0,
                    on_screen: false,
                },
                window(700, "Docs"),
            ],
        );

        assert_eq!(plan.window_title.as_deref(), Some("Docs"));
    }

    #[test]
    fn capture_size_caps_the_long_edge_and_never_upscales() {
        assert_eq!(current_frame_capture_size(3024.0, 1964.0), (1600, 1039));
        assert_eq!(current_frame_capture_size(1280.0, 800.0), (1280, 800));
        assert_eq!(current_frame_capture_size(0.0, 0.0), (0, 0));
    }
}
