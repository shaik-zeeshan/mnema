use super::*;

/// The list of files to download for a model, as **repo-relative paths**.
///
/// **Single file-list authority** (ADR 0036 deepening #4): download and
/// completeness now read the SAME list — `descriptor.expected_layout.required_files`
/// — so a model can never download "successfully" yet report broken. That list is
/// the hand-coded safetensors layout (ADR 0037): the three repo-root files
/// `model.safetensors`, `config.json`, `tokenizer.json`. The old ONNX graph +
/// external-data siblings (e.g. bge-m3's `onnx/model.onnx_data`) are gone, so
/// there is no second derivation here to drift out of sync.
///
/// Each path is zipped with its pinned SHA256 (when known) for the integrity gate.
pub(super) fn model_file_specs(descriptor: &SemanticSearchModelDescriptor) -> Vec<ModelFileSpec> {
    descriptor
        .expected_layout
        .required_files
        .iter()
        .map(|relative_path| ModelFileSpec {
            expected_sha256: pinned_file_sha256(&descriptor.hf_repo, relative_path)
                .map(str::to_owned),
            relative_path: relative_path.clone(),
        })
        .collect()
}

/// The pinned SHA256 for one repo-relative file of a **guided-tier** model, when
/// known. Returns `None` for any file we have not yet sourced a real hash for —
/// including every **Custom** (user-picked) model, which has no pinned digest by
/// definition. A `None` here is a deliberate "integrity unverified" signal, not a
/// silent trust: [`download_and_install_model`] logs it and `verify_file_checksum`
/// skips verification for it.
///
/// Mirrors the other model downloaders, which pin per-file SHA256 in their
/// manifests (`ModelArtifactFile.sha256`). The semantic-search downloader fetches
/// directly from each model's HuggingFace repo at the pinned revision
/// (`…/resolve/{hf_revision}/<path>`), so the hashes below are keyed by
/// `(hf_repo, repo-relative path)` and verified against the LFS-published digest.
///
/// Integrity is now **fail-CLOSED for the two guided tiers**: the real LFS sha256
/// of each weights file is pinned below for `nomic-embed-text-v1.5` (English) and
/// `multilingual-e5-small` (Multilingual), so a tampered/truncated weights file
/// fails verification and never installs (see also the guided-tier gate in
/// [`download_and_install_model`], which refuses to mark a guided tier Installed
/// unless its weights matched a pinned digest). The **Custom** tier `BAAI/bge-m3`
/// ships only a PyTorch `pytorch_model.bin` (no `model.safetensors`); that `.bin`
/// is its declared weights file, so it now ALSO carries a pinned digest below —
/// Custom stays exempt from the guided-tier MUST-pin gate, but a present pin makes
/// its install fail-closed too. Any file still without a digest falls through to
/// the logged "integrity unverified" path, trusting the revision pin + TLS.
pub(super) fn pinned_file_sha256(hf_repo: &str, relative_path: &str) -> Option<&'static str> {
    let pinned: &[(&str, &str)] = match hf_repo {
        // English tier — pinned sha256 of every required guided-tier file: the
        // safetensors weights plus the candle-loaded `config.json` and
        // `tokenizer.json`, so the whole guided-tier install is integrity-verified
        // (not just the weights). Hashes computed from the pinned revision.
        "nomic-ai/nomic-embed-text-v1.5" => &[
            (
                "model.safetensors",
                "9e7d262b1fe5ea350782829496efa831901b77486bbde1cea54a4c822d010d5c",
            ),
            (
                "config.json",
                "9ab00bd92cee80a569f708140b7b6c1661a65891ff3765b1519e181ba2f2c92b",
            ),
            (
                "tokenizer.json",
                "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
            ),
        ],
        // Multilingual tier — pinned sha256 of every required guided-tier file:
        // weights + `config.json` + `tokenizer.json`. Hashes computed from the
        // pinned revision.
        "intfloat/multilingual-e5-small" => &[
            (
                "model.safetensors",
                "1a55775f53449dac10a2bcbc312469fac40b96d53198c407081a831f81c98477",
            ),
            (
                "config.json",
                "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959",
            ),
            (
                "tokenizer.json",
                "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
            ),
        ],
        // Custom tier — `BAAI/bge-m3` ships only `pytorch_model.bin` (no
        // `model.safetensors`), and that `.bin` is now the descriptor's weights
        // file, so we DO have a pinnable weights digest: the LFS sha256 of the
        // PyTorch checkpoint at the pinned revision. Custom/bge-m3 stays exempt from
        // the guided-tier MUST-pin gate, but pinning here makes its install
        // fail-closed too (a tampered/truncated `.bin` fails verification and never
        // installs).
        "BAAI/bge-m3" => &[(
            "pytorch_model.bin",
            "b5e0ce3470abf5ef3831aa1bd5553b486803e83251590ab7ff35a117cf6aad38",
        )],
        // Custom tier — `NovaSearch/stella_en_400M_v5` loads a base backbone plus a
        // separate dense projection head (`2_Dense_2048/model.safetensors`). Both
        // are multi-GB-scale LFS files; pinning BOTH closes F7's "integrity
        // UNVERIFIED — trusting TLS only" branch, so a redirect to an un-allowlisted
        // CDN serving swapped bytes fails verification and never installs. The
        // digests are the LFS `oid sha256` published at the pinned revision
        // (`ffeb2b7ee715c226d4ffe5e4619f7dbb48624c20`).
        "NovaSearch/stella_en_400M_v5" => &[
            (
                "model.safetensors",
                "17e549d16172a548a3115739b55575968eb6523653daad76c46b0758e9425032",
            ),
            (
                "2_Dense_2048/model.safetensors",
                "a831055e5110e81c03ed6559f4ebf5842630f227ded6b6c18826700d548b990f",
            ),
        ],
        // Custom tier — `Snowflake/snowflake-arctic-embed-l-v2.0` ships a single
        // multi-GB safetensors backbone. Pinning its LFS sha256 (published at the
        // pinned revision `ac6544c8a46e00af67e330e85a9028c66b8cfd9a`) closes the same
        // F7 unverified branch for Arctic.
        "Snowflake/snowflake-arctic-embed-l-v2.0" => &[(
            "model.safetensors",
            "21bf1a120b1c6562aeec379dfa9039b0d360591c784cb1c6786e87256b738ee1",
        )],
        // Custom tier — the ModernBERT-backbone English options (issue #190/#193).
        // Each ships a single `model.safetensors`; pinning its LFS sha256 at the
        // pinned revision closes the same F7 "integrity UNVERIFIED" branch the other
        // Custom models close.
        "Alibaba-NLP/gte-modernbert-base" => &[(
            "model.safetensors",
            "3e85899d5728cb7de79781c0c3acfb91ccef9f875f1f7e0b3c9f3dd4b6a724ba",
        )],
        "ibm-granite/granite-embedding-english-r2" => &[(
            "model.safetensors",
            "23bd9625b32680f180703408ed9d8d7b3a9c1efe4c950111e3ae9d9695b21b29",
        )],
        "ibm-granite/granite-embedding-small-english-r2" => &[(
            "model.safetensors",
            "dcbfaf2ee50d358763cbc0c08e5b045ecbf6aeb02dc0e86ffb910029bf9ebb5f",
        )],
        _ => &[],
    };
    pinned
        .iter()
        .find(|(path, _)| *path == relative_path)
        .map(|(_, sha256)| *sha256)
}

