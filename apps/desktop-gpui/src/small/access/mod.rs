//! The CLI Access request window (`routes/access/request/+page.svelte`,
//! 520×600 / min 520×200, height fits content; ADR 0059). The anti-reflex rules:
//! focus starts on Deny, Enter is unbound (it presses nothing, not even the
//! focused button), Esc denies, scopes under the request's minimum are disabled,
//! and Allow carries no green fill. Closing the window answers `denied`/`closed`.
//! Backed by app-core's `broker_authorization_channel`; grant semantics live there.

pub mod model;
mod view;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_core::broker_authorization_channel as channel;
use app_core::AppCore;
use gpui_kit::{App, AppContext, Context, FocusHandle, KeyDownEvent, ScrollHandle, Task, Window};
use serde_json::Value;

use model::{Receipt, ReceiptKind, Request, Scope};

pub const SIZE: (f32, f32) = (520., 600.);
pub const MIN: (f32, f32) = (520., 200.);
/// Re-render so the "Requested Ns ago" label stays honest.
const AGE_TICK: Duration = Duration::from_secs(15);

/// The window's backend door. Sync: the view calls it on the background executor.
pub trait AccessBackend: Send + Sync + 'static {
    /// `get_pending_cli_access_request` (the DTO as JSON, `None` = nothing waiting).
    fn pending(&self) -> Result<Option<Value>, String>;
    fn approve(&self, request_id: &str, scope: Scope) -> Result<(), String>;
    fn block(&self, request_id: &str) -> Result<(), String>;
    /// Deny / Esc answer `userCancelled`; `closed` = the window went away unanswered.
    fn deny(&self, closed: bool) -> Result<(), String>;
}

pub struct CoreAccess(pub Arc<AppCore>);

impl AccessBackend for CoreAccess {
    fn pending(&self) -> Result<Option<Value>, String> {
        Ok(channel::get_pending_cli_access_request(&self.0)
            .and_then(|dto| serde_json::to_value(dto).ok()))
    }
    fn approve(&self, request_id: &str, scope: Scope) -> Result<(), String> {
        channel::approve_pending_cli_access_request(
            &self.0,
            channel::ApproveCliAccessRequest {
                request_id: request_id.to_string(),
                scope: scope.wire().to_string(),
            },
        )
    }
    fn block(&self, request_id: &str) -> Result<(), String> {
        channel::block_pending_cli_access_request(&self.0, request_id.to_string())
    }
    fn deny(&self, closed: bool) -> Result<(), String> {
        if closed {
            channel::cancel_pending_request(&self.0, "closed");
            Ok(())
        } else {
            channel::cancel_pending_cli_access_request(&self.0)
        }
    }
}

/// One backend call, as the timing / keyboard tests see it.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Pending,
    Approve(String, Scope),
    Block(String),
    Deny { closed: bool },
}

/// Serves a fixed request (parity fixture JSON) and records every call.
#[derive(Default)]
pub struct FixtureAccess {
    pub request: Option<Value>,
    /// Approve / block fail with this message.
    pub fail: Option<String>,
    /// Served instead of `request` once an approve / block has failed (another
    /// tool's request took the slot).
    pub successor: Option<Value>,
    pub(crate) calls: Mutex<Vec<Call>>,
}

impl FixtureAccess {
    pub fn new(request: Option<Value>) -> Self {
        Self {
            request,
            ..Default::default()
        }
    }
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
    fn record(&self, c: Call) {
        self.calls.lock().unwrap().push(c);
    }
}

