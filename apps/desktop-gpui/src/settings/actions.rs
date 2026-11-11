//! One-shot commands sections run (not autosaved): CLI Access block / unblock,
//! the model pickers' listing, and privacy commands. Each runs off the main
//! thread, then re-reads or adopts the answer.

use capture_types::RecordingSettings;
use gpui_kit::{AppContext as _, Context};

use super::{Reach, Settings};

impl Settings {
    /// `block_cli_access_client` / `unblock_cli_access_client`, by normalized label.
    /// The row reads Blocking… / Unblocking… until it lands; a failure says
    /// so above the list (`brokerGrantError`). Unsaved drafts are untouched.
    pub fn set_blocked(&mut self, client: String, blocked: bool, cx: &mut Context<Self>) {
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        if !self.ui.grant_saving.insert(client.clone()) {
            return;
        }
        self.ui.grant_error = None;
        let key = client.clone();
        let job = crate::bridge::tokio::Tokio::spawn(cx, async move {
            if blocked {
                app_core::cli_access::block_cli_access_client(&core, client).await
            } else {
                app_core::cli_access::unblock_cli_access_client(&core, client).await
            }
        });
        cx.spawn(async move |this, cx| {
            let failed = match job.await {
                Ok(Err(e)) => Some(e.to_string()),
                Err(e) => Some(e.to_string()),
                Ok(Ok(_)) => None,
            };
            let _ = this.update(cx, |s, cx| {
                s.ui.grant_saving.remove(&key);
                if let Some(e) = failed {
                    log::warn!("settings: CLI access change failed: {e}");
                    s.ui.grant_error = Some(e);
                }
                s.refresh(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
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
            crate::chat::data::list_models(&core, None).await
        });
        cx.spawn(async move |this, cx| {
            let (pool, failures) = task.await.unwrap_or_default();
            let _ = this.update(cx, |s, cx| {
                s.pool = pool;
                s.ui.pv.failures = failures;
                s.pool_loading = false;
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
        self.privacy_command_then(mutate, |_, _, _| {}, cx);
    }

    /// [`Self::privacy_command`], then `done(saved?)` once it lands. `false` =
    /// it never started (nothing loaded, the mutation refused, one in flight).
    pub fn privacy_command_then(
        &mut self,
        mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), capture_types::CaptureErrorResponse>,
        done: impl FnOnce(&mut Self, bool, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut next) = self.saved.clone() else {
            return false;
        };
        if mutate(&mut next).is_err() || self.privacy_in_flight {
            return false;
        }
        self.privacy_in_flight = true;
        self.engine.cancel_all();
        let backend = self.backend.clone();
        let job = cx.background_spawn(async move { backend.save_privacy(&next) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.privacy_in_flight = false;
                let ok = result.is_ok();
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
                done(this, ok, cx);
                this.tick(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
        true
    }

    /// "Grant access": the macOS prompt; a failure stays on its own row.
    pub fn request_permission(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        if self.ui.perm_busy.is_some() {
            return;
        }
        self.ui.perm_busy = Some(kind);
        self.ui.perm_error = None;
        self.run_command_with(
            "Permission request",
            cx,
            move |core| super::live::request_permission(core, kind),
            move |s, outcome| {
                s.ui.perm_busy = None;
                if let Err(e) = outcome {
                    s.ui.perm_error = Some((kind, e.clone()));
                }
                true
            },
        );
        if !self.ui.busy.contains("Permission request") {
            self.ui.perm_busy = None;
        }
    }

    /// The system-audio hint's Dismiss: hidden at once, recorded so it stays
    /// hidden across launches (`dismiss_one_time_prompt`).
    pub fn dismiss_system_audio_hint(&mut self, cx: &mut Context<Self>) {
        self.ui.sysaudio_dismissed = true;
        self.ui.sysaudio_error = None;
        cx.notify();
        let backend = self.backend.clone();
        let job = cx.background_spawn(async move {
            backend.prompt(
                app_core::native_capture::permissions::SYSTEM_AUDIO_BLOCKED_PROMPT_ID,
                super::privacy_prompt::Mark::Dismissed,
            )
        });
        cx.spawn(async move |this, cx| {
            if let Err(e) = job.await {
                let _ = this.update(cx, |s, cx| {
                    s.ui.sysaudio_error = Some(e);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The shell shows or leaves Settings; leaving stops the CLI Access poll.
    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
        if !shown {
            self.access_poll = None;
        }
    }

    /// `Access.svelte`'s `setInterval(refresh, 30000)`: grants and activity are
    /// re-read every 30 s while the Data group is on screen (approvals have no
    /// change event). Runs from `render`; dropping the task stops it.
    pub(crate) fn sync_access_poll(&mut self, cx: &mut Context<Self>) {
        let want = self.shown && self.group == super::groups::GroupId::Data;
        if !want {
            self.access_poll = None;
            return;
        }
        if self.access_poll.is_some() {
            return;
        }
        self.access_poll = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(ACCESS_POLL).await;
            if this.update(cx, |s, cx| s.on_window_focus(cx)).is_err() {
                return;
            }
        }));
    }
}

/// How often CLI Access re-reads while visible.
pub const ACCESS_POLL: std::time::Duration = std::time::Duration::from_secs(30);
