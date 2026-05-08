//! Integration tests for the `TaskManager`.
//!
//! Verifies: progress events, single-job cancellation, cancel_all,
//! semaphore bound, JobId monotonicity, error surfacing.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

use ichr::tasks::{JobId, TaskEvent, TaskManager};

fn new_manager(buf: usize, max_concurrent: usize) -> (TaskManager, mpsc::Receiver<TaskEvent>) {
    let (tx, rx) = mpsc::channel(buf);
    (TaskManager::new(tx, max_concurrent), rx)
}

async fn recv_next(rx: &mut mpsc::Receiver<TaskEvent>) -> TaskEvent {
    timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for TaskEvent")
        .expect("channel closed")
}

#[tokio::test]
async fn spawn_task_reports_progress() {
    let (mgr, mut rx) = new_manager(16, 4);
    let id = mgr.next_job_id();
    let _tok = mgr.spawn_task(id, move |tx, _t| async move {
        tx.send(TaskEvent::Progress {
            id,
            pct: 50,
            msg: "halfway".to_string(),
        })
        .await
        .unwrap();
        Ok(())
    });

    let first = recv_next(&mut rx).await;
    match first {
        TaskEvent::Progress { pct, msg, .. } => {
            assert_eq!(pct, 50);
            assert_eq!(msg, "halfway");
        }
        other => panic!("expected Progress, got {other:?}"),
    }

    let second = recv_next(&mut rx).await;
    matches!(second, TaskEvent::Completed { result: Ok(()), .. });
}

#[tokio::test]
async fn spawn_task_completes_ok() {
    let (mgr, mut rx) = new_manager(4, 2);
    let id = mgr.next_job_id();
    mgr.spawn_task(id, |_tx, _t| async { Ok(()) });

    let ev = recv_next(&mut rx).await;
    match ev {
        TaskEvent::Completed {
            id: got_id,
            result: Ok(()),
        } => assert_eq!(got_id, id),
        other => panic!("expected Completed(Ok), got {other:?}"),
    }
}

#[tokio::test]
async fn cancel_single_job_does_not_cancel_siblings() {
    let (mgr, mut rx) = new_manager(16, 4);
    let id_a = mgr.next_job_id();
    let id_b = mgr.next_job_id();

    let token_a = mgr.spawn_task(id_a, |_tx, t| async move {
        // long sleep, cancellable
        tokio::select! {
            _ = sleep(Duration::from_secs(5)) => Ok(()),
            _ = t.cancelled() => Err(anyhow::anyhow!("unreached")),
        }
    });
    let _token_b = mgr.spawn_task(id_b, |_tx, _t| async {
        sleep(Duration::from_millis(50)).await;
        Ok(())
    });

    // Cancel only A
    token_a.cancel();

    let mut saw_a_cancelled = false;
    let mut saw_b_ok = false;
    for _ in 0..2 {
        match recv_next(&mut rx).await {
            TaskEvent::Completed {
                id,
                result: Err(msg),
            } if id == id_a => {
                assert_eq!(msg, "Cancelled");
                saw_a_cancelled = true;
            }
            TaskEvent::Completed { id, result: Ok(()) } if id == id_b => {
                saw_b_ok = true;
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
    assert!(saw_a_cancelled, "job A should have been cancelled");
    assert!(saw_b_ok, "job B should have completed Ok");
}

#[tokio::test]
async fn cancel_all_cancels_everything() {
    let (mgr, mut rx) = new_manager(16, 4);
    let ids: Vec<JobId> = (0..4).map(|_| mgr.next_job_id()).collect();
    for id in &ids {
        let _id = *id;
        mgr.spawn_task(_id, |_tx, t| async move {
            tokio::select! {
                _ = sleep(Duration::from_secs(5)) => Ok(()),
                _ = t.cancelled() => Err(anyhow::anyhow!("cancelled-internal")),
            }
        });
    }

    sleep(Duration::from_millis(50)).await;
    mgr.cancel_all();

    let mut cancelled = 0;
    for _ in 0..4 {
        let ev = recv_next(&mut rx).await;
        if let TaskEvent::Completed {
            result: Err(msg), ..
        } = ev
        {
            assert_eq!(msg, "Cancelled");
            cancelled += 1;
        }
    }
    assert_eq!(cancelled, 4);
}

#[tokio::test]
async fn semaphore_bounds_concurrency_to_8() {
    let (mgr, mut rx) = new_manager(64, 8);
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    for _ in 0..20 {
        let id = mgr.next_job_id();
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        mgr.spawn_task(id, move |_tx, _t| async move {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            sleep(Duration::from_millis(100)).await;
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        });
    }

    // Drain all 20 Completed events
    let mut ok_count = 0;
    for _ in 0..20 {
        if let TaskEvent::Completed { result: Ok(()), .. } = recv_next(&mut rx).await {
            ok_count += 1;
        }
    }
    assert_eq!(ok_count, 20);
    let peak_val = peak.load(Ordering::SeqCst);
    assert!(
        peak_val <= 8,
        "peak concurrency {peak_val} exceeded semaphore bound 8"
    );
}

#[tokio::test]
async fn next_job_id_is_monotonic() {
    let (mgr, _rx) = new_manager(4, 2);
    let a = mgr.next_job_id();
    let b = mgr.next_job_id();
    assert!(b > a, "expected {b} > {a}");
}

#[tokio::test]
async fn error_from_job_surfaces_in_completed() {
    let (mgr, mut rx) = new_manager(4, 2);
    let id = mgr.next_job_id();
    mgr.spawn_task(id, |_tx, _t| async { Err(anyhow::anyhow!("boom")) });

    let ev = recv_next(&mut rx).await;
    match ev {
        TaskEvent::Completed {
            result: Err(msg), ..
        } => {
            assert!(
                msg.contains("boom"),
                "error message should mention 'boom': {msg}"
            );
        }
        other => panic!("expected Completed(Err), got {other:?}"),
    }
}
