use ancs_bridge::{
    audio::AudioRule,
    config::{ConfigurationStore, ValidatedConfiguration},
};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

const BINARY: &str = env!("CARGO_BIN_EXE_ancs-bridge");
const SERVICE: &str = "ancs-bridge.service";
const READY_TIMEOUT: Duration = Duration::from_secs(180);
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Debug)]
struct AcceptanceSnapshot {
    config_bytes: Vec<u8>,
    config_digest: u64,
    bonds: Vec<String>,
    adapter_powered: bool,
    suppress_phone_audio: bool,
    audio_rule_bytes: Option<Vec<u8>>,
    audio_role_rule_bytes: Option<Vec<u8>>,
    wireplumber_healthy: bool,
    service_active: bool,
    service_enabled: bool,
    pid: u32,
    restart_count: u64,
    state: String,
    stale: bool,
    transition: Option<String>,
    notification: Option<String>,
    rss_kib: u64,
    file_descriptors: usize,
}

impl AcceptanceSnapshot {
    fn capture() -> Result<Self> {
        let store = ConfigurationStore::from_environment()?;
        let config_bytes = fs::read(store.path())
            .with_context(|| format!("reading {}", store.path().display()))?;
        let configuration = store
            .load()?
            .context("ancs-bridge must be configured before hardware acceptance")?;
        let status = status()?;
        let pid = json_u64(&status, "pid")? as u32;
        let audio_rule = AudioRule::from_environment(configuration.device_address)?;
        let audio_rule_bytes = match fs::read(audio_rule.path()) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("reading configured audio rule"),
        };
        let audio_role_rule_bytes = match fs::read(audio_rule.role_path()) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("reading Bluetooth role policy"),
        };
        let bonds = normalized_lines(&checked_stdout("bluetoothctl", &["devices", "Paired"])?);
        let adapter_path = resolved_adapter_path(&configuration)?;
        let adapter_powered = checked_stdout(
            "busctl",
            &[
                "--system",
                "get-property",
                "org.bluez",
                &adapter_path,
                "org.bluez.Adapter1",
                "Powered",
            ],
        )?
        .split_ascii_whitespace()
        .last()
            == Some("true");
        let service_active = systemctl_property("ActiveState")? == "active";
        let service_enabled = matches!(
            systemctl_property("UnitFileState")?.as_str(),
            "enabled" | "enabled-runtime"
        );
        let restart_count = systemctl_property("NRestarts")?.parse()?;
        let wireplumber_healthy = command_output(
            "systemctl",
            &["--user", "is-active", "--quiet", "wireplumber.service"],
        )?
        .status
        .success()
            && command_output("wpctl", &["status"])?.status.success();

        Ok(Self {
            config_digest: digest(&config_bytes),
            config_bytes,
            bonds,
            adapter_powered,
            suppress_phone_audio: configuration.suppress_phone_audio,
            audio_rule_bytes,
            audio_role_rule_bytes,
            wireplumber_healthy,
            service_active,
            service_enabled,
            pid,
            restart_count,
            state: json_string(&status, "state")?,
            stale: status
                .get("stale")
                .and_then(Value::as_bool)
                .context("status stale field is missing")?,
            transition: json_optional_string(&status, "lastTransitionAt")?,
            notification: json_optional_string(&status, "lastNotificationAt")?,
            rss_kib: process_rss_kib(pid)?,
            file_descriptors: fs::read_dir(format!("/proc/{pid}/fd"))?.count(),
        })
    }

    fn assert_ready(&self) -> Result<()> {
        if !self.service_active || !self.service_enabled || self.state != "ready" || self.stale {
            bail!(
                "service is not enabled, active, live, and ready: active={} enabled={} state={} stale={}",
                self.service_active,
                self.service_enabled,
                self.state,
                self.stale
            );
        }
        Ok(())
    }

    fn assert_invariants(&self, baseline: &Self) -> Result<()> {
        if self.config_bytes != baseline.config_bytes {
            bail!("configuration changed during acceptance");
        }
        if self.bonds != baseline.bonds {
            bail!("Bluetooth bond set changed during acceptance");
        }
        if self.suppress_phone_audio != baseline.suppress_phone_audio
            || self.audio_rule_bytes != baseline.audio_rule_bytes
            || self.audio_role_rule_bytes != baseline.audio_role_rule_bytes
        {
            bail!("phone-audio suppression intent or rule changed during acceptance");
        }
        if self.adapter_powered != baseline.adapter_powered {
            bail!("adapter power was not restored to its baseline value");
        }
        if !self.wireplumber_healthy {
            bail!("WirePlumber is not healthy after recovery");
        }
        Ok(())
    }

    fn report(&self, label: &str) {
        eprintln!(
            "acceptance snapshot={label} state={} stale={} pid={} nRestarts={} configDigest={:016x} bondCount={} adapterPowered={} audioSuppressed={} wireplumberHealthy={} rssKiB={} fdCount={} transitionPresent={} notificationPresent={}",
            self.state,
            self.stale,
            self.pid,
            self.restart_count,
            self.config_digest,
            self.bonds.len(),
            self.adapter_powered,
            self.suppress_phone_audio,
            self.wireplumber_healthy,
            self.rss_kib,
            self.file_descriptors,
            self.transition.is_some(),
            self.notification.is_some(),
        );
    }
}

