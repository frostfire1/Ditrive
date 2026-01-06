//! Ditrive - Git Drive Sync
//!
//! Automatically manage large files in Git with Google Drive

mod app;
mod cli;
mod config;
mod drive;
mod error;
mod git;
mod github;
mod oauth;
mod tracker;

use anyhow::Result;
use clap::Parser;
use tracing::error;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::app::Ditrive;
use crate::cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();

    // Resolve repository path
    let repo_path = cli.repo.canonicalize().unwrap_or(cli.repo.clone());

    // Execute command
    let result = match cli.command {
        Commands::Configure => {
            let mut ditrive = Ditrive::new(&repo_path)?;
            ditrive.configure()
        }
        Commands::Login => {
            let ditrive = Ditrive::new(&repo_path)?;
            ditrive.login().await
        }
        Commands::Logout => {
            let ditrive = Ditrive::new(&repo_path)?;
            ditrive.logout().await
        }
        Commands::QuickSetup {
            name,
            description,
            public,
        } => {
            let mut ditrive = Ditrive::new(&repo_path)?;
            ditrive
                .quick_setup(name.as_deref(), &description, !public)
                .await
        }
        Commands::Init => {
            let mut ditrive = Ditrive::new(&repo_path)?;
            ditrive.initialize().await
        }
        Commands::Sync => {
            let mut ditrive = Ditrive::new(&repo_path)?;
            ditrive.sync().await
        }
        Commands::Status => {
            let ditrive = Ditrive::new(&repo_path)?;
            ditrive.status().await
        }
        Commands::Pull => {
            let ditrive = Ditrive::new(&repo_path)?;
            ditrive.sync_missing_files().await
        }
        Commands::Push => {
            let mut ditrive = Ditrive::new(&repo_path)?;
            ditrive.process_new_files().await
        }
        Commands::List => {
            let ditrive = Ditrive::new(&repo_path)?;
            ditrive.list_managed().await
        }
    };

    if let Err(e) = result {
        error!("Error: {}", e);
        eprintln!("\nError: {}", e);
        eprintln!("\nUse 'ditrive --help' for usage information.");
        std::process::exit(1);
    }

    Ok(())
}

