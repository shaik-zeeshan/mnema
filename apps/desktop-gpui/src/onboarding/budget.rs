//! `model-budget.ts`: what each Models-screen pick costs, the download budget
//! the footer prints, and the over-disk verdict. Pure. Every byte figure comes
//! from the loaded status/catalog (`Engine::models`, `Onboarding::semantic`,
//! the speaker `ModelFacts`); the copy below is the facts manifests don't carry.

use super::storage::{self, RESERVE_FLOOR_BYTES};
use crate::settings::model::format_bytes;

/// The Models strip's facts for one build (`PickerModel`).
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub name: String,
    pub short: String,
    pub bytes: u64,
    /// `approx_download_bytes` (every semantic figure).
    pub approx: bool,
    pub installed: bool,
    pub os_managed: bool,
    /// The strip's third cell: memory while running / language coverage.
    pub detail: &'static str,
}

/// `SHORT_LABELS`: segment labels short enough for a pill.
fn short_known(id: &str) -> Option<&'static str> {
    Some(match id {
        "tiny" => "Tiny",
        "base" => "Base",
        "small" => "Small",
        "medium" => "Medium",
        "parakeet-tdt-0.6b-v3-onnx" => "Full precision",
        "parakeet-tdt-0.6b-v3-onnx-int8" => "int8",
        "nomic-embed-text-v1.5" => "Nomic",
        "stella_en_400M_v5" => "Stella",
        "multilingual-e5-small" => "E5 Small",
        "bge-m3" => "BGE-M3",
        "snowflake-arctic-embed-l-v2.0" => "Snowflake",
        "gte-modernbert-base" => "GTE",
        "granite-embedding-english-r2" => "Granite R2",
        "granite-embedding-small-english-r2" => "Granite Small",
        _ => return None,
    })
}

/// "Nomic Embed Text v1.5 (English)" → "Nomic Embed Text v1.5".
pub fn strip_tier_suffix(name: &str) -> &str {
    let t = name.trim_end();
    match t
        .strip_suffix(')')
        .and_then(|s| s.rfind('(').map(|i| (s, i)))
    {
        Some((s, i)) if !s[i + 1..].contains(['(', ')']) => t[..i].trim(),
        _ => name.trim(),
    }
}

pub fn short_label(id: &str, name: &str) -> String {
    short_known(id).map_or_else(|| strip_tier_suffix(name).to_string(), str::to_string)
}

/// `MEMORY_NOTES`: by model id first, then provider.
fn memory_note(id: &str, provider: &str) -> &'static str {
    match (id, provider) {
        ("parakeet-tdt-0.6b-v3-onnx-int8", _) => "below Parakeet full precision",
        (_, "local_whisper") => "about 1.4 GB while running",
        (_, "apple_speech_on_device") => "macOS handles it",
        (_, "parakeet") => "about 3 GB while running",
        _ => "not measured",
    }
}

/// `FAMILY_NOTES`: the sentence beside (or instead of) the build picker.
pub fn family_note(family: &str) -> &'static str {
    match family {
        "apple_speech_on_device" => "One OS model. Nothing to choose, nothing to download.",
        "local_whisper" => "Larger is more accurate and slower.",
        "parakeet" => "Multilingual, CC-BY. int8 trades accuracy for memory.",
        SEMANTIC_OFF => "Keyword search still works. This is the only real saving on this row.",
        SEMANTIC_ENGLISH | SEMANTIC_MULTILINGUAL => {
            "Bigger indexes retrieve better; none are cheap."
        }
        _ => "",
    }
}

pub const SEMANTIC_OFF: &str = "off";
pub const SEMANTIC_ENGLISH: &str = "en";
pub const SEMANTIC_MULTILINGUAL: &str = "multi";

