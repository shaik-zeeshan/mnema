//! The shell's licence banner and the deep-link receipt (`LicenseBanner.svelte`,
//! `LicenseDeepLinkModal.svelte`). One `Licence` entity per main window holds
//! the last `license_status` (the store), the banner's dismissals, and the
//! receipt episode. `bridge/events.rs` feeds it app-core's `LicenseStatus` and
//! `LicenseDeepLinkDone` events; `native/deep_links.rs` starts episodes through
//! [`announce`]. Policy lives in `banner.rs` / `receipt.rs`; `view.rs` renders.

pub mod banner;
mod parts;
pub mod receipt;
mod ruler;
mod ticket;
mod view;

use std::sync::{Arc, Mutex};

use app_core::AppCore;
use capture_types::{LicenseStatus, ResetDevicesOutcome};
use gpui_kit::{App, AppContext, Context, Entity, FocusHandle, Global, Task};

use banner::{Action, Dismiss, Dismissed};
use receipt::{Done, Flow, Receipt};
pub use view::{banner as render_banner, receipt as render_receipt};

/// `lib/licensing.ts` `LICENSE_CHECKOUT_URL` (default).
pub const LICENSE_CHECKOUT_URL: &str = "https://sandbox-api.polar.sh/v1/checkout-links/polar_cl_lMoTLnM0OegXGCtfDMzfFi54ZZ41zhfSL8mvP1BpK1L/redirect";

/// What the banner and receipt ask of app-core. Sync; run on the background
/// executor, never on the main thread.
pub trait Backend: Send + Sync {
    /// `refresh_license_now` (Re-check license). The new status arrives as an event.
    fn refresh(&self) -> Result<(), String>;
    /// The "Free up my devices?" native confirm.
    fn confirm_reset(&self) -> bool;
    /// `reset_license_devices`; on success app-core already retried activation.
    fn reset_devices(&self) -> Result<ResetDevicesOutcome, String>;
}

pub struct CoreBackend(pub Arc<AppCore>);

impl Backend for CoreBackend {
    fn refresh(&self) -> Result<(), String> {
        self.0
            .block_on(app_core::licensing::refresh_license_now(self.0.clone()))
    }
    fn confirm_reset(&self) -> bool {
        self.0
            .host()
            .confirm(app_core::ConfirmDialog {
                title: Some("Free up my devices".into()),
                message: "Free up your devices? This clears the license's device list so this Mac can activate. You can do this once every 30 days.".into(),
                kind: app_core::DialogKind::Warning,
                ok_label: "OK".into(),
                cancel_label: "Cancel".into(),
            })
            .blocking_recv()
            .unwrap_or(false)
    }
    fn reset_devices(&self) -> Result<ResetDevicesOutcome, String> {
        self.0
            .block_on(app_core::licensing::reset::reset_license_devices(
                self.0.clone(),
            ))
    }
}

/// No app core (goldens): nothing answers.
pub struct NoBackend;

impl Backend for NoBackend {
    fn refresh(&self) -> Result<(), String> {
        Err("no app core".into())
    }
    fn confirm_reset(&self) -> bool {
        false
    }
    fn reset_devices(&self) -> Result<ResetDevicesOutcome, String> {
        Err("no app core".into())
    }
}

/// Records calls and answers as told (tests).
#[derive(Default)]
pub struct FixtureBackend {
    pub calls: Mutex<Vec<&'static str>>,
    pub confirm: bool,
    pub reset: Option<ResetDevicesOutcome>,
}

impl Backend for FixtureBackend {
    fn refresh(&self) -> Result<(), String> {
        self.calls.lock().unwrap().push("refresh");
        Ok(())
    }
    fn confirm_reset(&self) -> bool {
        self.calls.lock().unwrap().push("confirm_reset");
        self.confirm
    }
    fn reset_devices(&self) -> Result<ResetDevicesOutcome, String> {
        self.calls.lock().unwrap().push("reset_devices");
        self.reset
            .clone()
            .ok_or_else(|| "The license server refused the reset.".into())
    }
}

/// A deep link that arrived while main didn't exist (onboarding, or before
/// first open). One slot, latest wins; the next shell takes it, as Tauri's
/// `take_pending_license_deep_link` on a cold main window.
#[derive(Default)]
struct PendingEpisode(Option<Flow>);

impl Global for PendingEpisode {}

pub struct Licence {
    /// The last `license_status` (the Svelte store's value).
    pub status: Option<LicenseStatus>,
    /// The shell's low-disk suspension (the banner's last resort).
    pub low_disk: bool,
    pub dismissed: Dismissed,
    /// Re-check in flight.
    pub rechecking: bool,
    pub receipt: Receipt,
    backend: Arc<dyn Backend>,
    slow_timer: Option<Task<()>>,
    /// The receipt's primary action (made on the first episode).
    primary: Option<FocusHandle>,
    /// The face the primary action was last focused for (generation, kind).
    focused: Option<(u64, std::mem::Discriminant<receipt::Face>)>,
}

