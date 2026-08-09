//! Synthetic forward-pass gate for the **ModernBert** dispatch arm — RUNS IN CI on
//! CPU, no network, no real weights.
//!
//! The sibling `candle_synthetic_forward.rs` covers the XLM-RoBERTa arm. This file
//! is its ModernBERT counterpart, added with the `gte-modernbert-base` /
//! `granite-embedding-*-r2` Custom options (issue #190/#193). It mints a tiny
//! random-weight ModernBERT on disk whose tensor-name layout matches exactly what
//! `candle_transformers::models::modernbert::ModernBert::load` asks the VarBuilder
//! for, then drives it through the real public [`CandleBackend`].
//!
//! Unlike NomicBert (deliberately not synthesized in the sibling file), ModernBERT
//! synthesizes fine despite its rotary embeddings: `RotaryEmbedding::new` derives
//! its sin/cos table from the config alone, so no weight tensor has to be
//! hand-crafted for it.
//!
//! **The precision assertion is the point of this file.** candle 0.10.2's
//! `modernbert` forward builds its additive attention mask in F32 and adds it to
//! attention scores carrying the *weight* dtype, without casting either side. Under
//! F16 weights that add fails outright (`dtype mismatch in add, lhs: F16, rhs:
//! F32`), which is why `backend::candle::arch_dtype` pins this one architecture to
//! F32 on every device including Metal. Two tests guard that:
//!
//!   * [`modernbert_f16_weights_are_not_loadable_in_candle_0_10_2`] pins the
//!     upstream defect itself, at the candle API, on CPU — so it runs in CI and
//!     starts FAILING the day a candle bump fixes the mask cast, which is the
//!     signal to delete the special case and let ModernBERT be F16 on Metal.
//!   * [`modernbert_forwards_on_metal_when_available`] runs the real backend on
//!     Metal when one can be acquired (a no-op on CI / non-macOS / a non-`metal`
//!     build). Deleting the `arch_dtype` special case makes it fail on any Mac.
//!
//! The weights are random, so the *values* are meaningless — this is a structural
//! test (dispatch, shape, norm, pooling, pad-invariance, precision), never a quality
//! check.

use std::collections::HashMap;
use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::modernbert::{Config as ModernBertConfig, ModernBert};
use semantic_search::{CandleBackend, SemanticSearchBackend, SemanticSearchModelDescriptor};

/// A tiny ModernBERT — small enough to mint random weights fast, large enough to
/// exercise every load key and BOTH attention paths (`global_attn_every_n_layers = 3`
/// with 3 layers means layer 0 is global and layers 1-2 are local sliding-window).
const VOCAB_SIZE: usize = 32;
const HIDDEN_SIZE: usize = 16;
const NUM_HEADS: usize = 4;
const NUM_LAYERS: usize = 3;
const INTERMEDIATE_SIZE: usize = 32;
const MAX_POSITION_EMBEDDINGS: usize = 64;
const GLOBAL_ATTN_EVERY_N_LAYERS: usize = 3;
/// ModernBERT reserves a dedicated `[PAD]` id well away from 0, so a literal-0 pad
/// would alias a real content token here exactly as it would upstream.
const PAD_TOKEN_ID: u32 = 1;

fn randn(shape: &[usize]) -> Tensor {
    Tensor::randn(0f32, 1f32, shape, &Device::Cpu).expect("randn tensor")
}

/// `layer_norm_no_bias` reads only `{prefix}.weight` — ModernBERT uses bias-free
/// norms throughout. Ones keeps the norm well-conditioned.
fn insert_norm(t: &mut HashMap<String, Tensor>, prefix: &str) {
    t.insert(
        format!("{prefix}.weight"),
        Tensor::ones(HIDDEN_SIZE, DType::F32, &Device::Cpu).expect("norm weight"),
    );
}

/// The tensor-name prefix a real `ModernBertModel` export uses: **none**. All three
/// shipped options (gte-modernbert-base, granite-embedding-english-r2,
/// granite-embedding-small-english-r2) store `embeddings.tok_embeddings.weight`,
/// verified against the published safetensors headers.
const BARE_PREFIX: &str = "";
/// The prefix candle 0.10.2's `ModernBert::load` hardcodes into every VarBuilder
/// key — the layout of a `ModernBertForMaskedLM` export like
/// `answerdotai/ModernBERT-base`, where the backbone sits under `model.` beside
/// `head.` / `decoder.`. The backend detects which layout is on disk and renames.
const MASKED_LM_PREFIX: &str = "model.";

