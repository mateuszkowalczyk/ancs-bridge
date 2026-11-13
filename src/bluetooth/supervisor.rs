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
const PASSIVE_REFRESH_INITIAL_DELAY: Duration = Duration::from_secs(15);
const PASSIVE_REFRESH_MAX_DELAY: Duration = Duration::from_secs(300);
const SUBSCRIPTION_FAILURES_BEFORE_REFRESH: u8 = 2;

pub struct Supervisor<T, N, C, S> {
    transport: T,
    session: SessionEngine<N, C>,
    clock: C,
    status_writer: S,
    snapshot: StatusSnapshot,
    observed_session_error_count: u64,
    backoff_index: usize,
    session_active: bool,
    next_passive_refresh: Option<Duration>,
    passive_refresh_delay: Duration,
    subscription_failures: u8,
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
            next_passive_refresh: None,
            passive_refresh_delay: PASSIVE_REFRESH_INITIAL_DELAY,
            subscription_failures: 0,
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
            self.session_active = false;
        }
        self.transport.end_ancs_session().await;
        self.sync_session_metadata();
        self.snapshot.subscribed = false;
        self.snapshot.ancs_available = false;
    }

    fn reset_recovery_tracking(&mut self) {
        self.next_passive_refresh = None;
        self.passive_refresh_delay = PASSIVE_REFRESH_INITIAL_DELAY;
        self.subscription_failures = 0;
    }

    fn waiting_for_phone_needs_refresh(&mut self) -> bool {
        let now = self.clock.elapsed();
        let deadline = self
            .next_passive_refresh
            .get_or_insert(now + self.passive_refresh_delay);
        now >= *deadline
    }

    async fn refresh_bluez_session(&mut self, cause: &'static str) {
        self.passive_refresh_delay =
            (self.passive_refresh_delay * 2).min(PASSIVE_REFRESH_MAX_DELAY);
        tracing::info!(
            event_code = "bluez-session-refresh",
            recovery_cause = cause,
            retry_after_secs = self.passive_refresh_delay.as_secs(),
            "recreating BlueZ session and registrations for passive recovery"
        );
        self.transport.reset_bluez_session().await;
        self.next_passive_refresh = Some(self.clock.elapsed() + self.passive_refresh_delay);
        self.subscription_failures = 0;
    }

    async fn update_device_name(&mut self) {
        if let Err(error) = self
            .status_writer
            .update_device_name(self.transport.current_device_name())
            .await
        {
            tracing::warn!(
                error = %error,
                error_code = "status-publish-failed",
                "runtime device name update failed"
            );
        }
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
                self.transport.reset_bluez_session().await;
                self.reset_recovery_tracking();
                self.update_device_name().await;
                self.snapshot.record_error("bluez-unavailable");
                self.snapshot.connected = false;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::Backoff, Some("bluez-unavailable"))
                    .await;
                return Ok(());
            }
        };
        self.update_device_name().await;

        match observation {
            TransportObservation::AdapterPoweredOff => {
                self.end_session().await;
                self.next_passive_refresh = None;
                self.snapshot.connected = false;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::WaitingForAdapter, Some("adapter-powered-off"))
                    .await;
            }
            TransportObservation::Advertising => {
                self.end_session().await;
                self.snapshot.connected = false;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::Advertising, None).await;
            }
            TransportObservation::DeviceNotBonded => {
                self.end_session().await;
                self.next_passive_refresh = None;
                self.snapshot.record_error("configured-device-not-bonded");
                self.snapshot.connected = false;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::Error, Some("configured-device-not-bonded"))
                    .await;
            }
            TransportObservation::WaitingForPhone => {
                let had_active_session = self.session_active;
                self.end_session().await;
                if had_active_session {
                    self.refresh_bluez_session("phone-disconnected").await;
                } else if self.waiting_for_phone_needs_refresh() {
                    self.refresh_bluez_session("startup-stall").await;
                }
                self.snapshot.connected = false;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::WaitingForPhone, None).await;
            }
            TransportObservation::WaitingForServices => {
                self.end_session().await;
                self.next_passive_refresh = None;
                self.snapshot.connected = true;
                self.snapshot.services_resolved = false;
                self.publish(RuntimeState::WaitingForServices, None).await;
            }
            TransportObservation::WaitingForAuthorization => {
                self.end_session().await;
                self.next_passive_refresh = None;
                self.snapshot.connected = true;
                self.snapshot.services_resolved = true;
                self.publish(RuntimeState::WaitingForAuthorization, None)
                    .await;
            }
            TransportObservation::Available => {
                self.next_passive_refresh = None;
                self.snapshot.connected = true;
                self.snapshot.services_resolved = true;
                self.snapshot.ancs_available = true;
                self.publish(RuntimeState::Subscribing, None).await;
                match self.transport.subscribe().await {
                    Ok(()) => {
                        self.session_active = true;
                        self.snapshot.subscribed = true;
                        self.backoff_index = 0;
                        self.reset_recovery_tracking();
                        self.publish(RuntimeState::Ready, None).await;
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = %format_args!("{:#}", error.error()),
                            error_code = "subscribe-failed",
                            subscription_stage = error.stage(),
                            "ANCS subscription failed"
                        );
                        self.end_session().await;
                        self.snapshot.record_error("subscribe-failed");
                        self.subscription_failures = self.subscription_failures.saturating_add(1);
                        if self.subscription_failures >= SUBSCRIPTION_FAILURES_BEFORE_REFRESH {
                            self.refresh_bluez_session("subscription-failure").await;
                        }
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
                self.reset_recovery_tracking();
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
                    self.process_one_pending().await?;
                }
                Err(error) => {
                    tracing::warn!(
                        error = ?error,
                        packet_length = bytes.len(),
                        error_code = "malformed-notification-source",
                        "discarded malformed ANCS notification event"
                    );
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

    async fn process_one_pending(&mut self) -> Result<()> {
        self.session.process_next(&mut self.transport).await?;
        if self.sync_session_metadata() {
            self.publish_current().await;
        }
        Ok(())
    }

    async fn recover_stream_failure(&mut self) {
        self.snapshot.record_error("ancs-stream-failed");
        self.end_session().await;
        self.publish(RuntimeState::Backoff, Some("ancs-stream-failed"))
            .await;
        // BlueZ may still report the phone as connected after a stream ends.
        // Do not let successful discovery turn repeated stream failures into a busy loop.
        self.clock.sleep(RECONCILE_INTERVAL).await;
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
                    if self.session.pending_len() > 0 {
                        if self.process_one_pending().await.is_err() {
                            self.recover_stream_failure().await;
                        }
                        continue;
                    }
                    let clock = self.clock.clone();
                    let reconcile = clock.sleep(RECONCILE_INTERVAL);
                    tokio::pin!(reconcile);
                    tokio::select! {
                        () = &mut reconcile => {}
                        result = self.handle_one_packet() => {
                            if result.is_err() {
                                self.recover_stream_failure().await;
                            }
                        }
                    }
                }
                _ => self.clock.sleep(RECONCILE_INTERVAL).await,
            }
        }
    }

    pub async fn shutdown(&mut self) {
        if self.session_active {
            self.session.end().await;
            self.session_active = false;
        }
        self.sync_session_metadata();
        self.snapshot.subscribed = false;
        self.snapshot.ancs_available = false;
        self.transport.shutdown().await;
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
        assert!(!calls.contains(&"reset-bluez-session"));
    }

    #[tokio::test]
    async fn forwards_current_bluez_device_name_to_status_writer() {
        let probe = FakeBluetoothTransport::default();
        probe.device_name("iPhone (Mateusz)");
        probe.observation(TransportObservation::WaitingForPhone);
        let status = FakeStatusWriter::default();
        let mut supervisor = supervisor(probe, FakeClock::default(), status.clone());

        supervisor.reconcile_once().await.unwrap();

        assert_eq!(
            status.device_names(),
            vec![Some("iPhone (Mateusz)".to_owned())]
        );
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
        let calls = probe.calls();
        assert_eq!(
            calls
                .iter()
                .filter(|call| **call == "reset-bluez-session")
                .count(),
            1
        );
        assert!(!calls.contains(&"connect-device"));
    }

    #[tokio::test]
    async fn fresh_start_waiting_for_phone_refreshes_registrations_after_bounded_stall() {
        let probe = FakeBluetoothTransport::default();
        for observation in [
            TransportObservation::WaitingForPhone,
            TransportObservation::WaitingForPhone,
            TransportObservation::WaitingForPhone,
            TransportObservation::WaitingForPhone,
            TransportObservation::Available,
        ] {
            probe.observation(observation);
        }
        let clock = FakeClock::default();
        let mut supervisor = supervisor(probe.clone(), clock.clone(), FakeStatusWriter::default());

        for _ in 0..5 {
            supervisor.reconcile_once().await.unwrap();
            clock.advance(RECONCILE_INTERVAL);
        }

        assert_eq!(supervisor.snapshot().state, RuntimeState::Ready);
        assert_eq!(
            probe
                .calls()
                .iter()
                .filter(|call| **call == "reset-bluez-session")
                .count(),
            1
        );
        assert!(!probe.calls().contains(&"connect-device"));
    }

    #[tokio::test]
    async fn short_startup_wait_does_not_churn_registrations() {
        let probe = FakeBluetoothTransport::default();
        for observation in [
            TransportObservation::WaitingForPhone,
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

        for _ in 0..3 {
            supervisor.reconcile_once().await.unwrap();
        }

        assert_eq!(supervisor.snapshot().state, RuntimeState::Ready);
        assert!(!probe.calls().contains(&"reset-bluez-session"));
    }

    #[tokio::test]
    async fn startup_refresh_uses_elapsed_time_and_caps_the_delay() {
        let probe = FakeBluetoothTransport::default();
        let clock = FakeClock::default();
        let mut supervisor = supervisor(probe.clone(), clock.clone(), FakeStatusWriter::default());
        probe.observation(TransportObservation::WaitingForPhone);
        supervisor.reconcile_once().await.unwrap();

        for (attempt, seconds) in [15, 30, 60, 120, 240, 300, 300].into_iter().enumerate() {
            // Reconciliation frequency alone must not trigger a refresh.
            for _ in 0..20 {
                probe.observation(TransportObservation::WaitingForPhone);
                supervisor.reconcile_once().await.unwrap();
            }
            clock.advance(Duration::from_secs(seconds) - Duration::from_millis(1));
            probe.observation(TransportObservation::WaitingForPhone);
            supervisor.reconcile_once().await.unwrap();
            assert_eq!(
                probe
                    .calls()
                    .iter()
                    .filter(|call| **call == "reset-bluez-session")
                    .count(),
                attempt
            );

            clock.advance(Duration::from_millis(1));
            probe.observation(TransportObservation::WaitingForPhone);
            supervisor.reconcile_once().await.unwrap();
            assert_eq!(
                probe
                    .calls()
                    .iter()
                    .filter(|call| **call == "reset-bluez-session")
                    .count(),
                attempt + 1
            );
            // Re-registering the advertisement must not restart the deadline.
            probe.observation(TransportObservation::Advertising);
            supervisor.reconcile_once().await.unwrap();
        }
        assert!(!probe.calls().contains(&"connect-device"));
    }

    #[tokio::test]
    async fn ready_and_bluez_errors_reset_passive_refresh_delay() {
        for reconnect in [false, true] {
            let probe = FakeBluetoothTransport::default();
            let clock = FakeClock::default();
            let mut supervisor =
                supervisor(probe.clone(), clock.clone(), FakeStatusWriter::default());
            probe.observation(TransportObservation::WaitingForPhone);
            supervisor.reconcile_once().await.unwrap();
            for seconds in [15, 30, 60, 120, 240, 300] {
                clock.advance(Duration::from_secs(seconds));
                probe.observation(TransportObservation::WaitingForPhone);
                supervisor.reconcile_once().await.unwrap();
            }

            if reconnect {
                probe.observation(TransportObservation::Available);
            } else {
                probe.observation_error("BlueZ gone");
            }
            supervisor.reconcile_once().await.unwrap();
            assert_eq!(
                supervisor.snapshot().state,
                if reconnect {
                    RuntimeState::Ready
                } else {
                    RuntimeState::Backoff
                }
            );
            probe.observation(TransportObservation::WaitingForPhone);
            supervisor.reconcile_once().await.unwrap();
            let resets = probe
                .calls()
                .iter()
                .filter(|call| **call == "reset-bluez-session")
                .count();
            // An active disconnect refreshes immediately, consuming the first step.
            let delay = if reconnect { 30 } else { 15 };
            clock.advance(Duration::from_secs(delay - 1));
            probe.observation(TransportObservation::WaitingForPhone);
            supervisor.reconcile_once().await.unwrap();
            assert_eq!(
                probe
                    .calls()
                    .iter()
                    .filter(|call| **call == "reset-bluez-session")
                    .count(),
                resets
            );
            clock.advance(Duration::from_secs(1));
            probe.observation(TransportObservation::WaitingForPhone);
            supervisor.reconcile_once().await.unwrap();
            assert_eq!(
                probe
                    .calls()
                    .iter()
                    .filter(|call| **call == "reset-bluez-session")
                    .count(),
                resets + 1
            );
        }
    }

    #[tokio::test]
    async fn repeated_startup_subscription_failure_refreshes_and_recovers() {
        let probe = FakeBluetoothTransport::default();
        probe.fail_subscriptions(2);
        for _ in 0..3 {
            probe.observation(TransportObservation::Available);
        }
        let mut supervisor = supervisor(
            probe.clone(),
            FakeClock::default(),
            FakeStatusWriter::default(),
        );

        for _ in 0..3 {
            supervisor.reconcile_once().await.unwrap();
        }

        assert_eq!(supervisor.snapshot().state, RuntimeState::Ready);
        assert_eq!(
            probe
                .calls()
                .iter()
                .filter(|call| **call == "reset-bluez-session")
                .count(),
            1
        );
        assert_eq!(supervisor.current_backoff(), Duration::from_secs(1));
        let calls = probe.calls();
        let first_end = calls
            .iter()
            .position(|call| *call == "end-ancs-session")
            .unwrap();
        let refresh = calls
            .iter()
            .position(|call| *call == "reset-bluez-session")
            .unwrap();
        assert!(first_end < refresh);
        assert!(!calls.contains(&"connect-device"));
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
        assert_eq!(
            probe
                .calls()
                .iter()
                .filter(|call| **call == "reset-bluez-session")
                .count(),
            7
        );
        assert!(!probe.calls().contains(&"connect-device"));
    }

    #[tokio::test]
    async fn adapter_power_cycle_ends_the_session_and_resubscribes_passively() {
        let probe = FakeBluetoothTransport::default();
        for observation in [
            TransportObservation::Available,
            TransportObservation::AdapterPoweredOff,
            TransportObservation::Available,
        ] {
            probe.observation(observation);
        }
        let mut supervisor = supervisor(
            probe.clone(),
            FakeClock::default(),
            FakeStatusWriter::default(),
        );

        supervisor.reconcile_once().await.unwrap();
        assert_eq!(supervisor.snapshot().state, RuntimeState::Ready);
        supervisor.reconcile_once().await.unwrap();
        assert_eq!(supervisor.snapshot().state, RuntimeState::WaitingForAdapter);
        assert_eq!(
            supervisor.snapshot().reason_code,
            Some("adapter-powered-off")
        );
        supervisor.reconcile_once().await.unwrap();
        assert_eq!(supervisor.snapshot().state, RuntimeState::Ready);

        let calls = probe.calls();
        assert_eq!(
            calls
                .iter()
                .filter(|call| **call == "subscribe-data-source")
                .count(),
            2
        );
        assert!(calls.contains(&"end-ancs-session"));
        assert!(!calls.contains(&"reset-bluez-session"));
        assert!(!calls.contains(&"connect-device"));
    }

    #[tokio::test]
    async fn registration_recreation_clears_stale_connection_metadata() {
        let probe = FakeBluetoothTransport::default();
        probe.observation(TransportObservation::Available);
        probe.observation(TransportObservation::Advertising);
        let mut supervisor = supervisor(probe, FakeClock::default(), FakeStatusWriter::default());

        supervisor.reconcile_once().await.unwrap();
        assert!(supervisor.snapshot().connected);
        assert!(supervisor.snapshot().subscribed);
        supervisor.reconcile_once().await.unwrap();

        assert_eq!(supervisor.snapshot().state, RuntimeState::Advertising);
        assert!(!supervisor.snapshot().connected);
        assert!(!supervisor.snapshot().services_resolved);
        assert!(!supervisor.snapshot().ancs_available);
        assert!(!supervisor.snapshot().subscribed);
    }

    #[tokio::test]
    async fn graceful_shutdown_ends_the_active_session() {
        let probe = FakeBluetoothTransport::default();
        probe.observation(TransportObservation::Available);
        let mut supervisor = supervisor(
            probe.clone(),
            FakeClock::default(),
            FakeStatusWriter::default(),
        );
        supervisor.reconcile_once().await.unwrap();
        assert_eq!(supervisor.snapshot().state, RuntimeState::Ready);

        supervisor.shutdown().await;

        assert!(probe.calls().contains(&"end-ancs-session"));
        assert!(!supervisor.snapshot().subscribed);
        assert!(!supervisor.snapshot().ancs_available);
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

    #[tokio::test]
    async fn stream_failure_waits_before_resubscribing_and_then_resumes_delivery() {
        let probe = FakeBluetoothTransport::default();
        probe.wait_when_empty();
        probe.observation(TransportObservation::Available);
        probe.observation(TransportObservation::Available);
        probe.observation(TransportObservation::Available);
        probe.packet_error("Notification Source subscription ended");
        probe.packet_error("Data Source subscription ended");
        probe.packet(TransportPacket::NotificationSource(vec![
            0, 0, 0, 1, 7, 0, 0, 0,
        ]));
        probe.packet(TransportPacket::DataSource(notification_response(
            7, "app.one",
        )));
        probe.packet(TransportPacket::DataSource(app_response("app.one", "One")));
        let clock = FakeClock::default();
        let status = FakeStatusWriter::default();
        let mut supervisor = supervisor(probe, clock.clone(), status.clone());
        let run = supervisor.run();
        tokio::pin!(run);

        assert!(futures::poll!(&mut run).is_pending());
        let failed = status.values().last().unwrap().clone();
        assert_eq!(failed.state, RuntimeState::Backoff);
        assert_eq!(failed.reason_code, Some("ancs-stream-failed"));
        assert!(!failed.subscribed);
        assert_eq!(failed.delivered_count, 0);
        assert_eq!(failed.recoverable_error_count, 1);

        clock.advance(RECONCILE_INTERVAL - Duration::from_millis(1));
        assert!(futures::poll!(&mut run).is_pending());
        assert_eq!(status.values().last().unwrap().delivered_count, 0);

        clock.advance(Duration::from_millis(1));
        assert!(futures::poll!(&mut run).is_pending());
        let retried = status.values().last().unwrap().clone();
        assert_eq!(retried.state, RuntimeState::Backoff);
        assert_eq!(retried.delivered_count, 0);
        assert_eq!(retried.recoverable_error_count, 2);

        clock.advance(RECONCILE_INTERVAL - Duration::from_millis(1));
        assert!(futures::poll!(&mut run).is_pending());
        assert_eq!(status.values().last().unwrap().delivered_count, 0);

        clock.advance(Duration::from_millis(1));
        assert!(futures::poll!(&mut run).is_pending());
        let recovered = status.values().last().unwrap().clone();
        assert_eq!(recovered.state, RuntimeState::Ready);
        assert_eq!(recovered.delivered_count, 1);
        assert_eq!(recovered.recoverable_error_count, 2);
    }

    #[tokio::test]
    async fn continuous_ancs_work_returns_to_reconciliation_between_requests() {
        let probe = FakeBluetoothTransport::default();
        probe.wait_when_empty();
        let status = FakeStatusWriter::default();
        let mut supervisor = supervisor(probe.clone(), FakeClock::default(), status.clone());

        probe.packet(TransportPacket::NotificationSource(vec![
            0, 0, 0, 1, 1, 0, 0, 0,
        ]));
        probe.packet(TransportPacket::NotificationSource(vec![
            0, 0, 0, 1, 2, 0, 0, 0,
        ]));
        probe.packet(TransportPacket::DataSource(notification_response(
            1, "app.one",
        )));
        probe.packet(TransportPacket::DataSource(app_response("app.one", "One")));
        probe.packet(TransportPacket::DataSource(notification_response(
            2, "app.two",
        )));
        probe.packet(TransportPacket::DataSource(app_response("app.two", "Two")));

        let task = tokio::spawn(async move { supervisor.run().await });
        for _ in 0..100 {
            if status
                .values()
                .last()
                .is_some_and(|value| value.delivered_count == 2)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        task.abort();
        let _ = task.await;

        let calls = probe.calls();
        let second_reconcile = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| **call == "reconcile")
            .nth(1)
            .map(|(index, _)| index)
            .expect("second reconciliation");
        assert_eq!(
            calls[..second_reconcile]
                .iter()
                .filter(|call| **call == "write-control")
                .count(),
            2
        );
    }
}
