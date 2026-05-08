#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use ocr::{OcrOutput, OcrProvider, OcrRequest};

const DEFAULT_ITERATIONS: usize = 5;
const DEFAULT_WARMUP_ITERATIONS: usize = 1;

const DEFAULT_IMAGES: &[(&str, &str)] = &[
    (
        "high-quality",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/fixtures/high-quality.jpg"
        ),
    ),
    (
        "low-quality",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/fixtures/low-quality.jpg"
        ),
    ),
];

#[derive(Debug, Clone)]
pub struct BenchmarkArgs {
    pub images: Vec<BenchmarkImage>,
    pub iterations: usize,
    pub warmup_iterations: usize,
    pub models_dir: Option<PathBuf>,
    pub model_path: Option<PathBuf>,
    pub model_id: Option<String>,
    pub language: Option<String>,
    pub provider_options: BTreeMap<String, serde_json::Value>,
    pub expected_texts: BTreeMap<String, String>,
    pub output_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct BenchmarkImage {
    pub label: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
struct BenchmarkError(String);

#[derive(Debug, Clone, Copy)]
struct ResourceSnapshot {
    user_cpu: Duration,
    system_cpu: Duration,
    max_rss_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
struct ResourceDelta {
    user_cpu: Duration,
    system_cpu: Duration,
    max_rss_bytes: u64,
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for BenchmarkError {}

pub fn parse_args(provider_name: &str) -> Result<BenchmarkArgs, Box<dyn Error>> {
    let mut images = Vec::new();
    let mut iterations = env_usize("MNEMA_OCR_BENCH_ITERATIONS").unwrap_or(DEFAULT_ITERATIONS);
    let mut warmup_iterations =
        env_usize("MNEMA_OCR_BENCH_WARMUP").unwrap_or(DEFAULT_WARMUP_ITERATIONS);
    let mut models_dir = std::env::var_os("MNEMA_OCR_MODELS_DIR").map(PathBuf::from);
    let mut model_path = std::env::var_os("MNEMA_OCR_MODEL_PATH").map(PathBuf::from);
    let mut model_id = std::env::var("MNEMA_OCR_MODEL_ID").ok();
    let mut language = std::env::var("MNEMA_OCR_LANGUAGE").ok();
    let mut provider_options = parse_env_options()?;
    let mut expected_texts = BTreeMap::new();
    let mut output_dir = std::env::var_os("MNEMA_OCR_BENCH_OUTPUT_DIR").map(PathBuf::from);

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print_usage(provider_name);
                std::process::exit(0);
            }
            "--image" => {
                let value = required_value("--image", args.next())?;
                images.push(parse_image_arg(&value)?);
            }
            "--iterations" => {
                iterations = parse_positive_usize(
                    "--iterations",
                    &required_value("--iterations", args.next())?,
                )?;
            }
            "--warmup" => {
                warmup_iterations =
                    parse_usize("--warmup", &required_value("--warmup", args.next())?)?;
            }
            "--models-dir" => {
                models_dir = Some(PathBuf::from(required_value("--models-dir", args.next())?));
            }
            "--model-path" => {
                model_path = Some(PathBuf::from(required_value("--model-path", args.next())?));
            }
            "--model-id" => {
                model_id = Some(required_value("--model-id", args.next())?);
            }
            "--language" => {
                language = Some(required_value("--language", args.next())?);
            }
            "--option" => {
                let value = required_value("--option", args.next())?;
                let (key, value) = parse_option_arg(&value)?;
                provider_options.insert(key, value);
            }
            "--expected" => {
                let value = required_value("--expected", args.next())?;
                let (label, text) = parse_expected_arg(&value)?;
                expected_texts.insert(label, text);
            }
            "--output-dir" => {
                output_dir = Some(PathBuf::from(required_value("--output-dir", args.next())?));
            }
            other => {
                return Err(BenchmarkError(format!(
                    "unknown argument `{other}`; run with --help for usage"
                ))
                .into());
            }
        }
    }

    if images.is_empty() {
        images = DEFAULT_IMAGES
            .iter()
            .map(|(label, path)| BenchmarkImage {
                label: (*label).to_string(),
                path: PathBuf::from(path),
            })
            .collect();
    }

    for image in &images {
        if !image.path.is_file() {
            return Err(BenchmarkError(format!(
                "image `{}` does not exist at {}",
                image.label,
                image.path.display()
            ))
            .into());
        }
    }

    Ok(BenchmarkArgs {
        images,
        iterations,
        warmup_iterations,
        models_dir,
        model_path,
        model_id,
        language,
        provider_options,
        expected_texts,
        output_dir,
    })
}

pub async fn run_provider_benchmark<P, F>(
    provider: &P,
    args: &BenchmarkArgs,
    mut request_for_image: F,
) -> Result<(), Box<dyn Error>>
where
    P: OcrProvider,
    F: FnMut(&Path) -> OcrRequest,
{
    println!("provider={}", provider.provider());
    println!(
        "iterations={} warmup={} options={}",
        args.iterations,
        args.warmup_iterations,
        serde_json::to_string(&args.provider_options)?
    );

    for image in &args.images {
        println!("\nimage={} path={}", image.label, image.path.display());

        for warmup_index in 0..args.warmup_iterations {
            let output = provider.recognize(request_for_image(&image.path)).await?;
            println!(
                "warmup {}: chars={} observations={}",
                warmup_index + 1,
                output.text.chars().count(),
                output.structured_payload.observations.len()
            );
        }

        let mut measurements = Vec::with_capacity(args.iterations);
        let mut resources = Vec::with_capacity(args.iterations);
        for iteration_index in 0..args.iterations {
            let resources_before = resource_snapshot();
            let started_at = Instant::now();
            let output = provider.recognize(request_for_image(&image.path)).await?;
            let elapsed = started_at.elapsed();
            let resource_delta = resources_before.and_then(|before| {
                resource_snapshot().map(|after| ResourceDelta {
                    user_cpu: after.user_cpu.saturating_sub(before.user_cpu),
                    system_cpu: after.system_cpu.saturating_sub(before.system_cpu),
                    max_rss_bytes: after.max_rss_bytes,
                })
            });
            if let Some(output_dir) = args.output_dir.as_ref() {
                write_output_text(
                    output_dir,
                    provider.provider(),
                    &image.label,
                    iteration_index + 1,
                    &output.text,
                )?;
            }
            let accuracy = args
                .expected_texts
                .get(&image.label)
                .map(|expected| accuracy_metrics(expected, &output.text));
            print_iteration(
                iteration_index + 1,
                elapsed,
                resource_delta,
                accuracy,
                &output,
            );
            measurements.push(elapsed);
            if let Some(resource_delta) = resource_delta {
                resources.push(resource_delta);
            }
        }

        print_summary(&measurements, &resources);
    }

    Ok(())
}

pub fn require_model_root(
    args: &BenchmarkArgs,
    provider_name: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    args.models_dir.clone().ok_or_else(|| {
        BenchmarkError(format!(
            "{provider_name} needs --models-dir <ocr-models-dir> or MNEMA_OCR_MODELS_DIR; \
             use --model-path <provider-model-dir> only when the example request should override the provider model path"
        ))
        .into()
    })
}

pub fn model_path_option(args: &BenchmarkArgs) -> Option<serde_json::Value> {
    args.model_path
        .as_ref()
        .map(|path| serde_json::Value::String(path.to_string_lossy().into_owned()))
}

pub fn apply_common_request_options(args: &BenchmarkArgs, request: &mut OcrRequest) {
    if let Some(model_id) = args.model_id.as_ref() {
        request.model_id = Some(model_id.clone());
    }
    if let Some(language) = args.language.as_ref() {
        request.language = Some(language.clone());
    }
    for (key, value) in &args.provider_options {
        request.options.insert(key.clone(), value.clone());
    }
}

fn write_output_text(
    output_dir: &Path,
    provider: &str,
    image_label: &str,
    iteration_index: usize,
    text: &str,
) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(output_dir)?;
    let file_name = format!(
        "{}-{}-run-{}.txt",
        safe_file_stem(provider),
        safe_file_stem(image_label),
        iteration_index
    );
    std::fs::write(output_dir.join(file_name), text)?;
    Ok(())
}

