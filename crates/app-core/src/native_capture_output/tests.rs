use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "macos")]
struct TestDir {
    path: PathBuf,
}

#[cfg(target_os = "macos")]
impl TestDir {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("native-capture-output-{label}-{unique}"));
        fs::create_dir_all(&path).expect("test directory should exist");
        Self { path }
    }
}

#[cfg(target_os = "macos")]
fn write_openable_screen_file(path: &Path) {
    fs::write(path, b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10moovtrak")
        .expect("screen artifact should exist");
}

// Produces a real, positive-duration AAC/m4a file so that the AVFoundation
// duration validation inside `finalize_capture_outputs` accepts it as usable
// captured audio. The AAC fourcc is written directly to avoid depending on
// cidre's optional `cat` module from this crate.
#[cfg(target_os = "macos")]
fn write_valid_m4a_audio_file(path: &Path) {
    use cidre::{av, ns};

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    let path_str = path.to_string_lossy().to_string();
    let url = ns::Url::with_fs_path_str(&path_str, false);
    let sample_rate = 48_000.0_f64;
    let format_id = ns::Number::with_u32(u32::from_be_bytes(*b"aac "));
    let sample_rate_value = ns::Number::with_f64(sample_rate);
    let channel_count_value = ns::Number::with_i64(1);
    let settings: cidre::arc::R<ns::Dictionary<ns::String, ns::Id>> =
        ns::Dictionary::with_keys_values(
            &[
                av::audio::all_formats_keys::id(),
                av::audio::all_formats_keys::sample_rate(),
                av::audio::all_formats_keys::number_of_channels(),
            ],
            &[
                format_id.as_id_ref(),
                sample_rate_value.as_id_ref(),
                channel_count_value.as_id_ref(),
            ],
        );
    let mut file = av::AudioFile::open_write_common_format(
        &url,
        &settings,
        av::AudioCommonFormat::PcmF32,
        false,
    )
    .expect("writable test audio file should open");
    let processing_format = file.processing_format();
    let frames: u32 = 24_000; // 0.5s @ 48kHz — comfortably positive duration.
    let mut buffer = av::AudioPcmBuf::with_format(&processing_format, frames)
        .expect("test audio buffer should allocate");
    buffer
        .set_frame_len(frames)
        .expect("frame length should set");
    if let Some(samples) = buffer.data_f32_mut_at(0) {
        for (index, sample) in samples.iter_mut().enumerate() {
            *sample = ((index as f32) * 0.05).sin() * 0.1;
        }
    }
    file.write(&buffer).expect("test audio should write");
    file.close();
}

#[cfg(target_os = "macos")]
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn finalize_capture_outputs_drops_missing_screen_output_and_skips_video_only_conversion() {
    let dir = TestDir::new("missing-screen-output");
    let recording_file = dir.path.join("screen.mov");
    let recording_file = recording_file.to_string_lossy().to_string();
    let requested_sources = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: true,
    };
    let mut output_files = CaptureOutputFiles {
        screen_file: Some(recording_file.clone()),
        screen_files: vec![recording_file.clone()],
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };

    let error = finalize_capture_outputs(
        Some(&mut output_files),
        Some(&recording_file),
        None,
        None,
        Some(&requested_sources),
    )
    .expect_err("missing requested screen recording must fail finalization");

    assert_eq!(error.code, "capture_output_processing_failed");
    assert!(error
        .message
        .contains("screen output missing: expected screen recording file"));

    assert_eq!(output_files.screen_file, None);
    assert!(output_files.screen_files.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn finalize_capture_outputs_preserves_audio_when_requested_screen_output_missing() {
    let dir = TestDir::new("preserve-audio-missing-screen");
    // A screen .mov that exists on disk but is not openable (no `moov`),
    // exactly the failure mode that was discarding captured audio.
    let recording_file = dir.path.join("screen.mov");
    fs::write(
        &recording_file,
        b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10mdatjunk",
    )
    .expect("broken screen artifact should exist");
    let recording_file = recording_file.to_string_lossy().to_string();

    let microphone_file = dir.path.join("microphone.m4a");
    write_valid_m4a_audio_file(&microphone_file);
    let microphone_file = microphone_file.to_string_lossy().to_string();

    let requested_sources = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: false,
    };
    let mut output_files = CaptureOutputFiles {
        screen_file: Some(recording_file.clone()),
        screen_files: vec![recording_file.clone()],
        microphone_file: Some(microphone_file.clone()),
        microphone_files: vec![microphone_file.clone()],
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };

    finalize_capture_outputs(
        Some(&mut output_files),
        Some(&recording_file),
        None,
        None,
        Some(&requested_sources),
    )
    .expect("captured audio must survive a missing screen segment");

    // The unusable screen output is dropped from bookkeeping and disk...
    assert_eq!(output_files.screen_file, None);
    assert!(output_files.screen_files.is_empty());
    assert!(!Path::new(&recording_file).exists());
    // ...but the captured microphone audio is preserved for commit.
    assert_eq!(output_files.microphone_file, Some(microphone_file.clone()));
    assert_eq!(output_files.microphone_files, vec![microphone_file.clone()]);
    assert!(Path::new(&microphone_file).exists());
}

#[cfg(target_os = "macos")]
#[test]
fn finalize_capture_outputs_rejects_invalid_existing_screen_output() {
    let dir = TestDir::new("invalid-screen-output");
    let recording_file = dir.path.join("screen.mov");
    fs::write(
        &recording_file,
        b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10mdatjunk",
    )
    .expect("invalid screen artifact should exist");
    let recording_file = recording_file.to_string_lossy().to_string();
    let requested_sources = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: false,
    };
    let mut output_files = CaptureOutputFiles {
        screen_file: Some(recording_file.clone()),
        screen_files: vec![recording_file.clone()],
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };

    let error = finalize_capture_outputs(
        Some(&mut output_files),
        Some(&recording_file),
        None,
        None,
        Some(&requested_sources),
    )
    .expect_err("invalid existing screen recording must fail finalization");

    assert_eq!(error.code, "capture_output_processing_failed");
    assert!(error
        .message
        .contains("screen output missing: expected screen recording file"));
    assert_eq!(output_files.screen_file, None);
    assert!(output_files.screen_files.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn finalize_capture_outputs_requires_dedicated_microphone_source_recording() {
    let mut output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some("/tmp/final-microphone.m4a".to_string()),
        microphone_files: Vec::new(),
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };

    let error = finalize_capture_outputs(
        Some(&mut output_files),
        Some("/tmp/screen-recording.mov"),
        None,
        None,
        Some(&CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }),
    )
    .expect_err("microphone finalization should not fall back to the screen recording");

    assert_eq!(error.code, "capture_output_processing_failed");
    assert!(error
        .message
        .contains("microphone output conversion failed: missing source recording"));
}

