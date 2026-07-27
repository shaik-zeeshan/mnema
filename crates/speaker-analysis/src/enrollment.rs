//! Voice enrollment: an audio file in, one voiceprint or one typed rejection out.
//!
//! **All enrollment judgment lives here.** Callers (the Voice onboarding step and
//! the Settings re-enroll surface) get either a voiceprint they can store against
//! the account owner's Person Profile, or a reason they can show the user — never
//! a raw diarization result they have to interpret. No database, no Tauri, no
//! policy anywhere else.
//!
//! It runs the shipped speakrs provider through the same
//! [`analyze_speakrs_request_blocking`] entry the DER bench binary uses, so an
//! enrollment clip is diarized exactly like a recorded segment. Diarization,
//! recognition thresholds, and models are untouched.
//!
//! Identity is never inferred from capture family — this module only ever sees a
//! path.

use std::path::Path;

use crate::providers::shared::{
    audio_peak, decode_audio_to_mono_16khz, validate_decoded_samples, MIN_DIARIZATION_PEAK,
    SAMPLE_RATE_HZ,
};
use crate::providers::speakrs::analyze_speakrs_request_blocking;
use crate::{
    SpeakerAnalysisRequest, SpeakerAnalysisResult, SPEAKRS_DEFAULT_MODEL_ID, SPEAKRS_PROVIDER_ID,
};

/// Shortest clip that can produce a usable voiceprint. The mockup copy states it
/// as "under about ten seconds there is not enough voice to build a print from",
/// so the number and the message have to agree.
pub const MIN_ENROLLMENT_AUDIO_MS: u64 = 10_000;

/// The one thing enrollment can return: a voiceprint, or why it refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnrollmentOutcome {
    /// A single speaker was heard for long enough. `embedding` is the speakrs
    /// cluster centroid in the crate's little-endian storage encoding, ready for
    /// `person_voice_embeddings.embedding`.
    ///
    /// `model_id` is the **preset** id (the Voiceprint Space), not the embedding
    /// model id: recognition filters enrollments on
    /// `person_voice_embeddings.model_id == recording_speaker_clusters.model_id`,
    /// which is the preset. Storing the embedding model id here silently drops
    /// the enrollment from every future match.
    Voiceprint { embedding: Vec<u8>, model_id: String },
    /// Under [`MIN_ENROLLMENT_AUDIO_MS`].
    TooShort { duration_ms: u64 },
    /// Silence, or audio that carried no speech speakrs could cluster.
    NoSpeech,
    /// More than one speaker in the clip — someone else was audible, so we do not
    /// know which voice is the user's.
    MultipleSpeakers { speaker_count: usize },
}

/// Judge one enrollment clip.
///
/// `Err` is reserved for things that are not the recording's fault — a missing
/// model bundle, an undecodable file, a runtime failure. Everything the user can
/// fix by recording again comes back as an `Ok` rejection variant.
pub fn embed_enrollment_clip(
    audio_path: &Path,
    models_dir: &Path,
) -> SpeakerAnalysisResult<EnrollmentOutcome> {
    // Cheap gates first, off one decode: a too-short or silent clip is decided
    // without loading the pipeline (and so without the model bundle installed).
    // ponytail: this decodes once here and the provider decodes again. A 15s clip
    // is milliseconds; plumbing pre-decoded samples through the provider entry
    // would mean a second entry point to keep in sync.
    let samples = decode_audio_to_mono_16khz(audio_path)?;
    validate_decoded_samples(&samples)?;
    let duration_ms = samples.len() as u64 * 1000 / SAMPLE_RATE_HZ as u64;
    if duration_ms < MIN_ENROLLMENT_AUDIO_MS {
        return Ok(EnrollmentOutcome::TooShort { duration_ms });
    }
    if audio_peak(&samples) < MIN_DIARIZATION_PEAK {
        return Ok(EnrollmentOutcome::NoSpeech);
    }

    let request = SpeakerAnalysisRequest::new(
        audio_path,
        SPEAKRS_PROVIDER_ID,
        Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
        "voice-enrollment",
        0,
    );
    let output = analyze_speakrs_request_blocking(request, models_dir)?;

    // Placeholder clusters carry an empty embedding (a turn whose centroid was
    // skipped); they are not a speaker we can enroll or count.
    let mut voiceprints = output
        .clusters
        .into_iter()
        .filter(|cluster| !cluster.embedding.is_empty());
    let Some(first) = voiceprints.next() else {
        return Ok(EnrollmentOutcome::NoSpeech);
    };
    // ponytail: any second cluster rejects, however brief. A one-word interjection
    // from someone else costs the user a retake; learning the wrong voice as theirs
    // costs more. Add a minimum-speech-duration guard per cluster if retakes prove
    // annoying in practice.
    let extra = voiceprints.count();
    if extra > 0 {
        return Ok(EnrollmentOutcome::MultipleSpeakers {
            speaker_count: extra + 1,
        });
    }
    Ok(EnrollmentOutcome::Voiceprint {
        embedding: first.embedding,
        model_id: SPEAKRS_DEFAULT_MODEL_ID.to_string(),
    })
}

#[cfg(test)]
mod tests {
    //! Fixture-driven, following `scripts/diarization_bench`: the real shipped
    //! speakrs provider, no database.
    //!
    //! Fixtures are **generated offline** by macOS `say` (there is no speech audio
    //! in-tree and the DER corpus is a network download), plus a synthesised
    //! silent WAV. The two cases that actually diarize need the speakrs model
    //! bundle installed at the app's model store — the same prerequisite
    //! `run_der.py` has — and are `#[ignore]`d without it.

