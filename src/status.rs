use async_trait::async_trait;
use serde::Serialize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeState {
    WaitingForBluez,
    WaitingForAdapter,
    Advertising,
    WaitingForPhone,
    WaitingForServices,
    WaitingForAuthorization,
    Subscribing,
    Ready,
    Backoff,
    Error,
}

/// Metadata-only runtime state. Payload values have no representation here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSnapshot {
    pub state: RuntimeState,
    pub reason_code: Option<&'static str>,
    pub connected: bool,
    pub services_resolved: bool,
    pub ancs_available: bool,
    pub subscribed: bool,
    pub delivered_count: u64,
    pub recoverable_error_count: u64,
}

impl StatusSnapshot {
    pub fn new(state: RuntimeState) -> Self {
        Self {
            state,
            reason_code: None,
            connected: false,
            services_resolved: false,
            ancs_available: false,
            subscribed: false,
            delivered_count: 0,
            recoverable_error_count: 0,
        }
    }
}

#[async_trait]
pub trait StatusWriter: Send {
    async fn publish(&mut self, snapshot: StatusSnapshot) -> anyhow::Result<()>;
}

#[derive(Default)]
pub struct TracingStatusWriter;

#[async_trait]
impl StatusWriter for TracingStatusWriter {
    async fn publish(&mut self, snapshot: StatusSnapshot) -> anyhow::Result<()> {
        tracing::info!(
            state = ?snapshot.state,
            reason_code = snapshot.reason_code,
            connected = snapshot.connected,
            services_resolved = snapshot.services_resolved,
            ancs_available = snapshot.ancs_available,
            subscribed = snapshot.subscribed,
            delivered_count = snapshot.delivered_count,
            recoverable_error_count = snapshot.recoverable_error_count,
            "runtime state transition"
        );
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct FakeStatusWriter {
    values: Arc<Mutex<Vec<StatusSnapshot>>>,
}

impl FakeStatusWriter {
    pub fn values(&self) -> Vec<StatusSnapshot> {
        self.values.lock().expect("fake status poisoned").clone()
    }
}

#[async_trait]
impl StatusWriter for FakeStatusWriter {
    async fn publish(&mut self, snapshot: StatusSnapshot) -> anyhow::Result<()> {
        self.values
            .lock()
            .expect("fake status poisoned")
            .push(snapshot);
        Ok(())
    }
}
