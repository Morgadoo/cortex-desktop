//! Serializes macOS Keychain operations across accounts and connectors.
//!
//! Active operations retain exclusive access until native execution completes,
//! including when their awaiting tasks are cancelled.

use std::sync::{Arc, LazyLock};
use tokio::sync::{oneshot, Mutex};

// Process-wide: the macOS Keychain serves the whole process, and its operations run one at a time
// across every account and connector, so they wait in one queue.
static QUEUE: LazyLock<KeychainQueue> = LazyLock::new(KeychainQueue::default);

#[derive(Default)]
struct KeychainQueue {
    turn: Arc<Mutex<()>>,
}

impl KeychainQueue {
    async fn run<T, F>(&self, operation: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        let turn = self.turn.clone().lock_owned().await;
        let (cancellation_sender, cancellation_receiver) = oneshot::channel::<()>();
        let result = tokio::task::spawn_blocking(move || {
            let _turn = turn;
            if cancellation_sender.is_closed() {
                return Err("secure storage request cancelled".into());
            }
            operation()
        })
        .await
        .map_err(|err| format!("secure storage task failed: {err}"));
        drop(cancellation_receiver);
        result?
    }

    fn run_blocking<T>(&self, operation: impl FnOnce() -> T) -> T {
        let _turn = futures::executor::block_on(self.turn.clone().lock_owned());
        operation()
    }
}

pub(in crate::tools) async fn run<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    QUEUE.run(operation).await
}

pub(in crate::tools) fn run_blocking<T>(operation: impl FnOnce() -> T) -> T {
    QUEUE.run_blocking(operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::poll;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex as StdMutex,
    };
    use std::task::Poll;
    use std::time::Duration;

    #[tokio::test]
    async fn operations_run_in_fifo_order_and_skip_cancelled_waiters() {
        let queue = KeychainQueue::default();
        let held = queue.turn.clone().lock_owned().await;
        let order = Arc::new(StdMutex::new(Vec::new()));
        let operation = |id| {
            let order = order.clone();
            move || {
                order.lock().unwrap().push(id);
                Ok(id)
            }
        };
        let mut first = Box::pin(queue.run(operation(1)));
        let mut cancelled = Box::pin(queue.run(operation(99)));
        let mut second = Box::pin(queue.run(operation(2)));
        let mut third = Box::pin(queue.run(operation(3)));
        assert!(poll!(&mut first).is_pending());
        assert!(poll!(&mut cancelled).is_pending());
        assert!(poll!(&mut second).is_pending());
        assert!(poll!(&mut third).is_pending());
        drop(cancelled);
        drop(held);
        let results = futures::join!(third, second, first);
        assert_eq!(results, (Ok(3), Ok(2), Ok(1)));
        assert_eq!(*order.lock().unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn cancellation_before_the_blocking_task_starts_skips_native_access() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (started, ready) = oneshot::channel();
            let (release, finish) = mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                finish.recv().unwrap();
            });
            tokio::time::timeout(Duration::from_secs(5), ready)
                .await
                .unwrap()
                .unwrap();
            let queue = KeychainQueue::default();
            let ran = Arc::new(AtomicBool::new(false));
            let flag = ran.clone();
            let mut cancelled = Box::pin(queue.run(move || {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            }));
            assert!(poll!(&mut cancelled).is_pending());
            assert!(queue.turn.try_lock().is_err());
            drop(cancelled);
            release.send(()).unwrap();
            blocker.await.unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(5), queue.run(|| Ok(())))
                    .await
                    .unwrap(),
                Ok(())
            );
            assert!(!ran.load(Ordering::SeqCst));
        });
    }

    #[tokio::test]
    async fn cancelling_an_active_request_keeps_its_turn_until_native_completion() {
        let queue = Arc::new(KeychainQueue::default());
        let (started, ready) = oneshot::channel();
        let (release, finish) = mpsc::channel();
        let active_queue = queue.clone();
        let active = tokio::spawn(async move {
            active_queue
                .run(move || {
                    started.send(()).unwrap();
                    finish.recv().unwrap();
                    Ok(())
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap();
        active.abort();
        assert!(active.await.unwrap_err().is_cancelled());
        let mut next = Box::pin(queue.run(|| Ok("next")));
        assert!(poll!(&mut next).is_pending());
        release.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), next)
                .await
                .unwrap(),
            Ok("next")
        );
    }

    #[tokio::test]
    async fn synchronous_cleanup_and_async_requests_share_the_queue() {
        let queue = Arc::new(KeychainQueue::default());
        let (started, ready) = oneshot::channel();
        let (release, finish) = mpsc::channel();
        let cleanup_queue = queue.clone();
        let cleanup = std::thread::spawn(move || {
            cleanup_queue.run_blocking(|| {
                started.send(()).unwrap();
                finish.recv().unwrap();
            })
        });
        tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap();
        let mut next = Box::pin(queue.run(|| Ok(())));
        assert!(matches!(poll!(&mut next), Poll::Pending));
        release.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), next)
                .await
                .unwrap(),
            Ok(())
        );
        cleanup.join().unwrap();
    }
}
