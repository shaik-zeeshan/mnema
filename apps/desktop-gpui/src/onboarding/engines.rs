//! Change settings › Engines, the pure half (`transcription-engines.ts`): one
//! recommended engine that says why, and alternatives that carry their price
//! as a real delta against it. Only measured axes: download (from the
//! manifest) and memory (measured on a running app, so always "about").

use super::data::Engine;
use crate::settings::model::format_bytes;

/// `default_audio_transcription_provider()`.
pub const RECOMMENDED: &str = "local_whisper";

/// Resident memory with the engine's default model loaded, measured (no manifest
/// publishes one); Apple Speech runs in macOS's own process, so its figure is
/// the app's baseline.
fn ram_bytes(provider: &str) -> Option<u64> {
    match provider {
        "apple_speech_on_device" => Some(800_000_000),
        "local_whisper" => Some(1_400_000_000),
        "parakeet" => Some(3_000_000_000),
        _ => None,
    }
}

/// One engine card.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub provider: String,
    pub name: String,
    pub icon: Option<&'static str>,
    pub line: String,
    /// A bigger build of the same engine, picked in the model picker.
    pub foot: Option<String>,
    /// Bytes still to fetch: 0 = nothing to fetch, `None` = not known yet.
    pub bytes: Option<u64>,
    pub ram: Option<u64>,
    pub download: String,
    pub memory: String,
}

/// The model an engine starts on (`defaultTranscriptionModelIdForProvider`):
/// Whisper's Base, Parakeet's int8 build, else the first listed.
pub fn default_model(e: &Engine) -> Option<&(Option<String>, String, u64, bool)> {
    let wanted = match e.provider.as_str() {
        "local_whisper" => Some("base"),
        "parakeet" => Some("parakeet-tdt-0.6b-v3-onnx-int8"),
        _ => None,
    };
    e.models
        .iter()
        .find(|m| wanted.is_some() && m.0.as_deref() == wanted)
        .or(e.models.first())
}

pub fn cards(engines: &[Engine]) -> Vec<Card> {
    engines
        .iter()
        .map(|e| {
            let (name, icon, line, heavier_label) = match e.provider.as_str() {
                "apple_speech_on_device" => ("Apple Speech", Some("icons/feather.svg"), "Nothing to download and the app stays at its baseline \u{2014} macOS does the work in its own process. It transcribes the languages macOS has installed.", None),
                "local_whisper" => ("Whisper", Some("icons/audio-waveform.svg"), "Balanced speed, size and quality \u{2014} the manifest\u{2019}s own reason for making Base the default.", None),
                "parakeet" => ("Parakeet", Some("icons/cpu.svg"), "The heaviest of the three: the manifest calls it the highest memory use.", Some("full-precision build")),
                _ => (e.name.as_str(), None, "", None),
            };
            let model = default_model(e);
            // A model with no download is OS-managed.
            let os_managed = model.is_some_and(|m| m.2 == 0);
            let installed = model.is_some_and(|m| m.3);
            let bytes = model.map(|m| if installed || os_managed { 0 } else { m.2 });
            let base = model.map_or(0, |m| m.2);
            let biggest = e
                .models
                .iter()
                .filter(|m| model.is_none_or(|d| d.0 != m.0) && m.2 > 0)
                .map(|m| m.2)
                .max()
                .unwrap_or(0);
            let foot = heavier_label.filter(|_| biggest > base).map(|label| {
                format!(
                    "{label} {} (+{}) \u{2014} chosen in the model picker, not here",
                    format_bytes(biggest as f64),
                    format_bytes((biggest - base) as f64)
                )
            });
            let ram = ram_bytes(&e.provider);
            Card {
                provider: e.provider.clone(),
                name: name.to_string(),
                icon,
                line: line.to_string(),
                foot,
                bytes,
                ram,
                download: match bytes {
                    None => "checking\u{2026}".into(),
                    Some(_) if os_managed => "no download".into(),
                    Some(_) if installed => "already on this Mac".into(),
                    Some(b) => format_bytes(b as f64),
                },
                memory: ram.map_or("not measured".into(), |r| {
                    format!("about {}", format_bytes(r as f64))
                }),
            }
        })
        .collect()
}

