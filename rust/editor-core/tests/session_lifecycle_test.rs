#![cfg(feature = "ffi-v2-staging")]

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use editor_core::session_lifecycle_test_support::{
    create_session, destroy_session, get_session, registry_count, LifecycleState,
    LifecycleTestError,
};

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

fn wait_for_state(
    handle: &editor_core::session_lifecycle_test_support::SessionHandle,
    expected: LifecycleState,
) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while handle.lifecycle() != expected {
        assert!(
            Instant::now() < deadline,
            "session did not reach {expected:?}; current state is {:?}",
            handle.lifecycle()
        );
        thread::yield_now();
    }
}

#[test]
fn registry_owned_sessions_are_linearizable_and_teardown_once() {
    let baseline = registry_count();

    let rejected = create_session(true);
    assert!(rejected.is_err(), "invalid admission must fail creation");
    assert_eq!(
        registry_count(),
        baseline,
        "failed creation must never publish a registry entry"
    );

    let raced_id = create_session(false).expect("valid session should be admitted");
    let raced_handle = get_session(raced_id).expect("created session should be discoverable");
    assert_eq!(raced_handle.lifecycle(), LifecycleState::Alive);

    let (checked_tx, checked_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let paused_handle = raced_handle.clone();
    let paused_call =
        thread::spawn(move || paused_handle.normal_call_after_alive_check(checked_tx, resume_rx));
    checked_rx
        .recv_timeout(WAIT_TIMEOUT)
        .expect("call should complete its optimistic Alive check");

    destroy_session(raced_id);
    assert_eq!(raced_handle.lifecycle(), LifecycleState::Destroyed);
    assert_eq!(raced_handle.teardown_count(), 1);
    resume_tx
        .send(())
        .expect("paused call should still be waiting to take the lock");
    assert_eq!(
        paused_call.join().expect("paused call should not panic"),
        Err(LifecycleTestError::EngineDestroyed),
        "a pre-acquired handle must recheck Alive after taking the session lock"
    );
    assert_eq!(registry_count(), baseline);

    let id = create_session(false).expect("valid session should be admitted");
    let handle = get_session(id).expect("created session should be discoverable");

    const CALLERS: usize = 24;
    let callers: Vec<_> = (0..CALLERS)
        .map(|_| {
            let handle = handle.clone();
            thread::spawn(move || handle.normal_call())
        })
        .collect();
    for caller in callers {
        caller
            .join()
            .expect("normal caller should not panic")
            .expect("normal call on a live session should succeed");
    }
    assert_eq!(handle.normal_call_count(), CALLERS);

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let holding_handle = handle.clone();
    let holding_call =
        thread::spawn(move || holding_handle.hold_alive_call(entered_tx, release_rx));
    entered_rx
        .recv_timeout(WAIT_TIMEOUT)
        .expect("normal call should enter while Alive and hold the session lock");

    let destroyer = thread::spawn(move || destroy_session(id));
    wait_for_state(&handle, LifecycleState::Destroying);
    assert_eq!(
        registry_count(),
        baseline,
        "destroy must remove the entry before waiting on an in-flight call"
    );
    assert!(
        get_session(id).is_none(),
        "no new handle may be acquired after destroy begins"
    );

    let (late_result_tx, late_result_rx) = mpsc::channel();
    let late_handle = handle.clone();
    let late_call = thread::spawn(move || {
        late_result_tx
            .send(late_handle.normal_call())
            .expect("lifecycle test observer should be present");
    });
    assert_eq!(
        late_result_rx
            .recv_timeout(WAIT_TIMEOUT)
            .expect("call made during destroy should be rejected promptly"),
        Err(LifecycleTestError::EngineDestroying)
    );
    release_tx
        .send(())
        .expect("in-flight call should still own the session lock");
    holding_call
        .join()
        .expect("in-flight call should not panic")
        .expect("a call linearized before destroy must finish normally");
    destroyer.join().expect("destroy should not panic");

    late_call.join().expect("late call should not panic");
    assert_eq!(handle.lifecycle(), LifecycleState::Destroyed);
    assert_eq!(handle.teardown_count(), 1);

    let repeated_destroyers: Vec<_> = (0..16)
        .map(|_| thread::spawn(move || destroy_session(id)))
        .collect();
    for destroyer in repeated_destroyers {
        destroyer.join().expect("repeated destroy should not panic");
    }
    assert_eq!(handle.lifecycle(), LifecycleState::Destroyed);
    assert_eq!(handle.teardown_count(), 1, "teardown must run exactly once");
    assert_eq!(registry_count(), baseline);
}
