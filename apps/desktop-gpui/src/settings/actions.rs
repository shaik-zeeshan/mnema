//! One-shot commands sections run (not autosaved): CLI Access block / unblock,
//! the model pickers' listing, and privacy commands. Each runs off the main
//! thread, then re-reads or adopts the answer.

use capture_types::RecordingSettings;
use gpui_kit::{AppContext as _, Context, Window};

use super::{Reach, Settings};

impl Settings {
    /// `block_cli_access_client` / `unblock_cli_access_client`, by normalized label.
    pub fn set_blocked(&mut self, client: String, blocked: bool, cx: &mut Context<Self>) {
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        let job = crate::bridge::tokio::Tokio::spawn(cx, async move {
            if blocked {
                app_core::cli_access::block_cli_access_client(&core, client).await
            } else {
                app_core::cli_access::unblock_cli_access_client(&core, client).await
            }
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Err(e)) = job.await {
                log::warn!("settings: CLI access change failed: {e}");
            }
            let _ = this.update(cx, |s, cx| s.reload_all(cx));
        })
        .detach();
    }

    /// `verify_ai_provider` for one row, against the draft list (an unsaved base
    /// URL counts). The row reads "Testing…" until the verdict lands.
    pub fn test_provider(&mut self, id: String, cx: &mut Context<Self>) {
        let providers = self.d().ai_runtime.providers;
        self.ui.reach.insert(id.clone(), Reach::Checking);
        cx.notify();
        let backend = self.backend.clone();
        let probe = id.clone();
        let job = cx.background_spawn(async move { backend.verify_provider(&probe, &providers) });
        cx.spawn(async move |this, cx| {
            let verdict = match job.await {
                Ok(()) => Reach::Reachable,
                Err(e) => Reach::Failed(e),
            };
            let _ = this.update(cx, |s, cx| {
                if s.ui.reach.get(&id) == Some(&Reach::Checking) {
                    s.ui.reach.insert(id, verdict);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Every row's reachability once per session (this entity lives as long as
    /// the shell), the first time Settings loads with AI on (Tauri: the first
    /// open of Providers). ChatGPT's listing is static, so it proves nothing;
    /// its row speaks from the runtime status.
    pub fn test_providers_once(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.draft.clone().filter(|d| d.ai_runtime.enabled) else {
            return;
        };
        if std::mem::replace(&mut self.ui.reach_checked, true) {
            return;
        }
        let ids: Vec<String> = d
            .ai_runtime
            .providers
            .iter()
            .filter(|p| p.kind != capture_types::AiProviderKind::Chatgpt)
            .map(|p| p.id.clone())
            .collect();
        for id in ids {
            self.test_provider(id, cx);
        }
    }

    /// List every provider's models for a picker (Chat's door).
    pub fn load_pool(&mut self, cx: &mut Context<Self>) {
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        self.pool_loading = true;
        let task = crate::bridge::tokio::Tokio::spawn(cx, async move {
            crate::chat::data::list_models(&core).await
        });
        cx.spawn(async move |this, cx| {
            let pool = task.await.map(|(m, _)| m).unwrap_or_default();
            let _ = this.update(cx, |s, cx| {
                s.pool = pool;
                s.pool_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Save key: read the masked field once, clear it, vault the key by
    /// instance id, then show its last 4.
    pub fn save_provider_key(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.inputs.provider_key.clone();
        let key = input.read(cx).value().trim().to_string();
        if key.is_empty() {
            return;
        }
        input.update(cx, |s, cx| s.set_value("", window, cx));
        let backend = self.backend.clone();
        let vid = id.clone();
        let job = cx.background_spawn(async move { backend.save_key(&vid, &key) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |s, cx| {
                match result {
                    Ok(ending) => {
                        s.data.keys_saved.insert(id.clone());
                        match ending {
                            Some(e) => s.data.key_endings.insert(id, e),
                            None => s.data.key_endings.remove(&id),
                        };
                        s.ui.key_error = None;
                    }
                    Err(e) => {
                        log::warn!("settings: couldn't store the {id} key in the vault");
                        s.ui.key_error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Clear: revoke the instance's credential (key or ChatGPT token set).
    pub fn clear_provider_key(&mut self, id: String, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        let vid = id.clone();
        let job = cx.background_spawn(async move { backend.clear_key(&vid) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |s, cx| {
                match result {
                    Ok(()) => {
                        s.data.keys_saved.remove(&id);
                        s.data.key_endings.remove(&id);
                        s.ui.key_error = None;
                    }
                    Err(e) => s.ui.key_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A privacy command: every autosave waits until it lands (it mutates the
    /// same recording settings), then the drafts adopt its answer.
    pub fn privacy_command(
        &mut self,
        mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), capture_types::CaptureErrorResponse>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut next) = self.saved.clone() else {
            return;
        };
        if mutate(&mut next).is_err() || self.privacy_in_flight {
            return;
        }
        self.privacy_in_flight = true;
        self.engine.cancel_all();
        let backend = self.backend.clone();
        let job = cx.background_spawn(async move { backend.save_privacy(&next) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.privacy_in_flight = false;
                match result {
                    Ok(s) => {
                        if let Some(d) = this.draft.as_mut() {
                            d.privacy = s.privacy.clone();
                        }
                        this.saved = Some(s);
                        this.privacy_error = None;
                    }
                    Err(e) => this.privacy_error = Some(e),
                }
                this.tick(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
