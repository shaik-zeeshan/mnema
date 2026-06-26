//! Provider-neutral local speaker diarization and recognition contracts.
//!
//! The crate intentionally keeps app storage and Tauri download orchestration
//! out of the provider boundary. V1 providers receive local audio plus optional
//! local person embeddings and return anonymous speaker clusters, turns, and
//! cautious recognition suggestions.

mod core;
mod macos_audio_decode;
pub mod providers;
#[cfg(target_os = "windows")]
mod windows_audio_decode;

pub use core::{
    PersonEnrollment, PersonRecognitionRejection, RecognitionConfidence, SpeakerAnalysisError,
    SpeakerAnalysisMetadata, SpeakerAnalysisOutput, SpeakerAnalysisProvider,
    SpeakerAnalysisRequest, SpeakerAnalysisResult, SpeakerCluster, SpeakerRecognitionSuggestion,
    SpeakerTurn,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use thiserror::Error;

pub const MODEL_STORE_DIR_NAME: &str = "speaker-analysis-models";
pub const INSTALLED_MARKER_FILE_NAME: &str = ".installed.json";
pub const FAILED_MARKER_FILE_NAME: &str = ".failed.json";
pub const DOWNLOADING_MARKER_FILE_NAME: &str = ".download-in-progress";

/// The sole on-device diarization provider: speakrs (pure-Rust pyannote
/// community-1 pipeline with native CoreML acceleration). See ADR 0002.
pub const SPEAKRS_PROVIDER_ID: &str = "speakrs";
/// The single curated speakrs preset's `model_id`. The slug names intent: the
/// pyannote community-1 segmentation pipeline paired with WeSpeaker embeddings.
/// Slice 5 adds the matching manifest descriptor; until then the provider falls
/// back to a `models_dir/speakrs/<model_id>` install layout.
pub const SPEAKRS_DEFAULT_MODEL_ID: &str = "pyannote-community-1-wespeaker";
/// The embedding `model_id` stamped on speakrs clusters. This defines the
/// WeSpeaker Voiceprint Space (256-d voxceleb resnet34) so recognition only
/// matches enrolled people within the speakrs preset's space.
pub const SPEAKRS_EMBEDDING_MODEL_ID: &str = "wespeaker-voxceleb-resnet34";

/// Base URL for resolving every speakrs artifact from the HuggingFace repo.
/// Each file is fetched directly (raw) at `SPEAKRS_HF_RESOLVE_BASE + relative_path`
/// — unlike the sherpa presets there is no `.tar.bz2` to extract.
const SPEAKRS_HF_RESOLVE_BASE: &str =
    "https://huggingface.co/avencera/speakrs-models/resolve/main/";
/// HuggingFace repo that hosts the speakrs CoreML model bundle (used for the
/// descriptor `source_url` and the outer artifact `url`).
const SPEAKRS_HF_REPO_URL: &str = "https://huggingface.co/avencera/speakrs-models";

/// Verified `(relative_path, byte_size, sha256)` for every file speakrs's
/// `OwnedDiarizationPipeline::from_dir(.., ExecutionMode::CoreMl)` reads. This
/// is the authoritative `required_files(ExecutionMode::CoreMl)` set in
/// `speakrs/src/models.rs`, with HuggingFace-verified content sizes and SHA256
/// (LFS `oid` for LFS files, raw-download hashes for the small non-LFS files).
///
/// Both the `required_files` layout list AND the `MultiFile` artifact files are
/// derived from this single table (see `speakrs_artifact_files` /
/// `speakrs_required_files`) so they cannot drift. Files land flat under the
/// install dir at each `relative_path`, preserving `.mlmodelc/...` subpaths,
/// which is exactly what `from_dir` expects. `plda_phi.npy` is intentionally
/// absent — speakrs does not read it.
const SPEAKRS_COREML_FILES: &[(&str, u64, &str)] = &[
    ("plda_lda.npy", 131_200, "e20c9b012bebd1aabda5a38a127e63a43cf35debdc502715fc143e2fb6bc3c4b"),
    ("plda_tr.npy", 131_200, "e700b68cb319de3fafb5fa093eb9222c23c447084741f8d3a533640d425510ee"),
    ("plda_mu.npy", 1_152, "d286d48acf99bbc1ed1502fed0a3e361ae5626ce1870c8be9f7397c5e47886c6"),
    ("plda_psi.npy", 1_152, "d7128c9ed2f28a9781971805131129f077c04f948e2df12e52dcdb99f2b4e5f5"),
    ("plda_mean1.npy", 2_176, "e424c0c352182aa8e0f555dec1f3b30e29a20b9ed6b25d339f112af92e51e36f"),
    ("plda_mean2.npy", 640, "6f6fb708a2037197b5b84ffeaa8f140cb878088fbecd6ab042ad26a7691bd2cf"),
    ("wespeaker-voxceleb-resnet34.min_num_samples.txt", 4, "e4df891c484d7abb985dadf539fa1883a646dab6337af5cae4159c587b7050cc"),
    ("segmentation-3.0.onnx", 5_916_308, "038b971741ed623af9773ecafdefa4b7bc523520099c2a68f8568b24189e8ad9"),
    ("segmentation-3.0-b32.onnx", 6_178_495, "94deac93dacc90d3191511f87f6f4d8517e3b2f5e10a449a851bcbf0ba9cdc94"),
    ("wespeaker-voxceleb-resnet34.onnx", 26_894_815, "203a4c67112167580ab1fcb62f4568c633499fb283805890aebe1c48564fcc0f"),
    ("wespeaker-voxceleb-resnet34.onnx.data", 26_673_152, "dc105e7857156611381b95cc961b277d8e1e098e7af1c919a77c68c7257ce956"),
    ("wespeaker-fbank.onnx", 110_476, "67e4e9772eb344eafdbae0ca14b82129e67ade4bb980eb99e49f3d2a3c01a4a9"),
    ("wespeaker-fbank-b32.onnx", 110_476, "5cd98072f28a02358200213e2ff00551ecf170ed7f382a2131f5c8a745fc2263"),
    ("wespeaker-voxceleb-resnet34-tail.onnx", 26_733_283, "9e78a7566200700065c9d2ea677e7dbcd847420009a134aa5fa3ac656b09a45f"),
    ("wespeaker-voxceleb-resnet34-tail-b3.onnx", 26_774_950, "824510137f407f4fa50f342445ded0f59a42cfdbf7352acc8f38302887d9421e"),
    ("wespeaker-voxceleb-resnet34-tail-b32.onnx", 27_369_733, "f2aae56fd52fcb6afb0964e04ee6c81a4caec5d8a3bb1183f8fceb96855f1869"),
    ("segmentation-3.0.mlmodelc/model.mil", 37_089, "cb2ee400e54e7cd1ba1dfac3d60dc36c3d01f0152e5d4a559f7873eb7715d96f"),
    ("segmentation-3.0.mlmodelc/coremldata.bin", 439, "ac710fb9bcd0310d0c40fd82f2350ca5287b18596da33470bf5185be148aad81"),
    ("segmentation-3.0.mlmodelc/weights/weight.bin", 5_959_360, "c3189a64946c75bc24fcb98afe89ad78c52bdbadfdf65e857fb1b81e2cc9fbb2"),
    ("segmentation-3.0.mlmodelc/analytics/coremldata.bin", 243, "129d4f2316a01d29c2636cb72fea64880086685250918bd6a89ea2b770286e68"),
    ("segmentation-3.0-b32.mlmodelc/model.mil", 37_089, "cb2ee400e54e7cd1ba1dfac3d60dc36c3d01f0152e5d4a559f7873eb7715d96f"),
    ("segmentation-3.0-b32.mlmodelc/coremldata.bin", 439, "ac710fb9bcd0310d0c40fd82f2350ca5287b18596da33470bf5185be148aad81"),
    ("segmentation-3.0-b32.mlmodelc/weights/weight.bin", 5_959_360, "c3189a64946c75bc24fcb98afe89ad78c52bdbadfdf65e857fb1b81e2cc9fbb2"),
    ("segmentation-3.0-b32.mlmodelc/analytics/coremldata.bin", 243, "129d4f2316a01d29c2636cb72fea64880086685250918bd6a89ea2b770286e68"),
    ("segmentation-3.0-b64.mlmodelc/model.mil", 26_777, "d2d35b55977eb6631cfb456eb67e8a771e834ac972ae051196ec68e9cacd9bac"),
    ("segmentation-3.0-b64.mlmodelc/coremldata.bin", 150, "0460ffe22d22a0ae0dc86c0cc22d94c074f29bbd6be1434707a66b5e154796c8"),
    ("segmentation-3.0-b64.mlmodelc/weights/weight.bin", 6_024_960, "0b8ef91e4a97b435f0f3a4bb66ca647ff53820edc4cb747c242413045a4aaa56"),
    ("segmentation-3.0-b64.mlmodelc/analytics/coremldata.bin", 243, "7497fe39c383a49061eae9ebb3052a59360752ee96deeee46ae701f9afbd6e4d"),
    ("wespeaker-fbank.mlmodelc/model.mil", 7_997, "acee98c155b533afa6a7eb3d5e2158cb4005f85999be06f12276f94c7fab9d34"),
    ("wespeaker-fbank.mlmodelc/coremldata.bin", 168, "4241bd2543b6face59368f98e9cb7a049c46db8cc5ebd70a12814d3382324ede"),
    ("wespeaker-fbank.mlmodelc/weights/weight.bin", 1_778_432, "27396cb0afdd09164a9d6b2dbd10688bed15230948b3e0a6692ec490c03ae4d7"),
    ("wespeaker-fbank.mlmodelc/analytics/coremldata.bin", 243, "b4a6a692403029bf4d417d17ba0e43ab89482e9d862d4ed7d897b309d7455910"),
    ("wespeaker-fbank-b32.mlmodelc/model.mil", 7_997, "acee98c155b533afa6a7eb3d5e2158cb4005f85999be06f12276f94c7fab9d34"),
    ("wespeaker-fbank-b32.mlmodelc/coremldata.bin", 168, "4241bd2543b6face59368f98e9cb7a049c46db8cc5ebd70a12814d3382324ede"),
    ("wespeaker-fbank-b32.mlmodelc/weights/weight.bin", 1_778_432, "27396cb0afdd09164a9d6b2dbd10688bed15230948b3e0a6692ec490c03ae4d7"),
    ("wespeaker-fbank-b32.mlmodelc/analytics/coremldata.bin", 243, "b4a6a692403029bf4d417d17ba0e43ab89482e9d862d4ed7d897b309d7455910"),
    ("wespeaker-fbank-30s.mlmodelc/model.mil", 7_131, "34a76696a6318312ce0557b93e0d4defabfdbc540a682ae5dfbdc80d1f52f52d"),
    ("wespeaker-fbank-30s.mlmodelc/coremldata.bin", 153, "d52fb49c6521e14c9366e0194e1e91f4550d8ba39d31e03e72bfe014b1e826fe"),
    ("wespeaker-fbank-30s.mlmodelc/weights/weight.bin", 1_778_432, "27396cb0afdd09164a9d6b2dbd10688bed15230948b3e0a6692ec490c03ae4d7"),
    ("wespeaker-fbank-30s.mlmodelc/analytics/coremldata.bin", 243, "413a9d484295f712f6a496624abd9a663e72149d2fe43f8ad101dc1dc09981d0"),
    ("wespeaker-voxceleb-resnet34-tail.mlmodelc/model.mil", 60_706, "630709365400a429358025104c67a240ae9dc5c555dd9f9c72203be33ca51fc6"),
    ("wespeaker-voxceleb-resnet34-tail.mlmodelc/coremldata.bin", 218, "7e35919b7985082e3fe0fa8554679632383ef815f863f29a133a23a0bf17898a"),
    ("wespeaker-voxceleb-resnet34-tail.mlmodelc/weights/weight.bin", 26_525_120, "18f777be6e47d2d9d5792d475457add3b71a677814ac66cadc90e5410d14b252"),
    ("wespeaker-voxceleb-resnet34-tail.mlmodelc/analytics/coremldata.bin", 243, "07abd12d7cdb8af793b6d439b40bd9e5c1f44b9eb69cbb3d3d272f494a77a556"),
    ("wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/model.mil", 60_706, "630709365400a429358025104c67a240ae9dc5c555dd9f9c72203be33ca51fc6"),
    ("wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/coremldata.bin", 218, "7e35919b7985082e3fe0fa8554679632383ef815f863f29a133a23a0bf17898a"),
    ("wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/weights/weight.bin", 26_525_120, "18f777be6e47d2d9d5792d475457add3b71a677814ac66cadc90e5410d14b252"),
    ("wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/analytics/coremldata.bin", 243, "07abd12d7cdb8af793b6d439b40bd9e5c1f44b9eb69cbb3d3d272f494a77a556"),
    ("wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/model.mil", 60_706, "630709365400a429358025104c67a240ae9dc5c555dd9f9c72203be33ca51fc6"),
    ("wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/coremldata.bin", 218, "7e35919b7985082e3fe0fa8554679632383ef815f863f29a133a23a0bf17898a"),
    ("wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/weights/weight.bin", 26_525_120, "18f777be6e47d2d9d5792d475457add3b71a677814ac66cadc90e5410d14b252"),
    ("wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/analytics/coremldata.bin", 243, "07abd12d7cdb8af793b6d439b40bd9e5c1f44b9eb69cbb3d3d272f494a77a556"),
    ("wespeaker-multimask-tail-b32.mlmodelc/model.mil", 61_635, "15af971119deecf2729de0c32a9f9af17cf6769cf3a10d45ada114ddd7c54d3b"),
    ("wespeaker-multimask-tail-b32.mlmodelc/coremldata.bin", 201, "5e5467edd5d317c287cfa06474216990b8dfff1b7f6e725fb2e238a2264d8b16"),
    ("wespeaker-multimask-tail-b32.mlmodelc/weights/weight.bin", 26_525_120, "18f777be6e47d2d9d5792d475457add3b71a677814ac66cadc90e5410d14b252"),
    ("wespeaker-multimask-tail-b32.mlmodelc/analytics/coremldata.bin", 243, "77b60e64442b2140adebed82813bf619f76d92c9f419639093a50a27744ac14b"),
    ("wespeaker-chunk-emb-s12-w22.mlmodelc/model.mil", 66_382, "9661d1d90e55b67baddd7ebb5425bf8bc022f1f4501fc401416533edb22776a0"),
    ("wespeaker-chunk-emb-s12-w22.mlmodelc/coremldata.bin", 170, "b9d52f76d5063a5afa2617f9f116d871feb43465d04d794807782e1da7a39fb8"),
    ("wespeaker-chunk-emb-s12-w22.mlmodelc/weights/weight.bin", 27_212_096, "760c74421515a9407815321f45096a6b3c347fc79e8af69177a69700d7689acc"),
    ("wespeaker-chunk-emb-s12-w22.mlmodelc/analytics/coremldata.bin", 243, "aafa741516f6315fbf1aa462506532b29ade3735288894546ae6416fb3c43ce9"),
    ("wespeaker-chunk-emb-s12-w37.mlmodelc/model.mil", 66_449, "b9c1c547d5be9da2156372b45e7f458277212e83303efb70e4c34c728412552c"),
    ("wespeaker-chunk-emb-s12-w37.mlmodelc/coremldata.bin", 170, "0ad9d4580fa5349282e6d02f2c471c459045fe9c90f79242565c3e55ad6b80bb"),
    ("wespeaker-chunk-emb-s12-w37.mlmodelc/weights/weight.bin", 27_680_448, "cab4592f768a3490f709dd95e0405fa85b9c1d63c26d47cba73394841a7b5d07"),
    ("wespeaker-chunk-emb-s12-w37.mlmodelc/analytics/coremldata.bin", 243, "b2b90333ff02abd2deac8f5681908b07c8e4b288f2cbe92ecd2b7b220f8c184f"),
    ("wespeaker-chunk-emb-s12-w53.mlmodelc/model.mil", 66_449, "e4a31d06da147b7a2f4abc41190878ffd951a5c5815e19966aed4bfb73f4abc8"),
    ("wespeaker-chunk-emb-s12-w53.mlmodelc/coremldata.bin", 172, "8338f256e91562a68acaa7dd585f8975ee5ebf259471a0b8fbca0a02c3adb958"),
    ("wespeaker-chunk-emb-s12-w53.mlmodelc/weights/weight.bin", 28_179_968, "b8f5a65793ccd84603497c550af0b107f3227102a564831e20c81adaa831c414"),
    ("wespeaker-chunk-emb-s12-w53.mlmodelc/analytics/coremldata.bin", 243, "1f28514f1515e416f69935b2a391ccab7a1c9daab40899c447952be67045ab70"),
    ("wespeaker-chunk-emb-s12-w84.mlmodelc/model.mil", 66_470, "878c317466fdc5f1e5ff3d4aa646847531d8ab70564fc53ac4c30cd05e9e49a5"),
    ("wespeaker-chunk-emb-s12-w84.mlmodelc/coremldata.bin", 172, "d0347bea7234fb8a60519a05c6a150b5523fb86f0a50340f3dd30036e0ac1e7e"),
    ("wespeaker-chunk-emb-s12-w84.mlmodelc/weights/weight.bin", 29_147_776, "e2fe670276b392230ae0fad72c4fbce5b6bd6fd0002905e6f352b65afde0e7cf"),
    ("wespeaker-chunk-emb-s12-w84.mlmodelc/analytics/coremldata.bin", 243, "2baaac92ab37ced0a14006989e23748cbedfd58d3a3858da4ae0c77f3649a47a"),
    ("wespeaker-chunk-emb-s12-w116.mlmodelc/model.mil", 66_496, "8652083af9ed07a337dba438ae0c810dbd968e10e1f0b5b6f7c3b126e5f15f50"),
    ("wespeaker-chunk-emb-s12-w116.mlmodelc/coremldata.bin", 172, "dd2d8e8773a78800cd638b44748de4d7e1860f3b2727e49b199def03fd01f6d2"),
    ("wespeaker-chunk-emb-s12-w116.mlmodelc/weights/weight.bin", 30_146_816, "369991f2f7eff6dd7e89dd804fff843e4e1a576b40fda757f819db0907cf2688"),
    ("wespeaker-chunk-emb-s12-w116.mlmodelc/analytics/coremldata.bin", 243, "68234b4f7e8b542bd61d973184fc0c750af22085f8e5eb2d8be85514951da818"),
];

/// Which on-disk artifact subset of [`SPEAKRS_COREML_FILES`] a platform installs
/// for the single speakrs preset. The `model_id` is platform-stable (ADR 0004);
/// only the resolved files vary — macOS reads the full CoreML `.mlmodelc` set,
/// Windows reads the CPU ONNX + PLDA subset that speakrs's
/// `required_files(ExecutionMode::Cpu)` returns. Backend is orthogonal to
/// identity, so this never changes the Voiceprint Space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeakerArtifactSet {
    /// The full 76-file CoreML set (`.mlmodelc` + `.onnx` + `.npy`) macOS reads.
    CoreMl,
    /// The 10-file CPU subset (2 `.onnx` + 1 external `.onnx.data` + 6 PLDA
    /// `.npy` + the min-samples `.txt`) Windows reads via ONNX Runtime; named by
    /// [`SPEAKRS_CPU_FILE_NAMES`].
    Onnx,
}

