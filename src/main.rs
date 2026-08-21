use ancs_bridge::{
    audio::config_home_from_environment,
    bluetooth::{supervisor::Supervisor, transport::BluerTransport},
    clock::TokioClock,
    config::ConfigurationStore,
    diagnostics::{diagnose, probe},
    notification::FreedesktopSink,
    service::SystemdUserServiceControl,
    setup::{production::BluerSetupBackend, SetupOptions, SetupProtocol, StdinCommandInput},
    status::{
        status_output, PersistentStatusWriter, ProcfsProcessChecker, StatusIdentity, StatusStore,
        MACHINE_API_VERSION,
    },
    teardown::{BluezBondCleanup, Teardown},
};
use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::io::{self, Write};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "ancs-bridge",
    version,
    about = "Read-only Apple Notification Center Service bridge"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the configured long-lived bridge.
    Daemon,
    /// Return the current machine-readable runtime status.
    Status,
    /// Return package and machine API versions.
    Version,
    /// Return stable environment and readiness diagnostics.
    Doctor,
    /// Configure one explicitly confirmed iPhone.
    Setup {
        #[arg(long)]
        repair: bool,
    },
    /// Remove bridge-owned configuration and optionally its exact bond.
    Teardown {
        #[arg(long)]
        forget_device: bool,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionOutput<'a> {
    api_version: u32,
    version: &'a str,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("ancs-bridge: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Daemon => daemon().await,
        Command::Status => status(),
        Command::Version => write_json(&VersionOutput {
            api_version: MACHINE_API_VERSION,
            version: env!("CARGO_PKG_VERSION"),
        }),
        Command::Doctor => doctor().await,
        Command::Setup { repair } => setup(repair).await,
        Command::Teardown { forget_device } => teardown(forget_device).await,
    }
}

async fn doctor() -> Result<()> {
    let store = ConfigurationStore::from_environment()?;
    let configuration = store.load()?;
    let services = SystemdUserServiceControl;
    let output = diagnose(&probe(configuration.as_ref(), &services).await?);
    write_json(&output)?;
    if output.ok {
        Ok(())
    } else {
        Err(anyhow!("one or more diagnostic checks failed"))
    }
}

async fn setup(repair: bool) -> Result<()> {
    let store = ConfigurationStore::from_environment()?;
    let configured = store.load()?;
    let services = Arc::new(SystemdUserServiceControl);
    let backend = BluerSetupBackend::new(store, configured, services);
    let mut protocol = SetupProtocol::new(backend, TokioClock::default());
    let mut input = StdinCommandInput::new();
    let mut output = tokio::io::stdout();
    let successful = protocol
        .run(&mut input, &mut output, SetupOptions { repair })
        .await;
    if successful {
        Ok(())
    } else {
        Err(anyhow!("setup did not complete"))
    }
}

async fn teardown(forget_device: bool) -> Result<()> {
    let lifecycle = Teardown::new(
        ConfigurationStore::from_environment()?,
        config_home_from_environment()?,
        Arc::new(SystemdUserServiceControl),
        Arc::new(BluezBondCleanup),
    );
    lifecycle.run(forget_device).await
}

async fn daemon() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init()
        .map_err(|error| anyhow!("initializing daemon tracing: {error}"))?;
    let configuration = ConfigurationStore::from_environment()?
        .load()?
        .context("ancs-bridge is not configured")?;
    let status_store = StatusStore::from_environment()?;
    let status_writer =
        PersistentStatusWriter::new(status_store, StatusIdentity::from(&configuration));
    let mut supervisor = Supervisor::new(
        BluerTransport::new(
            configuration.adapter,
            configuration.adapter_address,
            configuration.device_address,
        ),
        FreedesktopSink::default(),
        TokioClock::default(),
        status_writer,
    );
    tokio::select! {
        result = supervisor.run() => result,
        signal = daemon_shutdown_signal() => {
            signal?;
            supervisor.shutdown().await;
            Ok(())
        }
    }
}

async fn daemon_shutdown_signal() -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("registering SIGTERM handler")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.context("waiting for interrupt signal"),
        _ = terminate.recv() => Ok(()),
    }
}

fn status() -> Result<()> {
    let configuration = ConfigurationStore::from_environment()?.load()?;
    let status_store = if configuration.is_some() {
        Some(StatusStore::from_environment()?)
    } else {
        None
    };
    let output = status_output(
        configuration.as_ref(),
        status_store.as_ref(),
        &ProcfsProcessChecker,
    )?;
    write_json(&output)
}

fn write_json(value: &impl Serialize) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, value).context("serializing machine output")?;
    output.write_all(b"\n").context("writing machine output")?;
    Ok(())
}
