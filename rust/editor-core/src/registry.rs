//! Thread-safe global registry for v2 editor sessions.
//!
//! Each session is identified by a unique `SessionId` (u64). The registry
//! stores `Arc<EditorSessionSlot>` handles whose lifecycle state machine
//! (alive -> destroying -> destroyed) guarantees that no work runs on a
//! session once destruction has begun.

#![allow(
    clippy::result_large_err,
    reason = "SessionError remains unboxed to preserve the stable session boundary shape"
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::session::{EditorSession, SessionError};

pub(crate) type SessionId = u64;

const SESSION_ALIVE: u8 = 0;
const SESSION_DESTROYING: u8 = 1;
const SESSION_DESTROYED: u8 = 2;

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

static SESSION_REGISTRY: OnceLock<Mutex<HashMap<SessionId, Arc<EditorSessionSlot>>>> =
    OnceLock::new();

fn global_session_registry() -> &'static Mutex<HashMap<SessionId, Arc<EditorSessionSlot>>> {
    SESSION_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

// Not reachable from production call paths after the Task 16C legacy runtime
// removal; exercised by crate tests.
#[allow(dead_code)]
pub(crate) fn session_registry_count() -> usize {
    global_session_registry()
        .lock()
        .expect("session registry lock poisoned")
        .len()
}

pub(crate) struct EditorSessionSlot {
    lifecycle: AtomicU8,
    session: Mutex<EditorSession>,
}

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
        crate::boundary::with_document_stack(|| self.with_alive_after_check(|| {}, operation))
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

fn session_not_alive(lifecycle: u8) -> SessionError {
    let (code, message) = if lifecycle == SESSION_DESTROYING {
        ("ENGINE_DESTROYING", "editor session is being destroyed")
    } else {
        ("ENGINE_DESTROYED", "editor session has been destroyed")
    };
    SessionError::new(crate::session::ErrorDomain::Lifecycle, code, message)
}

pub(crate) fn create_session(
    admit: impl FnOnce() -> Result<EditorSession, SessionError>,
) -> Result<SessionId, SessionError> {
    crate::boundary::with_document_stack(|| create_session_inner(admit))
}

fn create_session_inner(
    admit: impl FnOnce() -> Result<EditorSession, SessionError>,
) -> Result<SessionId, SessionError> {
    #[cfg(test)]
    let _concurrency_guard = crate::test_support::RegistryConcurrencyGuard::acquire();
    let session = admit()?;
    let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let slot = Arc::new(EditorSessionSlot::new(session));
    global_session_registry()
        .lock()
        .expect("session registry lock poisoned")
        .insert(id, slot);
    Ok(id)
}

pub(crate) fn get_session(id: SessionId) -> Option<Arc<EditorSessionSlot>> {
    global_session_registry()
        .lock()
        .expect("session registry lock poisoned")
        .get(&id)
        .cloned()
}

pub(crate) fn destroy_session(id: SessionId) {
    crate::boundary::with_document_stack(|| destroy_session_inner(id));
}

fn destroy_session_inner(id: SessionId) {
    #[cfg(test)]
    let _concurrency_guard = crate::test_support::RegistryConcurrencyGuard::acquire();
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

#[cfg(test)]
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