/// The exact `relative_path` set speakrs's `required_files(ExecutionMode::Cpu)`
/// reads (`PLDA_FILES` + `ONNX_FILES` in speakrs-0.4.2 `src/models.rs`). These
/// select rows out of [`SPEAKRS_COREML_FILES`] — the sizes/SHA256 are NOT
/// duplicated here, the table stays the single source of truth. The CoreML
/// `.mlmodelc` bundles and the batched `-b32`/`-tail` ONNX variants are absent:
/// the CPU backend reads only `segmentation-3.0.onnx`, the WeSpeaker embedding
/// `.onnx` (+ its external `.onnx.data`), and the PLDA scoring tensors.
const SPEAKRS_CPU_FILE_NAMES: &[&str] = &[
    "plda_lda.npy",
    "plda_tr.npy",
    "plda_mu.npy",
    "plda_psi.npy",
    "plda_mean1.npy",
    "plda_mean2.npy",
    "wespeaker-voxceleb-resnet34.min_num_samples.txt",
    "segmentation-3.0.onnx",
    "wespeaker-voxceleb-resnet34.onnx",
    "wespeaker-voxceleb-resnet34.onnx.data",
];

/// The rows of [`SPEAKRS_COREML_FILES`] selected by `set`, in table order.
///
/// `CoreMl` returns the whole table; `Onnx` keeps only the rows named in
/// [`SPEAKRS_CPU_FILE_NAMES`], asserting all of them are present so the CPU
/// subset can never silently drift from the table.
fn speakrs_artifact_rows(
    set: SpeakerArtifactSet,
) -> Vec<&'static (&'static str, u64, &'static str)> {
    match set {
        SpeakerArtifactSet::CoreMl => SPEAKRS_COREML_FILES.iter().collect(),
        SpeakerArtifactSet::Onnx => {
            let rows: Vec<&'static (&'static str, u64, &'static str)> = SPEAKRS_COREML_FILES
                .iter()
                .filter(|(relative_path, _, _)| SPEAKRS_CPU_FILE_NAMES.contains(relative_path))
                .collect();
            assert_eq!(
                rows.len(),
                SPEAKRS_CPU_FILE_NAMES.len(),
                "speakrs CPU file set drifted from SPEAKRS_COREML_FILES: expected {} rows, found {}",
                SPEAKRS_CPU_FILE_NAMES.len(),
                rows.len()
            );
            rows
        }
    }
}

