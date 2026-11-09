//! The step machine and the feature model (`onboarding-flow.svelte.ts`,
//! `lib/onboarding/{feature-rules,resolve-setup,permissions-gate}.ts`). Pure:
//! no GPUI, no app-core calls, so the flow and both hard gates test directly.
//!
//! Only TWO hard gates exist, both on Capture & Storage (the folder, and room
//! for it): see `storage::block_reason`. Permissions gate nothing; their one
//! hold is the relaunch a same-process Screen Recording grant needs. Setup and
//! Voice never disable Continue.

use capture_types::CapturePermissionState as P;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    Welcome,
    Permissions,
    Capture,
    YourSettings,
    Change,
    Setup,
    Voice,
    Finale,
}

impl Step {
    pub const ALL: [Step; 8] = [
        Step::Welcome,
        Step::Permissions,
        Step::Capture,
        Step::YourSettings,
        Step::Change,
        Step::Setup,
        Step::Voice,
        Step::Finale,
    ];
    pub const COUNT: usize = 8;

    /// Shown top-left in the strip.
    pub fn label(self) -> &'static str {
        match self {
            Step::Welcome => "Mnema",
            Step::Permissions => "Permissions",
            Step::Capture => "Capture & Storage",
            Step::YourSettings => "Your settings",
            Step::Change => "Change settings",
            Step::Setup => "Setup",
            Step::Voice => "Voice",
            Step::Finale => "Finale",
        }
    }
    /// "· round trip" / "· optional": the two detours, drawn as short ticks.
    pub fn suffix(self) -> Option<&'static str> {
        match self {
            Step::Change => Some("round trip"),
            Step::Voice => Some("optional"),
            _ => None,
        }
    }
    /// Ruler position, 1-8 (Change settings owns slot 5; the straight path jumps 4 → 6).
    pub fn position(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0) + 1
    }
    pub fn next(self) -> Option<Step> {
        Some(match self {
            Step::Welcome => Step::Permissions,
            Step::Permissions => Step::Capture,
            Step::Capture => Step::YourSettings,
            Step::YourSettings => Step::Setup,
            Step::Change => Step::YourSettings,
            Step::Setup => Step::Voice,
            Step::Voice => Step::Finale,
            Step::Finale => return None,
        })
    }
    pub fn back(self) -> Option<Step> {
        Some(match self {
            Step::Welcome => return None,
            Step::Permissions => Step::Welcome,
            Step::Capture => Step::Permissions,
            Step::YourSettings => Step::Capture,
            Step::Change => Step::YourSettings,
            Step::Setup => Step::YourSettings,
            Step::Voice => Step::Setup,
            Step::Finale => Step::Voice,
        })
    }
    /// The parity / mockup `?step=` names.
    pub fn wire(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::Permissions => "permissions",
            Step::Capture => "capture",
            Step::YourSettings => "settings",
            Step::Change => "change",
            Step::Setup => "setup",
            Step::Voice => "voice",
            Step::Finale => "finale",
        }
    }
    pub fn from_wire(s: &str) -> Option<Step> {
        Self::ALL.into_iter().find(|step| step.wire() == s)
    }
}

/// `isGranted`: `assumed_working` and `unsupported` count. `unsupported` is not a
/// denial: blocking on it would trap every user on such a machine.
pub fn is_granted(v: P) -> bool {
    matches!(v, P::Granted | P::AssumedWorking | P::Unsupported)
}

/// `isDenied` (`restricted` has no Rust variant: macOS reports it as denied).
pub fn is_denied(v: P) -> bool {
    v == P::Denied
}

/// `screenGate`: only a same-process grant holds (ScreenCaptureKit can't use it yet).
pub fn screen_gate(screen: P, needs_relaunch: bool) -> (bool, &'static str) {
    if needs_relaunch {
        (false, "Relaunch to continue")
    } else if is_denied(screen) {
        (true, "Continue without screen recording")
    } else {
        (true, "Continue")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feature {
    Screen,
    Ocr,
    Microphone,
    SystemAudio,
    Transcription,
    SpeakerSeparation,
    SemanticSearch,
    AiFeatures,
}

impl Feature {
    /// `FEATURE_ORDER` minus privacy (the onboarding-only toggle has no row here).
    pub const ORDER: [Feature; 8] = [
        Feature::Screen,
        Feature::Ocr,
        Feature::Microphone,
        Feature::SystemAudio,
        Feature::Transcription,
        Feature::SpeakerSeparation,
        Feature::SemanticSearch,
        Feature::AiFeatures,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Feature::Screen => "Screen capture",
            Feature::Ocr => "Read on-screen text",
            Feature::Microphone => "Microphone",
            Feature::SystemAudio => "System audio",
            Feature::Transcription => "Transcription",
            Feature::SpeakerSeparation => "Who\u{2019}s speaking",
            Feature::SemanticSearch => "Semantic Search",
            Feature::AiFeatures => "AI features",
        }
    }
    /// Tree depth on Change settings (indented rows need the row above them).
    pub fn depth(self) -> u8 {
        match self {
            Feature::Ocr | Feature::Transcription => 1,
            Feature::SpeakerSeparation => 2,
            _ => 0,
        }
    }
}

/// Permission INTENT: system audio is a raised prompt, never a read grant (ADR 0052).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Intents {
    pub screen: bool,
    pub microphone: bool,
    pub system_audio: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    pub perms: Intents,
    pub screen: bool,
    pub ocr: bool,
    pub microphone: bool,
    pub system_audio: bool,
    pub transcription: bool,
    pub speaker_separation: bool,
    pub semantic_search: bool,
    pub ai_features: bool,
    pub privacy: bool,
}

