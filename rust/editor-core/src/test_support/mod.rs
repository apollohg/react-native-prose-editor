//! Crate-internal relocated integration suites (Task 16A).
//!
//! Every module here was relocated verbatim from `rust/editor-core/tests/` so that
//! Task 16C can make `yrs_engine`, model, transform, schema internals, selection,
//! render, snapshot/operation types crate-private without losing
//! coverage. Test bodies are unchanged; the only adaptation is `editor_core::` →
//! `crate::` path rewriting. The legacy differential-oracle and legacy-runtime-only
//! suites were deleted in Task 16C per the user directive of 2026-07-20 ("We don't
//! need to keep legacy code"): `test_oracle/legacy/` is gone entirely,
//! `code_review_fixes_test` (a pure legacy-`Editor` harness suite) was deleted as
//! redundant with the v2 engine suites, and `boundary_test`/`position_test` were
//! ported to retained APIs (their legacy-harness tests dropped).

mod boundary_test;
mod collaboration_awareness_test;
mod collaboration_outbox_test;
mod collaboration_protocol_test;
mod collaboration_transport_state_test;
mod ffi_v2_test;
mod model_test;
mod native_transaction_bridge_test;
mod position_test;
mod render_test;
mod schema_test;
mod selection_test;
mod serialize_test;
mod session_initialization_test;
mod session_lifecycle_test;
mod session_replacement_test;
mod session_snapshot_lifecycle_test;
mod transform_test;
mod yrs_engine_awareness_test;
mod yrs_engine_compiler_test;
mod yrs_engine_convergence_test;
mod yrs_engine_derived_state_test;
mod yrs_engine_history_test;
mod yrs_engine_mutation_test;
mod yrs_engine_operation_contract_test;
mod yrs_engine_position_test;
mod yrs_engine_remote_update_test;
mod yrs_engine_render_state_test;
mod yrs_engine_resource_test;
mod yrs_engine_snapshot_test;
mod yrs_engine_split_regression_test;
mod yrs_engine_stored_marks_test;
mod yrs_engine_structural_operation_test;
mod yrs_engine_transaction_property_test;
mod yrs_engine_transaction_test;
mod yrs_engine_upgrade_test;

// ---------------------------------------------------------------------------
// Session-registry concurrency guard
// ---------------------------------------------------------------------------
//
// The relocated staging suites previously ran as separate test binaries, each
// with its own process-global session registry. Merged into the lib test
// process they share `registry::SESSION_REGISTRY`, and suites that assert
// global registry counts (`session_initialization`, `session_replacement`,
// `session_lifecycle`) can observe concurrent create/destroy calls from other
// suites. This reentrant, process-wide guard restores the old isolation:
// `registry::create_session`/`destroy_session` acquire it under `#[cfg(test)]`,
// and count-asserting tests hold it for their full duration. Reentrancy lets a
// guard-holding test call session helpers without deadlocking.

pub(crate) use registry_concurrency::RegistryConcurrencyGuard;

mod registry_concurrency {
    use std::cell::Cell;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    thread_local! {
        static REGISTRY_GUARD_DEPTH: Cell<usize> = const { Cell::new(0) };
    }

    static REGISTRY_GUARD: OnceLock<Mutex<()>> = OnceLock::new();

    pub(crate) struct RegistryConcurrencyGuard {
        _guard: Option<MutexGuard<'static, ()>>,
    }

    impl RegistryConcurrencyGuard {
        pub(crate) fn acquire() -> Self {
            let already_held = REGISTRY_GUARD_DEPTH.with(|depth| {
                let held = depth.get() > 0;
                depth.set(depth.get() + 1);
                held
            });
            if already_held {
                return Self { _guard: None };
            }
            let guard = REGISTRY_GUARD
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Self {
                _guard: Some(guard),
            }
        }

        /// Mark the current thread as belonging to the test that already holds
        /// the guard. Only for tests that intentionally race registry
        /// operations from threads they spawn while asserting global counts;
        /// the guard excludes *foreign* tests, never the holder's own threads.
        pub(crate) fn inherit_for_spawned_thread() -> Self {
            REGISTRY_GUARD_DEPTH.with(|depth| depth.set(depth.get() + 1));
            Self { _guard: None }
        }
    }

    impl Drop for RegistryConcurrencyGuard {
        fn drop(&mut self) {
            REGISTRY_GUARD_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }
}