/// The flat install-layout `required_files` for the speakrs preset's `set`,
/// derived from `SPEAKRS_COREML_FILES` so they cannot drift from the artifact.
fn speakrs_required_files(set: SpeakerArtifactSet) -> Vec<String> {
    speakrs_artifact_rows(set)
        .into_iter()
        .map(|(relative_path, _, _)| (*relative_path).to_string())
        .collect()
}

/// The `MultiFile` artifact files for the speakrs preset's `set`. Each file is a
/// direct HuggingFace download at `SPEAKRS_HF_RESOLVE_BASE + relative_path`;
/// there is no archive to extract.
fn speakrs_artifact_files(set: SpeakerArtifactSet) -> Vec<ModelArtifactFile> {
    speakrs_artifact_rows(set)
        .into_iter()
        .map(|(relative_path, byte_size, sha256)| ModelArtifactFile {
            relative_path: (*relative_path).to_string(),
            url: format!("{SPEAKRS_HF_RESOLVE_BASE}{relative_path}"),
            byte_size: *byte_size,
            sha256: Some((*sha256).to_string()),
        })
        .collect()
}

/// The outer `ModelArtifact.byte_size` for the speakrs preset's `set`: the sum
/// of the selected files' content sizes, derived from the table rather than
/// hardcoded so it can never drift (CoreML = 419_482_724, ONNX = 59_751_799).
fn speakrs_artifact_byte_size(set: SpeakerArtifactSet) -> u64 {
    speakrs_artifact_rows(set)
        .into_iter()
        .map(|(_, byte_size, _)| *byte_size)
        .sum()
}