fn resolved_adapter_path(configuration: &ValidatedConfiguration) -> Result<String> {
    let Some(identity) = configuration.adapter_address else {
        return Ok(format!("/org/bluez/{}", configuration.adapter));
    };
    let tree = checked_stdout("busctl", &["--system", "tree", "org.bluez"])?;
    let identity = identity.to_string();
    let mut matches = Vec::new();
    for path in direct_adapter_paths(&tree) {
        let address = checked_stdout(
            "busctl",
            &[
                "--system",
                "get-property",
                "org.bluez",
                path,
                "org.bluez.Adapter1",
                "Address",
            ],
        )?;
        if address
            .split_ascii_whitespace()
            .last()
            .map(|value| value.trim_matches('"'))
            == Some(identity.as_str())
        {
            matches.push((path.to_owned(), identity.clone()));
        }
    }
    matching_adapter_path(&identity, &matches)
        .map(str::to_owned)
        .context("configured Bluetooth adapter identity is missing or ambiguous in BlueZ")
}

fn direct_adapter_paths(tree: &str) -> impl Iterator<Item = &str> {
    tree.split_ascii_whitespace().filter(|value| {
        value
            .strip_prefix("/org/bluez/hci")
            .is_some_and(|suffix| !suffix.is_empty() && !suffix.contains('/'))
    })
}

fn matching_adapter_path<'a>(identity: &str, adapters: &'a [(String, String)]) -> Option<&'a str> {
    let matches = adapters
        .iter()
        .filter(|(_, address)| address == identity)
        .map(|(path, _)| path.as_str())
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

#[test]
#[ignore = "operator-gated physical iPhone and live user-service acceptance"]
fn service_runtime_reliability_acceptance() -> Result<()> {
    if env::var("ANCS_BRIDGE_ACCEPTANCE").as_deref() != Ok("1") {
        bail!("set ANCS_BRIDGE_ACCEPTANCE=1 to acknowledge disruptive live hardware testing");
    }
    let stage = env::var("ANCS_BRIDGE_ACCEPTANCE_STAGE").context(
        "set ANCS_BRIDGE_ACCEPTANCE_STAGE to baseline, notifications, lifecycle, service-restart, bluez-restart, adapter-cycle, iphone-cycle, range-loss, suspend, reboot, endurance, privacy, or final",
    )?;
    match stage.as_str() {
        "baseline" => baseline_stage(),
        "notifications" => notifications_stage(),
        "lifecycle" => lifecycle_stage(),
        "service-restart" => service_restart_stage(),
        "bluez-restart" => bluez_restart_stage(),
        "adapter-cycle" => adapter_cycle_stage(),
        "iphone-cycle" => iphone_cycle_stage(),
        "range-loss" => range_loss_stage(),
        "suspend" => suspend_stage(),
        "reboot" => reboot_stage(),
        "endurance" => endurance_stage(),
        "privacy" => privacy_stage(),
        "final" => final_stage(),
        _ => bail!("unknown hardware acceptance stage: {stage}"),
    }
}

fn baseline_stage() -> Result<()> {
    let snapshot = wait_for_ready(READY_TIMEOUT)?;
    snapshot.assert_ready()?;
    snapshot.report("baseline");
    let doctor = command_output(BINARY, &["doctor", "--json"])?;
    let value: Value = serde_json::from_slice(&doctor.stdout).context("parsing doctor output")?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        bail!("doctor did not report a passing baseline");
    }
    eprintln!("acceptance stage=baseline result=pass diagnostics=pass payloadLogged=false");
    Ok(())
}

