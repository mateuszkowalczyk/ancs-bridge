use crate::{
    ancs::{codec::NotificationEvent, session::SessionEngine},
    bluetooth::transport::{BluetoothTransport, TransportObservation, TransportPacket},
    clock::Clock,
    notification::NotificationSink,
    status::{RuntimeState, StatusSnapshot, StatusWriter},
};
use anyhow::Result;
use std::time::Duration;

pub const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);
const BACKOFF: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

pub struct Supervisor<T, N, C, S> {
    transport: T,
    session: SessionEngine<N, C>,
    clock: C,
    status_writer: S,
    snapshot: StatusSnapshot,
    observed_session_error_count: u64,
    backoff_index: usize,
    session_active: bool,
}

impl<T, N, C, S> Supervisor<T, N, C, S>
where
    T: BluetoothTransport,
    N: NotificationSink,
    C: Clock + Clone,
    S: StatusWriter,
{
    pub fn new(transport: T, sink: N, clock: C, status_writer: S) -> Self {
        Self {
            transport,
            session: SessionEngine::new(sink, clock.clone()),
            clock,
            status_writer,
            snapshot: StatusSnapshot::new(RuntimeState::WaitingForBluez),
            observed_session_error_count: 0,
            backoff_index: 0,
            session_active: false,
        }
    }

    pub fn snapshot(&self) -> &StatusSnapshot {
        &self.snapshot
    }

    pub fn delivered_count(&self) -> u64 {
        self.session.metadata().delivered_count
    }

    pub fn current_backoff(&self) -> Duration {
        BACKOFF[self.backoff_index.min(BACKOFF.len() - 1)]
    }

    fn advance_backoff(&mut self) {
        self.backoff_index = (self.backoff_index + 1).min(BACKOFF.len() - 1);
    }

    async fn publish_current(&mut self) {
        if self
            .status_writer
            .publish(self.snapshot.clone())
            .await
            .is_err()
        {
            tracing::warn!(
                error_code = "status-publish-failed",
                "runtime status update failed"
            );
        }
    }

    async fn publish(&mut self, state: RuntimeState, reason: Option<&'static str>) {
        let is_transition = self.snapshot.state != state || self.snapshot.reason_code != reason;
        self.snapshot.state = state;
        self.snapshot.reason_code = reason;
        if is_transition {
            self.publish_current().await;
        }
    }

    fn sync_session_metadata(&mut self) -> bool {
        let metadata = self.session.metadata();
        let changed = self.snapshot.delivered_count != metadata.delivered_count
            || self.observed_session_error_count != metadata.recoverable_error_count;
        self.snapshot.delivered_count = metadata.delivered_count;
        if metadata.recoverable_error_count > self.observed_session_error_count {
            self.snapshot.recoverable_error_count +=
                metadata.recoverable_error_count - self.observed_session_error_count;
            self.snapshot.last_error_code = metadata.last_error_code;
        }
        self.observed_session_error_count = metadata.recoverable_error_count;
        changed
    }

    async fn end_session(&mut self) {
        if self.session_active {
            self.session.end().await;
            self.transport.end_ancs_session();
            self.session_active = false;
        }
        self.sync_session_metadata();
        self.snapshot.subscribed = false;
        self.snapshot.ancs_available = false;
    }

    /// Reconcile one full layer of BlueZ state. Errors transition to backoff;
    /// callers choose when to sleep so tests remain deterministic.
    pub async fn reconcile_once(&mut self) -> Result<()> {
        let observation = match self.transport.reconcile().await {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    error_code = "bluez-unavailable",
                    "BlueZ reconciliation failed"
                );
                self.end_session().await;
                self.transport.reset_bluez_session();
                self.snapshot.record_error("bluez-unavailable");
                self.snapshot.connected = false;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::Backoff, Some("bluez-unavailable"))
                    .await;
                return Ok(());
            }
        };

        match observation {
            TransportObservation::AdapterPoweredOff => {
                self.end_session().await;
                self.snapshot.connected = false;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::WaitingForAdapter, Some("adapter-powered-off"))
                    .await;
            }
            TransportObservation::Advertising => {
                self.publish(RuntimeState::Advertising, None).await;
            }
            TransportObservation::DeviceNotBonded => {
                self.end_session().await;
                self.snapshot.record_error("configured-device-not-bonded");
                self.snapshot.connected = false;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::Error, Some("configured-device-not-bonded"))
                    .await;
            }
            TransportObservation::WaitingForPhone => {
                self.end_session().await;
                self.snapshot.connected = false;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::WaitingForPhone, None).await;
            }
            TransportObservation::WaitingForServices => {
                self.end_session().await;
                self.snapshot.connected = true;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::WaitingForServices, None).await;
            }
            TransportObservation::WaitingForAuthorization => {
                self.end_session().await;
                self.snapshot.connected = true;
                self.snapshot.services_resolved = true;
                self.publish(RuntimeState::WaitingForAuthorization, None)
                    .await;
            }
            TransportObservation::Available => {
                self.snapshot.connected = true;
                self.snapshot.services_resolved = true;
                self.snapshot.ancs_available = true;
                self.publish(RuntimeState::Subscribing, None).await;
                match self.transport.subscribe().await {
                    Ok(()) => {
                        self.session_active = true;
                        self.snapshot.subscribed = true;
                        self.backoff_index = 0;
                        self.publish(RuntimeState::Ready, None).await;
                    }
                    Err(_) => {
                        self.end_session().await;
                        self.snapshot.record_error("subscribe-failed");
                        self.publish(
                            RuntimeState::WaitingForAuthorization,
                            Some("subscribe-failed"),
                        )
                        .await;
                    }
                }
            }
            TransportObservation::Ready => {
                self.session_active = true;
                self.snapshot.connected = true;
                self.snapshot.services_resolved = true;
                self.snapshot.ancs_available = true;
                self.snapshot.subscribed = true;
                self.backoff_index = 0;
                self.publish(RuntimeState::Ready, None).await;
            }
        }
        Ok(())
    }

    pub async fn handle_one_packet(&mut self) -> Result<()> {
        match self.transport.next_packet().await? {
            TransportPacket::NotificationSource(bytes) => match NotificationEvent::parse(&bytes) {
                Ok(event) => {
                    self.session.ingest(event).await;
                    while self.session.process_next(&mut self.transport).await? {}
                    if self.sync_session_metadata() {
                        self.publish_current().await;
                    }
                }
                Err(_) => {
                    self.snapshot.record_error("malformed-notification-source");
                    self.publish_current().await;
                }
            },
            TransportPacket::DataSource(_) => {
                self.snapshot.record_error("unsolicited-data-source");
                tracing::warn!(
                    error_code = "unsolicited-data-source",
                    "discarded unsolicited ANCS data"
                );
                self.publish_current().await;
            }
        }
        Ok(())
    }

    pub async fn run(&mut self) -> Result<()> {
        self.publish_current().await;
        loop {
            self.reconcile_once().await?;
            match self.snapshot.state {
                RuntimeState::Backoff => {
                    self.clock.sleep(self.current_backoff()).await;
                    self.advance_backoff();
                }
                RuntimeState::Ready => {
                    let clock = self.clock.clone();
                    let reconcile = clock.sleep(RECONCILE_INTERVAL);
                    tokio::pin!(reconcile);
                    tokio::select! {
                        () = &mut reconcile => {}
                        result = self.handle_one_packet() => {
                            if result.is_err() {
                                self.snapshot.record_error("ancs-stream-failed");
                                self.end_session().await;
                                self.publish_current().await;
                            }
                        }
                    }
                }
                _ => self.clock.sleep(RECONCILE_INTERVAL).await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        bluetooth::transport::{FakeBluetoothTransport, TransportPacket},
        clock::FakeClock,
        notification::FakeNotificationSink,
        status::FakeStatusWriter,
    };

    fn notification_response(uid: u32, app: &str) -> Vec<u8> {
        let mut bytes = vec![0];
        bytes.extend(uid.to_le_bytes());
        for (id, value) in [(0, app), (1, "Title"), (3, "Message")] {
            bytes.push(id);
            bytes.extend((value.len() as u16).to_le_bytes());
            bytes.extend(value.as_bytes());
        }
        bytes
    }

    fn app_response(app: &str, name: &str) -> Vec<u8> {
        let mut bytes = vec![1];
        bytes.extend(app.as_bytes());
        bytes.push(0);
        bytes.push(0);
        bytes.extend((name.len() as u16).to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes
    }

    fn supervisor(
        transport: FakeBluetoothTransport,
        clock: FakeClock,
        status: FakeStatusWriter,
    ) -> Supervisor<FakeBluetoothTransport, FakeNotificationSink, FakeClock, FakeStatusWriter> {
        Supervisor::new(transport, FakeNotificationSink::default(), clock, status)
    }

    #[tokio::test]
    async fn reports_delayed_authorization_and_ordered_subscription() {
        let probe = FakeBluetoothTransport::default();
        for observation in [
            TransportObservation::WaitingForPhone,
            TransportObservation::WaitingForServices,
            TransportObservation::WaitingForAuthorization,
            TransportObservation::Available,
        ] {
            probe.observation(observation);
        }
        let status = FakeStatusWriter::default();
        let mut supervisor = supervisor(probe.clone(), FakeClock::default(), status.clone());
        for _ in 0..4 {
            supervisor.reconcile_once().await.unwrap();
        }
        assert_eq!(supervisor.snapshot().state, RuntimeState::Ready);
        let states: Vec<_> = status
            .values()
            .into_iter()
            .map(|value| value.state)
            .collect();
        assert_eq!(
            states,
            vec![
                RuntimeState::WaitingForPhone,
                RuntimeState::WaitingForServices,
                RuntimeState::WaitingForAuthorization,
                RuntimeState::Subscribing,
                RuntimeState::Ready,
            ]
        );
        let calls = probe.calls();
        let data = calls
            .iter()
            .position(|call| *call == "subscribe-data-source")
            .unwrap();
        let source = calls
            .iter()
            .position(|call| *call == "subscribe-notification-source")
            .unwrap();
        assert!(data < source);
    }

    #[tokio::test]
    async fn recovers_disappearance_disconnect_and_missed_events_by_reconciliation() {
        let probe = FakeBluetoothTransport::default();
        for observation in [
            TransportObservation::Available,
            TransportObservation::WaitingForAuthorization,
            TransportObservation::Available,
            TransportObservation::WaitingForPhone,
            TransportObservation::Available,
        ] {
            probe.observation(observation);
        }
        let mut supervisor = supervisor(
            probe.clone(),
            FakeClock::default(),
            FakeStatusWriter::default(),
        );
        let mut states = Vec::new();
        for _ in 0..5 {
            supervisor.reconcile_once().await.unwrap();
            states.push(supervisor.snapshot().state);
        }
        assert_eq!(
            states,
            vec![
                RuntimeState::Ready,
                RuntimeState::WaitingForAuthorization,
                RuntimeState::Ready,
                RuntimeState::WaitingForPhone,
                RuntimeState::Ready,
            ]
        );
        assert!(!probe.calls().contains(&"connect-device"));
    }

    #[tokio::test]
    async fn bluez_and_adapter_failures_back_off_and_success_resets_sequence() {
        let probe = FakeBluetoothTransport::default();
        for _ in 0..6 {
            probe.observation_error("BlueZ gone");
        }
        probe.observation(TransportObservation::Available);
        probe.observation_error("adapter gone");
        let mut supervisor = supervisor(
            probe.clone(),
            FakeClock::default(),
            FakeStatusWriter::default(),
        );
        let mut delays = Vec::new();
        for _ in 0..6 {
            supervisor.reconcile_once().await.unwrap();
            delays.push(supervisor.current_backoff());
            supervisor.advance_backoff();
        }
        assert_eq!(
            delays,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(5),
                Duration::from_secs(10),
                Duration::from_secs(30),
                Duration::from_secs(30)
            ]
        );
        supervisor.reconcile_once().await.unwrap();
        assert_eq!(supervisor.snapshot().state, RuntimeState::Ready);
        assert_eq!(supervisor.current_backoff(), Duration::from_secs(1));
        supervisor.reconcile_once().await.unwrap();
        assert_eq!(supervisor.snapshot().state, RuntimeState::Backoff);
    }

    #[tokio::test]
    async fn reports_unrecoverable_configuration_error() {
        let probe = FakeBluetoothTransport::default();
        probe.observation(TransportObservation::DeviceNotBonded);
        let status = FakeStatusWriter::default();
        let mut supervisor = supervisor(probe, FakeClock::default(), status.clone());
        supervisor.reconcile_once().await.unwrap();
        assert_eq!(supervisor.snapshot().state, RuntimeState::Error);
        assert_eq!(
            supervisor.snapshot().reason_code,
            Some("configured-device-not-bonded")
        );
        assert_eq!(status.values().last().unwrap().state, RuntimeState::Error);
    }

    #[tokio::test]
    async fn republishes_delivery_and_error_metadata_without_a_state_change() {
        let probe = FakeBluetoothTransport::default();
        probe.observation(TransportObservation::Available);
        let status = FakeStatusWriter::default();
        let mut supervisor = supervisor(probe.clone(), FakeClock::default(), status.clone());
        supervisor.reconcile_once().await.unwrap();

        probe.packet(TransportPacket::NotificationSource(vec![
            0, 0, 0, 1, 7, 0, 0, 0,
        ]));
        probe.packet(TransportPacket::DataSource(notification_response(
            7,
            "secret.bundle",
        )));
        probe.packet(TransportPacket::DataSource(app_response(
            "secret.bundle",
            "Secret App",
        )));
        supervisor.handle_one_packet().await.unwrap();
        let delivered = status.values().last().unwrap().clone();
        assert_eq!(delivered.state, RuntimeState::Ready);
        assert_eq!(delivered.delivered_count, 1);

        probe.packet(TransportPacket::DataSource(vec![0]));
        supervisor.handle_one_packet().await.unwrap();
        let error = status.values().last().unwrap().clone();
        assert_eq!(error.state, RuntimeState::Ready);
        assert_eq!(error.last_error_code, Some("unsolicited-data-source"));
        let json = serde_json::to_string(&error).unwrap();
        for canary in ["secret.bundle", "Secret App", "Title", "Message"] {
            assert!(!json.contains(canary));
        }
    }

    #[tokio::test]
    async fn status_publication_failure_does_not_stop_reconciliation() {
        let probe = FakeBluetoothTransport::default();
        probe.observation(TransportObservation::WaitingForPhone);
        probe.observation(TransportObservation::Available);
        let status = FakeStatusWriter::default();
        status.fail_next();
        let mut supervisor = supervisor(probe, FakeClock::default(), status.clone());
        supervisor.reconcile_once().await.unwrap();
        assert_eq!(supervisor.snapshot().state, RuntimeState::WaitingForPhone);
        supervisor.reconcile_once().await.unwrap();
        assert_eq!(supervisor.snapshot().state, RuntimeState::Ready);
        assert_eq!(status.values().last().unwrap().state, RuntimeState::Ready);
    }
}
