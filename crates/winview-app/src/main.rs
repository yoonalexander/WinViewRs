// winview-app/src/main.rs

use anyhow::Result;
use clap::{Parser, Subcommand};
use winview_app::cli;
use winview_app::logging;

#[derive(Parser)]
#[command(
    name = "winview-app",
    version,
    about = "WinViewRs - Windows Explorer Configuration Inspector"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Inspect and display Explorer configuration as a human-readable table
    Inspect,
    /// Output full inspection snapshot as JSON
    Dump {
        /// Output format (json)
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Export inspection snapshot as TOML (lossy, for debugging)
    Export,
}

fn main() -> Result<()> {
    logging::init_logging();

    let cli = Cli::parse();

    match cli.command {
        Commands::Inspect => {
            cli::inspect::run_inspect()?;
        }
        Commands::Dump { format } => {
            if format == "json" {
                cli::inspect::run_dump_json()?;
            } else {
                eprintln!("Unsupported format: {}", format);
                std::process::exit(1);
            }
        }
        Commands::Export => {
            cli::output::run_export_toml()?;
        }
    }

    Ok(())
}
