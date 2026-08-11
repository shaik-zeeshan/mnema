//! Real-weight smoke check for **every installed Semantic Search Model**.
//!
//! The gap the ModernBERT arm shipped with: all its coverage was synthetic
//! weights written with candle's own tensor naming, so nothing exercised a real
//! HuggingFace checkpoint until the desktop backfill worker did — and it failed
//! with `cannot find tensor model.embeddings.tok_embeddings.weight`. This walks
//! the built-in manifest, loads each model that is actually installed under the
//! app data dir, and embeds a fixed probe set: load → dimension → unit norm →
//! related pair scores above an unrelated pair.
//!
//! ```sh
//! cargo run --release -p semantic-search --features metal --example smoke
//! cargo run --release -p semantic-search --example smoke -- \
//!     --data-dir "$HOME/Library/Application Support/day.mnema"
//! ```
//!
//! ponytail: diagnostic, not a CI gate — the real gate is the env-var-driven
//! `tests/modernbert_parity.rs`. This exists because "is every model loadable at
//! all" is one command, not one env var per model.

use std::path::PathBuf;

use semantic_search::{
    builtin_model_manifest, model_install_dir, semantic_search_models_dir, EmbedKind,
    SemanticSearchEmbedder, CONFIG_FILE_NAME,
};

/// A related pair and a distractor. Every embedding model worth shipping must
/// score the first two above either against the third.
const RELATED_A: &str = "the meeting notes mention the quarterly budget review";
const RELATED_B: &str = "we discussed the budget for this quarter in the meeting";
const UNRELATED: &str = "sourdough starter needs feeding twice a day in summer";

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut data_dir: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => data_dir = args.next().map(PathBuf::from),
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }
    let data_dir = data_dir.unwrap_or_else(|| {
        let home = std::env::var("HOME").expect("HOME");
        PathBuf::from(home).join("Library/Application Support/day.mnema.dev")
    });
    let models_root = semantic_search_models_dir(&data_dir);
    println!("models root: {}", models_root.display());

    let manifest = builtin_model_manifest();
    let (mut checked, mut failed) = (0usize, 0usize);

    for descriptor in &manifest.models {
        let dir = model_install_dir(&models_root, &descriptor.provider, &descriptor.model_id)
            .expect("descriptor ids are manifest-controlled");
        if !dir.join(CONFIG_FILE_NAME).is_file() {
            println!("skip  {:<36} not installed", descriptor.model_id);
            continue;
        }
        checked += 1;
        print!("check {:<36} ", descriptor.model_id);

        let embedder = match SemanticSearchEmbedder::load_from_dir(&dir, descriptor) {
            Ok(embedder) => embedder,
            Err(error) => {
                failed += 1;
                println!("LOAD FAILED: {error}");
                continue;
            }
        };

        let embed = |text: &str, kind| embedder.embed_text(text, kind);
        let (a, b, c) = match (
            embed(RELATED_A, EmbedKind::Document),
            embed(RELATED_B, EmbedKind::Query),
            embed(UNRELATED, EmbedKind::Document),
        ) {
            (Ok(a), Ok(b), Ok(c)) => (a, b, c),
            (a, b, c) => {
                failed += 1;
                let error = [a.err(), b.err(), c.err()]
                    .into_iter()
                    .flatten()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("; ");
                println!("EMBED FAILED: {error}");
                continue;
            }
        };

        let norm = cosine(&a, &a).sqrt();
        let related = cosine(&a, &b);
        let unrelated = cosine(&a, &c);
        let dim_ok = a.len() == embedder.dimension() && a.len() == descriptor.dimension;
        let norm_ok = (norm - 1.0).abs() < 1e-3;
        let rank_ok = related > unrelated;
        let verdict = if dim_ok && norm_ok && rank_ok {
            "ok"
        } else {
            failed += 1;
            "BAD"
        };
        println!(
            "{verdict}  dim={} (want {}) norm={norm:.4} related={related:.4} unrelated={unrelated:.4}",
            a.len(),
            descriptor.dimension,
        );
    }

    println!("\n{checked} installed model(s) checked, {failed} failed");
    if failed > 0 {
        std::process::exit(1);
    }
}
