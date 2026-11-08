//! The rail's five groups and their sections (`lib/settings/groups.ts`), the
//! rail search (`rail-filter.ts`), `?tab=` deeplinks and the scroll-spy rule
//! (`scroll-spy.ts`). Pure, so it's tested without a window.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroupId {
    General,
    Capture,
    Intelligence,
    Data,
    About,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SectionId {
    Appearance,
    Startup,
    Shortcuts,
    Capture,
    Video,
    Audio,
    Privacy,
    Intelligence,
    AskAi,
    UserContext,
    Ocr,
    Transcription,
    Speakers,
    SemanticSearch,
    Storage,
    Access,
    License,
    About,
    Developer,
}

pub struct Section {
    pub id: SectionId,
    pub label: &'static str,
    pub keywords: &'static [&'static str],
}

pub struct Group {
    pub id: GroupId,
    pub label: &'static str,
    pub description: &'static str,
    /// Lucide asset path of the rail tile glyph (`section-icons.ts`).
    pub icon: &'static str,
    pub sections: &'static [Section],
}

const fn s(id: SectionId, label: &'static str, keywords: &'static [&'static str]) -> Section {
    Section {
        id,
        label,
        keywords,
    }
}

pub static GROUPS: [Group; 5] = [
    Group {
        id: GroupId::General,
        label: "General",
        description: "Appearance, startup, and keyboard shortcuts.",
        icon: "icons/sliders-horizontal.svg",
        sections: &[
            s(
                SectionId::Appearance,
                "Appearance",
                &["theme", "dark", "light", "time format", "24-hour", "12-hour", "clock", "follow live recording"],
            ),
            s(SectionId::Startup, "Startup", &["auto-start", "launch", "login", "auto start recording"]),
            s(SectionId::Shortcuts, "Shortcuts", &["keyboard", "hotkey", "global shortcuts", "key binding"]),
        ],
    },
    Group {
        id: GroupId::Capture,
        label: "Capture",
        description: "What gets recorded, how it\u{2019}s encoded, and what never is.",
        icon: "icons/monitor.svg",
        sections: &[
            s(
                SectionId::Capture,
                "Sources",
                &["screen", "microphone", "system audio", "segment duration", "idle", "pause", "sensitivity", "vad", "voice detection"],
            ),
            s(
                SectionId::Video,
                "Video",
                &["bitrate", "resolution", "frame rate", "fps", "capture rate", "snapshot", "interval"],
            ),
            s(SectionId::Audio, "Microphone", &["audio", "device", "input", "on disconnect"]),
            s(SectionId::Privacy, "Privacy", &["excluded apps", "browser url", "frame context", "metadata"]),
        ],
    },
    Group {
        id: GroupId::Intelligence,
        label: "Intelligence",
        description: "Models that read, transcribe, and answer over your history. Everything here is off until you turn it on.",
        icon: "icons/sparkles.svg",
        sections: &[
            s(
                SectionId::Intelligence,
                "Providers",
                &["api key", "anthropic", "openai", "chatgpt", "subscription", "ollama", "default model", "ai runtime"],
            ),
            s(SectionId::AskAi, "Ask AI", &["chat", "quick recall", "tool calls", "model override"]),
            s(SectionId::UserContext, "User Context", &["derive context", "derivation", "backfill", "subjects"]),
            s(SectionId::Ocr, "OCR", &["text recognition", "language", "preview cache", "tesseract"]),
            s(SectionId::Transcription, "Transcription", &["whisper", "parakeet", "audio", "transcribe", "model"]),
            s(
                SectionId::Speakers,
                "Speakers",
                &["diarization", "speaker separation", "recognize people", "voice", "my voice", "voiceprint", "enroll", "label my voice automatically"],
            ),
            s(SectionId::SemanticSearch, "Semantic Search", &["embedding", "vector search", "model"]),
        ],
    },
    Group {
        id: GroupId::Data,
        label: "Data",
        description: "Where capture lives on disk, how long it\u{2019}s kept, and who else can read it.",
        icon: "icons/database.svg",
        sections: &[
            s(
                SectionId::Storage,
                "Storage",
                &["retention", "save directory", "storage location", "disk", "delete recent capture"],
            ),
            s(SectionId::Access, "CLI Access", &["cli access", "agent", "broker"]),
        ],
    },
    Group {
        id: GroupId::About,
        label: "About",
        description: "License, version, updates, and diagnostics.",
        icon: "icons/info.svg",
        sections: &[
            s(
                SectionId::License,
                "License & Trial",
                &["license", "trial", "buy", "purchase", "activate", "renew", "read-only", "subscription"],
            ),
            s(SectionId::About, "Updates", &["about", "version", "third-party notices", "acknowledgements"]),
            s(SectionId::Developer, "Developer", &["logs", "debug logging", "developer options"]),
        ],
    },
];

pub fn group(id: GroupId) -> &'static Group {
    GROUPS
        .iter()
        .find(|g| g.id == id)
        .expect("every group id is listed")
}

