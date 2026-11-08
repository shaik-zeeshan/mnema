use super::*;

/// The HuggingFace `resolve/{hf_revision}` URL for one file in a model repo.
///
/// Pins the **immutable commit SHA** (the descriptor's `hf_revision`) rather than
/// the mutable `main` branch, so an upstream force-push / branch rewrite can never
/// swap the bytes under a pinned-digest verification and every install is
/// reproducible.
pub(super) fn hf_file_url(hf_repo: &str, hf_revision: &str, hf_relative_path: &str) -> String {
    format!("https://huggingface.co/{hf_repo}/resolve/{hf_revision}/{hf_relative_path}")
}

/// How long to wait for the TCP+TLS handshake before giving up. A wedged CDN that
/// never completes the handshake errors here rather than hanging the task (and
/// holding the single download slot) forever. Conservative so a slow-but-live link
/// still connects.
pub(super) const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum idle gap between received body bytes before the read is declared
/// stalled. A half-open TCP / wedged CDN that returns 200 headers and then stops
/// sending body bytes leaves `stream.next()` blocking forever — the per-chunk
/// cancel branch in the receive loop is never reached and the slot stays claimed
/// for the process lifetime (every later download then returns AlreadyRunning).
/// Wrapping each `stream.next()` in a `tokio::time::timeout` of this length resets
/// the clock on every chunk, so a stalled body errors and releases the slot while a
/// live-but-slow multi-GB transfer (which keeps producing chunks) is never falsely
/// aborted. Mirrors `reqwest`'s per-read idle bound via an explicit timeout we
/// control so it composes with the cancel-notify `select!`.
pub(super) const DOWNLOAD_READ_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// The shared `reqwest::Client` for every semantic-search model file fetch, built
/// once and reused (mirrors the single-client policy of the AI runtime's model
/// HTTP). Bare `reqwest::get` builds a fresh default client per call with NO
/// connect timeout and NO redirect constraint — F2/F7. This client adds:
///   - a **connect timeout** (`DOWNLOAD_CONNECT_TIMEOUT`) so a wedged CDN that
///     never finishes the handshake errors instead of hanging;
///   - a **read timeout** (`DOWNLOAD_READ_IDLE_TIMEOUT`) as a backstop idle bound
///     beneath the explicit per-`next()` timeout in the receive loop;
///   - an **HF-only redirect allowlist** (`hf_redirect_policy`) so the default
///     "follow up to 10 redirects to ANY host" can never bounce an unpinned (or
///     even pinned) large file to an arbitrary CDN. HF `resolve/` 30x-redirects to
///     its own LFS CDN; anything off the allowlist is refused.
pub(super) fn download_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
            .read_timeout(DOWNLOAD_READ_IDLE_TIMEOUT)
            .redirect(hf_redirect_policy())
            .build()
            // A client build failure is a programmer error (the rustls backend is
            // always compiled in); fall back to the crate default rather than
            // panicking the download task.
            .unwrap_or_default()
    })
}

/// Whether `host` is on the HuggingFace download allowlist: `huggingface.co`,
/// `hf.co`, or any `*.hf.co` subdomain (HF's LFS CDN resolves under `*.hf.co` /
/// `cdn-lfs*.hf.co`). The match is exact-or-suffix on a dot boundary so
/// `evilhf.co` or `huggingface.co.attacker.test` never pass.
pub(super) fn is_allowlisted_hf_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "huggingface.co"
        || host == "hf.co"
        || host.ends_with(".hf.co")
        || host.ends_with(".huggingface.co")
}