#[cfg(target_os = "macos")]
#[test]
fn sync_finalized_audio_output_files_clears_removed_audio_artifacts_from_bookkeeping() {
    let dir = TestDir::new("sync-audio-bookkeeping");
    let screen_file = dir.path.join("screen.mov");
    fs::write(&screen_file, b"screen").expect("screen artifact should exist");
    let screen_file = screen_file.to_string_lossy().to_string();
    let microphone_kept_file = dir
        .path
        .join("microphone-kept.m4a")
        .to_string_lossy()
        .to_string();
    fs::write(&microphone_kept_file, b"microphone").expect("microphone artifact should exist");
    let missing_microphone_file = dir
        .path
        .join("microphone-missing.m4a")
        .to_string_lossy()
        .to_string();
    let microphone_file = dir
        .path
        .join("microphone-legacy.m4a")
        .to_string_lossy()
        .to_string();
    fs::write(&microphone_file, b"microphone").expect("legacy microphone artifact should exist");
    let empty_microphone_file = dir
        .path
        .join("microphone-empty.m4a")
        .to_string_lossy()
        .to_string();
    fs::File::create(&empty_microphone_file)
        .expect("empty microphone artifact placeholder should exist");
    let empty_system_audio_file = dir
        .path
        .join("system-audio.m4a")
        .to_string_lossy()
        .to_string();
    fs::File::create(&empty_system_audio_file)
        .expect("empty system audio artifact placeholder should exist");
    let mut output_files = CaptureOutputFiles {
        screen_file: Some(screen_file.clone()),
        screen_files: vec![screen_file.clone()],
        microphone_file: Some(missing_microphone_file),
        microphone_files: vec![microphone_kept_file.clone(), empty_microphone_file.clone()],
        system_audio_file: Some(empty_system_audio_file.clone()),
        system_audio_files: Vec::new(),
    };
    let mut failures = Vec::new();
    let unusable_files = BTreeSet::new();

    sync_finalized_audio_output_file_with_duration_validator(
        &mut output_files.microphone_file,
        &mut output_files.microphone_files,
        "microphone",
        &unusable_files,
        &mut failures,
        |path| path == microphone_kept_file,
    );
    sync_finalized_audio_output_file_with_duration_validator(
        &mut output_files.system_audio_file,
        &mut output_files.system_audio_files,
        "system audio",
        &unusable_files,
        &mut failures,
        |_| false,
    );

    let mut legacy_output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some(microphone_file.clone()),
        microphone_files: Vec::new(),
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };
    sync_finalized_audio_output_file_with_duration_validator(
        &mut legacy_output_files.microphone_file,
        &mut legacy_output_files.microphone_files,
        "microphone",
        &unusable_files,
        &mut failures,
        |path| path == microphone_file,
    );

    assert_eq!(output_files.screen_file, Some(screen_file.clone()));
    assert_eq!(output_files.screen_files, vec![screen_file]);
    assert_eq!(
        output_files.microphone_file,
        Some(microphone_kept_file.clone())
    );
    assert_eq!(output_files.microphone_files, vec![microphone_kept_file]);
    assert_eq!(output_files.system_audio_file, None);
    assert!(output_files.system_audio_files.is_empty());

    assert_eq!(
        legacy_output_files.microphone_file,
        Some(microphone_file.clone())
    );
    assert_eq!(legacy_output_files.microphone_files, vec![microphone_file]);
    assert!(failures.is_empty());
    assert!(!Path::new(&empty_microphone_file).exists());
    assert!(!Path::new(&empty_system_audio_file).exists());
}

