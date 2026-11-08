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

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Mutex};

    use tokio::sync::oneshot;

    use super::*;
    use crate::{
        event::CoreEvent,
        host::{ConfirmDialog, ExitKind, MessageDialog, WindowRequest, WindowSignal},
    };

    #[derive(Default)]
    struct RecordingHost {
        events: Mutex<Vec<CoreEvent>>,
    }

    impl Host for RecordingHost {
        fn emit(&self, event: CoreEvent) {
            self.events.lock().unwrap().push(event);
        }
        fn message(&self, _req: MessageDialog) {}
        fn confirm(&self, _req: ConfirmDialog) -> oneshot::Receiver<bool> {
            let (tx, rx) = oneshot::channel();
            let _ = tx.send(true);
            rx
        }
        fn open_url(&self, _url: &str) {}
        fn refresh_tray(&self) {}
        fn open_window(&self, _req: WindowRequest) {}
        fn window_signal(&self, _signal: WindowSignal) {}
        fn exit(&self, _kind: ExitKind) {}
    }

    fn core_with(host: Arc<RecordingHost>) -> AppCore {
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
        let handle = core.runtime_handle();
        assert_eq!(core.block_on(handle.spawn(async { 5 })).unwrap(), 5);
    }

    #[test]
    fn host_receives_emits_and_confirms() {
        let host = Arc::new(RecordingHost::default());
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
