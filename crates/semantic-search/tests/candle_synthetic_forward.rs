//! CPU-only synthetic forward-pass smoke gate — RUNS IN CI, no network, no weights.
//!
//! Every other candle test in this crate (`candle_parity`, `stella_arctic_*`,
//! `bge_m3_pth_load`) is `#[ignore]` and env-gated on a multi-GB model download, so
//! the candle forward path itself has had ZERO automated coverage. This test closes
//! that gap: it mints a TINY *synthetic* model on disk in a `tempdir` — a random
//! safetensors whose tensor-name layout matches exactly what `backend::candle`'s
//! load path feeds candle, a minimal `config.json`, and a hand-written
//! `tokenizer.json` — then loads it through the real public [`CandleBackend`] on the
//! CPU and runs a forward pass.
//!
//! It covers the **XLM-RoBERTa** dispatch arm specifically, because that is the arm
//! the F10 finding is about: candle's `XLMRobertaEmbeddings` derives position ids
//! from `input_ids.ne(pad_token_id)` (pad id 1, not 0, for the e5 / bge-m3 / Arctic
//! family), so a literal-0 pad slot would be read as a real `<s>` token. The forward
//! pass here exercises: the architecture dispatch + load, the U8 attention-mask
//! handling, Mean and CLS pooling, and L2-normalization. It asserts:
//!   * output shape is `(batch, hidden_size)`,
//!   * each output vector is L2-normalized (norm ≈ 1.0),
//!   * pooling actually ran (vectors are finite and non-degenerate),
//!   * embedding a short string ALONE vs in a batch alongside a long string — which
//!     forces the short row to be padded — yields the same vector (the forward path
//!     is order/batch-stable), and
//!   * **the backend resolves the model's real pad id from the tokenizer** (F10): the
//!     synthetic tokenizer declares `pad_id = 1` and the backend must read 1, not
//!     fall back to a literal 0.
//!
//! Note on the pad-id guard: the embed-output parity check above (short solo vs
//! short-in-batch) is NOT a pad-id regression guard, and was wrongly documented as
//! one. `forward_batch` always RIGHT-pads (it appends pad slots to the tail), and
//! candle's `XLMRobertaEmbeddings` derives position ids via a LEFT-to-right
//! `cumsum(ne(pad_id))`: trailing pad slots never shift the real (leading) tokens'
//! position ids, and the additive `-inf` attention mask + pooling exclude the pad
//! slots entirely — so a wrong pad id on trailing slots is provably inert in the
//! pooled output. Forcing the pad id back to a literal 0 leaves both parity checks
//! green. The pad-id fix is therefore guarded DIRECTLY, by asserting
//! `backend.resolved_pad_id()` — the one place a revert to literal-0 is observable.
//!
//! NomicBert is intentionally NOT synthesized here: its block uses rotary embeddings
//! + a fused-QKV SwiGLU stack whose random-weight forward is far more fiddle to make
//! numerically meaningful, and the F10 finding it guards is XLM-RoBERTa-specific.
//! See the notes in the PR: the residual gap is automated CPU coverage of the
//! NomicBert and Stella arms (still only covered by the env-gated weight tests).
//!
//! The weights are random, so the *values* are meaningless — this is a structural /
//! invariant test (shape, norm, pad-invariance), never a quality check.

use std::collections::HashMap;
use std::path::Path;

use candle_core::{DType, Device, Tensor};
use semantic_search::{CandleBackend, SemanticSearchBackend, SemanticSearchModelDescriptor};

/// A tiny XLM-RoBERTa config — small enough to mint random weights fast, large
/// enough to exercise every load key (multiple layers, multi-head attention).
const VOCAB_SIZE: usize = 32;
const HIDDEN_SIZE: usize = 16;
const NUM_HEADS: usize = 4;
const NUM_LAYERS: usize = 2;
const INTERMEDIATE_SIZE: usize = 32;
const TYPE_VOCAB_SIZE: usize = 1;
const MAX_POSITION_EMBEDDINGS: usize = 64;
/// XLM-RoBERTa declares `pad_token_id = 1` (id 0 is `<s>`). The whole point of F10.
const PAD_TOKEN_ID: u32 = 1;

