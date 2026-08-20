use crate::{
    bluetooth::transport,
    config::ValidatedConfiguration,
    machine::{CheckStatus, DoctorCheck, DoctorOutput},
    service::UserServiceControl,
};
use anyhow::Result;
use bluer::Session;
use std::process::Command;

const VALIDATED_BLUEZ_VERSION: &str = "5.87";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdapterSelection {
    Selected(String),
    None,
    Ambiguous,
    ConfiguredMissing,
}

pub fn select_adapter(
    configured: Option<&ValidatedConfiguration>,
    adapters: &[String],
) -> AdapterSelection {
    if let Some(configuration) = configured {
        return if adapters.contains(&configuration.adapter) {
            AdapterSelection::Selected(configuration.adapter.clone())
        } else {
            AdapterSelection::ConfiguredMissing
        };
    }
    match adapters {
        [only] => AdapterSelection::Selected(only.clone()),
        [] => AdapterSelection::None,
        _ => AdapterSelection::Ambiguous,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticSnapshot {
    pub bluez_version: Option<String>,
    pub adapter: AdapterSelection,
    pub adapter_powered: Option<bool>,
    pub central_role: Option<bool>,
    pub peripheral_role: Option<bool>,
    pub le_advertising: Option<bool>,
    pub configured: bool,
    pub paired: Option<bool>,
    pub wireplumber_available: bool,
    pub wireplumber_required: bool,
    pub connected: Option<bool>,
    pub ancs_ready: Option<bool>,
}

pub fn diagnose(snapshot: &DiagnosticSnapshot) -> DoctorOutput {
    let version = match snapshot.bluez_version.as_deref() {
        None => fail("bluez-version", "bluez-unavailable"),
        Some(VALIDATED_BLUEZ_VERSION) => pass("bluez-version"),
        Some(version) if parse_version(version) => {
            warn("bluez-version", "bluez-version-unvalidated")
        }
        Some(_) => fail("bluez-version", "bluez-version-unreadable"),
    };
    let adapter_selection_code = match snapshot.adapter {
        AdapterSelection::Selected(_) => None,
        AdapterSelection::None => Some("adapter-not-found"),
        AdapterSelection::Ambiguous => Some("adapter-ambiguous"),
        AdapterSelection::ConfiguredMissing => Some("configured-adapter-missing"),
    };
    let power = match (adapter_selection_code, snapshot.adapter_powered) {
        (Some(code), _) => fail("adapter-power", code),
        (None, Some(true)) => pass("adapter-power"),
        (None, Some(false)) => fail("adapter-power", "adapter-powered-off"),
        (None, None) => fail("adapter-power", "adapter-power-unavailable"),
    };
    let roles = match (
        adapter_selection_code,
        snapshot.central_role,
        snapshot.peripheral_role,
    ) {
        (Some(code), _, _) => fail("adapter-roles", code),
        (None, Some(true), Some(true)) => pass("adapter-roles"),
        (None, Some(_), Some(_)) => fail("adapter-roles", "adapter-roles-missing"),
        _ => fail("adapter-roles", "adapter-roles-unavailable"),
    };
    let advertising = match (adapter_selection_code, snapshot.le_advertising) {
        (Some(code), _) => fail("le-advertising", code),
        (None, Some(true)) => pass("le-advertising"),
        (None, Some(false)) => fail("le-advertising", "le-advertising-unavailable"),
        (None, None) => fail("le-advertising", "le-advertising-unknown"),
    };
    let pairing = match (snapshot.configured, snapshot.paired) {
        (true, Some(true)) => pass("existing-pairing"),
        (true, _) => fail("existing-pairing", "configured-pairing-missing"),
        (false, Some(true)) => pass("existing-pairing"),
        (false, _) => warn("existing-pairing", "pairing-not-configured"),
    };
    let wireplumber = match (
        snapshot.wireplumber_available,
        snapshot.wireplumber_required,
    ) {
        (true, _) => pass("wireplumber"),
        (false, true) => fail("wireplumber", "wireplumber-required-unavailable"),
        (false, false) => warn("wireplumber", "wireplumber-optional-unavailable"),
    };
    let readiness = match (snapshot.configured, snapshot.connected, snapshot.ancs_ready) {
        (_, Some(true), Some(true)) => pass("ancs-readiness"),
        (true, Some(false), _) => warn("ancs-readiness", "phone-disconnected"),
        (true, _, _) => warn("ancs-readiness", "ancs-unavailable"),
        (false, _, _) => warn("ancs-readiness", "ancs-not-configured"),
    };
    DoctorOutput::new(vec![
        version,
        power,
        roles,
        advertising,
        pairing,
        wireplumber,
        readiness,
    ])
}

pub async fn probe(
    configured: Option<&ValidatedConfiguration>,
    services: &dyn UserServiceControl,
) -> Result<DiagnosticSnapshot> {
    let bluez_version = bluez_version();
    let wireplumber_available = services
        .unit_exists(crate::audio::WIREPLUMBER_UNIT)
        .unwrap_or(false);
    let session = match Session::new().await {
        Ok(session) => session,
        Err(_) => {
            return Ok(DiagnosticSnapshot {
                bluez_version,
                adapter: AdapterSelection::None,
                adapter_powered: None,
                central_role: None,
                peripheral_role: None,
                le_advertising: None,
                configured: configured.is_some(),
                paired: None,
                wireplumber_available,
                wireplumber_required: configured.is_some_and(|value| value.suppress_phone_audio),
                connected: None,
                ancs_ready: None,
            });
        }
    };
    let adapters = session.adapter_names().await?;
    let selection = select_adapter(configured, &adapters);
    let adapter = match &selection {
        AdapterSelection::Selected(name) => Some(session.adapter(name)?),
        _ => None,
    };
    let adapter_powered = match &adapter {
        Some(adapter) => adapter.is_powered().await.ok(),
        None => None,
    };
    let le_advertising = match &adapter {
        Some(adapter) => adapter
            .supported_advertising_instances()
            .await
            .ok()
            .map(|count| count > 0),
        None => None,
    };
    let adapter_address = match &adapter {
        Some(adapter) => adapter.address().await.ok().map(|value| value.to_string()),
        None => None,
    };
    let role_output = adapter_address
        .as_deref()
        .and_then(|address| command_stdout("bluetoothctl", &["show", address]));
    let central_role = role_output
        .as_deref()
        .map(|output| contains_role(output, "central"));
    let peripheral_role = role_output
        .as_deref()
        .map(|output| contains_role(output, "peripheral"));

    let mut paired = None;
    let mut connected = None;
    let mut ancs_ready = None;
    if let (Some(adapter), Some(configuration)) = (&adapter, configured) {
        let Some(device) =
            transport::device_by_identity(adapter, configuration.device_address).await?
        else {
            return Ok(DiagnosticSnapshot {
                bluez_version,
                adapter: selection,
                adapter_powered,
                central_role,
                peripheral_role,
                le_advertising,
                configured: true,
                paired: Some(false),
                wireplumber_available,
                wireplumber_required: configuration.suppress_phone_audio,
                connected: Some(false),
                ancs_ready: Some(false),
            });
        };
        let is_paired = device.is_paired().await.unwrap_or(false);
        paired = Some(is_paired);
        if is_paired {
            let is_connected = device.is_connected().await.unwrap_or(false);
            connected = Some(is_connected);
            if is_connected && device.is_services_resolved().await.unwrap_or(false) {
                ancs_ready = Some(transport::has_complete_ancs(&device).await.unwrap_or(false));
            } else {
                ancs_ready = Some(false);
            }
        }
    }
    Ok(DiagnosticSnapshot {
        bluez_version,
        adapter: selection,
        adapter_powered,
        central_role,
        peripheral_role,
        le_advertising,
        configured: configured.is_some(),
        paired,
        wireplumber_available,
        wireplumber_required: configured.is_some_and(|value| value.suppress_phone_audio),
        connected,
        ancs_ready,
    })
}

fn command_stdout(command: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(command).args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn bluez_version() -> Option<String> {
    [
        "bluetoothd",
        "/usr/lib/bluetooth/bluetoothd",
        "/usr/libexec/bluetooth/bluetoothd",
    ]
    .into_iter()
    .find_map(|command| command_stdout(command, &["-v"]))
}

fn contains_role(output: &str, role: &str) -> bool {
    output
        .lines()
        .filter(|line| line.to_ascii_lowercase().contains("role"))
        .any(|line| line.split_ascii_whitespace().any(|word| word == role))
}

fn parse_version(version: &str) -> bool {
    let mut components = version.split('.');
    matches!(components.next(), Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        && matches!(components.next(), Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        && components
            .all(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn pass(id: &'static str) -> DoctorCheck {
    DoctorCheck {
        id,
        status: CheckStatus::Pass,
        code: None,
    }
}

fn warn(id: &'static str, code: &'static str) -> DoctorCheck {
    DoctorCheck {
        id,
        status: CheckStatus::Warn,
        code: Some(code),
    }
}

fn fail(id: &'static str, code: &'static str) -> DoctorCheck {
    DoctorCheck {
        id,
        status: CheckStatus::Fail,
        code: Some(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> DiagnosticSnapshot {
        DiagnosticSnapshot {
            bluez_version: Some("5.87".into()),
            adapter: AdapterSelection::Selected("hci0".into()),
            adapter_powered: Some(true),
            central_role: Some(true),
            peripheral_role: Some(true),
            le_advertising: Some(true),
            configured: true,
            paired: Some(true),
            wireplumber_available: true,
            wireplumber_required: true,
            connected: Some(true),
            ancs_ready: Some(true),
        }
    }

    #[test]
    fn adapter_selection_never_guesses() {
        assert_eq!(select_adapter(None, &[]), AdapterSelection::None);
        assert_eq!(
            select_adapter(None, &["hci0".into(), "hci1".into()]),
            AdapterSelection::Ambiguous
        );
        assert_eq!(
            select_adapter(None, &["hci1".into()]),
            AdapterSelection::Selected("hci1".into())
        );
    }

    #[test]
    fn supported_snapshot_passes_all_checks() {
        let output = diagnose(&healthy());
        assert!(output.ok);
        assert_eq!(output.checks.len(), 7);
        assert!(output
            .checks
            .iter()
            .all(|check| check.status == CheckStatus::Pass));
    }

    #[test]
    fn diagnostic_matrix_classifies_required_and_transient_conditions() {
        type Case = (
            Box<dyn Fn(&mut DiagnosticSnapshot)>,
            &'static str,
            CheckStatus,
        );
        let cases: Vec<Case> = vec![
            (
                Box::new(|s| s.bluez_version = None),
                "bluez-version",
                CheckStatus::Fail,
            ),
            (
                Box::new(|s| s.bluez_version = Some("5.86".into())),
                "bluez-version",
                CheckStatus::Warn,
            ),
            (
                Box::new(|s| s.bluez_version = Some("unknown".into())),
                "bluez-version",
                CheckStatus::Fail,
            ),
            (
                Box::new(|s| s.adapter_powered = Some(false)),
                "adapter-power",
                CheckStatus::Fail,
            ),
            (
                Box::new(|s| s.adapter = AdapterSelection::Ambiguous),
                "adapter-power",
                CheckStatus::Fail,
            ),
            (
                Box::new(|s| s.paired = Some(false)),
                "existing-pairing",
                CheckStatus::Fail,
            ),
            (
                Box::new(|s| s.connected = Some(false)),
                "ancs-readiness",
                CheckStatus::Warn,
            ),
            (
                Box::new(|s| s.ancs_ready = Some(false)),
                "ancs-readiness",
                CheckStatus::Warn,
            ),
            (
                Box::new(|s| s.wireplumber_available = false),
                "wireplumber",
                CheckStatus::Fail,
            ),
        ];
        for (mutate, id, status) in cases {
            let mut snapshot = healthy();
            mutate(&mut snapshot);
            let output = diagnose(&snapshot);
            assert_eq!(
                output
                    .checks
                    .iter()
                    .find(|check| check.id == id)
                    .unwrap()
                    .status,
                status
            );
        }

        let mut optional = healthy();
        optional.configured = false;
        optional.paired = None;
        optional.connected = None;
        optional.ancs_ready = None;
        optional.wireplumber_available = false;
        optional.wireplumber_required = false;
        let output = diagnose(&optional);
        assert!(output.ok);
        assert_eq!(
            output.checks[5].status,
            CheckStatus::Warn,
            "optional WirePlumber is a warning"
        );
    }
}
