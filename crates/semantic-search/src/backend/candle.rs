//! The candle **Semantic Search Backend** — the raw model forward on the Apple
//! GPU (Metal) or CPU.
//!
//! Ported from the proven `nomic-embed-text-v1.5` reference embedder: load the
//! descriptor's weights file into a `VarBuilder` — either `model.safetensors` via
//! an mmaped `VarBuilder` (nomic / e5) or a PyTorch `pytorch_model.bin` / `.pth`
//! via the safe pickle reader (`VarBuilder::from_pth`, used by bge-m3, whose repo
//! ships no safetensors) — run the architecture the descriptor names (NomicBert
//! for the English default, XLM-Roberta for the multilingual-e5 / bge-m3
//! families), pool per the descriptor (Mean or CLS), and L2-normalize. Always
//! returns F32 vectors so the scoring path is unchanged.
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

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::nomic_bert::{
    Config as NomicConfig, NomicBertModel,
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

        let model = match descriptor.architecture {
            SemanticSearchArchitecture::NomicBert => {
                let cfg: NomicConfig = serde_json::from_slice(&config_bytes)
                    .map_err(|error| EmbeddingError::LoadConfig(error.to_string()))?;
                let model = NomicBertModel::load(vb, &cfg)
                    .map_err(|error| EmbeddingError::LoadModel(error.to_string()))?;
                LoadedModel::NomicBert(model)
            }
            SemanticSearchArchitecture::XlmRoberta => {
                let cfg: XlmConfig = serde_json::from_slice(&config_bytes)
                    .map_err(|error| EmbeddingError::LoadConfig(error.to_string()))?;
                let model = XLMRobertaModel::new(&cfg, vb)
                    .map_err(|error| EmbeddingError::LoadModel(error.to_string()))?;
                LoadedModel::XlmRoberta(model)
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
            dimension: descriptor.dimension,
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
