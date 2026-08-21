//! BlueZ-backed setup transaction. The implementation lives behind the
//! `SetupBackend` boundary so the protocol and all state transitions can be
//! exercised without a system bus or Bluetooth hardware.

use super::{Candidate, Preparation, SetupBackend, SetupOptions};
use crate::{
    audio::{reconcile_with_reload, rollback_change, AudioRule, WIREPLUMBER_UNIT},
    bluetooth::{hid, transport},
    config::{
        BluetoothConfiguration, Configuration, ConfigurationStore, DesktopConfiguration,
        ValidatedConfiguration, CONFIG_SCHEMA_VERSION,
    },
    diagnostics::{select_adapter, AdapterSelection},
    machine::{ConfirmationKind, SetupFailure},
    service::UserServiceControl,
};
use async_trait::async_trait;
use bluer::{
    adv::AdvertisementHandle,
    agent::{Agent, AgentHandle, ReqError as AgentReqError},
    gatt::{local::ApplicationHandle, remote::Characteristic},
    Adapter, Address, Session,
};
use futures::{stream::BoxStream, FutureExt, StreamExt};
use std::{
    collections::HashSet,
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

const PAIRING_SETTLE_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

struct AdapterSnapshot {
    pairable: bool,
    discoverable: bool,
}

struct SetupRegistrations {
    _application: ApplicationHandle,
    _advertisement: AdvertisementHandle,
    _agent: Option<AgentHandle>,
}

struct AncsSubscriptions {
    _data_source: BoxStream<'static, Vec<u8>>,
    _notification_source: BoxStream<'static, Vec<u8>>,
}

struct AgentNotice {
    address: Address,
    passkey: Option<u32>,
}

#[derive(Default)]
struct AgentGateState {
    active: Option<Address>,
    approved: Option<Address>,
    waiters: Vec<oneshot::Sender<bool>>,
}

struct AgentGate {
    state: Mutex<AgentGateState>,
    notices: mpsc::UnboundedSender<AgentNotice>,
}

impl AgentGate {
    async fn request(&self, address: Address, passkey: Option<u32>) -> Result<(), AgentReqError> {
        let receiver = {
            let mut state = self.state.lock().map_err(|_| AgentReqError::Canceled)?;
            if state.approved == Some(address) {
                return Ok(());
            }
            if state.approved.is_some() || state.active.is_some_and(|active| active != address) {
                return Err(AgentReqError::Rejected);
            }
            let first = state.active.is_none();
            state.active = Some(address);
            let (sender, receiver) = oneshot::channel();
            state.waiters.push(sender);
            if first && self.notices.send(AgentNotice { address, passkey }).is_err() {
                return Err(AgentReqError::Canceled);
            }
            receiver
        };
        match receiver.await {
            Ok(true) => Ok(()),
            Ok(false) => Err(AgentReqError::Rejected),
            Err(_) => Err(AgentReqError::Canceled),
        }
    }

    fn answer(&self, address: Address, accepted: bool) -> Result<(), SetupFailure> {
        let mut state = self.state.lock().map_err(|_| SetupFailure::BackendFailed)?;
        if state.active != Some(address) {
            return Err(SetupFailure::PairingFailed);
        }
        if accepted {
            state.approved = Some(address);
        } else {
            state.active = None;
        }
        for waiter in state.waiters.drain(..) {
            let _ = waiter.send(accepted);
        }
        Ok(())
    }
}

pub struct BluerSetupBackend {
    config_store: ConfigurationStore,
    configured: Option<ValidatedConfiguration>,
    services: Arc<dyn UserServiceControl>,
    session: Option<Session>,
    adapter: Option<Adapter>,
    snapshot: Option<AdapterSnapshot>,
    registrations: Option<SetupRegistrations>,
    subscriptions: Option<AncsSubscriptions>,
    initial_bonds: HashSet<Address>,
    gate: Option<Arc<AgentGate>>,
    notices: Option<mpsc::UnboundedReceiver<AgentNotice>>,
    resolved_identity: Option<(Address, String)>,
    legacy_adapter_migration: bool,
}

impl BluerSetupBackend {
    pub fn new(
        config_store: ConfigurationStore,
        configured: Option<ValidatedConfiguration>,
        services: Arc<dyn UserServiceControl>,
    ) -> Self {
        Self {
            config_store,
            configured,
            services,
            session: None,
            adapter: None,
            snapshot: None,
            registrations: None,
            subscriptions: None,
            initial_bonds: HashSet::new(),
            gate: None,
            notices: None,
            resolved_identity: None,
            legacy_adapter_migration: false,
        }
    }

    async fn environment(&mut self) -> Result<(), SetupFailure> {
        let session = Session::new()
            .await
            .map_err(|_| SetupFailure::EnvironmentUnavailable)?;
        let adapters = session
            .adapter_names()
            .await
            .map_err(|_| SetupFailure::EnvironmentUnavailable)?;
        let adapter = if let Some(configuration) = self.configured.as_ref() {
            if let Some(identity) = configuration.adapter_address {
                transport::adapter_by_identity(&session, identity)
                    .await
                    .map_err(|_| SetupFailure::AdapterUnavailable)?
                    .ok_or(SetupFailure::AdapterUnavailable)?
            } else if adapters.contains(&configuration.adapter) {
                session
                    .adapter(&configuration.adapter)
                    .map_err(|_| SetupFailure::AdapterUnavailable)?
            } else {
                let adapter = transport::unique_adapter_with_paired_device(
                    &session,
                    configuration.device_address,
                )
                .await
                .map_err(|_| SetupFailure::AdapterUnavailable)?
                .ok_or(SetupFailure::AdapterUnavailable)?;
                self.legacy_adapter_migration = true;
                adapter
            }
        } else {
            let name = match select_adapter(None, &adapters) {
                AdapterSelection::Selected(name) => name,
                _ => return Err(SetupFailure::AdapterUnavailable),
            };
            session
                .adapter(&name)
                .map_err(|_| SetupFailure::AdapterUnavailable)?
        };
        if !adapter
            .is_powered()
            .await
            .map_err(|_| SetupFailure::AdapterUnavailable)?
        {
            return Err(SetupFailure::AdapterPoweredOff);
        }
        if adapter
            .supported_advertising_instances()
            .await
            .map_err(|_| SetupFailure::AdapterCapabilityMissing)?
            == 0
        {
            return Err(SetupFailure::AdapterCapabilityMissing);
        }
        let address = adapter
            .address()
            .await
            .map_err(|_| SetupFailure::AdapterCapabilityMissing)?;
        let roles = Command::new("bluetoothctl")
            .args(["show", &address.to_string()])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).to_ascii_lowercase())
            .ok_or(SetupFailure::AdapterCapabilityMissing)?;
        if !roles
            .lines()
            .any(|line| line.contains("role") && line.contains("central"))
            || !roles
                .lines()
                .any(|line| line.contains("role") && line.contains("peripheral"))
        {
            return Err(SetupFailure::AdapterCapabilityMissing);
        }
        self.session = Some(session);
        self.adapter = Some(adapter);
        Ok(())
    }

    async fn start_fresh_window(&mut self) -> Result<(), SetupFailure> {
        let session = self
            .session
            .as_ref()
            .ok_or(SetupFailure::EnvironmentUnavailable)?;
        let adapter = self
            .adapter
            .as_ref()
            .ok_or(SetupFailure::AdapterUnavailable)?;
        self.initial_bonds = paired_addresses(adapter).await?;
        self.snapshot = Some(AdapterSnapshot {
            pairable: adapter
                .is_pairable()
                .await
                .map_err(|_| SetupFailure::BackendFailed)?,
            discoverable: adapter
                .is_discoverable()
                .await
                .map_err(|_| SetupFailure::BackendFailed)?,
        });
        let application = adapter
            .serve_gatt_application(hid::application())
            .await
            .map_err(|_| SetupFailure::BackendFailed)?;
        let advertisement = match adapter.advertise(hid::setup_advertisement()).await {
            Ok(handle) => handle,
            Err(_) => {
                drop(application);
                return Err(SetupFailure::BackendFailed);
            }
        };
        let (notice_tx, notice_rx) = mpsc::unbounded_channel();
        let gate = Arc::new(AgentGate {
            state: Mutex::new(AgentGateState::default()),
            notices: notice_tx,
        });
        let agent = match session
            .register_agent(pairing_agent(Arc::clone(&gate)))
            .await
        {
            Ok(handle) => handle,
            Err(_) => {
                drop(advertisement);
                drop(application);
                return Err(SetupFailure::BackendFailed);
            }
        };
        self.gate = Some(gate);
        self.notices = Some(notice_rx);
        self.registrations = Some(SetupRegistrations {
            _application: application,
            _advertisement: advertisement,
            _agent: Some(agent),
        });
        adapter
            .set_pairable(true)
            .await
            .map_err(|_| SetupFailure::BackendFailed)?;
        if let Err(error) = adapter.set_discoverable(true).await {
            let _ = adapter
                .set_pairable(self.snapshot.as_ref().unwrap().pairable)
                .await;
            return Err(error).map_err(|_| SetupFailure::BackendFailed);
        }
        Ok(())
    }

    async fn start_existing_reconnect_window(&mut self) -> Result<(), SetupFailure> {
        let adapter = self
            .adapter
            .as_ref()
            .ok_or(SetupFailure::AdapterUnavailable)?;
        let application = adapter
            .serve_gatt_application(hid::application())
            .await
            .map_err(|_| SetupFailure::BackendFailed)?;
        let advertisement = match adapter.advertise(hid::runtime_advertisement()).await {
            Ok(handle) => handle,
            Err(_) => {
                drop(application);
                return Err(SetupFailure::BackendFailed);
            }
        };
        self.registrations = Some(SetupRegistrations {
            _application: application,
            _advertisement: advertisement,
            _agent: None,
        });
        Ok(())
    }

    async fn configured_or_unique_existing(&mut self) -> Result<Option<Candidate>, SetupFailure> {
        let adapter = self
            .adapter
            .as_ref()
            .ok_or(SetupFailure::AdapterUnavailable)?;
        if let Some(configuration) = &self.configured {
            return existing_candidate(adapter, configuration.device_address).await;
        }
        let mut candidates = Vec::new();
        for address in paired_addresses(adapter).await? {
            if let Some(candidate) = existing_candidate(adapter, address).await? {
                candidates.push(candidate);
            }
        }
        Ok(match candidates.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        })
    }

    async fn restore_adapter(&mut self) -> Result<(), SetupFailure> {
        let Some(snapshot) = self.snapshot.take() else {
            return Ok(());
        };
        let adapter = self.adapter.as_ref().ok_or(SetupFailure::CleanupFailed)?;
        let pairable = adapter.set_pairable(snapshot.pairable).await;
        let discoverable = adapter.set_discoverable(snapshot.discoverable).await;
        pairable.map_err(|_| SetupFailure::CleanupFailed)?;
        discoverable.map_err(|_| SetupFailure::CleanupFailed)?;
        Ok(())
    }
}

