//! Onboarding goldens (`onboarding-<state>-<theme>.png`) at the window's real
//! 1120×800, every step × its key states, over
//! `apps/desktop/parity/fixtures/onboarding/<file>.json`; and the parity shot
//! for `scripts/parity.sh onboarding <state>`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::{
    px, size, transparent_black, AnyWindowHandle, AppContext, Entity, VisualTestAppContext,
};
use mnema_gpui::onboarding::data::Backend;
use mnema_gpui::onboarding::fixture::{FixtureBackend, STATES};
use mnema_gpui::onboarding::flow::Step;
use mnema_gpui::onboarding::{self, Onboarding};
use mnema_gpui::theme;
use serde_json::json;

use super::compare;

/// Parity states (surfaces.ts) that map onto a golden state.
fn golden_for(parity: &str) -> &str {
    match parity {
        "settings" | "change" | "setup" | "voice" | "finale" | "welcome" | "permissions"
        | "capture" => parity,
        "permissions-done" => "permissions-granted",
        "permissions-mic-denied" => "permissions-denied",
        "capture-tiny" => "capture-blocked",
        other => other,
    }
}

fn settle(cx: &mut VisualTestAppContext, window: AnyWindowHandle) -> anyhow::Result<()> {
    for _ in 0..8 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, _| window.refresh())?;
        // Frame previews decode on the background executor.
        std::thread::sleep(Duration::from_millis(20));
    }
    cx.run_until_parked();
    Ok(())
}

fn progress(
    o: &Entity<Onboarding>,
    cx: &mut VisualTestAppContext,
    events: &[(&str, &str, &str, u64)],
) {
    cx.update(|cx| {
        o.update(cx, |o, cx| {
            for (provider, model, status, got) in events {
                let e = json!({ "provider": provider, "modelId": model, "status": status, "downloadedBytes": got });
                o.on_download_event(&e, cx);
            }
        })
    });
}

pub fn render(
    cx: &mut VisualTestAppContext,
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    let (_, file) = STATES
        .iter()
        .find(|(s, _)| *s == state)
        .ok_or_else(|| anyhow::anyhow!("no onboarding state {state}"))?;
    cx.update(|cx| theme::set_setting(setting, cx));
    let backend: Arc<dyn Backend> = Arc::new(FixtureBackend::load(file)?);
    let mut view = None;
    let window = cx.open_offscreen_window(
        size(px(onboarding::SIZE.0), px(onboarding::SIZE.1)),
        |window, cx| {
            let v = cx.new(|cx| Onboarding::new(backend, |_, _| {}, cx));
            view = Some(v.clone());
            cx.new(|cx| Root::new(v, window, cx).bg(transparent_black()))
        },
    )?;
    let window: AnyWindowHandle = window.into();
    let o = view.expect("view");
    settle(cx, window)?;
    let step = match state {
        "welcome" => Step::Welcome,
        s if s.starts_with("permissions") => Step::Permissions,
        s if s.starts_with("capture") => Step::Capture,
        "settings" => Step::YourSettings,
        "change" => Step::Change,
        s if s.starts_with("setup") => Step::Setup,
        "voice" => Step::Voice,
        _ => Step::Finale,
    };
    cx.update(|cx| o.update(cx, |o, cx| o.go(step, cx)));
    settle(cx, window)?;
    match state {
        "setup" => progress(
            &o,
            cx,
            &[(
                "speakrs",
                "pyannote-community-1-wespeaker",
                "downloading",
                167_000_000,
            )],
        ),
        "setup-done" => progress(
            &o,
            cx,
            &[
                (
                    "speakrs",
                    "pyannote-community-1-wespeaker",
                    "completed",
                    419_482_724,
                ),
                ("local_whisper", "base", "completed", 147_951_465),
                ("local", "nomic-embed-text-v1.5", "completed", 548_000_000),
            ],
        ),
        // The first frame and its OCR land on the evidence poll.
        "finale" => cx.advance_clock(onboarding::EVIDENCE_POLL),
        _ => {}
    }
    settle(cx, window)?;
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
    for (state, _) in &STATES {
        for (theme_name, setting) in themes {
            let name = format!("onboarding-{state}-{theme_name}");
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = render(cx, state, setting.clone())?;
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
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<()> {
    let shot = render(cx, golden_for(state), setting)?;
    let shot = image::imageops::resize(
        &shot,
        onboarding::SIZE.0 as u32,
        onboarding::SIZE.1 as u32,
        image::imageops::FilterType::Triangle,
    );
    shot.save(dir.join("gpui.png"))?;
    println!("  parity: wrote {}", dir.join("gpui.png").display());
    Ok(())
}