fn safe_file_stem(value: &str) -> String {
    let stem = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if stem.is_empty() {
        "output".to_string()
    } else {
        stem
    }
}

fn print_iteration(
    index: usize,
    elapsed: Duration,
    resource_delta: Option<ResourceDelta>,
    accuracy: Option<AccuracyMetrics>,
    output: &OcrOutput,
) {
    let wall_ms = elapsed.as_secs_f64() * 1_000.0;
    let resource_text = resource_delta
        .map(|delta| format_resource_delta(delta, elapsed))
        .unwrap_or_else(|| "resource=unavailable".to_string());
    let accuracy_text = accuracy
        .map(format_accuracy_metrics)
        .unwrap_or_else(|| "accuracy=n/a".to_string());
    println!(
        "run {index}: wall={wall_ms:.2}ms {resource_text} {accuracy_text} chars={} observations={} preview={:?}",
        output.text.chars().count(),
        output.structured_payload.observations.len(),
        text_preview(&output.text)
    );
}

fn print_summary(measurements: &[Duration], resources: &[ResourceDelta]) {
    if measurements.is_empty() {
        println!("summary: no measured iterations");
        return;
    }

    let mut sorted = measurements.to_vec();
    sorted.sort_unstable();

    let total_secs: f64 = measurements.iter().map(Duration::as_secs_f64).sum();
    let mean_ms = total_secs * 1_000.0 / measurements.len() as f64;
    let min_ms = sorted.first().unwrap().as_secs_f64() * 1_000.0;
    let max_ms = sorted.last().unwrap().as_secs_f64() * 1_000.0;
    let p50_ms = percentile_ms(&sorted, 0.50);
    let p95_ms = percentile_ms(&sorted, 0.95);

    if resources.is_empty() {
        println!(
            "summary: mean={mean_ms:.2}ms min={min_ms:.2}ms p50={p50_ms:.2}ms p95={p95_ms:.2}ms max={max_ms:.2}ms resources=unavailable"
        );
        return;
    }

    let mean_cpu_pct = measurements
        .iter()
        .zip(resources.iter())
        .map(|(elapsed, delta)| cpu_percent(*delta, *elapsed))
        .sum::<f64>()
        / resources.len() as f64;
    let max_rss_mb = resources
        .iter()
        .map(|delta| delta.max_rss_bytes)
        .max()
        .unwrap_or_default() as f64
        / (1024.0 * 1024.0);

    println!(
        "summary: mean={mean_ms:.2}ms min={min_ms:.2}ms p50={p50_ms:.2}ms p95={p95_ms:.2}ms max={max_ms:.2}ms mean_cpu={mean_cpu_pct:.1}% max_rss={max_rss_mb:.1}MiB"
    );
}