#[async_trait]
impl SetupBackend for BluerSetupBackend {
    async fn prepare(&mut self, options: SetupOptions) -> Result<Preparation, SetupFailure> {
        self.environment().await?;
        if let Some(repair_target) = requested_repair_target(options, self.configured.as_ref())? {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(SetupFailure::AdapterUnavailable)?;
            if let Some((_, repair_device)) = identity_device(adapter, repair_target).await? {
                adapter
                    .remove_device(repair_device.address())
                    .await
                    .map_err(|_| SetupFailure::PairingFailed)?;
            }
            self.start_fresh_window().await?;
            return Ok(Preparation::Fresh);
        }
        if self.legacy_adapter_migration {
            let configuration = self
                .configured
                .as_ref()
                .ok_or(SetupFailure::AdapterUnavailable)?;
            let candidate = paired_candidate(
                self.adapter
                    .as_ref()
                    .ok_or(SetupFailure::AdapterUnavailable)?,
                configuration.device_address,
            )
            .await?
            .ok_or(SetupFailure::RepairRequired)?;
            self.start_existing_reconnect_window().await?;
            return Ok(Preparation::Existing(candidate));
        }
        if let Some(candidate) = self.configured_or_unique_existing().await? {
            return Ok(Preparation::Existing(candidate));
        }
        if self.configured.is_some() {
            return Err(SetupFailure::RepairRequired);
        }
        self.start_fresh_window().await?;
        Ok(Preparation::Fresh)
    }

