use crate::{atomic_file, config::ValidatedConfiguration};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub const MACHINE_API_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeState {
    Unconfigured,
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

/// Metadata-only supervisor state. Payload values have no representation here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSnapshot {
    pub state: RuntimeState,
    pub reason_code: Option<&'static str>,
    pub last_error_code: Option<&'static str>,
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
            last_error_code: None,
            connected: false,
            services_resolved: false,
            ancs_available: false,
            subscribed: false,
            delivered_count: 0,
            recoverable_error_count: 0,
        }
    }

    pub fn record_error(&mut self, code: &'static str) {
        self.last_error_code = Some(code);
        self.recoverable_error_count += 1;
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub api_version: u32,
    pub state: RuntimeState,
    pub reason_code: Option<String>,
    pub adapter: Option<String>,
    #[serde(default)]
    pub adapter_address: Option<String>,
    pub device_address: Option<String>,
    pub device_name: Option<String>,
    pub connected: bool,
    pub services_resolved: bool,
    pub ancs_available: bool,
    pub subscribed: bool,
    pub last_error_code: Option<String>,
    pub last_transition_at: Option<String>,
    pub last_notification_at: Option<String>,
    pub pid: Option<u32>,
}

impl RuntimeStatus {
    fn unconfigured() -> Self {
        Self {
            api_version: MACHINE_API_VERSION,
            state: RuntimeState::Unconfigured,
            reason_code: None,
            adapter: None,
            adapter_address: None,
            device_address: None,
            device_name: None,
            connected: false,
            services_resolved: false,
            ancs_available: false,
            subscribed: false,
            last_error_code: None,
            last_transition_at: None,
            last_notification_at: None,
            pid: None,
        }
    }

    fn daemon_not_running(configuration: &ValidatedConfiguration) -> Self {
        Self {
            api_version: MACHINE_API_VERSION,
            state: RuntimeState::Error,
            reason_code: Some("daemon-not-running".into()),
            adapter: Some(configuration.adapter.clone()),
            adapter_address: configuration.adapter_address.map(|value| value.to_string()),
            device_address: Some(configuration.device_address.to_string()),
            device_name: Some(configuration.device_name.clone()),
            connected: false,
            services_resolved: false,
            ancs_available: false,
            subscribed: false,
            last_error_code: None,
            last_transition_at: None,
            last_notification_at: None,
            pid: None,
        }
    }

    fn validate_api_version(&self) -> Result<()> {
        if self.api_version != MACHINE_API_VERSION {
            bail!(
                "unsupported runtime status API version {}",
                self.api_version
            );
        }
        for (field, value) in [
            ("lastTransitionAt", self.last_transition_at.as_deref()),
            ("lastNotificationAt", self.last_notification_at.as_deref()),
        ] {
            if let Some(value) = value {
                OffsetDateTime::parse(value, &Rfc3339)
                    .with_context(|| format!("invalid {field} runtime status timestamp"))?;
            }
        }
        Ok(())
    }

