use crate::bluetooth::hid;
use anyhow::{Context, Result};
use async_trait::async_trait;
use bluer::{
    adv::AdvertisementHandle,
    gatt::{
        local::ApplicationHandle,
        remote::{Characteristic, CharacteristicWriteRequest},
        WriteOp,
    },
    Adapter, Address, Device, Session, Uuid,
};
use futures::{stream::BoxStream, StreamExt};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

pub const NOTIFICATION_SOURCE_UUID: Uuid = Uuid::from_u128(0x9fbf120d_6301_42d9_8c58_25e699a21dbd);
pub const DATA_SOURCE_UUID: Uuid = Uuid::from_u128(0x22eac6e9_24d6_4bb5_be44_b36ace7c7bfb);
pub const CONTROL_POINT_UUID: Uuid = Uuid::from_u128(0x69d1d8f3_45e1_49a8_9821_9bbdfdaad9d9);

fn advertisement_count_contains_registration(active: u8, baseline: u8) -> bool {
    active > baseline
}

/// Resolve a canonical bonded identity through Device1.Address instead of
/// assuming BlueZ renamed the object path after LE privacy resolution.
pub async fn device_by_identity(adapter: &Adapter, identity: Address) -> Result<Option<Device>> {
    for path_address in adapter.device_addresses().await? {
        let device = adapter.device(path_address)?;
        if path_address == identity
            || device.remote_address().await.unwrap_or(path_address) == identity
        {
            return Ok(Some(device));
        }
    }
    Ok(None)
}