impl Licence {
    /// Starts from app-core's cached status (an in-memory read) and takes a
    /// deep link that arrived before this window existed.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let core = crate::chat::core(cx);
        let status = core.as_deref().and_then(app_core::licensing::cached_status);
        let backend: Arc<dyn Backend> = match core {
            Some(core) => Arc::new(CoreBackend(core)),
            None => Arc::new(NoBackend),
        };
        let mut this = Self::with_backend(status, backend);
        let pending = cx.try_global::<PendingEpisode>().and_then(|p| p.0);
        if let Some(flow) = pending {
            cx.set_global(PendingEpisode(None));
            this.start(flow, cx);
        }
        this
    }

    pub fn with_backend(status: Option<LicenseStatus>, backend: Arc<dyn Backend>) -> Self {
        Self {
            status,
            low_disk: false,
            dismissed: Dismissed::default(),
            rechecking: false,
            receipt: Receipt::default(),
            backend,
            slow_timer: None,
            primary: None,
            focused: None,
        }
    }

    /// A live `license_status` emit: the banner re-derives, the receipt morphs.
    pub fn on_status(&mut self, status: Option<LicenseStatus>, cx: &mut Context<Self>) {
        self.status = status;
        if self.receipt.on_status(self.status.as_ref()) {
            self.slow_timer = None;
        }
        cx.notify();
    }

    pub fn on_done(&mut self, done: Done, cx: &mut Context<Self>) {
        if self.receipt.listening() {
            self.slow_timer = None;
        }
        self.receipt.on_done(done);
        cx.notify();
    }

    /// Open the receipt in its working face and arm the 30 s "still working".
    pub fn start(&mut self, flow: Flow, cx: &mut Context<Self>) {
        self.receipt.start(flow, self.status.clone());
        if self.primary.is_none() {
            self.primary = Some(cx.focus_handle().tab_stop(true));
        }
        let generation = self.receipt.generation;
        self.slow_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(receipt::SLOW_AFTER).await;
            let _ = this.update(cx, |this, cx| {
                if this.receipt.generation == generation {
                    this.receipt.slow = true;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// Close the receipt if it is open (Esc). False when there was nothing to close.
    pub fn close_receipt(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.receipt.open {
            return false;
        }
        self.receipt.close();
        self.slow_timer = None;
        cx.notify();
        true
    }

    pub fn dismiss(&mut self, d: Dismiss, cx: &mut Context<Self>) {
        self.dismissed.apply(d);
        cx.notify();
    }

    /// A banner button.
    pub fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::BuyLicense | Action::BuyMnema => cx.open_url(LICENSE_CHECKOUT_URL),
            Action::EnterLicense => {
                cx.defer(|cx| crate::window::open_settings(Some("license"), cx))
            }
            Action::OpenStorage => cx.defer(|cx| crate::window::open_settings(Some("storage"), cx)),
            Action::Recheck => self.recheck(cx),
        }
    }

    /// Manual Receipt Refresh; a heal flips the banner away via the next event.
    fn recheck(&mut self, cx: &mut Context<Self>) {
        if self.rechecking {
            return;
        }
        self.rechecking = true;
        cx.notify();
        let backend = self.backend.clone();
        let task = cx.background_spawn(async move { backend.refresh() });
        cx.spawn(async move |this, cx| {
            if task.await.is_err() {
                log::warn!("license re-check could not reach the license server");
            }
            let _ = this.update(cx, |this, cx| {
                this.rechecking = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// "Free up my devices": native confirm, then the reset; the note lands
    /// only on the episode that asked.
    pub fn reset_devices(&mut self, cx: &mut Context<Self>) {
        if self.receipt.resetting {
            return;
        }
        let generation = self.receipt.generation;
        let backend = self.backend.clone();
        let confirm = cx.background_spawn({
            let backend = backend.clone();
            async move { backend.confirm_reset() }
        });
        cx.spawn(async move |this, cx| {
            if !confirm.await {
                return;
            }
            let started = this.update(cx, |this, cx| {
                let same = this.receipt.generation == generation && this.receipt.listening();
                if same {
                    this.receipt.resetting = true;
                    this.receipt.reset_note = None;
                    cx.notify();
                }
                same
            });
            if !matches!(started, Ok(true)) {
                return;
            }
            let result = cx
                .background_spawn(async move { backend.reset_devices() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.receipt.generation != generation {
                    return;
                }
                this.receipt.resetting = false;
                let offset = crate::ui::clock::now(cx).offset();
                this.receipt.reset_note = Some(receipt::reset_note(&result, offset));
                cx.notify();
            });
        })
        .detach();
    }
}

/// A licence deep link was routed: open the receipt on main now, or keep it
/// for the next main window (onboarding still showing, main not built yet).
pub fn announce(flow: Flow, cx: &mut App) {
    let Some(shell) = crate::window::shell(cx) else {
        cx.set_global(PendingEpisode(Some(flow)));
        return;
    };
    let licence = shell.read(cx).licence.clone();
    licence.update(cx, |l, cx| l.start(flow, cx));
}

/// `bridge/events.rs`: forward the two licence events to main's entity.
pub fn on_event(licence: &Entity<Licence>, event: &app_core::CoreEvent, cx: &mut App) {
    match event {
        app_core::CoreEvent::LicenseStatus(v) => {
            let status = serde_json::from_value::<LicenseStatus>(v.clone()).ok();
            licence.update(cx, |l, cx| l.on_status(status, cx));
        }
        app_core::CoreEvent::LicenseDeepLinkDone(v) => {
            if let Some(done) = Done::parse(v) {
                licence.update(cx, |l, cx| l.on_done(done, cx));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