    async fn wait_for_candidate(&mut self) -> Result<Candidate, SetupFailure> {
        loop {
            let notice = self
                .notices
                .as_mut()
                .ok_or(SetupFailure::BackendFailed)?
                .recv()
                .await
                .ok_or(SetupFailure::BackendFailed)?;
            if self.initial_bonds.contains(&notice.address) {
                if let Some(gate) = &self.gate {
                    let _ = gate.answer(notice.address, false);
                }
                continue;
            }
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(SetupFailure::AdapterUnavailable)?;
            let device = adapter
                .device(notice.address)
                .map_err(|_| SetupFailure::PairingFailed)?;
            return Ok(Candidate {
                kind: ConfirmationKind::Pairing,
                address: notice.address.to_string(),
                device_name: device
                    .name()
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "iPhone".into()),
                passkey: notice.passkey.map(|passkey| format!("{passkey:06}")),
            });
        }
    }

    async fn answer_confirmation(
        &mut self,
        candidate: &Candidate,
        accept: bool,
    ) -> Result<(), SetupFailure> {
        if candidate.kind == ConfirmationKind::ExistingBond {
            return Ok(());
        }
        let address: Address = candidate
            .address
            .parse()
            .map_err(|_| SetupFailure::PairingFailed)?;
        self.gate
            .as_ref()
            .ok_or(SetupFailure::PairingFailed)?
            .answer(address, accept)?;
        if !accept {
            return Ok(());
        }
        let adapter = self
            .adapter
            .as_ref()
            .ok_or(SetupFailure::AdapterUnavailable)?;
        let (_, device) = identity_device(adapter, address)
            .await?
            .ok_or(SetupFailure::PairingFailed)?;
        let deadline = tokio::time::Instant::now() + PAIRING_SETTLE_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if device.is_paired().await.unwrap_or(false) {
                device
                    .set_trusted(true)
                    .await
                    .map_err(|_| SetupFailure::TrustFailed)?;
                return Ok(());
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        Err(SetupFailure::PairingFailed)
    }

    async fn verify_ancs(&mut self, candidate: &Candidate) -> Result<(), SetupFailure> {
        let address: Address = candidate
            .address
            .parse()
            .map_err(|_| SetupFailure::PairingFailed)?;
        let adapter = self
            .adapter
            .as_ref()
            .ok_or(SetupFailure::AdapterUnavailable)?;
        let (identity, device) = identity_device(adapter, address)
            .await?
            .ok_or(SetupFailure::PairingFailed)?;
        loop {
            if device.is_paired().await.unwrap_or(false)
                && device.is_trusted().await.unwrap_or(false)
                && device.is_connected().await.unwrap_or(false)
                && device.is_services_resolved().await.unwrap_or(false)
            {
                if let Some((data_source, notification_source)) =
                    find_ancs_sources(&device)
                        .await
                        .map_err(|_| SetupFailure::BackendFailed)?
                {
                    let data_source = data_source
                        .notify()
                        .await
                        .map_err(|_| SetupFailure::BackendFailed)?
                        .boxed();
                    let notification_source = notification_source
                        .notify()
                        .await
                        .map_err(|_| SetupFailure::BackendFailed)?
                        .boxed();
                    self.subscriptions = Some(AncsSubscriptions {
                        _data_source: data_source,
                        _notification_source: notification_source,
                    });
                    let name = device
                        .name()
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| candidate.device_name.clone());
                    self.resolved_identity = Some((identity, name));
                    return Ok(());
                }
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn cleanup_temporary(&mut self) -> Result<(), SetupFailure> {
        self.subscriptions = None;
        self.registrations = None;
        self.gate = None;
        self.notices = None;
        self.restore_adapter().await
    }

    async fn commit(
        &mut self,
        candidate: &Candidate,
        options: SetupOptions,
    ) -> Result<(), SetupFailure> {
        let candidate_address: Address = candidate
            .address
            .parse()
            .map_err(|_| SetupFailure::ConfigurationWriteFailed)?;
        let (address, device_name) = self
            .resolved_identity
            .clone()
            .unwrap_or((candidate_address, candidate.device_name.clone()));
        let previous_audio_rule = self
            .configured
            .as_ref()
            .filter(|configuration| configuration.suppress_phone_audio)
            .map(|configuration| AudioRule::from_environment(configuration.device_address))
            .transpose()
            .map_err(|_| SetupFailure::AudioUnavailable)?;
        let desired_audio_rule = if options.disable_phone_audio {
            if !self
                .services
                .unit_exists(WIREPLUMBER_UNIT)
                .map_err(|_| SetupFailure::AudioUnavailable)?
            {
                return Err(SetupFailure::AudioUnavailable);
            }
            Some(AudioRule::from_environment(address).map_err(|_| SetupFailure::AudioUnavailable)?)
        } else {
            None
        };
        let adapter_name = self
            .adapter
            .as_ref()
            .ok_or(SetupFailure::ConfigurationWriteFailed)?
            .name()
            .to_owned();
        let configuration = Configuration {
            schema_version: CONFIG_SCHEMA_VERSION,
            bluetooth: BluetoothConfiguration {
                adapter: adapter_name,
                adapter_address: Some(
                    self.adapter
                        .as_ref()
                        .ok_or(SetupFailure::ConfigurationWriteFailed)?
                        .address()
                        .await
                        .map_err(|_| SetupFailure::ConfigurationWriteFailed)?
                        .to_string(),
                ),
                device_address: address.to_string(),
                device_name,
            },
            desktop: DesktopConfiguration {
                suppress_phone_audio: options.disable_phone_audio,
            },
        };
        commit_persistent(
            &self.config_store,
            &configuration,
            previous_audio_rule.as_ref(),
            desired_audio_rule.as_ref(),
            self.services.as_ref(),
        )
    }

    fn final_address(&self, candidate: &Candidate) -> String {
        self.resolved_identity
            .as_ref()
            .map(|(address, _)| address.to_string())
            .unwrap_or_else(|| candidate.address.clone())
    }
}

fn requested_repair_target(
    options: SetupOptions,
    configured: Option<&ValidatedConfiguration>,
) -> Result<Option<Address>, SetupFailure> {
    if !options.repair {
        return Ok(None);
    }
    configured
        .map(|configuration| Some(configuration.device_address))
        .ok_or(SetupFailure::RepairTargetUnknown)
}

fn commit_persistent(
    store: &ConfigurationStore,
    configuration: &Configuration,
    previous_audio_rule: Option<&AudioRule>,
    desired_audio_rule: Option<&AudioRule>,
    services: &dyn UserServiceControl,
) -> Result<(), SetupFailure> {
    let mut audio_change = None;
    if previous_audio_rule.is_some() || desired_audio_rule.is_some() {
        match reconcile_with_reload(previous_audio_rule, desired_audio_rule, services) {
            Ok(change) if change.changed() => audio_change = Some(change),
            Ok(_) => {}
            Err(error) if error.to_string().contains("audio-rule-conflict") => {
                return Err(SetupFailure::AudioRuleConflict)
            }
            Err(error) if error.to_string().contains("cleanup-failed") => {
                return Err(SetupFailure::CleanupFailed)
            }
            Err(error) if error.to_string().contains("audio-restart-failed") => {
                return Err(SetupFailure::AudioRestartFailed)
            }
            Err(_) => return Err(SetupFailure::AudioUnavailable),
        }
    }
    if store.save(configuration).is_err() {
        if let Some(change) = audio_change.as_ref() {
            rollback_change(change, services).map_err(|_| SetupFailure::CleanupFailed)?;
        }
        return Err(SetupFailure::ConfigurationWriteFailed);
    }
    Ok(())
}

fn pairing_agent(gate: Arc<AgentGate>) -> Agent {
    let confirmation = Arc::clone(&gate);
    let authorization = Arc::clone(&gate);
    let service = gate;
    Agent {
        request_default: true,
        request_confirmation: Some(Box::new(move |request| {
            let gate = Arc::clone(&confirmation);
            async move { gate.request(request.device, Some(request.passkey)).await }.boxed()
        })),
        request_authorization: Some(Box::new(move |request| {
            let gate = Arc::clone(&authorization);
            async move { gate.request(request.device, None).await }.boxed()
        })),
        authorize_service: Some(Box::new(move |request| {
            let gate = Arc::clone(&service);
            async move { gate.request(request.device, None).await }.boxed()
        })),
        ..Default::default()
    }
}

async fn paired_addresses(adapter: &Adapter) -> Result<HashSet<Address>, SetupFailure> {
    let mut paired = HashSet::new();
    for address in adapter
        .device_addresses()
        .await
        .map_err(|_| SetupFailure::BackendFailed)?
    {
        let device = adapter
            .device(address)
            .map_err(|_| SetupFailure::BackendFailed)?;
        if device.is_paired().await.unwrap_or(false) {
            paired.insert(address);
        }
    }
    Ok(paired)
}

async fn existing_candidate(
    adapter: &Adapter,
    address: Address,
) -> Result<Option<Candidate>, SetupFailure> {
    let Some((address, device)) = identity_device(adapter, address).await? else {
        return Ok(None);
    };
    let ready = device.is_paired().await.unwrap_or(false)
        && device.is_trusted().await.unwrap_or(false)
        && device.is_connected().await.unwrap_or(false)
        && device.is_services_resolved().await.unwrap_or(false)
        && transport::has_complete_ancs(&device).await.unwrap_or(false);
    if !ready {
        return Ok(None);
    }
    Ok(Some(Candidate {
        kind: ConfirmationKind::ExistingBond,
        address: address.to_string(),
        device_name: device
            .name()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "iPhone".into()),
        passkey: None,
    }))
}

async fn paired_candidate(
    adapter: &Adapter,
    address: Address,
) -> Result<Option<Candidate>, SetupFailure> {
    let Some((address, device)) = identity_device(adapter, address).await? else {
        return Ok(None);
    };
    if !device.is_paired().await.unwrap_or(false) || !device.is_trusted().await.unwrap_or(false) {
        return Ok(None);
    }
    Ok(Some(Candidate {
        kind: ConfirmationKind::ExistingBond,
        address: address.to_string(),
        device_name: device
            .name()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "iPhone".into()),
        passkey: None,
    }))
}

