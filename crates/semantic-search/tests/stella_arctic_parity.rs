//! Manual macOS CPU(F32)-vs-Metal(F16) parity gates for the two **Custom-tier**
//! models — Stella (`stella_en_400M_v5`) and Arctic
//! (`snowflake-arctic-embed-l-v2.0`) — NOT CI gates.
//!
//! Sibling of `candle_parity.rs`: same shape, same `#[ignore]` + env-var gating
//! idiom, same CPU(F32)-reference / Metal(F16)-cross-check structure. These two
//! models live here (rather than in `candle_parity.rs`) because each carries a
//! model-specific dimension wrinkle and, for Stella, a relaxed precision tolerance
//! that the nomic gate doesn't need.
//!
//! Both tests drive the candle [`CandleBackend`] DIRECTLY, so the vectors are the
//! model's **native** produced width — the width before the
//! [`SemanticSearchEmbedder`] applies any Matryoshka truncation. That is the load-
//! bearing distinction:
//!
//! - **Stella**: native width == stored width == `descriptor.dimension` == **2048**
//!   (the native `2_Dense_2048` head, no truncation), so this test can assert the
//!   vector length against the descriptor directly.
//! - **Arctic**: native width is the XLM-R backbone hidden size **1024**, but the
//!   descriptor's stored `dimension` is **256** (`mrl_truncate_dim: Some(256)`, the
//!   embedder truncates + renormalizes ABOVE the backend). So driving the backend
//!   directly yields 1024-wide vectors and this test asserts **1024**, NOT
//!   `descriptor.dimension`. The 256-wide stored path is covered by the smoke test
//!   (`stella_arctic_smoke.rs`), which goes through the embedder.
//!
//! Each test embeds a fixed set of strings and asserts each vector is unit-length,
//! that re-embedding the same string is deterministic (cosine ≈ 1.0 against itself),
//! AND — when Metal is acquirable — that the CPU (F32) and Metal (F16) backends
//! agree on the SAME model (per-string cosine above a tolerance), proving the two
//! precisions produce the same vector.
//!
//! **Stella is the F16 precision watch-item.** Its forward runs a mean-pool through
//! a learned dense decoder head (1024 → 2048) on top of a 24-layer transformer;
//! that extra projection plus the deeper stack accumulate more F16 rounding than the
//! shallower-head XLM-R / NomicBert models, so the Metal(F16)-vs-CPU(F32) cosine sits
//! a touch lower. We therefore RELAX Stella's cross-precision tolerance to **≥ 0.98**
//! (vs the ≥ 0.99 the nomic gate and the XLM-R-family models hold). Arctic is plain
//! XLM-R + CLS pooling, so it keeps the **≥ 0.99** tolerance used by the other
//! XLM-R-family models — the MRL truncation that lowers its *stored* width happens
//! above the backend and is not exercised here.
//!
//! Both are `#[ignore]` and additionally gated on a per-model env var pointing at an
//! installed model dir — `MNEMA_STELLA_PARITY_MODEL_DIR` /
//! `MNEMA_ARCTIC_PARITY_MODEL_DIR` — mirroring the nomic gate's
//! `MNEMA_SEMANTIC_PARITY_MODEL_DIR`: CI lacks the multi-GB weights, so each skips
//! cleanly (eprintln + return) when its var is unset. The CPU-vs-Metal cross-check
//! is further guarded on Metal actually being acquirable (`try_load_metal` returns
//! `None` on CI / non-macOS / headless / non-`metal` builds), so a CPU-only run
//! still passes. Run them manually with:
//!
//! ```text
//! MNEMA_STELLA_PARITY_MODEL_DIR=~/.mnema/semantic_search_models/local/stella_en_400M_v5 \
//!   cargo test -p semantic-search --features metal -- --ignored stella_parity
//! MNEMA_ARCTIC_PARITY_MODEL_DIR=~/.mnema/semantic_search_models/local/snowflake-arctic-embed-l-v2.0 \
//!   cargo test -p semantic-search --features metal -- --ignored arctic_parity
//! ```

