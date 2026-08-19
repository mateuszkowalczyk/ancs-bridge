use crate::{
    ancs::codec::{
        app_attributes_request, notification_attributes_request, AppAttributes, DataSourceDecoder,
        DecodedResponse, EventKind, NotificationAttributes, NotificationEvent, ResponseExpectation,
    },
    bluetooth::transport::{BluetoothTransport, ControlWrite, TransportPacket},
    clock::Clock,
    notification::{DesktopHandle, NotificationPayload, NotificationSink},
};
use anyhow::{bail, Result};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_PENDING_UIDS: usize = 100;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionMetadata {
    pub delivered_count: u64,
    pub recoverable_error_count: u64,
    pub dropped_event_count: u64,
    pub timed_out_request_count: u64,
    pub last_error_code: Option<&'static str>,
}

/// One active iPhone ANCS session. All payload-bearing maps are destroyed by
/// `end`; only count-based diagnostic metadata is externally visible.
pub struct SessionEngine<N, C> {
    sink: N,
    clock: C,
    pending_order: VecDeque<u32>,
    pending_kind: HashMap<u32, EventKind>,
    outstanding: Option<u32>,
    canceled: HashSet<u32>,
    handles: HashMap<u32, DesktopHandle>,
    app_names: HashMap<String, String>,
    decoder: DataSourceDecoder,
    metadata: SessionMetadata,
}

