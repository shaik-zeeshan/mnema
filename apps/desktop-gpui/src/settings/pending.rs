//! A slow app-core answer (network, a child process) that no thread waits for:
//! it runs on tokio and lands through a GPUI task. Dropping that task cancels
//! the call, so a view keeps it while it wants the answer.

use std::future::Future;
use std::pin::Pin;

use gpui_kit::{AppContext, Task};

use crate::bridge::tokio::Tokio;

pub enum Pending<T> {
    /// Answered already (a fixture, a missing prerequisite): lands without tokio.
    Ready(Result<T, String>),
    /// The call itself, for tokio.
    Later(Pin<Box<dyn Future<Output = Result<T, String>> + Send>>),
}

impl<T: Send + 'static> Pending<T> {
    pub fn later(call: impl Future<Output = Result<T, String>> + Send + 'static) -> Self {
        Self::Later(Box::pin(call))
    }

    /// Start it; the task answers once it lands.
    pub fn spawn<C: AppContext>(self, cx: &C) -> Task<Result<T, String>> {
        match self {
            Self::Ready(answer) => Task::ready(answer),
            Self::Later(call) => {
                let job = Tokio::spawn(cx, call);
                cx.background_spawn(async move { job.await.unwrap_or_else(|e| Err(e.to_string())) })
            }
        }
    }
}