    fn matches_configuration(&self, configuration: &ValidatedConfiguration) -> bool {
        let adapter_address = configuration.adapter_address.map(|value| value.to_string());
        let address = configuration.device_address.to_string();
        self.adapter.as_deref() == Some(configuration.adapter.as_str())
            && self.adapter_address == adapter_address
            && self.device_address.as_deref() == Some(address.as_str())
            && self.device_name.as_deref() == Some(configuration.device_name.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusOutput {
    #[serde(flatten)]
    pub status: RuntimeStatus,
    pub stale: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusIdentity {
    adapter: String,
    adapter_address: Option<String>,
    device_address: String,
    device_name: String,
}

impl From<&ValidatedConfiguration> for StatusIdentity {
    fn from(configuration: &ValidatedConfiguration) -> Self {
        Self {
            adapter: configuration.adapter.clone(),
            adapter_address: configuration.adapter_address.map(|value| value.to_string()),
            device_address: configuration.device_address.to_string(),
            device_name: configuration.device_name.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusStore {
    path: PathBuf,
}

impl StatusStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn from_environment() -> Result<Self> {
        Self::from_environment_value(std::env::var_os("XDG_RUNTIME_DIR"))
    }

    pub fn from_environment_value(runtime_directory: Option<OsString>) -> Result<Self> {
        let directory = absolute(runtime_directory.as_deref())
            .context("no absolute XDG_RUNTIME_DIR is available")?;
        Ok(Self::new(directory.join("ancs-bridge/status.json")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read(&self) -> Result<Option<RuntimeStatus>> {
        let source = match fs::read(&self.path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading runtime status {}", self.path.display()))
            }
        };
        let status: RuntimeStatus = serde_json::from_slice(&source)
            .with_context(|| format!("parsing runtime status {}", self.path.display()))?;
        status.validate_api_version()?;
        Ok(Some(status))
    }

    fn write(&self, status: &RuntimeStatus) -> Result<()> {
        let mut source = serde_json::to_vec(status).context("serializing runtime status")?;
        source.push(b'\n');
        atomic_file::replace(&self.path, &source, 0o700)
    }
}

fn absolute(value: Option<&OsStr>) -> Option<PathBuf> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|value| value.is_absolute())
}

pub trait TimestampSource: Send {
    fn now(&mut self) -> Result<String>;
}

#[derive(Default)]
pub struct SystemTimestampSource;

impl TimestampSource for SystemTimestampSource {
    fn now(&mut self) -> Result<String> {
        OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .context("formatting runtime status timestamp")
    }
}

pub struct PersistentStatusWriter<T = SystemTimestampSource> {
    store: StatusStore,
    timestamps: T,
    current: RuntimeStatus,
    observed_delivered_count: u64,
}

impl PersistentStatusWriter<SystemTimestampSource> {
    pub fn new(store: StatusStore, identity: StatusIdentity) -> Self {
        Self::with_timestamp_source(store, identity, SystemTimestampSource)
    }
}

impl<T: TimestampSource> PersistentStatusWriter<T> {
    pub fn with_timestamp_source(
        store: StatusStore,
        identity: StatusIdentity,
        timestamps: T,
    ) -> Self {
        let current = RuntimeStatus {
            api_version: MACHINE_API_VERSION,
            state: RuntimeState::WaitingForBluez,
            reason_code: None,
            adapter: Some(identity.adapter),
            adapter_address: identity.adapter_address,
            device_address: Some(identity.device_address),
            device_name: Some(identity.device_name),
            connected: false,
            services_resolved: false,
            ancs_available: false,
            subscribed: false,
            last_error_code: None,
            last_transition_at: None,
            last_notification_at: None,
            pid: Some(std::process::id()),
        };
        Self {
            store,
            timestamps,
            current,
            observed_delivered_count: 0,
        }
    }
}

#[async_trait]
pub trait StatusWriter: Send {
    async fn publish(&mut self, snapshot: StatusSnapshot) -> Result<()>;
}

#[async_trait]
impl<T: TimestampSource> StatusWriter for PersistentStatusWriter<T> {
    async fn publish(&mut self, snapshot: StatusSnapshot) -> Result<()> {
        let reason = snapshot.reason_code.map(str::to_owned);
        let transition = self.current.last_transition_at.is_none()
            || self.current.state != snapshot.state
            || self.current.reason_code != reason;
        if transition {
            self.current.last_transition_at = Some(self.timestamps.now()?);
        }
        if snapshot.delivered_count > self.observed_delivered_count {
            self.current.last_notification_at = Some(self.timestamps.now()?);
        }
        self.observed_delivered_count = snapshot.delivered_count;
        if let Some(code) = snapshot.last_error_code {
            self.current.last_error_code = Some(code.to_owned());
        }
        self.current.state = snapshot.state;
        self.current.reason_code = reason;
        self.current.connected = snapshot.connected;
        self.current.services_resolved = snapshot.services_resolved;
        self.current.ancs_available = snapshot.ancs_available;
        self.current.subscribed = snapshot.subscribed;
        self.store.write(&self.current)
    }
}

#[derive(Default)]
pub struct TracingStatusWriter;

#[async_trait]
impl StatusWriter for TracingStatusWriter {
    async fn publish(&mut self, snapshot: StatusSnapshot) -> Result<()> {
        tracing::info!(
            state = ?snapshot.state,
            reason_code = snapshot.reason_code,
            last_error_code = snapshot.last_error_code,
            connected = snapshot.connected,
            services_resolved = snapshot.services_resolved,
            ancs_available = snapshot.ancs_available,
            subscribed = snapshot.subscribed,
            delivered_count = snapshot.delivered_count,
            recoverable_error_count = snapshot.recoverable_error_count,
            "runtime state update"
        );
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct FakeStatusWriter {
    values: Arc<Mutex<Vec<StatusSnapshot>>>,
    fail_next: Arc<Mutex<bool>>,
}

impl FakeStatusWriter {
    pub fn values(&self) -> Vec<StatusSnapshot> {
        self.values.lock().expect("fake status poisoned").clone()
    }

    pub fn fail_next(&self) {
        *self.fail_next.lock().expect("fake status poisoned") = true;
    }
}

#[async_trait]
impl StatusWriter for FakeStatusWriter {
    async fn publish(&mut self, snapshot: StatusSnapshot) -> Result<()> {
        if std::mem::take(&mut *self.fail_next.lock().expect("fake status poisoned")) {
            bail!("injected status publication failure");
        }
        self.values
            .lock()
            .expect("fake status poisoned")
            .push(snapshot);
        Ok(())
    }
}

pub trait ProcessChecker {
    fn is_bridge_daemon(&self, pid: u32) -> bool;
}

#[derive(Default)]
pub struct ProcfsProcessChecker;

impl ProcessChecker for ProcfsProcessChecker {
    fn is_bridge_daemon(&self, pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        let Ok(command_line) = fs::read(format!("/proc/{pid}/cmdline")) else {
            return false;
        };
        let mut arguments = command_line
            .split(|value| *value == 0)
            .filter(|value| !value.is_empty());
        let Some(executable) = arguments.next() else {
            return false;
        };
        Path::new(OsStr::from_bytes(executable))
            .file_name()
            .is_some_and(|name| name == "ancs-bridge")
            && arguments.any(|argument| argument == b"daemon")
    }
}

pub fn status_output(
    configuration: Option<&ValidatedConfiguration>,
    store: Option<&StatusStore>,
    processes: &impl ProcessChecker,
) -> Result<StatusOutput> {
    let Some(configuration) = configuration else {
        return Ok(StatusOutput {
            status: RuntimeStatus::unconfigured(),
            stale: false,
        });
    };
    let store = store.context("runtime status path is unavailable")?;
    let Some(status) = store.read()? else {
        return Ok(StatusOutput {
            status: RuntimeStatus::daemon_not_running(configuration),
            stale: true,
        });
    };
    let live = status
        .pid
        .is_some_and(|pid| processes.is_bridge_daemon(pid));
    let stale = !live || !status.matches_configuration(configuration);
    Ok(StatusOutput { status, stale })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        atomic_file::test_support::TestDirectory,
        config::{
            BluetoothConfiguration, Configuration, DesktopConfiguration, CONFIG_SCHEMA_VERSION,
        },
    };
    use std::{collections::VecDeque, os::unix::fs::PermissionsExt};

    fn configuration() -> ValidatedConfiguration {
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
        .validate()
        .unwrap()
    }

    struct FakeTimestamps(VecDeque<String>);

    impl TimestampSource for FakeTimestamps {
        fn now(&mut self) -> Result<String> {
            self.0.pop_front().context("no fake timestamp remains")
        }
    }

    struct FakeProcesses(bool);

    impl ProcessChecker for FakeProcesses {
        fn is_bridge_daemon(&self, _: u32) -> bool {
            self.0
        }
    }

    #[test]
    fn runtime_path_requires_an_absolute_xdg_directory() {
        assert_eq!(
            StatusStore::from_environment_value(Some(OsString::from("/run/user/1000")))
                .unwrap()
                .path(),
            Path::new("/run/user/1000/ancs-bridge/status.json")
        );
        assert!(StatusStore::from_environment_value(Some(OsString::from("relative"))).is_err());
        assert!(StatusStore::from_environment_value(None).is_err());
    }

    #[tokio::test]
    async fn publishes_atomic_private_status_with_stable_and_event_timestamps() {
        let directory = TestDirectory::new("status");
        let store = StatusStore::new(directory.path().join("runtime/status.json"));
        let timestamps = FakeTimestamps(
            [
                "2026-08-19T10:00:00Z".into(),
                "2026-08-19T10:01:00Z".into(),
                "2026-08-19T10:02:00Z".into(),
            ]
            .into(),
        );
        let mut writer = PersistentStatusWriter::with_timestamp_source(
            store.clone(),
            StatusIdentity::from(&configuration()),
            timestamps,
        );
        let waiting = StatusSnapshot::new(RuntimeState::WaitingForPhone);
        writer.publish(waiting.clone()).await.unwrap();
        writer.publish(waiting.clone()).await.unwrap();
        let mut delivered = waiting;
        delivered.delivered_count = 1;
        delivered.record_error("desktop-delivery-recovered");
        writer.publish(delivered).await.unwrap();

        let status = store.read().unwrap().unwrap();
        assert_eq!(status.adapter.as_deref(), Some("hci0"));
        assert_eq!(status.adapter_address.as_deref(), Some("11:22:33:44:55:66"));
        assert_eq!(status.device_address.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert_eq!(status.device_name.as_deref(), Some("iPhone"));
        assert_eq!(
            status.last_transition_at.as_deref(),
            Some("2026-08-19T10:00:00Z")
        );
        assert_eq!(
            status.last_notification_at.as_deref(),
            Some("2026-08-19T10:01:00Z")
        );
        assert_eq!(
            status.last_error_code.as_deref(),
            Some("desktop-delivery-recovered")
        );
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(store.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o666)).unwrap();
        let mut transitioned = StatusSnapshot::new(RuntimeState::Ready);
        transitioned.delivered_count = 1;
        writer.publish(transitioned).await.unwrap();
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let retained = store.read().unwrap().unwrap();
        assert_eq!(
            retained.last_error_code.as_deref(),
            Some("desktop-delivery-recovered")
        );
        assert_eq!(
            retained.last_notification_at.as_deref(),
            Some("2026-08-19T10:01:00Z")
        );
        assert_eq!(
            retained.last_transition_at.as_deref(),
            Some("2026-08-19T10:02:00Z")
        );
    }

    #[tokio::test]
    async fn status_write_failure_is_reported_for_an_invalid_parent() {
        let directory = TestDirectory::new("status-failure");
        let parent_file = directory.path().join("not-a-directory");
        fs::write(&parent_file, b"occupied").unwrap();
        let store = StatusStore::new(parent_file.join("status.json"));
        let mut writer = PersistentStatusWriter::with_timestamp_source(
            store,
            StatusIdentity::from(&configuration()),
            FakeTimestamps(["2026-08-19T10:00:00Z".into()].into()),
        );
        assert!(writer
            .publish(StatusSnapshot::new(RuntimeState::WaitingForBluez))
            .await
            .is_err());
        assert_eq!(fs::read(parent_file).unwrap(), b"occupied");
    }

    #[test]
    fn reports_live_stale_unconfigured_and_not_running_states() {
        let directory = TestDirectory::new("status-output");
        let store = StatusStore::new(directory.path().join("status.json"));
        let configured = configuration();

        let unconfigured = status_output(None, None, &FakeProcesses(false)).unwrap();
        assert_eq!(unconfigured.status.state, RuntimeState::Unconfigured);
        assert!(!unconfigured.stale);

        let missing =
            status_output(Some(&configured), Some(&store), &FakeProcesses(false)).unwrap();
        assert_eq!(missing.status.state, RuntimeState::Error);
        assert_eq!(
            missing.status.reason_code.as_deref(),
            Some("daemon-not-running")
        );
        assert!(missing.stale);

        let status = RuntimeStatus {
            api_version: MACHINE_API_VERSION,
            state: RuntimeState::Ready,
            reason_code: None,
            adapter: Some("hci0".into()),
            adapter_address: Some("11:22:33:44:55:66".into()),
            device_address: Some("AA:BB:CC:DD:EE:FF".into()),
            device_name: Some("iPhone".into()),
            connected: true,
            services_resolved: true,
            ancs_available: true,
            subscribed: true,
            last_error_code: None,
            last_transition_at: Some("2026-08-19T10:00:00Z".into()),
            last_notification_at: None,
            pid: Some(1234),
        };
        store.write(&status).unwrap();
        assert!(
            !status_output(Some(&configured), Some(&store), &FakeProcesses(true))
                .unwrap()
                .stale
        );
        assert!(
            status_output(Some(&configured), Some(&store), &FakeProcesses(false))
                .unwrap()
                .stale
        );

        let mut changed = configured.clone();
        changed.device_name = "Other phone".into();
        assert!(
            status_output(Some(&changed), Some(&store), &FakeProcesses(true))
                .unwrap()
                .stale
        );

        let mut changed_controller = configured.clone();
        changed_controller.adapter_address = Some("22:33:44:55:66:77".parse().unwrap());
        assert!(
            status_output(
                Some(&changed_controller),
                Some(&store),
                &FakeProcesses(true)
            )
            .unwrap()
            .stale
        );
    }

    #[test]
    fn rejects_malformed_or_unsupported_status_but_ignores_additive_fields() {
        let directory = TestDirectory::new("status-input");
        let store = StatusStore::new(directory.path().join("status.json"));
        fs::write(store.path(), b"not-json").unwrap();
        assert!(store.read().is_err());

        let unsupported = serde_json::json!({
            "apiVersion": 2,
            "state": "ready",
            "reasonCode": null,
            "adapter": "hci0",
            "adapterAddress": "11:22:33:44:55:66",
            "deviceAddress": "AA:BB:CC:DD:EE:FF",
            "deviceName": "iPhone",
            "connected": true,
            "servicesResolved": true,
            "ancsAvailable": true,
            "subscribed": true,
            "lastErrorCode": null,
            "lastTransitionAt": null,
            "lastNotificationAt": null,
            "pid": 1
        });
        fs::write(store.path(), serde_json::to_vec(&unsupported).unwrap()).unwrap();
        assert!(store.read().is_err());

        let mut additive = unsupported;
        additive["apiVersion"] = serde_json::json!(1);
        additive["futureField"] = serde_json::json!("ignored");
        fs::write(store.path(), serde_json::to_vec(&additive).unwrap()).unwrap();
        assert!(store.read().unwrap().is_some());

        additive["lastTransitionAt"] = serde_json::json!("not-a-timestamp");
        fs::write(store.path(), serde_json::to_vec(&additive).unwrap()).unwrap();
        assert!(store.read().is_err());
    }

    #[test]
    fn serialized_status_has_no_payload_representation() {
        let status = RuntimeStatus::daemon_not_running(&configuration());
        let json = serde_json::to_string(&status).unwrap();
        for canary in [
            "secret.bundle",
            "Secret title",
            "Secret message",
            "appPayload",
        ] {
            assert!(!json.contains(canary));
        }
    }
}