async fn identity_device(
    adapter: &Adapter,
    requested: Address,
) -> Result<Option<(Address, bluer::Device)>, SetupFailure> {
    let path_addresses = adapter
        .device_addresses()
        .await
        .map_err(|_| SetupFailure::BackendFailed)?;
    if path_addresses.contains(&requested) {
        let device = adapter
            .device(requested)
            .map_err(|_| SetupFailure::BackendFailed)?;
        let identity = device.remote_address().await.unwrap_or(requested);
        return Ok(Some((identity, device)));
    }
    let device = transport::device_by_identity(adapter, requested)
        .await
        .map_err(|_| SetupFailure::BackendFailed)?;
    Ok(device.map(|device| (requested, device)))
}

async fn find_ancs_sources(
    device: &bluer::Device,
) -> bluer::Result<Option<(Characteristic, Characteristic)>> {
    for service in device.services().await? {
        if service.uuid().await? != hid::ANCS_SERVICE_UUID {
            continue;
        }
        let mut notification_source = None;
        let mut data_source = None;
        let mut control_point = false;
        for characteristic in service.characteristics().await? {
            match characteristic.uuid().await? {
                transport::NOTIFICATION_SOURCE_UUID => notification_source = Some(characteristic),
                transport::DATA_SOURCE_UUID => data_source = Some(characteristic),
                transport::CONTROL_POINT_UUID => control_point = true,
                _ => {}
            }
        }
        return Ok(match (data_source, notification_source, control_point) {
            (Some(data_source), Some(notification_source), true) => {
                Some((data_source, notification_source))
            }
            _ => None,
        });
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atomic_file::test_support::TestDirectory;
    use anyhow::{bail, Result};

    #[derive(Default)]
    struct FakeServices {
        restarts: Mutex<usize>,
        fail_after: Option<usize>,
    }

    impl UserServiceControl for FakeServices {
        fn unit_exists(&self, _: &str) -> Result<bool> {
            Ok(true)
        }
        fn restart(&self, _: &str) -> Result<()> {
            let mut restarts = self.restarts.lock().unwrap();
            *restarts += 1;
            if self.fail_after.is_some_and(|limit| *restarts >= limit) {
                bail!("restart failure")
            }
            Ok(())
        }
        fn stop_and_disable(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    fn configuration() -> Configuration {
        Configuration {
            schema_version: CONFIG_SCHEMA_VERSION,
            bluetooth: BluetoothConfiguration {
                adapter: "hci0".into(),
                adapter_address: Some("11:22:33:44:55:66".into()),
                device_address: "AA:BB:CC:DD:EE:FF".into(),
                device_name: "iPhone".into(),
            },
            desktop: DesktopConfiguration {
                suppress_phone_audio: true,
            },
        }
    }

    #[test]
    fn repair_requires_and_returns_only_the_configured_identity() {
        let validated = configuration().validate().unwrap();
        assert_eq!(
            requested_repair_target(SetupOptions::default(), Some(&validated)),
            Ok(None)
        );
        assert_eq!(
            requested_repair_target(
                SetupOptions {
                    repair: true,
                    ..Default::default()
                },
                None,
            ),
            Err(SetupFailure::RepairTargetUnknown)
        );
        assert_eq!(
            requested_repair_target(
                SetupOptions {
                    repair: true,
                    ..Default::default()
                },
                Some(&validated),
            ),
            Ok(Some("AA:BB:CC:DD:EE:FF".parse().unwrap()))
        );
    }

    #[tokio::test]
    async fn agent_gate_allows_one_identity_and_rejects_competitors() {
        let (notices, mut receiver) = mpsc::unbounded_channel();
        let gate = Arc::new(AgentGate {
            state: Mutex::new(AgentGateState::default()),
            notices,
        });
        let first_address: Address = "AA:BB:CC:DD:EE:FF".parse().unwrap();
        let other_address: Address = "11:22:33:44:55:66".parse().unwrap();
        let first_gate = Arc::clone(&gate);
        let first =
            tokio::spawn(async move { first_gate.request(first_address, Some(123456)).await });
        let notice = receiver.recv().await.unwrap();
        assert_eq!(notice.address, first_address);
        assert_eq!(notice.passkey, Some(123456));
        assert_eq!(
            gate.request(other_address, Some(654321)).await,
            Err(AgentReqError::Rejected)
        );
        gate.answer(first_address, true).unwrap();
        assert_eq!(first.await.unwrap(), Ok(()));
        assert_eq!(gate.request(first_address, None).await, Ok(()));
        assert!(
            receiver.try_recv().is_err(),
            "same identity is not re-prompted"
        );
    }

    #[tokio::test]
    async fn rejecting_an_initial_bond_reopens_gate_for_a_fresh_identity() {
        let (notices, mut receiver) = mpsc::unbounded_channel();
        let gate = Arc::new(AgentGate {
            state: Mutex::new(AgentGateState::default()),
            notices,
        });
        let old: Address = "AA:BB:CC:DD:EE:FF".parse().unwrap();
        let fresh: Address = "11:22:33:44:55:66".parse().unwrap();
        let old_gate = Arc::clone(&gate);
        let old_request = tokio::spawn(async move { old_gate.request(old, None).await });
        assert_eq!(receiver.recv().await.unwrap().address, old);
        gate.answer(old, false).unwrap();
        assert_eq!(old_request.await.unwrap(), Err(AgentReqError::Rejected));

        let fresh_gate = Arc::clone(&gate);
        let fresh_request = tokio::spawn(async move { fresh_gate.request(fresh, None).await });
        assert_eq!(receiver.recv().await.unwrap().address, fresh);
        gate.answer(fresh, true).unwrap();
        assert_eq!(fresh_request.await.unwrap(), Ok(()));
    }

    #[test]
    fn persistent_commit_writes_configuration_last_and_rolls_back_only_new_rule() {
        let directory = TestDirectory::new("setup-commit");
        let rule = AudioRule::new(
            directory.path().join("wp"),
            "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        );
        let blocker = directory.path().join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        let failing_store = ConfigurationStore::new(blocker.join("config.toml"));
        let services = FakeServices::default();
        assert_eq!(
            commit_persistent(
                &failing_store,
                &configuration(),
                None,
                Some(&rule),
                &services,
            ),
            Err(SetupFailure::ConfigurationWriteFailed)
        );
        assert!(!rule.path().exists());
        assert!(!rule.role_path().exists());
        assert_eq!(*services.restarts.lock().unwrap(), 2);

        std::fs::create_dir_all(rule.path().parent().unwrap()).unwrap();
        std::fs::write(rule.path(), rule.content()).unwrap();
        assert_eq!(
            commit_persistent(
                &failing_store,
                &configuration(),
                Some(&rule),
                Some(&rule),
                &services,
            ),
            Err(SetupFailure::ConfigurationWriteFailed)
        );
        assert!(
            rule.path().exists(),
            "preexisting exact-device rule is preserved during role-policy upgrade"
        );
        assert!(
            !rule.role_path().exists(),
            "new role policy is rolled back after configuration failure"
        );
        assert_eq!(*services.restarts.lock().unwrap(), 4);

        rule.apply().unwrap();
        assert_eq!(
            commit_persistent(
                &failing_store,
                &configuration(),
                Some(&rule),
                Some(&rule),
                &services,
            ),
            Err(SetupFailure::ConfigurationWriteFailed)
        );
        assert!(
            rule.path().exists(),
            "preexisting identical rule is preserved"
        );
        assert_eq!(*services.restarts.lock().unwrap(), 4);

        let successful_store = ConfigurationStore::new(directory.path().join("config/config.toml"));
        commit_persistent(
            &successful_store,
            &configuration(),
            Some(&rule),
            Some(&rule),
            &services,
        )
        .unwrap();
        assert!(successful_store.load().unwrap().is_some());
    }

    #[test]
    fn rollback_failure_is_never_hidden_by_configuration_failure() {
        let directory = TestDirectory::new("setup-rollback-failure");
        let rule = AudioRule::new(
            directory.path().join("wp"),
            "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        );
        let blocker = directory.path().join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        let store = ConfigurationStore::new(blocker.join("config.toml"));
        let services = FakeServices {
            fail_after: Some(2),
            ..Default::default()
        };
        assert_eq!(
            commit_persistent(&store, &configuration(), None, Some(&rule), &services,),
            Err(SetupFailure::CleanupFailed)
        );
        assert!(!rule.path().exists());
    }

    #[test]
    fn disabling_suppression_is_reversible_until_configuration_commits() {
        let directory = TestDirectory::new("setup-disable-audio");
        let rule = AudioRule::new(
            directory.path().join("wp"),
            "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        );
        rule.apply().unwrap();
        let blocker = directory.path().join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        let store = ConfigurationStore::new(blocker.join("config.toml"));
        let services = FakeServices::default();
        let mut disabled = configuration();
        disabled.desktop.suppress_phone_audio = false;

        assert_eq!(
            commit_persistent(&store, &disabled, Some(&rule), None, &services),
            Err(SetupFailure::ConfigurationWriteFailed)
        );
        assert!(rule.path().exists());
        assert!(rule.role_path().exists());
        assert_eq!(*services.restarts.lock().unwrap(), 2);
    }

    #[test]
    fn setup_commits_false_to_true_and_true_to_false_audio_intent() {
        let directory = TestDirectory::new("setup-audio-intent");
        let store = ConfigurationStore::new(directory.path().join("config/config.toml"));
        let rule = AudioRule::new(
            directory.path().join("wp"),
            "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        );
        let services = FakeServices::default();
        let enabled = configuration();
        let mut disabled = enabled.clone();
        disabled.desktop.suppress_phone_audio = false;

        commit_persistent(&store, &enabled, None, Some(&rule), &services).unwrap();
        assert!(rule.path().exists());
        assert!(rule.role_path().exists());
        assert!(store.load().unwrap().unwrap().suppress_phone_audio);

        commit_persistent(&store, &disabled, Some(&rule), None, &services).unwrap();
        assert!(!rule.path().exists());
        assert!(!rule.role_path().exists());
        assert!(!store.load().unwrap().unwrap().suppress_phone_audio);
        assert_eq!(*services.restarts.lock().unwrap(), 2);
    }
}