pub async fn has_complete_ancs(device: &bluer::Device) -> Result<bool> {
    for service in device.services().await? {
        if service.uuid().await? != hid::ANCS_SERVICE_UUID {
            continue;
        }
        let mut found = [false; 3];
        for characteristic in service.characteristics().await? {
            match characteristic.uuid().await? {
                NOTIFICATION_SOURCE_UUID => found[0] = true,
                DATA_SOURCE_UUID => found[1] = true,
                CONTROL_POINT_UUID => found[2] = true,
                _ => {}
            }
        }
        return Ok(found.into_iter().all(|value| value));
    }
    Ok(false)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportObservation {
    AdapterPoweredOff,
    Advertising,
    DeviceNotBonded,
    WaitingForPhone,
    WaitingForServices,
    WaitingForAuthorization,
    Available,
    Ready,
}

pub enum TransportPacket {
    NotificationSource(Vec<u8>),
    DataSource(Vec<u8>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlWrite {
    Request,
}

#[async_trait]
pub trait BluetoothTransport: Send {
    async fn reconcile(&mut self) -> Result<TransportObservation>;
    async fn subscribe(&mut self) -> Result<()>;
    async fn next_packet(&mut self) -> Result<TransportPacket>;
    async fn write_control(&mut self, value: &[u8], operation: ControlWrite) -> Result<()>;
    fn end_ancs_session(&mut self);
    fn reset_bluez_session(&mut self);

    async fn shutdown(&mut self) {
        self.end_ancs_session();
    }
}

struct RegistrationOwner<A, B> {
    _application: A,
    _advertisement: B,
}

type Registrations = RegistrationOwner<ApplicationHandle, AdvertisementHandle>;

struct Characteristics {
    notification_source: Characteristic,
    data_source: Characteristic,
    control_point: Characteristic,
}

struct Subscriptions {
    notification_source: BoxStream<'static, Vec<u8>>,
    data_source: BoxStream<'static, Vec<u8>>,
}

/// System-bus BlueZ transport. Registration handles are owned for their full
/// valid lifetime; dropping this value unregisters both objects through bluer RAII.
pub struct BluerTransport {
    adapter_name: String,
    device_address: Address,
    session: Option<Session>,
    adapter: Option<Adapter>,
    registrations: Option<Registrations>,
    advertising_baseline: Option<u8>,
    characteristics: Option<Characteristics>,
    subscriptions: Option<Subscriptions>,
}

impl BluerTransport {
    pub fn new(adapter_name: impl Into<String>, device_address: Address) -> Self {
        Self {
            adapter_name: adapter_name.into(),
            device_address,
            session: None,
            adapter: None,
            registrations: None,
            advertising_baseline: None,
            characteristics: None,
            subscriptions: None,
        }
    }

    async fn ensure_bluez(&mut self) -> Result<()> {
        if self.session.is_none() {
            let session = Session::new()
                .await
                .context("connecting to BlueZ system bus")?;
            self.session = Some(session);
        }
        if self.adapter.is_none() {
            let adapter = self
                .session
                .as_ref()
                .context("BlueZ session unavailable")?
                .adapter(&self.adapter_name)
                .context("locating configured Bluetooth adapter")?;
            self.adapter = Some(adapter);
        }
        Ok(())
    }

    fn invalidate_bluez_objects(&mut self) {
        self.end_ancs_session();
        self.registrations = None;
        self.advertising_baseline = None;
        self.adapter = None;
    }

    async fn registrations_are_live(&mut self) -> Result<bool> {
        let Some(baseline) = self.advertising_baseline else {
            return Ok(self.registrations.is_none());
        };
        let adapter = self.adapter.as_ref().context("adapter unavailable")?;
        Ok(advertisement_count_contains_registration(
            adapter.active_advertising_instances().await?,
            baseline,
        ))
    }

    async fn ensure_registrations(&mut self) -> Result<bool> {
        if self.registrations.is_some() {
            return Ok(false);
        }
        let adapter = self.adapter.as_ref().context("adapter unavailable")?;
        let advertising_baseline = adapter.active_advertising_instances().await?;
        let application = adapter
            .serve_gatt_application(hid::application())
            .await
            .context("registering HID GATT application")?;
        let advertisement = match adapter.advertise(hid::runtime_advertisement()).await {
            Ok(value) => value,
            Err(error) => {
                drop(application);
                return Err(error).context("registering runtime advertisement");
            }
        };
        self.registrations = Some(RegistrationOwner {
            _application: application,
            _advertisement: advertisement,
        });
        self.advertising_baseline = Some(advertising_baseline);
        Ok(true)
    }

    async fn discover_characteristics(&mut self) -> Result<bool> {
        let adapter = self.adapter.as_ref().context("adapter unavailable")?;
        let device = device_by_identity(adapter, self.device_address)
            .await?
            .context("configured Bluetooth identity is not present in BlueZ")?;
        let mut notification_source = None;
        let mut data_source = None;
        let mut control_point = None;
        for service in device.services().await? {
            if service.uuid().await? != hid::ANCS_SERVICE_UUID {
                continue;
            }
            for characteristic in service.characteristics().await? {
                match characteristic.uuid().await? {
                    NOTIFICATION_SOURCE_UUID => notification_source = Some(characteristic),
                    DATA_SOURCE_UUID => data_source = Some(characteristic),
                    CONTROL_POINT_UUID => control_point = Some(characteristic),
                    _ => {}
                }
            }
        }
        if let (Some(notification_source), Some(data_source), Some(control_point)) =
            (notification_source, data_source, control_point)
        {
            self.characteristics = Some(Characteristics {
                notification_source,
                data_source,
                control_point,
            });
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[async_trait]
impl BluetoothTransport for BluerTransport {
    async fn reconcile(&mut self) -> Result<TransportObservation> {
        self.ensure_bluez().await?;
        let adapter = self.adapter.as_ref().context("adapter unavailable")?;
        if !adapter.is_powered().await? {
            return Ok(TransportObservation::AdapterPoweredOff);
        }
        if self.registrations.is_some() && !self.registrations_are_live().await? {
            tracing::info!(
                event_code = "bluez-registrations-lost",
                "recreating registrations missing from BlueZ"
            );
            self.invalidate_bluez_objects();
            self.ensure_bluez().await?;
        }
        if self.ensure_registrations().await? {
            return Ok(TransportObservation::Advertising);
        }
        let adapter = self.adapter.as_ref().context("adapter unavailable")?;
        let device = device_by_identity(adapter, self.device_address)
            .await?
            .context("configured Bluetooth identity is not present in BlueZ")?;
        if !device.is_paired().await? {
            self.end_ancs_session();
            return Ok(TransportObservation::DeviceNotBonded);
        }
        if !device.is_connected().await? {
            self.end_ancs_session();
            return Ok(TransportObservation::WaitingForPhone);
        }
        if !device.is_services_resolved().await? {
            self.end_ancs_session();
            return Ok(TransportObservation::WaitingForServices);
        }
        if !self.discover_characteristics().await? {
            self.end_ancs_session();
            return Ok(TransportObservation::WaitingForAuthorization);
        }
        if self.subscriptions.is_some() {
            Ok(TransportObservation::Ready)
        } else {
            Ok(TransportObservation::Available)
        }
    }

    async fn subscribe(&mut self) -> Result<()> {
        let characteristics = self.characteristics.as_ref().context("ANCS unavailable")?;
        // The order is protocol-significant: Data Source must be active first.
        let data_source = characteristics
            .data_source
            .notify()
            .await
            .context("subscribing Data Source")?
            .boxed();
        let notification_source = characteristics
            .notification_source
            .notify()
            .await
            .context("subscribing Notification Source")?
            .boxed();
        self.subscriptions = Some(Subscriptions {
            notification_source,
            data_source,
        });
        Ok(())
    }

    async fn next_packet(&mut self) -> Result<TransportPacket> {
        let subscriptions = self.subscriptions.as_mut().context("ANCS not subscribed")?;
        tokio::select! {
            value = subscriptions.data_source.next() => value
                .map(TransportPacket::DataSource)
                .context("Data Source subscription ended"),
            value = subscriptions.notification_source.next() => value
                .map(TransportPacket::NotificationSource)
                .context("Notification Source subscription ended"),
        }
    }

    async fn write_control(&mut self, value: &[u8], operation: ControlWrite) -> Result<()> {
        let characteristics = self.characteristics.as_ref().context("ANCS unavailable")?;
        let op_type = match operation {
            ControlWrite::Request => WriteOp::Request,
        };
        characteristics
            .control_point
            .write_ext(
                value,
                &CharacteristicWriteRequest {
                    op_type,
                    ..Default::default()
                },
            )
            .await
            .context("writing ANCS Control Point with response")
    }

    fn end_ancs_session(&mut self) {
        self.subscriptions = None;
        self.characteristics = None;
    }

    fn reset_bluez_session(&mut self) {
        self.end_ancs_session();
        self.registrations = None;
        self.advertising_baseline = None;
        self.adapter = None;
        self.session = None;
    }

    async fn shutdown(&mut self) {
        // Dropping bluer notification streams schedules StopNotify on its
        // session task. Keep the session and characteristic handles alive
        // until BlueZ confirms both CCCDs are disabled so an immediate
        // systemd restart cannot inherit a stale Notifying=true session.
        self.subscriptions = None;
        let mut stopped = false;
        for _ in 0..40 {
            let Some(characteristics) = self.characteristics.as_ref() else {
                stopped = true;
                break;
            };
            let notification_source = characteristics.notification_source.notifying().await;
            let data_source = characteristics.data_source.notifying().await;
            if notification_source.as_ref().ok().copied().flatten() != Some(true)
                && data_source.as_ref().ok().copied().flatten() != Some(true)
            {
                stopped = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        if !stopped {
            tracing::warn!(
                error_code = "notify-shutdown-timeout",
                "timed out waiting for ANCS notifications to stop"
            );
        }
        self.characteristics = None;
        self.registrations = None;
        self.advertising_baseline = None;
        tokio::task::yield_now().await;
    }
}

#[derive(Clone)]
pub struct FakeBluetoothTransport {
    inner: Arc<Mutex<FakeTransportState>>,
    packet_ready: Arc<Notify>,
}

impl Default for FakeBluetoothTransport {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeTransportState::default())),
            packet_ready: Arc::new(Notify::new()),
        }
    }
}

#[derive(Default)]
struct FakeTransportState {
    observations: VecDeque<Result<TransportObservation, String>>,
    packets: VecDeque<Result<TransportPacket, String>>,
    writes: Vec<Vec<u8>>,
    calls: Vec<&'static str>,
    fail_subscribe: bool,
    wait_when_empty: bool,
}

impl FakeBluetoothTransport {
    pub fn observation(&self, value: TransportObservation) {
        self.inner
            .lock()
            .expect("fake transport poisoned")
            .observations
            .push_back(Ok(value));
    }

    pub fn observation_error(&self, message: impl Into<String>) {
        self.inner
            .lock()
            .expect("fake transport poisoned")
            .observations
            .push_back(Err(message.into()));
    }

    pub fn packet(&self, value: TransportPacket) {
        self.inner
            .lock()
            .expect("fake transport poisoned")
            .packets
            .push_back(Ok(value));
        self.packet_ready.notify_waiters();
    }

    pub fn packet_error(&self, message: impl Into<String>) {
        self.inner
            .lock()
            .expect("fake transport poisoned")
            .packets
            .push_back(Err(message.into()));
        self.packet_ready.notify_waiters();
    }

    pub fn wait_when_empty(&self) {
        self.inner
            .lock()
            .expect("fake transport poisoned")
            .wait_when_empty = true;
    }

    pub fn writes(&self) -> Vec<Vec<u8>> {
        self.inner
            .lock()
            .expect("fake transport poisoned")
            .writes
            .clone()
    }

    pub fn calls(&self) -> Vec<&'static str> {
        self.inner
            .lock()
            .expect("fake transport poisoned")
            .calls
            .clone()
    }
}

#[async_trait]
impl BluetoothTransport for FakeBluetoothTransport {
    async fn reconcile(&mut self) -> Result<TransportObservation> {
        let mut inner = self.inner.lock().expect("fake transport poisoned");
        inner.calls.push("reconcile");
        match inner
            .observations
            .pop_front()
            .unwrap_or(Ok(TransportObservation::Ready))
        {
            Ok(value) => Ok(value),
            Err(message) => anyhow::bail!(message),
        }
    }

    async fn subscribe(&mut self) -> Result<()> {
        let mut inner = self.inner.lock().expect("fake transport poisoned");
        inner.calls.push("subscribe-data-source");
        inner.calls.push("subscribe-notification-source");
        if inner.fail_subscribe {
            anyhow::bail!("injected subscribe failure");
        }
        Ok(())
    }

    async fn next_packet(&mut self) -> Result<TransportPacket> {
        loop {
            let (value, wait) = {
                let mut inner = self.inner.lock().expect("fake transport poisoned");
                (inner.packets.pop_front(), inner.wait_when_empty)
            };
            match value {
                Some(Ok(value)) => return Ok(value),
                Some(Err(message)) => anyhow::bail!(message),
                None if wait => self.packet_ready.notified().await,
                None => anyhow::bail!("no scripted packet"),
            }
        }
    }

    async fn write_control(&mut self, value: &[u8], operation: ControlWrite) -> Result<()> {
        assert_eq!(operation, ControlWrite::Request);
        self.inner
            .lock()
            .expect("fake transport poisoned")
            .writes
            .push(value.to_vec());
        Ok(())
    }

    fn end_ancs_session(&mut self) {
        self.inner
            .lock()
            .expect("fake transport poisoned")
            .calls
            .push("end-ancs-session");
    }

    fn reset_bluez_session(&mut self) {
        self.inner
            .lock()
            .expect("fake transport poisoned")
            .calls
            .push("reset-bluez-session");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropProbe(Arc<AtomicUsize>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn registration_owner_drops_both_raii_handles() {
        let drops = Arc::new(AtomicUsize::new(0));
        let owner = RegistrationOwner {
            _application: DropProbe(Arc::clone(&drops)),
            _advertisement: DropProbe(Arc::clone(&drops)),
        };
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(owner);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn advertisement_count_detects_lost_registration() {
        assert!(advertisement_count_contains_registration(1, 0));
        assert!(advertisement_count_contains_registration(3, 2));
        assert!(!advertisement_count_contains_registration(0, 0));
        assert!(!advertisement_count_contains_registration(2, 2));
        assert!(!advertisement_count_contains_registration(1, 2));
    }
}
