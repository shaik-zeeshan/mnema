//! The candle **Semantic Search Backend** — the raw model forward on the Apple
//! GPU (Metal) or CPU.
//!
//! Ported from the proven `nomic-embed-text-v1.5` reference embedder: load the
//! descriptor's weights file into a `VarBuilder` — either `model.safetensors` via
//! an mmaped `VarBuilder` (nomic / e5 / Stella base) or a PyTorch
//! `pytorch_model.bin` / `.pth` via the safe pickle reader
//! (`VarBuilder::from_pth`, used by bge-m3, whose repo ships no safetensors) — run
//! the architecture the descriptor names (NomicBert for the English default,
//! XLM-Roberta for the multilingual-e5 / bge-m3 / Arctic families, StellaEnV5 for
//! the Stella English option), pool per the descriptor (Mean or CLS), and
//! L2-normalize. Always returns F32 vectors so the scoring path is unchanged.
//!
//! **StellaEnV5 (`stella_en_400M_v5`) is the one architecture that owns its own
//! pooling.** It is a backbone + a dense projection head: candle's
//! `stella_en_v5::EmbeddingModel` is built from TWO VarBuilders — the BASE (the
//! `new.`-prefixed backbone, reusing the same mmaped device-dtype `vb` every other
//! arch uses) and the HEAD (the `2_Dense_2048/model.safetensors` `linear.weight`
//! [2048,1024] + `linear.bias` [2048], named from `descriptor.expected_layout.
//! aux_weights_relative_path`). Its `forward` mean-pools the backbone hidden states
//! AND applies the dense head internally, so the external Mean/CLS pool step below
//! is BYPASSED for Stella — we l2-normalize the module's (B, 2048) output directly.
//! The candle module casts the pooled hidden to F32 BEFORE the head linear, so the
//! HEAD VarBuilder MUST be loaded at `DType::F32` in BOTH CPU and Metal builds
//! (the base VarBuilder still uses the device dtype: F16 on Metal, F32 on CPU).
//! `EmbeddingModel::forward` takes `&mut self` while the backend's `embed_batch` is
//! `&self`, so the model is held in a `std::sync::Mutex` (locked in the forward
//! path) — that bridges the `&mut` requirement and keeps `CandleBackend: Send`.
//!
//! **Device & precision.** Tries `Device::new_metal(0)` then falls back to CPU.
//! Metal kernels only link when the crate `metal` feature is on, so the metal
//! attempt is `#[cfg(feature = "metal")]`; a non-metal build is CPU-only and still
//! compiles. Precision is device-dependent: F16 on Metal (the RAM win, ~11% slower
//! — accepted), F32 on CPU (F16 is emulated/slow there).
//!
//! **U8 attention mask** (not U32): candle's `nomic_bert` builds the additive mask
//! via `where_cond`, whose condition dtype is the mask's dtype. On Metal only
//! `where_u8_{f16,f32}` (and `where_u32_f32`) kernels exist, so a U32 condition on
//! the F16 weight path hits "Metal where_cond U32 F16 not implemented". U8 has
//! kernels for BOTH branches, so this one dtype choice works in both precisions.

use std::path::Path;
use std::sync::Mutex;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::nomic_bert::{
    Config as NomicConfig, NomicBertModel,
};
use candle_transformers::models::stella_en_v5::{
    Config as StellaConfig, EmbedDim, EmbeddingModel as StellaEmbeddingModel,
};
use candle_transformers::models::xlm_roberta::{
    Config as XlmConfig, XLMRobertaModel,
};
use tokenizers::Tokenizer;

use super::EmbeddingError;
use crate::models::{
    SemanticSearchArchitecture, SemanticSearchModelDescriptor, SemanticSearchPooling,
    CONFIG_FILE_NAME, TOKENIZER_FILE_NAME,
};

