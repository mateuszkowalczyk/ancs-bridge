mod ancs;
mod hid;

use ancs::{EventKind, NotificationEvent, ResponseAssembler};
use anyhow::{anyhow, bail, Context, Result};
use bluer::{
    adv::Advertisement,
    agent::{Agent, ReqError as AgentReqError},
    gatt::{
        remote::{Characteristic, CharacteristicWriteRequest},
        WriteOp,
    },
    Adapter, Address, Device, Session, Uuid,
};
use futures::{FutureExt, StreamExt};
use notify_rust::Notification;
use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    time::Duration,
};
use tokio::time::{sleep, timeout, Instant};

const ANCS_SERVICE_UUID: Uuid = Uuid::from_u128(0x7905f431_b5ce_4e99_a40f_4b1e122d00d0);
const NOTIFICATION_SOURCE_UUID: Uuid = Uuid::from_u128(0x9fbf120d_6301_42d9_8c58_25e699a21dbd);
const CONTROL_POINT_UUID: Uuid = Uuid::from_u128(0x69d1d8f3_45e1_49a8_9821_9bbdfdaad9d9);
const DATA_SOURCE_UUID: Uuid = Uuid::from_u128(0x22eac6e9_24d6_4bb5_be44_b36ace7c7bfb);

const DEVICE_WAIT: Duration = Duration::from_secs(5 * 60);
const ATTRIBUTE_WAIT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug)]
enum Mode {
    Probe,
    Fresh(Option<Address>),
    Reuse(Address),
    Reconnect(Address),
}

#[derive(Debug)]
struct Options {
    adapter_name: Option<String>,
    mode: Mode,
}

#[derive(Clone, Copy)]
struct AdapterSnapshot {
    pairable: bool,
    discoverable: bool,
}

impl AdapterSnapshot {
    async fn capture(adapter: &Adapter) -> Result<Self> {
        Ok(Self {
            pairable: adapter.is_pairable().await?,
            discoverable: adapter.is_discoverable().await?,
        })
    }

