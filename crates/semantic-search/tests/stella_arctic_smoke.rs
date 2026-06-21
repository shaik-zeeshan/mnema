//! Manual end-to-end functional smoke gates for Stella (`stella_en_400M_v5`) and
//! Arctic (`snowflake-arctic-embed-l-v2.0`) — NOT CI gates.
//!
//! Where `stella_arctic_parity.rs` drives the candle [`CandleBackend`] directly (so
//! it sees the model's NATIVE width — Stella 2048, Arctic 1024), this smoke test
//! goes through the full [`SemanticSearchEmbedder`] via
//! [`SemanticSearchEmbedder::load_from_dir`], the same public entry the desktop app
//! uses. The embedder is the only layer that applies the per-model input prompt
//! ([`EmbedKind::Query`] / [`EmbedKind::Document`]) and the Matryoshka truncation,
//! so it is the layer that produces the model's **STORED** width:
//!
//! - **Stella**: stored == native == **2048** (the `2_Dense_2048` head, no
//!   truncation — `mrl_truncate_dim: None`). The embedder reports 2048.
//! - **Arctic**: stored == **256** (`mrl_truncate_dim: Some(256)`): the embedder
//!   truncates the native 1024-wide backbone vector to its first 256 elements and
//!   L2-renormalizes ABOVE the backend. So while the backend reports 1024 (see the
//!   parity test), the embedder reports — and returns — 256.
//!
//! For each model the test embeds ONE query string ([`EmbedKind::Query`], which
//! applies the model's query prompt) and ONE document string
//! ([`EmbedKind::Document`], which applies the document prompt — `None`/bare for
//! both these models) and asserts each result vector is:
//!   * all-finite (no NaN/Inf — a misload or a bad prompt/truncation pollutes the
//!     vector),
//!   * unit-norm (L2 ≈ 1.0 — the embedder L2-normalizes; for Arctic this also proves
//!     the post-truncation renormalize landed back on the unit sphere), and
//!   * exactly `embedder.dimension()` == `descriptor.dimension` long — i.e. the
//!     STORED width (Stella 2048, Arctic 256), the width storage and query index.
//!
//! Both are `#[ignore]` and gated on the SAME per-model env vars as the parity gate
//! — `MNEMA_STELLA_PARITY_MODEL_DIR` / `MNEMA_ARCTIC_PARITY_MODEL_DIR` — so CI,
//! which lacks the multi-GB weights, skips each cleanly (eprintln + return) when its
//! var is unset. CPU/F32 only (deterministic, no Metal needed). Run them with:
//!
//! ```text
//! MNEMA_STELLA_PARITY_MODEL_DIR=~/.mnema/semantic_search_models/local/stella_en_400M_v5 \
//!   cargo test -p semantic-search -- --ignored stella_smoke
//! MNEMA_ARCTIC_PARITY_MODEL_DIR=~/.mnema/semantic_search_models/local/snowflake-arctic-embed-l-v2.0 \
//!   cargo test -p semantic-search -- --ignored arctic_smoke
//! ```

use semantic_search::{
    resolve_descriptor, EmbedKind, SemanticSearchEmbedder, SEMANTIC_SEARCH_PROVIDER_ID,
};

/// A query-side and a document-side input. The embedder applies the model's
/// per-side input prompt (`EmbedKind::Query` / `EmbedKind::Document`) before the
/// forward; for both Stella and Arctic the document side is bare (`document_prompt:
/// None`), and Stella's query prompt is its retrieval instruction while Arctic's is
/// `"query: "` — either way the output is one finite, unit-norm, stored-width vector.
const QUERY_TEXT: &str = "how does Mnema make my screen searchable";
const DOCUMENT_TEXT: &str = "Mnema records the screen and makes it searchable by meaning, not keywords";

