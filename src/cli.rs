//! Command-line interface definitions

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "ditrive",
    version,
    about = "Git Drive Sync - Automatically manage large files in Git with Google Drive",
    long_about = r#"
Ditrive automatically manages large files in Git repositories by:
1. Detecting files that exceed the configured threshold
2. Uploading them to Google Drive with folder structure preservation
3. Adding them to .gitignore with tracking metadata
4. Downloading missing files when cloning or pulling

This allows you to keep large files out of Git while maintaining a seamless workflow.
"#
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Path to the Git repository
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,

    /// Enable verbose logging
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Configure global settings for GitHub and Google Drive
    Configure,

    /// Login to Google Drive using OAuth
    Login,

    /// Logout from Google Drive (clear stored tokens)
    Logout,

    /// Quick setup for a new repository with GitHub and Google Drive
    #[command(name = "quick-setup")]
    QuickSetup {
        /// Repository name (defaults to directory name)
        #[arg(short, long)]
        name: Option<String>,

        /// Repository description
        #[arg(short, long, default_value = "")]
        description: String,

        /// Create as public repository (default: private)
        #[arg(long)]
        public: bool,
    },

    /// Initialize Ditrive for an existing repository
    Init,

    /// Synchronize files between the repository and Google Drive
    Sync,

    /// Show status of Ditrive configuration and login
    Status,

    /// Download missing files from Google Drive
    Pull,

    /// Upload new/changed large files to Google Drive
    Push,

    /// List all managed files
    List,
}
