use std::{
    any::{Any, TypeId},
    collections::HashMap,
    future::Future,
    sync::{Arc, RwLock},
};

use tokio::{runtime::Runtime, task::JoinHandle};

use crate::{host::Host, paths::Paths};

type StateMap = RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>;

/// The shared app: the host seam, the paths, the one tokio runtime and a typed
/// state map mirroring Tauri's `app.state::<T>()`, so moved code renames its
/// lookups instead of being redesigned.
pub struct AppCore {
    host: Arc<dyn Host>,
    paths: Paths,
    runtime: Runtime,
    states: StateMap,
}

impl AppCore {
    /// Starts the runtime: multi-thread with IO + time, the same runtime Tauri
    /// builds by default. No states yet: the builder states still live in
    /// src-tauri until S2 moves them.
    pub fn new(host: Arc<dyn Host>, paths: Paths) -> std::io::Result<Self> {
        Ok(Self {
            host,
            paths,
            runtime: Runtime::new()?,
            states: StateMap::default(),
        })
    }

    pub fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// For hosts that run their own executor on this runtime
    /// (`tauri::async_runtime::set`, the GPUI tokio bridge).
    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    /// Register a state. Returns `false` (and keeps the old one) when a state of
    /// this type is already managed, like `tauri::Manager::manage`.
    pub fn manage<T: Send + Sync + 'static>(&self, state: T) -> bool {
        let mut states = self.states.write().unwrap_or_else(|e| e.into_inner());
        if states.contains_key(&TypeId::of::<T>()) {
            return false;
        }
        states.insert(TypeId::of::<T>(), Arc::new(state));
        true
    }

    pub fn try_state<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let states = self.states.read().unwrap_or_else(|e| e.into_inner());
        states
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|state| state.downcast::<T>().ok())
    }

    /// Panics when `T` was never managed, like `tauri::Manager::state`.
    pub fn state<T: Send + Sync + 'static>(&self) -> Arc<T> {
        self.try_state::<T>()
            .unwrap_or_else(|| panic!("state not managed for type {}", std::any::type_name::<T>()))
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime.spawn(future)
    }

    pub fn spawn_blocking<F, R>(&self, func: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        self.runtime.spawn_blocking(func)
    }

    /// Runs `func` on the blocking pool and waits for it, so slow synchronous work
    /// (hashing, decoding, capture start/stop) never holds a runtime worker. A
    /// panic in `func` resumes here, as if it had run inline.
    pub async fn run_blocking<F, R>(&self, func: F) -> R
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        match self.spawn_blocking(func).await {
            Ok(value) => value,
            Err(error) => std::panic::resume_unwind(error.into_panic()),
        }
    }

    /// Polls `future` to completion on the calling thread, or, when that thread's
    /// stack is smaller than a tokio worker's, on a scoped thread with a large one.
    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future + Send,
        F::Output: Send,
    {
        if caller_stack_size() >= MIN_POLL_STACK {
            return self.runtime.block_on(future);
        }
        // GPUI calls this from 512 KB libdispatch workers, where a deep future (an AI call) overflowed.
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .name("mnema-block-on".into())
                .stack_size(POLL_STACK)
                .spawn_scoped(scope, || self.runtime.block_on(future))
                .expect("spawn the block_on thread")
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        })
    }

    /// Runs `future` on a runtime worker and parks the calling thread until it
    /// finishes. The way for GPUI's libdispatch workers (512 KB stacks) to wait
    /// on app-core: the future polls on a worker's stack, so no thread is
    /// spawned per call, unlike `block_on`'s fallback. Panics if called from a
    /// runtime worker (tokio refuses a nested `block_on`).
    pub fn run<F>(&self, future: F) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        // Only the JoinHandle polls here, which needs almost no stack.
        match self.runtime.block_on(self.runtime.spawn(future)) {
            Ok(output) => output,
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(e) => panic!("app-core task ended without an answer: {e}"),
        }
    }
}

/// A tokio worker's stack, where the Tauri host polls the same futures.
const MIN_POLL_STACK: usize = 2 * 1024 * 1024;
/// The main thread's stack.
const POLL_STACK: usize = 8 * 1024 * 1024;

#[cfg(target_os = "macos")]
fn caller_stack_size() -> usize {
    // SAFETY: reads the calling thread's own stack size.
    unsafe { libc::pthread_get_stacksize_np(libc::pthread_self()) }
}