/// The tensor map for the VarBuilder keys `ModernBert::load` requests, under
/// `prefix` (`""` for a bare `ModernBertModel` export, `"model."` for a MaskedLM
/// one).
///
/// Two shapes are easy to get wrong and are load-bearing: the fused `attn.Wqkv` is
/// `[3 * hidden, hidden]` (q, k and v in one matrix), and the GeGLU `mlp.Wi` is
/// `[2 * intermediate, hidden]` because its output is chunked in half — one half
/// gated by the gelu of the other. Layer 0 has NO `attn_norm` (upstream ModernBERT
/// omits it; candle's `layer_norm_no_bias(..).ok()` turns the missing tensor into
/// `None`), so leaving it out here is what makes that branch real rather than
/// incidental.
fn modernbert_tensors(prefix: &str) -> HashMap<String, Tensor> {
    let mut t: HashMap<String, Tensor> = HashMap::new();
    t.insert(
        format!("{prefix}embeddings.tok_embeddings.weight"),
        randn(&[VOCAB_SIZE, HIDDEN_SIZE]),
    );
    insert_norm(&mut t, &format!("{prefix}embeddings.norm"));
    for i in 0..NUM_LAYERS {
        let l = format!("{prefix}layers.{i}");
        if i != 0 {
            insert_norm(&mut t, &format!("{l}.attn_norm"));
        }
        t.insert(
            format!("{l}.attn.Wqkv.weight"),
            randn(&[HIDDEN_SIZE * 3, HIDDEN_SIZE]),
        );
        t.insert(
            format!("{l}.attn.Wo.weight"),
            randn(&[HIDDEN_SIZE, HIDDEN_SIZE]),
        );
        insert_norm(&mut t, &format!("{l}.mlp_norm"));
        t.insert(
            format!("{l}.mlp.Wi.weight"),
            randn(&[INTERMEDIATE_SIZE * 2, HIDDEN_SIZE]),
        );
        t.insert(
            format!("{l}.mlp.Wo.weight"),
            randn(&[HIDDEN_SIZE, INTERMEDIATE_SIZE]),
        );
    }
    insert_norm(&mut t, &format!("{prefix}final_norm"));
    t
}

fn write_safetensors(path: &Path, prefix: &str) {
    candle_core::safetensors::save(&modernbert_tensors(prefix), path)
        .expect("save synthetic safetensors");
}

/// The minimal `config.json` `modernbert::Config` deserializes. Note that the real
/// repos also carry `"classifier_pooling": "mean"` — the *classification head's*
/// pooling, which has nothing to do with the embedding pooling and is deliberately
/// not mirrored into the descriptor (see `pooling_is_a_declared_field_hand_coded_per_model`).
fn write_config(path: &Path) {
    let config = serde_json::json!({
        "vocab_size": VOCAB_SIZE,
        "hidden_size": HIDDEN_SIZE,
        "num_hidden_layers": NUM_LAYERS,
        "num_attention_heads": NUM_HEADS,
        "intermediate_size": INTERMEDIATE_SIZE,
        "max_position_embeddings": MAX_POSITION_EMBEDDINGS,
        "layer_norm_eps": 1e-5,
        "pad_token_id": PAD_TOKEN_ID,
        "global_attn_every_n_layers": GLOBAL_ATTN_EVERY_N_LAYERS,
        "global_rope_theta": 160000.0,
        "local_attention": 128,
        "local_rope_theta": 10000.0,
    });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&config).expect("serialize config"),
    )
    .expect("write config.json");
}

