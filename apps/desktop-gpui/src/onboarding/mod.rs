//! Onboarding (`routes/onboarding/*`, issue #195): its own undecorated window,
//! 1120×800 / min 920×620, transparent with 12 px continuous corners. Eight
//! steps under a 52 px strip; the only hard gates are on Capture & Storage.
//! Closing the window before onboarding completes ends the app.
//!
//! State lives on [`Onboarding`]; every backend call runs on the background
//! executor through [`data::Backend`] (`CoreBackend` = app-core, `FixtureBackend`
//! = the parity JSON). Screens render in `screens/*`; the step machine, the
//! storage verdict, downloads and the voice take are pure modules.

pub mod ai;
pub mod data;
pub mod downloads;
#[cfg(feature = "fixtures")]
pub mod fixture;
pub mod flow;
pub mod parts;
pub mod storage;
pub mod voice;

mod actions;
mod screens;

pub use actions::EVIDENCE_POLL;
#[cfg(test)]
mod ai_tests;
#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use capture_types::{CapturePermissionState as P, CapturePermissions, RecordingSettings};
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, transparent_black, AnyElement, AnyWindowHandle, App, AppContext, Context, Global, Render,
    Task, WeakEntity, Window, WindowKind,
};
use time::OffsetDateTime;

use data::{Backend, Engine, Evidence};
use downloads::Downloads;
use flow::{Feature, Features, Intents, ModelFacts, Saved, Step, Subsystem, WorkItem};
use storage::{Probe, ProbeState};
use voice::Take;

use crate::theme::{self, radius};

pub const SIZE: (f32, f32) = (1120., 800.);
pub const MIN: (f32, f32) = (920., 620.);

/// One permission question on the Permissions screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moment {
    Screen,
    Microphone,
    SystemAudio,
    Accessibility,
}

