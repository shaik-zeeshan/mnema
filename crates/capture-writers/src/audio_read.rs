use cidre::{arc, av, ns};
use std::ops::{Deref, DerefMut};

/// An `AVAudioFile` opened for reading that frees its file handle when dropped.
/// Open every read through this, never raw `av::AudioFile::open_read*`: those
/// leak the file and its handle (see `balance_extra_retain`).
pub struct AudioFileReader(arc::R<av::AudioFile>);

impl AudioFileReader {
    /// Opens `url` for reading, decoding to non-interleaved `common_format` when
    /// given. The error is the Display text of the `NSError`.
    pub fn open(
        url: &ns::Url,
        common_format: Option<av::AudioCommonFormat>,
    ) -> Result<Self, String> {
        let file = match common_format {
            Some(format) => av::AudioFile::open_read_common_format(url, format, false),
            None => av::AudioFile::open_read(url),
        }
        .map_err(|error| error.to_string())?;
        Ok(Self(balance_extra_retain(file)))
    }
}

/// cidre 0.15 adds a retain to `initForReading:` results, which are already +1,
/// so release once here to leave `file` the only owner.
fn balance_extra_retain(file: arc::R<av::AudioFile>) -> arc::R<av::AudioFile> {
    // SAFETY: cidre 0.15 over-retains `initForReading:` (see the canary test), so this release only drops that extra count.
    drop(unsafe {
        std::mem::transmute_copy::<arc::R<av::AudioFile>, arc::R<av::AudioFile>>(&file)
    });
    file
}

impl Deref for AudioFileReader {
    type Target = av::AudioFile;

    fn deref(&self) -> &av::AudioFile {
        &self.0
    }
}

impl DerefMut for AudioFileReader {
    fn deref_mut(&mut self) -> &mut av::AudioFile {
        &mut self.0
    }
}

impl Drop for AudioFileReader {
    fn drop(&mut self) {
        // Release the handle now even if something else still holds the object.
        // `-[AVAudioFile close]` is macOS 15+; below that the release that
        // follows frees the object, which closes the file.
        if cidre::api::version!(macos = 15.0) {
            self.0.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mnema-audio-read-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        // lsof reports resolved paths (/private/var/...), so match on the canonical form.
        std::fs::canonicalize(&dir).expect("temp dir should canonicalize")
    }

    /// Paths under `dir` this process still holds open, per lsof. Looks only
    /// inside `dir`, so other tests running in parallel do not affect it.
    fn open_handles_under(dir: &Path) -> Vec<String> {
        let output = std::process::Command::new("/usr/sbin/lsof")
            .args(["-p", &std::process::id().to_string(), "-Fn"])
            .output()
            .expect("lsof should run");
        assert!(
            output.status.success(),
            "lsof failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let prefix = dir.to_string_lossy().to_string();
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix('n'))
            .filter(|name| name.starts_with(&prefix))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn audio_reads_leave_no_open_file_handles() {
        let dir = unique_temp_dir("leak");
        let input = dir.join("tone.m4a").to_string_lossy().to_string();
        let output = dir.join("trimmed.m4a").to_string_lossy().to_string();
        crate::tests::write_test_tone_m4a(&input, 2.0);

        for _ in 0..50 {
            crate::decode_audio_file_to_mono_pcm(Path::new(&input)).expect("tone should decode");
        }
        // A range past the end fails after the reader is open.
        crate::trim_audio_file_to_m4a(&input, &output, 10.0, 11.0)
            .expect_err("a range past the end should fail");
        // Covers the trim reader and the trimmed-output validation read.
        assert!(
            crate::trim_audio_file_to_m4a(&input, &output, 0.5, 1.0).expect("trim should succeed")
        );

        let open = open_handles_under(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            open.is_empty(),
            "audio reads leaked open file handles: {open:?}"
        );
    }

    #[test]
    fn cidre_still_over_retains_init_for_reading() {
        let dir = unique_temp_dir("canary");
        let input = dir.join("tone.m4a").to_string_lossy().to_string();
        crate::tests::write_test_tone_m4a(&input, 0.5);
        let _pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
        let url = ns::Url::with_fs_path_str(&input, false);
        for format in [None, Some(av::AudioCommonFormat::PcmF32)] {
            let raw = match format {
                Some(f) => av::AudioFile::open_read_common_format(&url, f, false),
                None => av::AudioFile::open_read(&url),
            }
            .expect("tone should open");
            assert_eq!(
                raw.as_type_ref().retain_count(),
                2,
                "cidre no longer over-retains `initForReading:` ({format:?}), so delete the balancing \
                 release in audio_read.rs (balance_extra_retain); otherwise it becomes a double free"
            );
            drop(balance_extra_retain(raw));
            let reader = AudioFileReader::open(&url, format).expect("tone should open");
            assert_eq!(
                reader.0.as_type_ref().retain_count(),
                1,
                "reader must be the sole owner ({format:?})"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
