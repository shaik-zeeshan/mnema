//! Small-window goldens (`<surface>-<state>-<theme>.png`) at each window's real
//! size: the CLI Access request, Update, the permission helper, Developer and
//! its mini mode, each over `apps/desktop/parity/fixtures/<surface>/<state>.json`
//! (what the Svelte stub serves), and the parity shot for
//! `scripts/parity.sh <access|update|permission-helper|debug|debug-mini> <state>`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::{transparent_black, AnyWindowHandle, AppContext, VisualTestAppContext};
use mnema_gpui::shell::fixture;
use mnema_gpui::small::access::{AccessBackend, AccessRequest, FixtureAccess};
use mnema_gpui::small::debug::data::FixtureDebug;
use mnema_gpui::small::debug::model::Tab;
use mnema_gpui::small::debug::{DebugWindow, Mode};
use mnema_gpui::small::helper::{GrantCheck, PermKind, PermissionHelper};
use mnema_gpui::small::update::UpdateWindow;
use mnema_gpui::small::{access, debug, helper, update};
use mnema_gpui::theme;

use super::compare;

/// `(surface, state, fixture file)` for every golden; the fixture defaults like `shoot.ts`.
pub const STATES: [(&str, &str, &str); 23] = [
    ("access", "request", "request"),
    ("access", "widen", "widen"),
    ("access", "no-request", "no-request"),
    ("access", "granted", "granted"),
    ("update", "available", "available"),
    ("update", "checking", "checking"),
    ("update", "uptodate", "uptodate"),
    ("update", "check-failed", "check-failed"),
    ("update", "downloading", "downloading"),
    ("update", "install-failed", "install-failed"),
    ("update", "ready", "ready"),
    ("permission-helper", "waiting", "waiting"),
    ("permission-helper", "granted", "granted"),
    ("debug", "overview-issues", "default"),
    ("debug", "overview-ok", "overview-ok"),
    ("debug", "not-recording", "not-recording"),
    ("debug", "pipeline", "default"),
    // The processor drill-in (Pipeline › Transcription) and a failed job's inspector.
    ("debug", "jobs", "default"),
    ("debug", "job", "default"),
    ("debug", "capture", "default"),
    ("debug", "ai", "default"),
    ("debug", "logs", "default"),
    ("debug-mini", "mini", "default"),
];

pub const SURFACES: [&str; 5] = [
    "access",
    "update",
    "permission-helper",
    "debug",
    "debug-mini",
];

fn viewport(surface: &str) -> (f32, f32) {
    match surface {
        "access" => access::SIZE,
        "update" => update::SIZE,
        "permission-helper" => helper::SIZE,
        "debug-mini" => debug::MINI,
        _ => debug::SIZE,
    }
}

struct Fixed(bool);

impl GrantCheck for Fixed {
    fn granted(&self, _: PermKind) -> Result<bool, String> {
        Ok(self.0)
    }
}

fn settle(cx: &mut VisualTestAppContext, window: AnyWindowHandle) -> anyhow::Result<()> {
    for _ in 0..5 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, _| window.refresh())?;
        // The app icon decodes on the background executor.
        std::thread::sleep(Duration::from_millis(20));
    }
    cx.run_until_parked();
    Ok(())
}

pub fn render(
    cx: &mut VisualTestAppContext,
    surface: &str,
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    let (_, _, file) = STATES
        .iter()
        .find(|(s, st, _)| *s == surface && *st == state)
        .ok_or_else(|| anyhow::anyhow!("no {surface} state {state}"))?;
    let map = fixture::load(surface, file)?;
    cx.update(|cx| theme::set_setting(setting, cx));
    let vp = viewport(surface);
    let mut access_view = None;
    let mut debug_view = None;
    let window = crate::pointer::open(cx, compare::viewport(vp), |window, cx| {
        let root =
            |view: gpui_kit::AnyView, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
                cx.new(|cx| Root::new(view, window, cx).bg(transparent_black()))
            };
        match surface {
            "access" => {
                let request = map["get_pending_cli_access_request"].clone();
                let b: Arc<dyn AccessBackend> =
                    Arc::new(FixtureAccess::new((!request.is_null()).then_some(request)));
                let v = cx.new(|cx| AccessRequest::new(b, window, cx));
                access_view = Some(v.clone());
                root(v.into(), window, cx)
            }
            "update" => {
                let status = map["get_app_update_status"].clone();
                let v = cx.new(|_| UpdateWindow::new(Some(status), None));
                root(v.into(), window, cx)
            }
            "permission-helper" => {
                let granted = map["get_capture_permissions"]["permissions"]["screen"] == "granted";
                let v = cx.new(|cx| {
                    PermissionHelper::new(PermKind::Screen, Arc::new(Fixed(granted)), cx)
                });
                root(v.into(), window, cx)
            }
            _ => {
                let v = cx.new(|cx| {
                    let mut w =
                        DebugWindow::new(Arc::new(FixtureDebug::new(map.clone())), window, cx);
                    w.tab = match state {
                        "pipeline" | "jobs" | "job" => Tab::Pipeline,
                        "capture" => Tab::Capture,
                        "ai" => Tab::Ai,
                        "logs" => Tab::Logs,
                        _ => Tab::Overview,
                    };
                    if surface == "debug-mini" {
                        w.mode = Mode::Mini;
                    }
                    if matches!(state, "jobs" | "job") {
                        w.open_processor(1, cx);
                    }
                    w
                });
                debug_view = Some(v.clone());
                root(v.into(), window, cx)
            }
        }
    })?;
    let window: AnyWindowHandle = window.into();
    settle(cx, window)?;
    if let (Some(v), "granted") = (&access_view, state) {
        cx.update_window(window, |_, window, cx| {
            v.update(cx, |v, cx| v.approve(window, cx))
        })?;
        settle(cx, window)?;
    }
    if let (Some(v), "job") = (&debug_view, state) {
        cx.update_window(window, |_, _, cx| {
            v.update(cx, |w, cx| {
                // The failed job at its attempt ceiling, with a multi-line error.
                let job = w.detail.jobs.iter().find(|j| j["id"] == 48213871).cloned();
                w.open_job(job.expect("the fixture's failed job"), cx);
            })
        })?;
        settle(cx, window)?;
    }
    let shot = cx.capture_screenshot(window)?;
    cx.update_window(window, |_, window, _| window.remove_window())?;
    Ok(shot)
}

pub fn goldens(
    cx: &mut VisualTestAppContext,
    filters: &[String],
    themes: &[(&str, AppearanceSetting)],
) -> anyhow::Result<Vec<String>> {
    let mut failures = Vec::new();
    for (surface, state, _) in &STATES {
        for (theme_name, setting) in themes {
            let name = format!("{surface}-{state}-{theme_name}");
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = render(cx, surface, state, setting.clone())?;
            if let Err(e) = compare::check_golden(&name, &shot) {
                failures.push(e.to_string());
            }
        }
    }
    Ok(failures)
}

/// Writes `<dir>/gpui.png` for one parity state, at the Tauri shot's 1x size.
pub fn parity(
    cx: &mut VisualTestAppContext,
    dir: &Path,
    surface: &str,
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<()> {
    let shot = render(cx, surface, state, setting)?;
    let vp = viewport(surface);
    let shot = image::imageops::resize(
        &shot,
        vp.0 as u32,
        vp.1 as u32,
        image::imageops::FilterType::Triangle,
    );
    shot.save(dir.join("gpui.png"))?;
    println!("  parity: wrote {}", dir.join("gpui.png").display());
    Ok(())
}
