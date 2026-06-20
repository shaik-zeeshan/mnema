//! App-wide third-party attribution / acknowledgements surface.
//!
//! Several bundled or on-demand-downloaded models ship under licenses that
//! require attribution (e.g. CC-BY-4.0 for the Parakeet and WeSpeaker
//! embeddings). The attribution metadata already lives on each model crate's
//! descriptor (`license_label` / `source_url`); this module assembles a single
//! provider-neutral payload out of every model manifest (speaker diarization,
//! transcription, OCR) and exposes it to the frontend so the acknowledgements
//! are user-reachable.
//!
//! The assembly is intentionally descriptor-driven, not provider-specific: a
//! new model becomes attributed simply by appearing in its crate's
//! `builtin_model_manifest()`. Descriptors that lack a `license_label` are still
//! listed (never dropped) with a fallback "see source" label, because the point
//! of the surface is completeness.

use serde::Serialize;

/// One attributed component (a single model descriptor) in the notices payload.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThirdPartyNoticeEntry {
    /// Stable component identifier (`provider` or `provider/modelId`). Useful as
    /// a list key on the frontend; not shown verbatim to users.
    pub component: String,
    /// Human-readable model category, e.g. "Speaker Diarization".
    pub kind: String,
    /// Display name from the descriptor.
    pub display_name: String,
    /// License label if the descriptor carries one (e.g. "CC-BY-4.0", "MIT").
    pub license: Option<String>,
    /// Upstream source URL if the descriptor carries one.
    pub source_url: Option<String>,
}

/// Render-ready third-party notices payload returned by the Tauri command.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThirdPartyNotices {
    /// Structured entries, grouped on the frontend by `kind`.
    pub entries: Vec<ThirdPartyNoticeEntry>,
    /// A flat, copy-pasteable plain-text rendering of every entry, suitable for
    /// a "Copy notices" affordance.
    pub plain_text: String,
}

const KIND_SPEAKER: &str = "Speaker Diarization";
const KIND_TRANSCRIPTION: &str = "Transcription";
const KIND_OCR: &str = "OCR";

/// Build a notice entry from the descriptor's attribution fields, applying a
/// sensible fallback license label when the descriptor carries none. Entries
/// are never dropped — completeness is the whole point of the surface.
fn make_entry(
    component: String,
    kind: &str,
    display_name: String,
    license_label: Option<String>,
    source_url: Option<String>,
) -> ThirdPartyNoticeEntry {
    let source_url = source_url.filter(|url| !url.trim().is_empty());
    // Surface *something* even without an explicit label so the entry stays
    // honest and is never dropped: fall back to "see source".
    let license = license_label
        .filter(|label| !label.trim().is_empty())
        .or_else(|| Some("see source".to_string()));
    ThirdPartyNoticeEntry {
        component,
        kind: kind.to_string(),
        display_name,
        license,
        source_url,
    }
}

fn component_id(provider: &str, model_id: Option<&str>) -> String {
    match model_id {
        Some(model_id) => format!("{provider}/{model_id}"),
        None => provider.to_string(),
    }
}

/// Assemble the full notices payload from every model manifest.
pub fn collect_third_party_notices() -> ThirdPartyNotices {
    let mut entries = Vec::new();

    // Speaker diarization presets (sherpa-onnx pyannote/NeMo, and any future
    // on-device provider added to the manifest).
    for descriptor in speaker_analysis::builtin_model_manifest().models {
        entries.push(make_entry(
            component_id(&descriptor.provider, descriptor.model_id.as_deref()),
            KIND_SPEAKER,
            descriptor.display_name,
            descriptor.license_label,
            descriptor.source_url,
        ));
    }

    // Transcription models (whisper.cpp, Parakeet, Apple Speech).
    for descriptor in audio_transcription::builtin_model_manifest().models {
        entries.push(make_entry(
            component_id(&descriptor.provider, descriptor.model_id.as_deref()),
            KIND_TRANSCRIPTION,
            descriptor.display_name,
            descriptor.license_label,
            descriptor.source_url,
        ));
    }

    // OCR engines (Apple Vision, Tesseract, PaddleOCR).
    for descriptor in ocr::builtin_model_manifest().models {
        entries.push(make_entry(
            component_id(&descriptor.provider, descriptor.model_id.as_deref()),
            KIND_OCR,
            descriptor.display_name,
            descriptor.license_label,
            descriptor.source_url,
        ));
    }

    let plain_text = render_plain_text(&entries);
    ThirdPartyNotices {
        entries,
        plain_text,
    }
}

