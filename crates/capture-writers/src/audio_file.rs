//! Opens an `AVAudioFile` for writing with exactly one reference.
//!
//! cidre 0.15 over-retains `initForWriting:` the same way it does
//! `initForReading:` (see `audio_read.rs`), so cidre's `open_write*` never frees
//! the file. This calls the raw `_ar` binding and adopts its +1 as-is, so
//! dropping the `arc::R` deallocates the file and closes its descriptor even on
//! an early error return before `close()`.

use cidre::{arc, av, ns};

/// Opens `url` for writing `settings`, fed deinterleaved f32 buffers.
pub fn open_audio_file_for_writing<'ear>(
    url: &ns::Url,
    settings: &ns::Dictionary<ns::String, ns::Id>,
) -> ns::Result<'ear, arc::R<av::AudioFile>> {
    ns::if_none(|err| unsafe {
        let owned = av::AudioFile::alloc().init_for_writing_common_format_err_ar(
            url,
            settings,
            av::AudioCommonFormat::PcmF32,
            false,
            err,
        );
        // SAFETY: an init-family method returns +1. `Rar` and `R` are both a
        // transparent non-null pointer; adopting it without a retain leaves
        // exactly the one release `R`'s Drop performs.
        std::mem::transmute::<Option<arc::Rar<av::AudioFile>>, Option<arc::R<av::AudioFile>>>(owned)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cidre::objc::autorelease_pool::AutoreleasePoolPage;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    /// A copy of a stock system sound per test, so parallel tests never share
    /// the path whose descriptors they count.
    fn fixture(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("mnema-{name}-{}.aiff", std::process::id()));
        std::fs::copy("/System/Library/Sounds/Tink.aiff", &path).expect("copy fixture");
        path
    }

    unsafe extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }

    /// Open descriptors in this process whose path is `path`.
    fn open_fds_for(path: &Path) -> usize {
        const F_GETPATH: i32 = 50;
        let want = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let fds: Vec<i32> = std::fs::read_dir("/dev/fd")
            .expect("list /dev/fd")
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .collect();
        fds.into_iter()
            .filter(|&fd| {
                let mut buf = [0u8; 1024];
                if unsafe { fcntl(fd, F_GETPATH, buf.as_mut_ptr()) } == -1 {
                    return false;
                }
                let len = buf.iter().position(|&b| b == 0).unwrap_or(0);
                Path::new(std::ffi::OsStr::from_bytes(&buf[..len])) == want
            })
            .count()
    }

    #[test]
    fn writer_is_deallocated_exactly_once_on_drop() {
        use cidre::cat;
        let output = fixture("fd-writer").with_extension("m4a");
        for _ in 0..50 {
            let _pool = AutoreleasePoolPage::push();
            let url = ns::Url::with_fs_path_str(&output.to_string_lossy(), false);
            let format_id = ns::Number::with_u32(cat::audio::Format::MPEG4_AAC.0);
            let rate = ns::Number::with_f64(48_000.0);
            let channels = ns::Number::with_i64(1);
            let settings: arc::R<ns::Dictionary<ns::String, ns::Id>> =
                ns::Dictionary::with_keys_values(
                    &[
                        av::audio::all_formats_keys::id(),
                        av::audio::all_formats_keys::sample_rate(),
                        av::audio::all_formats_keys::number_of_channels(),
                    ],
                    &[
                        format_id.as_id_ref(),
                        rate.as_id_ref(),
                        channels.as_id_ref(),
                    ],
                );
            let mut file = open_audio_file_for_writing(&url, &settings).expect("writer opens");
            let mut buffer = av::AudioPcmBuf::with_format(&file.processing_format(), 4_800)
                .expect("buffer allocates");
            buffer.set_frame_len(4_800).expect("frame length sets");
            file.write(&buffer).expect("silence writes");
            file.close();
            let weak = arc::downgrade(&file);
            drop(file);
            assert!(
                weak.upgrade().is_none(),
                "AVAudioFile writer outlived its last owner"
            );
        }
        assert_eq!(open_fds_for(&output), 0);
        let _ = std::fs::remove_file(&output);
    }

    #[test]
    fn decode_and_trim_close_every_file_they_open() {
        let input = fixture("fd-trim-input");
        let input_str = input.to_string_lossy().to_string();
        let output = input.with_extension("m4a");
        let output_str = output.to_string_lossy().to_string();
        for _ in 0..50 {
            crate::decode_audio_file_to_mono_pcm(&input).expect("fixture decodes");
            // Keep the middle: drops well over the trim threshold, so it re-encodes.
            assert!(
                crate::trim_audio_file_to_m4a(&input_str, &output_str, 0.1, 0.2).expect("trim")
            );
        }
        assert_eq!(open_fds_for(&input), 0, "reader fds leaked");
        assert_eq!(open_fds_for(&output), 0, "writer/validator fds leaked");
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&output);
    }
}
