//! candle version lockstep gate (CI, no network).
//!
//! `candle-core`, `candle-nn`, and `candle-transformers` MUST move together: a
//! non-lockstep bump (e.g. `candle-transformers` to a version whose `nomic_bert` /
//! `xlm_roberta` modules drift while `candle-core` stays put) can silently change
//! the architecture the catalog dispatches to. The three are EXACT-pinned (`=x.y.z`)
//! in `Cargo.toml`; this test parses those three pins straight out of the manifest
//! and asserts they are (a) all `=` exact pins and (b) the SAME version — so a
//! one-crate bump fails CI here. This is the candle-era replacement for the retired
//! `ort` lockstep guard. It does NOT change the pins; it only enforces them.

/// The crate's own `Cargo.toml`, baked into the test binary at compile time so the
/// test needs no filesystem path resolution at runtime.
const CARGO_TOML: &str = include_str!("../Cargo.toml");

/// Extract the `version = "..."` string from a `name = { version = "..." }` line in
/// the `[dependencies]` section. Returns the raw version literal INCLUDING any `=`
/// exact-pin prefix (e.g. `=0.10.2`), so the caller can assert both the pin form and
/// the value. Minimal hand parse — no toml dep — matching the line shape this
/// manifest uses (`candle-core = { version = "=0.10.2" }`).
fn pinned_version(manifest: &str, crate_name: &str) -> String {
    let needle = format!("{crate_name} = ");
    let line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with(&needle))
        .unwrap_or_else(|| panic!("`{crate_name}` dependency line not found in Cargo.toml"));

    // Find `version = "..."` on that line and lift the quoted literal.
    let after = line
        .split_once("version")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(_, rest)| rest)
        .unwrap_or_else(|| panic!("no `version = \"...\"` on the `{crate_name}` line"));
    let value = after
        .split_once('"')
        .map(|(value, _)| value)
        .unwrap_or_else(|| panic!("unterminated version string on the `{crate_name}` line"));
    value.to_string()
}

#[test]
fn candle_crates_are_pinned_in_lockstep() {
    let core = pinned_version(CARGO_TOML, "candle-core");
    let nn = pinned_version(CARGO_TOML, "candle-nn");
    let transformers = pinned_version(CARGO_TOML, "candle-transformers");

    // (a) Every candle crate must be an EXACT pin (`=x.y.z`), not a caret/range — a
    // range would let one crate float and break lockstep silently.
    for (name, pin) in [
        ("candle-core", &core),
        ("candle-nn", &nn),
        ("candle-transformers", &transformers),
    ] {
        assert!(
            pin.starts_with('='),
            "`{name}` must be an exact `=` pin (got `{pin}`) so the three candle \
             crates can be enforced in lockstep"
        );
    }

    // (b) The three pins must be the SAME version. A non-lockstep bump (one crate
    // moved, the others not) fails here.
    assert_eq!(
        core, nn,
        "candle-core ({core}) and candle-nn ({nn}) must be pinned to the same version"
    );
    assert_eq!(
        core, transformers,
        "candle-core ({core}) and candle-transformers ({transformers}) must be pinned to the same version"
    );
}
