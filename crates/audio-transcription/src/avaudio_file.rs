//! Opens an `AVAudioFile` for reading while owning exactly one reference.
//!
//! cidre 0.15's `msg_send` macro treats only `new*`/`initWith*` selectors as
//! returning an owned (+1) object, so `AVAudioFile::open_read*`
//! (`initForReading:…`, init-family too) retains the owned result again and the
//! `arc::R` never reaches zero: every decode leaked the AVAudioFile, its
//! ExtAudioFile and an open file descriptor. This calls the raw `_ar` binding
//! and adopts its +1 as-is, so dropping the `arc::R` deallocates the file and
//! closes the fd.
//!
//! Copy of `capture_writers::open_audio_file_for_reading` (this crate does not
//! depend on capture-writers); keep the copies in sync.

use cidre::{arc, av, ns};

/// Opens `url` for reading with a deinterleaved f32 processing format.
pub(crate) fn open_audio_file_for_reading<'ear>(
    url: &ns::Url,
) -> ns::Result<'ear, arc::R<av::AudioFile>> {
    ns::if_none(|err| unsafe {
        let owned = av::AudioFile::alloc().init_for_reading_common_format_err_ar(
            url,
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

    #[test]
    fn reader_is_deallocated_exactly_once_on_drop() {
        for _ in 0..200 {
            let _pool = AutoreleasePoolPage::push();
            let url = ns::Url::with_fs_path_str("/System/Library/Sounds/Tink.aiff", false);
            let file = open_audio_file_for_reading(&url).expect("stock sound opens");
            assert!(file.len() > 0);
            let weak = arc::downgrade(&file);
            drop(file);
            // Still alive here = leaked fd; an over-release would crash instead.
            assert!(
                weak.upgrade().is_none(),
                "AVAudioFile outlived its last owner"
            );
        }
    }
}