const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ModelStatusError {
    #[error("model descriptor for provider {provider} is missing an app-managed model id")]
    MissingAppManagedModelId { provider: String },
    #[error("unsafe path component in {field}: {value}")]
    UnsafePathComponent { field: &'static str, value: String },
    #[error("failed to read marker {path}: {source}")]
    ReadMarker {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse marker {path}: {source}")]
    ParseMarker {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write marker {path}: {source}")]
    WriteMarker {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub enum ModelInstallError {
    #[error("failed to remove model path {path}: {source}")]
    RemovePath {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to move downloaded model file to {path}: {source}")]
    MoveFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("downloaded model checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("failed to read downloaded model file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("installed model layout is incomplete; missing files: {missing_files:?}")]
    IncompleteInstalledLayout { missing_files: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelArtifactShape {
    MultiFile { files: Vec<ModelArtifactFile> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifactFile {
    pub relative_path: String,
    pub url: String,
    pub byte_size: u64,
    pub sha256: Option<String>,
}

// NOTE: `SpeakerAnalysisModelManifest`, `SpeakerAnalysisModelDescriptor`, and
// `SpeakerAnalysisModelStatus` keep `PartialEq` only (no `Eq`). The descriptor
// holds no floats today, but staying `PartialEq`-only keeps the door open for a
// future provider preset that carries f32 tuning without churning these derives
// again. No code compares these whole structs for `Eq`/hashing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisModelManifest {
    pub version: u32,
    pub models: Vec<SpeakerAnalysisModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisModelDescriptor {
    pub provider: String,
    pub model_id: Option<String>,
    pub display_name: String,
    pub description: String,
    pub license_label: Option<String>,
    pub source_url: Option<String>,
    pub management: ModelManagement,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelManagement {
    AppManaged {
        expected_layout: InstalledModelLayout,
        artifact: Option<ModelArtifact>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelLayout {
    pub marker_file_name: String,
    pub required_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifact {
    pub url: String,
    pub byte_size: u64,
    pub sha256: Option<String>,
    pub shape: ModelArtifactShape,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatusKind {
    NotInstalled,
    Installed,
    Incomplete,
    Failed,
    Downloading,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisModelStatus {
    pub descriptor: SpeakerAnalysisModelDescriptor,
    pub status: ModelStatusKind,
    pub install_path: PathBuf,
    pub missing_files: Vec<String>,
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelMarker {
    pub manifest_version: u32,
    pub provider: String,
    pub model_id: String,
}

/// Build the single speakrs preset descriptor for a given artifact `set`.
///
/// The sole on-device provider (ADR 0002): a pure-Rust pyannote community-1
/// diarization pipeline paired with WeSpeaker VoxCeleb ResNet34 embeddings. It
/// ships RAW files — each `relative_path` is fetched directly from HuggingFace
/// and placed flat under the install dir (preserving `.mlmodelc/...` subpaths)
/// where speakrs's `OwnedDiarizationPipeline::from_dir` reads it.
///
/// The `model_id`, provider, source URL, license, and marker file are
/// platform-stable; only the resolved files, derived `byte_size`, display name,
/// and description vary with the `set` (ADR 0004): macOS reads the full 76-file
/// CoreML set (`required_files(ExecutionMode::CoreMl)`) and is accelerated
/// natively with CoreML; Windows reads the 10-file CPU subset
/// (`required_files(ExecutionMode::Cpu)`) and runs on the CPU via ONNX Runtime.
fn speakrs_descriptor(set: SpeakerArtifactSet) -> SpeakerAnalysisModelDescriptor {
    // Backend name in the data-driven UI tracks the resolved artifact so it is
    // honest on each platform with zero frontend changes.
    let (display_name, description) = match set {
        SpeakerArtifactSet::CoreMl => (
            "pyannote community-1 + WeSpeaker (CoreML)",
            "On-device speaker diarization: a pure-Rust pyannote community-1 segmentation pipeline paired with WeSpeaker VoxCeleb ResNet34 embeddings, accelerated natively with Apple CoreML.",
        ),
        SpeakerArtifactSet::Onnx => (
            "pyannote community-1 + WeSpeaker (CPU)",
            "On-device speaker diarization: a pure-Rust pyannote community-1 segmentation pipeline paired with WeSpeaker VoxCeleb ResNet34 embeddings, accelerated on the CPU (ONNX Runtime).",
        ),
    };

    SpeakerAnalysisModelDescriptor {
        provider: SPEAKRS_PROVIDER_ID.to_string(),
        model_id: Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
        display_name: display_name.to_string(),
        description: description.to_string(),
        license_label: Some(
            "CC-BY-4.0 (WeSpeaker VoxCeleb ResNet34) + MIT (pyannote segmentation-3.0)".to_string(),
        ),
        source_url: Some(SPEAKRS_HF_REPO_URL.to_string()),
        management: ModelManagement::AppManaged {
            expected_layout: InstalledModelLayout {
                marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                required_files: speakrs_required_files(set),
            },
            artifact: Some(ModelArtifact {
                url: SPEAKRS_HF_REPO_URL.to_string(),
                byte_size: speakrs_artifact_byte_size(set),
                sha256: None,
                shape: ModelArtifactShape::MultiFile {
                    files: speakrs_artifact_files(set),
                },
            }),
        },
    }
}

pub fn builtin_model_manifest() -> SpeakerAnalysisModelManifest {
    // One platform-stable `model_id`; only the on-disk artifact subset varies by
    // build platform (ADR 0004). Windows installs the CPU ONNX subset, every
    // other target the full CoreML set.
    let set = if cfg!(target_os = "windows") {
        SpeakerArtifactSet::Onnx
    } else {
        SpeakerArtifactSet::CoreMl
    };
    SpeakerAnalysisModelManifest {
        version: MANIFEST_VERSION,
        models: vec![speakrs_descriptor(set)],
    }
}

pub fn find_model_descriptor<'a>(
    manifest: &'a SpeakerAnalysisModelManifest,
    provider: &str,
    model_id: Option<&str>,
) -> Option<&'a SpeakerAnalysisModelDescriptor> {
    manifest.models.iter().find(|descriptor| {
        descriptor.provider == provider && descriptor.model_id.as_deref() == model_id
    })
}

pub fn write_downloading_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<(), ModelStatusError> {
    let install_dir = models_dir
        .as_ref()
        .join(safe_path_component("provider", provider)?)
        .join(safe_path_component("modelId", model_id)?);
    fs::create_dir_all(&install_dir).map_err(|source| ModelStatusError::CreateDir {
        path: install_dir.clone(),
        source,
    })?;
    let marker = install_dir.join(DOWNLOADING_MARKER_FILE_NAME);
    fs::write(&marker, "").map_err(|source| ModelStatusError::WriteMarker {
        path: marker,
        source,
    })
}

pub fn write_failed_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
    message: impl AsRef<str>,
) -> Result<(), ModelStatusError> {
    let install_dir = models_dir
        .as_ref()
        .join(safe_path_component("provider", provider)?)
        .join(safe_path_component("modelId", model_id)?);
    fs::create_dir_all(&install_dir).map_err(|source| ModelStatusError::CreateDir {
        path: install_dir.clone(),
        source,
    })?;
    let marker = install_dir.join(FAILED_MARKER_FILE_NAME);
    fs::write(&marker, message.as_ref()).map_err(|source| ModelStatusError::WriteMarker {
        path: marker,
        source,
    })
}

pub fn write_installed_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<(), ModelStatusError> {
    let install_dir = models_dir
        .as_ref()
        .join(safe_path_component("provider", provider)?)
        .join(safe_path_component("modelId", model_id)?);
    fs::create_dir_all(&install_dir).map_err(|source| ModelStatusError::CreateDir {
        path: install_dir.clone(),
        source,
    })?;
    let marker = install_dir.join(INSTALLED_MARKER_FILE_NAME);
    let payload = InstalledModelMarker {
        manifest_version: MANIFEST_VERSION,
        provider: provider.to_string(),
        model_id: model_id.to_string(),
    };
    let json =
        serde_json::to_vec_pretty(&payload).map_err(|source| ModelStatusError::ParseMarker {
            path: marker.clone(),
            source,
        })?;
    fs::write(&marker, json).map_err(|source| ModelStatusError::WriteMarker {
        path: marker,
        source,
    })
}

pub fn remove_model_file_if_exists(path: impl AsRef<Path>) -> Result<(), ModelInstallError> {
    let path = path.as_ref();
    if path.exists() {
        fs::remove_file(path).map_err(|source| ModelInstallError::RemovePath {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

pub fn remove_model_dir_if_exists(path: impl AsRef<Path>) -> Result<(), ModelInstallError> {
    let path = path.as_ref();
    if path.exists() {
        fs::remove_dir_all(path).map_err(|source| ModelInstallError::RemovePath {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

pub fn validate_artifact_sha256(
    path: impl AsRef<Path>,
    expected: Option<&str>,
) -> Result<(), ModelInstallError> {
    let Some(expected) = expected.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let path = path.as_ref();
    let mut file = fs::File::open(path).map_err(|source| ModelInstallError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| ModelInstallError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err(ModelInstallError::ChecksumMismatch {
            path: path.to_path_buf(),
            expected: expected.to_string(),
            actual,
        });
    }
    Ok(())
}

pub fn install_model_file(
    destination: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<(), ModelInstallError> {
    let destination = destination.as_ref();
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|source| ModelInstallError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut file = fs::File::create(destination).map_err(|source| ModelInstallError::MoveFile {
        path: destination.to_path_buf(),
        source,
    })?;
    file.write_all(bytes)
        .map_err(|source| ModelInstallError::MoveFile {
            path: destination.to_path_buf(),
            source,
        })
}

pub fn speaker_analysis_models_dir(app_data_dir: impl AsRef<Path>) -> PathBuf {
    app_data_dir.as_ref().join(MODEL_STORE_DIR_NAME)
}

// ---------------------------------------------------------------------------
// GPU Acceleration Pack (Windows CUDA Execution Backend, #137 / ADR 0005)
// ---------------------------------------------------------------------------
//
// The CUDA backend is gated behind an opt-in, in-app NVIDIA-redist *pack* that is
// kept SEPARATE from the model store: the model is identity (the Voiceprint Space
// / Speaker Continuity), the pack is hardware (CUDA 12 + cuDNN 9 redistributables
// loaded via ORT's `preload_dylibs`). The base installer never ships it; Slice 4
// downloads it on demand. These helpers are platform-neutral and feature-free so
// the pure selection logic below can be unit-tested with no GPU and no speakrs
// build. Slices 3–4 build on them (pack-presence gating, the downloader).

/// App-data subdir holding the opt-in NVIDIA GPU Acceleration Pack (CUDA 12 +
/// cuDNN 9 redistributables). Sits alongside the speaker-analysis model store but
/// is a distinct provisioning unit (ADR 0005).
pub const GPU_ACCELERATION_PACK_DIR_NAME: &str = "gpu-acceleration-pack";

/// Marker written once the GPU Acceleration Pack is fully installed + verified
/// (reuses the `.installed.json` pattern of the model store). Its presence — not
/// merely the dir existing — is what [`gpu_pack_present`] treats as "provisioned".
pub const GPU_ACCELERATION_PACK_MARKER: &str = ".installed.json";

/// Resolve the GPU Acceleration Pack dir under the app data dir.
pub fn gpu_acceleration_pack_dir(app_data_dir: impl AsRef<Path>) -> PathBuf {
    app_data_dir.as_ref().join(GPU_ACCELERATION_PACK_DIR_NAME)
}

/// Whether the GPU Acceleration Pack is present (its install marker exists).
///
/// This is the ONLY gate the helper uses to decide whether CUDA may be
/// *attempted* (ADR 0005): a GPU machine with no pack is plain CPU with **no**
/// fallback noise — "not provisioned" is not a failure. NVML detection drives the
/// Settings *offer* (Slice 3/5), never the attempt.
pub fn gpu_pack_present(pack_dir: &Path) -> bool {
    pack_dir.join(GPU_ACCELERATION_PACK_MARKER).is_file()
}

// ---------------------------------------------------------------------------
// Execution Backend selection + provenance (pure, platform-neutral — Slice 2)
// ---------------------------------------------------------------------------
//
// Backend is orthogonal to identity (ADR 0004/0005): it only chooses a *hardware
// path*, never a `model_id`, Voiceprint Space, or Speaker Continuity key, and is
// observable only in result provenance (`executionMode`). These two functions are
// the GPU-free, feature-free heart of that decision so they unit-test under plain
// `cargo test -p speaker-analysis` (the heavy speakrs/ort/CUDA build stays opt-in);
// the runtime try/CPU-fallback that consumes them lives in providers/speakrs.rs.

/// Whether a Speaker Analysis Job should *attempt* the CUDA Execution Backend or
/// run plain CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionModeSelection {
    /// Try `ExecutionMode::Cuda`; on an init failure the caller falls back to CPU
    /// (still a successful job — ADR 0005). Chosen only when the pack is present
    /// AND the user has not forced CPU.
    AttemptCuda,
    /// Run plain CPU — no CUDA attempt and therefore no fallback diagnostics.
    Cpu,
}

/// Decide whether to attempt the CUDA Execution Backend for one job.
///
/// `AttemptCuda` iff `!force_cpu && pack_present`; otherwise `Cpu`.
///
/// `gpu_detected` is DELIBERATELY not part of the decision: it informs the
/// Settings *offer* (Slice 5), not the attempt. The try/CPU-fallback in
/// `run_speakrs_blocking` subsumes detection — attempting CUDA without the pack is
/// pointless (no provider DLLs to load), and attempting it *with* the pack but no
/// usable GPU just fails init and falls back to CPU, so a separate detect-gate
/// here could only add a way to wrongly skip a GPU that would actually have
/// worked. It stays a parameter so the signature documents the full input space
/// and the tests can prove it is inert.
pub fn select_execution_mode(
    force_cpu: bool,
    pack_present: bool,
    gpu_detected: bool,
) -> ExecutionModeSelection {
    // Intentionally inert; see the doc-comment above. Bound to `_` so a future
    // edit that tries to branch on it has to delete this line on purpose.
    let _ = gpu_detected;
    if !force_cpu && pack_present {
        ExecutionModeSelection::AttemptCuda
    } else {
        ExecutionModeSelection::Cpu
    }
}

/// Stamp the **Execution Backend** outcome into a result's provenance map.
///
/// `executionMode` always records the backend that ACTUALLY ran
/// (`"cpu"` | `"cuda"` | `"coreml"`). Only on a CUDA-init fallback — the caller
/// ran CPU because `from_dir(.., Cuda)` returned `Err` — do we also add the two
/// diagnostics `executionModeRequested = "cuda"` and `cudaFallbackReason = <error>`.
/// A plain CPU run (no pack or Force-CPU), and a successful CUDA or CoreML run,
/// get `executionMode` ONLY, with NO extra keys ("not provisioned" is not a
/// failure; ADR 0005). Pure + map-only so it unit-tests without a GPU.
///
/// Takes the concrete `BTreeMap` the metadata provenance uses (not a generic
/// `serde_json::Map`) so it can be called directly on `output.metadata.provenance`.
pub fn apply_execution_mode_provenance(
    provenance: &mut std::collections::BTreeMap<String, serde_json::Value>,
    actual_mode: &str,
    cuda_fallback: Option<&str>,
) {
    provenance.insert(
        "executionMode".to_string(),
        serde_json::Value::String(actual_mode.to_string()),
    );
    if let Some(reason) = cuda_fallback {
        provenance.insert(
            "executionModeRequested".to_string(),
            serde_json::Value::String("cuda".to_string()),
        );
        provenance.insert(
            "cudaFallbackReason".to_string(),
            serde_json::Value::String(reason.to_string()),
        );
    }
}

pub fn model_install_dir(
    models_dir: impl AsRef<Path>,
    descriptor: &SpeakerAnalysisModelDescriptor,
) -> Result<PathBuf, ModelStatusError> {
    let model_id = descriptor.model_id.as_deref().ok_or_else(|| {
        ModelStatusError::MissingAppManagedModelId {
            provider: descriptor.provider.clone(),
        }
    })?;
    Ok(models_dir
        .as_ref()
        .join(safe_path_component("provider", &descriptor.provider)?)
        .join(safe_path_component("modelId", model_id)?))
}

pub fn detect_model_status(
    models_dir: impl AsRef<Path>,
    descriptor: &SpeakerAnalysisModelDescriptor,
) -> Result<SpeakerAnalysisModelStatus, ModelStatusError> {
    let install_path = model_install_dir(models_dir, descriptor)?;
    let ModelManagement::AppManaged {
        expected_layout, ..
    } = &descriptor.management;
    let downloading_marker = install_path.join(DOWNLOADING_MARKER_FILE_NAME);
    if downloading_marker.exists() {
        return Ok(status(
            descriptor,
            ModelStatusKind::Downloading,
            install_path,
            vec![],
            None,
        ));
    }

    let missing_files = expected_layout
        .required_files
        .iter()
        .filter(|file| !install_path.join(file).is_file())
        .cloned()
        .collect::<Vec<_>>();
    let installed_marker = install_path.join(&expected_layout.marker_file_name);
    if installed_marker.is_file() && missing_files.is_empty() {
        return Ok(status(
            descriptor,
            ModelStatusKind::Installed,
            install_path,
            vec![],
            None,
        ));
    }
    if install_path.join(FAILED_MARKER_FILE_NAME).is_file() {
        let message =
            fs::read_to_string(install_path.join(FAILED_MARKER_FILE_NAME)).map_err(|source| {
                ModelStatusError::ReadMarker {
                    path: install_path.join(FAILED_MARKER_FILE_NAME),
                    source,
                }
            })?;
        return Ok(status(
            descriptor,
            ModelStatusKind::Failed,
            install_path,
            missing_files,
            Some(message),
        ));
    }
    let kind = if install_path.exists() && !missing_files.is_empty() {
        ModelStatusKind::Incomplete
    } else {
        ModelStatusKind::NotInstalled
    };
    Ok(status(descriptor, kind, install_path, missing_files, None))
}

fn status(
    descriptor: &SpeakerAnalysisModelDescriptor,
    status: ModelStatusKind,
    install_path: PathBuf,
    missing_files: Vec<String>,
    failure_message: Option<String>,
) -> SpeakerAnalysisModelStatus {
    SpeakerAnalysisModelStatus {
        descriptor: descriptor.clone(),
        status,
        install_path,
        missing_files,
        failure_message,
    }
}

pub(crate) fn safe_path_component(
    field: &'static str,
    value: &str,
) -> Result<String, ModelStatusError> {
    let path = Path::new(value);
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(value.to_string()),
        _ => Err(ModelStatusError::UnsafePathComponent {
            field,
            value: value.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_exposes_speakrs_as_default_model() {
        let manifest = builtin_model_manifest();
        assert_eq!(manifest.version, 1);
        // speakrs is the sole on-device provider and the default (first) entry.
        assert_eq!(manifest.models[0].provider, SPEAKRS_PROVIDER_ID);
        assert_eq!(
            manifest.models[0].model_id.as_deref(),
            Some(SPEAKRS_DEFAULT_MODEL_ID)
        );
    }

    fn descriptor_for(model_id: &str) -> SpeakerAnalysisModelDescriptor {
        builtin_model_manifest()
            .models
            .into_iter()
            .find(|model| model.model_id.as_deref() == Some(model_id))
            .unwrap_or_else(|| panic!("manifest is missing model id '{model_id}'"))
    }

    #[test]
    fn manifest_exposes_only_the_speakrs_preset() {
        let manifest = builtin_model_manifest();
        let ids: Vec<&str> = manifest
            .models
            .iter()
            .filter_map(|model| model.model_id.as_deref())
            .collect();
        // sherpa is fully removed: the manifest carries the single speakrs preset.
        assert_eq!(ids, vec![SPEAKRS_DEFAULT_MODEL_ID]);
        for model in &manifest.models {
            assert_eq!(model.provider, SPEAKRS_PROVIDER_ID);
        }
    }

    /// Shared invariants for either artifact set's descriptor: identity is
    /// platform-stable, required_files == artifact relative_paths (derived from
    /// one table), the outer byte_size is the summed per-file size, and every
    /// file resolves directly from the HF repo with a content SHA256.
    fn assert_descriptor_invariants(
        descriptor: &SpeakerAnalysisModelDescriptor,
        expected_byte_size: u64,
        expected_file_count: usize,
    ) {
        assert_eq!(descriptor.provider, SPEAKRS_PROVIDER_ID);
        assert_eq!(descriptor.model_id.as_deref(), Some(SPEAKRS_DEFAULT_MODEL_ID));
        let license = descriptor
            .license_label
            .as_deref()
            .expect("speakrs preset carries a license label");
        assert!(
            license.contains("CC-BY-4.0"),
            "speakrs license must mention CC-BY-4.0 attribution, got {license:?}"
        );
        assert_eq!(
            descriptor.source_url.as_deref(),
            Some("https://huggingface.co/avencera/speakrs-models")
        );

        let ModelManagement::AppManaged {
            expected_layout,
            artifact,
        } = &descriptor.management;
        let artifact = artifact.as_ref().expect("speakrs preset has an artifact");
        let ModelArtifactShape::MultiFile { files } = &artifact.shape;

        assert_eq!(expected_layout.required_files.len(), expected_file_count);
        assert_eq!(files.len(), expected_file_count);
        // required_files == the artifact file relative_paths (both derived from
        // `SPEAKRS_COREML_FILES`, so they cannot drift).
        let artifact_paths: Vec<&str> =
            files.iter().map(|file| file.relative_path.as_str()).collect();
        let required_paths: Vec<&str> = expected_layout
            .required_files
            .iter()
            .map(|path| path.as_str())
            .collect();
        assert_eq!(required_paths, artifact_paths);

        // Outer byte_size is the derived sum of the per-file sizes; every file
        // carries a direct HuggingFace URL + content SHA256 (no archive).
        let summed: u64 = files.iter().map(|file| file.byte_size).sum();
        assert_eq!(artifact.byte_size, summed);
        assert_eq!(artifact.byte_size, expected_byte_size);
        for file in files {
            assert!(
                file.url.starts_with(
                    "https://huggingface.co/avencera/speakrs-models/resolve/main/"
                ),
                "speakrs file URL must resolve from the HF repo, got {:?}",
                file.url
            );
            assert!(file.url.ends_with(&file.relative_path));
            assert!(!file.url.ends_with(".tar.bz2"), "speakrs ships raw files");
            assert!(file.sha256.is_some(), "every speakrs file is checksummed");
        }
    }

    #[test]
    fn coreml_artifact_set_is_the_full_mlmodelc_set() {
        let descriptor = speakrs_descriptor(SpeakerArtifactSet::CoreMl);
        // The full 76-file CoreMl required_files set speakrs's from_dir reads.
        assert_descriptor_invariants(&descriptor, 419_482_724, 76);
        assert!(descriptor.display_name.contains("CoreML"));

        let required = speakrs_required_files(SpeakerArtifactSet::CoreMl);
        assert_eq!(required.len(), 76);
        // PLDA + segmentation + WeSpeaker embedding land flat where from_dir
        // looks; sanity-check a flat root file and a nested .mlmodelc subpath.
        assert!(required.iter().any(|path| path == "segmentation-3.0.onnx"));
        assert!(required.iter().any(|path| path == "wespeaker-voxceleb-resnet34.onnx"));
        assert!(required.iter().any(|path| path == "plda_lda.npy"));
        assert!(required
            .iter()
            .any(|path| path == "segmentation-3.0.mlmodelc/weights/weight.bin"));
        assert!(required.iter().any(|path| path.contains(".mlmodelc")));
        // plda_phi.npy is intentionally absent — speakrs does not read it.
        assert!(!required.iter().any(|path| path.contains("plda_phi")));
        // The derived total matches the historically verified CoreMl size.
        assert_eq!(speakrs_artifact_byte_size(SpeakerArtifactSet::CoreMl), 419_482_724);
    }

    #[test]
    fn onnx_artifact_set_is_the_ten_cpu_files() {
        let descriptor = speakrs_descriptor(SpeakerArtifactSet::Onnx);
        // The 10-file CPU subset speakrs's required_files(ExecutionMode::Cpu)
        // reads, with the derived ONNX total.
        assert_descriptor_invariants(&descriptor, 59_751_799, 10);
        // The Windows preset runs on the CPU (ONNX Runtime), not CoreML.
        assert!(descriptor.display_name.contains("CPU"));
        assert!(!descriptor.display_name.contains("CoreML"));
        assert!(descriptor.description.contains("ONNX Runtime"));

        let required = speakrs_required_files(SpeakerArtifactSet::Onnx);
        // Exactly the 10 CPU paths, in table order.
        assert_eq!(required, SPEAKRS_CPU_FILE_NAMES.to_vec());
        assert_eq!(required.len(), 10);
        // No CoreML bundles, and none of the batched/CoreML-only ONNX variants.
        assert!(!required.iter().any(|path| path.contains(".mlmodelc")));
        assert!(!required.iter().any(|path| path.contains("-b32.onnx")));
        assert!(!required.iter().any(|path| path.contains("-tail")));
        // The 2 ONNX graphs (+ external weights) + the PLDA tensors + min-samples
        // txt are all present.
        assert!(required.contains(&"segmentation-3.0.onnx".to_string()));
        assert!(required.contains(&"wespeaker-voxceleb-resnet34.onnx".to_string()));
        assert!(required.contains(&"wespeaker-voxceleb-resnet34.onnx.data".to_string()));
        assert!(required.contains(&"plda_lda.npy".to_string()));
        assert!(required.contains(&"wespeaker-voxceleb-resnet34.min_num_samples.txt".to_string()));
        assert_eq!(speakrs_artifact_byte_size(SpeakerArtifactSet::Onnx), 59_751_799);
    }

    #[test]
    fn both_artifact_sets_share_one_model_id() {
        // Backend/artifact is orthogonal to identity (ADR 0004): the
        // platform-stable model_id, provider, source, and license are identical
        // across the CoreML and CPU/ONNX sets, so one Voiceprint Space stays
        // comparable across Execution Backends; only the resolved artifact differs.
        let coreml = speakrs_descriptor(SpeakerArtifactSet::CoreMl);
        let onnx = speakrs_descriptor(SpeakerArtifactSet::Onnx);
        assert_eq!(coreml.model_id, onnx.model_id);
        assert_eq!(coreml.model_id.as_deref(), Some(SPEAKRS_DEFAULT_MODEL_ID));
        assert_eq!(coreml.provider, onnx.provider);
        assert_eq!(coreml.source_url, onnx.source_url);
        assert_eq!(coreml.license_label, onnx.license_label);
        assert_ne!(
            speakrs_required_files(SpeakerArtifactSet::CoreMl),
            speakrs_required_files(SpeakerArtifactSet::Onnx)
        );
        assert_ne!(
            speakrs_artifact_byte_size(SpeakerArtifactSet::CoreMl),
            speakrs_artifact_byte_size(SpeakerArtifactSet::Onnx)
        );
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let manifest = builtin_model_manifest();
        let encoded = serde_json::to_string(&manifest).expect("manifest encodes");
        let decoded: SpeakerAnalysisModelManifest =
            serde_json::from_str(&encoded).expect("manifest decodes");
        assert_eq!(decoded, manifest);
        // The speakrs preset's multi-file artifact survives the round trip.
        assert!(encoded.contains("\"multi_file\""));
        assert!(encoded.contains("\"requiredFiles\""));
    }

    #[test]
    fn descriptor_round_trips() {
        let descriptor = descriptor_for(SPEAKRS_DEFAULT_MODEL_ID);
        let encoded = serde_json::to_string(&descriptor).expect("encodes");
        let decoded: SpeakerAnalysisModelDescriptor =
            serde_json::from_str(&encoded).expect("decodes");
        assert_eq!(decoded, descriptor);
    }

    #[test]
    fn select_execution_mode_matrix_gpu_detected_is_inert() {
        use ExecutionModeSelection::*;
        // The full {force_cpu × pack_present × gpu_detected} matrix (8 cases).
        // AttemptCuda iff (!force_cpu && pack_present); everything else is Cpu, and
        // `gpu_detected` NEVER changes the result (it only drives the Settings
        // offer, not the attempt — ADR 0005).
        for &gpu_detected in &[false, true] {
            assert_eq!(select_execution_mode(false, true, gpu_detected), AttemptCuda);
            assert_eq!(select_execution_mode(false, false, gpu_detected), Cpu);
            assert_eq!(select_execution_mode(true, true, gpu_detected), Cpu);
            assert_eq!(select_execution_mode(true, false, gpu_detected), Cpu);
        }
        // Pairwise proof that toggling gpu_detected ALONE is inert at every
        // (force_cpu, pack_present) corner — the invariant the plan calls out.
        for &force_cpu in &[false, true] {
            for &pack_present in &[false, true] {
                assert_eq!(
                    select_execution_mode(force_cpu, pack_present, false),
                    select_execution_mode(force_cpu, pack_present, true),
                    "gpu_detected changed the selection at force_cpu={force_cpu}, pack_present={pack_present}"
                );
            }
        }
    }

    #[test]
    fn execution_mode_provenance_plain_cpu_has_no_diagnostics() {
        // (a) plain cpu (no pack / Force-CPU): only executionMode, no fallback keys.
        let mut provenance = std::collections::BTreeMap::new();
        apply_execution_mode_provenance(&mut provenance, "cpu", None);
        assert_eq!(provenance.get("executionMode"), Some(&serde_json::json!("cpu")));
        assert!(!provenance.contains_key("executionModeRequested"));
        assert!(!provenance.contains_key("cudaFallbackReason"));
    }

    #[test]
    fn execution_mode_provenance_records_cuda_init_fallback() {
        // (b) init-fallback: all three keys; executionMode is what actually ran
        // (cpu), requested is cuda, and the reason is carried verbatim.
        let mut provenance = std::collections::BTreeMap::new();
        let reason = "CUDA EP init failed: cudnn64_9.dll not found";
        apply_execution_mode_provenance(&mut provenance, "cpu", Some(reason));
        assert_eq!(provenance.get("executionMode"), Some(&serde_json::json!("cpu")));
        assert_eq!(
            provenance.get("executionModeRequested"),
            Some(&serde_json::json!("cuda"))
        );
        assert_eq!(
            provenance.get("cudaFallbackReason"),
            Some(&serde_json::json!(reason))
        );
    }

    #[test]
    fn execution_mode_provenance_cuda_success_has_no_diagnostics() {
        // (c) cuda success: executionMode=cuda, no requested/reason keys.
        let mut provenance = std::collections::BTreeMap::new();
        apply_execution_mode_provenance(&mut provenance, "cuda", None);
        assert_eq!(provenance.get("executionMode"), Some(&serde_json::json!("cuda")));
        assert!(!provenance.contains_key("executionModeRequested"));
        assert!(!provenance.contains_key("cudaFallbackReason"));
    }

    #[test]
    fn execution_mode_provenance_coreml() {
        // (d) macOS CoreML: executionMode=coreml, no requested/reason keys (the
        // macOS path is byte-identical, no new keys — ADR 0005).
        let mut provenance = std::collections::BTreeMap::new();
        apply_execution_mode_provenance(&mut provenance, "coreml", None);
        assert_eq!(
            provenance.get("executionMode"),
            Some(&serde_json::json!("coreml"))
        );
        assert!(!provenance.contains_key("executionModeRequested"));
        assert!(!provenance.contains_key("cudaFallbackReason"));
    }

    #[test]
    fn gpu_pack_present_requires_install_marker() {
        // Pack-presence is the install MARKER, not just the dir: an empty/absent
        // dir is "not provisioned" (plain CPU, no fallback noise — ADR 0005).
        let temp = tempfile::tempdir().expect("tempdir");
        let pack_dir = gpu_acceleration_pack_dir(temp.path());
        assert_eq!(pack_dir.file_name().unwrap(), GPU_ACCELERATION_PACK_DIR_NAME);
        assert!(!gpu_pack_present(&pack_dir));
        fs::create_dir_all(&pack_dir).expect("create pack dir");
        assert!(!gpu_pack_present(&pack_dir), "dir alone is not provisioned");
        fs::write(pack_dir.join(GPU_ACCELERATION_PACK_MARKER), "{}").expect("write marker");
        assert!(gpu_pack_present(&pack_dir), "install marker means provisioned");
    }

    #[test]
    fn request_and_output_contract_round_trips_json() {
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.wav",
            SPEAKRS_PROVIDER_ID,
            Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
            "session-a",
            42,
        );
        let mut output =
            SpeakerAnalysisOutput::new(SpeakerAnalysisMetadata::from_request(&request));
        output.clusters.push(SpeakerCluster {
            provider_cluster_id: "spk0".to_string(),
            stable_label: "Unknown Speaker 1".to_string(),
            embedding: vec![1, 2, 3],
            embedding_model_id: SPEAKRS_DEFAULT_MODEL_ID.to_string(),
            suggestion: None,
        });
        output.turns.push(SpeakerTurn {
            provider_cluster_id: "spk0".to_string(),
            start_ms: 0,
            end_ms: 1000,
            transcript_text: Some("hello".to_string()),
            overlaps: false,
        });

        let encoded = output.structured_payload_json().expect("payload encodes");
        let decoded: SpeakerAnalysisOutput =
            serde_json::from_str(&encoded).expect("payload decodes");
        assert_eq!(decoded.turns[0].provider_cluster_id, "spk0");
        assert_eq!(decoded.metadata.audio_segment_id, 42);
    }
}