/// The loaded architecture, dispatched off `descriptor.architecture`.
enum LoadedModel {
    NomicBert(NomicBertModel),
    XlmRoberta(XLMRobertaModel),
    /// Stella's backbone + dense-head embedder. Held behind a `Mutex` because its
    /// `forward` is `&mut self` (it mutates the base model's per-layer state) while
    /// the backend's `embed_batch` is `&self`; the lock bridges that and keeps
    /// `CandleBackend: Send`.
    Stella(Mutex<StellaEmbeddingModel>),
}

/// The candle embedding backend: one loaded model + the forward-pass tokenizer.
///
/// `cpu_fallback` records that Metal init failed and the device fell back to CPU
/// (the footprint then is not the F16-on-GPU figure, but parity holds). It is kept
/// for diagnostics by the caller; the embed path itself only reads `device`.
pub struct CandleBackend {
    model: LoadedModel,
    /// Tokenizer loaded from `tokenizer.json`, used for the forward pass
    /// (`encode_batch(.., true)` — WITH special tokens). The non-truncating split
    /// tokenizer used to compute chunk windows lives in the embedder wrapper above.
    tokenizer: Tokenizer,
    device: Device,
    pooling: SemanticSearchPooling,
    dimension: usize,
    /// True when Metal was unavailable and we fell back to CPU.
    pub cpu_fallback: bool,
}

impl CandleBackend {
    /// Load the backend from a `semantic_search_models/{provider}/{model_id}/`
    /// directory and its descriptor: read `config.json`, load the weights at the
    /// descriptor's weights path (mmaped safetensors for nomic / e5, or the safe
    /// pickle reader for a PyTorch `.bin` / `.pth` like bge-m3), dispatch the
    /// architecture, load the forward-pass tokenizer.
    pub fn load_from_dir(
        model_dir: impl AsRef<Path>,
        descriptor: &SemanticSearchModelDescriptor,
    ) -> Result<Self, EmbeddingError> {
        let (device, cpu_fallback) = pick_device();
        Self::load_on_device(model_dir, descriptor, device, cpu_fallback)
    }

    /// Load the backend forced onto the CPU (F32) — the always-available reference
    /// precision. Exists so the parity gate can load the F32 reference next to the
    /// Metal (F16) backend and prove the two precisions agree; production always
    /// goes through [`load_from_dir`](Self::load_from_dir).
    #[doc(hidden)]
    pub fn load_cpu(
        model_dir: impl AsRef<Path>,
        descriptor: &SemanticSearchModelDescriptor,
    ) -> Result<Self, EmbeddingError> {
        Self::load_on_device(model_dir, descriptor, Device::Cpu, false)
    }

    /// Load the backend forced onto Metal (F16) when one can be acquired (the
    /// `metal` feature is compiled in AND `Device::new_metal(0)` succeeds), else
    /// `None`. Mirrors the Metal branch of [`pick_device`]; the parity gate uses the
    /// `None` to skip the CPU-vs-Metal compare on CI / non-macOS / headless runners
    /// where Metal is unavailable. Keeps `candle_core::Device` out of the caller.
    #[doc(hidden)]
    pub fn try_load_metal(
        model_dir: impl AsRef<Path>,
        descriptor: &SemanticSearchModelDescriptor,
    ) -> Option<Result<Self, EmbeddingError>> {
        #[cfg(feature = "metal")]
        {
            let device = Device::new_metal(0).ok()?;
            Some(Self::load_on_device(model_dir, descriptor, device, false))
        }
        #[cfg(not(feature = "metal"))]
        {
            let _ = (model_dir, descriptor);
            None
        }
    }