impl AccessBackend for FixtureAccess {
    fn pending(&self) -> Result<Option<Value>, String> {
        let answered = self.fail.is_some()
            && self
                .calls()
                .iter()
                .any(|c| matches!(c, Call::Approve(..) | Call::Block(_)));
        self.record(Call::Pending);
        match (&self.successor, answered) {
            (Some(next), true) => Ok(Some(next.clone())),
            _ => Ok(self.request.clone()),
        }
    }
    fn approve(&self, request_id: &str, scope: Scope) -> Result<(), String> {
        self.record(Call::Approve(request_id.into(), scope));
        self.fail.clone().map_or(Ok(()), Err)
    }
    fn block(&self, request_id: &str) -> Result<(), String> {
        self.record(Call::Block(request_id.into()));
        self.fail.clone().map_or(Ok(()), Err)
    }
    fn deny(&self, closed: bool) -> Result<(), String> {
        self.record(Call::Deny { closed });
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Loading,
    LoadFailed(String),
    Empty,
    Request(Request),
    Receipt(Receipt),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Acting {
    #[default]
    None,
    Approving,
    Blocking,
    Cancelling,
}

pub struct AccessRequest {
    backend: Arc<dyn AccessBackend>,
    pub phase: Phase,
    pub selected: Scope,
    /// Approve / block / deny failures, above the footer.
    pub error: Option<String>,
    /// The rendered request can no longer be answered: only Close is left.
    pub gone: bool,
    pub acting: Acting,
    /// The safe anchor (Deny, or Close when nothing is pending).
    pub deny_focus: FocusHandle,
    pub close_focus: FocusHandle,
    /// Real windows resize to their content; goldens keep the opening size.
    pub fit: bool,
    pub(crate) fitted_view: String,
    pub(crate) body_scroll: ScrollHandle,
    _age: Task<()>,
}

impl AccessRequest {
    pub fn new(
        backend: Arc<dyn AccessBackend>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let age = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(AGE_TICK).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        });
        let mut this = Self {
            backend,
            phase: Phase::Loading,
            selected: Scope::LastDay,
            error: None,
            gone: false,
            acting: Acting::None,
            deny_focus: cx.focus_handle().tab_stop(true),
            close_focus: cx.focus_handle().tab_stop(true),
            fit: false,
            fitted_view: String::new(),
            body_scroll: ScrollHandle::new(),
            _age: age,
        };
        this.deny_focus.focus(window, cx);
        this.reload(window, cx);
        this
    }