impl Moment {
    pub fn name(self) -> &'static str {
        match self {
            Moment::Screen => "Screen recording",
            Moment::Microphone => "Microphone",
            Moment::SystemAudio => "System audio",
            Moment::Accessibility => "Accessibility",
        }
    }
    /// The permission kind app-core and the helper window speak.
    pub fn wire(self) -> &'static str {
        match self {
            Moment::Screen => "screen",
            Moment::Microphone => "microphone",
            Moment::SystemAudio => "systemAudio",
            Moment::Accessibility => "accessibility",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FinalePhase {
    Starting,
    Running,
    Idle,
    Failed(String),
}

type Exit = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct Onboarding {
    pub step: Step,
    backend: Arc<dyn Backend>,
    exit: Exit,
    pub completed: bool,
    pub loaded: bool,
    pub error: Option<String>,
    pub draft: RecordingSettings,
    pub perms: CapturePermissions,
    pub gecko: Option<(bool, Vec<String>)>,
    pub features: Features,
    touched: Vec<Feature>,
    ever_saved: bool,
    pub models: Vec<(Subsystem, ModelFacts)>,
    pub engines: Vec<Engine>,
    pub semantic: Vec<(String, String, u64, bool)>,
    pub recommended: Vec<(String, String)>,
    seeded_apps: bool,
    // Permissions
    pub passed: Vec<Moment>,
    pub requesting: Option<Moment>,
    pub sys_prompt_raised: bool,
    /// Screen Recording granted in THIS process: macOS honours it after a relaunch.
    pub screen_needs_relaunch: bool,
    pub acc_asked: bool,
    pub sys_levels: Vec<f32>,
    pub probing_sys: bool,
    sys_poll: Option<Task<()>>,
    // Capture & Storage
    pub probe: Option<Probe>,
    pub probe_state: ProbeState,
    probe_seq: u64,
    pub native_pixels: Option<f64>,
    pub app_pick: screens::AppPick,
    // Change settings
    pub tab: usize,
    pub ai: ai::AiSetup,
    pub undo: Option<(Features, String)>,
    // Setup
    pub dl: Downloads,
    // Voice
    pub take: Take,
    pub owner_enrolled: bool,
    voice_task: Option<Task<()>>,
    // Finale
    pub finale: FinalePhase,
    pub evidence: Option<Evidence>,
    pub rec_started: Option<OffsetDateTime>,
    finale_task: Option<Task<()>>,
    rec_tick: Option<Task<()>>,
    committed: bool,
    // Welcome
    pub welcome_anchor: OffsetDateTime,
    pub welcome_started: Option<Instant>,
}

impl Onboarding {
    /// `exit` runs when the window is closed before onboarding completes
    /// (production: `AppCore::shutdown`).
    pub fn new(
        backend: Arc<dyn Backend>,
        exit: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let unknown = CapturePermissions {
            screen: P::NotDetermined,
            microphone: P::NotDetermined,
            system_audio: P::NotDetermined,
        };
        let mut this = Self {
            step: Step::Welcome,
            backend,
            exit: Rc::new(exit),
            completed: false,
            loaded: false,
            error: None,
            draft: app_core::recording_settings::default_recording_settings(),
            perms: unknown,
            gecko: None,
            features: Features::default(),
            touched: Vec::new(),
            ever_saved: false,
            models: Vec::new(),
            engines: Vec::new(),
            semantic: Vec::new(),
            recommended: Vec::new(),
            seeded_apps: false,
            passed: Vec::new(),
            requesting: None,
            sys_prompt_raised: false,
            screen_needs_relaunch: false,
            acc_asked: false,
            sys_levels: vec![0.; 20],
            probing_sys: false,
            sys_poll: None,
            probe: None,
            probe_state: ProbeState::Checking,
            probe_seq: 0,
            native_pixels: None,
            app_pick: Default::default(),
            tab: 0,
            ai: ai::AiSetup::default(),
            undo: None,
            dl: Downloads::default(),
            take: Take::default(),
            owner_enrolled: false,
            voice_task: None,
            finale: FinalePhase::Starting,
            evidence: None,
            rec_started: None,
            finale_task: None,
            rec_tick: None,
            committed: false,
            welcome_anchor: crate::ui::clock::now(cx),
            welcome_started: None,
        };
        this.load(cx);
        this
    }

    /// Run `work` on the background executor, then `done` on this entity.
    fn run<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&dyn Backend) -> T + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        let b = self.backend.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let out = cx.background_spawn(async move { work(&*b) }).await;
            let _ = this.update(cx, |s, cx| {
                done(s, out, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.run(
            cx,
            |b| b.load(),
            |s, loaded, cx| match loaded {
                Ok(l) => {
                    s.ever_saved = l.ever_saved;
                    s.draft = l.settings;
                    s.perms = l.perms;
                    s.gecko = l.gecko;
                    s.models = l.models;
                    s.engines = l.engines;
                    s.semantic = l.semantic;
                    s.recommended = l.recommended;
                    s.owner_enrolled = l.owner_enrolled;
                    if s.owner_enrolled {
                        s.take.phase = voice::Phase::Enrolled;
                    }
                    s.loaded = true;
                    s.resolve();
                    s.ai_loaded(cx);
                    s.entered(cx);
                }
                Err(e) => s.error = Some(e),
            },
        );
    }

    /// Permission INTENT: system audio counts once its prompt was raised (ADR 0052).
    pub fn intents(&self) -> Intents {
        Intents {
            screen: self.perms.screen == P::Granted,
            microphone: self.perms.microphone == P::Granted,
            system_audio: self.sys_prompt_raised || self.perms.system_audio == P::AssumedWorking,
        }
    }

    /// Re-run the resolver: saved settings win, it only fills gaps; a row the
    /// user flipped on Change settings always travels as a choice.
    pub fn resolve(&mut self) {
        let mut saved = Saved::default();
        if self.ever_saved {
            let d = &self.draft;
            saved.features = vec![
                (Feature::Screen, d.capture_screen),
                (Feature::Microphone, d.capture_microphone),
                (Feature::SystemAudio, d.capture_system_audio),
                (Feature::Ocr, d.ocr.enabled),
                (Feature::Transcription, d.transcription.enabled),
                (
                    Feature::SpeakerSeparation,
                    d.speaker_analysis.separate_speakers,
                ),
                (Feature::SemanticSearch, d.semantic_search.enabled),
                (Feature::AiFeatures, d.ai_runtime.enabled),
            ];
        }
        for f in &self.touched {
            saved.features.push((*f, self.features.get(*f)));
        }
        self.features = flow::resolve(self.intents(), &saved);
    }

    pub fn interval_s(&self) -> f64 {
        storage::LADDER[storage::nearest_ladder_index(self.draft.screen_frame_rate)]
    }

    pub fn pixels(&self) -> f64 {
        storage::video_pixels(&self.draft.screen_resolution, self.native_pixels)
    }

    pub fn work_list(&self) -> Vec<WorkItem> {
        flow::work_list(&self.features, &self.models)
    }

    pub fn download_bytes(&self) -> f64 {
        flow::work_bytes(&self.work_list()) as f64
    }

    /// The Capture & Storage hold (the two hard gates), or `None`.
    pub fn capture_block(&self) -> Option<String> {
        if self.step != Step::Capture {
            return None;
        }
        storage::block_reason(
            self.probe.as_ref(),
            self.download_bytes(),
            self.interval_s(),
            self.pixels(),
        )
    }

    /// The two storage gates on Capture & Storage, plus the relaunch hold on
    /// Permissions (a same-process screen grant ScreenCaptureKit can't use yet).
    pub fn can_continue(&self) -> bool {
        if self.step == Step::Permissions && self.screen_needs_relaunch {
            return false;
        }
        flow::can_continue(self.step, self.capture_block().as_deref())
    }

    // ── Navigation ─────────────────────────────────────────────────────────
    pub fn go(&mut self, step: Step, cx: &mut Context<Self>) {
        self.step = step;
        self.undo = None;
        self.entered(cx);
        cx.notify();
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        if !self.can_continue() {
            return;
        }
        // Permissions are the resolver's input: a grant made there must reach the settings.
        if self.step == Step::Permissions {
            self.resolve();
        }
        if let Some(to) = self.step.next() {
            self.go(to, cx);
        }
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(to) = self.step.back() {
            self.go(to, cx);
        }
    }

    /// Work a step starts on arrival.
    fn entered(&mut self, cx: &mut Context<Self>) {
        if !self.loaded {
            return;
        }
        match self.step {
            // The moments start over on every arrival (a denial is asked again).
            Step::Permissions => {
                self.passed.clear();
                self.acc_asked = false;
            }
            Step::Capture => {
                self.seed_recommended_apps();
                self.run_probe(cx);
            }
            Step::Setup | Step::Voice => self.drive_downloads(cx),
            Step::Finale => self.commit(cx),
            _ => {}
        }
        if self.step != Step::Permissions {
            self.sys_poll = None;
            self.probing_sys = false;
        }
    }

    /// The strip's close light (and the window's own close).
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match flow::close_action(self.completed) {
            flow::CloseAction::Exit => (self.exit.clone())(window, cx),
            flow::CloseAction::Close => window.remove_window(),
        }
    }

    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::request_fade_frames(window, cx);
        let p = theme::tokens(cx);
        if self.native_pixels.is_none() {
            self.native_pixels = window
                .display(cx)
                .map(|d| {
                    let b = d.bounds().size;
                    let s = window.scale_factor() as f64;
                    f64::from(b.width) * f64::from(b.height) * s * s
                })
                .filter(|v| *v > 0.);
        }
        let (main, foot): (AnyElement, Option<AnyElement>) = match self.step {
            Step::Welcome => (self.welcome(window, cx), None),
            Step::Permissions => self.permissions(window, cx),
            Step::Capture => self.capture(window, cx),
            Step::YourSettings => self.your_settings(cx),
            Step::Change => self.change(window, cx),
            Step::Setup => self.setup(cx),
            Step::Voice => self.voice_step(window, cx),
            Step::Finale => self.finale_step(cx),
        };
        let close = cx.listener(|this, _: &(), window, cx| this.request_close(window, cx));
        div()
            .id("onboarding")
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(radius::LG)
            .bg(p.bg)
            .font_family(theme::FONT_UI)
            .text_color(p.text)
            .child(parts::strip(self.step, move |w, cx| close(&(), w, cx), &p))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(parts::texture(&p))
                    .child(main)
                    .children(foot),
            )
    }
}