#[cfg(target_os = "macos")]
#[test]
fn zero_duration_non_empty_audio_output_is_unusable() {
    let dir = TestDir::new("zero-duration-audio-output");
    let microphone_file = dir
        .path
        .join("microphone-zero-duration.m4a")
        .to_string_lossy()
        .to_string();
    fs::write(&microphone_file, b"non-empty").expect("audio artifact should exist");
    let mut output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some(microphone_file.clone()),
        microphone_files: vec![microphone_file.clone()],
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };
    let mut failures = Vec::new();
    let unusable_files = BTreeSet::new();

    sync_finalized_audio_output_file_with_duration_validator(
        &mut output_files.microphone_file,
        &mut output_files.microphone_files,
        "microphone",
        &unusable_files,
        &mut failures,
        |_| false,
    );

    assert_eq!(output_files.microphone_file, None);
    assert!(output_files.microphone_files.is_empty());
    assert!(failures.is_empty());
    assert!(!Path::new(&microphone_file).exists());
}

#[cfg(target_os = "macos")]
#[test]
fn positive_duration_short_system_audio_output_is_usable() {
    let dir = TestDir::new("short-system-audio-output");
    let system_audio_file = dir
        .path
        .join("system-audio-short.m4a")
        .to_string_lossy()
        .to_string();
    fs::write(&system_audio_file, b"short-active").expect("audio artifact should exist");
    let mut output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: Some(system_audio_file.clone()),
        system_audio_files: vec![system_audio_file.clone()],
    };
    let mut failures = Vec::new();
    let unusable_files = BTreeSet::new();

    sync_finalized_audio_output_file_with_duration_validator(
        &mut output_files.system_audio_file,
        &mut output_files.system_audio_files,
        "system audio",
        &unusable_files,
        &mut failures,
        |_| true,
    );

    assert_eq!(
        output_files.system_audio_file,
        Some(system_audio_file.clone())
    );
    assert_eq!(output_files.system_audio_files, vec![system_audio_file]);
    assert!(failures.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn positive_duration_ignored_microphone_output_is_removed_from_payload() {
    let dir = TestDir::new("ignored-microphone-output");
    let microphone_file = dir
        .path
        .join("microphone-silent-startup.m4a")
        .to_string_lossy()
        .to_string();
    fs::write(&microphone_file, b"positive-duration-silence")
        .expect("microphone artifact should exist");
    let mut output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some(microphone_file.clone()),
        microphone_files: vec![microphone_file.clone()],
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };
    let mut failures = Vec::new();
    let unusable_files = BTreeSet::from([microphone_file.clone()]);

    sync_finalized_audio_output_file_with_duration_validator(
        &mut output_files.microphone_file,
        &mut output_files.microphone_files,
        "microphone",
        &unusable_files,
        &mut failures,
        |_| true,
    );

    assert_eq!(output_files.microphone_file, None);
    assert!(output_files.microphone_files.is_empty());
    assert!(failures.is_empty());
    assert!(!Path::new(&microphone_file).exists());
}