/// Render the structured entries into a flat plain-text block grouped by kind.
fn render_plain_text(entries: &[ThirdPartyNoticeEntry]) -> String {
    let mut out = String::new();
    out.push_str("THIRD-PARTY NOTICES\n");
    out.push_str(
        "Mnema bundles or downloads the on-device models listed below. Each is\n\
         attributed to its upstream project under the stated license.\n",
    );

    for kind in [KIND_SPEAKER, KIND_TRANSCRIPTION, KIND_OCR] {
        let group: Vec<&ThirdPartyNoticeEntry> =
            entries.iter().filter(|entry| entry.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        out.push('\n');
        out.push_str(kind);
        out.push('\n');
        for entry in group {
            let license = entry.license.as_deref().unwrap_or("see source");
            out.push_str("  - ");
            out.push_str(&entry.display_name);
            out.push_str(" (license: ");
            out.push_str(license);
            out.push(')');
            if let Some(source_url) = &entry.source_url {
                out.push_str("\n    ");
                out.push_str(source_url);
            }
            out.push('\n');
        }
    }

    out
}

/// Tauri command: return the assembled third-party notices payload.
#[tauri::command]
pub fn get_third_party_notices() -> ThirdPartyNotices {
    collect_third_party_notices()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_non_empty_and_covers_all_model_kinds() {
        let notices = collect_third_party_notices();
        assert!(
            !notices.entries.is_empty(),
            "notices payload should not be empty"
        );

        let has_kind = |kind: &str| notices.entries.iter().any(|entry| entry.kind == kind);
        assert!(has_kind(KIND_SPEAKER), "missing speaker diarization entries");
        assert!(has_kind(KIND_TRANSCRIPTION), "missing transcription entries");
        assert!(has_kind(KIND_OCR), "missing OCR entries");
    }

    #[test]
    fn includes_all_speaker_presets_with_source_urls() {
        let notices = collect_third_party_notices();
        let speaker: Vec<&ThirdPartyNoticeEntry> = notices
            .entries
            .iter()
            .filter(|entry| entry.kind == KIND_SPEAKER)
            .collect();

        // Mirror the speaker manifest: every preset descriptor must be listed.
        let manifest = speaker_analysis::builtin_model_manifest();
        assert_eq!(
            speaker.len(),
            manifest.models.len(),
            "every speaker preset should be attributed"
        );
        assert!(
            speaker.len() >= 3,
            "expected at least the three built-in speaker presets"
        );

        // The sherpa presets carry no explicit license_label, but each must
        // still surface a source URL and a non-empty (fallback) license.
        for entry in &speaker {
            assert!(
                entry.source_url.is_some(),
                "speaker entry {} should carry a source URL",
                entry.component
            );
            assert!(
                entry
                    .license
                    .as_deref()
                    .map(|label| !label.trim().is_empty())
                    .unwrap_or(false),
                "speaker entry {} should carry a (fallback) license label",
                entry.component
            );
        }
    }

    #[test]
    fn surfaces_attribution_required_licenses() {
        let notices = collect_third_party_notices();
        // Parakeet ships under CC-BY-4.0, which mandates attribution; it must be
        // present with its license intact.
        assert!(
            notices
                .entries
                .iter()
                .any(|entry| entry.license.as_deref() == Some("CC-BY-4.0")),
            "CC-BY-4.0 attribution must be surfaced"
        );
    }

    #[test]
    fn plain_text_lists_grouped_entries() {
        let notices = collect_third_party_notices();
        assert!(notices.plain_text.contains("THIRD-PARTY NOTICES"));
        assert!(notices.plain_text.contains(KIND_SPEAKER));
        assert!(notices.plain_text.contains(KIND_TRANSCRIPTION));
        assert!(notices.plain_text.contains(KIND_OCR));
        // Each structured entry's display name should appear in the bundle.
        for entry in &notices.entries {
            assert!(
                notices.plain_text.contains(&entry.display_name),
                "plain text should mention {}",
                entry.display_name
            );
        }
    }
}
