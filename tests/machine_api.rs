use ancs_bridge::status::{RuntimeState, RuntimeStatus, StatusOutput, MACHINE_API_VERSION};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const BINARY: &str = env!("CARGO_BIN_EXE_ancs-bridge");

struct TestEnvironment {
    root: PathBuf,
    config_home: PathBuf,
    runtime_directory: PathBuf,
}

impl TestEnvironment {
    fn new(label: &str) -> Self {
        let sequence = DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ancs-bridge-cli-{label}-{}-{sequence}",
            std::process::id()
        ));
        let config_home = root.join("config");
        let runtime_directory = root.join("runtime");
        fs::create_dir_all(&config_home).unwrap();
        fs::create_dir_all(&runtime_directory).unwrap();
        Self {
            root,
            config_home,
            runtime_directory,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(BINARY);
        command
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_RUNTIME_DIR", &self.runtime_directory)
            .env("HOME", self.root.join("home"));
        command
    }

    fn config_path(&self) -> PathBuf {
        self.config_home.join("ancs-bridge/config.toml")
    }

    fn status_path(&self) -> PathBuf {
        self.runtime_directory.join("ancs-bridge/status.json")
    }

    fn write_config(&self) {
        fs::create_dir_all(self.config_path().parent().unwrap()).unwrap();
        fs::write(
            self.config_path(),
            concat!(
                "schema_version = 1\n",
                "\n[bluetooth]\n",
                "adapter = \"hci0\"\n",
                "device_address = \"AA:BB:CC:DD:EE:FF\"\n",
                "device_name = \"iPhone\"\n"
            ),
        )
        .unwrap();
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.command().args(arguments).output().unwrap()
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/machine-api-v1")
            .join(name),
    )
    .unwrap()
}

fn fixed_status(stale: bool) -> StatusOutput {
    StatusOutput {
        status: RuntimeStatus {
            api_version: MACHINE_API_VERSION,
            state: RuntimeState::Ready,
            reason_code: None,
            adapter: Some("hci0".into()),
            adapter_address: None,
            device_address: Some("AA:BB:CC:DD:EE:FF".into()),
            device_name: Some("iPhone".into()),
            connected: true,
            services_resolved: true,
            ancs_available: true,
            subscribed: true,
            last_error_code: None,
            last_transition_at: Some("2026-08-19T10:00:00Z".into()),
            last_notification_at: Some("2026-08-19T10:01:00Z".into()),
            pid: Some(1234),
        },
        stale,
    }
}

#[test]
fn committed_v1_fixtures_match_serialized_contracts() {
    assert_eq!(
        format!("{}\n", serde_json::to_string(&fixed_status(false)).unwrap()),
        fixture("status-ready-live.json")
    );
    assert_eq!(
        format!("{}\n", serde_json::to_string(&fixed_status(true)).unwrap()),
        fixture("status-ready-stale.json")
    );
}

#[test]
fn version_and_synthesized_status_commands_match_golden_fixtures() {
    let environment = TestEnvironment::new("golden");
    let version = environment.run(&["version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        fixture("version.json")
    );
    assert!(version.stderr.is_empty());

    let unconfigured = environment.run(&["status"]);
    assert!(unconfigured.status.success());
    assert_eq!(
        String::from_utf8(unconfigured.stdout).unwrap(),
        fixture("status-unconfigured.json")
    );
    assert!(unconfigured.stderr.is_empty());

    environment.write_config();
    let not_running = environment.run(&["status"]);
    assert!(not_running.status.success());
    assert_eq!(
        String::from_utf8(not_running.stdout).unwrap(),
        fixture("status-not-running.json")
    );
    assert!(not_running.stderr.is_empty());
}

#[test]
fn stale_status_preserves_last_snapshot_and_ignores_additive_fields() {
    let environment = TestEnvironment::new("stale");
    environment.write_config();
    fs::create_dir_all(environment.status_path().parent().unwrap()).unwrap();
    let mut status: Value = serde_json::from_str(&fixture("status-ready-stale.json")).unwrap();
    status.as_object_mut().unwrap().remove("stale");
    status["futureField"] = Value::String("ignored".into());
    fs::write(
        environment.status_path(),
        format!("{}\n", serde_json::to_string(&status).unwrap()),
    )
    .unwrap();

    let output = environment.run(&["status"]);
    assert!(output.status.success());
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "ready");
    assert_eq!(output["lastNotificationAt"], "2026-08-19T10:01:00Z");
    assert_eq!(output["stale"], true);
    assert!(output.get("futureField").is_none());
}

#[test]
fn failures_keep_stdout_empty_and_diagnostics_on_stderr() {
    let environment = TestEnvironment::new("failures");
    fs::create_dir_all(environment.config_path().parent().unwrap()).unwrap();
    fs::write(environment.config_path(), "not = [valid").unwrap();
    let malformed = environment.run(&["status"]);
    assert!(!malformed.status.success());
    assert!(malformed.stdout.is_empty());
    assert!(String::from_utf8(malformed.stderr)
        .unwrap()
        .contains("parsing configuration"));

    let obsolete_flag = environment.run(&["version", "--json"]);
    assert!(!obsolete_flag.status.success());
    assert!(obsolete_flag.stdout.is_empty());
    assert!(String::from_utf8(obsolete_flag.stderr)
        .unwrap()
        .contains("unexpected argument '--json'"));
}

#[test]
fn live_daemon_is_not_stale_and_becomes_stale_after_exit() {
    let environment = TestEnvironment::new("live");
    environment.write_config();
    let mut daemon = spawn_daemon(&environment);
    wait_for_file(&environment.status_path(), &mut daemon);

    let live = environment.run(&["status"]);
    assert!(live.status.success());
    let live: Value = serde_json::from_slice(&live.stdout).unwrap();
    assert_eq!(live["stale"], false);
    assert_eq!(live["adapter"], "hci0");
    assert_eq!(live["deviceAddress"], "AA:BB:CC:DD:EE:FF");

    daemon.kill().unwrap();
    daemon.wait().unwrap();
    let stale = environment.run(&["status"]);
    assert!(stale.status.success());
    let stale: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(stale["stale"], true);
    assert_eq!(stale["pid"], live["pid"]);
}

fn spawn_daemon(environment: &TestEnvironment) -> Child {
    environment
        .command()
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_for_file(path: &Path, daemon: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        assert!(
            daemon.try_wait().unwrap().is_none(),
            "daemon exited before status write"
        );
        thread::sleep(Duration::from_millis(20));
    }
    panic!("daemon did not publish status within five seconds");
}
