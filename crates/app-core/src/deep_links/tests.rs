use super::{
    broker_opaque_id_from_url, claim_checkout_id_from_url, is_license_renewed_url,
    is_oauth_callback_url, license_key_from_url, route, DeepLink,
};

/// The deep-link dispatch discriminator: an OAuth callback and a capture-broker
/// handoff share the `mnema`/`mnema-dev` scheme, so they must NOT swallow each
/// other. `is_oauth_callback_url` claims only the `oauth/callback` host+path,
/// and `broker_opaque_id_from_url` claims only the broker hosts — in BOTH
/// directions.
#[test]
fn oauth_callback_and_broker_urls_dispatch_apart() {
    let prod = url::Url::parse("mnema://oauth/callback?code=abc&state=xyz").expect("url");
    let dev = url::Url::parse("mnema-dev://oauth/callback?code=abc&state=xyz").expect("url");
    assert!(is_oauth_callback_url(&prod));
    assert!(is_oauth_callback_url(&dev));

    let broker_open = url::Url::parse("mnema://open/f1").expect("url");
    let broker_nested = url::Url::parse("mnema://broker/open/f1").expect("url");
    // A broker URL is never read as an oauth callback…
    assert!(!is_oauth_callback_url(&broker_open));
    assert!(!is_oauth_callback_url(&broker_nested));
    // …and an oauth callback is never read as a broker URL (no cross-swallow).
    assert!(broker_opaque_id_from_url(&prod).is_none());
    assert_eq!(
        broker_opaque_id_from_url(&broker_open).as_deref(),
        Some("f1")
    );
    assert_eq!(
        broker_opaque_id_from_url(&broker_nested).as_deref(),
        Some("f1")
    );
}

#[test]
fn license_deep_link_extracts_url_decoded_key_and_stays_apart() {
    // Standard base64 (`+ / =`) joined by `.`, percent-encoded in the URL.
    let key = "eyJlbWFpbCI6ImFAYi5jbyJ9.sig+with/slash=";
    let url = url::Url::parse(&format!(
        "mnema://license/activate?key={}",
        "eyJlbWFpbCI6ImFAYi5jbyJ9.sig%2Bwith%2Fslash%3D"
    ))
    .expect("url");
    assert_eq!(license_key_from_url(&url).as_deref(), Some(key));

    // Never confused for oauth or broker; never claims their URLs.
    assert!(!is_oauth_callback_url(&url));
    assert!(broker_opaque_id_from_url(&url).is_none());
    let oauth = url::Url::parse("mnema://oauth/callback?code=x").expect("url");
    let broker = url::Url::parse("mnema://open/f1").expect("url");
    assert!(license_key_from_url(&oauth).is_none());
    assert!(license_key_from_url(&broker).is_none());
    // Missing/empty key -> not an activation link.
    assert!(
        license_key_from_url(&url::Url::parse("mnema://license/activate").expect("url")).is_none()
    );
}

#[test]
fn claim_deep_link_extracts_checkout_id_and_stays_apart() {
    let prod = url::Url::parse("mnema://license/claim?checkout_id=co_abc123").expect("url");
    let dev = url::Url::parse("mnema-dev://license/claim?checkout_id=co_abc123").expect("url");
    assert_eq!(
        claim_checkout_id_from_url(&prod).as_deref(),
        Some("co_abc123")
    );
    assert_eq!(
        claim_checkout_id_from_url(&dev).as_deref(),
        Some("co_abc123")
    );

    // Never claims the activate/oauth/broker payloads…
    let activate = url::Url::parse("mnema://license/activate?key=abc.def").expect("url");
    let oauth = url::Url::parse("mnema://oauth/callback?code=x").expect("url");
    let broker = url::Url::parse("mnema://open/f1").expect("url");
    assert!(claim_checkout_id_from_url(&activate).is_none());
    assert!(claim_checkout_id_from_url(&oauth).is_none());
    assert!(claim_checkout_id_from_url(&broker).is_none());
    // …and none of them claims a claim URL (no cross-swallow).
    assert!(license_key_from_url(&prod).is_none());
    assert!(!is_oauth_callback_url(&prod));
    assert!(broker_opaque_id_from_url(&prod).is_none());

    // Missing/empty checkout_id -> not a claim link. Wrong scheme neither.
    assert!(
        claim_checkout_id_from_url(&url::Url::parse("mnema://license/claim").expect("url"))
            .is_none()
    );
    assert!(claim_checkout_id_from_url(
        &url::Url::parse("mnema://license/claim?checkout_id=").expect("url")
    )
    .is_none());
    assert!(claim_checkout_id_from_url(
        &url::Url::parse("https://license/claim?checkout_id=co_abc123").expect("url")
    )
    .is_none());
}

#[test]
fn renewed_deep_link_matches_both_schemes_and_stays_apart() {
    let prod = url::Url::parse("mnema://license/renewed").expect("url");
    let dev = url::Url::parse("mnema-dev://license/renewed").expect("url");
    assert!(is_license_renewed_url(&prod));
    assert!(is_license_renewed_url(&dev));
    // A stray query string (checkout providers append them) still matches.
    assert!(is_license_renewed_url(
        &url::Url::parse("mnema://license/renewed?checkout_id=co_x").expect("url")
    ));

    // Never claims the activate/claim/oauth/broker payloads…
    let activate = url::Url::parse("mnema://license/activate?key=abc.def").expect("url");
    let claim = url::Url::parse("mnema://license/claim?checkout_id=co_x").expect("url");
    let oauth = url::Url::parse("mnema://oauth/callback?code=x").expect("url");
    let broker = url::Url::parse("mnema://open/f1").expect("url");
    assert!(!is_license_renewed_url(&activate));
    assert!(!is_license_renewed_url(&claim));
    assert!(!is_license_renewed_url(&oauth));
    assert!(!is_license_renewed_url(&broker));
    // …and none of them claims a renewed URL (no cross-swallow).
    assert!(license_key_from_url(&prod).is_none());
    assert!(claim_checkout_id_from_url(&prod).is_none());
    assert!(!is_oauth_callback_url(&prod));
    assert!(broker_opaque_id_from_url(&prod).is_none());

    // Wrong scheme -> not a renewed link.
    assert!(!is_license_renewed_url(
        &url::Url::parse("https://license/renewed").expect("url")
    ));
}

#[test]
fn route_picks_each_payload_once() {
    let parse = |s: &str| url::Url::parse(s).expect("url");
    assert_eq!(
        route(&parse("mnema://license/activate?key=abc.def")),
        Some(DeepLink::LicenseActivate {
            key: "abc.def".into()
        })
    );
    assert_eq!(
        route(&parse("mnema-gpui-dev://license/claim?checkout_id=co_1")),
        Some(DeepLink::LicenseClaim {
            checkout_id: "co_1".into()
        })
    );
    assert_eq!(
        route(&parse("mnema-dev://license/renewed")),
        Some(DeepLink::LicenseRenewed)
    );
    assert_eq!(
        route(&parse("mnema-gpui-dev://oauth/callback?code=x")),
        Some(DeepLink::OAuthCallback)
    );
    assert_eq!(
        route(&parse("mnema-gpui-dev://open/f1")),
        Some(DeepLink::BrokerOpen {
            opaque_id: "f1".into()
        })
    );
    // The Tauri dev scheme never carried broker links (the CLI mints `mnema://`).
    assert_eq!(route(&parse("mnema-dev://open/f1")), None);
    assert_eq!(route(&parse("https://open/f1")), None);
    assert_eq!(route(&parse("mnema://settings")), None);
}
