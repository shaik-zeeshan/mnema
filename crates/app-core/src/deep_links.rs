//! The pure half of deep links (`mnema://…`, `mnema-dev://…`, and the GPUI dev
//! bundle's `mnema-gpui-dev://…`): which of the five unrelated payloads a URL
//! carries (license activation, purchase claim, renewal return, MCP OAuth
//! callback, capture-broker handoff), discriminated by host+path so none
//! swallows another. Each host routes the [`DeepLink`] to its windows.

#[cfg(test)]
mod tests;

/// The schemes a Mnema build registers: prod, the Tauri dev sandbox, and the
/// GPUI dev bundle (`scripts/bundle-gpui-app.sh --dev`).
fn is_mnema_scheme(scheme: &str) -> bool {
    matches!(scheme, "mnema" | "mnema-dev" | "mnema-gpui-dev")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    LicenseActivate { key: String },
    LicenseClaim { checkout_id: String },
    LicenseRenewed,
    OAuthCallback,
    BrokerOpen { opaque_id: String },
}

/// Which payload `url` carries, or `None` for a URL that isn't ours.
pub fn route(url: &url::Url) -> Option<DeepLink> {
    if let Some(key) = license_key_from_url(url) {
        Some(DeepLink::LicenseActivate { key })
    } else if let Some(checkout_id) = claim_checkout_id_from_url(url) {
        Some(DeepLink::LicenseClaim { checkout_id })
    } else if is_license_renewed_url(url) {
        Some(DeepLink::LicenseRenewed)
    } else if is_oauth_callback_url(url) {
        Some(DeepLink::OAuthCallback)
    } else {
        broker_opaque_id_from_url(url).map(|opaque_id| DeepLink::BrokerOpen { opaque_id })
    }
}

pub fn broker_opaque_id_from_url(url: &url::Url) -> Option<String> {
    // The CLI mints `mnema://open/<id>` only; the GPUI dev bundle owns its own scheme.
    if !matches!(url.scheme(), "mnema" | "mnema-gpui-dev") {
        return None;
    }
    let mut segments = url.path_segments()?.collect::<Vec<_>>();
    if let Some(host) = url.host_str() {
        segments.insert(0, host);
    }
    match segments.as_slice() {
        ["open", opaque_id] | ["broker", "open", opaque_id] => Some((*opaque_id).to_string()),
        _ => None,
    }
}

/// Whether `url` is our MCP OAuth deep-link callback (`mnema://oauth/callback` in
/// prod, `mnema-dev://oauth/callback` in dev). A custom-scheme URL parses its first
/// path element as the HOST, so the callback lands as host `oauth` + path
/// `/callback`. Broker deep links use host `open`/`broker`, so the two never
/// collide.
pub fn is_oauth_callback_url(url: &url::Url) -> bool {
    is_mnema_scheme(url.scheme()) && url.host_str() == Some("oauth") && url.path() == "/callback"
}

/// The license-activation deep link (`mnema://license/activate?key=…` in prod,
/// `mnema-dev://…` in dev). Host `license` + path `/activate` is distinct from the
/// oauth (`oauth`) and broker (`open`/`broker`) hosts, so the three never collide.
/// Returns the URL-decoded key (standard base64 `.`-joined, as minted).
pub fn license_key_from_url(url: &url::Url) -> Option<String> {
    if !is_mnema_scheme(url.scheme()) {
        return None;
    }
    if url.host_str() != Some("license") || url.path() != "/activate" {
        return None;
    }
    url.query_pairs()
        .find(|(k, _)| k == "key")
        .map(|(_, v)| v.into_owned())
        .filter(|k| !k.is_empty())
}

/// The purchase-claim deep link (`mnema://license/claim?checkout_id=…` in prod,
/// `mnema-dev://…` in dev) — Polar's success redirect. Host `license` + path
/// `/claim` sits beside the activate route and apart from the oauth/broker hosts.
pub fn claim_checkout_id_from_url(url: &url::Url) -> Option<String> {
    if !is_mnema_scheme(url.scheme()) {
        return None;
    }
    if url.host_str() != Some("license") || url.path() != "/claim" {
        return None;
    }
    url.query_pairs()
        .find(|(k, _)| k == "checkout_id")
        .map(|(_, v)| v.into_owned())
        .filter(|id| !id.is_empty())
}

/// The renewal-return deep link (`mnema://license/renewed` in prod,
/// `mnema-dev://…` in dev) — the Polar RENEWAL product's success redirect. No
/// payload: the machine already holds the key (a renewal extends the existing
/// license, ADR 0055), so the hit only cues a short Receipt Refresh poll.
/// Ops note: the Polar renewal product's success URL must point at
/// `https://mnema.day/license/open?flow=renewal&checkout_id={CHECKOUT_ID}` — the
/// bounce page that fires this deep link (providers can't emit custom schemes).
pub fn is_license_renewed_url(url: &url::Url) -> bool {
    is_mnema_scheme(url.scheme()) && url.host_str() == Some("license") && url.path() == "/renewed"
}