fn format_resource_delta(delta: ResourceDelta, elapsed: Duration) -> String {
    let user_ms = delta.user_cpu.as_secs_f64() * 1_000.0;
    let system_ms = delta.system_cpu.as_secs_f64() * 1_000.0;
    let cpu_pct = cpu_percent(delta, elapsed);
    let max_rss_mb = delta.max_rss_bytes as f64 / (1024.0 * 1024.0);
    format!("cpu={cpu_pct:.1}% user={user_ms:.2}ms sys={system_ms:.2}ms max_rss={max_rss_mb:.1}MiB")
}

fn cpu_percent(delta: ResourceDelta, elapsed: Duration) -> f64 {
    if elapsed.is_zero() {
        return 0.0;
    }
    (delta.user_cpu + delta.system_cpu).as_secs_f64() / elapsed.as_secs_f64() * 100.0
}

fn percentile_ms(sorted: &[Duration], percentile: f64) -> f64 {
    let index = ((sorted.len().saturating_sub(1)) as f64 * percentile).round() as usize;
    sorted[index].as_secs_f64() * 1_000.0
}

fn text_preview(text: &str) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = flattened.chars().take(120).collect::<String>();
    if flattened.chars().count() > 120 {
        preview.push('…');
    }
    preview
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

fn parse_env_options() -> Result<BTreeMap<String, serde_json::Value>, Box<dyn Error>> {
    let Some(value) = std::env::var("MNEMA_OCR_BENCH_OPTIONS")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(BTreeMap::new());
    };

    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(parse_option_arg)
        .collect()
}

