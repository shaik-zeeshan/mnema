//! Windows audio-endpoint hotplug support-gate smoke test.
//!
//! Regression harness for the stale support-gate bug: the app used to latch
//! `microphone_capture_supported()` / `system_audio_loopback_capture_supported()`
//! in a `OnceLock` at first call, so a microphone connected after launch stayed
//! "unsupported" (recording start failed with `microphone_permission_denied`)
//! until the app was restarted. The gates must now track endpoint hotplug live.
//!
//! Manual procedure (single process — do NOT restart between steps):
//!   1. Unplug/disable every capture device (Settings -> System -> Sound, or
//!      unplug the USB mic), then run:
//!        cargo run -p capture-microphone --example smoke_support_hotplug
//!      Expect: `microphone supported = false`, permission `Unsupported`.
//!   2. While the loop is running, plug the microphone back in (or re-enable it).
//!      Expect within ~2s: a CHANGE line flipping `microphone supported = true`
//!      and permission to `Unknown`. That flip inside one process is the PASS.
//!   3. Optionally unplug again and watch it flip back to `false`.
//!
//! The same flip should appear for `system audio` when the default render
//! endpoint (speakers/headphones) is removed/added.
//!
//! Run from a PowerShell with the MSVC + Perl/NASM env imported.

#[cfg(target_os = "windows")]
fn main() {
    use capture_microphone::{
        ensure_microphone_permission, list_microphone_devices, microphone_permission_state,
        system_audio_loopback_capture_supported,
    };
    use std::time::Duration;

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(60);

    println!("== Windows audio-endpoint hotplug support-gate smoke test ==");
    println!("polling every 1s for {seconds}s; plug/unplug the microphone while running");
    println!("PASS = `microphone supported` flips within this single process\n");

    let mut last: Option<(bool, bool, String)> = None;
    for tick in 0..seconds {
        // `ensure_microphone_permission()` is the exact gate recording start
        // uses (`start_capture_runtime`), so probe through it rather than a
        // lower-level helper.
        let microphone = ensure_microphone_permission();
        let system_audio = system_audio_loopback_capture_supported();
        let permission = format!("{:?}", microphone_permission_state());
        let snapshot = (microphone, system_audio, permission.clone());

        if last.as_ref() != Some(&snapshot) {
            let device_count = list_microphone_devices()
                .map(|devices| devices.len().to_string())
                .unwrap_or_else(|e| format!("enumeration failed: {}", e.code));
            let label = if last.is_some() { "CHANGE" } else { "initial" };
            println!(
                "[t+{tick:>3}s] {label}: microphone supported = {microphone}, \
                 system audio supported = {system_audio}, \
                 permission = {permission}, capture devices = {device_count}"
            );
            last = Some(snapshot);
        }

        std::thread::sleep(Duration::from_secs(1));
    }

    println!("\ndone ({seconds}s elapsed); exit is success either way — PASS/FAIL is the observed flip");
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("smoke_support_hotplug is a Windows-only smoke test");
}