/// Verify a downloaded file's SHA256 against its pinned digest.
///
/// Mirrors the **speaker-analysis** `validate_artifact_sha256(path, Option<&str>)`:
/// a `None`/empty pinned hash is a no-op success (the file is **not yet pinned** —
/// integrity unverified, logged by the caller); a present hash is hashed from disk
/// and compared case-insensitively, returning [`ModelDownloadError::ChecksumMismatch`]
/// on disagreement so a tampered/corrupt file fails the install before the
/// `.installed.json` marker is ever written.
pub(super) fn verify_file_checksum(
    file_path: &Path,
    relative_path: &str,
    expected_sha256: Option<&str>,
) -> Result<(), ModelDownloadError> {
    let Some(expected) = expected_sha256.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let actual = sha256_of_file(file_path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(ModelDownloadError::ChecksumMismatch {
            relative_path: relative_path.to_string(),
            expected: expected.to_string(),
            actual,
        });
    }
    Ok(())
}

/// Stream the file through SHA256 in 8 KiB chunks so a multi-GB safetensors file
/// never loads into memory. Returns the lowercase hex digest.
pub(super) fn sha256_of_file(file_path: &Path) -> Result<String, ModelDownloadError> {
    let mut file =
        std::fs::File::open(file_path).map_err(|source| ModelDownloadError::ReadFile {
            path: file_path.to_path_buf(),
            source,
        })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer).map_err(|source| {
            ModelDownloadError::ReadFile {
                path: file_path.to_path_buf(),
                source,
            }
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Fail-closed install gate for the **guided tiers** (B2 / F14): a model whose
/// `tier` is `English` or `Multilingual` MUST verify EVERY required weights file
/// against a pinned digest, so a guided tier can never install down the unverified
/// "trust TLS only" path. The per-file checksum loop already ran and a mismatch
/// aborted; this only refuses the finalize when a guided tier's weights file
/// carries no pinned digest at all (`pinned_file_sha256` returns `None` for it).
///
/// The check covers BOTH the base `weights_relative_path` AND the auxiliary head
/// `aux_weights_relative_path` when present (F14): the layout supports an aux head
/// (Stella's `2_Dense_2048/model.safetensors`), and the "guided tiers can never
/// install unverified" contract is about every required weights file, not just the
/// base. Today no English/Multilingual tier carries an aux head, so the aux branch
/// is dormant for them — but it is the correct, future-proof contract: a guided
/// tier that ever loaded a second head would be required to pin it too.
///
/// Custom tier (and any unpinned file) is exempt — it keeps the "integrity
/// unverified" path, trusting the revision pin + TLS (and, for Stella/Arctic,
/// happens to be pinned anyway). So this is a no-op for the Custom tier.
pub(super) fn guard_guided_tier_weights_pinned(plan: &DownloadPlan) -> Result<(), ModelDownloadError> {
    let is_guided = matches!(
        plan.tier,
        SemanticSearchModelTier::English | SemanticSearchModelTier::Multilingual
    );
    if !is_guided {
        return Ok(());
    }
    // Every required weights file of a guided tier must be pinned: the base
    // backbone and, when the model loads a second safetensors head, that head too.
    let required_weights = std::iter::once(plan.weights_relative_path.as_str())
        .chain(plan.aux_weights_relative_path.as_deref());
    for relative_path in required_weights {
        if pinned_file_sha256(&plan.hf_repo, relative_path).is_none() {
            return Err(ModelDownloadError::GuidedTierWeightsUnpinned {
                provider: plan.provider.clone(),
                model_id: plan.model_id.clone(),
                weights_relative_path: relative_path.to_string(),
            });
        }
    }
    Ok(())
}