fn parse_image_arg(value: &str) -> Result<BenchmarkImage, Box<dyn Error>> {
    let (label, path) = value
        .split_once('=')
        .map(|(label, path)| (label.to_string(), PathBuf::from(path)))
        .unwrap_or_else(|| {
            let path = PathBuf::from(value);
            let label = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("image")
                .to_string();
            (label, path)
        });

    if label.trim().is_empty() {
        return Err(BenchmarkError("--image label cannot be empty".to_string()).into());
    }

    Ok(BenchmarkImage { label, path })
}

fn parse_expected_arg(value: &str) -> Result<(String, String), Box<dyn Error>> {
    let (label, path) = value.split_once('=').ok_or_else(|| {
        BenchmarkError(format!(
            "expected text `{value}` must use label=path syntax"
        ))
    })?;
    let label = label.trim();
    if label.is_empty() {
        return Err(BenchmarkError("expected text label cannot be empty".to_string()).into());
    }
    let text = std::fs::read_to_string(path.trim()).map_err(|error| {
        BenchmarkError(format!(
            "failed to read expected text for `{label}` from {}: {error}",
            path.trim()
        ))
    })?;
    Ok((label.to_string(), text))
}

fn parse_option_arg(value: &str) -> Result<(String, serde_json::Value), Box<dyn Error>> {
    let (key, raw_value) = value.split_once('=').ok_or_else(|| {
        BenchmarkError(format!(
            "provider option `{value}` must use key=value syntax"
        ))
    })?;
    let key = key.trim();
    if key.is_empty() {
        return Err(BenchmarkError("provider option key cannot be empty".to_string()).into());
    }
    Ok((key.to_string(), parse_option_value(raw_value.trim())))
}

fn parse_option_value(value: &str) -> serde_json::Value {
    serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
}

fn parse_positive_usize(name: &str, value: &str) -> Result<usize, Box<dyn Error>> {
    let parsed = parse_usize(name, value)?;
    if parsed == 0 {
        return Err(BenchmarkError(format!("{name} must be greater than zero")).into());
    }
    Ok(parsed)
}

fn parse_usize(name: &str, value: &str) -> Result<usize, Box<dyn Error>> {
    value.trim().parse::<usize>().map_err(|error| {
        BenchmarkError(format!(
            "failed to parse {name} value `{value}` as usize: {error}"
        ))
        .into()
    })
}

fn required_value(name: &str, value: Option<String>) -> Result<String, Box<dyn Error>> {
    value.ok_or_else(|| BenchmarkError(format!("{name} needs a value")).into())
}

#[cfg(unix)]
fn resource_snapshot() -> Option<ResourceSnapshot> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    Some(ResourceSnapshot {
        user_cpu: timeval_to_duration(usage.ru_utime),
        system_cpu: timeval_to_duration(usage.ru_stime),
        max_rss_bytes: max_rss_bytes(usage.ru_maxrss),
    })
}

#[cfg(unix)]
fn timeval_to_duration(value: libc::timeval) -> Duration {
    Duration::new(
        value.tv_sec.max(0) as u64,
        (value.tv_usec.max(0) as u32) * 1_000,
    )
}

#[cfg(all(unix, target_os = "macos"))]
fn max_rss_bytes(value: libc::c_long) -> u64 {
    value.max(0) as u64
}

