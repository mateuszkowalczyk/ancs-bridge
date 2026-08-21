use serde::{ser::SerializeStruct, Deserialize, Serialize, Serializer};

pub const API_VERSION: u32 = 1;
pub const MAX_SETUP_COMMAND_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorOutput {
    pub api_version: u32,
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DoctorCheck {
    pub id: &'static str,
    pub status: CheckStatus,
    pub code: Option<&'static str>,
}

impl DoctorOutput {
    pub fn new(checks: Vec<DoctorCheck>) -> Self {
        let ok = checks.iter().all(|check| check.status != CheckStatus::Fail);
        Self {
            api_version: API_VERSION,
            ok,
            checks,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SetupState {
    CheckingEnvironment,
    WaitingForIphone,
    VerifyingAncs,
    ApplyingConfiguration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfirmationKind {
    Pairing,
    ExistingBond,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetupEvent {
    State {
        v: u32,
        state: SetupState,
    },
    ConfirmationRequest {
        v: u32,
        kind: ConfirmationKind,
        request_id: String,
        device_name: String,
        address: String,
        passkey: Option<String>,
    },
    Complete {
        v: u32,
        address: String,
    },
    Error {
        v: u32,
        code: &'static str,
        recoverable: bool,
    },
}

impl Serialize for SetupEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::State { v, state } => {
                let mut object = serializer.serialize_struct("SetupEvent", 3)?;
                object.serialize_field("v", v)?;
                object.serialize_field("event", "state")?;
                object.serialize_field("state", state)?;
                object.end()
            }
            Self::ConfirmationRequest {
                v,
                kind,
                request_id,
                device_name,
                address,
                passkey,
            } => {
                let mut object = serializer.serialize_struct("SetupEvent", 7)?;
                object.serialize_field("v", v)?;
                object.serialize_field("event", "confirmation-request")?;
                object.serialize_field("kind", kind)?;
                object.serialize_field("requestId", request_id)?;
                object.serialize_field("deviceName", device_name)?;
                object.serialize_field("address", address)?;
                object.serialize_field("passkey", passkey)?;
                object.end()
            }
            Self::Complete { v, address } => {
                let mut object = serializer.serialize_struct("SetupEvent", 3)?;
                object.serialize_field("v", v)?;
                object.serialize_field("event", "complete")?;
                object.serialize_field("address", address)?;
                object.end()
            }
            Self::Error {
                v,
                code,
                recoverable,
            } => {
                let mut object = serializer.serialize_struct("SetupEvent", 4)?;
                object.serialize_field("v", v)?;
                object.serialize_field("event", "error")?;
                object.serialize_field("code", code)?;
                object.serialize_field("recoverable", recoverable)?;
                object.end()
            }
        }
    }
}

impl SetupEvent {
    pub fn state(state: SetupState) -> Self {
        Self::State {
            v: API_VERSION,
            state,
        }
    }

    pub fn error(error: SetupFailure) -> Self {
        Self::Error {
            v: API_VERSION,
            code: error.code(),
            recoverable: error.recoverable(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetupCommand {
    Confirm { request_id: String, accept: bool },
    Cancel,
}

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
enum WireCommand {
    Confirm {
        v: u32,
        #[serde(rename = "requestId")]
        request_id: String,
        accept: bool,
    },
    Cancel {
        v: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    Malformed,
    UnsupportedVersion,
    WrongState,
    ConfirmationMismatch,
}

pub fn parse_command(line: &str) -> Result<SetupCommand, ProtocolError> {
    if line.len() > MAX_SETUP_COMMAND_BYTES {
        return Err(ProtocolError::Malformed);
    }
    let command: WireCommand = serde_json::from_str(line).map_err(|_| ProtocolError::Malformed)?;
    match command {
        WireCommand::Confirm {
            v,
            request_id,
            accept,
        } => {
            if v != API_VERSION {
                return Err(ProtocolError::UnsupportedVersion);
            }
            Ok(SetupCommand::Confirm { request_id, accept })
        }
        WireCommand::Cancel { v } => {
            if v != API_VERSION {
                return Err(ProtocolError::UnsupportedVersion);
            }
            Ok(SetupCommand::Cancel)
        }
    }
}

pub fn validate_command(
    command: &SetupCommand,
    active_confirmation: Option<&str>,
) -> Result<(), ProtocolError> {
    match command {
        SetupCommand::Cancel => Ok(()),
        SetupCommand::Confirm { request_id, .. } => match active_confirmation {
            Some(active) if active == request_id => Ok(()),
            Some(_) => Err(ProtocolError::ConfirmationMismatch),
            None => Err(ProtocolError::WrongState),
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupFailure {
    EnvironmentUnavailable,
    AdapterUnavailable,
    AdapterPoweredOff,
    AdapterCapabilityMissing,
    CandidateTimeout,
    ConfirmationTimeout,
    AncsTimeout,
    Cancelled,
    Rejected,
    InvalidProtocol,
    UnsupportedApiVersion,
    StdinClosed,
    PairingFailed,
    TrustFailed,
    RepairRequired,
    RepairTargetUnknown,
    AudioUnavailable,
    AudioRuleConflict,
    AudioRestartFailed,
    ConfigurationWriteFailed,
    CleanupFailed,
    BackendFailed,
}

impl SetupFailure {
    pub const ALL: [Self; 22] = [
        Self::EnvironmentUnavailable,
        Self::AdapterUnavailable,
        Self::AdapterPoweredOff,
        Self::AdapterCapabilityMissing,
        Self::CandidateTimeout,
        Self::ConfirmationTimeout,
        Self::AncsTimeout,
        Self::Cancelled,
        Self::Rejected,
        Self::InvalidProtocol,
        Self::UnsupportedApiVersion,
        Self::StdinClosed,
        Self::PairingFailed,
        Self::TrustFailed,
        Self::RepairRequired,
        Self::RepairTargetUnknown,
        Self::AudioUnavailable,
        Self::AudioRuleConflict,
        Self::AudioRestartFailed,
        Self::ConfigurationWriteFailed,
        Self::CleanupFailed,
        Self::BackendFailed,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::EnvironmentUnavailable => "environment-unavailable",
            Self::AdapterUnavailable => "adapter-unavailable",
            Self::AdapterPoweredOff => "adapter-powered-off",
            Self::AdapterCapabilityMissing => "adapter-capability-missing",
            Self::CandidateTimeout => "candidate-timeout",
            Self::ConfirmationTimeout => "confirmation-timeout",
            Self::AncsTimeout => "ancs-timeout",
            Self::Cancelled => "cancelled",
            Self::Rejected => "rejected",
            Self::InvalidProtocol => "invalid-protocol",
            Self::UnsupportedApiVersion => "unsupported-api-version",
            Self::StdinClosed => "stdin-closed",
            Self::PairingFailed => "pairing-failed",
            Self::TrustFailed => "trust-failed",
            Self::RepairRequired => "repair-required",
            Self::RepairTargetUnknown => "repair-target-unknown",
            Self::AudioUnavailable => "audio-unavailable",
            Self::AudioRuleConflict => "audio-rule-conflict",
            Self::AudioRestartFailed => "audio-restart-failed",
            Self::ConfigurationWriteFailed => "configuration-write-failed",
            Self::CleanupFailed => "cleanup-failed",
            Self::BackendFailed => "backend-failed",
        }
    }

    pub fn recoverable(self) -> bool {
        !matches!(
            self,
            Self::AdapterCapabilityMissing
                | Self::InvalidProtocol
                | Self::UnsupportedApiVersion
                | Self::RepairTargetUnknown
                | Self::AudioRuleConflict
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_parser_rejects_unknown_malformed_and_unsupported_input() {
        assert_eq!(
            parse_command(r#"{"v":1,"command":"cancel"}"#),
            Ok(SetupCommand::Cancel)
        );
        assert_eq!(
            parse_command(r#"{"v":2,"command":"cancel"}"#),
            Err(ProtocolError::UnsupportedVersion)
        );
        for invalid in [
            r#"{"v":1,"command":"unknown"}"#,
            r#"{"v":"1","command":"cancel"}"#,
            r#"{"v":1,"command":"confirm","requestId":7,"accept":true}"#,
            "not-json",
        ] {
            assert_eq!(parse_command(invalid), Err(ProtocolError::Malformed));
        }

        let oversized = format!(
            r#"{{"v":1,"command":"cancel","padding":"{}"}}"#,
            "x".repeat(8 * 1024)
        );
        assert_eq!(parse_command(&oversized), Err(ProtocolError::Malformed));
    }

    #[test]
    fn confirmation_validation_happens_before_authorization() {
        let command = SetupCommand::Confirm {
            request_id: "expected".into(),
            accept: true,
        };
        assert_eq!(
            validate_command(&command, None),
            Err(ProtocolError::WrongState)
        );
        assert_eq!(
            validate_command(&command, Some("other")),
            Err(ProtocolError::ConfirmationMismatch)
        );
        assert_eq!(validate_command(&command, Some("expected")), Ok(()));
    }

    #[test]
    fn doctor_ok_depends_only_on_failed_checks() {
        assert!(
            DoctorOutput::new(vec![DoctorCheck {
                id: "wireplumber",
                status: CheckStatus::Warn,
                code: Some("wireplumber-optional-unavailable"),
            }])
            .ok
        );
        assert!(
            !DoctorOutput::new(vec![DoctorCheck {
                id: "adapter-power",
                status: CheckStatus::Fail,
                code: Some("adapter-powered-off"),
            }])
            .ok
        );
    }
}