use semantic_search::{resolve_descriptor, CandleBackend, SemanticSearchBackend, SEMANTIC_SEARCH_PROVIDER_ID};

const PARITY_STRINGS: &[&str] = &[
    "the quick brown fox jumps over the lazy dog",
    "semantic search retrieves by meaning, not keywords",
    "Mnema records the screen and makes it searchable",
    "a short fragment",
];

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (na * nb)
}

#[test]
#[ignore = "needs the ~1.75 GB stella_en_400M_v5 model; set MNEMA_STELLA_PARITY_MODEL_DIR and run on macOS"]
fn stella_parity() {
    let Ok(model_dir) = std::env::var("MNEMA_STELLA_PARITY_MODEL_DIR") else {
        eprintln!("MNEMA_STELLA_PARITY_MODEL_DIR unset; skipping Stella candle parity gate");
        return;
    };

    let descriptor = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, "stella_en_400M_v5")
        .expect("stella descriptor resolves");

    // The CPU (F32) backend is the reference: always available, full precision.
    let cpu_backend = CandleBackend::load_cpu(&model_dir, &descriptor)
        .expect("candle backend loads the stella model on CPU");

    let cpu_vectors = cpu_backend
        .embed_batch(PARITY_STRINGS)
        .expect("candle embeds the parity strings");
    assert_eq!(cpu_vectors.len(), PARITY_STRINGS.len());

    for (text, vector) in PARITY_STRINGS.iter().zip(&cpu_vectors) {
        // Stella's native produced width == its stored width == the descriptor's
        // dimension (2048): the `2_Dense_2048` head IS the output, no MRL
        // truncation, so the backend's vector and `descriptor.dimension` agree.
        assert_eq!(
            vector.len(),
            descriptor.dimension,
            "{text}: Stella vector dimension must be the native 2048 (= descriptor.dimension)"
        );
        assert_eq!(
            vector.len(),
            2048,
            "{text}: Stella's native width is 2048"
        );
        let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "{text}: vector must be L2-normalized (got norm {norm})"
        );
    }

    // Re-embedding the same strings is deterministic: cosine ≈ 1.0 against itself.
    let again = cpu_backend.embed_batch(PARITY_STRINGS).expect("re-embed");
    for (i, (a, b)) in cpu_vectors.iter().zip(&again).enumerate() {
        let c = cosine(a, b);
        assert!(
            c > 0.999,
            "string {i} must re-embed to a near-identical vector (cosine {c})"
        );
    }

    // Cross-precision reference: when Metal is acquirable, the F16-on-Metal vectors
    // must be cosine-close to the F32-on-CPU reference. `try_load_metal` returns
    // `None` on CI / non-macOS / headless / non-`metal` builds, so a CPU-only run
    // skips this assertion and still passes.
    //
    // RELAXED tolerance (≥ 0.98, vs ≥ 0.99 elsewhere): Stella is the F16 precision
    // watch-item. Its 24-layer backbone feeds a learned dense DECODER head
    // (1024 → 2048); that projection plus the deeper stack accumulate more F16
    // rounding than the shallower XLM-R / NomicBert heads, so the Metal(F16) vector
    // diverges slightly more from the CPU(F32) reference. ≥ 0.98 still proves the
    // two precisions agree on the same model without flaking on benign F16 drift.
    match CandleBackend::try_load_metal(&model_dir, &descriptor) {
        None => eprintln!("Metal unavailable; skipping Stella CPU-vs-Metal precision cross-check"),
        Some(metal_backend) => {
            let metal_backend = metal_backend.expect("candle backend loads the stella model on Metal");
            let metal_vectors = metal_backend
                .embed_batch(PARITY_STRINGS)
                .expect("candle embeds the parity strings on Metal");
            assert_eq!(metal_vectors.len(), PARITY_STRINGS.len());
            for (i, (cpu, metal)) in cpu_vectors.iter().zip(&metal_vectors).enumerate() {
                let c = cosine(cpu, metal);
                assert!(
                    c >= 0.98,
                    "string {i}: Stella Metal (F16) must agree with CPU (F32) reference \
                     within the relaxed decoder-head tolerance (cosine {c})"
                );
            }
        }
    }
}