/// Redirect policy constraining every hop to the HF allowlist (F7). The default
/// policy follows up to 10 redirects to ANY host, so an HF `resolve/` 30x could
/// land bytes from an un-allowlisted CDN that the integrity gate never sees for an
/// unpinned file. This refuses any hop whose host is off the allowlist, and caps
/// the chain length, so a redirect loop or an off-host bounce fails the download
/// instead of silently fetching foreign bytes.
pub(super) fn hf_redirect_policy() -> reqwest::redirect::Policy {
    const MAX_HF_REDIRECTS: usize = 10;
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > MAX_HF_REDIRECTS {
            return attempt.error(std::io::Error::other(
                "semantic search model download exceeded the redirect limit",
            ));
        }
        // Resolve the target host to an owned decision BEFORE consuming `attempt`:
        // `attempt.url()` borrows `attempt`, and both `follow()`/`error()` move it.
        let host = attempt.url().host_str().map(str::to_owned);
        match host.as_deref() {
            Some(host) if is_allowlisted_hf_host(host) => attempt.follow(),
            other => attempt.error(std::io::Error::other(format!(
                "semantic search model download refused a redirect to a non-HuggingFace host: {}",
                other.unwrap_or("<no host>")
            ))),
        }
    })
}

pub fn build_download_plan(
    core: &AppCore,
    request: &SemanticSearchModelDownloadRequestDto,
) -> Result<DownloadPlan, ModelDownloadError> {
    // Resolve through the shared resolver (a pure catalog lookup under candle, ADR
    // 0037) so the selected model downloads with the right safetensors file
    // list/layout. An unknown id resolves to None and fails as ModelNotFound below.
    let descriptor = descriptor_for(&request.provider, &request.model_id).ok_or_else(|| {
        ModelDownloadError::ModelNotFound {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
        }
    })?;

    let models_dir = semantic_search_models_dir(&core.paths().config);
    let install_dir = model_install_dir(&models_dir, &request.provider, &request.model_id)?;

    Ok(DownloadPlan {
        provider: request.provider.clone(),
        model_id: request.model_id.clone(),
        hf_repo: descriptor.hf_repo.clone(),
        hf_revision: descriptor.hf_revision.clone(),
        tier: descriptor.tier,
        weights_relative_path: descriptor.expected_layout.weights_relative_path.clone(),
        aux_weights_relative_path: descriptor.expected_layout.aux_weights_relative_path.clone(),
        install_dir,
        files: model_file_specs(&descriptor),
        total_bytes: descriptor.approx_download_bytes,
    })
}

/// Resolve a descriptor for a `{provider}/{model_id}` selection via the shared
/// catalog lookup (no synthesis under candle, ADR 0037: an unknown id is `None`).
/// Both the download plan and the install verification go through this so a model
/// installs under the same layout the picker advertised.
pub(super) fn descriptor_for(provider: &str, model_id: &str) -> Option<SemanticSearchModelDescriptor> {
    resolve_descriptor(provider, model_id)
}

pub fn claim_model_download(
    state: &SemanticSearchModelDownloadState,
    provider: &str,
    model_id: &str,
    cancel_requested: Arc<AtomicBool>,
    cancel_notify: Arc<tokio::sync::Notify>,
) -> Result<(), ModelDownloadError> {
    let mut active = state
        .lock()
        .map_err(|_| ModelDownloadError::AlreadyRunning {
            provider: provider.to_string(),
            model_id: model_id.to_string(),
        })?;
    if let Some(existing) = active.as_ref() {
        return Err(ModelDownloadError::AlreadyRunning {
            provider: existing.provider.clone(),
            model_id: existing.model_id.clone(),
        });
    }
    *active = Some(ActiveSemanticSearchModelDownload {
        provider: provider.to_string(),
        model_id: model_id.to_string(),
        cancel_requested,
        cancel_notify,
    });
    Ok(())
}

pub fn emit_download_progress(
    core: &AppCore,
    progress: &SemanticSearchModelDownloadProgressDto,
) {
    core.host()
        .emit(CoreEvent::SemanticSearchModelDownloadProgress(progress.clone()));
}