/// One transcription build (`transcriptionPicks`). A model with no id is the
/// OS-managed one (Apple Speech).
pub fn transcription_pick(provider: &str, m: &(Option<String>, String, u64, bool)) -> Pick {
    let os_managed = m.0.is_none();
    let id = m.0.as_deref().unwrap_or("");
    Pick {
        name: m.1.clone(),
        short: short_label(id, &m.1),
        bytes: if os_managed { 0 } else { m.2 },
        approx: false,
        installed: m.3,
        os_managed,
        detail: memory_note(id, provider),
    }
}

/// One embedding model (`semanticPicks`): `(id, name, approx bytes, installed, multilingual)`.
pub fn semantic_pick(m: &(String, String, u64, bool, bool)) -> Pick {
    Pick {
        name: strip_tier_suffix(&m.1).to_string(),
        short: short_label(&m.0, &m.1),
        bytes: m.2,
        approx: m.2 > 0,
        installed: m.3,
        os_managed: false,
        detail: if m.4 { "100+ languages" } else { "English" },
    }
}

/// What a pick adds to the download: nothing once it is on disk.
pub fn pick_bytes(pick: Option<&Pick>, enabled: bool) -> u64 {
    match pick {
        Some(p) if enabled && !p.installed && !p.os_managed => p.bytes,
        _ => 0,
    }
}

/// The strip's / legend's Download cell (`sizeCell`).
pub fn size_cell(pick: Option<&Pick>, prefix: &str) -> String {
    match pick {
        None => "none".into(),
        Some(p) if p.os_managed => "none \u{2014} OS managed".into(),
        Some(p) if p.installed => "already on this Mac".into(),
        Some(p) if p.bytes == 0 => "none".into(),
        Some(p) => format!("{prefix}{}", format_bytes(p.bytes as f64)),
    }
}

/// `downloadBudget`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Budget {
    pub speaker: u64,
    pub transcription: u64,
    pub semantic: u64,
    pub semantic_approx: bool,
}

impl Budget {
    pub fn bytes(&self) -> u64 {
        self.speaker + self.transcription + self.semantic
    }
    /// A total carrying an approximate part IS approximate.
    pub fn approx(&self) -> bool {
        self.semantic_approx && self.semantic > 0
    }
}

pub fn total_label(bytes: u64, approx: bool) -> String {
    if bytes == 0 {
        return "Nothing to download".into();
    }
    format!(
        "{}{}",
        if approx { "about " } else { "" },
        format_bytes(bytes as f64)
    )
}