fn notifications_stage() -> Result<()> {
    wait_for_ready(READY_TIMEOUT)?.assert_ready()?;
    for (previews, locked, source, source_class) in [
        ("Always", false, "TickTick", "ticktick"),
        ("Always", true, "Apple Reminders", "reminders"),
        ("When Unlocked", false, "TickTick", "ticktick"),
        ("When Unlocked", true, "Apple Reminders", "reminders"),
        ("Never", false, "TickTick", "ticktick"),
        ("Never", true, "Apple Reminders", "reminders"),
    ] {
        let before = current_notification()?;
        prompt(&format!(
            "Set iPhone notification previews to {previews}, leave the phone {}, send one new notification from {source}, then press Enter.",
            if locked { "locked" } else { "unlocked" }
        ))?;
        wait_for_new_notification(before, READY_TIMEOUT)?;
        confirm_once("Did exactly one corresponding notification appear on the desktop?")?;
        eprintln!(
            "acceptance stage=notifications previews={} phoneLocked={} sourceClass={} result=pass",
            previews.replace(' ', "-"),
            locked,
            source_class
        );
    }
    Ok(())
}

fn lifecycle_stage() -> Result<()> {
    wait_for_ready(READY_TIMEOUT)?.assert_ready()?;
    let before_added = current_notification()?;
    prompt("Create an Apple Reminder on the iPhone that becomes due in about one minute; leave its notification active after it appears, then press Enter.")?;
    wait_for_new_notification(before_added, READY_TIMEOUT)?;
    confirm_once("Did one new desktop notification appear?")?;

    let before_modified = current_notification()?;
    prompt("Open that synced reminder on the same-account iPad and edit its title while the iPhone notification remains active, then press Enter.")?;
    wait_for_new_notification(before_modified, READY_TIMEOUT)?;
    confirm_once("Was the existing desktop notification replaced rather than duplicated?")?;

    prompt("Clear that same notification from iPhone Notification Center, wait for the desktop copy to close, then press Enter.")?;
    confirm_once("Did the mapped desktop notification close?")?;
    eprintln!("acceptance stage=lifecycle added=pass modified=pass removed=pass duplicate=false payloadLogged=false");
    Ok(())
}

fn service_restart_stage() -> Result<()> {
    let baseline = wait_for_ready(READY_TIMEOUT)?;
    checked_status("systemctl", &["--user", "restart", SERVICE])?;
    let recovered = wait_for_ready(READY_TIMEOUT)?;
    if recovered.pid == baseline.pid {
        bail!("service restart did not produce a new daemon PID");
    }
    recovered.assert_invariants(&baseline)?;
    recovery_canary("service-restart", &baseline, &recovered)
}

fn bluez_restart_stage() -> Result<()> {
    let baseline = wait_for_ready(READY_TIMEOUT)?;
    prompt(
        "In another terminal run `sudo systemctl restart bluetooth.service`; after it finishes, return here and press Enter.",
    )?;
    let recovered = wait_for_ready(READY_TIMEOUT)?;
    if recovered.transition == baseline.transition {
        bail!("BlueZ restart did not produce an observed bridge state transition");
    }
    recovered.assert_invariants(&baseline)?;
    recovery_canary("bluez-restart", &baseline, &recovered)
}

fn adapter_cycle_stage() -> Result<()> {
    let baseline = wait_for_ready(READY_TIMEOUT)?;
    let configuration = ConfigurationStore::from_environment()?
        .load()?
        .context("ancs-bridge must be configured before adapter-cycle acceptance")?;
    let adapter_path = resolved_adapter_path(&configuration)?;
    prompt("This will power the selected Bluetooth adapter off. Press Enter to continue, or Ctrl-C to cancel.")?;
    set_adapter_powered(&adapter_path, false)?;
    wait_for_not_ready(DISCONNECT_TIMEOUT)?;
    prompt("The adapter is off. Press Enter to restore adapter power.")?;
    set_adapter_powered(&adapter_path, true)?;
    let recovered = wait_for_ready(READY_TIMEOUT)?;
    recovered.assert_invariants(&baseline)?;
    recovery_canary("adapter-cycle", &baseline, &recovered)
}