/// A minimal WordLevel `tokenizer.json` declaring `pad_id = PAD_TOKEN_ID`, so the
/// backend resolves the real pad id from the tokenizer rather than defaulting to 0.
fn write_tokenizer(path: &Path) {
    let tokenizer = serde_json::json!({
        "version": "1.0",
        "truncation": null,
        "padding": {
            "strategy": "BatchLongest",
            "direction": "Right",
            "pad_to_multiple_of": null,
            "pad_id": PAD_TOKEN_ID,
            "pad_type_id": 0,
            "pad_token": "[PAD]"
        },
        "added_tokens": [],
        "normalizer": null,
        "pre_tokenizer": { "type": "WhitespaceSplit" },
        "post_processor": null,
        "decoder": null,
        "model": {
            "type": "WordLevel",
            "vocab": {
                "[CLS]": 0,
                "[PAD]": 1,
                "[UNK]": 2,
                "the": 3,
                "quick": 4,
                "brown": 5,
                "fox": 6,
                "jumps": 7,
                "over": 8,
                "lazy": 9,
                "dog": 10,
                "short": 11,
                "text": 12
            },
            "unk_token": "[UNK]"
        }
    });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&tokenizer).expect("serialize tokenizer"),
    )
    .expect("write tokenizer.json");
}

/// A synthetic `modern_bert` descriptor pointing at the on-disk model. Deserialized
/// from JSON so the test does not hand-construct the large struct; `pooling` is
/// parameterised so one model dir covers both strategies.
fn synthetic_descriptor(pooling: &str) -> SemanticSearchModelDescriptor {
    let json = serde_json::json!({
        "provider": "local",
        "modelId": "synthetic-modernbert",
        "displayName": "Synthetic ModernBERT (test)",
        "description": "In-test synthetic model for the ModernBert forward smoke gate.",
        "tier": "custom",
        "architecture": "modern_bert",
        "hfRepo": "test/synthetic",
        "hfRevision": "0000000000000000000000000000000000000000",
        "licenseLabel": null,
        "dimension": HIDDEN_SIZE,
        "maxTokens": MAX_POSITION_EMBEDDINGS,
        "approxDownloadBytes": 0,
        "pooling": pooling,
        "queryPrompt": null,
        "documentPrompt": null,
        "mrlTruncateDim": null,
        "expectedLayout": {
            "markerFileName": ".mnema_installed",
            "requiredFiles": ["model.safetensors", "config.json", "tokenizer.json"],
            "weightsRelativePath": "model.safetensors",
            "auxWeightsRelativePath": null
        }
    });
    serde_json::from_value(json).expect("synthetic descriptor deserializes")
}

/// A synthetic model dir whose weights use `prefix`. The default
/// [`synthetic_model_dir`] uses [`BARE_PREFIX`] — the layout every shipped model
/// actually has.
fn synthetic_model_dir_with_prefix(prefix: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    write_safetensors(&dir.path().join("model.safetensors"), prefix);
    write_config(&dir.path().join("config.json"));
    write_tokenizer(&dir.path().join("tokenizer.json"));
    dir
}

fn synthetic_model_dir() -> tempfile::TempDir {
    synthetic_model_dir_with_prefix(BARE_PREFIX)
}

fn l2_norm(vector: &[f32]) -> f32 {
    vector.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    dot / (l2_norm(a) * l2_norm(b))
}

fn assert_unit_vector(label: &str, vector: &[f32]) {
    assert_eq!(
        vector.len(),
        HIDDEN_SIZE,
        "{label}: output width must equal the config hidden_size"
    );
    assert!(
        vector.iter().all(|v| v.is_finite()),
        "{label}: every component must be finite (no NaN/Inf)"
    );
    let norm = l2_norm(vector);
    assert!(
        (norm - 1.0).abs() < 1e-4,
        "{label}: output must be L2-normalized (got norm {norm})"
    );
}

