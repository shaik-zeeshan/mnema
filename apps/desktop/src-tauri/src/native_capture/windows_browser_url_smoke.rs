//! On-device smoke harness for the Windows browser-URL-via-UI-Automation path
//! (ADR 0044). The sibling of `windows_transient_liveness_smoke`, it exercises the
//! *production* resolver ([`capture_metadata::known_browser_engine_for_exe_stem`])
//! and reader ([`super::browser_url_uia::read_active_tab_url`]) directly against an
//! installed browser, so a browser update that breaks the UIA read is caught by
//! rerunning a single command rather than only surfacing as silently-missing
//! metadata in a full capture.
//!
//! Unlike the transient-liveness smoke it does NOT drive a Tauri capture: it just
//! resolves a target browser window (the foreground window by default, or the
//! first visible top-level window of `--exe <stem>`) and times one real read.
//!
//! It is operator-run on-device — it needs a real browser with a real tab — and
//! cannot run in CI. Run from the repo with:
//! ```text
//! cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-browser-url-smoke
//! ```
//!
//! Exit codes: `--help`/`-h` ⇒ 0, a config/arg error ⇒ 2, PASS ⇒ 0, FAIL ⇒ 1.

#[cfg(target_os = "windows")]
use std::time::Instant;

#[cfg(target_os = "windows")]
use capture_metadata::{app_display_name_from_exe_path, known_browser_engine_for_exe_stem};

const SMOKE_ARG: &str = "--windows-browser-url-smoke";

#[cfg(target_os = "windows")]
pub(crate) fn maybe_run_from_args_and_exit() {
    let args = std::env::args().collect::<Vec<_>>();
    if !args.iter().any(|arg| arg == SMOKE_ARG) {
        return;
    }

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        std::process::exit(0);
    }

    // Optional `--exe <stem>` targets a specific installed browser by exe stem
    // (case-insensitive) instead of the foreground window. Mirrors the transient
    // smoke's arg loop, tolerating the argv[0] / `--` tokens.
    let mut target_stem: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            SMOKE_ARG => {}
            "--exe" => {
                index += 1;
                match args.get(index) {
                    Some(value) => target_stem = Some(value.clone()),
                    None => {
                        eprintln!(
                            "Windows browser-URL smoke configuration error: --exe requires a value"
                        );
                        print_usage();
                        std::process::exit(2);
                    }
                }
            }
            arg if index == 0 || arg == "--" => {}
            other => {
                eprintln!("Windows browser-URL smoke configuration error: unknown argument: {other}");
                print_usage();
                std::process::exit(2);
            }
        }
        index += 1;
    }

    let resolved = match &target_stem {
        Some(stem) => {
            println!(
                "Windows browser-URL smoke: targeting the first visible top-level window of exe stem {stem:?}; for the Gecko focus-climb that window should also be FOCUSED with a real tab."
            );
            resolve_target_browser_window(stem)
        }
        None => resolve_foreground_window(),
    };
    let Some((hwnd_isize, pid, exe_path)) = resolved else {
        eprintln!(
            "Windows browser-URL smoke: FAIL: no browser window found; focus a browser or pass --exe <stem>"
        );
        std::process::exit(1);
    };

    let stem = app_display_name_from_exe_path(&exe_path);
    let Some(engine) = known_browser_engine_for_exe_stem(&stem) else {
        eprintln!(
            "Windows browser-URL smoke: FAIL: foreground window {stem} is not a recognized browser; focus a Chromium/Gecko browser or pass --exe <stem>"
        );
        std::process::exit(1);
    };

    let start = Instant::now();
    let url = super::browser_url_uia::read_active_tab_url(hwnd_isize, pid, engine);
    let elapsed = start.elapsed();

    match url {
        Some(url) if url::Url::parse(&url).is_ok() => {
            println!(
                "Windows browser-URL smoke: PASS engine={engine:?} url={url} exe={stem} elapsed={elapsed:?}"
            );
            std::process::exit(0);
        }
        Some(value) => {
            eprintln!("Windows browser-URL smoke: FAIL: read a non-URL value: {value}");
            std::process::exit(1);
        }
        None => {
            eprintln!(
                "Windows browser-URL smoke: FAIL: no URL read within budget (browser dormant, focus in chrome/address bar, or read timed out)"
            );
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn maybe_run_from_args_and_exit() {
    if std::env::args().any(|arg| arg == SMOKE_ARG) {
        eprintln!("Windows browser-URL smoke is Windows-only");
        std::process::exit(2);
    }
}

/// The foreground window's HWND (as `isize`), owning PID, and exe path. `None`
/// when there is no foreground window, its PID is 0, or the exe is unresolvable.
#[cfg(target_os = "windows")]
fn resolve_foreground_window() -> Option<(isize, u32, String)> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    // SAFETY: standard foreground-window queries. `GetForegroundWindow` returns a
    // borrowed HWND we never free; `GetWindowThreadProcessId` writes the PID into a
    // stack local. `windows_process_image_path` sizes its own buffer and closes the
    // handle it opens.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }
        let exe_path = windows_process_image_path(pid)?;
        Some((hwnd as isize, pid, exe_path))
    }
}