fn set_adapter_powered(adapter_path: &str, powered: bool) -> Result<()> {
    checked_status(
        "busctl",
        &[
            "--system",
            "set-property",
            "org.bluez",
            adapter_path,
            "org.bluez.Adapter1",
            "Powered",
            "b",
            if powered { "true" } else { "false" },
        ],
    )
}

fn iphone_cycle_stage() -> Result<()> {
    let baseline = wait_for_ready(READY_TIMEOUT)?;
    prompt("Turn iPhone Bluetooth off, then press Enter.")?;
    wait_for_not_ready(DISCONNECT_TIMEOUT)?;
    prompt("Turn iPhone Bluetooth on, then press Enter. Do not reopen the Omarchy device entry.")?;
    let recovered = wait_for_ready(READY_TIMEOUT)?;
    recovered.assert_invariants(&baseline)?;
    recovery_canary("iphone-cycle", &baseline, &recovered)
}

fn range_loss_stage() -> Result<()> {
    let baseline = wait_for_ready(READY_TIMEOUT)?;
    prompt("Move the iPhone far enough away to lose Bluetooth range, then press Enter.")?;
    wait_for_not_ready(Duration::from_secs(180))?;
    prompt(
        "Bring the iPhone back into range, then press Enter without opening Bluetooth Settings.",
    )?;
    let recovered = wait_for_ready(Duration::from_secs(300))?;
    recovered.assert_invariants(&baseline)?;
    recovery_canary("range-loss", &baseline, &recovered)
}

fn suspend_stage() -> Result<()> {
    let baseline = wait_for_ready(READY_TIMEOUT)?;
    prompt("Suspend the computer or close the lid. After resuming and unlocking the session, press Enter.")?;
    let recovered = wait_for_ready(Duration::from_secs(300))?;
    if recovered.pid != baseline.pid {
        bail!("daemon PID changed across suspend; the existing service process should reconcile");
    }
    recovered.assert_invariants(&baseline)?;
    recovery_canary("suspend", &baseline, &recovered)
}

fn reboot_stage() -> Result<()> {
    let snapshot = wait_for_ready(Duration::from_secs(300))?;
    snapshot.assert_ready()?;
    snapshot.report("post-reboot");
    notification_canary("post-reboot")?;
    eprintln!(
        "acceptance stage=reboot autoStarted=true ready=true result=pass payloadLogged=false"
    );
    Ok(())
}

fn endurance_stage() -> Result<()> {
    let baseline = wait_for_ready(READY_TIMEOUT)?;
    notification_canary("endurance-pre")?;
    prompt(
        "Start the prepared iPhone Shortcut that repeats Bluetooth off for 10 seconds and on for 30 seconds twenty times, then immediately press Enter here.",
    )?;
    let mut rss_samples = vec![baseline.rss_kib];
    let mut fd_samples = vec![baseline.file_descriptors];

    for cycle in 1..=20 {
        wait_for_not_ready(Duration::from_secs(60))?;
        let recovered = wait_for_ready(READY_TIMEOUT)?;
        recovered.assert_invariants(&baseline)?;
        rss_samples.push(recovered.rss_kib);
        fd_samples.push(recovered.file_descriptors);
        eprintln!(
            "acceptance stage=endurance cycle={cycle} state=ready rssKiB={} fdCount={} result=pass",
            recovered.rss_kib, recovered.file_descriptors
        );
    }

    notification_canary("endurance-post")?;
    if fd_samples
        .iter()
        .skip(1)
        .any(|count| *count > baseline.file_descriptors + 2)
    {
        bail!("file descriptor count did not return to the post-warm-up baseline range");
    }
    if sustained_monotonic_growth(&rss_samples[5..], 6) {
        bail!("RSS showed sustained monotonic growth after warm-up");
    }
    if rss_samples.last().copied().unwrap_or_default() > baseline.rss_kib + 16 * 1024 {
        bail!("final RSS exceeded the post-warm-up baseline by more than 16 MiB");
    }
    eprintln!(
        "acceptance stage=endurance cycles=20 disruption=iphone-bluetooth-shortcut passiveReconnect=true genericConnect=false maxFd={} rssStartKiB={} rssEndKiB={} result=pass payloadLogged=false",
        fd_samples.into_iter().max().unwrap_or_default(),
        rss_samples[0],
        rss_samples.last().copied().unwrap_or_default()
    );
    Ok(())
}