#[cfg(target_os = "macos")]
#[test]
fn finalize_capture_outputs_treats_deleted_audio_only_outputs_as_nonfatal() {
    let dir = TestDir::new("deleted-audio-only-output");
    let microphone_file = dir
        .path
        .join("microphone-empty.m4a")
        .to_string_lossy()
        .to_string();
    let system_audio_file = dir
        .path
        .join("system-audio-empty.m4a")
        .to_string_lossy()
        .to_string();
    fs::File::create(&microphone_file).expect("empty microphone placeholder should exist");
    fs::File::create(&system_audio_file).expect("empty system-audio placeholder should exist");
    let mut output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: Some(microphone_file.clone()),
        microphone_files: vec![microphone_file.clone()],
        system_audio_file: Some(system_audio_file.clone()),
        system_audio_files: vec![system_audio_file.clone()],
    };

    finalize_capture_outputs(
        Some(&mut output_files),
        None,
        Some(&microphone_file),
        Some(&system_audio_file),
        Some(&CaptureSources {
            screen: false,
            microphone: true,
            system_audio: true,
        }),
    )
    .expect("empty audio-only inactivity outputs should be ignored");

    assert_eq!(output_files.microphone_file, None);
    assert!(output_files.microphone_files.is_empty());
    assert_eq!(output_files.system_audio_file, None);
    assert!(output_files.system_audio_files.is_empty());
    assert!(!Path::new(&microphone_file).exists());
    assert!(!Path::new(&system_audio_file).exists());
}

#[cfg(target_os = "macos")]
#[test]
fn finalize_capture_outputs_treats_ignored_system_audio_only_output_as_nonfatal() {
    let dir = TestDir::new("ignored-system-audio-only-output");
    let system_audio_file = dir
        .path
        .join("system-audio-empty.m4a")
        .to_string_lossy()
        .to_string();
    fs::File::create(&system_audio_file).expect("empty system-audio placeholder should exist");
    let mut output_files = CaptureOutputFiles {
        screen_file: None,
        screen_files: Vec::new(),
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: Some(system_audio_file.clone()),
        system_audio_files: vec![system_audio_file.clone()],
    };

    finalize_capture_outputs(
        Some(&mut output_files),
        None,
        None,
        None,
        Some(&CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        }),
    )
    .expect("ignored audio-only inactivity output should be nonfatal");

    assert_eq!(output_files.system_audio_file, None);
    assert!(output_files.system_audio_files.is_empty());
    assert!(!Path::new(&system_audio_file).exists());
}