#[test]
#[ignore = "needs the ~2.3 GB snowflake-arctic-embed-l-v2.0 model; set MNEMA_ARCTIC_PARITY_MODEL_DIR and run on macOS"]
fn arctic_parity() {
    let Ok(model_dir) = std::env::var("MNEMA_ARCTIC_PARITY_MODEL_DIR") else {
        eprintln!("MNEMA_ARCTIC_PARITY_MODEL_DIR unset; skipping Arctic candle parity gate");
        return;
    };

    let descriptor = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, "snowflake-arctic-embed-l-v2.0")
        .expect("arctic descriptor resolves");

    // The CPU (F32) backend is the reference: always available, full precision.
    let cpu_backend = CandleBackend::load_cpu(&model_dir, &descriptor)
        .expect("candle backend loads the arctic model on CPU");

    let cpu_vectors = cpu_backend
        .embed_batch(PARITY_STRINGS)
        .expect("candle embeds the parity strings");
    assert_eq!(cpu_vectors.len(), PARITY_STRINGS.len());

    for (text, vector) in PARITY_STRINGS.iter().zip(&cpu_vectors) {
        // Arctic's NATIVE produced width is the XLM-R backbone hidden size (1024),
        // NOT the descriptor's stored `dimension` (256). The 256-dim stored vector
        // is the Matryoshka-truncated + renormalized vector the
        // `SemanticSearchEmbedder` produces ABOVE the backend
        // (`mrl_truncate_dim: Some(256)`); the backend itself reports and returns
        // the full 1024. Driving the backend directly, we therefore assert 1024 —
        // the stored-256 path is covered by `stella_arctic_smoke.rs`.
        assert_eq!(
            vector.len(),
            1024,
            "{text}: Arctic's native backend width is the XLM-R hidden size 1024 \
             (descriptor.dimension is the MRL-truncated 256, applied above the backend)"
        );
        assert_eq!(
            cpu_backend.dimension(),
            1024,
            "{text}: the backend reports the native 1024, not the stored 256"
        );
        let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "{text}: vector must be L2-normalized (got norm {norm})"
        );
    }

    // Re-embedding the same strings is deterministic: cosine ≈ 1.0 against itself.
    let again = cpu_backend.embed_batch(PARITY_STRINGS).expect("re-embed");
    for (i, (a, b)) in cpu_vectors.iter().zip(&again).enumerate() {
        let c = cosine(a, b);
        assert!(
            c > 0.999,
            "string {i} must re-embed to a near-identical vector (cosine {c})"
        );
    }

    // Cross-precision reference: when Metal is acquirable, the F16-on-Metal vectors
    // must be cosine-close to the F32-on-CPU reference, proving the two backends
    // agree. `try_load_metal` returns `None` on CI / non-macOS / headless / non-
    // `metal` builds, so a CPU-only run skips this assertion and still passes.
    //
    // Arctic is plain XLM-R + CLS pooling (no decoder head), so it keeps the
    // ≥ 0.99 tolerance the other XLM-R-family models hold — the F16 drift that
    // forces Stella's relaxed bound does not apply here. The MRL truncation that
    // lowers Arctic's STORED width happens above the backend and is not exercised
    // by this direct-backend parity check.
    match CandleBackend::try_load_metal(&model_dir, &descriptor) {
        None => eprintln!("Metal unavailable; skipping Arctic CPU-vs-Metal precision cross-check"),
        Some(metal_backend) => {
            let metal_backend = metal_backend.expect("candle backend loads the arctic model on Metal");
            let metal_vectors = metal_backend
                .embed_batch(PARITY_STRINGS)
                .expect("candle embeds the parity strings on Metal");
            assert_eq!(metal_vectors.len(), PARITY_STRINGS.len());
            for (i, (cpu, metal)) in cpu_vectors.iter().zip(&metal_vectors).enumerate() {
                let c = cosine(cpu, metal);
                assert!(
                    c >= 0.99,
                    "string {i}: Arctic Metal (F16) must agree with CPU (F32) reference (cosine {c})"
                );
            }
        }
    }
}