/// The ModernBert arm dispatches, loads, and forwards on CPU for both pooling
/// strategies, and padding a short row alongside a long one does not change its
/// vector.
#[test]
fn modernbert_forwards_and_pools_on_cpu() {
    let dir = synthetic_model_dir();
    let long = "the quick brown fox jumps over the lazy dog";
    let short = "short text";

    for pooling in ["mean", "cls"] {
        let descriptor = synthetic_descriptor(pooling);
        let backend = CandleBackend::load_cpu(dir.path(), &descriptor)
            .unwrap_or_else(|error| panic!("{pooling}: synthetic backend loads on CPU: {error}"));
        assert_eq!(
            backend.dimension(),
            HIDDEN_SIZE,
            "{pooling}: backend reports the ModernBERT backbone hidden_size"
        );
        assert_eq!(
            backend.resolved_pad_id(),
            PAD_TOKEN_ID,
            "{pooling}: backend must read the pad id from the tokenizer, not default to 0"
        );

        let solo = backend.embed_batch(&[short]).expect("solo embed");
        assert_eq!(solo.len(), 1);
        assert_unit_vector(&format!("{pooling}/solo"), &solo[0]);

        // The short row is padded to the long row's length in this batch. Padding
        // must be inert: the attention mask excludes the pad slots, so the short
        // vector must come back unchanged.
        let batched = backend.embed_batch(&[short, long]).expect("batched embed");
        assert_eq!(batched.len(), 2);
        assert_unit_vector(&format!("{pooling}/batched-short"), &batched[0]);
        assert_unit_vector(&format!("{pooling}/batched-long"), &batched[1]);
        let similarity = cosine(&solo[0], &batched[0]);
        assert!(
            similarity > 0.999,
            "{pooling}: a padded short row must embed identically to the same text \
             alone (cosine {similarity})"
        );
    }
}

/// The descriptor's declared pooling actually selects the pool. A ModernBERT config
/// carries `classifier_pooling`, which the loader must ignore in favour of the
/// hand-coded descriptor field — if the arm ever started reading the config instead,
/// both descriptors would produce the same vector and this fails.
#[test]
fn modernbert_pooling_follows_the_descriptor_not_the_config() {
    let dir = synthetic_model_dir();
    let text = "the quick brown fox jumps over the lazy dog";

    let mean = CandleBackend::load_cpu(dir.path(), &synthetic_descriptor("mean"))
        .expect("mean backend loads")
        .embed_batch(&[text])
        .expect("mean embed");
    let cls = CandleBackend::load_cpu(dir.path(), &synthetic_descriptor("cls"))
        .expect("cls backend loads")
        .embed_batch(&[text])
        .expect("cls embed");

    let similarity = cosine(&mean[0], &cls[0]);
    assert!(
        similarity < 0.999,
        "Mean and CLS pooling over a multi-token sequence must differ (cosine \
         {similarity}); an identical result means the declared pooling was ignored"
    );
}

/// **Upstream-defect pin.** candle 0.10.2's `ModernBert::forward` adds an F32-only
/// attention mask to attention scores in the weight dtype, so an F16 load cannot
/// complete a forward pass. `backend::candle::arch_dtype` works around that by
/// pinning this architecture to F32 on every device, at a real RAM cost (~596 MB
/// resident for the 149M-param models instead of ~298 MB).
///
/// This test asserts the defect still exists. When it starts FAILING after a candle
/// bump, the workaround is obsolete: delete the `ModernBert` arm of `arch_dtype` so
/// Metal gets F16 like every other architecture, and delete this test.
#[test]
fn modernbert_f16_weights_are_not_loadable_in_candle_0_10_2() {
    // Drives `ModernBert::load` directly rather than through the backend, so the
    // weights must carry the `model.` prefix candle hardcodes — the backend's
    // rename shim is not in this path.
    let dir = synthetic_model_dir_with_prefix(MASKED_LM_PREFIX);
    let weights = dir.path().join("model.safetensors");
    let config: ModernBertConfig = serde_json::from_slice(
        &std::fs::read(dir.path().join("config.json")).expect("read config"),
    )
    .expect("parse config");

    // SAFETY: mmap of a trusted safetensors file this test just wrote; the mapping
    // outlives the VarBuilder use below.
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[&weights], DType::F16, &Device::Cpu)
            .expect("F16 VarBuilder builds — the load itself is fine, the forward is not")
    };
    let model = ModernBert::load(vb, &config).expect("F16 ModernBert LOADS");
    let ids = Tensor::from_vec(vec![3u32, 4, 5, 6], (1, 4), &Device::Cpu).expect("ids");
    let mask = Tensor::from_vec(vec![1u8, 1, 1, 1], (1, 4), &Device::Cpu).expect("mask");

    let error = model
        .forward(&ids, &mask)
        .expect_err(
            "candle 0.10.2 ModernBERT cannot forward in F16 — if this now SUCCEEDS, the \
             upstream mask-dtype defect is fixed: drop the ModernBert arm of \
             backend::candle::arch_dtype (so Metal uses F16) and delete this test",
        )
        .to_string();
    assert!(
        error.contains("dtype mismatch"),
        "expected the F32-mask/F16-hidden dtype mismatch, got a different failure: {error}"
    );
}

