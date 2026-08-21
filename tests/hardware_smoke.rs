use ancs_bridge::{
    bluetooth::{supervisor::Supervisor, transport::BluerTransport},
    clock::TokioClock,
    notification::FreedesktopSink,
    status::{RuntimeState, TracingStatusWriter},
};
use bluer::Address;
use std::{env, time::Duration};

/// Opt in explicitly. This uses production modules and never pairs, selects a
/// device, changes adapter power, or calls Device1.Connect().
#[tokio::test]
#[ignore = "requires ANCS_BRIDGE_SMOKE_ADAPTER and ANCS_BRIDGE_SMOKE_DEVICE plus a bonded iPhone"]
async fn bonded_iphone_ready_notification_and_passive_reconnect() {
    let _ = tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
    let adapter = env::var("ANCS_BRIDGE_SMOKE_ADAPTER").expect("set ANCS_BRIDGE_SMOKE_ADAPTER");
    let device: Address = env::var("ANCS_BRIDGE_SMOKE_DEVICE")
        .expect("set ANCS_BRIDGE_SMOKE_DEVICE")
        .parse()
        .expect("valid Bluetooth identity address");
    tokio::time::timeout(Duration::from_secs(15 * 60), async move {
        let mut supervisor = Supervisor::new(
            BluerTransport::new(adapter, None, device),
            FreedesktopSink::default(),
            TokioClock::default(),
            TracingStatusWriter,
        );

        wait_for_ready(&mut supervisor).await;
        eprintln!("hardware-smoke state=ready deliveredCount=0; send one new iPhone notification");
        wait_for_delivery(&mut supervisor, 1).await;
        eprintln!("hardware-smoke deliveredCount=1; turn iPhone Bluetooth off");
        wait_for_state(&mut supervisor, RuntimeState::WaitingForPhone).await;
        eprintln!("hardware-smoke state=waiting-for-phone; turn iPhone Bluetooth on");
        wait_for_ready(&mut supervisor).await;
        eprintln!("hardware-smoke state=ready-after-reconnect deliveredCount=1; send one new notification");
        wait_for_delivery(&mut supervisor, 2).await;
        eprintln!("hardware-smoke complete deliveredCount=2 passiveReconnect=true payloadLogged=false");
    })
    .await
    .expect("hardware smoke timed out");
}

type HardwareSupervisor =
    Supervisor<BluerTransport, FreedesktopSink, TokioClock, TracingStatusWriter>;

async fn wait_for_ready(supervisor: &mut HardwareSupervisor) {
    wait_for_state(supervisor, RuntimeState::Ready).await;
}

async fn wait_for_state(supervisor: &mut HardwareSupervisor, wanted: RuntimeState) {
    let mut last = None;
    loop {
        supervisor
            .reconcile_once()
            .await
            .expect("hardware reconciliation failed");
        let snapshot = supervisor.snapshot();
        if last != Some(snapshot.state) {
            eprintln!(
                "hardware-smoke state={:?} connected={} servicesResolved={} ancsAvailable={} subscribed={} reasonCode={:?}",
                snapshot.state,
                snapshot.connected,
                snapshot.services_resolved,
                snapshot.ancs_available,
                snapshot.subscribed,
                snapshot.reason_code,
            );
            last = Some(snapshot.state);
        }
        if supervisor.snapshot().state == wanted {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn wait_for_delivery(supervisor: &mut HardwareSupervisor, wanted: u64) {
    loop {
        supervisor
            .reconcile_once()
            .await
            .expect("hardware reconciliation failed");
        if supervisor.delivered_count() >= wanted {
            return;
        }
        if supervisor.snapshot().state == RuntimeState::Ready {
            let _ =
                tokio::time::timeout(Duration::from_secs(5), supervisor.handle_one_packet()).await;
        } else {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