fn privacy_stage() -> Result<()> {
    wait_for_ready(READY_TIMEOUT)?.assert_ready()?;
    let canary = secret_line(
        "Choose a unique text canary, type it here without echo, and press Enter. It will remain in test-process memory only: ",
    )?;
    if canary.trim().len() < 12 {
        bail!("privacy canary must contain at least 12 characters");
    }
    let before = current_notification()?;
    prompt("Send a TickTick notification containing that exact canary, then press Enter.")?;
    wait_for_new_notification(before, READY_TIMEOUT)?;

    let store = ConfigurationStore::from_environment()?;
    let mut surfaces = vec![
        ("configuration".to_owned(), fs::read(store.path())?),
        (
            "status-command".to_owned(),
            command_output(BINARY, &["status", "--json"])?.stdout,
        ),
        (
            "doctor-command".to_owned(),
            command_output(BINARY, &["doctor", "--json"])?.stdout,
        ),
        (
            "service-status".to_owned(),
            command_output("systemctl", &["--user", "status", "--no-pager", SERVICE])?.stdout,
        ),
        (
            "service-journal".to_owned(),
            command_output(
                "journalctl",
                &["--user-unit", SERVICE, "--boot", "--no-pager"],
            )?
            .stdout,
        ),
    ];
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .context("XDG_RUNTIME_DIR is required")?
        .join("ancs-bridge");
    append_files(&runtime, "runtime", &mut surfaces)?;
    for path in [
        Path::new("/usr/bin/ancs-bridge"),
        Path::new("/usr/lib/systemd/user/ancs-bridge.service"),
        Path::new("/usr/share/licenses/ancs-bridge/LICENSE"),
    ] {
        if path.exists() {
            surfaces.push((format!("installed:{}", path.display()), fs::read(path)?));
        }
    }
    append_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/machine-api-v1"),
        "setup-diagnostics-fixtures",
        &mut surfaces,
    )?;

    let needle = canary.trim().as_bytes();
    let bytes_scanned: usize = surfaces.iter().map(|(_, bytes)| bytes.len()).sum();
    for (label, bytes) in &surfaces {
        if bytes.windows(needle.len()).any(|window| window == needle) {
            bail!("privacy canary was retained in {label}");
        }
    }
    eprintln!(
        "acceptance stage=privacy surfaces={} bytesScanned={} matches=0 result=pass payloadLogged=false",
        surfaces.len(), bytes_scanned
    );
    Ok(())
}

fn final_stage() -> Result<()> {
    let snapshot = wait_for_ready(READY_TIMEOUT)?;
    snapshot.assert_ready()?;
    if !snapshot.adapter_powered || !snapshot.wireplumber_healthy {
        bail!("Bluetooth adapter or WirePlumber is not healthy");
    }
    let configuration = ConfigurationStore::from_environment()?
        .load()?
        .context("ancs-bridge is not configured")?;
    let audio_rule = AudioRule::from_environment(configuration.device_address)?;
    if !configuration.suppress_phone_audio
        || snapshot.audio_rule_bytes.as_deref() != Some(audio_rule.content().as_bytes())
        || snapshot.audio_role_rule_bytes.as_deref() != Some(audio_rule.role_content().as_bytes())
    {
        bail!("phone-audio suppression intent or canonical rules are missing");
    }
    let pipewire = checked_stdout("wpctl", &["status", "-n"])?;
    let identity = configuration.device_address.to_string().replace(':', "_");
    if pipewire.contains(&format!("bluez_output.{identity}"))
        || pipewire.contains(&format!("bluez_input.{identity}"))
    {
        bail!("configured iPhone has an active PipeWire audio node");
    }
    if let Some(id) = pipewire_object_id(&pipewire, &format!("bluez_card.{identity}")) {
        let device = checked_stdout("wpctl", &["inspect", &id])?;
        if !device.contains("bluez5.profile = \"off\"")
            || !device.contains("device.disabled = \"true\"")
        {
            bail!("configured iPhone PipeWire card has an active audio profile");
        }
    }
    let controller = configuration
        .adapter_address
        .map(|value| value.to_string())
        .unwrap_or_else(|| configuration.adapter.clone());
    let local = checked_stdout("bluetoothctl", &["show", &controller])?;
    for forbidden in [
        "0000110b-0000-1000-8000-00805f9b34fb",
        "0000111e-0000-1000-8000-00805f9b34fb",
    ] {
        if local.contains(forbidden) {
            bail!("Omarchy still advertises a phone-facing Bluetooth audio role");
        }
    }
    confirm_once("Is Omarchy absent from the iPhone audio-output picker?")?;
    confirm_once("Do AirPods playback and microphone both work now?")?;
    snapshot.report("final");
    eprintln!("acceptance stage=final serviceEnabled=true serviceReady=true phoneAudioNodesAbsent=true phoneAudioDestinationAbsent=true airpodsPlayback=true airpodsMicrophone=true result=pass payloadLogged=false");
    Ok(())
}

