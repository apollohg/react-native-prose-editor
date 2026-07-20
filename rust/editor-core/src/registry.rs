//! Thread-safe global registry for multiple editor instances.
//!
//! Each editor is identified by a unique `EditorId` (u64). The registry uses
//! `Arc<Mutex<Editor>>` for thread-safe access from native platform threads.

#![allow(
    clippy::result_large_err,
    reason = "SessionError remains unboxed to preserve the stable session boundary shape"
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(feature = "ffi-v2-staging")]
use std::sync::atomic::AtomicU8;

#[cfg(feature = "ffi-v2-staging")]
use crate::session::{EditorSession, SessionError};

use crate::boundary::ResourceLimits;
use crate::editor::Editor;
use crate::intercept::InterceptorPipeline;
use crate::schema::Schema;

/// Unique identifier for an editor instance.
pub type EditorId = u64;

/// Sentinel value indicating no valid editor.
pub const INVALID_EDITOR_ID: EditorId = 0;

/// Global atomic counter for generating unique editor IDs.
/// Starts at 1 so that 0 can serve as INVALID_EDITOR_ID.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The global registry singleton.
static REGISTRY: OnceLock<Mutex<HashMap<EditorId, Arc<Mutex<Editor>>>>> = OnceLock::new();

fn global_registry() -> &'static Mutex<HashMap<EditorId, Arc<Mutex<Editor>>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Thread-safe global registry for editor instances.
pub struct EditorRegistry;

impl EditorRegistry {
    /// Create a new editor and return its ID.
    pub fn create(
        schema: Schema,
        interceptors: InterceptorPipeline,
        allow_base64_images: bool,
    ) -> EditorId {
        Self::create_with_limits(
            schema,
            interceptors,
            allow_base64_images,
            ResourceLimits::default(),
        )
    }

    pub fn create_with_limits(
        schema: Schema,
        interceptors: InterceptorPipeline,
        allow_base64_images: bool,
        resource_limits: ResourceLimits,
    ) -> EditorId {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let editor =
            Editor::new_with_limits(schema, interceptors, allow_base64_images, resource_limits);
        let arc = Arc::new(Mutex::new(editor));

        let mut map = global_registry().lock().expect("registry lock poisoned");
        map.insert(id, arc);
        id
    }

    /// Get a handle to an editor by ID.
    pub fn get(id: EditorId) -> Option<Arc<Mutex<Editor>>> {
        let map = global_registry().lock().expect("registry lock poisoned");
        map.get(&id).cloned()
    }

    /// Destroy an editor, removing it from the registry.
    pub fn destroy(id: EditorId) {
        let mut map = global_registry().lock().expect("registry lock poisoned");
        map.remove(&id);
    }

    /// Number of active editors in the registry.
    pub fn count() -> usize {
        let map = global_registry().lock().expect("registry lock poisoned");
        map.len()
    }
}

#[cfg(feature = "ffi-v2-staging")]
pub(crate) type SessionId = u64;

#[cfg(feature = "ffi-v2-staging")]
const SESSION_ALIVE: u8 = 0;
#[cfg(feature = "ffi-v2-staging")]
const SESSION_DESTROYING: u8 = 1;
#[cfg(feature = "ffi-v2-staging")]
const SESSION_DESTROYED: u8 = 2;

#[cfg(feature = "ffi-v2-staging")]
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "ffi-v2-staging")]
static SESSION_REGISTRY: OnceLock<Mutex<HashMap<SessionId, Arc<EditorSessionSlot>>>> =
    OnceLock::new();

#[cfg(feature = "ffi-v2-staging")]
fn global_session_registry() -> &'static Mutex<HashMap<SessionId, Arc<EditorSessionSlot>>> {
    SESSION_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "ffi-v2-staging")]
pub(crate) fn session_registry_count() -> usize {
    global_session_registry()
        .lock()
        .expect("session registry lock poisoned")
        .len()
}

#[cfg(feature = "ffi-v2-staging")]
pub(crate) struct EditorSessionSlot {
    lifecycle: AtomicU8,
    session: Mutex<EditorSession>,
}

#[cfg(feature = "ffi-v2-staging")]
impl EditorSessionSlot {
    fn new(session: EditorSession) -> Self {
        Self {
            lifecycle: AtomicU8::new(SESSION_ALIVE),
            session: Mutex::new(session),
        }
    }

    pub(crate) fn with_alive<T>(
        &self,
        operation: impl FnOnce(&mut EditorSession) -> T,
    ) -> Result<T, SessionError> {
        self.with_alive_after_check(|| {}, operation)
    }

    fn with_alive_after_check<T>(
        &self,
        after_alive_check: impl FnOnce(),
        operation: impl FnOnce(&mut EditorSession) -> T,
    ) -> Result<T, SessionError> {
        let lifecycle = self.lifecycle.load(Ordering::Acquire);
        if lifecycle != SESSION_ALIVE {
            return Err(session_not_alive(lifecycle));
        }
        after_alive_check();
        let mut session = self.session.lock().expect("session lock poisoned");
        let lifecycle = self.lifecycle.load(Ordering::Acquire);
        if lifecycle != SESSION_ALIVE {
            return Err(session_not_alive(lifecycle));
        }
        Ok(operation(&mut session))
    }
}