/// Random F32 tensor of the given shape on CPU, the on-disk precision the loader
/// casts from. `randn` keeps the layer-norm / linear weights non-degenerate.
fn randn(shape: &[usize]) -> Tensor {
    Tensor::randn(0f32, 1f32, shape, &Device::Cpu).expect("randn tensor")
}

/// Insert a `with_tracing::linear` pair (`{prefix}.weight` [out,in] + `{prefix}.bias`
/// [out]) — candle's `linear()` helper, used by every XLM-RoBERTa dense/qkv/output.
fn insert_linear(t: &mut HashMap<String, Tensor>, prefix: &str, in_dim: usize, out_dim: usize) {
    t.insert(format!("{prefix}.weight"), randn(&[out_dim, in_dim]));
    t.insert(format!("{prefix}.bias"), randn(&[out_dim]));
}

/// Insert a `candle_nn::layer_norm` pair (`{prefix}.weight` + `{prefix}.bias`, both
/// [hidden]). Weight near 1.0 / bias near 0.0 keeps the norm well-conditioned.
fn insert_layer_norm(t: &mut HashMap<String, Tensor>, prefix: &str) {
    t.insert(
        format!("{prefix}.weight"),
        Tensor::ones(HIDDEN_SIZE, DType::F32, &Device::Cpu).expect("ln weight"),
    );
    t.insert(
        format!("{prefix}.bias"),
        Tensor::zeros(HIDDEN_SIZE, DType::F32, &Device::Cpu).expect("ln bias"),
    );
}

/// Build the synthetic `model.safetensors` for the XLM-RoBERTa VarBuilder keys the
/// `candle_transformers::models::xlm_roberta` load path requests (mirrors that
/// module exactly: `embeddings.*`, `encoder.layer.{i}.*`).
fn write_xlm_roberta_safetensors(path: &Path) {
    let mut t: HashMap<String, Tensor> = HashMap::new();

    // embeddings.* — word / position / token-type embeddings + the post-embed
    // LayerNorm. `embedding(num, dim, vb.pp(name))` reads `{name}.weight` [num, dim].
    t.insert(
        "embeddings.word_embeddings.weight".into(),
        randn(&[VOCAB_SIZE, HIDDEN_SIZE]),
    );
    t.insert(
        "embeddings.position_embeddings.weight".into(),
        randn(&[MAX_POSITION_EMBEDDINGS, HIDDEN_SIZE]),
    );
    t.insert(
        "embeddings.token_type_embeddings.weight".into(),
        randn(&[TYPE_VOCAB_SIZE, HIDDEN_SIZE]),
    );
    insert_layer_norm(&mut t, "embeddings.LayerNorm");

    // encoder.layer.{i}.* — attention (self q/k/v + output dense+LN), intermediate,
    // output (dense+LN). The exact prefixes the xlm_roberta module's `vb.pp(..)`
    // chain produces.
    for i in 0..NUM_LAYERS {
        let l = format!("encoder.layer.{i}");
        insert_linear(&mut t, &format!("{l}.attention.self.query"), HIDDEN_SIZE, HIDDEN_SIZE);
        insert_linear(&mut t, &format!("{l}.attention.self.key"), HIDDEN_SIZE, HIDDEN_SIZE);
        insert_linear(&mut t, &format!("{l}.attention.self.value"), HIDDEN_SIZE, HIDDEN_SIZE);
        insert_linear(&mut t, &format!("{l}.attention.output.dense"), HIDDEN_SIZE, HIDDEN_SIZE);
        insert_layer_norm(&mut t, &format!("{l}.attention.output.LayerNorm"));
        insert_linear(&mut t, &format!("{l}.intermediate.dense"), HIDDEN_SIZE, INTERMEDIATE_SIZE);
        insert_linear(&mut t, &format!("{l}.output.dense"), INTERMEDIATE_SIZE, HIDDEN_SIZE);
        insert_layer_norm(&mut t, &format!("{l}.output.LayerNorm"));
    }

    candle_core::safetensors::save(&t, path).expect("save synthetic safetensors");
}