impl<N, C> SessionEngine<N, C>
where
    N: NotificationSink,
    C: Clock + Clone,
{
    pub fn new(sink: N, clock: C) -> Self {
        Self {
            sink,
            clock,
            pending_order: VecDeque::new(),
            pending_kind: HashMap::new(),
            outstanding: None,
            canceled: HashSet::new(),
            handles: HashMap::new(),
            app_names: HashMap::new(),
            decoder: DataSourceDecoder::default(),
            metadata: SessionMetadata::default(),
        }
    }

    pub fn metadata(&self) -> SessionMetadata {
        self.metadata
    }

    pub fn pending_len(&self) -> usize {
        self.pending_kind.len()
            + usize::from(
                self.outstanding
                    .is_some_and(|uid| !self.pending_kind.contains_key(&uid)),
            )
    }

    pub fn cached_app_count(&self) -> usize {
        self.app_names.len()
    }

    pub async fn ingest(&mut self, event: NotificationEvent) {
        if event.flags.is_pre_existing() && event.kind != EventKind::Removed {
            self.metadata.dropped_event_count += 1;
            return;
        }
        match event.kind {
            EventKind::Removed => {
                self.pending_kind.remove(&event.uid);
                self.pending_order.retain(|uid| *uid != event.uid);
                if self.outstanding == Some(event.uid) {
                    self.canceled.insert(event.uid);
                }
                if let Some(handle) = self.handles.remove(&event.uid) {
                    if self.sink.close(handle).await.is_err() {
                        self.metadata.recoverable_error_count += 1;
                        self.metadata.last_error_code = Some("desktop-close-failed");
                    }
                }
            }
            EventKind::Added | EventKind::Modified => {
                if let Some(kind) = self.pending_kind.get_mut(&event.uid) {
                    if event.kind == EventKind::Modified {
                        *kind = EventKind::Modified;
                    }
                    return;
                }
                if self.outstanding == Some(event.uid) {
                    if event.kind == EventKind::Modified {
                        self.pending_kind.insert(event.uid, EventKind::Modified);
                        self.pending_order.push_back(event.uid);
                    }
                    return;
                }
                if self.pending_len() >= MAX_PENDING_UIDS {
                    self.metadata.dropped_event_count += 1;
                    return;
                }
                self.pending_kind.insert(event.uid, event.kind);
                self.pending_order.push_back(event.uid);
            }
        }
    }

    pub async fn process_next<T: BluetoothTransport>(&mut self, transport: &mut T) -> Result<bool> {
        let Some(uid) = self.pending_order.pop_front() else {
            return Ok(false);
        };
        let Some(kind) = self.pending_kind.remove(&uid) else {
            return Ok(true);
        };
        self.outstanding = Some(uid);
        self.canceled.remove(&uid);

        let notification = self.fetch_notification(transport, uid).await;
        self.outstanding = None;
        let attributes = match notification {
            Ok(value) if !self.canceled.remove(&uid) => value,
            Ok(_) => return Ok(true),
            Err(_error) => {
                self.metadata.recoverable_error_count += 1;
                self.metadata.last_error_code = Some("notification-attributes-failed");
                tracing::warn!(
                    uid,
                    error_code = "notification-attributes-failed",
                    "recoverable ANCS request failure"
                );
                return Ok(true);
            }
        };

        let app_name = match self.app_names.get(&attributes.app_identifier) {
            Some(value) => value.clone(),
            None => match self
                .fetch_app_name(transport, &attributes.app_identifier)
                .await
            {
                Ok(value) => {
                    self.app_names
                        .insert(attributes.app_identifier.clone(), value.clone());
                    value
                }
                Err(error) => {
                    self.metadata.recoverable_error_count += 1;
                    self.metadata.last_error_code = Some("app-attributes-failed");
                    tracing::warn!(
                        uid,
                        error_code = "app-attributes-failed",
                        "using app identifier fallback after recoverable lookup failure"
                    );
                    let _ = error;
                    attributes.app_identifier.clone()
                }
            },
        };
        if self.canceled.remove(&uid) {
            return Ok(true);
        }
        let payload = NotificationPayload::new(app_name, attributes.title, attributes.message);
        let delivery = if let Some(handle) = self.handles.get(&uid).copied() {
            self.sink.replace(handle, payload).await.map(|()| handle)
        } else {
            self.sink.create(payload).await
        };
        match delivery {
            Ok(handle) => {
                self.handles.insert(uid, handle);
                self.metadata.delivered_count += 1;
            }
            Err(_) => {
                self.metadata.recoverable_error_count += 1;
                self.metadata.last_error_code = Some("desktop-delivery-failed");
                tracing::warn!(uid, event = ?kind, error_code = "desktop-delivery-failed", "recoverable notification delivery failure");
            }
        }
        Ok(true)
    }

    async fn fetch_notification<T: BluetoothTransport>(
        &mut self,
        transport: &mut T,
        uid: u32,
    ) -> Result<NotificationAttributes> {
        let request = notification_attributes_request(uid);
        self.decoder
            .expect(ResponseExpectation::Notification { uid });
        if let Err(error) = transport
            .write_control(&request.bytes, ControlWrite::Request)
            .await
        {
            self.decoder.clear();
            return Err(error);
        }
        let response = self.wait_for_response(transport).await;
        if response.is_err() {
            self.decoder.clear();
        }
        match response? {
            DecodedResponse::Notification(value) => Ok(value),
            DecodedResponse::App(_) => bail!("unexpected app response"),
        }
    }

    async fn fetch_app_name<T: BluetoothTransport>(
        &mut self,
        transport: &mut T,
        app_identifier: &str,
    ) -> Result<String> {
        let request = app_attributes_request(app_identifier)?;
        self.decoder.expect(ResponseExpectation::App {
            app_identifier: app_identifier.to_owned(),
        });
        if let Err(error) = transport
            .write_control(&request.bytes, ControlWrite::Request)
            .await
        {
            self.decoder.clear();
            return Err(error);
        }
        let response = self.wait_for_response(transport).await;
        if response.is_err() {
            self.decoder.clear();
        }
        match response? {
            DecodedResponse::App(AppAttributes { display_name, .. }) => Ok(display_name),
            DecodedResponse::Notification(_) => bail!("unexpected notification response"),
        }
    }

    async fn wait_for_response<T: BluetoothTransport>(
        &mut self,
        transport: &mut T,
    ) -> Result<DecodedResponse> {
        let clock = self.clock.clone();
        let sleep = clock.sleep(REQUEST_TIMEOUT);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                () = &mut sleep => {
                    self.decoder.clear();
                    self.metadata.timed_out_request_count += 1;
                    bail!("ANCS attribute request timed out");
                }
                packet = transport.next_packet() => match packet? {
                    TransportPacket::DataSource(fragment) => {
                        if let Some(response) = self.decoder.push(&fragment)?.into_iter().next() {
                            return Ok(response);
                        }
                    }
                    TransportPacket::NotificationSource(bytes) => {
                        match NotificationEvent::parse(&bytes) {
                            Ok(event) => self.ingest(event).await,
                            Err(_) => {
                                self.metadata.recoverable_error_count += 1;
                                self.metadata.last_error_code = Some("malformed-notification-source");
                            }
                        }
                    }
                }
            }
        }
    }

    pub async fn end(&mut self) {
        for (_, handle) in self.handles.drain() {
            if self.sink.close(handle).await.is_err() {
                self.metadata.recoverable_error_count += 1;
                self.metadata.last_error_code = Some("desktop-close-failed");
            }
        }
        self.pending_order.clear();
        self.pending_kind.clear();
        self.outstanding = None;
        self.canceled.clear();
        self.app_names.clear();
        self.decoder.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ancs::codec::{Category, EventFlags},
        bluetooth::transport::{FakeBluetoothTransport, TransportPacket},
        clock::FakeClock,
        notification::{
            DesktopHandle, FakeNotificationSink, NotificationPayload, NotificationSink, SinkCall,
        },
        status::{RuntimeState, StatusSnapshot},
    };
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    fn event(kind: EventKind, uid: u32) -> NotificationEvent {
        NotificationEvent::parse(&[
            match kind {
                EventKind::Added => 0,
                EventKind::Modified => 1,
                EventKind::Removed => 2,
            },
            0,
            Category::Other as u8,
            1,
            uid as u8,
            (uid >> 8) as u8,
            (uid >> 16) as u8,
            (uid >> 24) as u8,
        ])
        .unwrap()
    }

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

    #[tokio::test]
    async fn serializes_requests_caches_apps_and_replaces_then_closes() {
        let transport_probe = FakeBluetoothTransport::default();
        let mut transport = transport_probe.clone();
        let sink_probe = FakeNotificationSink::default();
        let mut engine = SessionEngine::new(sink_probe.clone(), FakeClock::default());

        engine.ingest(event(EventKind::Added, 1)).await;
        transport.packet(TransportPacket::DataSource(notification_response(
            1,
            "com.example",
        )));
        transport.packet(TransportPacket::DataSource(app_response(
            "com.example",
            "Example",
        )));
        assert!(engine.process_next(&mut transport).await.unwrap());
        assert_eq!(transport_probe.writes().len(), 2);
        assert_eq!(engine.cached_app_count(), 1);

        engine.ingest(event(EventKind::Modified, 1)).await;
        transport.packet(TransportPacket::DataSource(notification_response(
            1,
            "com.example",
        )));
        engine.process_next(&mut transport).await.unwrap();
        assert_eq!(transport_probe.writes().len(), 3);
        assert!(matches!(
            sink_probe.calls().as_slice(),
            [SinkCall::Create(_), SinkCall::Replace(_)]
        ));

        engine.ingest(event(EventKind::Removed, 1)).await;
        assert!(matches!(
            sink_probe.calls().last(),
            Some(SinkCall::Close(_))
        ));
    }

    #[tokio::test]
    async fn coalesces_bounds_skips_preexisting_and_cancels() {
        let sink = FakeNotificationSink::default();
        let mut engine = SessionEngine::new(sink, FakeClock::default());
        let preexisting =
            NotificationEvent::parse(&[0, EventFlags::PRE_EXISTING, 0, 1, 1, 0, 0, 0]).unwrap();
        engine.ingest(preexisting).await;
        assert_eq!(engine.pending_len(), 0);

        for uid in 0..MAX_PENDING_UIDS as u32 {
            engine.ingest(event(EventKind::Added, uid)).await;
            engine.ingest(event(EventKind::Modified, uid)).await;
        }
        assert_eq!(engine.pending_len(), MAX_PENDING_UIDS);
        engine.ingest(event(EventKind::Added, 999)).await;
        assert_eq!(engine.pending_len(), MAX_PENDING_UIDS);
        engine.ingest(event(EventKind::Removed, 50)).await;
        assert_eq!(engine.pending_len(), MAX_PENDING_UIDS - 1);
    }

    #[tokio::test]
    async fn delivery_failure_is_recoverable_and_end_clears_session_state() {
        let mut transport = FakeBluetoothTransport::default();
        let sink = FakeNotificationSink::default();
        sink.fail_next();
        let mut engine = SessionEngine::new(sink, FakeClock::default());
        engine.ingest(event(EventKind::Added, 1)).await;
        transport.packet(TransportPacket::DataSource(notification_response(
            1,
            "secret.bundle",
        )));
        transport.packet(TransportPacket::DataSource(app_response(
            "secret.bundle",
            "Secret App",
        )));
        engine.process_next(&mut transport).await.unwrap();
        assert_eq!(engine.metadata().recoverable_error_count, 1);
        assert_eq!(engine.cached_app_count(), 1);
        engine.end().await;
        assert_eq!(engine.cached_app_count(), 0);
        assert_eq!(engine.pending_len(), 0);

        let json = serde_json::to_string(&StatusSnapshot::new(RuntimeState::Ready)).unwrap();
        for canary in ["secret.bundle", "Secret App", "Title", "Message"] {
            assert!(!json.contains(canary));
        }
    }

    #[tokio::test]
    async fn timeout_clears_outstanding_request_and_later_work_recovers() {
        let probe = FakeBluetoothTransport::default();
        probe.wait_when_empty();
        let mut transport = probe.clone();
        let clock = FakeClock::default();
        let mut engine = SessionEngine::new(FakeNotificationSink::default(), clock.clone());
        engine.ingest(event(EventKind::Added, 1)).await;
        let work = engine.process_next(&mut transport);
        let advance = async {
            tokio::task::yield_now().await;
            clock.advance(REQUEST_TIMEOUT);
        };
        let (result, ()) = tokio::join!(work, advance);
        assert!(result.unwrap());
        assert_eq!(engine.metadata().timed_out_request_count, 1);

        engine.ingest(event(EventKind::Added, 2)).await;
        probe.packet(TransportPacket::DataSource(notification_response(
            2,
            "com.example",
        )));
        probe.packet(TransportPacket::DataSource(app_response(
            "com.example",
            "Example",
        )));
        assert!(engine.process_next(&mut transport).await.unwrap());
        assert_eq!(engine.metadata().delivered_count, 1);
    }

    #[tokio::test]
    async fn removed_while_outstanding_cancels_without_stale_app_request() {
        let probe = FakeBluetoothTransport::default();
        let mut transport = probe.clone();
        let sink = FakeNotificationSink::default();
        let mut engine = SessionEngine::new(sink.clone(), FakeClock::default());
        engine.ingest(event(EventKind::Added, 7)).await;
        let removed = [2, 0, 0, 1, 7, 0, 0, 0];
        probe.packet(TransportPacket::NotificationSource(removed.to_vec()));
        probe.packet(TransportPacket::DataSource(notification_response(
            7,
            "com.example",
        )));
        engine.process_next(&mut transport).await.unwrap();
        assert!(sink.calls().is_empty());
        assert_eq!(probe.writes().len(), 1);
    }

    #[derive(Clone, Default)]
    struct CapturingSink(Arc<Mutex<Vec<String>>>);

    #[async_trait]
    impl NotificationSink for CapturingSink {
        async fn create(&mut self, payload: NotificationPayload) -> Result<DesktopHandle> {
            self.0
                .lock()
                .unwrap()
                .push(payload.test_app_name().to_owned());
            Ok(DesktopHandle(1))
        }
        async fn replace(&mut self, _: DesktopHandle, _: NotificationPayload) -> Result<()> {
            Ok(())
        }
        async fn close(&mut self, _: DesktopHandle) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn app_lookup_failure_uses_bundle_identifier_and_does_not_poison_decoder() {
        let probe = FakeBluetoothTransport::default();
        let mut transport = probe.clone();
        let sink = CapturingSink::default();
        let captured = sink.0.clone();
        let mut engine = SessionEngine::new(sink, FakeClock::default());
        engine.ingest(event(EventKind::Added, 1)).await;
        probe.packet(TransportPacket::DataSource(notification_response(
            1,
            "fallback.bundle",
        )));
        probe.packet(TransportPacket::DataSource(vec![9, 0, 0]));
        engine.process_next(&mut transport).await.unwrap();
        assert_eq!(&*captured.lock().unwrap(), &["fallback.bundle"]);

        engine.ingest(event(EventKind::Added, 2)).await;
        probe.packet(TransportPacket::DataSource(notification_response(
            2,
            "com.example",
        )));
        probe.packet(TransportPacket::DataSource(app_response(
            "com.example",
            "Example",
        )));
        engine.process_next(&mut transport).await.unwrap();
        assert_eq!(&*captured.lock().unwrap(), &["fallback.bundle", "Example"]);
    }
}