#[cfg(feature = "ffi-v2-staging")]
fn session_not_alive(lifecycle: u8) -> SessionError {
    let (code, message) = if lifecycle == SESSION_DESTROYING {
        ("ENGINE_DESTROYING", "editor session is being destroyed")
    } else {
        ("ENGINE_DESTROYED", "editor session has been destroyed")
    };
    SessionError::new(crate::session::ErrorDomain::Lifecycle, code, message)
}

#[cfg(feature = "ffi-v2-staging")]
pub(crate) fn create_session(
    admit: impl FnOnce() -> Result<EditorSession, SessionError>,
) -> Result<SessionId, SessionError> {
    let session = admit()?;
    let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let slot = Arc::new(EditorSessionSlot::new(session));
    global_session_registry()
        .lock()
        .expect("session registry lock poisoned")
        .insert(id, slot);
    Ok(id)
}

#[cfg(feature = "ffi-v2-staging")]
pub(crate) fn get_session(id: SessionId) -> Option<Arc<EditorSessionSlot>> {
    global_session_registry()
        .lock()
        .expect("session registry lock poisoned")
        .get(&id)
        .cloned()
}

#[cfg(feature = "ffi-v2-staging")]
pub(crate) fn destroy_session(id: SessionId) {
    let slot = {
        let mut registry = global_session_registry()
            .lock()
            .expect("session registry lock poisoned");
        let Some(slot) = registry.get(&id).cloned() else {
            return;
        };
        if slot
            .lifecycle
            .compare_exchange(
                SESSION_ALIVE,
                SESSION_DESTROYING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }
        registry.remove(&id);
        slot
    };

    let mut session = slot.session.lock().expect("session lock poisoned");
    session.teardown();
    slot.lifecycle.store(SESSION_DESTROYED, Ordering::Release);
}

#[cfg(feature = "ffi-v2-staging")]
pub mod session_lifecycle_test_support {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::Arc;

    use super::{
        EditorSession, EditorSessionSlot, SESSION_ALIVE, SESSION_DESTROYED, SESSION_DESTROYING,
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum LifecycleState {
        Alive,
        Destroying,
        Destroyed,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum LifecycleTestError {
        AdmissionRejected,
        EngineDestroying,
        EngineDestroyed,
    }

    #[derive(Clone)]
    pub struct SessionHandle {
        slot: Arc<EditorSessionSlot>,
    }

    impl SessionHandle {
        pub fn lifecycle(&self) -> LifecycleState {
            match self.slot.lifecycle.load(Ordering::Acquire) {
                SESSION_ALIVE => LifecycleState::Alive,
                SESSION_DESTROYING => LifecycleState::Destroying,
                SESSION_DESTROYED => LifecycleState::Destroyed,
                _ => unreachable!("invalid session lifecycle state"),
            }
        }

        pub fn normal_call(&self) -> Result<usize, LifecycleTestError> {
            self.slot
                .with_alive(EditorSession::record_lifecycle_test_call)
                .map_err(map_lifecycle_error)
        }

        pub fn normal_call_after_alive_check(
            &self,
            checked: Sender<()>,
            resume: Receiver<()>,
        ) -> Result<usize, LifecycleTestError> {
            self.slot
                .with_alive_after_check(
                    || {
                        checked
                            .send(())
                            .expect("lifecycle test observer should be present");
                        resume
                            .recv()
                            .expect("lifecycle test should resume the paused call");
                    },
                    EditorSession::record_lifecycle_test_call,
                )
                .map_err(map_lifecycle_error)
        }

        pub fn hold_alive_call(
            &self,
            entered: Sender<()>,
            release: Receiver<()>,
        ) -> Result<(), LifecycleTestError> {
            self.slot
                .with_alive(|session| {
                    entered
                        .send(())
                        .expect("lifecycle test observer should be present");
                    release
                        .recv()
                        .expect("lifecycle test should release the held call");
                    session.record_lifecycle_test_call();
                })
                .map_err(map_lifecycle_error)
        }

        pub fn normal_call_count(&self) -> usize {
            self.slot
                .session
                .lock()
                .expect("session lock poisoned")
                .lifecycle_test_call_count()
        }

        pub fn teardown_count(&self) -> usize {
            self.slot
                .session
                .lock()
                .expect("session lock poisoned")
                .lifecycle_test_teardown_count()
        }
    }

    fn map_lifecycle_error(error: crate::session::SessionError) -> LifecycleTestError {
        match error.code.as_str() {
            "ENGINE_DESTROYING" => LifecycleTestError::EngineDestroying,
            "ENGINE_DESTROYED" => LifecycleTestError::EngineDestroyed,
            _ => unreachable!("unexpected lifecycle test error code"),
        }
    }

    pub fn create_session(reject_admission: bool) -> Result<u64, LifecycleTestError> {
        super::create_session(|| EditorSession::lifecycle_test_session(reject_admission))
            .map_err(|_| LifecycleTestError::AdmissionRejected)
    }

    pub fn get_session(id: u64) -> Option<SessionHandle> {
        super::get_session(id).map(|slot| SessionHandle { slot })
    }

    pub fn destroy_session(id: u64) {
        super::destroy_session(id);
    }

    pub fn registry_count() -> usize {
        super::global_session_registry()
            .lock()
            .expect("session registry lock poisoned")
            .len()
    }
}
