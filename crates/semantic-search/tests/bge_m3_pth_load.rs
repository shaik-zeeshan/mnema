//! Manual bge-m3 PyTorch-`.bin` load gate (NOT a CI gate).
//!
//! `BAAI/bge-m3` ships only a PyTorch `pytorch_model.bin` (no `model.safetensors`),
//! so its descriptor points the weights file at `pytorch_model.bin` and the candle
//! backend reads it through the safe pickle path (`VarBuilder::from_pth`) instead of
//! mmaping safetensors. This test proves that chain end-to-end: it resolves the
//! bge-m3 descriptor from the manifest (asserting its weights file IS the `.bin`),
//! loads the [`CandleBackend`] from a real model dir on CPU (F32 — deterministic, no
//! Metal needed), and embeds a couple of strings, asserting each vector is the right
//! dimension (1024), all-finite, and unit-norm — the real proof the pickle weights
//! loaded into the correct tensors and pooled/normalized.
//!
//! Like `candle_parity.rs`, it is `#[ignore]` and additionally gated on
//! `MNEMA_BGE_M3_MODEL_DIR` pointing at an installed bge-m3 model dir (containing
//! `pytorch_model.bin`, `config.json`, `tokenizer.json`); CI lacks the ~2.27 GB
//! weights, so it skips cleanly when the var is unset. Run it manually with:
//!
//! ```text
//! MNEMA_BGE_M3_MODEL_DIR=~/.mnema/semantic_search_models/local/bge-m3 \
//!   cargo test -p semantic-search -- --ignored bge_m3_loads_from_pytorch_bin
//! ```

use semantic_search::{builtin_model_manifest, CandleBackend, SemanticSearchBackend, SemanticSearchModelTier};

/// bge-m3's declared vector dimension (config.json `hidden_size` 1024).
const BGE_M3_DIMENSION: usize = 1024;

const EMBED_STRINGS: &[&str] = &[
    "semantic search retrieves by meaning, not keywords",
    "Mnema records the screen and makes it searchable",
];

#[test]
#[ignore = "needs the ~2.27 GB bge-m3 model; set MNEMA_BGE_M3_MODEL_DIR and run manually"]
fn bge_m3_loads_from_pytorch_bin() {
    let Ok(model_dir) = std::env::var("MNEMA_BGE_M3_MODEL_DIR") else {
        eprintln!("MNEMA_BGE_M3_MODEL_DIR unset; skipping bge-m3 pytorch_model.bin load gate");
        return;
    };

    // Resolve the Custom-tier bge-m3 descriptor from the manifest and assert its
    // weights file is the PyTorch `.bin` (proves FIX 1: the descriptor points
    // bge-m3 at `pytorch_model.bin`, not `model.safetensors`).
    let descriptor = builtin_model_manifest()
        .models
        .into_iter()
        .find(|model| model.tier == SemanticSearchModelTier::Custom && model.model_id == "bge-m3")
        .expect("bge-m3 is the Custom-tier catalog model");
    assert_eq!(
        descriptor.expected_layout.weights_relative_path, "pytorch_model.bin",
        "bge-m3 must declare its PyTorch .bin as the weights file"
    );

    // Load the backend on CPU (F32) via the same public entry the parity test uses.
    // This exercises the new `VarBuilder::from_pth` branch — deterministic, no Metal.
    let backend = CandleBackend::load_cpu(&model_dir, &descriptor)
        .expect("candle backend loads bge-m3 from pytorch_model.bin on CPU");

    let vectors = backend
        .embed_batch(EMBED_STRINGS)
        .expect("candle embeds the bge-m3 strings");
    assert_eq!(vectors.len(), EMBED_STRINGS.len());

    for (text, vector) in EMBED_STRINGS.iter().zip(&vectors) {
        // Right dimension: the pickle weights loaded into the 1024-dim model.
        assert_eq!(
            vector.len(),
            BGE_M3_DIMENSION,
            "{text}: vector dimension must be bge-m3's 1024"
        );
        assert_eq!(
            vector.len(),
            descriptor.dimension,
            "{text}: vector dimension must match the descriptor"
        );
        // All-finite: the real proof the pickle weights loaded into the right
        // tensors and pooled/normalized (a misload pollutes vectors with NaN/Inf).
        assert!(
            vector.iter().all(|value| value.is_finite()),
            "{text}: every component must be finite (no NaN/Inf)"
        );
        // Unit-norm (L2 ≈ 1.0): the backend L2-normalizes, so a correct embedding
        // sits on the unit sphere.
        let norm: f32 = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "{text}: vector must be L2-normalized (got norm {norm})"
        );
    }

    // Different strings produce different vectors, and the same string is
    // deterministic — together a sanity check that the forward pass is real (not a
    // constant) and stable.
    assert_ne!(
        vectors[0], vectors[1],
        "two different strings must produce different vectors"
    );
    let again = backend
        .embed_batch(EMBED_STRINGS)
        .expect("candle re-embeds the bge-m3 strings");
    for (i, (first, second)) in vectors.iter().zip(&again).enumerate() {
        assert_eq!(
            first, second,
            "string {i} must re-embed to an identical vector (deterministic on CPU/F32)"
        );
    }
}