#[cfg(not(target_os = "macos"))]
fn caller_stack_size() -> usize {
    usize::MAX
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{event::CoreEvent, host::ConfirmDialog, test_support::FakeHost};

    fn core_with(host: Arc<FakeHost>) -> AppCore {
        let paths = Paths::resolve("day.mnema.test", Path::new("/tmp/home"), |_| None);
        AppCore::new(host, paths).expect("runtime starts")
    }

    #[derive(Debug, PartialEq)]
    struct Counter(u32);

    #[test]
    fn manage_registers_once_and_lookups_find_it() {
        let core = core_with(Arc::default());
        assert!(core.try_state::<Counter>().is_none());
        assert!(core.manage(Counter(1)));
        assert!(!core.manage(Counter(2)), "a second manage keeps the first");
        assert_eq!(*core.state::<Counter>(), Counter(1));
    }

    #[test]
    #[should_panic(expected = "state not managed")]
    fn state_panics_when_missing() {
        core_with(Arc::default()).state::<Counter>();
    }

    #[test]
    fn spawn_and_block_on_share_the_runtime() {
        let core = core_with(Arc::default());
        let task = core.spawn(async { 2 + 2 });
        let blocking = core.spawn_blocking(|| 3);
        assert_eq!(core.block_on(task).unwrap(), 4);
        assert_eq!(core.block_on(blocking).unwrap(), 3);
        assert_eq!(core.block_on(core.run_blocking(|| 6)), 6);
        let handle = core.runtime_handle();
        assert_eq!(core.block_on(handle.spawn(async { 5 })).unwrap(), 5);
    }

    /// Uses ~1.5 MB of stack: 96 frames, each holding a 16 KB buffer.
    fn burn_stack(depth: u32) -> u32 {
        let pad = std::hint::black_box([depth as u8; 16 * 1024]);
        if depth == 0 {
            return 0;
        }
        1 + burn_stack(depth - 1) + std::hint::black_box(pad[pad.len() - 1] as u32 & 0)
    }

    /// GPUI calls `block_on` from libdispatch workers (512 KB stacks); a deep
    /// future there overflowed the stack and killed the app (crash of 12 Nov).
    #[test]
    fn block_on_from_a_small_stack_thread_has_room_for_a_deep_future() {
        let core = Arc::new(core_with(Arc::default()));
        let small_thread = |core: Arc<AppCore>, run: fn(&AppCore) -> u32| {
            std::thread::Builder::new()
                .stack_size(512 * 1024)
                .spawn(move || run(&core))
                .unwrap()
                .join()
        };
        let deep = small_thread(core.clone(), |core| {
            core.block_on(async {
                // The runtime context is still entered for the future.
                let spawned = tokio::spawn(async { 1 }).await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                spawned + burn_stack(96)
            })
        });
        assert_eq!(deep.unwrap(), 97);

        let panicked = small_thread(core, |core| core.block_on(async { panic!("boom") }));
        let payload = panicked.expect_err("the future's panic reaches the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"boom"));
    }

    /// `run` polls on a worker, so a deep future works from a 512 KB thread
    /// without the scoped-thread hop, and a panic still reaches the caller.
    #[test]
    fn run_from_a_small_stack_thread_polls_on_a_worker() {
        let core = Arc::new(core_with(Arc::default()));
        let small_thread = |core: Arc<AppCore>, run: fn(&AppCore) -> u32| {
            std::thread::Builder::new()
                .stack_size(512 * 1024)
                .spawn(move || run(&core))
                .unwrap()
                .join()
        };
        let deep = small_thread(core.clone(), |core| {
            core.run(async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                let name = std::thread::current().name().map(str::to_owned);
                assert_ne!(name.as_deref(), Some("mnema-block-on"));
                // ~768 KB: past a libdispatch worker, inside a tokio worker's 2 MB.
                burn_stack(48)
            })
        });
        assert_eq!(deep.unwrap(), 48);

        let panicked = small_thread(core, |core| core.run(async { panic!("boom") }));
        let payload = panicked.expect_err("the future's panic reaches the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"boom"));
    }

    #[test]
    fn host_receives_emits_and_confirms() {
        let host = Arc::new(FakeHost::default());
        let core = core_with(host.clone());
        core.host().emit(CoreEvent::ConversationChanged);
        assert_eq!(
            *host.events.lock().unwrap(),
            vec![CoreEvent::ConversationChanged]
        );
        let answer = core.host().confirm(ConfirmDialog {
            title: None,
            message: "Delete?".into(),
            kind: crate::host::DialogKind::Warning,
            ok_label: "Delete".into(),
            cancel_label: "Cancel".into(),
        });
        assert!(core.block_on(answer).unwrap());
    }
}
