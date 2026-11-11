//! Settings goldens (`settings-<state>-<theme>.png`): the shell on the Settings
//! surface, filled from `apps/desktop/parity/fixtures/settings/default.json`,
//! one per group top plus the deeper sections of the tall groups (each a
//! 1280x800 viewport, under the offscreen clamp), and the parity shot for
//! `scripts/parity.sh settings <state>`. `settings-*privacy-prompt-<theme>`
//! add the one-time Sensitive Capture Protection prompt (catalog below).

use std::path::Path;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::{AppContext, Context, VisualTestAppContext, Window};
use mnema_gpui::settings::fixture::{
    open_mcp_state, open_provider_state, settings_data, FixtureBackend, MCP_STATES,
    PROVIDER_STATES, STATES,
};
use mnema_gpui::settings::privacy_prompt::{PromptData, Recommended};
use mnema_gpui::settings::Settings;
use mnema_gpui::shell::{Shell, ShellModel, Surface};
use mnema_gpui::theme;

use super::compare;

pub const VIEWPORT: (f32, f32) = (1280., 800.);

/// Prompt goldens: `(state, ?tab=)`. The fixture already excludes 1Password
/// and Passwords, so two of the four catalog apps are pending.
const PROMPT_STATES: [(&str, &str); 2] = [
    ("privacy-prompt", "appearance"),
    ("capture-privacy-prompt", "privacy"),
];

fn prompt_catalog() -> PromptData {
    let app = |b: &str, n: &str| Recommended {
        bundle_id: b.into(),
        display_name: n.into(),
    };
    PromptData {
        prompt_id: "sensitive_capture_protection_v1_recommended_exclusions".into(),
        apps: vec![
            app("com.apple.Passwords", "Apple Passwords"),
            app("com.apple.keychainaccess", "Keychain Access"),
            app("com.1password.1password", "1Password"),
            app("com.bitwarden.desktop", "Bitwarden"),
        ],
        closed: false,
    }
}

/// The MCP modals open over Intelligence.
fn mcp_modal(state: &str) -> Option<&'static str> {
    MCP_STATES
        .iter()
        .find(|(s, _)| *s == state)
        .map(|(_, m)| *m)
}

fn tab_for(state: &str) -> anyhow::Result<&'static str> {
    if mcp_modal(state).is_some() || PROVIDER_STATES.contains(&state) {
        return Ok("intelligence");
    }
    STATES
        .iter()
        .chain(PROMPT_STATES.iter())
        .find(|(s, _)| *s == state)
        .map(|(_, t)| *t)
        .ok_or_else(|| anyhow::anyhow!("no settings state {state}"))
}

pub fn render(
    cx: &mut VisualTestAppContext,
    shell_model: ShellModel,
    state: &str,
    setting: AppearanceSetting,
) -> anyhow::Result<image::RgbaImage> {
    let tab = tab_for(state)?;
    let mut data = settings_data()?;
    if PROMPT_STATES.iter().any(|(s, _)| *s == state) {
        data.privacy_prompt = Some(prompt_catalog());
    }
    let modal = mcp_modal(state);
    let provider = PROVIDER_STATES.iter().find(|s| **s == state).copied();
    render_open(
        cx,
        shell_model,
        tab,
        data,
        setting,
        None,
        move |s, window, cx| {
            if let Some(which) = modal {
                open_mcp_state(s, which, window, cx)
            }
            if let Some(which) = provider {
                open_provider_state(s, which, window, cx)
            }
        },
    )
}

/// The shell on Settings at `tab`, then `open` run on the settled page (a
/// modal, a menu) and element `click` clicked (pointer parked after), before the shot.
pub fn render_open(
    cx: &mut VisualTestAppContext,
    shell_model: ShellModel,
    tab: &str,
    data: mnema_gpui::settings::SettingsData,
    setting: AppearanceSetting,
    click: Option<&'static str>,
    open: impl FnOnce(&mut Settings, &mut Window, &mut Context<Settings>),
) -> anyhow::Result<image::RgbaImage> {
    cx.update(|cx| theme::set_setting(setting, cx));
    let mut settings = None;
    let window = crate::pointer::open(cx, compare::viewport(VIEWPORT), |window, cx| {
        let shell = cx.new(|cx| {
            let mut shell = Shell::new(shell_model, window, cx);
            shell.go(Surface::Settings, cx);
            shell
        });
        let s = shell.read(cx).settings.clone();
        s.update(cx, |s, cx| {
            let backend = FixtureBackend::default();
            *backend.mcp_tools.lock().unwrap() = vec![
                (
                    "search_code".into(),
                    Some("Search code across your repositories.".into()),
                ),
                (
                    "get_issue".into(),
                    Some("Read one issue and its comments.".into()),
                ),
                ("create_pull_request".into(), None),
            ];
            s.set_backend(std::sync::Arc::new(backend));
            s.set_data(data, cx)
        });
        settings = Some(s);
        cx.new(|cx| Root::new(shell, window, cx))
    })?;
    let settings = settings.expect("settings");
    let mut open = Some(open);
    // Lay out once, then deeplink (the scroll needs the sections' bounds), then settle.
    for step in 0..4 {
        cx.run_until_parked();
        if step == 1 {
            cx.update(|cx| settings.update(cx, |s, cx| s.open_tab(tab, cx)));
        }
        if step == 2 {
            let open = open.take().expect("once");
            cx.update_window(window.into(), |_, window, cx| {
                settings.update(cx, |s, cx| open(s, window, cx))
            })?;
        }
        if let (3, Some(id)) = (step, click) {
            crate::layers::click(cx, window.into(), id)?;
        }
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
    }
    cx.run_until_parked();
    let shot = cx.capture_screenshot(window.into())?;
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(shot)
}

pub fn goldens(
    cx: &mut VisualTestAppContext,
    filters: &[String],
    themes: &[(&str, AppearanceSetting)],
    shell_model: &ShellModel,
) -> anyhow::Result<Vec<String>> {
    let mut failures = Vec::new();
    let states = STATES
        .iter()
        .chain(PROMPT_STATES.iter())
        .chain(MCP_STATES.iter())
        .map(|(s, _)| *s)
        .chain(PROVIDER_STATES);
    for state in states {
        for (theme_name, setting) in themes {
            let name = format!("settings-{state}-{theme_name}");
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            let shot = render(cx, shell_model.clone(), state, setting.clone())?;
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
    shell_model: ShellModel,
) -> anyhow::Result<()> {
    let shot = render(cx, shell_model, state, setting)?;
    let shot = image::imageops::resize(
        &shot,
        VIEWPORT.0 as u32,
        VIEWPORT.1 as u32,
        image::imageops::FilterType::Triangle,
    );
    shot.save(dir.join("gpui.png"))?;
    println!("  parity: wrote {}", dir.join("gpui.png").display());
    Ok(())
}