/// **Regression pin for the shipped-weights load failure.** candle 0.10.2's
/// `ModernBert::load` hardcodes a `model.` prefix on every VarBuilder key, but all
/// three shipped ModernBERT options are bare `ModernBertModel` exports with no
/// prefix — so before the backend's rename shim, the desktop backfill worker died
/// with `cannot find tensor model.embeddings.tok_embeddings.weight` the first time
/// it touched a real checkpoint. Both layouts must load, and identical weights under
/// either naming must produce the identical vector.
#[test]
fn modernbert_loads_both_the_bare_and_the_model_prefixed_weight_layouts() {
    let descriptor = synthetic_descriptor("cls");
    let text = "the quick brown fox jumps over the lazy dog";

    // Same tensor VALUES under both namings: mint one map, write it twice.
    let tensors = modernbert_tensors(BARE_PREFIX);
    let prefixed: HashMap<String, Tensor> = tensors
        .iter()
        .map(|(k, v)| (format!("{MASKED_LM_PREFIX}{k}"), v.clone()))
        .collect();

    let mut vectors = Vec::new();
    for map in [&tensors, &prefixed] {
        let dir = tempfile::tempdir().expect("tempdir");
        candle_core::safetensors::save(map, dir.path().join("model.safetensors"))
            .expect("save synthetic safetensors");
        write_config(&dir.path().join("config.json"));
        write_tokenizer(&dir.path().join("tokenizer.json"));
        let backend = CandleBackend::load_cpu(dir.path(), &descriptor)
            .unwrap_or_else(|error| panic!("both ModernBERT weight layouts must load: {error}"));
        vectors.push(backend.embed_batch(&[text]).expect("embed")[0].clone());
    }

    let similarity = cosine(&vectors[0], &vectors[1]);
    assert!(
        similarity > 0.9999,
        "the same weights under either naming must embed identically (cosine {similarity})"
    );
}

/// The real backend loads and forwards on Metal. Self-skipping: `try_load_metal`
/// returns `None` without the `metal` feature or without a GPU, so CI and non-macOS
/// runs are a no-op. On a Mac with `--features metal` this is the direct guard on
/// the `arch_dtype` F32 rule — remove that rule and this fails to embed.
#[test]
fn modernbert_forwards_on_metal_when_available() {
    let dir = synthetic_model_dir();
    let descriptor = synthetic_descriptor("cls");
    let Some(loaded) = CandleBackend::try_load_metal(dir.path(), &descriptor) else {
        return;
    };
    let backend = loaded.expect(
        "ModernBERT must load on Metal — it is pinned to F32 there on purpose \
         (backend::candle::arch_dtype)",
    );
    let text = "the quick brown fox";
    let vectors = backend
        .embed_batch(&[text])
        .expect("ModernBERT must complete a Metal forward pass at F32");
    assert_unit_vector("metal", &vectors[0]);

    // Not just "it did not error": the same text must produce the same vector on both
    // devices. `arch_dtype` pins ModernBERT to F32 on Metal because candle 0.10.2's
    // forward adds an F32-only attention mask to hidden states in the weight dtype;
    // relaxing that to the F16 the other architectures use would either fail the
    // forward outright or silently degrade precision, and only a parity assertion
    // catches the second case. Needs no weights on disk — the model is synthetic.
    let cpu = CandleBackend::load_cpu(dir.path(), &descriptor)
        .expect("the same synthetic model loads on CPU");
    let cpu_vectors = cpu.embed_batch(&[text]).expect("CPU forward");
    let cosine: f32 = vectors[0]
        .iter()
        .zip(&cpu_vectors[0])
        .map(|(a, b)| a * b)
        .sum();
    assert!(
        cosine >= 0.999,
        "CPU and Metal must agree to 0.999 for the same input; got {cosine}. \
         A drop here means the Metal dtype pin changed."
    );
}
