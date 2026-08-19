use ancs_bridge::{
    bluetooth::{supervisor::Supervisor, transport::BluerTransport},
    clock::TokioClock,
    notification::FreedesktopSink,
    status::TracingStatusWriter,
};
use anyhow::{anyhow, Result};
use bluer::Address;
use clap::{Parser, Subcommand};
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
    /// Run directly with explicit development inputs. Stable configuration is a later iteration.
    Daemon {
        #[arg(long)]
        adapter: String,
        #[arg(long)]
        device: Address,
    },
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        tracing::error!(error_code = "daemon-failed", error = %error, "daemon stopped");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init()
        .map_err(|error| anyhow!("initializing tracing: {error}"))?;
    let Cli { command } = Cli::parse();
    match command {
        Command::Daemon { adapter, device } => {
            let transport = BluerTransport::new(adapter, device);
            Supervisor::new(
                transport,
                FreedesktopSink::default(),
                TokioClock::default(),
                TracingStatusWriter,
            )
            .run()
            .await
        }
    }
}
