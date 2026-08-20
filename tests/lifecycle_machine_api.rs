use ancs_bridge::machine::{
    CheckStatus, ConfirmationKind, DoctorCheck, DoctorOutput, SetupEvent, SetupFailure, SetupState,
    API_VERSION,
};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

const BINARY: &str = env!("CARGO_BIN_EXE_ancs-bridge");
static DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/machine-api-v1")
            .join(name),
    )
    .unwrap()
}

fn line(value: &impl serde::Serialize) -> String {
    format!("{}\n", serde_json::to_string(value).unwrap())
}

#[test]
fn doctor_pass_warn_and_fail_fixtures_are_stable() {
    let ids = [
        "bluez-version",
        "adapter-power",
        "adapter-roles",
        "le-advertising",
        "existing-pairing",
        "wireplumber",
        "ancs-readiness",
    ];
    let passing = DoctorOutput::new(
        ids.into_iter()
            .map(|id| DoctorCheck {
                id,
                status: CheckStatus::Pass,
                code: None,
            })
            .collect(),
    );
    assert_eq!(line(&passing), fixture("doctor-pass.json"));

    let warning = DoctorOutput::new(vec![
        DoctorCheck {
            id: "bluez-version",
            status: CheckStatus::Warn,
            code: Some("bluez-version-unvalidated"),
        },
        DoctorCheck {
            id: "adapter-power",
            status: CheckStatus::Pass,
            code: None,
        },
        DoctorCheck {
            id: "adapter-roles",
            status: CheckStatus::Pass,
            code: None,
        },
        DoctorCheck {
            id: "le-advertising",
            status: CheckStatus::Pass,
            code: None,
        },
        DoctorCheck {
            id: "existing-pairing",
            status: CheckStatus::Warn,
            code: Some("pairing-not-configured"),
        },
        DoctorCheck {
            id: "wireplumber",
            status: CheckStatus::Warn,
            code: Some("wireplumber-optional-unavailable"),
        },
        DoctorCheck {
            id: "ancs-readiness",
            status: CheckStatus::Warn,
            code: Some("ancs-not-configured"),
        },
    ]);
    assert_eq!(line(&warning), fixture("doctor-warn.json"));

    let failing = DoctorOutput::new(vec![
        DoctorCheck {
            id: "bluez-version",
            status: CheckStatus::Fail,
            code: Some("bluez-unavailable"),
        },
        DoctorCheck {
            id: "adapter-power",
            status: CheckStatus::Fail,
            code: Some("adapter-not-found"),
        },
        DoctorCheck {
            id: "adapter-roles",
            status: CheckStatus::Fail,
            code: Some("adapter-not-found"),
        },
        DoctorCheck {
            id: "le-advertising",
            status: CheckStatus::Fail,
            code: Some("adapter-not-found"),
        },
        DoctorCheck {
            id: "existing-pairing",
            status: CheckStatus::Fail,
            code: Some("configured-pairing-missing"),
        },
        DoctorCheck {
            id: "wireplumber",
            status: CheckStatus::Fail,
            code: Some("wireplumber-required-unavailable"),
        },
        DoctorCheck {
            id: "ancs-readiness",
            status: CheckStatus::Warn,
            code: Some("ancs-unavailable"),
        },
    ]);
    assert_eq!(line(&failing), fixture("doctor-fail.json"));
}

#[test]
fn every_setup_event_and_stable_error_pair_matches_v1_fixtures() {
    let mut events = vec![
        SetupEvent::state(SetupState::CheckingEnvironment),
        SetupEvent::state(SetupState::WaitingForIphone),
        SetupEvent::state(SetupState::VerifyingAncs),
        SetupEvent::state(SetupState::ApplyingConfiguration),
    ];
    for (kind, passkey) in [
        (ConfirmationKind::Pairing, Some("123456".into())),
        (ConfirmationKind::ExistingBond, None),
    ] {
        events.push(SetupEvent::ConfirmationRequest {
            v: API_VERSION,
            kind,
            request_id: "opaque-id".into(),
            device_name: "iPhone".into(),
            address: "AA:BB:CC:DD:EE:FF".into(),
            passkey,
        });
    }
    events.push(SetupEvent::Complete {
        v: API_VERSION,
        address: "AA:BB:CC:DD:EE:FF".into(),
    });
    assert_eq!(
        events.iter().map(line).collect::<String>(),
        fixture("setup-events.jsonl")
    );
    assert_eq!(
        SetupFailure::ALL
            .iter()
            .map(|failure| line(&SetupEvent::error(*failure)))
            .collect::<String>(),
        fixture("setup-errors.jsonl")
    );
    assert_eq!(
        line(&SetupEvent::error(SetupFailure::InvalidProtocol)),
        fixture("setup-malformed-input-error.json")
    );
}

fn command(label: &str) -> (Command, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "ancs-bridge-lifecycle-{label}-{}-{}",
        std::process::id(),
        DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let mut command = Command::new(BINARY);
    command
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("HOME", root.join("home"));
    (command, root)
}

#[test]
fn setup_subprocess_flushes_jsonl_and_separates_stderr() {
    let (mut command, root) = command("flush");
    let mut child = command
        .args(["setup", "--jsonl"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    stdout.read_line(&mut first).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&first).unwrap()["state"],
        "checking-environment"
    );
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"{\"v\":1,\"command\":\"cancel\"}\n");
    }
    let output = child.wait_with_output().unwrap();
    let mut machine = first;
    stdout.read_to_string(&mut machine).unwrap();
    assert!(!output.status.success());
    assert!(machine
        .lines()
        .all(|line| serde_json::from_str::<serde_json::Value>(line).is_ok()));
    assert!(!String::from_utf8(output.stderr).unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn setup_subprocess_rejects_unsupported_api_cancel_and_stdin_closure() {
    for (label, input, expected) in [
        (
            "unsupported",
            Some("{\"v\":2,\"command\":\"cancel\"}\n"),
            "unsupported-api-version",
        ),
        (
            "cancel",
            Some("{\"v\":1,\"command\":\"cancel\"}\n"),
            "cancelled",
        ),
        ("closed", None, "stdin-closed"),
    ] {
        let (mut command, root) = command(label);
        let mut child = command
            .args(["setup", "--jsonl"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(input) = input {
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
        }
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events[0]["state"], "checking-environment");
        assert_eq!(events.last().unwrap()["code"], expected);
        assert!(!output.stderr.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn doctor_is_one_json_line_and_empty_teardown_has_no_stdout() {
    let (mut doctor, root) = command("doctor");
    let output = doctor.args(["doctor", "--json"]).output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["apiVersion"], 1);
    assert_eq!(value["checks"].as_array().unwrap().len(), 7);

    let mut teardown = Command::new(BINARY);
    let output = teardown
        .args(["teardown"])
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("HOME", root.join("home"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    fs::remove_dir_all(root).unwrap();
}
