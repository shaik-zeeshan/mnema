//! System facts — the real numbers Settings needs to state a consequence.
//!
//! Round-4 decision **G8** ("honest numbers only"): a denominator ships only
//! where the value is real on *this* machine. Every reader here is best-effort
//! and returns `None` on failure so the UI can drop the number instead of
//! inventing one. Nothing here produces a temperature or a minute-precise ETA;
//! both are banned by G8.
//!
//! Everything but two facts is read through machinery that already exists:
//! free space via [`crate::native_capture::disk_space::measure_free_space`],
//! backlogs via `AppInfra::processing_pipeline_status`, vector counts via the
//! semantic-search store, DB size via `AppInfra::status`. The two new readers
//! are physical RAM (a `hw.memsize` sysctl) and the measured capture rate (the
//! recordings tree is day-partitioned, so a day's bytes are one directory).

use std::path::{Path, PathBuf};

use capture_types::SystemFacts;

use crate::app_infra::AppInfraState;

/// Bytes of one Semantic Search Vector at rest — `int8[768]`, migration `0039`.
const SEMANTIC_VECTOR_BYTES: u64 = 768;

/// How many recent complete day-directories the capture-rate average covers.
/// A week smooths weekday/weekend without reaching back past a plausible
/// settings change.
const MEASURED_DAY_WINDOW: usize = 7;

/// One `#[tauri::command]`, one facts struct. `(async)` matters: this stats a
/// directory tree on the user's capture volume, which may be an external or
/// network disk whose syscalls block — on the plain (main-thread) path that
/// would freeze the whole app for a readout, exactly as
/// `probe_storage_path` documents.
#[tauri::command(async)]
pub async fn get_system_facts(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppInfraState>,
) -> Result<SystemFacts, String> {
    let settings = crate::native_capture::settings::load_recording_settings_or_default(&app_handle);
    let base_dir = crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
        &settings.save_directory,
    );
    let capture_root = base_dir.base_dir().clone();
    let recordings_root = base_dir.recordings_root();

    let infra = std::sync::Arc::clone(&*state);

    // Backlogs: one grouped query already used by the debug pipeline page.
    // "Backlog" = work not yet done, so queued + running.
    let (ocr_backlog, transcription_backlog) = match infra.processing_pipeline_status().await {
        Ok(rows) => {
            let depth = |processor: &str| {
                rows.iter()
                    .find(|row| row.processor == processor)
                    .map(|row| row.queued + row.running)
                    .unwrap_or(0)
            };
            (
                Some(depth(::app_infra::OCR_PROCESSOR)),
                Some(depth(::app_infra::AUDIO_TRANSCRIPTION_PROCESSOR)),
            )
        }
        Err(_) => (None, None),
    };

    let semantic = infra.semantic_search();
    let (measured_bytes_per_day, measured_days) = measure_bytes_per_day(&recordings_root);

    Ok(SystemFacts {
        capture_path: capture_root.display().to_string(),
        disk_free_bytes: crate::native_capture::disk_space::measure_free_space(
            &capture_root,
            crate::native_capture::disk_space::default_free_space_probe,
        ),
        total_ram_bytes: total_ram_bytes(),
        measured_bytes_per_day,
        measured_days,
        screen_frame_rate: Some(settings.screen_frame_rate),
        ocr_backlog,
        transcription_backlog,
        semantic_vector_count: semantic.count_vectors().await.ok(),
        semantic_pending_count: semantic.count_anchors_missing_vector().await.ok(),
        semantic_vector_bytes: SEMANTIC_VECTOR_BYTES,
        database_bytes: infra
            .status()
            .await
            .ok()
            .and_then(|s| s.database_size_bytes),
    })
}

/// Physical RAM from the `hw.memsize` sysctl — the same `sysctlbyname` shape
/// `app_infra::machine_id` uses for `hw.model`. `None` rather than a guess when
/// the call fails, so the UI drops the denominator (G8).
#[cfg(target_os = "macos")]
fn total_ram_bytes() -> Option<u64> {
    let name = std::ffi::CString::new("hw.memsize").ok()?;
    let mut value: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: `value`/`len` are a correctly sized, aligned out-parameter pair
    // for a `uint64_t` sysctl; the name is NUL-terminated; no new value is set.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0 && len == std::mem::size_of::<u64>() && value > 0).then_some(value)
}

#[cfg(not(target_os = "macos"))]
fn total_ram_bytes() -> Option<u64> {
    None
}

