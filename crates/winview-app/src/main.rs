// winview-app/src/main.rs

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::process::ExitCode;
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
    /// Launch the read-only desktop inspector
    #[cfg(feature = "gui")]
    Gui,
}

fn main() -> Result<ExitCode> {
    logging::init_logging();

    let cli = Cli::parse();

    match cli.command {
        Commands::Inspect => cli::inspect::run_inspect(),

        Commands::Dump { format } => {
            if format == "json" {
                cli::inspect::run_dump_json()
            } else {
                eprintln!("Unsupported format: {}", format);

                Ok(ExitCode::from(2))
            }
        }

        Commands::Export => cli::output::run_export_toml(),

        #[cfg(feature = "gui")]
        Commands::Gui => {
            winview_app::gui::run_gui()?;

            Ok(ExitCode::SUCCESS)
        }
    }
}
