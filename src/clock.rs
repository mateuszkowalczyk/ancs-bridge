use async_trait::async_trait;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;

#[async_trait]
pub trait Clock: Send + Sync {
    async fn sleep(&self, duration: Duration);
    fn elapsed(&self) -> Duration;
}

#[derive(Clone, Debug)]
pub struct TokioClock {
    started: tokio::time::Instant,
}

impl Default for TokioClock {
    fn default() -> Self {
        Self {
            started: tokio::time::Instant::now(),
        }
    }
}

#[async_trait]
impl Clock for TokioClock {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

#[derive(Clone, Default)]
pub struct FakeClock {
    elapsed: Arc<Mutex<Duration>>,
    changed: Arc<Notify>,
}

impl FakeClock {
    pub fn advance(&self, duration: Duration) {
        *self.elapsed.lock().expect("fake clock poisoned") += duration;
        self.changed.notify_waiters();
    }
}

#[async_trait]
impl Clock for FakeClock {
    async fn sleep(&self, duration: Duration) {
        let target = self.elapsed() + duration;
        while self.elapsed() < target {
            self.changed.notified().await;
        }
    }

    fn elapsed(&self) -> Duration {
        *self.elapsed.lock().expect("fake clock poisoned")
    }
}