impl Features {
    pub fn get(&self, f: Feature) -> bool {
        match f {
            Feature::Screen => self.screen,
            Feature::Ocr => self.ocr,
            Feature::Microphone => self.microphone,
            Feature::SystemAudio => self.system_audio,
            Feature::Transcription => self.transcription,
            Feature::SpeakerSeparation => self.speaker_separation,
            Feature::SemanticSearch => self.semantic_search,
            Feature::AiFeatures => self.ai_features,
        }
    }
    fn set(&mut self, f: Feature, on: bool) {
        *match f {
            Feature::Screen => &mut self.screen,
            Feature::Ocr => &mut self.ocr,
            Feature::Microphone => &mut self.microphone,
            Feature::SystemAudio => &mut self.system_audio,
            Feature::Transcription => &mut self.transcription,
            Feature::SpeakerSeparation => &mut self.speaker_separation,
            Feature::SemanticSearch => &mut self.semantic_search,
            Feature::AiFeatures => &mut self.ai_features,
        } = on;
    }

    /// `normalizeFeatures`: transcription needs an audio source; speakers need transcription.
    pub fn normalized(mut self) -> Self {
        self.transcription &= self.microphone || self.system_audio;
        self.speaker_separation &= self.transcription;
        self
    }

    pub fn lock_reason(&self, f: Feature) -> Option<&'static str> {
        match f {
            Feature::Microphone if !self.perms.microphone => Some("Needs Microphone permission"),
            Feature::SpeakerSeparation if !self.transcription => {
                Some("Needs Audio transcription on")
            }
            Feature::Transcription if !(self.microphone || self.system_audio) => {
                Some("Needs an audio source on")
            }
            _ => None,
        }
    }

    pub fn toggle_disabled(&self, f: Feature) -> bool {
        !self.get(f) && self.lock_reason(f).is_some()
    }

    /// `applyToggle`: `None` = a refused (locked) enable. Turning an audio source
    /// on re-enables the processing it feeds; turning one off cascades down.
    pub fn toggled(&self, f: Feature) -> Option<Self> {
        let on = !self.get(f);
        if on && self.lock_reason(f).is_some() {
            return None;
        }
        let mut next = *self;
        next.set(f, on);
        if on && matches!(f, Feature::Microphone | Feature::SystemAudio) {
            next.transcription = true;
            next.speaker_separation = true;
        }
        Some(next.normalized())
    }

    /// The rows a toggle moved besides the one touched (`cascadeOf`).
    pub fn cascade(&self, after: &Self, touched: Feature) -> Vec<Feature> {
        Feature::ORDER
            .into_iter()
            .filter(|f| *f != touched && self.get(*f) != after.get(*f))
            .collect()
    }

    /// `lockFix`: the action that unlocks a disabled row (`Grant`, or the parent toggle).
    pub fn lock_fix(&self, f: Feature) -> Option<(LockFix, Feature, String)> {
        if !self.toggle_disabled(f) {
            return None;
        }
        if f == Feature::Microphone {
            return Some((LockFix::Grant, f, "Grant Microphone".into()));
        }
        let parent = match f {
            Feature::SpeakerSeparation => Feature::Transcription,
            Feature::Transcription => Feature::SystemAudio,
            _ => return None,
        };
        if self.toggle_disabled(parent) {
            return self.lock_fix(parent);
        }
        Some((
            LockFix::Toggle,
            parent,
            format!("Turn {} on", parent.label()),
        ))
    }

    /// `systemAudioNeedsRequest`: on, but the prompt was never raised.
    pub fn system_audio_needs_request(&self) -> bool {
        self.system_audio && !self.perms.system_audio
    }

    /// `featureNote`: the one-line description under each Change-settings row.
    pub fn note(&self, f: Feature, ai_ready: bool, engine: &str) -> String {
        match f {
            Feature::Screen if self.perms.screen => {
                "Frames of your screen. Everything else is built on these.".into()
            }
            Feature::Screen => {
                "Screen Recording is not granted \u{2014} stays listed, records nothing.".into()
            }
            Feature::Microphone if self.perms.microphone => {
                "Your voice, from the built-in or a connected mic.".into()
            }
            Feature::Microphone if self.microphone => {
                "Microphone permission is not granted \u{2014} stays on, records nothing.".into()
            }
            Feature::Microphone => {
                "Microphone permission is not granted \u{2014} grant it to turn this on.".into()
            }
            Feature::SystemAudio => format!(
                "What your Mac plays. Excludes Mnema itself and every privacy-listed app.{}",
                if self.system_audio_needs_request() {
                    " macOS can\u{2019}t confirm this grant."
                } else {
                    ""
                }
            ),
            Feature::Ocr if self.screen => "On-device. Apple Vision needs no download.".into(),
            Feature::Ocr => "Nothing to read while screen capture is off.".into(),
            Feature::Transcription if self.transcription => format!("Runs locally on {engine}."),
            Feature::Transcription if self.lock_reason(f).is_some() => {
                "Needs an audio source above it.".into()
            }
            Feature::Transcription => {
                "Off \u{2014} the audio is still recorded, just never turned into text.".into()
            }
            Feature::SpeakerSeparation if self.speaker_separation => {
                "Splits a conversation into voices, on-device.".into()
            }
            Feature::SpeakerSeparation => "A transcript is what it splits.".into(),
            Feature::SemanticSearch => {
                "Finds by meaning, not by word. The biggest single download.".into()
            }
            Feature::AiFeatures if ai_ready => {
                "Ready \u{2014} a default model is configured.".into()
            }
            Feature::AiFeatures if self.ai_features => {
                "On, with nothing connected to answer with.".into()
            }
            Feature::AiFeatures => {
                "Never pre-ticked. Connecting a provider is what turns it on.".into()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockFix {
    Grant,
    Toggle,
}

/// What the user already chose (saved settings win; the resolver only fills gaps).
#[derive(Clone, Debug, Default)]
pub struct Saved {
    pub features: Vec<(Feature, bool)>,
}

/// `resolveSetup`'s feature half: every capture source defaults ON (the grant is
/// what makes each real); processing follows a granted audio source; AI is never
/// pre-ticked.
pub fn resolve(perms: Intents, saved: &Saved) -> Features {
    let pick = |f: Feature, fallback: bool| {
        saved
            .features
            .iter()
            .rev()
            .find(|(s, _)| *s == f)
            .map_or(fallback, |(_, on)| *on)
    };
    let audio = perms.microphone || perms.system_audio;
    Features {
        perms,
        screen: pick(Feature::Screen, true),
        ocr: pick(Feature::Ocr, true),
        microphone: pick(Feature::Microphone, true),
        system_audio: pick(Feature::SystemAudio, true),
        transcription: pick(Feature::Transcription, audio),
        speaker_separation: pick(Feature::SpeakerSeparation, audio),
        semantic_search: pick(Feature::SemanticSearch, true),
        ai_features: pick(Feature::AiFeatures, false),
        privacy: true,
    }
    .normalized()
}

/// The three downloadable subsystems, in the resolver's fixed order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subsystem {
    Speaker,
    Transcription,
    Semantic,
}

/// The selected model's live facts, per subsystem (`SelectedModelFacts`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelFacts {
    pub provider: String,
    pub model_id: Option<String>,
    pub bytes: u64,
    pub installed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkItem {
    pub subsystem: Subsystem,
    pub feature: Feature,
    pub provider: String,
    pub model_id: String,
    pub bytes: u64,
}

impl WorkItem {
    pub fn name(&self) -> &'static str {
        self.feature.label()
    }
    /// Any total containing the semantic model is approximate (`approx_download_bytes`).
    pub fn approx(&self) -> bool {
        self.subsystem == Subsystem::Semantic
    }
}

/// `buildWorkList`: speakrs → transcription → semantic (speakrs first: Voice needs it).
pub fn work_list(f: &Features, models: &[(Subsystem, ModelFacts)]) -> Vec<WorkItem> {
    let want = [
        (Subsystem::Speaker, Feature::SpeakerSeparation),
        (Subsystem::Transcription, Feature::Transcription),
        (Subsystem::Semantic, Feature::SemanticSearch),
    ];
    want.into_iter()
        .filter(|(_, feature)| f.get(*feature))
        .filter_map(|(subsystem, feature)| {
            let (_, m) = models.iter().find(|(s, _)| *s == subsystem)?;
            // Deepgram (cloud) and OS-managed engines download nothing.
            if m.installed || m.bytes == 0 || m.provider == "deepgram" {
                return None;
            }
            Some(WorkItem {
                subsystem,
                feature,
                provider: m.provider.clone(),
                model_id: m.model_id.clone()?,
                bytes: m.bytes,
            })
        })
        .collect()
}

pub fn work_bytes(items: &[WorkItem]) -> u64 {
    items.iter().map(|i| i.bytes).sum()
}

/// Continue is live everywhere except a blocked Capture & Storage.
pub fn can_continue(step: Step, capture_block: Option<&str>) -> bool {
    step != Step::Capture || capture_block.is_none()
}

/// Closing the window: before completion it ends the app; after, it just closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    Exit,
    Close,
}

pub fn close_action(completed: bool) -> CloseAction {
    if completed {
        CloseAction::Close
    } else {
        CloseAction::Exit
    }
}