/// 0..=1 for a meter track (`trackWidth`): an unknown value fills nothing.
pub fn track(value: Option<u64>, max: u64) -> f32 {
    match value {
        Some(v) if max > 0 => v as f32 / max as f32,
        _ => 0.,
    }
}

/// What picking `provider` costs against the recommended engine (`engineDelta`):
/// the sentence, and whether it costs more on some axis.
pub fn delta(cards: &[Card], provider: &str) -> (String, bool) {
    let (Some(chosen), Some(rec)) = (
        cards.iter().find(|c| c.provider == provider),
        cards.iter().find(|c| c.provider == RECOMMENDED),
    ) else {
        return (String::new(), false);
    };
    if chosen.provider == rec.provider {
        return (
            "This is what Mnema picked. Nothing to decide unless you want to.".into(),
            false,
        );
    }
    let diff = |a: Option<u64>, b: Option<u64>| Some(a? as i64 - b? as i64).filter(|d| *d != 0);
    let sign = |d: i64| if d > 0 { "+" } else { "\u{2212}" };
    let mut parts = Vec::new();
    let mut up = false;
    if let Some(d) = diff(chosen.bytes, rec.bytes) {
        parts.push(format!(
            "{}{} to download",
            sign(d),
            format_bytes(d.unsigned_abs() as f64)
        ));
        up |= d > 0;
    }
    if let Some(d) = diff(chosen.ram, rec.ram) {
        parts.push(format!(
            "about {}{} of memory",
            sign(d),
            format_bytes(d.unsigned_abs() as f64)
        ));
        up |= d > 0;
    }
    if parts.is_empty() {
        return (
            "Costs the same as the recommended engine, as far as anything measured says.".into(),
            false,
        );
    }
    (
        format!("{} vs the recommended engine", parts.join(" \u{b7} ")),
        up,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(provider: &str, models: Vec<(&str, u64, bool)>) -> Engine {
        Engine {
            provider: provider.into(),
            name: provider.into(),
            models: models
                .into_iter()
                .map(|(id, b, i)| (Some(id.to_string()), id.to_string(), b, i))
                .collect(),
        }
    }

    #[test]
    fn cards_and_delta_match_transcription_engines_ts() {
        let engines = vec![
            engine("apple_speech_on_device", vec![("system", 0, true)]),
            engine(
                "local_whisper",
                vec![("tiny", 77_700_000, false), ("base", 147_000_000, false)],
            ),
            engine(
                "parakeet",
                vec![("int8", 670_000_000, false), ("fp32", 2_430_000_000, false)],
            ),
        ];
        let c = cards(&engines);
        assert_eq!(c[0].download, "no download");
        assert_eq!(c[1].download, "147.0 MB", "Whisper starts on Base");
        assert_eq!(c[1].memory, "about 1.4 GB");
        assert_eq!(
            c[2].foot.as_deref(),
            Some("full-precision build 2.4 GB (+1.8 GB) \u{2014} chosen in the model picker, not here")
        );
        assert_eq!(
            delta(&c, "local_whisper").0,
            "This is what Mnema picked. Nothing to decide unless you want to."
        );
        assert_eq!(
            delta(&c, "parakeet"),
            (
                "+523.0 MB to download \u{b7} about +1.6 GB of memory vs the recommended engine"
                    .into(),
                true
            )
        );
        let (apple, up) = delta(&c, "apple_speech_on_device");
        assert_eq!(apple, "\u{2212}147.0 MB to download \u{b7} about \u{2212}600.0 MB of memory vs the recommended engine");
        assert!(!up);
    }
}