    /// Load the backend onto a caller-chosen `device` (otherwise identical to
    /// [`load_from_dir`](Self::load_from_dir), which picks the device itself). The
    /// shared core behind `load_from_dir`, `load_cpu`, and `try_load_metal`.
    fn load_on_device(
        model_dir: impl AsRef<Path>,
        descriptor: &SemanticSearchModelDescriptor,
        device: Device,
        cpu_fallback: bool,
    ) -> Result<Self, EmbeddingError> {
        let model_dir = model_dir.as_ref();

        // F16 on Metal (RAM win, ~11% slower — accepted), F32 on CPU (F16 is
        // emulated/slow there). The on-disk weights are F32; the dtype arg casts
        // them into device memory at load.
        let dtype = if device.is_metal() {
            DType::F16
        } else {
            DType::F32
        };

        let weights_path = model_dir.join(&descriptor.expected_layout.weights_relative_path);
        // The weights file must exist before we hand it to candle; surface a
        // neutral ReadModelFile (not a deep candle string) if it does not. The
        // message names the ACTUAL weights file the descriptor declares
        // (`model.safetensors` for nomic / e5, `pytorch_model.bin` for bge-m3) so a
        // missing-file error is honest about what was sought.
        if !weights_path.is_file() {
            return Err(EmbeddingError::ReadModelFile {
                path: weights_path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{} not found", descriptor.expected_layout.weights_relative_path),
                ),
            });
        }

        let config_path = model_dir.join(CONFIG_FILE_NAME);
        let config_bytes = read_file(&config_path)?;

