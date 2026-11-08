use super::PrivateIpBlockingResolver;
use std::str::FromStr;

// SSRF connect-time boundary: the fetch client's custom resolver must REFUSE a
// host that resolves to a private / loopback address — the case a URL-host
// check cannot catch (a public hostname pointed at an internal IP via
// split-horizon DNS / rebinding). `localhost` resolves via /etc/hosts to
// 127.0.0.1 / ::1 with no network, so this is offline-deterministic: the
// resolver must return an error rather than the loopback address, so an
// internal service can never be fetched and streamed into the cloud model.
#[test]
fn resolver_refuses_host_resolving_to_loopback() {
    let resolver = PrivateIpBlockingResolver;
    let name = reqwest::dns::Name::from_str("localhost").expect("localhost is a valid dns name");
    let result = crate::test_support::block_on(reqwest::dns::Resolve::resolve(&resolver, name));
    assert!(
        result.is_err(),
        "resolver must refuse a host that resolves to loopback (SSRF), got Ok"
    );
}
