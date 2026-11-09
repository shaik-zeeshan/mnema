//! Onboarding's inline AI provider setup (`AiSetup.svelte`,
//! `onboarding-ai.svelte.ts`, `ai-readiness.ts`). Settings lives in the main
//! window, which doesn't open until onboarding completes, so the provider is
//! connected here: pick a kind, paste a key (or sign in with ChatGPT, or scan
//! for a local engine), and the model list that comes back is the proof.
//!
//! **Ready means verified.** A default model counts only when its provider
//! answered `verify_ai_provider` live this session and that listing still has
//! the model. A key in the vault is not evidence, and no kind is exempt.
//! Unreachable (offline, engine not running, ChatGPT's auth host silent) is not
//! a rejected key: the two read differently (ADR 0048 / 0058).
//!
//! Providers and the default model are draft settings (`draft.ai_runtime`),
//! committed by the Finale's one save. Keys go to the vault eagerly, by
//! instance id; the typed string is read once, handed over and cleared.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use capture_types::{AiEngineRef, AiProviderConfig, AiProviderKind};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, Subscription, Task, Window};
use serde_json::Value;

use super::flow::Feature;
use super::Onboarding;
use crate::chat::format::{provider_instance_label, url_host};
use crate::settings::sections::providers::{local_default, new_id};

/// A typed key verifies itself this long after the last keystroke.
pub const KEY_DEBOUNCE: Duration = Duration::from_millis(650);

/// The ports a local scan asks: the two default endpoints plus the port LM
/// Studio and friends serve an OpenAI-compatible API on. Nothing else is guessed.
pub const SCAN_TARGETS: [(u16, AiProviderKind, &str); 3] = [
    (11434, AiProviderKind::Ollama, "Ollama"),
    (8080, AiProviderKind::Llamafile, "Llamafile"),
    (1234, AiProviderKind::Llamafile, "OpenAI-compatible server"),
];

/// Why a verification failed, by what the user should do about it.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// The provider answered and refused the credential.
    Rejected,
    /// Nothing answered: offline, the engine isn't running, a timeout. The key
    /// hasn't been judged, so this never reads as "invalid".
    Unreachable,
    /// ChatGPT has no usable token set (`needs_reconnect:<id>`).
    NeedsSignIn,
    Other(String),
}

impl Failure {
    /// `classify_listing_failure`'s reasons (case-insensitive prefixes).
    pub fn classify(reason: &str) -> Self {
        let r = reason.trim().to_ascii_lowercase();
        if r.starts_with("needs_reconnect:") {
            Failure::NeedsSignIn
        } else if r.starts_with("provider_unreachable:") || r == "unreachable" {
            Failure::Unreachable
        } else if r == "authentication failed" {
            Failure::Rejected
        } else {
            Failure::Other(reason.trim().to_string())
        }
    }

    pub fn word(&self) -> String {
        match self {
            Failure::Rejected => "key rejected".into(),
            Failure::Unreachable => "can't reach it".into(),
            Failure::NeedsSignIn => "sign in to finish".into(),
            Failure::Other(s) => s.clone(),
        }
    }
}

/// One `verify_ai_provider` round trip, per instance id. Session-only.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    Checking,
    Live {
        models: Vec<String>,
        latency_ms: u64,
    },
    Failed(Failure),
}

