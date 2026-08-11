//! Manual macOS real-weight parity gate for the **ModernBert** arm — the three
//! Custom-tier English options added in issue #190/#193 — NOT a CI gate.
//!
//! Sibling of `candle_parity.rs` and `stella_arctic_parity.rs`: same `#[ignore]` +
//! env-var gating idiom, same "CPU reference, then cross-check Metal" structure.
//! Where those files diverge per model, this one diverges per *architecture*:
//!
//! **The CPU-vs-Metal cross-check here is not a precision comparison.** Every other
//! parity gate compares F32-on-CPU against F16-on-Metal and picks a cosine tolerance
//! to absorb F16 rounding (≥ 0.99 for the XLM-R family and NomicBert, relaxed to
//! ≥ 0.98 for Stella's decoder head). ModernBERT runs at **F32 on both devices** —
//! candle 0.10.2's `modernbert` forward adds an F32-only attention mask to hidden
//! states in the weight dtype, so F16 cannot complete a forward pass at all (pinned
//! directly, on CPU and in CI, by
//! `candle_modernbert_synthetic::modernbert_f16_weights_are_not_loadable_in_candle_0_10_2`;
//! the rule itself lives in `backend::candle::arch_dtype`). Same weights, same
//! precision, different backend kernels — so the tolerance here is **≥ 0.999**,
//! matching the *determinism* threshold rather than a cross-precision one. Copying
//! the ≥ 0.99 F16 tolerance would silently accept real Metal kernel divergence.
//!
//! If a candle bump ever fixes the mask cast and `arch_dtype` drops its ModernBert
//! special case, this tolerance MUST be relaxed to an F16-appropriate one at the
//! same time — the synthetic test above will fail first and say so.
//!
//! All three models share one backbone and one tokenizer, so they run from a single
//! parameterised body. Each is gated on its own env var pointing at an installed
//! model dir (registered in `turbo.json` `passThroughEnv`, or turbo strips it):
//!
//! ```text
//! MNEMA_GTE_MODERNBERT_PARITY_MODEL_DIR=~/.mnema/semantic_search_models/local/gte-modernbert-base \
//!   cargo test -p semantic-search --features metal -- --ignored gte_modernbert_parity
//! MNEMA_GRANITE_R2_PARITY_MODEL_DIR=~/.mnema/semantic_search_models/local/granite-embedding-english-r2 \
//!   cargo test -p semantic-search --features metal -- --ignored granite_english_r2_parity
//! MNEMA_GRANITE_SMALL_R2_PARITY_MODEL_DIR=~/.mnema/semantic_search_models/local/granite-embedding-small-english-r2 \
//!   cargo test -p semantic-search --features metal -- --ignored granite_small_english_r2_parity
//! ```

use semantic_search::{
    resolve_descriptor, CandleBackend, SemanticSearchBackend, SEMANTIC_SEARCH_PROVIDER_ID,
};

const PARITY_STRINGS: &[&str] = &[
    "the quick brown fox jumps over the lazy dog",
    "semantic search retrieves by meaning, not keywords",
    "Mnema records the screen and makes it searchable",
    "a short fragment",
];

/// CPU and Metal both run ModernBERT at F32 (see the module docs), so their vectors
/// must agree to determinism precision, not merely to an F16 tolerance.
const SAME_PRECISION_TOLERANCE: f32 = 0.999;

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (na * nb)
}