/// Write the minimal `config.json` the candle `xlm_roberta::Config` deserializes.
fn write_config(path: &Path) {
    let config = serde_json::json!({
        "hidden_size": HIDDEN_SIZE,
        "layer_norm_eps": 1e-5,
        "attention_probs_dropout_prob": 0.0,
        "hidden_dropout_prob": 0.0,
        "num_attention_heads": NUM_HEADS,
        "position_embedding_type": "absolute",
        "intermediate_size": INTERMEDIATE_SIZE,
        "hidden_act": "gelu",
        "num_hidden_layers": NUM_LAYERS,
        "vocab_size": VOCAB_SIZE,
        "max_position_embeddings": MAX_POSITION_EMBEDDINGS,
        "type_vocab_size": TYPE_VOCAB_SIZE,
        "pad_token_id": PAD_TOKEN_ID,
    });
    std::fs::write(path, serde_json::to_vec_pretty(&config).expect("serialize config"))
        .expect("write config.json");
}

/// Write a minimal WordLevel `tokenizer.json` whose **padding config declares
/// `pad_id = PAD_TOKEN_ID`** — so the backend reads the real pad id from the
/// tokenizer (F10) rather than defaulting to 0. A whitespace pre-tokenizer maps each
/// space-separated word to its vocab id; the vocab assigns small distinct ids so a
/// short text and a long text produce different real tokens. No post-processor, so
/// `encode_batch(.., true)` adds no extra special tokens (irrelevant to the pad-id
/// and pooling invariants under test).
fn write_tokenizer(path: &Path) {
    // Reserve ids 0/1 for <s>/<pad> (XLM-RoBERTa convention: id 0 is <s>, id 1 is
    // <pad>) so a literal-0 pad would alias the real <s> token — exactly the F10
    // hazard. Content words start at id 2.
    let tokenizer = serde_json::json!({
        "version": "1.0",
        "truncation": null,
        "padding": {
            "strategy": "BatchLongest",
            "direction": "Right",
            "pad_to_multiple_of": null,
            "pad_id": PAD_TOKEN_ID,
            "pad_type_id": 0,
            "pad_token": "<pad>"
        },
        "added_tokens": [],
        "normalizer": null,
        "pre_tokenizer": { "type": "WhitespaceSplit" },
        "post_processor": null,
        "decoder": null,
        "model": {
            "type": "WordLevel",
            "vocab": {
                "<s>": 0,
                "<pad>": 1,
                "<unk>": 2,
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
            "unk_token": "<unk>"
        }
    });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&tokenizer).expect("serialize tokenizer"),
    )
    .expect("write tokenizer.json");
}