#[cfg(target_os = "macos")]
#[test]
fn finalize_capture_outputs_preserves_screen_output_while_dropping_audio_conversion_failures() {
    let dir = TestDir::new("preserve-screen-drop-audio-conversion-failures");
    let screen_file = dir.path.join("screen.mov");
    write_openable_screen_file(&screen_file);
    let screen_file = screen_file.to_string_lossy().to_string();
    let microphone_file = dir
        .path
        .join("microphone.m4a")
        .to_string_lossy()
        .to_string();
    let system_audio_file = dir
        .path
        .join("system-audio.m4a")
        .to_string_lossy()
        .to_string();
    let mut output_files = CaptureOutputFiles {
        screen_file: Some(screen_file.clone()),
        screen_files: vec![screen_file.clone()],
        microphone_file: Some(microphone_file),
        microphone_files: Vec::new(),
        system_audio_file: Some(system_audio_file),
        system_audio_files: Vec::new(),
    };

    finalize_capture_outputs(
        Some(&mut output_files),
        Some(&screen_file),
        None,
        None,
        None,
    )
    .expect("audio-only conversion failures should not block valid screen output");

    assert_eq!(output_files.screen_file, Some(screen_file.clone()));
    assert_eq!(output_files.screen_files, vec![screen_file]);
    assert_eq!(output_files.microphone_file, None);
    assert!(output_files.microphone_files.is_empty());
    assert_eq!(output_files.system_audio_file, None);
    assert!(output_files.system_audio_files.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn finalize_capture_outputs_rejects_invalid_screen_output_when_system_audio_is_requested() {
    let dir = TestDir::new("preserve-screen-strip-audio-failure");
    let screen_file = dir.path.join("screen.mov");
    fs::write(&screen_file, b"not a real mov").expect("screen artifact should exist");
    let screen_file = screen_file.to_string_lossy().to_string();
    let requested_sources = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: true,
    };
    let mut output_files = CaptureOutputFiles {
        screen_file: Some(screen_file.clone()),
        screen_files: vec![screen_file.clone()],
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: None,
        system_audio_files: Vec::new(),
    };

    let error = finalize_capture_outputs(
        Some(&mut output_files),
        Some(&screen_file),
        None,
        None,
        Some(&requested_sources),
    )
    .expect_err("invalid screen output should not be committed");

    assert_eq!(error.code, "capture_output_processing_failed");
    assert!(error
        .message
        .contains("screen output missing: expected screen recording file"));
    assert_eq!(output_files.screen_file, None);
    assert!(output_files.screen_files.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn finalize_capture_outputs_preserves_screen_output_while_dropping_missing_audio_outputs() {
    let dir = TestDir::new("preserve-screen-drop-audio");
    let screen_file = dir.path.join("screen.mov");
    write_openable_screen_file(&screen_file);
    let screen_file = screen_file.to_string_lossy().to_string();
    let microphone_file = dir
        .path
        .join("microphone.m4a")
        .to_string_lossy()
        .to_string();
    let system_audio_file = dir
        .path
        .join("system-audio.m4a")
        .to_string_lossy()
        .to_string();
    let mut output_files = CaptureOutputFiles {
        screen_file: Some(screen_file.clone()),
        screen_files: vec![screen_file.clone()],
        microphone_file: Some(microphone_file.clone()),
        microphone_files: vec![microphone_file.clone()],
        system_audio_file: Some(system_audio_file.clone()),
        system_audio_files: vec![system_audio_file.clone()],
    };

    finalize_capture_outputs(
        Some(&mut output_files),
        Some(&screen_file),
        Some(&microphone_file),
        Some(&system_audio_file),
        None,
    )
    .expect("missing audio artifacts should not block finalized screen output");

    assert_eq!(output_files.screen_file, Some(screen_file.clone()));
    assert_eq!(output_files.screen_files, vec![screen_file]);
    assert_eq!(output_files.microphone_file, None);
    assert!(output_files.microphone_files.is_empty());
    assert_eq!(output_files.system_audio_file, None);
    assert!(output_files.system_audio_files.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn cleanup_unusable_segment_artifacts_removes_audio_files_outside_screen_segment_directory() {
    let dir = TestDir::new("cleanup-separate-audio-dirs");
    let screen_dir = dir.path.join("session-1-segment-0001");
    let audio_dir = dir.path.join("session-1-audio").join("segment-0001");
    fs::create_dir_all(&screen_dir).expect("screen dir should exist");
    fs::create_dir_all(&audio_dir).expect("audio dir should exist");

    let screen_file = screen_dir.join("screen.mov");
    let microphone_file = audio_dir.join("microphone.m4a");
    let system_audio_file = audio_dir.join("system-audio.m4a");
    fs::write(&screen_file, b"screen").expect("screen artifact should exist");
    fs::write(&microphone_file, b"microphone").expect("microphone artifact should exist");
    fs::write(&system_audio_file, b"system-audio").expect("system audio artifact should exist");

    let output_files = CaptureOutputFiles {
        screen_file: Some(screen_file.to_string_lossy().to_string()),
        screen_files: vec![screen_file.to_string_lossy().to_string()],
        microphone_file: Some(microphone_file.to_string_lossy().to_string()),
        microphone_files: vec![microphone_file.to_string_lossy().to_string()],
        system_audio_file: Some(system_audio_file.to_string_lossy().to_string()),
        system_audio_files: vec![system_audio_file.to_string_lossy().to_string()],
    };

    cleanup_unusable_segment_artifacts(
        Some(&output_files),
        output_files.screen_file.as_deref(),
        output_files.microphone_file.as_deref(),
        output_files.system_audio_file.as_deref(),
    );

    assert!(!screen_file.exists());
    assert!(!microphone_file.exists());
    assert!(!system_audio_file.exists());
}