/// The shared body: load `model_id` from the dir named by `env_var`, assert the CPU
/// vectors are the right width / unit-length / deterministic, then cross-check Metal
/// when one is acquirable. Skips cleanly (eprintln + return) when the var is unset.
fn run_parity(env_var: &str, model_id: &str) {
    let Ok(model_dir) = std::env::var(env_var) else {
        eprintln!("{env_var} unset; skipping the {model_id} candle parity gate");
        return;
    };

    let descriptor = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, model_id)
        .unwrap_or_else(|| panic!("{model_id} descriptor resolves"));

    let cpu_backend = CandleBackend::load_cpu(&model_dir, &descriptor)
        .unwrap_or_else(|error| panic!("{model_id} loads on CPU: {error}"));

    // ModernBERT stores the backbone hidden state directly — no dense head, no MRL
    // truncation — so the backend's native width IS `descriptor.dimension` and the
    // two can be asserted against each other (unlike Stella and Arctic).
    assert_eq!(
        cpu_backend.dimension(),
        descriptor.dimension,
        "{model_id}: ModernBERT's native width is the backbone hidden size, which is \
         also the stored dimension (no dense head, no MRL truncation)"
    );

    let cpu_vectors = cpu_backend
        .embed_batch(PARITY_STRINGS)
        .unwrap_or_else(|error| panic!("{model_id} embeds the parity strings: {error}"));
    assert_eq!(cpu_vectors.len(), PARITY_STRINGS.len());

    for (text, vector) in PARITY_STRINGS.iter().zip(&cpu_vectors) {
        assert_eq!(
            vector.len(),
            descriptor.dimension,
            "{model_id} / {text}: vector width must equal descriptor.dimension"
        );
        let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "{model_id} / {text}: vector must be L2-normalized (got norm {norm})"
        );
    }

    // Distinct inputs must yield distinct vectors — a CLS pool reading a degenerate
    // position, or a mask bug collapsing every sequence, would show up here as
    // everything embedding to the same point.
    let distinct = cosine(&cpu_vectors[0], &cpu_vectors[1]);
    assert!(
        distinct < 0.99,
        "{model_id}: two unrelated sentences must not embed to the same vector \
         (cosine {distinct}) — a collapsed pool or a broken attention mask"
    );

    // Re-embedding is deterministic.
    let again = cpu_backend
        .embed_batch(PARITY_STRINGS)
        .expect("re-embed on CPU");
    for (i, (a, b)) in cpu_vectors.iter().zip(&again).enumerate() {
        let c = cosine(a, b);
        assert!(
            c > SAME_PRECISION_TOLERANCE,
            "{model_id}: string {i} must re-embed to a near-identical vector (cosine {c})"
        );
    }

    // Metal cross-check. `try_load_metal` returns `None` on CI / non-macOS /
    // headless / non-`metal` builds, so a CPU-only run skips it and still passes.
    // Both sides are F32 here, so the bar is determinism, not an F16 tolerance.
    match CandleBackend::try_load_metal(&model_dir, &descriptor) {
        None => eprintln!("Metal unavailable; skipping the {model_id} CPU-vs-Metal cross-check"),
        Some(metal_backend) => {
            let metal_backend = metal_backend.unwrap_or_else(|error| {
                panic!(
                    "{model_id} must load on Metal — ModernBERT is pinned to F32 there on \
                     purpose (backend::candle::arch_dtype); an F16 load would fail its \
                     forward pass: {error}"
                )
            });
            let metal_vectors = metal_backend
                .embed_batch(PARITY_STRINGS)
                .unwrap_or_else(|error| {
                    panic!("{model_id} must complete a Metal forward pass at F32: {error}")
                });
            assert_eq!(metal_vectors.len(), PARITY_STRINGS.len());
            for (i, (cpu, metal)) in cpu_vectors.iter().zip(&metal_vectors).enumerate() {
                let c = cosine(cpu, metal);
                assert!(
                    c >= SAME_PRECISION_TOLERANCE,
                    "{model_id}: string {i} — CPU and Metal both run this architecture at \
                     F32, so they must agree to determinism precision (cosine {c}). If a \
                     candle bump has made ModernBERT F16-capable and arch_dtype no longer \
                     pins F32, relax this tolerance deliberately instead of lowering it to \
                     make the test pass"
                );
            }
        }
    }
}

#[test]
#[ignore = "needs the ~302 MB gte-modernbert-base model; set MNEMA_GTE_MODERNBERT_PARITY_MODEL_DIR"]
fn gte_modernbert_parity() {
    run_parity("MNEMA_GTE_MODERNBERT_PARITY_MODEL_DIR", "gte-modernbert-base");
}

#[test]
#[ignore = "needs the ~302 MB granite-embedding-english-r2 model; set MNEMA_GRANITE_R2_PARITY_MODEL_DIR"]
fn granite_english_r2_parity() {
    run_parity(
        "MNEMA_GRANITE_R2_PARITY_MODEL_DIR",
        "granite-embedding-english-r2",
    );
}

#[test]
#[ignore = "needs the ~99 MB granite-embedding-small-english-r2 model; set MNEMA_GRANITE_SMALL_R2_PARITY_MODEL_DIR"]
fn granite_small_english_r2_parity() {
    run_parity(
        "MNEMA_GRANITE_SMALL_R2_PARITY_MODEL_DIR",
        "granite-embedding-small-english-r2",
    );
}