/// Average bytes/day over the most recent complete day-directories under the
/// recordings root, which is partitioned `recordings/YYYY/MM/DD/`. Returns
/// `(None, 0)` until at least one complete day exists — before then there is no
/// honest rate to state.
///
/// Only days that actually hold capture files count, so the figure reads "on a
/// day you record", not "averaged over an idle week". Today is excluded: a
/// partial day would drag the rate down.
///
/// ponytail: the average spans whatever capture rate/bitrate was in force on
/// those days. A settings change shows up gradually as the window rolls
/// forward; storing a per-day rate stamp is the upgrade if that ever misleads.
fn measure_bytes_per_day(recordings_root: &Path) -> (Option<u64>, u32) {
    let mut days = day_directories(recordings_root);
    // Newest first, then drop today (incomplete).
    days.sort_by(|a, b| b.0.cmp(&a.0));
    let today = today_ymd();
    let mut total: u64 = 0;
    let mut counted: u32 = 0;
    for (ymd, dir) in days {
        if counted as usize >= MEASURED_DAY_WINDOW {
            break;
        }
        if Some(ymd) == today {
            continue;
        }
        let bytes = directory_bytes(&dir);
        if bytes == 0 {
            continue;
        }
        total = total.saturating_add(bytes);
        counted += 1;
    }
    if counted == 0 {
        return (None, 0);
    }
    (Some(total / u64::from(counted)), counted)
}

/// `(yyyymmdd, path)` for every `YYYY/MM/DD` directory under the root.
/// Non-numeric entries (`.DS_Store`, anything hand-dropped) are skipped.
fn day_directories(recordings_root: &Path) -> Vec<(u32, PathBuf)> {
    let mut out = Vec::new();
    for (year, year_dir) in numeric_children(recordings_root, 1970..=9999) {
        for (month, month_dir) in numeric_children(&year_dir, 1..=12) {
            for (day, day_dir) in numeric_children(&month_dir, 1..=31) {
                out.push((year * 10_000 + month * 100 + day, day_dir));
            }
        }
    }
    out
}

fn numeric_children(dir: &Path, range: std::ops::RangeInclusive<u32>) -> Vec<(u32, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|entry| {
            let value: u32 = entry.file_name().to_str()?.parse().ok()?;
            range.contains(&value).then(|| (value, entry.path()))
        })
        .collect()
}

/// Sum of the file sizes directly inside one day directory (the capture writers
/// keep segments flat there). Unreadable entries contribute nothing.
fn directory_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .sum()
}

/// Today as `yyyymmdd` in UTC — the same clock the recordings tree is written
/// with. `None` if the date can't be read, in which case no day is excluded.
fn today_ymd() -> Option<u32> {
    let now = time::OffsetDateTime::now_utc();
    Some(now.year() as u32 * 10_000 + u32::from(u8::from(now.month())) * 100 + u32::from(now.day()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_day(root: &Path, ymd: &str, sizes: &[usize]) {
        let dir = root.join(&ymd[0..4]).join(&ymd[4..6]).join(&ymd[6..8]);
        std::fs::create_dir_all(&dir).expect("day dir");
        for (i, size) in sizes.iter().enumerate() {
            std::fs::write(dir.join(format!("segment-{i}.mov")), vec![0u8; *size]).expect("write");
        }
    }

    #[test]
    fn measures_the_average_over_complete_recording_days() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path();
        write_day(root, "20260701", &[1_000, 2_000]);
        write_day(root, "20260702", &[3_000]);

        let (bytes_per_day, days) = measure_bytes_per_day(root);

        assert_eq!(days, 2);
        assert_eq!(bytes_per_day, Some(3_000)); // (3_000 + 3_000) / 2
    }

    #[test]
    fn empty_days_and_non_numeric_entries_never_count() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path();
        write_day(root, "20260701", &[4_000]);
        // A day directory that exists but holds nothing must not dilute the
        // average to 2_000/day.
        std::fs::create_dir_all(root.join("2026").join("07").join("02")).expect("empty day");
        std::fs::write(root.join(".DS_Store"), b"junk").expect("junk file");
        std::fs::create_dir_all(root.join("scratch")).expect("non-numeric dir");

        assert_eq!(measure_bytes_per_day(root), (Some(4_000), 1));
    }

    #[test]
    fn no_complete_day_reports_nothing_rather_than_zero() {
        let temp = tempfile::tempdir().expect("temp");

        assert_eq!(measure_bytes_per_day(temp.path()), (None, 0));
        assert_eq!(
            measure_bytes_per_day(&temp.path().join("missing")),
            (None, 0)
        );
    }

    #[test]
    fn todays_partial_day_is_excluded() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path();
        let today = today_ymd().expect("today");
        write_day(root, &today.to_string(), &[9_999_999]);

        assert_eq!(measure_bytes_per_day(root), (None, 0));
    }

    #[test]
    fn the_window_caps_how_far_back_the_average_reaches() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path();
        // 8 days, oldest 10x the rest: excluded, so the average stays 1_000.
        write_day(root, "20260701", &[10_000]);
        for day in 2..=8 {
            write_day(root, &format!("202607{day:02}"), &[1_000]);
        }

        let (bytes_per_day, days) = measure_bytes_per_day(root);

        assert_eq!(days, MEASURED_DAY_WINDOW as u32);
        assert_eq!(bytes_per_day, Some(1_000));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn physical_ram_reads_a_plausible_value() {
        let ram = total_ram_bytes().expect("macOS always answers hw.memsize");
        assert!(ram >= 1 << 30, "at least 1 GB, got {ram}");
    }
}
