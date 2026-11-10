//! Deep links: `on_open_urls` (no `&mut App`) → channel → here, on the main
//! thread. app-core's `deep_links::route` says what a URL carries; this routes
//! it: license links open the receipt on main (`licence::announce`), start their
//! app-core flow and bring main up, an OAuth
//! callback finishes the MCP login, a broker link lands the Timeline on its
//! frame. The dev bundle's scheme is `mnema-gpui-dev`.

use std::sync::Arc;

use app_core::deep_links::{route, DeepLink};
use app_core::AppCore;
use gpui_kit::App;

use crate::bridge::tokio::Tokio;
use crate::licence::receipt::Flow;

/// What one URL string asks for (`None` for garbage or a URL that isn't ours).
pub fn parse(url: &str) -> Option<(url::Url, DeepLink)> {
    let url = url::Url::parse(url).ok()?;
    let link = route(&url)?;
    Some((url, link))
}

/// Handle URLs as they arrive, including the ones that launched the app.
pub fn spawn(core: Arc<AppCore>, rx: async_channel::Receiver<Vec<String>>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Ok(urls) = rx.recv().await {
            for url in urls {
                let core = core.clone();
                let _ = cx.update(|cx| open(&core, &url, cx));
            }
        }
    })
    .detach();
}

fn open(core: &Arc<AppCore>, url: &str, cx: &mut App) {
    let Some((url, link)) = parse(url) else {
        // Never log the URL: it can carry a license key or an id.
        log::debug!("deep link ignored: not a Mnema link");
        return;
    };
    // The variant name only: payloads carry keys and ids.
    log::debug!(
        "deep link routed: {}",
        format!("{link:?}").split([' ', '{']).next().unwrap_or("?")
    );
    match link {
        // The receipt opens first (working face); every result rides the
        // `license_status` / `license_deep_link_done` events. As Tauri's
        // dispatcher: main comes up after activation, before the claim and
        // renewal polls; while onboarding shows, the receipt waits for main.
        DeepLink::LicenseActivate { key } => {
            crate::licence::announce(Flow::Activate, cx);
            let core = core.clone();
            let task = Tokio::spawn(cx, app_core::licensing::activate_from_deep_link(core, key));
            cx.spawn(async move |cx| {
                let _ = task.await;
                let _ = cx.update(crate::window::ensure_main);
            })
            .detach();
        }
        DeepLink::LicenseClaim { checkout_id } => {
            crate::licence::announce(Flow::Claim, cx);
            // Surface the app first: the claim poll can take ~30 s.
            crate::window::ensure_main(cx);
            Tokio::spawn(
                cx,
                app_core::licensing::claim_from_deep_link(core.clone(), checkout_id),
            )
            .detach();
        }
        DeepLink::LicenseRenewed => {
            crate::licence::announce(Flow::Renewed, cx);
            crate::window::ensure_main(cx);
            Tokio::spawn(
                cx,
                app_core::licensing::renewed_from_deep_link(core.clone()),
            )
            .detach();
        }
        DeepLink::OAuthCallback => core
            .state::<app_core::ask_ai::mcp::McpManager>()
            .complete_oauth_callback(core, &url),
        DeepLink::BrokerOpen { opaque_id } => broker_open(core, opaque_id, cx),
    }
}

/// Re-authorize the signed opaque id, then land the Timeline on its frame.
fn broker_open(core: &Arc<AppCore>, opaque_id: String, cx: &mut App) {
    let config = core.paths().config.clone();
    let task = Tokio::spawn(cx, async move {
        app_infra::brokered_access::authorize_active_opaque_capture_reference(&config, &opaque_id)
            .await
            .ok()
            .flatten()
    });
    cx.spawn(async move |cx| {
        let Ok(Some(capture)) = task.await else {
            return;
        };
        let _ = cx.update(|cx| {
            crate::window::show_main(cx);
            let Some(shell) = crate::window::shell(cx) else {
                return;
            };
            shell.update(cx, |shell, cx| {
                shell.go(crate::shell::Surface::Timeline, cx);
                if let Some(frame) = capture.frame_id {
                    let timeline = shell.timeline.clone();
                    timeline.update(cx, |t, cx| t.jump_to(frame, cx));
                }
            });
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_bundle_urls_route_and_garbage_does_not() {
        let link = |s: &str| parse(s).map(|(_, l)| l);
        assert_eq!(
            link("mnema-gpui-dev://open/abc"),
            Some(DeepLink::BrokerOpen {
                opaque_id: "abc".into()
            })
        );
        assert_eq!(
            link("mnema-gpui-dev://license/activate?key=k.sig"),
            Some(DeepLink::LicenseActivate {
                key: "k.sig".into()
            })
        );
        assert_eq!(
            link("mnema-gpui-dev://oauth/callback?code=1&state=2"),
            Some(DeepLink::OAuthCallback)
        );
        assert_eq!(
            link("mnema://license/renewed"),
            Some(DeepLink::LicenseRenewed)
        );
        assert_eq!(link("not a url"), None);
        assert_eq!(link("https://mnema.day/open/abc"), None);
        assert_eq!(link("mnema-gpui-dev://nothing/here"), None);
    }
}