        // Branch on the weights file format. A PyTorch checkpoint (`.bin` / `.pth`,
        // case-insensitive) goes through the SAFE pickle reader; a safetensors file
        // goes through the mmap path below.
        //
        // SAFETY: the unsafe mmap is reached ONLY on the safetensors branch — it is
        // an mmap of a trusted local safetensors file; the mapping's lifetime ends
        // with the VarBuilder and the model copies what it needs at load time. The
        // `.bin`/`.pth` branch is entirely safe: `VarBuilder::from_pth` is a safe fn
        // backed by `candle_core::pickle::PthTensors`, which reads tensors lazily
        // per-name — so a large PyTorch checkpoint like bge-m3's 2.27 GB does NOT
        // double-load (only the tensors the model actually requests are read, and
        // any int64 buffers the architecture never asks for are never touched). In
        // BOTH branches the `dtype` arg casts the on-disk F32 tensors into device
        // memory at the requested precision (F16 ~halves the resident weight bytes
        // on Metal).
        let is_pytorch_checkpoint = weights_path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("bin") || ext.eq_ignore_ascii_case("pth")
            });
        let vb = if is_pytorch_checkpoint {
            VarBuilder::from_pth(&weights_path, dtype, &device)
                .map_err(|error| EmbeddingError::LoadModel(error.to_string()))?
        } else {
            unsafe {
                VarBuilder::from_mmaped_safetensors(&[&weights_path], dtype, &device)
                    .map_err(|error| EmbeddingError::LoadModel(error.to_string()))?
            }
        };

        // Each arm yields the loaded model AND the NATIVE width the backend
        // actually produces — the dimension the scoring path sees BEFORE the
        // embedder wrapper applies any MRL truncation. For NomicBert and StellaEnV5
        // the native width equals `descriptor.dimension` (no truncation; Stella's
        // 2048 head IS the stored width). For XlmRoberta it is the backbone
        // `cfg.hidden_size` (1024 for Arctic, which truncates 1024 → 256 ABOVE this
        // trait via `mrl_truncate_dim`; equals `descriptor.dimension` for e5 / bge,
        // which do not truncate). Reporting the native width here is the Arctic fix:
        // the backend honestly reports 1024 even though the descriptor stores 256.
        let (model, dimension) = match descriptor.architecture {
            SemanticSearchArchitecture::NomicBert => {
                let cfg: NomicConfig = serde_json::from_slice(&config_bytes)
                    .map_err(|error| EmbeddingError::LoadConfig(error.to_string()))?;
                let model = NomicBertModel::load(vb, &cfg)
                    .map_err(|error| EmbeddingError::LoadModel(error.to_string()))?;
                (LoadedModel::NomicBert(model), descriptor.dimension)
            }
            SemanticSearchArchitecture::XlmRoberta => {
                let cfg: XlmConfig = serde_json::from_slice(&config_bytes)
                    .map_err(|error| EmbeddingError::LoadConfig(error.to_string()))?;
                let native_dim = cfg.hidden_size;
                let model = XLMRobertaModel::new(&cfg, vb)
                    .map_err(|error| EmbeddingError::LoadModel(error.to_string()))?;
                (LoadedModel::XlmRoberta(model), native_dim)
            }
            // Stella is a backbone + dense projection head. The base `vb` built
            // above (device dtype, mmaped `model.safetensors`) is the BASE; the head
            // is a SECOND VarBuilder over `2_Dense_2048/model.safetensors`. Stella
            // does NOT parse `config.json` — the candle `Config` is constructed
            // (`new_400_m_v5`), not deserialized — but `config.json` is still a
            // required, separately-downloaded file (left untouched here).
            SemanticSearchArchitecture::StellaEnV5 => {
                // The head weights path is mandatory for Stella; a None aux path is a
                // descriptor wiring error, not a missing-file-on-disk condition.
                let head_rel = descriptor
                    .expected_layout
                    .aux_weights_relative_path
                    .as_ref()
                    .ok_or_else(|| {
                        EmbeddingError::LoadModel(
                            "stella_en_v5 requires aux_weights_relative_path (the \
                             2_Dense_2048 head), but the descriptor layout has none"
                                .to_string(),
                        )
                    })?;
                let head_path = model_dir.join(head_rel);
                // Surface a neutral ReadModelFile (not a deep candle string) if the
                // head safetensors is absent, naming the file the descriptor declares.
                if !head_path.is_file() {
                    return Err(EmbeddingError::ReadModelFile {
                        path: head_path.clone(),
                        source: std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("{head_rel} not found"),
                        ),
                    });
                }

                // The head MUST be F32 in BOTH CPU and Metal builds: candle casts the
                // pooled hidden to F32 before the head linear, so an F16 head would
                // dtype-mismatch the linear on Metal. The base `vb` keeps the device
                // dtype (F16 on Metal, F32 on CPU); only the head is pinned to F32.
                //
                // SAFETY: an mmap of a trusted local safetensors file; the mapping's
                // lifetime ends with the VarBuilder and `EmbeddingModel::new` copies
                // the head tensors it needs at construction time.
                let head_vb = unsafe {
                    VarBuilder::from_mmaped_safetensors(&[&head_path], DType::F32, &device)
                        .map_err(|error| EmbeddingError::LoadModel(error.to_string()))?
                };

                // The candle Stella `Config` is parameterised by the head's output
                // width. We only ship the native `2_Dense_2048` head, so the stored
                // descriptor dimension must be exactly 2048; anything else is an
                // unsupported wiring.
                let embed_dim = match descriptor.dimension {
                    2048 => EmbedDim::Dim2048,
                    other => {
                        return Err(EmbeddingError::LoadModel(format!(
                            "stella_en_v5 only supports the 2048-dim head, but the \
                             descriptor declares dimension {other}"
                        )));
                    }
                };
                let cfg = StellaConfig::new_400_m_v5(embed_dim);
                // Base = the device-dtype `vb` reused from above; head = the F32
                // head VarBuilder. The model owns mean-pool + the dense head. The
                // native width is the head's output (2048) = `descriptor.dimension`;
                // Stella does not truncate.
                let model = StellaEmbeddingModel::new(&cfg, vb, head_vb)
                    .map_err(|error| EmbeddingError::LoadModel(error.to_string()))?;
                (LoadedModel::Stella(Mutex::new(model)), descriptor.dimension)
            }
        };

        let tokenizer_path = model_dir.join(TOKENIZER_FILE_NAME);
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|error| EmbeddingError::LoadTokenizer(error.to_string()))?;

        Ok(Self {
            model,
            tokenizer,
            device,
            pooling: descriptor.pooling,
            // The NATIVE produced width computed per-arch above (= backbone
            // `hidden_size` for XlmRoberta, so Arctic reports 1024 not its
            // truncated 256; = `descriptor.dimension` for the others).
            dimension,
            cpu_fallback,
        })
    }

    /// One candle forward pass over a batch of in-window fragments → one pooled,
    /// L2-normalized vector per fragment. Pads to the batch's longest sequence
    /// (BatchLongest), masks padding via the attention mask.
    fn forward_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        // Encode WITH special tokens (CLS/SEP) for the model forward pass.
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|error| EmbeddingError::Tokenize(error.to_string()))?;

        let max_len = encodings.iter().map(|e| e.len()).max().unwrap_or(1).max(1);
        let b = encodings.len();

        let mut ids: Vec<u32> = Vec::with_capacity(b * max_len);
        // U8 mask, NOT U32 — see the module docs (Metal `where_cond` kernel
        // availability). `mean_pool` casts the mask to the hidden dtype itself, so
        // pooling is precision-agnostic regardless.
        let mut mask: Vec<u8> = Vec::with_capacity(b * max_len);
        for enc in &encodings {
            let e_ids = enc.get_ids();
            let e_mask = enc.get_attention_mask();
            for j in 0..max_len {
                if j < e_ids.len() {
                    ids.push(e_ids[j]);
                    mask.push(e_mask[j] as u8);
                } else {
                    ids.push(0); // pad token id (0)
                    mask.push(0);
                }
            }
        }

        let input_ids = Tensor::from_vec(ids, (b, max_len), &self.device)
            .map_err(|error| EmbeddingError::Embed(error.to_string()))?;
        let attention_mask = Tensor::from_vec(mask, (b, max_len), &self.device)
            .map_err(|error| EmbeddingError::Embed(error.to_string()))?;

        // Stella owns its pooling (mean-pool + dense head), so it bypasses the
        // external Mean/CLS pool below entirely. `forward(&mut self, ..)` mutates
        // the base model's per-layer state, so we lock the `Mutex` to call it from
        // this `&self` path. It returns (B, 2048) F32 un-normalized — the candle
        // module already cast the pooled hidden to F32 before its head linear — so
        // we only need to L2-normalize, then drop to CPU `f32` rows. The U8
        // `attention_mask` is the `where_cond` condition inside the backbone, which
        // has `where_u8_{f16,f32}` kernels (the same reason the U8 mask is used
        // everywhere here), so it works in both precisions.
        if let LoadedModel::Stella(model) = &self.model {
            let pooled = model
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .forward(&input_ids, &attention_mask)
                .map_err(|error| EmbeddingError::Embed(error.to_string()))?; // (B, 2048)
            let normed =
                l2_normalize(&pooled).map_err(|error| EmbeddingError::Embed(error.to_string()))?;
            let normed = normed
                .to_dtype(DType::F32)
                .and_then(|t| t.to_device(&Device::Cpu))
                .map_err(|error| EmbeddingError::Embed(error.to_string()))?;
            return normed
                .to_vec2()
                .map_err(|error| EmbeddingError::Embed(error.to_string()));
        }

        // (B, L, H)
        let hidden = match &self.model {
            LoadedModel::NomicBert(model) => model
                .forward(&input_ids, None, Some(&attention_mask))
                .map_err(|error| EmbeddingError::Embed(error.to_string()))?,
            LoadedModel::XlmRoberta(model) => {
                // XLM-Roberta's forward takes a non-optional `token_type_ids` and
                // builds its own additive 4d mask internally (in F32, cast to the
                // hidden dtype at use), so it works in both F16 and F32.
                let token_type_ids = input_ids
                    .zeros_like()
                    .map_err(|error| EmbeddingError::Embed(error.to_string()))?;
                model
                    .forward(
                        &input_ids,
                        &attention_mask,
                        &token_type_ids,
                        None,
                        None,
                        None,
                    )
                    .map_err(|error| EmbeddingError::Embed(error.to_string()))?
            }
            // Stella returned above via its own module-owned pooling path; it never
            // reaches this hidden-states-then-external-pool branch.
            LoadedModel::Stella(_) => unreachable!(
                "Stella is handled by the early-return forward path before this match"
            ),
        };

        // Pool per the model's declared strategy, then L2-normalize.
        let pooled = match self.pooling {
            SemanticSearchPooling::Mean => mean_pool(&hidden, &attention_mask),
            SemanticSearchPooling::Cls => cls_pool(&hidden),
        }
        .map_err(|error| EmbeddingError::Embed(error.to_string()))?; // (B, H)

        let normed = l2_normalize(&pooled).map_err(|error| EmbeddingError::Embed(error.to_string()))?;
        let normed = normed
            .to_dtype(DType::F32)
            .and_then(|t| t.to_device(&Device::Cpu))
            .map_err(|error| EmbeddingError::Embed(error.to_string()))?;
        let rows: Vec<Vec<f32>> = normed
            .to_vec2()
            .map_err(|error| EmbeddingError::Embed(error.to_string()))?;
        Ok(rows)
    }
}