/// What is left for downloads once the reserve and a day of capture are set aside.
pub fn room_for_downloads(free: Option<f64>, interval_s: f64, pixels: f64) -> Option<f64> {
    free.map(|f| (f - RESERVE_FLOOR_BYTES - storage::daily_bytes(interval_s, pixels)).max(0.))
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiskVerdict {
    pub need: f64,
    pub room: f64,
    pub message: String,
    /// `Some` only when turning Semantic Search off clears the whole need.
    pub escape: Option<u64>,
}

/// `diskVerdict`: `None` when the downloads fit or free space is unmeasured.
pub fn disk_verdict(
    b: &Budget,
    free: Option<f64>,
    interval_s: f64,
    pixels: f64,
) -> Option<DiskVerdict> {
    let free = free?;
    let need = storage::need_bytes(b.bytes() as f64, interval_s, pixels);
    let day = storage::daily_bytes(interval_s, pixels);
    let room = (free - RESERVE_FLOOR_BYTES - day).max(0.);
    if free >= need {
        return None;
    }
    let without = storage::need_bytes((b.bytes() - b.semantic) as f64, interval_s, pixels);
    let clears = b.semantic > 0 && free >= without;
    let tail = if b.semantic > 0 {
        format!(
            " Turning Semantic Search off frees about {}, which {}.",
            format_bytes(b.semantic as f64),
            if clears {
                "clears it"
            } else {
                "is still not enough"
            }
        )
    } else {
        " Every remaining download is required by a feature you turned on.".into()
    };
    Some(DiskVerdict {
        need,
        room,
        message: format!(
            "Not enough room. {} free \u{b7} {} needed \u{2014} that includes a day of recording ({}) and the {} kept free.{tail}",
            storage::measured(free),
            format_bytes(need),
            format_bytes(day),
            format_bytes(RESERVE_FLOOR_BYTES),
        ),
        escape: clears.then_some(b.semantic),
    })
}

#[cfg(test)]
mod tests {
    //! Mirrors `model-budget.test.ts` (the manifest figures as of 2026-11).
    use super::*;

    const WHISPER_BASE: u64 = 147_951_465;
    const SPEAKRS: u64 = 419_482_724;
    const NOMIC: u64 = 548_000_000;
    const PARAKEET_INT8: u64 = 670_619_803;
    const ANCHOR: f64 = storage::ANCHOR_VIDEO_PIXELS;
    const INTERVAL_S: f64 = 2.;

    fn whisper(id: &str, bytes: u64) -> (Option<String>, String, u64, bool) {
        (Some(id.into()), format!("Whisper {id}"), bytes, false)
    }
    fn nomic() -> (String, String, u64, bool, bool) {
        (
            "nomic-embed-text-v1.5".into(),
            "Nomic Embed Text v1.5 (English)".into(),
            NOMIC,
            false,
            false,
        )
    }
    fn budget(t: u64, s: Option<&Pick>) -> Budget {
        Budget {
            speaker: SPEAKRS,
            transcription: t,
            semantic: pick_bytes(s, true),
            semantic_approx: s.is_some_and(|p| p.approx),
        }
    }
    fn default_budget() -> Budget {
        budget(WHISPER_BASE, Some(&semantic_pick(&nomic())))
    }

    #[test]
    fn the_default_set_reads_about_1_1_gb() {
        let b = default_budget();
        assert_eq!(b.bytes(), 1_115_434_189);
        assert_eq!(total_label(b.bytes(), b.approx()), "about 1.1 GB");
    }

    #[test]
    fn about_disappears_with_nomic() {
        let b = budget(WHISPER_BASE, None);
        assert!(!b.approx());
        assert_eq!(b.bytes(), WHISPER_BASE + SPEAKRS);
        assert!(!total_label(b.bytes(), b.approx()).contains("about"));
    }

    #[test]
    fn an_installed_off_or_os_managed_model_contributes_nothing() {
        let mut base = transcription_pick("local_whisper", &whisper("base", WHISPER_BASE));
        assert_eq!(pick_bytes(Some(&base), false), 0, "feature off");
        base.installed = true;
        assert_eq!(pick_bytes(Some(&base), true), 0);
        let apple = transcription_pick(
            "apple_speech_on_device",
            &(None, "Apple Speech (on-device)".into(), 0, true),
        );
        assert!(apple.os_managed);
        assert_eq!(pick_bytes(Some(&apple), true), 0);
        assert_eq!(apple.detail, "macOS handles it");
        assert_eq!(size_cell(Some(&apple), ""), "none \u{2014} OS managed");
    }

    #[test]
    fn the_total_moves_with_every_pick() {
        let base = default_budget().bytes();
        let nomic = semantic_pick(&nomic());
        assert_eq!(
            budget(PARAKEET_INT8, Some(&nomic)).bytes(),
            base - WHISPER_BASE + PARAKEET_INT8
        );
        let none = Budget {
            speaker: 0,
            ..default_budget()
        };
        assert_eq!(none.bytes(), base - SPEAKRS);
        assert_eq!(total_label(0, false), "Nothing to download");
    }

    #[test]
    fn strip_cells_and_short_labels() {
        let int8 = transcription_pick(
            "parakeet",
            &(
                Some("parakeet-tdt-0.6b-v3-onnx-int8".into()),
                "Parakeet TDT 0.6B v3 ONNX int8".into(),
                PARAKEET_INT8,
                false,
            ),
        );
        assert_eq!(int8.short, "int8");
        assert_eq!(int8.detail, "below Parakeet full precision");
        assert_eq!(size_cell(Some(&int8), ""), "670.6 MB");
        let n = semantic_pick(&nomic());
        assert_eq!(
            (n.name.as_str(), n.short.as_str()),
            ("Nomic Embed Text v1.5", "Nomic")
        );
        assert_eq!(size_cell(Some(&n), "about "), "about 548.0 MB");
        assert_eq!(n.detail, "English");
        assert_eq!(size_cell(None, "about "), "none");
        let mut on_disk = n.clone();
        on_disk.installed = true;
        assert_eq!(size_cell(Some(&on_disk), "about "), "already on this Mac");
        assert_eq!(strip_tier_suffix("BGE-M3 (Multilingual, large)"), "BGE-M3");
        assert_eq!(short_label("x", "Some Model (English)"), "Some Model");
    }

    #[test]
    fn a_roomy_or_unmeasured_volume_says_nothing() {
        let b = default_budget();
        let need = storage::need_bytes(b.bytes() as f64, INTERVAL_S, ANCHOR);
        assert_eq!(disk_verdict(&b, Some(need), INTERVAL_S, ANCHOR), None);
        assert_eq!(disk_verdict(&b, None, INTERVAL_S, ANCHOR), None);
        assert_eq!(room_for_downloads(None, INTERVAL_S, ANCHOR), None);
    }

    #[test]
    fn the_shortfall_states_the_figures_including_the_reserve_and_a_day() {
        let b = default_budget();
        let v = disk_verdict(&b, Some(2.5e9), INTERVAL_S, ANCHOR).expect("short");
        assert_eq!(
            v.need,
            storage::need_bytes(b.bytes() as f64, INTERVAL_S, ANCHOR)
        );
        assert!(v.message.starts_with("Not enough room."));
        assert!(
            v.message.contains("a day of recording (405.0 MB)"),
            "{}",
            v.message
        );
        assert!(v.message.contains("1.1 GB kept free"));
        assert_eq!(v.room, 2.5e9 - RESERVE_FLOOR_BYTES - 405e6);
        assert_eq!(
            disk_verdict(&b, Some(620e6), INTERVAL_S, ANCHOR).map(|v| v.room),
            Some(0.)
        );
        let full = disk_verdict(&b, Some(0.), INTERVAL_S, ANCHOR).expect("full");
        assert!(full.message.contains("0 B free") && !full.message.contains("unknown size"));
    }

    #[test]
    fn the_escape_is_offered_only_when_semantic_off_clears_it() {
        let b = default_budget();
        let free = storage::need_bytes((b.bytes() - b.semantic) as f64, INTERVAL_S, ANCHOR);
        let narrow = disk_verdict(&b, Some(free), INTERVAL_S, ANCHOR).expect("short");
        assert_eq!(narrow.escape, Some(b.semantic));
        assert!(narrow.message.contains("which clears it"));
        let escaped = Budget { semantic: 0, ..b };
        assert_eq!(disk_verdict(&escaped, Some(free), INTERVAL_S, ANCHOR), None);
        let hopeless = disk_verdict(&b, Some(620e6), INTERVAL_S, ANCHOR).expect("short");
        assert_eq!(hopeless.escape, None);
        assert!(hopeless.message.contains("is still not enough"));
        let off = disk_verdict(&budget(WHISPER_BASE, None), Some(620e6), INTERVAL_S, ANCHOR)
            .expect("short");
        assert_eq!(off.escape, None);
        assert!(off
            .message
            .contains("Every remaining download is required by a feature you turned on."));
    }

    #[test]
    fn the_verdict_prices_the_draft_resolution() {
        let b = default_budget();
        let hd = 1920. * 1080.;
        let (n720, n1080) = (
            storage::need_bytes(b.bytes() as f64, INTERVAL_S, ANCHOR),
            storage::need_bytes(b.bytes() as f64, INTERVAL_S, hd),
        );
        assert!(n1080 > n720);
        let free = ((n720 + n1080) / 2.).round();
        assert_eq!(disk_verdict(&b, Some(free), INTERVAL_S, ANCHOR), None);
        assert_eq!(
            disk_verdict(&b, Some(free), INTERVAL_S, hd).map(|v| v.need),
            Some(n1080)
        );
    }
}