    async fn restore(self, adapter: &Adapter) -> Result<()> {
        let pairable_result = adapter.set_pairable(self.pairable).await;
        let discoverable_result = adapter.set_discoverable(self.discoverable).await;
        pairable_result.context("restoring adapter Pairable state")?;
        discoverable_result.context("restoring adapter Discoverable state")?;
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("spike failed: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let options = parse_options()?;
    let session = Session::new()
        .await
        .context("connecting to the BlueZ system bus")?;
    let adapter = match options.adapter_name {
        Some(name) => session.adapter(&name)?,
        None => session.default_adapter().await?,
    };

    print_adapter_probe(&adapter).await?;
    if matches!(options.mode, Mode::Probe) {
        return Ok(());
    }
    if !adapter.is_powered().await? {
        bail!(
            "adapter {} is powered off; the spike will not force Bluetooth power",
            adapter.name()
        );
    }

    let restore_state = if matches!(options.mode, Mode::Fresh(_)) {
        Some(AdapterSnapshot::capture(&adapter).await?)
    } else {
        None
    };

    let active_result = run_active(&session, &adapter, options.mode).await;
    let restore_result = match restore_state {
        Some(snapshot) => snapshot.restore(&adapter).await,
        None => Ok(()),
    };

    match (active_result, restore_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(active), Ok(())) => Err(active),
        (Ok(()), Err(restore)) => Err(restore),
        (Err(active), Err(restore)) => {
            Err(active.context(format!("adapter restoration also failed: {restore:#}")))
        }
    }
}

async fn run_active(session: &Session, adapter: &Adapter, mode: Mode) -> Result<()> {
    let initially_paired = paired_addresses(adapter).await?;

    let _gatt_handle = adapter
        .serve_gatt_application(hid::application())
        .await
        .context("registering the disposable HID-over-GATT application")?;

    let discoverable = matches!(mode, Mode::Fresh(_));
    let advertisement = Advertisement {
        service_uuids: [hid::HID_SERVICE_UUID].into_iter().collect(),
        solicit_uuids: [ANCS_SERVICE_UUID].into_iter().collect(),
        discoverable: Some(discoverable),
        local_name: Some("ANCS Bridge Spike".to_owned()),
        appearance: Some(0x03c1),
        ..Default::default()
    };
    let _advertisement_handle = adapter
        .advertise(advertisement)
        .await
        .context("registering the HID/ANCS LE advertisement")?;

    let _agent_handle = if let Mode::Fresh(expected_address) = mode {
        adapter
            .set_pairable(true)
            .await
            .context("making the adapter temporarily pairable")?;
        adapter
            .set_discoverable(true)
            .await
            .context("making the adapter temporarily discoverable")?;
        Some(
            session
                .register_agent(pairing_agent(expected_address))
                .await
                .context("registering the temporary pairing agent")?,
        )
    } else {
        None
    };

    eprintln!(
        "HID service and ANCS solicitation are registered on {}.",
        adapter.name()
    );
    let address = match mode {
        Mode::Fresh(expected_address) => {
            eprintln!("On the iPhone, open Settings > Bluetooth and select 'ANCS Bridge Spike'.");
            wait_for_fresh_paired_device(adapter, &initially_paired, expected_address).await?
        }
        Mode::Reuse(address) | Mode::Reconnect(address) => {
            eprintln!(
                "Waiting for the bonded iPhone {address} to connect without Device1.Connect()."
            );
            wait_for_bonded_connection(adapter, address).await?;
            address
        }
        Mode::Probe => unreachable!("probe returns before registrations"),
    };

    let device = adapter.device(address)?;
    device
        .set_trusted(true)
        .await
        .context("marking the explicitly confirmed iPhone trusted")?;
    let name = device.name().await?.unwrap_or_else(|| "unknown".to_owned());
    eprintln!("Using paired device {address} ({name}); waiting for authorized ANCS.");
    forward_one_notification(&device).await?;

    if matches!(mode, Mode::Reconnect(_)) {
        eprintln!(
            "First notification delivered. Turn off iPhone Bluetooth or move it out of range."
        );
        wait_for_connected_state(&device, false, DEVICE_WAIT).await?;
        eprintln!("Disconnect observed. Turn iPhone Bluetooth back on; no generic connect call will be made.");
        wait_for_connected_state(&device, true, DEVICE_WAIT).await?;
        eprintln!("Reconnect observed; waiting for services and a second notification.");
        forward_one_notification(&device).await?;
    }

    eprintln!(
        "Spike run completed for device {address}. Temporary BlueZ objects will now be dropped."
    );
    Ok(())
}

async fn print_adapter_probe(adapter: &Adapter) -> Result<()> {
    eprintln!("adapter.name={}", adapter.name());
    eprintln!("adapter.address={}", adapter.address().await?);
    eprintln!("adapter.powered={}", adapter.is_powered().await?);
    eprintln!("adapter.pairable={}", adapter.is_pairable().await?);
    eprintln!("adapter.discoverable={}", adapter.is_discoverable().await?);
    Ok(())
}

async fn paired_addresses(adapter: &Adapter) -> Result<HashSet<Address>> {
    let mut paired = HashSet::new();
    for address in adapter.device_addresses().await? {
        let device = adapter.device(address)?;
        if device.is_paired().await.unwrap_or(false) {
            paired.insert(address);
        }
    }
    Ok(paired)
}

async fn wait_for_fresh_paired_device(
    adapter: &Adapter,
    initially_paired: &HashSet<Address>,
    expected_address: Option<Address>,
) -> Result<Address> {
    let deadline = Instant::now() + DEVICE_WAIT;
    let mut observed_candidates = HashMap::new();
    loop {
        if Instant::now() >= deadline {
            bail!("timed out waiting for a fresh iPhone pairing");
        }
        for address in adapter.device_addresses().await? {
            if initially_paired.contains(&address)
                || expected_address.is_some_and(|expected| address != expected)
            {
                continue;
            }
            let device = adapter.device(address)?;
            let paired = device.is_paired().await.unwrap_or(false);
            let connected = device.is_connected().await.unwrap_or(false);
            let services_resolved = device.is_services_resolved().await.unwrap_or(false);
            let state = (paired, connected, services_resolved);
            if observed_candidates.get(&address) != Some(&state) {
                observed_candidates.insert(address, state);
                let name = device
                    .name()
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "unknown".to_owned());
                eprintln!(
                    "Observed pairing candidate {address} ({name}): paired={paired}, connected={connected}, services_resolved={services_resolved}"
                );
            }
            if paired && connected {
                return Ok(address);
            }
        }
        wait_or_cancel(POLL_INTERVAL).await?;
    }
}

