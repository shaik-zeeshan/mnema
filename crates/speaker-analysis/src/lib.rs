//! Provider-neutral local speaker diarization and recognition contracts.
//!
//! The crate intentionally keeps app storage and Tauri download orchestration
//! out of the provider boundary. V1 providers receive local audio plus optional
//! local person embeddings and return anonymous speaker clusters, turns, and
//! cautious recognition suggestions.

mod core;
mod macos_audio_decode;
pub mod providers;

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
pub const SHERPA_ONNX_PROVIDER_ID: &str = "sherpa_onnx";
pub const DEFAULT_SHERPA_ONNX_MODEL_ID: &str = "pyannote-3.0-nemo-titanet-small";
/// Multilingual (English + Mandarin) preset: pyannote segmentation 3.0 plus
/// 3D-Speaker CAM++ zh/en embeddings.
pub const MULTILINGUAL_SHERPA_ONNX_MODEL_ID: &str = "pyannote-3.0-campplus-zh-en";
/// High-accuracy preset: reverb-diarization-v1 segmentation (robust in
/// noise/reverb) plus NeMo Titanet Large English embeddings.
pub const HIGH_ACCURACY_SHERPA_ONNX_MODEL_ID: &str = "reverb-v1-nemo-titanet-large";

/// Second on-device diarization provider: speakrs (pure-Rust pyannote
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

/// The flat install-layout `required_files` for the speakrs CoreML preset,
/// derived from `SPEAKRS_COREML_FILES` so they cannot drift from the artifact.
fn speakrs_required_files() -> Vec<String> {
    SPEAKRS_COREML_FILES
        .iter()
        .map(|(relative_path, _, _)| (*relative_path).to_string())
        .collect()
}

/// The `MultiFile` artifact files for the speakrs CoreML preset. Each file is a
/// direct HuggingFace download at `SPEAKRS_HF_RESOLVE_BASE + relative_path`;
/// there is no archive to extract.
fn speakrs_artifact_files() -> Vec<ModelArtifactFile> {
    SPEAKRS_COREML_FILES
        .iter()
        .map(|(relative_path, byte_size, sha256)| ModelArtifactFile {
            relative_path: (*relative_path).to_string(),
            url: format!("{SPEAKRS_HF_RESOLVE_BASE}{relative_path}"),
            byte_size: *byte_size,
            sha256: Some((*sha256).to_string()),
        })
        .collect()
}

const MANIFEST_VERSION: u32 = 1;

/// Default per-model clustering similarity threshold for the original Balanced
/// preset. Historically the global `DEFAULT_CLUSTERING_THRESHOLD` const in the
/// sherpa provider; now a per-descriptor field so new presets can tune it.
pub const DEFAULT_CLUSTERING_THRESHOLD: f32 = 0.65;
/// Conservative cross-chunk cluster similarity threshold used by presets whose
/// embedding model has not been empirically calibrated. Historically the global
/// `CROSS_CHUNK_CLUSTER_SIMILARITY_THRESHOLD` const (0.60); now only the two
/// newer presets (campplus / titanet-large) inherit it, since their embedding
/// similarity scales differ from titanet-small and have not been measured.
pub const DEFAULT_CROSS_CHUNK_THRESHOLD: f32 = 0.60;
/// Cross-chunk cluster similarity threshold for the Balanced (titanet-small)
/// preset. A brief experiment lowered this from the historical 0.60 to 0.50,
/// calibrated on a single 3-speaker clip. The DER benchmark
/// (`scripts/diarization_bench/`, VoxConverse 10-clip subset) showed that
/// over-fit: 0.50 wrongly merges distinct speakers on harder multi-speaker audio
/// (confusion-dominated DER), and 0.60 is the empirical optimum of a clean
/// U-curve (DER 10.89% -> 9.71%). Restored to 0.60, matching the other presets.
pub const BALANCED_CROSS_CHUNK_THRESHOLD: f32 = 0.60;
/// Default minimum speaker-turn duration (milliseconds) below which a turn is
/// skipped when forming per-chunk cluster embeddings (accuracy improvement #2).
pub const DEFAULT_MIN_TURN_MS: u64 = 500;

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
// `SpeakerAnalysisModelStatus` intentionally drop `Eq` (keeping `PartialEq`)
// because the descriptor now transitively holds f32 clustering thresholds via
// `SherpaModelParams`, and f32 cannot implement `Eq`. No code compares these
// whole structs for `Eq`/hashing; `ModelStatusKind` and the artifact/layout
// structs keep `Eq` since they hold no floats.
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
    /// Sherpa-onnx provider parameters (segmentation/embedding relative paths
    /// plus per-model clustering/cross-chunk thresholds and minimum turn
    /// duration). Optional so the shared descriptor stays forward-compatible
    /// with a future non-sherpa provider (e.g. FluidAudio/ANE) that carries no
    /// sherpa params.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sherpa_params: Option<SherpaModelParams>,
}