#[cfg(target_os = "windows")]
struct WindowEnumState {
    windows: Vec<(isize, u32)>,
}

/// `EnumWindows` callback: record each visible top-level window's HWND (as `isize`)
/// and owning PID into the [`WindowEnumState`] handed in via `lparam`.
///
/// # Safety
/// `lparam` must be a valid `*mut WindowEnumState` for the whole enumeration, as
/// passed by [`resolve_target_browser_window`].
#[cfg(target_os = "windows")]
unsafe extern "system" fn collect_visible_windows(
    hwnd: windows_sys::Win32::Foundation::HWND,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible};

    if IsWindowVisible(hwnd) == 0 {
        return 1; // TRUE: skip this one, keep enumerating.
    }
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid != 0 {
        let state = &mut *(lparam as *mut WindowEnumState);
        state.windows.push((hwnd as isize, pid));
    }
    1 // TRUE: continue enumeration.
}

/// Enumerate visible top-level windows and return the first whose owning exe stem
/// equals `target_stem` (ASCII case-insensitive) AND resolves to a known browser
/// engine. `None` when no such window exists.
#[cfg(target_os = "windows")]
fn resolve_target_browser_window(target_stem: &str) -> Option<(isize, u32, String)> {
    use windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows;

    let mut state = WindowEnumState {
        windows: Vec::new(),
    };
    // SAFETY: `EnumWindows` invokes `collect_visible_windows` synchronously for each
    // top-level window, passing the `&mut state` pointer we hand it as the LPARAM.
    // The pointer stays valid for the whole call and is not retained afterwards.
    unsafe {
        EnumWindows(
            Some(collect_visible_windows),
            &mut state as *mut WindowEnumState as isize,
        );
    }

    for (hwnd_isize, pid) in state.windows {
        // SAFETY: `windows_process_image_path` opens and unconditionally closes one
        // process handle per PID.
        let Some(exe_path) = (unsafe { windows_process_image_path(pid) }) else {
            continue;
        };
        let stem = app_display_name_from_exe_path(&exe_path);
        if stem.eq_ignore_ascii_case(target_stem)
            && known_browser_engine_for_exe_stem(&stem).is_some()
        {
            return Some((hwnd_isize, pid, exe_path));
        }
    }
    None
}

/// Resolve a process's canonical executable path via `QueryFullProcessImageNameW`
/// (`PROCESS_NAME_WIN32`). `None` on any failure, including access-denied. Mirrors
/// `native_capture_metadata::windows_process_image_path`.
///
/// # Safety
/// Opens and unconditionally closes a process handle for `pid`.
#[cfg(target_os = "windows")]
unsafe fn windows_process_image_path(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        return None;
    }

    // MAX_PATH is only a floor — long paths exceed it — so start generous.
    let mut buffer: Vec<u16> = vec![0u16; 1024];
    let mut size: u32 = buffer.len() as u32;
    let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size);
    CloseHandle(handle);

    if ok == 0 || size == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buffer[..size as usize]);
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_string())
}

#[cfg(target_os = "windows")]
fn print_usage() {
    println!(
        "Windows browser-URL smoke (ADR 0044)\n\nFocus a Chromium (Chrome/Edge/Brave/Vivaldi/Opera/Arc, incl. Helium-as-chrome.exe) or Gecko (Firefox/Zen/LibreWolf/Waterfox/Floorp) browser with a REAL tab open (URL focus, not the address bar), then run from the repo with:\n  cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-browser-url-smoke\n\nIt exercises the production resolver + UI Automation reader against the foreground window and PASSes when a recognized browser yields a well-formed URL within budget, printing the engine, URL, and timing.\n\nOptions:\n  --exe <stem>   Target a specific installed browser by exe stem instead of the\n                 foreground window (case-insensitive, e.g. Helium ships as\n                 `chrome`, Zen as `zen`). For the Gecko focus-climb the targeted\n                 window should still be focused with a real tab.\n"
    );
}