/// Assert one embedder result vector is finite, unit-norm, and the expected STORED
/// width — the shared per-vector checks for both sides of both models.
fn assert_stored_vector(label: &str, vector: &[f32], stored_dimension: usize) {
    // STORED width: the descriptor's `dimension` and `embedder.dimension()`
    // (Stella 2048, Arctic 256). For Arctic this is the MRL-truncated width, NOT
    // the backend's native 1024.
    assert_eq!(
        vector.len(),
        stored_dimension,
        "{label}: vector length must equal the stored dimension {stored_dimension}"
    );
    // All-finite: a misload, a wrong prompt budget, or a botched MRL truncation
    // pollutes the vector with NaN/Inf.
    assert!(
        vector.iter().all(|value| value.is_finite()),
        "{label}: every component must be finite (no NaN/Inf)"
    );
    // Unit-norm (L2 ≈ 1.0): the embedder L2-normalizes; for Arctic this proves the
    // post-truncation renormalize landed back on the unit sphere.
    let norm: f32 = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    assert!(
        (norm - 1.0).abs() < 1e-3,
        "{label}: vector must be L2-normalized (got norm {norm})"
    );
}

#[test]
#[ignore = "needs the ~1.75 GB stella_en_400M_v5 model; set MNEMA_STELLA_PARITY_MODEL_DIR and run manually"]
fn stella_smoke() {
    let Ok(model_dir) = std::env::var("MNEMA_STELLA_PARITY_MODEL_DIR") else {
        eprintln!("MNEMA_STELLA_PARITY_MODEL_DIR unset; skipping Stella embedder smoke gate");
        return;
    };

    let descriptor = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, "stella_en_400M_v5")
        .expect("stella descriptor resolves");

    // Load the FULL embedder (prompt + MRL layers above the backend) — the same
    // public entry the desktop app uses. CPU/F32 is deterministic; no Metal needed.
    let embedder = SemanticSearchEmbedder::load_from_dir(&model_dir, &descriptor)
        .expect("embedder loads the stella model");

    // Stella stores its native 2048-dim vector (no MRL truncation): the embedder's
    // reported dimension is the descriptor's 2048.
    assert_eq!(
        embedder.dimension(),
        descriptor.dimension,
        "embedder must report the descriptor's stored dimension"
    );
    assert_eq!(embedder.dimension(), 2048, "Stella's stored width is 2048");

    let query_vector = embedder
        .embed_text(QUERY_TEXT, EmbedKind::Query)
        .expect("embeds the query string");
    let document_vector = embedder
        .embed_text(DOCUMENT_TEXT, EmbedKind::Document)
        .expect("embeds the document string");

    assert_stored_vector("Stella query", &query_vector, descriptor.dimension);
    assert_stored_vector("Stella document", &document_vector, descriptor.dimension);
}

#[test]
#[ignore = "needs the ~2.3 GB snowflake-arctic-embed-l-v2.0 model; set MNEMA_ARCTIC_PARITY_MODEL_DIR and run manually"]
fn arctic_smoke() {
    let Ok(model_dir) = std::env::var("MNEMA_ARCTIC_PARITY_MODEL_DIR") else {
        eprintln!("MNEMA_ARCTIC_PARITY_MODEL_DIR unset; skipping Arctic embedder smoke gate");
        return;
    };

    let descriptor = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, "snowflake-arctic-embed-l-v2.0")
        .expect("arctic descriptor resolves");

    // Load the FULL embedder (prompt + MRL layers above the backend). This is the
    // layer that truncates Arctic's native 1024-wide backbone vector to the stored
    // 256 and renormalizes — the path the desktop app exercises.
    let embedder = SemanticSearchEmbedder::load_from_dir(&model_dir, &descriptor)
        .expect("embedder loads the arctic model");

    // Arctic STORES the MRL-truncated 256-dim vector even though the backend
    // produces 1024: the embedder reports the descriptor's stored 256, not the
    // backend's native 1024 (which `stella_arctic_parity.rs::arctic_parity` checks).
    assert_eq!(
        embedder.dimension(),
        descriptor.dimension,
        "embedder must report the descriptor's stored dimension"
    );
    assert_eq!(
        embedder.dimension(),
        256,
        "Arctic's stored width is the MRL-truncated 256"
    );

    let query_vector = embedder
        .embed_text(QUERY_TEXT, EmbedKind::Query)
        .expect("embeds the query string");
    let document_vector = embedder
        .embed_text(DOCUMENT_TEXT, EmbedKind::Document)
        .expect("embeds the document string");

    assert_stored_vector("Arctic query", &query_vector, descriptor.dimension);
    assert_stored_vector("Arctic document", &document_vector, descriptor.dimension);
}