/// Best-effort free-disk preflight for a download plan: fail fast when the models
/// volume cannot hold the plan's approximate total bytes (`plan.total_bytes`, the
/// catalog footprint — bge-m3 is ~2.27GB), so a multi-GB download does not fill the
/// disk and error mid-stream.
///
/// "Best-effort" means an inability to MEASURE never blocks the download: if the
/// free-space query itself errors (the path resolves to no existing ancestor, or
/// `fs2::available_space` fails), we log and return `Ok(())` so the download
/// proceeds. Only a *measured* shortfall fails. Uses `fs2::available_space`
/// (already a desktop dependency), which reports the space available to a
/// non-privileged process — the right bound for a user-space download.
/// The free space the disk preflight requires for a plan: the catalog estimate
/// plus 10% headroom (F19). The estimate is a lower bound (the real Content-Length
/// can be larger), so demanding `estimate * 1.1` free turns a modest underestimate
/// into a clean fail-fast instead of a mid-write disk-full error. Saturating so an
/// implausibly huge estimate never overflows.
pub(super) fn required_free_with_headroom(estimate_bytes: u64) -> u64 {
    estimate_bytes.saturating_add(estimate_bytes / 10)
}

pub(super) fn preflight_free_disk_space(plan: &DownloadPlan) -> Result<(), ModelDownloadError> {
    // The install dir does not exist yet; probe the nearest existing ancestor on
    // the same volume so `available_space` has a real path to stat.
    let probe_path = plan
        .install_dir
        .ancestors()
        .find(|ancestor| ancestor.exists());
    let Some(probe_path) = probe_path else {
        log(format!(
            "semantic search model download disk preflight skipped for {}/{}: no existing ancestor of {} to stat",
            plan.provider,
            plan.model_id,
            plan.install_dir.display()
        ));
        return Ok(());
    };
    match fs2::available_space(probe_path) {
        Ok(available_bytes) => {
            // `plan.total_bytes` is the STATIC catalog estimate (`approx_download_bytes`),
            // not the real Content-Length — an underestimate would pass preflight then
            // fill the volume mid-write (F19). Treat it as a lower bound with 10%
            // headroom so a modest underestimate still fails fast here rather than
            // erroring partway through a multi-GB write. (The in-loop overrun guard in
            // `download_file_to` is the second line of defence against the live size
            // exceeding the advertised Content-Length.)
            let needed_bytes = required_free_with_headroom(plan.total_bytes);
            if available_bytes < needed_bytes {
                return Err(ModelDownloadError::InsufficientDiskSpace {
                    provider: plan.provider.clone(),
                    model_id: plan.model_id.clone(),
                    needed_bytes,
                    available_bytes,
                });
            }
            Ok(())
        }
        Err(error) => {
            // Cannot measure — log and proceed rather than blocking the download.
            log_warn(format!(
                "semantic search model download disk preflight could not read free space at {} for {}/{}: {error}; proceeding",
                probe_path.display(),
                plan.provider,
                plan.model_id
            ));
            Ok(())
        }
    }
}