    /// Read the slot (on open, and when a reused window comes forward again).
    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Receipt(_)) || self.acting != Acting::None {
            return;
        }
        self.error = None;
        self.gone = false;
        self.phase = Phase::Loading;
        let backend = self.backend.clone();
        let job = cx.background_spawn(async move { backend.pending() });
        cx.spawn_in(window, async move |this, cx| {
            let res = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loaded(res);
                // The skeleton → content swap must not drop focus off the safe anchor.
                if !this.close_focus.is_focused(window) {
                    this.deny_focus.focus(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn loaded(&mut self, res: Result<Option<Value>, String>) {
        let previous = match &self.phase {
            Phase::Request(r) => Some(r.id.clone()),
            _ => None,
        };
        self.phase = match res {
            Err(e) => Phase::LoadFailed(e),
            Ok(None) => Phase::Empty,
            Ok(Some(v)) => match Request::from_json(&v) {
                // Pre-select the NARROWEST scope that works, and only for a new request.
                Some(r) => {
                    if previous.as_deref() != Some(r.id.as_str()) {
                        self.selected = r.minimum;
                    }
                    Phase::Request(r)
                }
                None => Phase::LoadFailed("unreadable request".into()),
            },
        };
    }

    pub fn request(&self) -> Option<&Request> {
        match &self.phase {
            Phase::Request(r) => Some(r),
            _ => None,
        }
    }

    pub fn select(&mut self, scope: Scope, cx: &mut Context<Self>) {
        if self.acting == Acting::None && !self.gone {
            self.selected = scope;
            cx.notify();
        }
    }

    /// Deny (or Close when nothing is pending): `userCancelled`, then the window goes.
    pub fn deny(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.acting != Acting::None {
            return;
        }
        self.error = None;
        self.acting = Acting::Cancelling;
        let backend = self.backend.clone();
        let job = cx.background_spawn(async move { backend.deny(false) });
        cx.spawn_in(window, async move |this, cx| {
            let res = job.await;
            let _ = this.update_in(cx, |this, window, cx| match res {
                Ok(()) => window.remove_window(),
                Err(e) => {
                    this.error = Some(model::action_error(&e));
                    this.acting = Acting::None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn approve(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.answer(true, window, cx);
    }

    /// A standing rejection: persisted even for a never-granted tool.
    pub fn block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.answer(false, window, cx);
    }

    fn answer(&mut self, approve: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.request().cloned() else {
            return;
        };
        if self.acting != Acting::None || self.gone {
            return;
        }
        self.error = None;
        self.acting = if approve {
            Acting::Approving
        } else {
            Acting::Blocking
        };
        // Snapshot the terms: the receipt names them while the backend tears the window down.
        let receipt = Receipt {
            kind: if approve {
                ReceiptKind::Granted
            } else {
                ReceiptKind::Blocked
            },
            tool: r.tool.clone(),
            scope: self.selected,
            current: r.current,
            widen: r.is_widen(),
        };
        let (backend, scope) = (self.backend.clone(), self.selected);
        let job = cx.background_spawn(async move {
            if approve {
                backend.approve(&r.id, scope)
            } else {
                backend.block(&r.id)
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let res = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.acting = Acting::None;
                match res {
                    Ok(()) => {
                        this.phase = Phase::Receipt(receipt);
                        this.close_focus.focus(window, cx);
                    }
                    Err(e) => {
                        this.gone = model::is_gone(&e);
                        this.error = Some(model::action_error(&e));
                        this.deny_focus.focus(window, cx);
                        // The tool stopped waiting, or another tool's request took
                        // the slot since this one rendered: show that one instead.
                        if e.contains("no pending CLI Access request") {
                            this.show_successor(window, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// `showSuccessorIfAnyIsWaiting`: a different request in the slot replaces
    /// the dead one (its own narrowest scope, no error, answerable again).
    fn show_successor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        let job = cx.background_spawn(async move { backend.pending() });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Some(v)) = job.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(next) = Request::from_json(&v) else {
                    return;
                };
                if this.request().is_none_or(|r| r.id == next.id) {
                    return;
                }
                this.selected = next.minimum;
                this.phase = Phase::Request(next);
                this.error = None;
                this.gone = false;
                this.deny_focus.focus(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Esc denies (never while an answer is in flight); on a receipt it just closes.
    pub(crate) fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.keystroke.key != "escape" || self.acting != Acting::None {
            return;
        }
        cx.stop_propagation();
        if matches!(self.phase, Phase::Receipt(_)) {
            window.remove_window();
        } else {
            self.deny(window, cx);
        }
    }

    pub fn backend(&self) -> &Arc<dyn AccessBackend> {
        &self.backend
    }
}

/// A dismissed window is a non-verdict: answer the waiting CLI `denied`/`closed`
/// (retryable). Our own teardown after a verdict uses `remove_window`, which
/// never lands here, so it can't cancel whatever request has since taken the slot.
pub fn answer_closed_on_dismiss(
    backend: Arc<dyn AccessBackend>,
    window: &mut Window,
    cx: &mut App,
) {
    window.on_window_should_close(cx, move |_, _| {
        let _ = backend.deny(true);
        true
    });
}

/// Open the request window (or bring the reused one forward and re-read the slot).
pub fn open(cx: &mut App) {
    let Some(core) = crate::chat::core(cx) else {
        log::warn!("cli access: no app core; the request window can't open");
        return;
    };
    super::open_or_focus::<AccessRequest>(
        super::Kind::Access,
        cx,
        |window, cx| {
            let backend: Arc<dyn AccessBackend> = Arc::new(CoreAccess(core.clone()));
            let view = cx.new(|cx| {
                let mut v = AccessRequest::new(backend.clone(), window, cx);
                v.fit = true;
                v
            });
            answer_closed_on_dismiss(backend, window, cx);
            view
        },
        |view, window, cx| view.update(cx, |v, cx| v.reload(window, cx)),
    );
}