/// Sherpa-onnx-specific tuning carried per **Speaker Model Preset** descriptor.
///
/// Holds the segmentation/embedding model relative paths (previously module
/// consts) and the per-model accuracy thresholds settled in ADR 0001 so new
/// presets are not stuck with values tuned for the original combo. Does not
/// derive `Eq` because `clustering_threshold`/`cross_chunk_threshold` are f32.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SherpaModelParams {
    /// Relative path (under the model install dir) to the segmentation
    /// `model.onnx`.
    pub segmentation_relative_path: String,
    /// Relative path (under the model install dir) to the embedding `.onnx`.
    pub embedding_relative_path: String,
    /// Per-model fast-clustering similarity threshold (accuracy #3); a
    /// request-option override still wins at runtime.
    pub clustering_threshold: f32,
    /// Per-model cross-chunk cluster similarity threshold used when stitching
    /// safe-chunked diarization clusters together.
    pub cross_chunk_threshold: f32,
    /// Minimum speaker-turn duration in milliseconds (accuracy #2); turns
    /// shorter than this are skipped when forming per-chunk embeddings.
    pub min_turn_ms: u64,
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

pub fn builtin_model_manifest() -> SpeakerAnalysisModelManifest {
    SpeakerAnalysisModelManifest {
        version: MANIFEST_VERSION,
        models: vec![
            // Balanced (default). Paths/thresholds preserved exactly from the
            // historical module consts so this preset behaves identically.
            SpeakerAnalysisModelDescriptor {
                provider: SHERPA_ONNX_PROVIDER_ID.to_string(),
                model_id: Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
                display_name: "Balanced (pyannote 3.0 + NeMo Titanet Small)".to_string(),
                description: "Balanced English-first local speaker diarization using pyannote segmentation 3.0 plus NeMo Titanet Small speaker embeddings.".to_string(),
                license_label: None,
                source_url: Some("https://github.com/k2-fsa/sherpa-onnx".to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: vec![
                            "pyannote-segmentation-3.0/model.onnx".to_string(),
                            "nemo_en_titanet_small.onnx".to_string(),
                        ],
                    },
                    artifact: Some(ModelArtifact {
                        url: "https://github.com/k2-fsa/sherpa-onnx".to_string(),
                        byte_size: 47_215_727,
                        sha256: None,
                        shape: ModelArtifactShape::MultiFile {
                            files: vec![
                                ModelArtifactFile {
                                    relative_path: "pyannote-segmentation-3.0/model.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2".to_string(),
                                    byte_size: 6_958_444,
                                    sha256: Some("24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488".to_string()),
                                },
                                ModelArtifactFile {
                                    relative_path: "nemo_en_titanet_small.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/nemo_en_titanet_small.onnx".to_string(),
                                    byte_size: 40_257_283,
                                    sha256: Some("ad4a1802485d8b34c722d2a9d04249662f2ece5d28a7a039063ca22f515a789e".to_string()),
                                },
                            ],
                        },
                    }),
                },
                sherpa_params: Some(SherpaModelParams {
                    segmentation_relative_path: "pyannote-segmentation-3.0/model.onnx".to_string(),
                    embedding_relative_path: "nemo_en_titanet_small.onnx".to_string(),
                    clustering_threshold: DEFAULT_CLUSTERING_THRESHOLD,
                    cross_chunk_threshold: BALANCED_CROSS_CHUNK_THRESHOLD,
                    min_turn_ms: DEFAULT_MIN_TURN_MS,
                }),
            },
            // Multilingual (English + Mandarin). pyannote segmentation 3.0
            // (shared with Balanced) plus 3D-Speaker CAM++ zh/en embeddings.
            // ~33MB total download.
            SpeakerAnalysisModelDescriptor {
                provider: SHERPA_ONNX_PROVIDER_ID.to_string(),
                model_id: Some(MULTILINGUAL_SHERPA_ONNX_MODEL_ID.to_string()),
                display_name: "Multilingual (pyannote 3.0 + CAM++ zh/en)".to_string(),
                description: "Multilingual (English + Mandarin) local speaker diarization using pyannote segmentation 3.0 plus 3D-Speaker CAM++ zh/en speaker embeddings.".to_string(),
                license_label: None,
                source_url: Some("https://github.com/k2-fsa/sherpa-onnx".to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: vec![
                            "pyannote-segmentation-3.0/model.onnx".to_string(),
                            "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".to_string(),
                        ],
                    },
                    artifact: Some(ModelArtifact {
                        url: "https://github.com/k2-fsa/sherpa-onnx".to_string(),
                        byte_size: 35_239_608,
                        sha256: None,
                        shape: ModelArtifactShape::MultiFile {
                            files: vec![
                                ModelArtifactFile {
                                    relative_path: "pyannote-segmentation-3.0/model.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2".to_string(),
                                    byte_size: 6_958_444,
                                    sha256: Some("24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488".to_string()),
                                },
                                ModelArtifactFile {
                                    relative_path: "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".to_string(),
                                    byte_size: 28_281_164,
                                    sha256: Some("aa3cfc16963a10586a9393f5035d6d6b57e98d358b347f80c2a30bf4f00ceba2".to_string()),
                                },
                            ],
                        },
                    }),
                },
                sherpa_params: Some(SherpaModelParams {
                    segmentation_relative_path: "pyannote-segmentation-3.0/model.onnx".to_string(),
                    embedding_relative_path: "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".to_string(),
                    clustering_threshold: DEFAULT_CLUSTERING_THRESHOLD,
                    cross_chunk_threshold: DEFAULT_CROSS_CHUNK_THRESHOLD,
                    min_turn_ms: DEFAULT_MIN_TURN_MS,
                }),
            },
            // High-accuracy (English). reverb-diarization-v1 segmentation
            // (robust in noise/reverb) plus NeMo Titanet Large embeddings.
            // ~106MB total download. The segmentation tarball extracts its
            // fp32 `model.onnx` into `reverb-diarization-v1/model.onnx`.
            SpeakerAnalysisModelDescriptor {
                provider: SHERPA_ONNX_PROVIDER_ID.to_string(),
                model_id: Some(HIGH_ACCURACY_SHERPA_ONNX_MODEL_ID.to_string()),
                display_name: "High-accuracy (reverb v1 + NeMo Titanet Large)".to_string(),
                description: "High-accuracy English local speaker diarization using reverb-diarization-v1 segmentation (robust in noise/reverb) plus NeMo Titanet Large speaker embeddings.".to_string(),
                license_label: None,
                source_url: Some("https://github.com/k2-fsa/sherpa-onnx".to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: vec![
                            "reverb-diarization-v1/model.onnx".to_string(),
                            "nemo_en_titanet_large.onnx".to_string(),
                        ],
                    },
                    artifact: Some(ModelArtifact {
                        url: "https://github.com/k2-fsa/sherpa-onnx".to_string(),
                        byte_size: 112_324_078,
                        sha256: None,
                        shape: ModelArtifactShape::MultiFile {
                            files: vec![
                                ModelArtifactFile {
                                    relative_path: "reverb-diarization-v1/model.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-reverb-diarization-v1.tar.bz2".to_string(),
                                    byte_size: 10_918_585,
                                    sha256: Some("615761e980be1688da0ef81618c056134d63aa55ea0a5f1494c47393b9398eab".to_string()),
                                },
                                ModelArtifactFile {
                                    relative_path: "nemo_en_titanet_large.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/nemo_en_titanet_large.onnx".to_string(),
                                    byte_size: 101_405_493,
                                    sha256: Some("d51abcf31717ef28162f26acb9d44dd4127c3d44c9b8624f699f3425daca8e77".to_string()),
                                },
                            ],
                        },
                    }),
                },
                sherpa_params: Some(SherpaModelParams {
                    segmentation_relative_path: "reverb-diarization-v1/model.onnx".to_string(),
                    embedding_relative_path: "nemo_en_titanet_large.onnx".to_string(),
                    clustering_threshold: DEFAULT_CLUSTERING_THRESHOLD,
                    cross_chunk_threshold: DEFAULT_CROSS_CHUNK_THRESHOLD,
                    min_turn_ms: DEFAULT_MIN_TURN_MS,
                }),
            },
            // speakrs (pyannote community-1 + WeSpeaker, CoreML). The second
            // on-device provider (ADR 0002): a pure-Rust pyannote community-1
            // diarization pipeline with native CoreML acceleration. Unlike the
            // sherpa presets this ships RAW files — each `relative_path` is
            // fetched directly from HuggingFace and placed flat under the install
            // dir (preserving `.mlmodelc/...` subpaths) where speakrs's
            // `OwnedDiarizationPipeline::from_dir(.., ExecutionMode::CoreMl)`
            // reads it. `sherpa_params` is absent — speakrs is not sherpa-onnx.
            // The 76-file set + sizes/SHA256 mirror
            // `required_files(ExecutionMode::CoreMl)` in `speakrs/src/models.rs`.
            SpeakerAnalysisModelDescriptor {
                provider: SPEAKRS_PROVIDER_ID.to_string(),
                model_id: Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
                display_name: "speakrs (pyannote community-1 + WeSpeaker, CoreML)".to_string(),
                description: "On-device speaker diarization via speakrs: a pure-Rust pyannote community-1 segmentation pipeline paired with WeSpeaker VoxCeleb ResNet34 embeddings, accelerated natively with Apple CoreML.".to_string(),
                license_label: Some(
                    "CC-BY-4.0 (WeSpeaker VoxCeleb ResNet34) + MIT (pyannote segmentation-3.0)"
                        .to_string(),
                ),
                source_url: Some(SPEAKRS_HF_REPO_URL.to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: speakrs_required_files(),
                    },
                    artifact: Some(ModelArtifact {
                        url: SPEAKRS_HF_REPO_URL.to_string(),
                        byte_size: 419_482_724,
                        sha256: None,
                        shape: ModelArtifactShape::MultiFile {
                            files: speakrs_artifact_files(),
                        },
                    }),
                },
                sherpa_params: None,
            },
        ],
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
    fn manifest_exposes_app_managed_sherpa_model() {
        let manifest = builtin_model_manifest();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.models[0].provider, SHERPA_ONNX_PROVIDER_ID);
        assert_eq!(
            manifest.models[0].model_id.as_deref(),
            Some(DEFAULT_SHERPA_ONNX_MODEL_ID)
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
    fn manifest_exposes_curated_presets_across_providers() {
        let manifest = builtin_model_manifest();
        let ids: Vec<&str> = manifest
            .models
            .iter()
            .filter_map(|model| model.model_id.as_deref())
            .collect();
        assert_eq!(
            ids,
            vec![
                DEFAULT_SHERPA_ONNX_MODEL_ID,
                MULTILINGUAL_SHERPA_ONNX_MODEL_ID,
                HIGH_ACCURACY_SHERPA_ONNX_MODEL_ID,
                SPEAKRS_DEFAULT_MODEL_ID,
            ]
        );
        // The three sherpa presets carry sherpa_params; the speakrs preset is a
        // different provider and does not.
        for model in &manifest.models {
            if model.provider == SHERPA_ONNX_PROVIDER_ID {
                assert!(
                    model.sherpa_params.is_some(),
                    "sherpa preset {:?} must carry sherpa_params",
                    model.model_id
                );
            } else {
                assert_eq!(model.provider, SPEAKRS_PROVIDER_ID);
                assert!(
                    model.sherpa_params.is_none(),
                    "speakrs preset {:?} must not carry sherpa_params",
                    model.model_id
                );
            }
        }
    }

    #[test]
    fn manifest_exposes_speakrs_coreml_preset() {
        let descriptor = descriptor_for(SPEAKRS_DEFAULT_MODEL_ID);
        assert_eq!(descriptor.provider, SPEAKRS_PROVIDER_ID);
        assert_eq!(descriptor.model_id.as_deref(), Some(SPEAKRS_DEFAULT_MODEL_ID));
        assert!(descriptor.sherpa_params.is_none());
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

        // Exactly the 76 CoreMl required_files speakrs's from_dir reads.
        assert_eq!(expected_layout.required_files.len(), 76);
        assert_eq!(files.len(), 76);
        // required_files == the artifact file relative_paths (derived from the
        // same table, so they cannot drift).
        let artifact_paths: Vec<&str> =
            files.iter().map(|file| file.relative_path.as_str()).collect();
        let required_paths: Vec<&str> = expected_layout
            .required_files
            .iter()
            .map(|path| path.as_str())
            .collect();
        assert_eq!(required_paths, artifact_paths);

        // Outer byte_size is the sum of the per-file sizes; every file carries a
        // direct HuggingFace URL + content SHA256 (no archive to extract).
        let summed: u64 = files.iter().map(|file| file.byte_size).sum();
        assert_eq!(artifact.byte_size, summed);
        assert_eq!(artifact.byte_size, 419_482_724);
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
        // PLDA + segmentation + WeSpeaker embedding land flat where from_dir
        // looks; sanity-check a flat root file and a nested .mlmodelc subpath.
        assert!(required_paths.contains(&"segmentation-3.0.onnx"));
        assert!(required_paths.contains(&"wespeaker-voxceleb-resnet34.onnx"));
        assert!(required_paths.contains(&"plda_lda.npy"));
        assert!(required_paths.contains(&"segmentation-3.0.mlmodelc/weights/weight.bin"));
        // plda_phi.npy is intentionally absent — speakrs does not read it.
        assert!(!required_paths.iter().any(|path| path.contains("plda_phi")));
    }

    #[test]
    fn default_preset_thresholds_match_calibrated_values() {
        let params = descriptor_for(DEFAULT_SHERPA_ONNX_MODEL_ID)
            .sherpa_params
            .expect("default preset has sherpa_params");
        assert_eq!(
            params.segmentation_relative_path,
            "pyannote-segmentation-3.0/model.onnx"
        );
        assert_eq!(params.embedding_relative_path, "nemo_en_titanet_small.onnx");
        // clustering_threshold (fast-clustering, #3) keeps the historical 0.65.
        assert_eq!(params.clustering_threshold, 0.65_f32);
        assert_eq!(params.clustering_threshold, DEFAULT_CLUSTERING_THRESHOLD);
        // cross_chunk_threshold restored to 0.60 (the DER-benchmark optimum;
        // a single-clip calibration had briefly lowered it to 0.50).
        assert_eq!(params.cross_chunk_threshold, 0.60_f32);
        assert_eq!(params.cross_chunk_threshold, BALANCED_CROSS_CHUNK_THRESHOLD);
        assert_eq!(params.min_turn_ms, DEFAULT_MIN_TURN_MS);
    }

    #[test]
    fn multilingual_preset_uses_campplus_zh_en_embedding() {
        let descriptor = descriptor_for(MULTILINGUAL_SHERPA_ONNX_MODEL_ID);
        let params = descriptor
            .sherpa_params
            .expect("multilingual sherpa_params");
        assert_eq!(
            params.segmentation_relative_path,
            "pyannote-segmentation-3.0/model.onnx"
        );
        assert_eq!(
            params.embedding_relative_path,
            "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx"
        );
        let ModelManagement::AppManaged {
            expected_layout, ..
        } = &descriptor.management;
        assert_eq!(
            expected_layout.required_files,
            vec![
                "pyannote-segmentation-3.0/model.onnx".to_string(),
                "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".to_string(),
            ]
        );
    }

    #[test]
    fn high_accuracy_preset_uses_reverb_v1_and_titanet_large() {
        let descriptor = descriptor_for(HIGH_ACCURACY_SHERPA_ONNX_MODEL_ID);
        let params = descriptor
            .sherpa_params
            .expect("high-accuracy sherpa_params");
        assert_eq!(
            params.segmentation_relative_path,
            "reverb-diarization-v1/model.onnx"
        );
        assert_eq!(params.embedding_relative_path, "nemo_en_titanet_large.onnx");
        let ModelManagement::AppManaged {
            expected_layout, ..
        } = &descriptor.management;
        assert_eq!(
            expected_layout.required_files,
            vec![
                "reverb-diarization-v1/model.onnx".to_string(),
                "nemo_en_titanet_large.onnx".to_string(),
            ]
        );
    }

    #[test]
    fn manifest_round_trips_through_json_including_sherpa_params() {
        let manifest = builtin_model_manifest();
        let encoded = serde_json::to_string(&manifest).expect("manifest encodes");
        let decoded: SpeakerAnalysisModelManifest =
            serde_json::from_str(&encoded).expect("manifest decodes");
        assert_eq!(decoded, manifest);
        // sherpa_params serializes camelCase and survives the round trip.
        assert!(encoded.contains("\"sherpaParams\""));
        assert!(encoded.contains("\"segmentationRelativePath\""));
        assert!(encoded.contains("\"clusteringThreshold\""));
        assert!(encoded.contains("\"crossChunkThreshold\""));
        assert!(encoded.contains("\"minTurnMs\""));
    }

    #[test]
    fn descriptor_without_sherpa_params_omits_field_and_round_trips() {
        // Forward-compat: a future non-sherpa provider descriptor has no params.
        let descriptor = SpeakerAnalysisModelDescriptor {
            provider: "future_provider".to_string(),
            model_id: Some("future-model".to_string()),
            display_name: "Future".to_string(),
            description: "Future provider".to_string(),
            license_label: None,
            source_url: None,
            management: ModelManagement::AppManaged {
                expected_layout: InstalledModelLayout {
                    marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                    required_files: vec!["model.bin".to_string()],
                },
                artifact: None,
            },
            sherpa_params: None,
        };
        let encoded = serde_json::to_string(&descriptor).expect("encodes");
        assert!(!encoded.contains("sherpaParams"));
        let decoded: SpeakerAnalysisModelDescriptor =
            serde_json::from_str(&encoded).expect("decodes");
        assert_eq!(decoded, descriptor);
        assert!(decoded.sherpa_params.is_none());
    }

    #[test]
    fn request_and_output_contract_round_trips_json() {
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.wav",
            SHERPA_ONNX_PROVIDER_ID,
            Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
            "session-a",
            42,
        );
        let mut output =
            SpeakerAnalysisOutput::new(SpeakerAnalysisMetadata::from_request(&request));
        output.clusters.push(SpeakerCluster {
            provider_cluster_id: "spk0".to_string(),
            stable_label: "Unknown Speaker 1".to_string(),
            embedding: vec![1, 2, 3],
            embedding_model_id: DEFAULT_SHERPA_ONNX_MODEL_ID.to_string(),
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