    use super::*;
    use std::{path::PathBuf, process::Command};

    fn models_dir() -> PathBuf {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join("Library/Application Support/day.mnema")
            .join(crate::MODEL_STORE_DIR_NAME)
    }

    fn models_installed() -> bool {
        models_dir().join(SPEAKRS_PROVIDER_ID).join(SPEAKRS_DEFAULT_MODEL_ID).is_dir()
    }

    /// 16-bit mono PCM payload of a WAV file (chunk-walked: `say` emits a FLLR
    /// padding chunk before `data`).
    fn wav_pcm(path: &Path) -> Vec<u8> {
        let bytes = std::fs::read(path).expect("read wav");
        let mut cursor = 12; // past "RIFF<size>WAVE"
        while cursor + 8 <= bytes.len() {
            let id = &bytes[cursor..cursor + 4];
            let size =
                u32::from_le_bytes([bytes[cursor + 4], bytes[cursor + 5], bytes[cursor + 6], bytes[cursor + 7]])
                    as usize;
            let body = cursor + 8;
            if id == b"data" {
                return bytes[body..(body + size).min(bytes.len())].to_vec();
            }
            cursor = body + size + (size % 2);
        }
        panic!("no data chunk in {}", path.display());
    }

    fn write_wav(path: &Path, pcm: &[u8]) {
        let mut out = Vec::with_capacity(44 + pcm.len());
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + pcm.len()) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16_u32.to_le_bytes());
        out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1_u16.to_le_bytes()); // mono
        out.extend_from_slice(&16_000_u32.to_le_bytes());
        out.extend_from_slice(&32_000_u32.to_le_bytes()); // byte rate
        out.extend_from_slice(&2_u16.to_le_bytes()); // block align
        out.extend_from_slice(&16_u16.to_le_bytes()); // bits
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        out.extend_from_slice(pcm);
        std::fs::write(path, out).expect("write wav");
    }

    /// Synthesise speech with macOS `say` straight to mono 16 kHz PCM WAV.
    fn say(voice: &str, text: &str, path: &Path) {
        let status = Command::new("/usr/bin/say")
            .args(["-v", voice, "--data-format=LEI16@16000", "--file-format=WAVE", "-o"])
            .arg(path)
            .arg(text)
            .status()
            .expect("run say");
        assert!(status.success(), "say failed for voice {voice}");
    }

    const LONG_TEXT: &str = "The quick brown fox jumps over the lazy dog. \
        Pack my box with five dozen liquor jugs. How razorback jumping frogs can \
        level six piqued gymnasts. The five boxing wizards jump quickly at dawn.";
    const SECOND_TEXT: &str = "Sphinx of black quartz, judge my vow. The job \
        requires extra pluck and zeal from every young wage earner. Crazy Fredrick \
        bought many very exquisite opal jewels.";

    #[test]
    fn silence_is_rejected_as_no_speech() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("silence.wav");
        // 12s of digital silence — past the length gate, so this tests the speech
        // gate and not the length one.
        write_wav(&path, &vec![0_u8; 16_000 * 2 * 12]);

        let outcome = embed_enrollment_clip(&path, &models_dir()).expect("silence decodes");
        assert_eq!(outcome, EnrollmentOutcome::NoSpeech);
    }

    #[test]
    fn sub_minimum_clip_is_rejected_as_too_short() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("short.wav");
        say("Alex", "Just a few words.", &path);

        let outcome = embed_enrollment_clip(&path, &models_dir()).expect("short clip decodes");
        match outcome {
            EnrollmentOutcome::TooShort { duration_ms } => {
                assert!(duration_ms < MIN_ENROLLMENT_AUDIO_MS, "got {duration_ms}ms");
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn clean_single_speaker_clip_yields_a_voiceprint() {
        if !models_installed() {
            eprintln!("skipping: speakrs models not installed at {}", models_dir().display());
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("one.wav");
        say("Alex", LONG_TEXT, &path);

        let outcome = embed_enrollment_clip(&path, &models_dir()).expect("single-speaker clip");
        match outcome {
            EnrollmentOutcome::Voiceprint { embedding, model_id } => {
                assert!(!embedding.is_empty());
                assert_eq!(embedding.len() % 4, 0, "embedding is little-endian f32");
                // Recognition filters on the preset id, not the embedding model id.
                assert_eq!(model_id, SPEAKRS_DEFAULT_MODEL_ID);
            }
            other => panic!("expected a voiceprint, got {other:?}"),
        }
    }

    #[test]
    fn two_speaker_clip_is_rejected_as_multiple_speakers() {
        if !models_installed() {
            eprintln!("skipping: speakrs models not installed at {}", models_dir().display());
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let (first, second) = (dir.path().join("a.wav"), dir.path().join("b.wav"));
        say("Alex", LONG_TEXT, &first);
        say("Samantha", SECOND_TEXT, &second);
        let mut pcm = wav_pcm(&first);
        pcm.extend_from_slice(&wav_pcm(&second));
        let path = dir.path().join("two.wav");
        write_wav(&path, &pcm);

        let outcome = embed_enrollment_clip(&path, &models_dir()).expect("two-speaker clip");
        match outcome {
            EnrollmentOutcome::MultipleSpeakers { speaker_count } => {
                assert!(speaker_count >= 2, "got {speaker_count}");
            }
            other => panic!("expected MultipleSpeakers, got {other:?}"),
        }
    }
}