async fn wait_for_bonded_connection(adapter: &Adapter, address: Address) -> Result<()> {
    let device = adapter.device(address)?;
    if !device
        .is_paired()
        .await
        .context("checking the requested device pairing")?
    {
        bail!("device {address} is not paired");
    }
    wait_for_connected_state(&device, true, DEVICE_WAIT).await
}

async fn wait_for_connected_state(device: &Device, wanted: bool, duration: Duration) -> Result<()> {
    let deadline = Instant::now() + duration;
    loop {
        if device.is_connected().await.unwrap_or(false) == wanted {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for device connected={wanted}");
        }
        wait_or_cancel(POLL_INTERVAL).await?;
    }
}

async fn wait_or_cancel(duration: Duration) -> Result<()> {
    tokio::select! {
        () = sleep(duration) => Ok(()),
        signal = tokio::signal::ctrl_c() => {
            signal.context("installing Ctrl-C handler")?;
            bail!("cancelled by user")
        }
    }
}

struct AncsCharacteristics {
    notification_source: Characteristic,
    data_source: Characteristic,
    control_point: Characteristic,
}

async fn discover_ancs(device: &Device) -> Result<AncsCharacteristics> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let services = timeout(Duration::from_secs(45), device.services())
            .await
            .context("timed out waiting for ServicesResolved and ANCS authorization")??;
        let mut notification_source = None;
        let mut data_source = None;
        let mut control_point = None;

        for service in services {
            if service.uuid().await? != ANCS_SERVICE_UUID {
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
            return Ok(AncsCharacteristics {
                notification_source,
                data_source,
                control_point,
            });
        }
        if !device.is_connected().await.unwrap_or(false) {
            bail!("iPhone disconnected while waiting for ANCS publication");
        }
        if Instant::now() >= deadline {
            bail!("ANCS and its three required characteristics were not exposed within 60 seconds");
        }
        eprintln!("ANCS is not exposed yet; waiting for iOS authorization/publication.");
        wait_or_cancel(Duration::from_secs(1)).await?;
    }
}

async fn forward_one_notification(device: &Device) -> Result<()> {
    let characteristics = discover_ancs(device).await?;
    let mut data_notifications = characteristics
        .data_source
        .notify()
        .await
        .context("subscribing to ANCS Data Source")?
        .boxed();
    eprintln!("ANCS Data Source subscribed.");
    let mut source_notifications = characteristics
        .notification_source
        .notify()
        .await
        .context("subscribing to ANCS Notification Source")?
        .boxed();
    eprintln!("ANCS Notification Source subscribed. Send a new iPhone notification now.");

    loop {
        let raw_event = tokio::select! {
            event = source_notifications.next() => event.context("ANCS Notification Source subscription ended")?,
            signal = tokio::signal::ctrl_c() => {
                signal.context("installing Ctrl-C handler")?;
                bail!("cancelled by user");
            }
        };
        let event = match NotificationEvent::parse(&raw_event) {
            Ok(event) => event,
            Err(error) => {
                eprintln!("Discarded malformed ANCS notification event: {error}");
                continue;
            }
        };
        eprintln!(
            "Received ANCS {:?} event: uid={}, flags=0x{:02x}, category={}, count={}.",
            event.kind, event.uid, event.flags, event.category_id, event.category_count
        );
        if event.kind == EventKind::Removed || event.is_pre_existing() {
            continue;
        }

        let request = ancs::notification_attributes_request(event.uid);
        characteristics
            .control_point
            .write_ext(
                &request,
                &CharacteristicWriteRequest {
                    op_type: WriteOp::Request,
                    ..Default::default()
                },
            )
            .await
            .context("writing Get Notification Attributes to ANCS Control Point")?;

        let attributes = timeout(ATTRIBUTE_WAIT, async {
            let mut assembler = ResponseAssembler::default();
            loop {
                let fragment = data_notifications
                    .next()
                    .await
                    .context("ANCS Data Source subscription ended")?;
                if let Some(attributes) = assembler.push(&fragment, event.uid)? {
                    return Ok::<_, anyhow::Error>(attributes);
                }
            }
        })
        .await
        .context("timed out waiting for ANCS notification attributes")??;

        let summary = if attributes.title.is_empty() {
            attributes.app_identifier.as_str()
        } else {
            attributes.title.as_str()
        };
        Notification::new()
            .appname("ancs-bridge-spike")
            .summary(summary)
            .body(&attributes.message)
            .show()
            .context("delivering the Freedesktop notification")?;
        eprintln!(
            "Delivered one Freedesktop notification for ANCS UID {} (content not logged).",
            attributes.uid
        );
        return Ok(());
    }
}