impl super::SemanticSearchBackend for CandleBackend {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.forward_batch(texts)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

/// Try Metal (when the `metal` feature is compiled in), fall back to CPU. Returns
/// `(device, cpu_fallback)` — `cpu_fallback` is true only when a Metal attempt was
/// made and failed (so a CPU-only build is `false`, not a "fallback").
fn pick_device() -> (Device, bool) {
    #[cfg(feature = "metal")]
    {
        match Device::new_metal(0) {
            Ok(device) => (device, false),
            Err(_) => (Device::Cpu, true),
        }
    }
    #[cfg(not(feature = "metal"))]
    {
        (Device::Cpu, false)
    }
}

fn read_file(path: &Path) -> Result<Vec<u8>, EmbeddingError> {
    std::fs::read(path).map_err(|source| EmbeddingError::ReadModelFile {
        path: path.to_path_buf(),
        source,
    })
}

/// Mean-pool token hidden states over the attention mask. hidden: (B,L,H),
/// mask: (B,L). Returns (B,H). The mask is cast to the hidden dtype, so this is
/// precision-agnostic.
fn mean_pool(hidden: &Tensor, mask: &Tensor) -> candle_core::Result<Tensor> {
    let (_b, _l, h) = hidden.dims3()?;
    let mask_f = mask.to_dtype(hidden.dtype())?.unsqueeze(2)?; // (B,L,1)
    let masked = hidden.broadcast_mul(&mask_f)?; // (B,L,H)
    let summed = masked.sum(1)?; // (B,H)
    let counts = mask_f.sum(1)?; // (B,1)
    let counts = counts
        .broadcast_as((counts.dim(0)?, h))?
        .clamp(1e-9, f64::INFINITY)?;
    summed.div(&counts)
}

/// CLS-pool: take the `[CLS]` token (row 0 of the sequence dim) hidden state.
/// hidden: (B,L,H) → (B,H). Used by bge-m3 (CLS pooling).
fn cls_pool(hidden: &Tensor) -> candle_core::Result<Tensor> {
    // index 0 along the sequence dim (dim 1) → (B, H).
    hidden.get_on_dim(1, 0)?.contiguous()
}

/// L2-normalize each row of (B,H), in F32.
fn l2_normalize(x: &Tensor) -> candle_core::Result<Tensor> {
    // Normalize in F32 regardless of the device dtype: on Metal `x` is F16, where
    // the `1e-9` zero-guard floor below underflows to 0.0 and stops guarding — a
    // degenerate all-zero pooled row would then divide by zero and store NaN. The
    // cast makes the floor effective AND drops F16 norm precision loss; the output
    // is f32 anyway (the caller casts to F32 before `to_vec2`).
    let x = x.to_dtype(DType::F32)?;
    // Floor the norm at a tiny positive value so a degenerate all-zero pooled row
    // divides to a finite (zero) vector instead of NaN/Inf — the same guard the
    // wrapper's `mean_pool_l2` applies via its epsilon, and the clamp idiom
    // `mean_pool` above uses for its token counts. A real pooled vector's
    // pre-normalization norm is O(√H), far above the floor, so a normal embedding
    // is never perturbed.
    let norm = x.sqr()?.sum_keepdim(1)?.sqrt()?.clamp(1e-9, f64::INFINITY)?; // (B,1)
    x.broadcast_div(&norm)
}