impl Check {
    pub fn models(&self) -> &[String] {
        match self {
            Check::Live { models, .. } => models,
            _ => &[],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Login {
    Starting,
    Waiting { code: String, url: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    Trying,
    Live { models: usize, latency_ms: u64 },
    Dead,
}

pub struct AiSetup {
    /// The fork: `None` unanswered, `Some(false)` later, `Some(true)` now.
    pub mode: Option<bool>,
    pub kind: AiProviderKind,
    pub key: Option<Entity<InputState>>,
    pub url: Option<Entity<InputState>>,
    /// "or an engine on your network", for a local kind.
    pub endpoint: Option<Entity<InputState>>,
    pub checks: HashMap<String, Check>,
    tickets: HashMap<String, u64>,
    pub keys_saved: HashSet<String>,
    /// The ChatGPT device login in flight: `(instance id, phase)`.
    pub login: Option<(String, Login)>,
    /// The instance the status line reports on.
    pub focus: Option<String>,
    /// A pre-flight problem (no base URL) or a failed sign-in, in words.
    pub note: Option<String>,
    pub scan: Option<Vec<Probe>>,
    /// A credential clear is in flight: connecting waits (a same-kind re-add
    /// reuses the bare kind id, ADR 0035).
    pub removing: bool,
    /// A persisted default the provider no longer lists, cleared on load.
    pub restored_note: Option<String>,
    debounce: Option<Task<()>>,
    subs: Vec<Subscription>,
}

impl Default for AiSetup {
    fn default() -> Self {
        Self {
            mode: None,
            kind: AiProviderKind::Ollama,
            key: None,
            url: None,
            endpoint: None,
            checks: HashMap::new(),
            tickets: HashMap::new(),
            keys_saved: HashSet::new(),
            login: None,
            focus: None,
            note: None,
            scan: None,
            removing: false,
            restored_note: None,
            debounce: None,
            subs: Vec::new(),
        }
    }
}

/// `aiReadinessMissing`: the short reason AI can't answer yet, `None` when it can.
pub fn missing(
    providers: &[AiProviderConfig],
    checks: &HashMap<String, Check>,
    default: Option<&AiEngineRef>,
) -> Option<String> {
    if providers.is_empty() {
        return Some("Connect a reasoning engine to use Ask AI.".into());
    }
    let Some(d) = default.filter(|d| !d.model.trim().is_empty()) else {
        let live = providers
            .iter()
            .any(|p| matches!(checks.get(&p.id), Some(Check::Live { .. })));
        if live {
            return Some("Choose a default model.".into());
        }
        // Nothing answered yet: say what the newest connection is doing.
        return providers
            .iter()
            .rev()
            .find_map(|p| match checks.get(&p.id)? {
                Check::Checking => {
                    Some(format!("Verifying {}\u{2026}", provider_instance_label(p)))
                }
                c => failed(p, c),
            })
            .or_else(|| Some("No engine has verified yet \u{2014} nothing can answer.".into()));
    };
    let Some(p) = providers.iter().find(|p| p.id == d.provider) else {
        return Some("Pick a default model from a connected engine.".into());
    };
    let label = provider_instance_label(p);
    match checks.get(&p.id) {
        Some(Check::Live { models, .. }) => (!models.contains(&d.model))
            .then(|| format!("{label} no longer lists {} \u{2014} pick another.", d.model)),
        Some(c @ Check::Failed(_)) => failed(p, c),
        Some(Check::Checking) => Some(format!("Verifying {label}\u{2026}")),
        None => Some(format!("Verify {label} before finishing.")),
    }
}

fn failed(p: &AiProviderConfig, c: &Check) -> Option<String> {
    let label = provider_instance_label(p);
    match c {
        Check::Failed(Failure::Unreachable) => Some(format!(
            "Can\u{2019}t reach {label} right now \u{2014} nothing was judged."
        )),
        Check::Failed(f) => Some(format!("{label} is not answering \u{2014} {}.", f.word())),
        _ => None,
    }
}

/// Where a provider's prompts go: the endpoint host, or the cloud's.
pub fn host_of(kind: AiProviderKind, base_url: &str) -> String {
    match kind {
        AiProviderKind::Anthropic => "api.anthropic.com".into(),
        AiProviderKind::Openai => "api.openai.com".into(),
        AiProviderKind::Chatgpt => "chatgpt.com".into(),
        _ if !base_url.trim().is_empty() => url_host(base_url),
        AiProviderKind::OpenaiCompatible => "the host you type".into(),
        _ => url_host(local_default(kind)),
    }
}

impl Onboarding {
    pub fn ai_missing(&self) -> Option<String> {
        let ai = &self.draft.ai_runtime;
        missing(&ai.providers, &self.ai.checks, ai.default_model.as_ref())
    }

    /// AI can answer: a verified engine lists the chosen model right now.
    pub fn ai_ready(&self) -> bool {
        self.ai_missing().is_none()
    }

    fn provider(&self, id: &str) -> Option<&AiProviderConfig> {
        self.draft.ai_runtime.providers.iter().find(|p| p.id == id)
    }

    /// The key / URL fields need a window, so they're made on first render.
    pub(super) fn ai_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai.key.is_some() {
            return;
        }
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("paste your key \u{2014} it verifies itself")
        });
        let url =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://api.example.com/v1"));
        self.ai.subs.push(
            cx.subscribe_in(&key, window, |o, _, e: &InputEvent, window, cx| {
                if !matches!(e, InputEvent::Change) {
                    return;
                }
                o.ai.debounce = Some(cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(KEY_DEBOUNCE).await;
                    let _ = this.update_in(cx, |o, window, cx| o.ai_connect_cloud(window, cx));
                }));
            }),
        );
        self.ai.key = Some(key);
        self.ai.url = Some(url);
        self.ai.endpoint = Some(cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("or an engine on your network \u{2014} studio.local:11434")
        }));
    }

    /// After load: answer the fork for a returning user, learn which keys are
    /// stored, and re-prove a persisted default (it may be gone since).
    pub(super) fn ai_loaded(&mut self, cx: &mut Context<Self>) {
        let ai = self.draft.ai_runtime.clone();
        if !ai.providers.is_empty() {
            self.ai.mode = Some(true);
        }
        let ids: Vec<String> = ai.providers.iter().map(|p| p.id.clone()).collect();
        self.run(
            cx,
            move |b| {
                ids.into_iter()
                    .filter(|id| b.has_key(id))
                    .collect::<Vec<_>>()
            },
            |s, saved, _| s.ai.keys_saved.extend(saved),
        );
        if let Some(d) = ai.default_model {
            self.ai_verify(d.provider, cx);
        }
    }

    pub fn ai_set_mode(&mut self, now: bool, cx: &mut Context<Self>) {
        self.ai.mode = Some(now);
        if !now && self.features.ai_features {
            self.toggle(Feature::AiFeatures, cx);
        }
        cx.notify();
    }

    pub fn ai_set_kind(&mut self, kind: AiProviderKind, cx: &mut Context<Self>) {
        self.ai.kind = kind;
        self.ai.scan = None;
        self.ai.note = None;
        self.ai.focus = None;
        cx.notify();
    }

    fn ai_add(&mut self, kind: AiProviderKind, base_url: String) -> Option<String> {
        if self.ai.removing {
            return None;
        }
        let providers = &mut self.draft.ai_runtime.providers;
        let id = new_id(kind, providers);
        providers.push(AiProviderConfig {
            id: id.clone(),
            kind,
            label: String::new(),
            base_url,
        });
        Some(id)
    }

    /// Read the typed key ONCE, clear the field, vault it, then verify.
    pub fn ai_connect_cloud(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ai.debounce = None;
        let (Some(key_in), Some(url_in)) = (self.ai.key.clone(), self.ai.url.clone()) else {
            return;
        };
        let key = key_in.read(cx).value().trim().to_string();
        if key.chars().count() < 6 {
            return;
        }
        let kind = self.ai.kind;
        let base = url_in.read(cx).value().trim().to_string();
        if kind == AiProviderKind::OpenaiCompatible && base.is_empty() {
            self.ai.note =
                Some("Add the base URL first \u{2014} there is nowhere to send the key.".into());
            cx.notify();
            return;
        }
        let base = if kind == AiProviderKind::OpenaiCompatible {
            base
        } else {
            String::new()
        };
        let Some(id) = self.ai_add(kind, base) else {
            return;
        };
        key_in.update(cx, |s, cx| s.set_value("", window, cx));
        self.ai.note = None;
        self.ai.focus = Some(id.clone());
        self.ai.checks.insert(id.clone(), Check::Checking);
        let vid = id.clone();
        self.run(
            cx,
            move |b| b.save_key(&vid, &key),
            move |s, saved, cx| match saved {
                Ok(()) => {
                    s.ai.keys_saved.insert(id.clone());
                    s.ai_verify(id, cx);
                }
                Err(e) => {
                    log::warn!("onboarding: couldn't store the {id} key in the vault");
                    s.ai.checks.insert(id, Check::Failed(Failure::Other(e)));
                }
            },
        );
        cx.notify();
    }

    pub fn ai_connect_local(&mut self, kind: AiProviderKind, url: String, cx: &mut Context<Self>) {
        if let Some(id) = self.ai_add(kind, url) {
            self.ai_verify(id, cx);
        }
    }

    /// "or an engine on your network": the URL field, for a local kind.
    pub fn ai_connect_manual(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url_in) = self.ai.endpoint.clone() else {
            return;
        };
        let typed = url_in.read(cx).value().trim().to_string();
        if typed.is_empty() {
            return;
        }
        url_in.update(cx, |s, cx| s.set_value("", window, cx));
        let url = if typed.contains("://") {
            typed
        } else {
            format!("http://{typed}")
        };
        self.ai_connect_local(self.ai.kind, url, cx);
    }

    /// THE proof: list the instance's models with what the vault holds,
    /// against the draft list. Only the newest probe per id may land.
    pub fn ai_verify(&mut self, id: String, cx: &mut Context<Self>) {
        if self.provider(&id).is_none() {
            return;
        }
        let ticket = self.ai.tickets.get(&id).copied().unwrap_or(0) + 1;
        self.ai.tickets.insert(id.clone(), ticket);
        self.ai.checks.insert(id.clone(), Check::Checking);
        self.ai.focus = Some(id.clone());
        let providers = self.draft.ai_runtime.providers.clone();
        let vid = id.clone();
        self.run(
            cx,
            move |b| b.verify(&vid, &providers),
            move |s, result, cx| {
                if s.ai.tickets.get(&id) != Some(&ticket) || s.provider(&id).is_none() {
                    return;
                }
                let was = s.ai_ready();
                let check = match result {
                    Ok((models, latency_ms)) => Check::Live { models, latency_ms },
                    Err(e) => Check::Failed(Failure::classify(&e)),
                };
                s.ai.checks.insert(id.clone(), check);
                s.ai_settled(&id, was, cx);
            },
        );
        cx.notify();
    }

    /// After a proof or a pick: a persisted default the engine dropped is
    /// cleared, the first engine to answer seeds the default, and becoming
    /// ready switches the AI row on (the one auto-enable).
    fn ai_settled(&mut self, id: &str, was: bool, cx: &mut Context<Self>) {
        let models = self.ai.checks.get(id).map(|c| c.models().to_vec());
        let live = matches!(self.ai.checks.get(id), Some(Check::Live { .. }));
        let label = self.provider(id).map(provider_instance_label);
        let ai = &mut self.draft.ai_runtime;
        let mut cleared = false;
        if let (Some(models), Some(d)) = (&models, ai.default_model.as_ref()) {
            if live && d.provider == id && !models.contains(&d.model) {
                self.ai.restored_note = Some(format!(
                    "{} no longer lists {} \u{2014} choose another default model.",
                    label.unwrap_or_default(),
                    d.model
                ));
                ai.default_model = None;
                cleared = true;
            }
        }
        if ai.default_model.is_none() && !cleared {
            if let Some(first) = models.and_then(|m| m.first().cloned()) {
                ai.default_model = Some(AiEngineRef {
                    provider: id.to_string(),
                    model: first,
                });
            }
        }
        if !was && self.ai_ready() && !self.features.ai_features {
            self.toggle(Feature::AiFeatures, cx);
        }
    }

    pub fn ai_pick(&mut self, provider: String, model: String, cx: &mut Context<Self>) {
        let was = self.ai_ready();
        self.ai.restored_note = None;
        self.draft.ai_runtime.default_model = Some(AiEngineRef {
            provider: provider.clone(),
            model,
        });
        self.ai_settled(&provider, was, cx);
        cx.notify();
    }

    pub fn ai_remove(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(removed) = self.provider(&id).cloned() else {
            return;
        };
        let ai = &mut self.draft.ai_runtime;
        ai.providers.retain(|p| p.id != id);
        if ai.default_model.as_ref().is_some_and(|d| d.provider == id) {
            ai.default_model = None;
        }
        *self.ai.tickets.entry(id.clone()).or_default() += 1;
        self.ai.checks.remove(&id);
        if self.ai.login.as_ref().is_some_and(|(l, _)| *l == id) {
            self.ai_chatgpt_cancel(cx);
        }
        if !removed.kind.is_local() {
            self.ai.removing = true;
            self.run(
                cx,
                move |b| b.clear_key(&id).map(|_| id),
                |s, cleared, _| {
                    s.ai.removing = false;
                    match cleared {
                        Ok(id) => {
                            s.ai.keys_saved.remove(&id);
                        }
                        Err(_) => log::warn!("onboarding: couldn't clear a removed provider's key"),
                    }
                },
            );
        }
        cx.notify();
    }

    /// Ask the three usual ports at once with an ephemeral instance that never
    /// enters the draft; nothing is added until "Use this".
    pub fn ai_scan(&mut self, cx: &mut Context<Self>) {
        self.ai.scan = Some(vec![Probe::Trying; SCAN_TARGETS.len()]);
        for (ix, (port, kind, _)) in SCAN_TARGETS.into_iter().enumerate() {
            self.run(
                cx,
                move |b| {
                    let url = format!("http://localhost:{port}");
                    let id = format!("scan-{url}");
                    let probe = AiProviderConfig {
                        id: id.clone(),
                        kind,
                        label: String::new(),
                        base_url: url,
                    };
                    b.verify(&id, &[probe])
                },
                move |s, r, _| {
                    if let Some(rows) = s.ai.scan.as_mut() {
                        rows[ix] = match r {
                            Ok((m, latency_ms)) => Probe::Live {
                                models: m.len(),
                                latency_ms,
                            },
                            Err(_) => Probe::Dead,
                        };
                    }
                },
            );
        }
        cx.notify();
    }

    /// A connected instance already serves `localhost:<port>`.
    pub fn ai_port_connected(&self, port: u16) -> bool {
        let want = format!("localhost:{port}");
        self.draft
            .ai_runtime
            .providers
            .iter()
            .any(|p| p.kind.is_local() && host_of(p.kind, &p.base_url) == want)
    }

    /// "Sign in with ChatGPT": the click that creates the instance also starts
    /// the device login and opens the page.
    pub fn ai_chatgpt_sign_in(&mut self, cx: &mut Context<Self>) {
        let existing = self
            .draft
            .ai_runtime
            .providers
            .iter()
            .find(|p| p.kind == AiProviderKind::Chatgpt)
            .map(|p| p.id.clone());
        let Some(id) = existing.or_else(|| self.ai_add(AiProviderKind::Chatgpt, String::new()))
        else {
            return;
        };
        self.ai.note = None;
        self.ai.focus = Some(id.clone());
        self.ai.login = Some((id.clone(), Login::Starting));
        let bid = id.clone();
        self.run(
            cx,
            move |b| {
                let prompt = b.chatgpt_begin(&bid);
                if let Ok((_, url)) = &prompt {
                    b.open_url(url);
                }
                prompt
            },
            move |s, prompt, _| {
                if s.ai.login.as_ref().map(|(l, _)| l) != Some(&id) {
                    return;
                }
                match prompt {
                    Ok((code, url)) => s.ai.login = Some((id, Login::Waiting { code, url })),
                    Err(e) => {
                        s.ai.login = None;
                        s.ai.note = Some(e);
                    }
                }
            },
        );
        cx.notify();
    }

    pub fn ai_chatgpt_open(&mut self, cx: &mut Context<Self>) {
        if let Some((_, Login::Waiting { url, .. })) = self.ai.login.clone() {
            self.run(cx, move |b| b.open_url(&url), |_, _, _| {});
        }
    }

    /// Give up on a pending login, and tell the backend: an abandoned poll
    /// keeps hitting auth.openai.com and can rate-limit the next real one.
    pub fn ai_chatgpt_cancel(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.ai.login.take() {
            self.run(cx, move |b| b.chatgpt_cancel(&id), |_, _, _| {});
        }
        cx.notify();
    }

    /// `chatgpt_login_update`: the one terminal outcome of the 15-minute poll.
    /// Owned here, not by the AI tab, so a tab change can't drop it.
    pub fn on_chatgpt_update(&mut self, v: &Value, cx: &mut Context<Self>) {
        let Some(id) = v["providerId"].as_str().map(str::to_string) else {
            return;
        };
        if self.provider(&id).is_none() {
            return;
        }
        if self.ai.login.as_ref().is_some_and(|(l, _)| *l == id) {
            self.ai.login = None;
        }
        if v["connected"].as_bool() == Some(true) {
            self.ai.keys_saved.insert(id.clone());
            self.ai_verify(id, cx);
        } else {
            *self.ai.tickets.entry(id.clone()).or_default() += 1;
            let why = v["error"]
                .as_str()
                .unwrap_or("the ChatGPT sign-in didn\u{2019}t complete");
            self.ai
                .checks
                .insert(id, Check::Failed(Failure::Other(why.into())));
        }
        cx.notify();
    }
}