pub async fn download_and_install_model(
    core: &AppCore,
    plan: &DownloadPlan,
    cancel_requested: &AtomicBool,
    cancel_notify: &tokio::sync::Notify,
) -> Result<u64, ModelDownloadTaskError> {
    let descriptor = descriptor_for(&plan.provider, &plan.model_id).ok_or_else(|| {
        ModelDownloadError::ModelNotFound {
            provider: plan.provider.clone(),
            model_id: plan.model_id.clone(),
        }
    })?;
    let models_dir = plan
        .install_dir
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .ok_or_else(|| ModelDownloadError::CreateDir {
            path: plan.install_dir.clone(),
            source: std::io::Error::other("install dir has no models root"),
        })?;

    // Free-disk preflight: bge-m3 alone is ~2.27GB, so fail fast with a clear
    // message rather than filling the volume and erroring mid-stream. Best-effort —
    // if the free-space query itself fails we log and proceed (an inability to
    // MEASURE must never block a download). The check runs against an existing
    // ancestor of the install dir (the install dir does not exist yet).
    if plan.total_bytes > 0 {
        preflight_free_disk_space(plan)?;
    }

    // A fresh install: clear any partial leftovers so a half-download never lingers.
    let _ = std::fs::remove_dir_all(&plan.install_dir);
    std::fs::create_dir_all(&plan.install_dir).map_err(|source| ModelDownloadError::CreateDir {
        path: plan.install_dir.clone(),
        source,
    })?;

    let mut downloaded_total = 0_u64;
    for spec in &plan.files {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        let url = hf_file_url(&plan.hf_repo, &plan.hf_revision, &spec.relative_path);
        // Preserve the repo-relative path on disk. The safetensors layout keeps all
        // three files at the repo root, but joining the relative path keeps this
        // correct for any future model whose weights live in a subdirectory.
        let destination = plan.install_dir.join(&spec.relative_path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ModelDownloadError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        log(format!(
            "semantic search model download fetching {}/{} file '{}' from {url}",
            plan.provider, plan.model_id, spec.relative_path
        ));
        let file_bytes = download_file_to(
            core,
            plan,
            &url,
            &spec.relative_path,
            &destination,
            downloaded_total,
            spec.expected_sha256.is_some(),
            cancel_requested,
            cancel_notify,
        )
        .await?;
        downloaded_total = downloaded_total.saturating_add(file_bytes);

        // Integrity gate (ADR 0036 / finding L5): verify the bytes we just wrote
        // BEFORE the `.installed.json` marker, so a tampered/corrupt file fails the
        // install rather than being silently loaded by candle. A pinned hash that
        // mismatches aborts here (the partial install dir is then removed by the
        // task runner); an unpinned file installs but is recorded as "integrity
        // unverified" so the supply-chain gap is visible.
        match spec.expected_sha256.as_deref() {
            Some(_) => {
                verify_file_checksum(
                    &destination,
                    &spec.relative_path,
                    spec.expected_sha256.as_deref(),
                )?;
                log(format!(
                    "semantic search model download verified '{}' against pinned SHA256 for {}/{}",
                    spec.relative_path, plan.provider, plan.model_id
                ));
            }
            None => {
                // Not an error — just an honest signal that no digest is pinned for
                // this file (every Custom pick, and any guided-tier file not yet
                // sourced). The model still installs.
                log_info(format!(
                    "semantic search model download integrity UNVERIFIED for '{}' ({}/{}): no pinned SHA256 — trusting TLS only",
                    spec.relative_path, plan.provider, plan.model_id
                ));
            }
        }

        log(format!(
            "semantic search model download saved '{}' ({file_bytes} bytes) for {}/{}",
            spec.relative_path, plan.provider, plan.model_id
        ));
    }

    // Verify every required file landed before claiming Installed. This covers all
    // three safetensors-layout files (`model.safetensors`, `config.json`,
    // `tokenizer.json`), so a model missing its weights is never marked Installed.
    let missing: Vec<String> = descriptor
        .expected_layout
        .required_files
        .iter()
        .filter(|file| !plan.install_dir.join(file).is_file())
        .cloned()
        .collect();
    if !missing.is_empty() {
        log_error(format!(
            "semantic search model download incomplete for {}/{}: missing required files {missing:?}",
            plan.provider, plan.model_id
        ));
        return Err(ModelDownloadError::IncompleteLayout(missing).into());
    }

    // Fail-closed install policy (B2): a **guided tier** (English / Multilingual)
    // MUST have verified its weights file against a pinned digest before we mark it
    // Installed. The per-file loop above already ran `verify_file_checksum` and a
    // mismatch aborted; here we additionally refuse to finalize if the guided tier's
    // weights have no pinned digest at all (so a guided tier can never install on the
    // unverified "trust TLS only" path). Custom/unpinned files keep that path — they
    // install with the "integrity unverified" log, trusting the revision pin + TLS.
    guard_guided_tier_weights_pinned(plan)?;

    emit_download_progress(
        core,
        &SemanticSearchModelDownloadProgressDto::new(
            &plan.provider,
            &plan.model_id,
            SemanticSearchModelDownloadStatusDto::Installing,
            downloaded_total,
            Some(plan.total_bytes.max(downloaded_total)),
            Some("finalizing".to_string()),
        ),
    );

    // The marker is written last (and only on a complete layout), so the detector
    // never reports a partial download as Installed.
    write_installed_marker(&models_dir, &plan.provider, &plan.model_id)
        .map_err(ModelDownloadError::Status)?;
    // Return the real accumulated byte total so the Completed event reports what was
    // actually downloaded, not the approximate catalog footprint.
    Ok(downloaded_total)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn download_file_to(
    core: &AppCore,
    plan: &DownloadPlan,
    url: &str,
    relative_path: &str,
    destination: &Path,
    already_downloaded_bytes: u64,
    has_pinned_sha256: bool,
    cancel_requested: &AtomicBool,
    cancel_notify: &tokio::sync::Notify,
) -> Result<u64, ModelDownloadTaskError> {
    // This HTTPS fetch runs over rustls: with fastembed/ort's `native-tls` gone the
    // desktop `reqwest` is built with `rustls-tls` (no default-tls), aligning the
    // downloader with the workspace's other TLS users — sqlx's
    // `runtime-tokio-rustls` (ADR 0037). The shared `download_client` adds a connect
    // timeout, a read-idle timeout, and an HF-only redirect allowlist (F2/F7) — a
    // bare `reqwest::get` had none of these, so a wedged CDN could hang the request
    // forever (holding the single download slot) and a redirect could fetch
    // un-allowlisted bytes the integrity gate never sees.
    let response = download_client().get(url).send().await.map_err(|error| {
        log_error(format!(
            "semantic search model download HTTP request failed for {}/{} at {url}: {error}",
            plan.provider, plan.model_id
        ));
        ModelDownloadError::Http(error)
    })?;
    let status = response.status();
    if !status.is_success() {
        // Non-2xx (404 missing file, 401/403 gated repo, 5xx) — log the status
        // before turning it into the typed error so the failure is never silent.
        log_error(format!(
            "semantic search model download got HTTP {status} for {}/{} at {url}",
            plan.provider, plan.model_id
        ));
    }
    let response = response
        .error_for_status()
        .map_err(ModelDownloadError::Http)?;
    // Prefer the live content-length sum (the catalog size is only approximate).
    let content_length = response.content_length();
    let total_hint = match content_length {
        Some(len) => Some(
            already_downloaded_bytes
                .saturating_add(len)
                .max(plan.total_bytes),
        ),
        None => Some(plan.total_bytes),
    };
    let mut stream = response.bytes_stream();
    let mut output =
        std::fs::File::create(destination).map_err(|source| ModelDownloadError::CreateFile {
            path: destination.to_path_buf(),
            source,
        })?;
    let mut file_downloaded = 0_u64;
    loop {
        // Cancel-first: a Cancel requested between chunks is honoured immediately,
        // even if the next chunk never arrives (a stall would otherwise leave the
        // per-chunk check unreached). The notify wakes the `select!` below the
        // instant Cancel runs.
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        // Wait for the next chunk OR a cancel OR an idle timeout, whichever happens
        // first. Wrapping `stream.next()` in `tokio::time::timeout` bounds the gap
        // between chunks: a half-open TCP / wedged CDN that stops sending body bytes
        // makes `next()` block forever, so without this the per-chunk cancel branch
        // is never reached and the single download slot is held for the process
        // lifetime (every later download returns AlreadyRunning). The timeout resets
        // on each chunk, so a live-but-slow multi-GB transfer is never falsely
        // aborted; only a genuinely stalled body errors and releases the slot. Racing
        // it against `cancel_notify.notified()` (via `futures_util::future::select`,
        // since the desktop `tokio` is built without the `macros` feature so
        // `tokio::select!` is unavailable) makes Cancel work even with zero inbound
        // bytes — the notify resolves first and the future returns immediately.
        let timed_next = std::pin::pin!(tokio::time::timeout(
            DOWNLOAD_READ_IDLE_TIMEOUT,
            stream.next()
        ));
        let cancelled = std::pin::pin!(cancel_notify.notified());
        let chunk = match futures_util::future::select(timed_next, cancelled).await {
            // Cancel won the race (notify resolved before the next chunk / timeout).
            futures_util::future::Either::Right(_) => {
                return Err(ModelDownloadTaskError::Cancelled);
            }
            // The stream ended: the body is fully received.
            futures_util::future::Either::Left((Ok(None), _)) => break,
            futures_util::future::Either::Left((Ok(Some(chunk)), _)) => {
                chunk.map_err(ModelDownloadError::Http)?
            }
            // No chunk arrived within the idle window: a stalled body. Abort so the
            // task returns and the slot is released (the partial install dir is then
            // removed by the task runner).
            futures_util::future::Either::Left((Err(_elapsed), _)) => {
                log_error(format!(
                    "semantic search model download stalled for {}/{} at {url}: no bytes for {}s after {file_downloaded} byte(s)",
                    plan.provider,
                    plan.model_id,
                    DOWNLOAD_READ_IDLE_TIMEOUT.as_secs()
                ));
                return Err(ModelDownloadError::Stalled {
                    relative_path: relative_path.to_string(),
                    idle_secs: DOWNLOAD_READ_IDLE_TIMEOUT.as_secs(),
                }
                .into());
            }
        };
        std::io::Write::write_all(&mut output, &chunk).map_err(|source| {
            ModelDownloadError::WriteFile {
                path: destination.to_path_buf(),
                source,
            }
        })?;
        file_downloaded = file_downloaded.saturating_add(chunk.len() as u64);
        // Abort as soon as the written bytes exceed the advertised Content-Length
        // (F19): a server that under-reports its length then keeps streaming would
        // otherwise fill the volume past the size the preflight budgeted for. Failing
        // here removes the partial install instead of writing to disk-full.
        if let Some(expected_len) = content_length {
            if file_downloaded > expected_len {
                log_error(format!(
                    "semantic search model download overran the advertised length for {}/{} at {url}: received {file_downloaded} of {expected_len} advertised bytes",
                    plan.provider, plan.model_id
                ));
                return Err(ModelDownloadError::ContentLengthOverrun {
                    relative_path: relative_path.to_string(),
                    advertised: expected_len,
                    received: file_downloaded,
                }
                .into());
            }
        }
        // Real per-chunk byte progress (not a CLI bool) on every chunk.
        emit_download_progress(
            core,
            &SemanticSearchModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                SemanticSearchModelDownloadStatusDto::Downloading,
                already_downloaded_bytes.saturating_add(file_downloaded),
                total_hint,
                destination
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
            ),
        );
    }

    // Truncation guard (finding CT3): if the server advertised a Content-Length,
    // the fully-written byte count MUST equal it. A short stream (dropped
    // connection mid-download) otherwise produces a truncated file that passes the
    // is_file() existence check and gets marked Installed — then fails to load as
    // corrupt safetensors. Failing here removes the partial install instead.
    if let Some(expected_len) = content_length {
        if file_downloaded != expected_len {
            log_error(format!(
                "semantic search model download truncated for {}/{} at {url}: wrote {file_downloaded} of {expected_len} advertised bytes",
                plan.provider, plan.model_id
            ));
            return Err(ModelDownloadError::ContentLengthMismatch {
                relative_path: relative_path.to_string(),
                expected: expected_len,
                actual: file_downloaded,
            }
            .into());
        }
    } else if !has_pinned_sha256 {
        // No Content-Length to bound truncation against AND no pinned SHA256 to catch
        // a corrupt/short body (F8): this file is otherwise unguarded. Previously
        // only a 0-byte stream was rejected, so an unpinned file served with no
        // length whose stream dropped after some non-zero bytes slipped through,
        // passed the is_file() check, was marked Installed, then failed as corrupt
        // safetensors at embed time with no self-heal. We now fail CLOSED: an
        // unpinned large file MUST advertise a Content-Length so we can verify the
        // written size; absent one we refuse the install rather than trust an
        // unverifiable, unboundable body. (A file WITH a pinned digest stays exempt —
        // the post-download checksum fully protects it, length or not.)
        log_error(format!(
            "semantic search model download refused for {}/{} at {url}: {file_downloaded} byte(s) received with no Content-Length and no pinned SHA256 — cannot verify the body is complete (fail-closed)",
            plan.provider, plan.model_id
        ));
        if file_downloaded == 0 {
            // Keep the precise "empty download" signal for the zero-byte case.
            return Err(ModelDownloadError::EmptyDownload {
                relative_path: relative_path.to_string(),
            }
            .into());
        }
        return Err(ModelDownloadError::MissingContentLength {
            relative_path: relative_path.to_string(),
        }
        .into());
    }

    Ok(file_downloaded)
}
