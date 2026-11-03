use std::process::ExitCode;

use app::{
    bootstrap::{server, telemetry},
    commands,
    config::Settings,
    scheduler,
};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "app", version, about = "Rust backend template")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP API.
    Serve,
    /// Run the RabbitMQ job worker.
    Worker,
    /// Run the cron scheduler.
    Scheduler,
    /// Apply database migrations and exit.
    Migrate,
    /// Create a verified administrator account.
    CreateAdmin {
        #[arg(long)]
        email: String,
        /// Prefer the ADMIN_PASSWORD env var so the password stays out of shell history.
        #[arg(long, env = "ADMIN_PASSWORD", hide_env_values = true)]
        password: String,
        #[arg(long, default_value = "Administrator")]
        name: String,
    },
    /// Probe /health/live on localhost (for container HEALTHCHECK).
    Healthcheck,
}

#[tokio::main]
async fn main() -> ExitCode {
    // One crypto backend for every TLS user (sqlx, lettre, lapin, aws-sdk, redis).
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        eprintln!("a rustls crypto provider was already installed");
    }

    let cli = Cli::parse();

    if matches!(cli.command, Command::Healthcheck) {
        return match commands::healthcheck().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{err:#}");
                ExitCode::FAILURE
            }
        };
    }

    let settings = match Settings::from_env() {
        Ok(s) => s,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };
    telemetry::init_tracing(&settings);

    let result = match cli.command {
        Command::Serve => match telemetry::init_metrics() {
            Ok(handle) => server::serve(settings, Some(handle)).await,
            Err(err) => Err(err),
        },
        Command::Worker => commands::worker(settings).await,
        Command::Scheduler => match telemetry::init_metrics() {
            Ok(_) => scheduler::run(settings).await,
            Err(err) => Err(err),
        },
        Command::Migrate => commands::migrate(settings).await,
        Command::CreateAdmin {
            email,
            password,
            name,
        } => commands::create_admin(settings, email, password, name).await,
        Command::Healthcheck => unreachable!("handled above"),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = ?err, "fatal");
            ExitCode::FAILURE
        }
    }
}