fn pipewire_object_id(status: &str, name: &str) -> Option<String> {
    status
        .lines()
        .find(|line| line.contains(name))
        .and_then(|line| {
            line.split_ascii_whitespace().find_map(|value| {
                value
                    .strip_suffix('.')
                    .and_then(|id| id.parse::<u32>().ok())
            })
        })
        .map(|id| id.to_string())
}

fn recovery_canary(
    stage: &str,
    baseline: &AcceptanceSnapshot,
    recovered: &AcceptanceSnapshot,
) -> Result<()> {
    recovered.assert_ready()?;
    recovered.report(stage);
    notification_canary(stage)?;
    eprintln!(
        "acceptance stage={stage} ready=true pidChanged={} genericConnect=false invariants=preserved result=pass payloadLogged=false",
        recovered.pid != baseline.pid
    );
    Ok(())
}

fn notification_canary(stage: &str) -> Result<()> {
    let before = current_notification()?;
    prompt(&format!(
        "Send one new notification to the iPhone for the {stage} check, then press Enter."
    ))?;
    wait_for_new_notification(before, READY_TIMEOUT)?;
    confirm_once("Did exactly one corresponding notification appear on the desktop?")
}

fn wait_for_ready(timeout: Duration) -> Result<AcceptanceSnapshot> {
    let deadline = Instant::now() + timeout;
    let mut last_state = String::new();
    loop {
        let value = status()?;
        let state = json_string(&value, "state")?;
        if state != last_state {
            eprintln!("acceptance observation state={state}");
            last_state = state.clone();
        }
        if state == "ready" && value.get("stale").and_then(Value::as_bool) == Some(false) {
            return AcceptanceSnapshot::capture();
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for ready; last state was {state}");
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn wait_for_not_ready(timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let value = status()?;
        if json_string(&value, "state")? != "ready"
            || value.get("stale").and_then(Value::as_bool) != Some(false)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for a disconnected or recovering state");
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn wait_for_new_notification(before: Option<String>, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let current = current_notification()?;
        if current.is_some() && current != before {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for metadata-only notification delivery timestamp");
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn current_notification() -> Result<Option<String>> {
    json_optional_string(&status()?, "lastNotificationAt")
}

fn status() -> Result<Value> {
    let output = command_output(BINARY, &["status", "--json"])?;
    if !output.status.success() {
        bail!("status command failed");
    }
    serde_json::from_slice(&output.stdout).context("parsing status JSON")
}

fn json_string(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("status {field} field is missing"))
}

fn json_optional_string(value: &Value, field: &str) -> Result<Option<String>> {
    match value.get(field) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(Value::Null) => Ok(None),
        _ => bail!("status {field} field is missing or invalid"),
    }
}

fn json_u64(value: &Value, field: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .with_context(|| format!("status {field} field is missing"))
}

fn systemctl_property(property: &str) -> Result<String> {
    Ok(checked_stdout(
        "systemctl",
        &["--user", "show", SERVICE, "--property", property, "--value"],
    )?
    .trim()
    .to_owned())
}

fn process_rss_kib(pid: u32) -> Result<u64> {
    let source = fs::read_to_string(format!("/proc/{pid}/status"))?;
    let line = source
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .context("VmRSS is missing from proc status")?;
    line.split_ascii_whitespace()
        .nth(1)
        .context("VmRSS value is missing")?
        .parse()
        .context("parsing VmRSS")
}

fn normalized_lines(source: &str) -> Vec<String> {
    let mut lines: Vec<_> = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

fn digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn sustained_monotonic_growth(samples: &[u64], window: usize) -> bool {
    samples.len() >= window
        && samples
            .windows(window)
            .any(|values| values.windows(2).all(|pair| pair[1] > pair[0]))
}

fn command_output(program: &str, arguments: &[&str]) -> Result<Output> {
    Command::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("running {program}"))
}

fn checked_stdout(program: &str, arguments: &[&str]) -> Result<String> {
    let output = command_output(program, arguments)?;
    if !output.status.success() {
        bail!(
            "{} failed: {}",
            program,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn checked_status(program: &str, arguments: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(arguments)
        .status()
        .with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("{program} exited unsuccessfully");
    }
    Ok(())
}

fn prompt(message: &str) -> Result<()> {
    eprint!("acceptance action: {message} ");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(())
}

fn confirm_once(message: &str) -> Result<()> {
    eprint!("acceptance confirmation: {message} Type yes: ");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    if !line.trim().eq_ignore_ascii_case("yes") {
        bail!("operator did not confirm the expected desktop behavior");
    }
    Ok(())
}

fn secret_line(message: &str) -> Result<String> {
    struct EchoGuard;
    impl Drop for EchoGuard {
        fn drop(&mut self) {
            let _ = Command::new("stty").arg("echo").status();
            eprintln!();
        }
    }

    eprint!("{message}");
    io::stderr().flush()?;
    let echo_disabled = Command::new("stty").arg("-echo").status()?.success();
    let _guard = echo_disabled.then_some(EchoGuard);
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line)
}

fn append_files(root: &Path, label: &str, surfaces: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            append_files(&path, label, surfaces)?;
        } else if path.is_file() {
            surfaces.push((format!("{label}:{}", path.display()), fs::read(path)?));
        }
    }
    Ok(())
}

#[test]
fn production_runtime_has_no_generic_bluez_connect_path() {
    let transport = include_str!("../src/bluetooth/transport.rs");
    assert!(!transport.contains(".connect("));
    assert!(!transport.contains("connect-device"));
    assert!(!transport.contains("Device1.Connect"));
}

#[test]
fn endurance_growth_detector_requires_a_sustained_run() {
    assert!(sustained_monotonic_growth(&[10, 11, 12, 13, 14, 15], 6));
    assert!(!sustained_monotonic_growth(&[10, 11, 12, 11, 13, 14], 6));
    assert!(!sustained_monotonic_growth(&[10, 11, 12], 6));
}

#[test]
fn adapter_resolution_ignores_children_and_rejects_missing_or_duplicate_identities() {
    let tree = "\
└─ /org\n\
  └─ /org/bluez\n\
    ├─ /org/bluez/hci0\n\
    │ └─ /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF\n\
    └─ /org/bluez/hci1\n\
      └─ /org/bluez/hci1/dev_11_22_33_44_55_66\n";
    assert_eq!(
        direct_adapter_paths(tree).collect::<Vec<_>>(),
        vec!["/org/bluez/hci0", "/org/bluez/hci1"]
    );

    let identity = "11:22:33:44:55:66";
    assert_eq!(
        matching_adapter_path(
            identity,
            &[
                ("/org/bluez/hci0".into(), "AA:BB:CC:DD:EE:FF".into()),
                ("/org/bluez/hci1".into(), identity.into()),
            ],
        ),
        Some("/org/bluez/hci1")
    );
    assert_eq!(matching_adapter_path(identity, &[]), None);
    assert_eq!(
        matching_adapter_path(
            identity,
            &[
                ("/org/bluez/hci0".into(), identity.into()),
                ("/org/bluez/hci1".into(), identity.into()),
            ],
        ),
        None
    );
}

#[test]
fn pipewire_card_id_is_extracted_without_matching_unrelated_devices() {
    let status = "│     109. bluez_card.C4_C1_7D_85_7A_55 [bluez5]\n│  *  110. bluez_card.F0_04_E1_E0_82_80 [bluez5]\n";
    assert_eq!(
        pipewire_object_id(status, "bluez_card.C4_C1_7D_85_7A_55").as_deref(),
        Some("109")
    );
    assert_eq!(
        pipewire_object_id(status, "bluez_card.F0_04_E1_E0_82_80").as_deref(),
        Some("110")
    );
    assert_eq!(pipewire_object_id(status, "bluez_card.missing"), None);
}