#[cfg(all(unix, not(target_os = "macos")))]
fn max_rss_bytes(value: libc::c_long) -> u64 {
    (value.max(0) as u64).saturating_mul(1024)
}

#[cfg(not(unix))]
fn resource_snapshot() -> Option<ResourceSnapshot> {
    None
}

fn accuracy_metrics(expected: &str, actual: &str) -> AccuracyMetrics {
    let expected_normalized = normalize_accuracy_text(expected);
    let actual_normalized = normalize_accuracy_text(actual);
    let distance = levenshtein_distance(&expected_normalized, &actual_normalized);
    let max_chars = expected_normalized
        .chars()
        .count()
        .max(actual_normalized.chars().count());
    let char_similarity = if max_chars == 0 {
        1.0
    } else {
        1.0 - (distance as f64 / max_chars as f64)
    };

    let expected_words = word_counts(&expected_normalized);
    let actual_words = word_counts(&actual_normalized);
    let expected_total: usize = expected_words.values().sum();
    let actual_total: usize = actual_words.values().sum();
    let matched: usize = expected_words
        .iter()
        .map(|(word, expected_count)| {
            actual_words
                .get(word)
                .copied()
                .unwrap_or_default()
                .min(*expected_count)
        })
        .sum();
    let word_recall = ratio(matched, expected_total);
    let word_precision = ratio(matched, actual_total);
    let word_f1 = if word_precision + word_recall == 0.0 {
        0.0
    } else {
        2.0 * word_precision * word_recall / (word_precision + word_recall)
    };

    AccuracyMetrics {
        char_similarity,
        word_precision,
        word_recall,
        word_f1,
    }
}

#[derive(Debug, Clone, Copy)]
struct AccuracyMetrics {
    char_similarity: f64,
    word_precision: f64,
    word_recall: f64,
    word_f1: f64,
}

fn format_accuracy_metrics(metrics: AccuracyMetrics) -> String {
    format!(
        "accuracy=char:{:.1}% word_f1:{:.1}% precision:{:.1}% recall:{:.1}%",
        metrics.char_similarity * 100.0,
        metrics.word_f1 * 100.0,
        metrics.word_precision * 100.0,
        metrics.word_recall * 100.0
    )
}

fn normalize_accuracy_text(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn word_counts(text: &str) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for word in text.split_whitespace() {
        *counts.entry(word).or_default() += 1;
    }
    counts
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution_cost = usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution_cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right_chars.len()]
}

fn print_usage(provider_name: &str) {
    eprintln!(
        "Usage: cargo run -p ocr --example {provider_name} [--features <feature>] -- [options]\n\
\n\
Options:\n\
  --image <label=path|path>     Add an image. Defaults to bundled high/low quality fixtures.\n\
  --iterations <n>              Measured runs per image (default {DEFAULT_ITERATIONS}).\n\
  --warmup <n>                  Warmup runs per image (default {DEFAULT_WARMUP_ITERATIONS}).\n\
  --models-dir <path>           Root OCR models dir containing provider/model folders.\n\
  --model-path <path>           Override provider model path via request option.\n\
  --model-id <id>               Override the request model id.\n\
  --language <language>         Override the request language.\n\
  --option <key=value>          Override/add a provider request option. JSON values are accepted.\n\
  --expected <label=path>        Compare OCR text with expected text for an image label.\n\
  --output-dir <path>            Write measured OCR text outputs for later comparison.\n\
\n\
Environment alternatives: MNEMA_OCR_BENCH_ITERATIONS, MNEMA_OCR_BENCH_WARMUP,\n\
MNEMA_OCR_MODELS_DIR, MNEMA_OCR_MODEL_PATH, MNEMA_OCR_MODEL_ID, MNEMA_OCR_LANGUAGE,\n\
MNEMA_OCR_BENCH_OPTIONS (comma-separated key=value pairs), MNEMA_OCR_BENCH_OUTPUT_DIR."
    );
}
