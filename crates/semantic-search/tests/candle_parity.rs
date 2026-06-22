//! Manual macOS lossless-parity gate (NOT a CI gate).
//!
//! The candle cutover claims to be behavior-identical to the retired fastembed/ONNX
//! path: the same `nomic-embed-text-v1.5` weights, the same chunking/pooling, no
//! task prefixes. This test embeds a fixed set of strings through the candle
//! [`CandleBackend`] and asserts each vector is unit-length, that re-embedding the
//! same string is deterministic (cosine ≈ 1.0 against itself), AND — when Metal is
//! available — that the CPU (F32) and Metal (F16) backends agree on the SAME model
//! (cosine ≥ 0.99 per string), proving the two precisions produce the same vector.
//! A full baseline comparison (cosine ≈ 1.0 vs a stored nomic-ONNX baseline) is run
//! by hand on macOS against the real ~250 MB model.
//!
//! It is `#[ignore]` and additionally gated on `MNEMA_SEMANTIC_PARITY_MODEL_DIR`
//! pointing at an installed nomic model dir (CI lacks the weights), so it skips
//! cleanly when unset. The CPU-vs-Metal cross-check is further guarded on Metal
//! actually being acquirable (`try_load_metal` returns `None` on CI / non-macOS /
//! headless / non-`metal` builds), so a CPU-only run still passes. Run it manually
//! with:
//!
//! ```text
//! MNEMA_SEMANTIC_PARITY_MODEL_DIR=~/.mnema/semantic_search_models/local/nomic-embed-text-v1.5 \
//!   cargo test -p semantic-search --features metal -- --ignored candle_nomic_parity
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
#[ignore = "needs the ~250 MB nomic model; set MNEMA_SEMANTIC_PARITY_MODEL_DIR and run on macOS"]
fn candle_nomic_parity() {
    let Ok(model_dir) = std::env::var("MNEMA_SEMANTIC_PARITY_MODEL_DIR") else {
        eprintln!("MNEMA_SEMANTIC_PARITY_MODEL_DIR unset; skipping candle parity gate");
        return;
    };

    let descriptor = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, "nomic-embed-text-v1.5")
        .expect("nomic descriptor resolves");

    // The CPU (F32) backend is the reference: always available, full precision.
    let cpu_backend = CandleBackend::load_cpu(&model_dir, &descriptor)
        .expect("candle backend loads the nomic model on CPU");

    let cpu_vectors = cpu_backend
        .embed_batch(PARITY_STRINGS)
        .expect("candle embeds the parity strings");
    assert_eq!(cpu_vectors.len(), PARITY_STRINGS.len());

    for (text, vector) in PARITY_STRINGS.iter().zip(&cpu_vectors) {
        assert_eq!(
            vector.len(),
            descriptor.dimension,
            "{text}: vector dimension must match the descriptor"
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
    match CandleBackend::try_load_metal(&model_dir, &descriptor) {
        None => eprintln!("Metal unavailable; skipping CPU-vs-Metal precision cross-check"),
        Some(metal_backend) => {
            let metal_backend = metal_backend.expect("candle backend loads the nomic model on Metal");
            let metal_vectors = metal_backend
                .embed_batch(PARITY_STRINGS)
                .expect("candle embeds the parity strings on Metal");
            assert_eq!(metal_vectors.len(), PARITY_STRINGS.len());
            for (i, (cpu, metal)) in cpu_vectors.iter().zip(&metal_vectors).enumerate() {
                let c = cosine(cpu, metal);
                assert!(
                    c >= 0.99,
                    "string {i}: Metal (F16) must agree with CPU (F32) reference (cosine {c})"
                );
            }
        }
    }
}