pub fn group_for(section: SectionId) -> GroupId {
    GROUPS
        .iter()
        .find(|g| g.sections.iter().any(|s| s.id == section))
        .map(|g| g.id)
        .expect("every section is in a group")
}

pub fn section_label(id: SectionId) -> &'static str {
    GROUPS
        .iter()
        .flat_map(|g| g.sections.iter())
        .find(|s| s.id == id)
        .map(|s| s.label)
        .unwrap_or_default()
}

/// The groups (and, inside each, the sections) a rail query keeps: a group-label
/// hit keeps every section, else sections whose label or keywords contain it.
pub fn filter(query: &str) -> Vec<(GroupId, Vec<SectionId>)> {
    let needle = query.trim().to_lowercase();
    GROUPS
        .iter()
        .filter_map(|g| {
            let all = needle.is_empty() || g.label.to_lowercase().contains(&needle);
            let ids: Vec<SectionId> = g
                .sections
                .iter()
                .filter(|s| {
                    all || s.label.to_lowercase().contains(&needle)
                        || s.keywords
                            .iter()
                            .any(|k| k.to_lowercase().contains(&needle))
                })
                .map(|s| s.id)
                .collect();
            (!ids.is_empty()).then_some((g.id, ids))
        })
        .collect()
}

/// `?tab=` (or a legacy alias) to the section it lands on (`resolveTabDeeplink`).
pub fn resolve_tab(value: &str) -> Option<SectionId> {
    use SectionId::*;
    Some(match value {
        "about" => About,
        "capture" | "behavior" => Capture,
        "access" | "cliAccess" | "cli-access" => Access,
        "intelligence" | "reasoning" | "reasoning-engine" | "ai" | "ai-runtime" => Intelligence,
        "user-context" | "userContext" => UserContext,
        "privacy" | "metadata" => Privacy,
        "shortcuts" | "keyboard" | "keyboard-shortcuts" | "keyboard_bindings" => Shortcuts,
        "video" => Video,
        "audio" | "microphone" => Audio,
        "processing" | "ocr" => Ocr,
        "transcription" => Transcription,
        "speakers" => Speakers,
        "semanticSearch" | "semantic-search" => SemanticSearch,
        "storage" => Storage,
        "license" => License,
        "appearance" => Appearance,
        "developer" => Developer,
        _ => return None,
    })
}

/// How far under the scroll column's top a section's top must cross to count
/// as the one in view (`SPY_LINE_PX`).
pub const SPY_LINE: f32 = 80.;

/// The section in view: the last whose top crossed `line`, the first when none
/// has, the last once the column bottoms out (`activeSectionAt`).
pub fn active_section_at(
    tops: &[(SectionId, f32)],
    line: f32,
    at_bottom: bool,
) -> Option<SectionId> {
    let first = tops.first()?.0;
    if at_bottom {
        return tops.last().map(|t| t.0);
    }
    Some(
        tops.iter()
            .filter(|(_, top)| *top <= line)
            .last()
            .map_or(first, |t| t.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_matches_labels_keywords_and_group_names() {
        assert_eq!(filter("").len(), 5);
        assert_eq!(
            filter("retention"),
            vec![(GroupId::Data, vec![SectionId::Storage])]
        );
        assert_eq!(
            filter("BITRATE"),
            vec![(GroupId::Capture, vec![SectionId::Video])]
        );
        let intel = filter("intel");
        assert_eq!(intel.len(), 1);
        assert_eq!(intel[0].1.len(), 7);
        assert!(filter("zzz").is_empty());
    }

    #[test]
    fn tab_aliases_resolve() {
        assert_eq!(resolve_tab("keyboard"), Some(SectionId::Shortcuts));
        assert_eq!(resolve_tab("processing"), Some(SectionId::Ocr));
        assert_eq!(resolve_tab("nope"), None);
        assert_eq!(group_for(SectionId::Access), GroupId::Data);
    }

    #[test]
    fn spy_picks_the_last_crossed_section() {
        use SectionId::*;
        let tops = [(Capture, 10.), (Video, 79.), (Audio, 81.)];
        assert_eq!(active_section_at(&tops, SPY_LINE, false), Some(Video));
        assert_eq!(active_section_at(&tops, SPY_LINE, true), Some(Audio));
        assert_eq!(
            active_section_at(&[(Capture, 200.)], SPY_LINE, false),
            Some(Capture)
        );
        assert_eq!(active_section_at(&[], SPY_LINE, false), None);
    }
}