/// The open onboarding window, so Complete Setup brings it forward.
struct Open(AnyWindowHandle, WeakEntity<Onboarding>);

impl Global for Open {}

/// Onboarding is done (the persisted flag). No app core (tests) reads as done.
pub fn is_complete(cx: &App) -> bool {
    cx.try_global::<crate::bridge::events::Core>()
        .is_none_or(|c| crate::native::onboarding_complete(&c.0))
}

/// Open the onboarding window (or bring the open one forward). Closing it
/// before completion runs `AppCore::shutdown`.
pub fn open(cx: &mut App) {
    if let Some(Open(handle, _)) = cx.try_global::<Open>() {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            cx.activate(true);
            return;
        }
    }
    // Completed (the window was closed after the Finale): main is the way back.
    if is_complete(cx) {
        crate::window::ensure_main(cx);
        return;
    }
    let Some(core) = cx
        .try_global::<crate::bridge::events::Core>()
        .map(|c| c.0.clone())
    else {
        return;
    };
    let developer = crate::shell::developer_options(cx);
    let mut options = crate::small::options(SIZE, MIN, WindowKind::Normal, cx);
    options.focus = true;
    let backend: Arc<dyn Backend> = Arc::new(data::CoreBackend(core.clone()));
    let mut built = None;
    let opened = cx.open_window(options, |window, cx| {
        crate::native::apply_sharing_type(window, developer);
        crate::native::chrome::frameless(window, 12.);
        let exit_core = core.clone();
        let view = cx.new(|cx| {
            Onboarding::new(
                backend,
                move |_, _| exit_core.shutdown(app_core::ExitKind::Quit),
                cx,
            )
        });
        // Coming back from System Settings: pick up a grant without Re-check.
        view.update(cx, |_, cx| {
            cx.observe_window_activation(window, |o, window, cx| {
                if window.is_window_active() && o.step == Step::Permissions {
                    o.recheck(cx);
                }
            })
            .detach();
        });
        let v = view.downgrade();
        built = Some(v.clone());
        window.on_window_should_close(cx, move |window, cx| {
            let _ = v.update(cx, |o, cx| o.request_close(window, cx));
            false
        });
        cx.new(|cx| Root::new(view, window, cx).bg(transparent_black()))
    });
    match (opened, built) {
        (Ok(handle), Some(view)) => {
            let any: AnyWindowHandle = handle.into();
            let _ = any.update(cx, |_, window, _| window.activate_window());
            cx.set_global(Open(any, view));
            cx.activate(true);
            crate::native::dock::set_regular(true, cx);
        }
        (Err(e), _) => log::error!("onboarding window failed to open: {e:#}"),
        _ => {}
    }
}

/// Route a `CoreEvent` the open onboarding window cares about (download
/// progress, the ChatGPT sign-in outcome).
pub fn on_event(event: &app_core::CoreEvent, cx: &mut App) {
    use app_core::CoreEvent as E;
    if let (E::ChatgptLoginUpdate(v), Some(Open(_, view))) = (event, cx.try_global::<Open>()) {
        let view = view.clone();
        let _ = view.update(cx, |o, cx| o.on_chatgpt_update(v, cx));
        return;
    }
    let payload = match event {
        E::AudioTranscriptionModelDownloadProgress(p) => serde_json::to_value(p),
        E::SpeakerAnalysisModelDownloadProgress(p) => serde_json::to_value(p),
        E::SemanticSearchModelDownloadProgress(p) => serde_json::to_value(p),
        _ => return,
    };
    let (Ok(payload), Some(Open(_, view))) = (payload, cx.try_global::<Open>()) else {
        return;
    };
    let view = view.clone();
    let _ = view.update(cx, |o, cx| o.on_download_event(&payload, cx));
}