/// A synthetic XLM-RoBERTa descriptor pointing at the on-disk model. Deserialized
/// from JSON (every descriptor field is `pub` + `Deserialize`) so the test does not
/// hand-construct the large struct. `pooling` is parameterised so the same model dir
/// covers both Mean and CLS pooling.
fn synthetic_descriptor(pooling: &str) -> SemanticSearchModelDescriptor {
    let json = serde_json::json!({
        "provider": "local",
        "modelId": "synthetic-xlm-roberta",
        "displayName": "Synthetic XLM-RoBERTa (test)",
        "description": "In-test synthetic model for the CPU forward smoke gate.",
        "tier": "custom",
        "architecture": "xlm_roberta",
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

/// Mint the full synthetic model dir (safetensors + config + tokenizer) under a
/// fresh tempdir and return it; the dir lives until the returned guard drops.
fn synthetic_model_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    write_xlm_roberta_safetensors(&dir.path().join("model.safetensors"));
    write_config(&dir.path().join("config.json"));
    write_tokenizer(&dir.path().join("tokenizer.json"));
    dir
}

fn l2_norm(vector: &[f32]) -> f32 {
    vector.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    dot / (l2_norm(a) * l2_norm(b))
}

/// Shape + norm + finiteness of a single output vector — the structural invariants
/// every forward result must satisfy regardless of pooling.
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

/// Load the synthetic XLM-RoBERTa on CPU and run the forward path for one pooling
/// strategy: assert shape, normalization, and pad-invariance.
fn run_pooling(pooling: &str) {
    let dir = synthetic_model_dir();
    let descriptor = synthetic_descriptor(pooling);

    let backend =
        CandleBackend::load_cpu(dir.path(), &descriptor).expect("synthetic backend loads on CPU");
    assert_eq!(
        backend.dimension(),
        HIDDEN_SIZE,
        "backend reports the XLM-RoBERTa backbone hidden_size as its native width"
    );

    // F10 regression guard (DIRECT): the synthetic tokenizer's padding config declares
    // `pad_id = PAD_TOKEN_ID` (1, the XLM-RoBERTa convention), and the backend MUST
    // read it rather than fall back to a literal 0. This is the ONLY observable point
    // of the pad-id fix: under right padding the wrong pad id is inert in the pooled
    // output (see the module docs), so the embed-output checks below cannot catch a
    // revert — this assert is what trips if the backend reverts to a hardcoded 0.
    assert_eq!(
        backend.resolved_pad_id(),
        PAD_TOKEN_ID,
        "backend must resolve the model's real pad id from the tokenizer's padding \
         config (XLM-RoBERTa declares 1; id 0 is <s>); a literal-0 fallback here is \
         the F10 regression"
    );

    // A short text and a long text. Embedding them together forces the SHORT row to
    // be right-padded to the long row's length — the padded slots that F10 is about.
    let short = "short text";
    let long = "the quick brown fox jumps over the lazy dog";

    let batch = backend
        .embed_batch(&[short, long])
        .expect("embeds the batch on CPU");
    assert_eq!(batch.len(), 2, "one vector per input, in order");
    assert_unit_vector(&format!("{pooling} short(batched)"), &batch[0]);
    assert_unit_vector(&format!("{pooling} long(batched)"), &batch[1]);

    // Embed the SHORT text alone (no padding) and compare to its batched (padded)
    // vector. The forward path must be batch/order-stable: padding a row to match a
    // longer one in the same batch must not change its pooled output. (This is a
    // forward-correctness check — masking + pooling exclude the padded slots — NOT the
    // pad-id guard; that is `resolved_pad_id()` above. Under right padding a wrong pad
    // id is inert here, so this check stays green either way; see the module docs.)
    let solo = backend
        .embed_batch(&[short])
        .expect("embeds the short text alone");
    assert_eq!(solo.len(), 1);
    assert_unit_vector(&format!("{pooling} short(solo)"), &solo[0]);

    let c = cosine(&batch[0], &solo[0]);
    assert!(
        c > 0.9999,
        "{pooling}: padding a short row to a longer batch sibling must NOT perturb \
         its pooled output — short text batched-with-padding vs solo must match \
         (cosine {c}); the attention mask + pooling must exclude the padded slots"
    );

    // The two distinct inputs must not collapse to the same vector — proves pooling
    // and the forward actually ran (not a constant/degenerate output).
    let distinct = cosine(&batch[0], &batch[1]);
    assert!(
        distinct < 0.9999,
        "{pooling}: distinct inputs must yield distinct vectors (cosine {distinct})"
    );
}

/// XLM-RoBERTa, Mean pooling (the e5 / Arctic family path): exercises forward
/// dispatch, the U8 attention mask, mean-pool over the mask, L2-normalize, and the
/// F10 pad-invariance guard. CPU-only, no network.
#[test]
fn synthetic_xlm_roberta_mean_pool_cpu_forward() {
    run_pooling("mean");
}

/// XLM-RoBERTa, CLS pooling (the bge-m3 path): same forward dispatch + U8 mask, but
/// takes the row-0 (`[CLS]`/`<s>`) hidden state instead of mean-pooling. Confirms the
/// pad-invariance guard holds for the CLS pool too. CPU-only, no network.
#[test]
fn synthetic_xlm_roberta_cls_pool_cpu_forward() {
    run_pooling("cls");
}
