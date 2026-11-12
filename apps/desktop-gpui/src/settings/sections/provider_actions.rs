//! What the provider rows' buttons do (`ProviderRow.svelte`,
//! `ChatgptConnect.svelte`, the `ai-runtime` store and the controller's
//! add / remove): the per-row fields, Save key / Clear, Remove provider, the
//! ChatGPT device sign-in, and the AI runtime's Test connection. Every
//! backend call runs off the main thread; nothing here logs a key.

use capture_types::{AiProviderConfig, AiProviderKind};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Window};
use serde_json::Value;

use super::provider_row::{Login, PvInputs};
use crate::chat::format::provider_instance_label;
use crate::settings::commands::{confirm_as, confirm_then};
use crate::settings::Settings;

impl Settings {
    /// One set of fields per provider instance (an unsaved key belongs to its
    /// row, as Svelte's `aiProviderKeyInputs[id]`). Run from `render`, which has
    /// the window an `InputState` needs; a re-read refills label and URL.
    pub(crate) fn ensure_provider_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.draft.as_ref() else { return };
        let providers = d.ai_runtime.providers.clone();
        self.ui
            .pv
            .inputs
            .retain(|id, _| providers.iter().any(|p| &p.id == id));
        for pv in providers {
            if let Some(i) = self.ui.pv.inputs.get(&pv.id) {
                if self.needs_sync {
                    for (e, v) in [(&i.label, &pv.label), (&i.base_url, &pv.base_url)] {
                        if e.read(cx).value().as_ref() != v.as_str() {
                            e.update(cx, |s, cx| s.set_value(v.clone(), window, cx));
                        }
                    }
                }
                continue;
            }
            let field = |v: &str, ph: &str, window: &mut Window, cx: &mut Context<Self>| {
                let (v, ph) = (v.to_string(), ph.to_string());
                cx.new(|cx| {
                    let mut s = InputState::new(window, cx).placeholder(ph);
                    s.set_value(v, window, cx);
                    s
                })
            };
            let label = field(
                &pv.label,
                crate::chat::format::kind_label(pv.kind),
                window,
                cx,
            );
            let url_ph = match pv.kind {
                AiProviderKind::OpenaiCompatible => "https://api.fireworks.ai/inference/v1",
                k if k.is_local() => super::providers::local_default(k),
                _ => "",
            };
            let base_url = field(&pv.base_url, url_ph, window, cx);
            let key = cx.new(|cx| InputState::new(window, cx).masked(true));
            let id = pv.id.clone();
            let subs = vec![
                cx.subscribe_in(&label, window, {
                    let id = id.clone();
                    move |s, e, ev: &InputEvent, _, cx| {
                        if matches!(ev, InputEvent::Change) {
                            let v = e.read(cx).value().to_string();
                            s.edit_provider(&id, |p| p.label = v, cx);
                        }
                    }
                }),
                cx.subscribe_in(&base_url, window, {
                    let id = id.clone();
                    move |s, e, ev: &InputEvent, _, cx| {
                        if matches!(ev, InputEvent::Change) {
                            let v = e.read(cx).value().to_string();
                            s.edit_provider(&id, |p| p.base_url = v, cx);
                            s.reset_test();
                            s.ui.reach.remove(&id);
                        }
                    }
                }),
                // The row stays open while a typed key is unsaved.
                cx.subscribe_in(&key, window, |_, _, ev: &InputEvent, _, cx| {
                    if matches!(ev, InputEvent::Change) {
                        cx.notify();
                    }
                }),
            ];
            self.ui.pv.inputs.insert(
                id,
                PvInputs {
                    label,
                    base_url,
                    key,
                    saved_placeholder: None,
                    _subs: subs,
                },
            );
        }
        // A landed save / clear forgets the typed key.
        for id in std::mem::take(&mut self.ui.pv.clear_key_field) {
            if let Some(i) = self.ui.pv.inputs.get(&id) {
                i.key.update(cx, |s, cx| s.set_value("", window, cx));
            }
        }
        // The key's placeholder follows whether one is saved.
        for (id, i) in self.ui.pv.inputs.iter_mut() {
            let saved = self.data.keys_saved.contains(id);
            if i.saved_placeholder != Some(saved) {
                i.saved_placeholder = Some(saved);
                let ph = if saved {
                    "A key is saved \u{2014} enter a new one to replace it"
                } else {
                    "Paste your provider API key"
                };
                i.key.update(cx, |s, cx| s.set_placeholder(ph, window, cx));
            }
        }
    }

    fn edit_provider(
        &mut self,
        id: &str,
        f: impl FnOnce(&mut AiProviderConfig),
        cx: &mut Context<Self>,
    ) {
        self.edit(
            |d| {
                if let Some(p) = d.ai_runtime.providers.iter_mut().find(|p| p.id == id) {
                    f(p)
                }
            },
            cx,
        );
    }

    /// The typed (unsaved) key of one row.
    pub(crate) fn typed_key(&self, id: &str, cx: &gpui_kit::App) -> String {
        self.ui
            .pv
            .inputs
            .get(id)
            .map(|i| i.key.read(cx).value().trim().to_string())
            .unwrap_or_default()
    }

    /// The last test-connection line names the tested provider / model; a
    /// change to either makes it describe nothing.
    pub(crate) fn reset_test(&mut self) {
        self.ui.pv.test.result = None;
        self.ui.pv.test.raw_open = false;
    }

    /// `addAiProvider`: the new instance's row opens itself.
    pub(crate) fn add_provider(&mut self, kind: AiProviderKind, cx: &mut Context<Self>) {
        self.ui.add_provider_open = false;
        if self.ui.pv.removing {
            return;
        }
        let mut added = None;
        self.edit(
            |d| {
                let id = super::providers::new_id(kind, &d.ai_runtime.providers);
                added = Some(id.clone());
                d.ai_runtime.providers.push(AiProviderConfig {
                    id,
                    kind,
                    label: String::new(),
                    base_url: String::new(),
                });
            },
            cx,
        );
        if added.is_some() {
            self.ui.provider_open = added;
        }
    }

    /// Save key: the typed key goes to the vault by instance id; the field
    /// keeps it (disabled) until the save lands, and forgets it only then.
    pub fn save_provider_key(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.typed_key(&id, cx);
        if self.ui.pv.saving.is_some() {
            return;
        }
        if key.is_empty() {
            self.ui
                .pv
                .key_errors
                .insert(id, "Enter an API key first.".into());
            cx.notify();
            return;
        }
        // A save that lost the race to a removal must not vault an orphan key.
        if !self.d().ai_runtime.providers.iter().any(|p| p.id == id) {
            return;
        }
        let _ = window;
        self.ui.pv.saving = Some(id.clone());
        self.ui.pv.key_errors.remove(&id);
        let backend = self.backend.clone();
        let vid = id.clone();
        let job = cx.background_spawn(async move { backend.save_key(&vid, &key) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.pv.saving = None;
                match result {
                    Ok(ending) => {
                        s.data.keys_saved.insert(id.clone());
                        match ending {
                            Some(e) => s.data.key_endings.insert(id.clone(), e),
                            None => s.data.key_endings.remove(&id),
                        };
                        s.ui.pv.clear_key_field.insert(id.clone());
                        s.reset_test();
                        s.ui.reach.remove(&id);
                        s.refresh(cx);
                    }
                    Err(e) => {
                        log::warn!("settings: couldn't store a provider key in the vault");
                        s.ui.pv.key_errors.insert(id, e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Clear: asks first (the key has no undo), then deletes it from the vault.
    pub fn clear_provider_key(&mut self, id: String, cx: &mut Context<Self>) {
        if self.ui.pv.saving.is_some() {
            return;
        }
        let label = self.provider_label(&id);
        // Armed before the dialog so a second click can't open a second one.
        self.ui.pv.saving = Some(id.clone());
        cx.notify();
        confirm_then(
            cx,
            "Delete this API key?",
            &format!("Deleting the API key for \u{201c}{label}\u{201d} deletes it from this Mac right away. Any AI feature using this provider will stop working until you enter a new key."),
            "Delete Key",
            "Keep Key",
            move |s, ok, cx| {
                if ok {
                    s.revoke_key(id, cx);
                } else {
                    s.ui.pv.saving = None;
                    cx.notify();
                }
            },
        );
    }

    /// `ai_runtime_clear_provider_key` for a key-kind row, after its confirm.
    fn revoke_key(&mut self, id: String, cx: &mut Context<Self>) {
        self.ui.pv.saving = Some(id.clone());
        self.ui.pv.key_errors.remove(&id);
        let backend = self.backend.clone();
        let vid = id.clone();
        let job = cx.background_spawn(async move { backend.clear_key(&vid) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.pv.saving = None;
                match result {
                    Ok(()) => {
                        s.data.keys_saved.remove(&id);
                        s.data.key_endings.remove(&id);
                        s.ui.pv.clear_key_field.insert(id.clone());
                        s.reset_test();
                        s.ui.reach.remove(&id);
                        s.refresh(cx);
                    }
                    Err(e) => {
                        s.ui.pv.key_errors.insert(id, e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn provider_label(&self, id: &str) -> String {
        let d = self.d();
        d.ai_runtime
            .providers
            .iter()
            .find(|p| p.id == id)
            .map(provider_instance_label)
            .unwrap_or_else(|| id.to_string())
    }

    /// `removeAiProvider`: a cloud instance asks first (its key goes with it);
    /// a local one just goes.
    pub(crate) fn remove_provider(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(removed) = self
            .d()
            .ai_runtime
            .providers
            .into_iter()
            .find(|p| p.id == id)
        else {
            return;
        };
        if removed.kind.is_local() {
            self.drop_provider(removed, cx);
            return;
        }
        let label = provider_instance_label(&removed);
        confirm_as(
            cx,
            "Remove this provider?",
            &format!("Removing \u{201c}{label}\u{201d} deletes its API key from this Mac right away. Any AI feature using this provider will stop working until you reconnect it."),
            "Remove & Delete Key",
            "Keep Provider",
            move |s, cx| s.drop_provider(removed, cx),
        );
    }

    fn drop_provider(&mut self, removed: AiProviderConfig, cx: &mut Context<Self>) {
        let id = removed.id.clone();
        self.edit(
            |d| {
                d.ai_runtime.providers.retain(|p| p.id != id);
                if d.ai_runtime
                    .default_model
                    .as_ref()
                    .is_some_and(|m| m.provider == id)
                {
                    d.ai_runtime.default_model = None;
                }
                // Ask AI skips a model whose provider is gone; a same-kind re-add
                // (which reuses the id) must not silently bring it back.
                if d.access.ask_ai_model_provider.as_deref() == Some(id.as_str()) {
                    d.access.ask_ai_model = None;
                    d.access.ask_ai_model_provider = None;
                }
            },
            cx,
        );
        if self.ui.provider_open.as_deref() == Some(id.as_str()) {
            self.ui.provider_open = None;
        }
        self.reset_test();
        self.ui.reach.remove(&id);
        self.ui.pv.key_errors.remove(&id);
        if removed.kind.is_local() {
            return;
        }
        // Awaited: a same-kind re-add reuses the bare id and must not race the clear.
        let label = provider_instance_label(&removed);
        self.ui.pv.removing = true;
        let backend = self.backend.clone();
        let vid = id.clone();
        let job = cx.background_spawn(async move { backend.clear_key(&vid) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.pv.removing = false;
                match result {
                    Ok(()) => {
                        s.data.keys_saved.remove(&id);
                        s.data.key_endings.remove(&id);
                        s.ui.pv.removal_error = None;
                    }
                    Err(e) => {
                        s.ui.pv.removal_error = Some(format!("Could not clear the saved key for the removed provider {label} \u{2014} it may still be in the keychain. {e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// `beginLogin`: ask for a device code, then open the verify page.
    pub(crate) fn chatgpt_begin(&mut self, id: String, cx: &mut Context<Self>) {
        self.ui.pv.login_error.remove(&id);
        self.ui.pv.login.insert(id.clone(), Login::Starting);
        let job = self.backend.chatgpt_begin(&id).spawn(cx);
        let key = format!("chatgpt:{id}");
        let task = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |s, cx| {
                // Cancelled (or superseded) while it was asking.
                if s.ui.pv.login.get(&id) != Some(&Login::Starting) {
                    return;
                }
                match result {
                    Ok((code, url)) => {
                        cx.open_url(&url);
                        s.ui.pv.login.insert(id, Login::Waiting { code, url });
                    }
                    Err(e) => {
                        s.ui.pv.login.remove(&id);
                        s.ui.pv.login_error.insert(id, e);
                    }
                }
                cx.notify();
            });
        });
        self.jobs.insert(key, task);
        cx.notify();
    }

    /// `cancelLogin`: an abandoned poll keeps hitting auth.openai.com, whose
    /// rate limit is terminal, so tell the backend too.
    pub(crate) fn chatgpt_cancel(&mut self, id: String, cx: &mut Context<Self>) {
        self.ui.pv.login.remove(&id);
        self.ui.pv.login_error.remove(&id);
        // A device-code request still asking is dropped (cancelled) too.
        self.jobs.remove(&format!("chatgpt:{id}"));
        let backend = self.backend.clone();
        cx.background_spawn(async move { backend.chatgpt_cancel(&id) })
            .detach();
        cx.notify();
    }

    /// Disconnect: asks first, then removes the sign-in token from the vault.
    pub(crate) fn chatgpt_disconnect(&mut self, id: String, cx: &mut Context<Self>) {
        if self.ui.pv.disconnecting.is_some() {
            return;
        }
        self.ui.pv.login_error.remove(&id);
        // Armed before the dialog: a second click must not open a second one.
        self.ui.pv.disconnecting = Some(id.clone());
        cx.notify();
        confirm_then(
            cx,
            "Disconnect ChatGPT?",
            "Disconnecting removes the ChatGPT sign-in token from the vault right away. Any AI feature using this provider stops working until you sign in again.",
            "Disconnect",
            "Stay Connected",
            move |s, ok, cx| {
                if !ok {
                    s.ui.pv.disconnecting = None;
                    cx.notify();
                    return;
                }
                let backend = s.backend.clone();
                let vid = id.clone();
                let job = cx.background_spawn(async move { backend.clear_key(&vid) });
                cx.spawn(async move |this, cx| {
                    let result = job.await;
                    let _ = this.update(cx, |s, cx| {
                        s.ui.pv.disconnecting = None;
                        match result {
                            Ok(()) => {
                                s.data.keys_saved.remove(&id);
                                s.reset_test();
                                s.refresh(cx);
                            }
                            Err(e) => {
                                s.ui.pv.login_error.insert(id, e);
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            },
        );
    }

    /// `chatgpt_login_update`: the poll's one outcome, which may land long
    /// after the row closed. A sign-in refreshes presence, status and models.
    pub fn on_chatgpt_update(&mut self, v: &Value, cx: &mut Context<Self>) {
        let Some(id) = v["providerId"].as_str().map(str::to_string) else {
            return;
        };
        self.ui.pv.login.remove(&id);
        if v["connected"].as_bool() == Some(true) {
            self.ui.pv.login_error.remove(&id);
            self.data.keys_saved.insert(id);
            self.reset_test();
            self.refresh(cx);
            self.load_pool(cx);
        } else {
            let why = v["error"].as_str().unwrap_or("Sign-in failed.");
            self.ui.pv.login_error.insert(id, why.to_string());
        }
        cx.notify();
    }

    /// The picker's star (`ModelPrefs.toggleFavorite`): the starred list is an
    /// AI setting, so it saves with the AI runtime domain.
    pub(crate) fn toggle_favorite(&mut self, engine: (String, String), cx: &mut Context<Self>) {
        self.edit(
            |d| {
                let favs = &mut d.ai_runtime.favorite_models;
                let before = favs.len();
                favs.retain(|f| {
                    (f.provider.as_str(), f.model.as_str()) != (&*engine.0, &*engine.1)
                });
                if favs.len() == before {
                    favs.push(capture_types::AiEngineRef {
                        provider: engine.0,
                        model: engine.1,
                    });
                }
            },
            cx,
        );
    }

    /// `runAiRuntimeTestConnection`: one tiny round trip on the default model,
    /// then the status re-read.
    pub(crate) fn run_test_connection(&mut self, cx: &mut Context<Self>) {
        if self.ui.pv.test.running {
            return;
        }
        self.ui.pv.test.running = true;
        self.reset_test();
        let job = self.backend.test_connection().spawn(cx);
        let task = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.pv.test.running = false;
                s.ui.pv.test.result = Some(result);
                s.refresh(cx);
                cx.notify();
            });
        });
        self.jobs.insert("ai-test".into(), task);
        cx.notify();
    }
}