fn pairing_agent(expected_address: Option<Address>) -> Agent {
    let confirmation_address = expected_address;
    let authorization_address = expected_address;
    let service_address = expected_address;
    Agent {
        request_default: true,
        request_confirmation: Some(Box::new(move |request| {
            async move {
                if confirmation_address == Some(request.device) {
                    eprintln!(
                        "Auto-confirming expected device {} with passkey {:06}.",
                        request.device, request.passkey
                    );
                    return Ok(());
                }
                let prompt = format!(
                    "Confirm pairing from {} with passkey {:06}? [y/N] ",
                    request.device, request.passkey
                );
                approve(prompt).await
            }
            .boxed()
        })),
        request_authorization: Some(Box::new(move |request| {
            async move {
                if authorization_address == Some(request.device) {
                    eprintln!("Auto-authorizing expected device {}.", request.device);
                    Ok(())
                } else {
                    approve(format!("Authorize pairing from {}? [y/N] ", request.device)).await
                }
            }
            .boxed()
        })),
        authorize_service: Some(Box::new(move |request| {
            async move {
                if service_address == Some(request.device) {
                    eprintln!(
                        "Auto-authorizing service {} for expected device {}.",
                        request.service, request.device
                    );
                    Ok(())
                } else {
                    approve(format!(
                        "Authorize Bluetooth service {} from {}? [y/N] ",
                        request.service, request.device
                    ))
                    .await
                }
            }
            .boxed()
        })),
        ..Default::default()
    }
}

async fn approve(prompt: String) -> std::result::Result<(), AgentReqError> {
    let accepted = tokio::task::spawn_blocking(move || prompt_yes_no(&prompt))
        .await
        .unwrap_or(false);
    if accepted {
        Ok(())
    } else {
        Err(AgentReqError::Rejected)
    }
}

fn prompt_yes_no(prompt: &str) -> bool {
    eprint!("{prompt}");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn parse_options() -> Result<Options> {
    let mut args = std::env::args().skip(1).peekable();
    let mut adapter_name = None;
    if args.peek().map(String::as_str) == Some("--adapter") {
        args.next();
        adapter_name = Some(
            args.next()
                .context("--adapter requires a BlueZ adapter name")?,
        );
    }
    let command = args.next().unwrap_or_else(|| "fresh".to_owned());
    let mode = match command.as_str() {
        "probe" => Mode::Probe,
        "fresh" => Mode::Fresh(
            args.next()
                .map(|value| value.parse())
                .transpose()
                .context("invalid expected Bluetooth device address")?,
        ),
        "reuse" => Mode::Reuse(parse_address(args.next(), "reuse")?),
        "reconnect" => Mode::Reconnect(parse_address(args.next(), "reconnect")?),
        "help" | "--help" | "-h" => {
            print_usage();
            std::process::exit(0);
        }
        other => return Err(anyhow!("unknown command {other:?}")),
    };
    if let Some(extra) = args.next() {
        return Err(anyhow!("unexpected argument {extra:?}"));
    }
    Ok(Options { adapter_name, mode })
}

fn parse_address(value: Option<String>, command: &str) -> Result<Address> {
    value
        .with_context(|| format!("{command} requires a Bluetooth device address"))?
        .parse()
        .context("invalid Bluetooth device address")
}

fn print_usage() {
    eprintln!(
        "Usage: ancs-feasibility-spike [--adapter hci0] <command>\n\
         Commands:\n\
           probe                 Read adapter state without changing it\n\
           fresh [ADDRESS]       Require a new bond; auto-confirm only ADDRESS when supplied\n\
           reuse ADDRESS         Test daemon restart with an existing bond\n\
           reconnect ADDRESS     Deliver, observe disconnect/reconnect, deliver again"
    );
}
