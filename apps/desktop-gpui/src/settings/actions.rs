//! One-shot commands sections run (not autosaved): CLI Access block / unblock,
//! the model pickers' listing, and privacy commands. Each runs off the main
//! thread, then re-reads or adopts the answer.

use capture_types::RecordingSettings;
use gpui_kit::{AppContext as _, Context};

use super::Settings;

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
