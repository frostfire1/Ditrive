//! Main application orchestrator

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::config::{DriveAuthType, GlobalConfig, RepoConfig};
use crate::drive::DriveManager;
use crate::error::{DitriveError, Result};
use crate::git::{GitIgnoreParser, GitManager};
use crate::github::GitHubManager;
use crate::oauth::OAuthCredentials;
use crate::tracker::WoilahTracker;

/// Main application struct
pub struct Ditrive {
    repo_path: PathBuf,
    repo_name: String,
    global_config: GlobalConfig,
    repo_config: RepoConfig,
    git_manager: Option<GitManager>,
    gitignore_parser: Option<GitIgnoreParser>,
    tracker: WoilahTracker,
}

impl Ditrive {
    /// Create a new Ditrive instance
    pub fn new(repo_path: &Path) -> Result<Self> {
        let repo_path = repo_path.canonicalize().unwrap_or_else(|_| repo_path.to_path_buf());
        let repo_name = repo_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string());

        let global_config = GlobalConfig::load()?;
        let repo_config = RepoConfig::load(&repo_path)?;

        let git_manager = if repo_path.join(".git").exists() {
            Some(GitManager::open(&repo_path)?)
        } else {
            None
        };

        let gitignore_parser = git_manager.as_ref().map(|_| GitIgnoreParser::new(&repo_path));

        let tracker = WoilahTracker::new(&repo_path);

        Ok(Self {
            repo_path,
            repo_name,
            global_config,
            repo_config,
            git_manager,
            gitignore_parser,
            tracker,
        })
    }

    /// Create a DriveManager based on configured auth type (OAuth or Service Account)
    async fn create_drive_manager(&self) -> Result<DriveManager> {
        let folder_id = if !self.repo_config.drive.folder_id.is_empty() {
            &self.repo_config.drive.folder_id
        } else {
            &self.global_config.drive.root_folder_id
        };

        match self.global_config.drive.auth_type {
            DriveAuthType::OAuth => {
                let credentials = OAuthCredentials {
                    client_id: self.global_config.drive.client_id.clone(),
                    client_secret: self.global_config.drive.client_secret.clone(),
                    redirect_uri: "http://localhost:8085".to_string(),
                };
                DriveManager::with_oauth(credentials, folder_id, &self.repo_name).await
            }
            DriveAuthType::ServiceAccount => {
                DriveManager::with_service_account(
                    &self.global_config.drive.service_account_file,
                    folder_id,
                    &self.repo_name,
                ).await
            }
        }
    }

    /// Get the repository path
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    /// Check if a file exceeds the large file threshold
    pub fn is_large_file(&self, path: &Path) -> bool {
        let threshold = self.repo_config.large_file_threshold_bytes();
        fs::metadata(path)
            .map(|m| m.len() > threshold)
            .unwrap_or(false)
    }

    /// Check if a file is ignored by gitignore or additional patterns
    pub fn is_ignored(&self, path: &Path) -> bool {
        // Check gitignore
        if let Some(ref parser) = self.gitignore_parser {
            if parser.is_ignored(path) {
                return true;
            }
        }

        // Check additional patterns
        let filename = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        for pattern in &self.repo_config.settings.additional_ignore_patterns {
            if pattern.starts_with('*') {
                let suffix = &pattern[1..];
                if filename.ends_with(suffix) {
                    return true;
                }
            } else if filename == *pattern {
                return true;
            }
        }

        false
    }

    /// Check if a file is managed by woilah
    pub fn is_managed(&self, path: &Path) -> Result<bool> {
        self.tracker.is_managed(path)
    }

    /// Configure global settings interactively
    pub fn configure(&mut self) -> Result<()> {
        println!("Ditrive Configuration");
        println!("=====================\n");

        // Helper function to read input
        fn read_input() -> std::io::Result<String> {
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            Ok(input.trim().to_string())
        }

        // GitHub configuration
        println!("GitHub Configuration:");

        print!(
            "GitHub username [{}]: ",
            self.global_config.github.username
        );
        io::stdout().flush()?;
        let input = read_input()?;
        if !input.is_empty() {
            self.global_config.github.username = input;
        }

        print!("GitHub personal access token (leave blank to keep current): ");
        io::stdout().flush()?;
        let input = read_input()?;
        if !input.is_empty() {
            self.global_config.github.token = input;
        }

        print!(
            "Default repository visibility (public/private) [{}]: ",
            self.global_config.github.default_visibility
        );
        io::stdout().flush()?;
        let input = read_input()?;
        if input == "public" || input == "private" {
            self.global_config.github.default_visibility = input;
        }

        // Google Drive configuration
        println!("\nGoogle Drive Configuration:");
        
        let current_auth = match self.global_config.drive.auth_type {
            crate::config::DriveAuthType::OAuth => "oauth",
            crate::config::DriveAuthType::ServiceAccount => "service_account",
        };
        
        println!("\nAuthentication method:");
        println!("  1. OAuth (recommended for collaboration - each user logs in with their Google account)");
        println!("  2. Service Account (for automation/CI)");
        print!("Choose auth method (1/2) [{}]: ", if current_auth == "oauth" { "1" } else { "2" });
        io::stdout().flush()?;
        let input = read_input()?;
        
        match input.as_str() {
            "1" | "oauth" => {
                self.global_config.drive.auth_type = crate::config::DriveAuthType::OAuth;
                
                println!("\nOAuth Configuration:");
                println!("(Get these from Google Cloud Console > APIs & Services > Credentials)");
                
                print!("OAuth Client ID [{}]: ", 
                    if self.global_config.drive.client_id.is_empty() { "<not set>" } 
                    else { &self.global_config.drive.client_id });
                io::stdout().flush()?;
                let input = read_input()?;
                if !input.is_empty() {
                    self.global_config.drive.client_id = input;
                }
                
                print!("OAuth Client Secret (leave blank to keep current): ");
                io::stdout().flush()?;
                let input = read_input()?;
                if !input.is_empty() {
                    self.global_config.drive.client_secret = input;
                }
            }
            "2" | "service_account" => {
                self.global_config.drive.auth_type = crate::config::DriveAuthType::ServiceAccount;
                
                print!(
                    "Service account file path [{}]: ",
                    self.global_config.drive.service_account_file
                );
                io::stdout().flush()?;
                let input = read_input()?;
                if !input.is_empty() {
                    if Path::new(&input).exists() {
                        self.global_config.drive.service_account_file = input;
                    } else {
                        warn!("File {} does not exist", input);
                    }
                }
            }
            _ => {
                // Keep current setting
            }
        }

        print!(
            "Root folder ID [{}]: ",
            if self.global_config.drive.root_folder_id.is_empty() { "<not set>" }
            else { &self.global_config.drive.root_folder_id }
        );
        io::stdout().flush()?;
        let input = read_input()?;
        if !input.is_empty() {
            self.global_config.drive.root_folder_id = input;
        }

        // Settings
        println!("\nApplication Settings:");

        print!(
            "Large file threshold in MB [{}]: ",
            self.global_config.settings.large_file_threshold_mb
        );
        io::stdout().flush()?;
        let input = read_input()?;
        if let Ok(threshold) = input.parse::<u64>() {
            self.global_config.settings.large_file_threshold_mb = threshold;
        }

        // Save configuration
        self.global_config.save()?;
        println!("\nConfiguration saved!");

        if self.global_config.is_configured() {
            println!("\nConfiguration is complete. You can now use 'quick-setup' to create a new repository.");
            if self.global_config.drive.auth_type == crate::config::DriveAuthType::OAuth {
                println!("Run 'ditrive login' to authenticate with Google Drive.");
            }
        } else {
            println!("\nConfiguration is incomplete. Please fill in all required fields.");
        }

        Ok(())
    }

    /// Quick setup for a new repository
    pub async fn quick_setup(
        &mut self,
        name: Option<&str>,
        description: &str,
        private: bool,
    ) -> Result<()> {
        if !self.global_config.is_configured() {
            return Err(DitriveError::Config(
                "Global configuration is not complete. Please run 'configure' first.".to_string(),
            ));
        }

        let repo_name = name.unwrap_or(&self.repo_name).to_string();
        info!("Setting up repository: {}", repo_name);

        // Initialize Git repository if not already initialized
        if self.git_manager.is_none() {
            info!("Initializing Git repository...");
            self.git_manager = Some(GitManager::init(&self.repo_path)?);
            self.gitignore_parser = Some(GitIgnoreParser::new(&self.repo_path));
        }

        // Create GitHub repository
        let github = GitHubManager::new(
            &self.global_config.github.username,
            &self.global_config.github.token,
        )?;

        info!("Creating GitHub repository: {}", repo_name);
        let github_repo = github.create_repository(&repo_name, description, private).await?;
        info!("GitHub repository created: {}", github_repo.html_url);

        // Configure Git
        if let Some(ref git) = self.git_manager {
            git.configure_user(
                &self.global_config.github.username,
                &format!("{}@users.noreply.github.com", self.global_config.github.username),
            )?;

            let auth_url = github.get_auth_url(&repo_name);
            git.set_remote_url("origin", &auth_url)?;
        }

        // Update repo config
        self.repo_config.github.repository_url = github_repo.html_url;
        self.repo_config.save(&self.repo_path)?;

        // Set up Drive folder
        info!("Setting up Google Drive folder...");
        let drive = self.create_drive_manager().await?;

        self.repo_config.drive.folder_id = drive.repo_folder_id().to_string();
        self.repo_config.save(&self.repo_path)?;
        info!("Google Drive folder created with ID: {}", drive.repo_folder_id());

        // Create initial commit
        self.create_initial_commit().await?;

        info!("Quick setup completed!");
        Ok(())
    }

    /// Create initial commit
    async fn create_initial_commit(&self) -> Result<()> {
        let git = self
            .git_manager
            .as_ref()
            .ok_or_else(|| DitriveError::NotGitRepo(self.repo_path.display().to_string()))?;

        // Create .gitignore if it doesn't exist
        let gitignore_path = self.repo_path.join(".gitignore");
        if !gitignore_path.exists() {
            fs::write(
                &gitignore_path,
                "# Ditrive\n.woilah-config.json\n.woilah\n",
            )?;
        }

        // Stage files
        git.stage_files(&[".gitignore", ".woilah-config.json"])?;

        // Create commit
        git.commit("Initial commit")?;

        info!("Created initial commit");
        Ok(())
    }

    /// Initialize repository for ditrive
    pub async fn initialize(&mut self) -> Result<()> {
        info!("Initializing ditrive for repository: {}", self.repo_name);

        // Process existing files
        self.process_new_files().await?;

        // Stage config files
        if let Some(ref git) = self.git_manager {
            let mut files_to_stage = Vec::new();

            if self.repo_path.join(".woilah-config.json").exists() {
                files_to_stage.push(".woilah-config.json");
            }
            if self.repo_path.join(".gitignore").exists() {
                files_to_stage.push(".gitignore");
            }

            if !files_to_stage.is_empty() {
                git.stage_files(&files_to_stage)?;
            }
        }

        info!("Ditrive initialized for repository");
        Ok(())
    }

    /// Process new files in the repository
    pub async fn process_new_files(&mut self) -> Result<()> {
        let large_files = self.find_large_files()?;

        if large_files.is_empty() {
            info!("No large files to process");
            return Ok(());
        }

        info!("Found {} large files to process", large_files.len());

        // Initialize Drive manager
        let mut drive = self.create_drive_manager().await?;

        for file_path in large_files {
            // Skip if already managed
            if self.tracker.is_managed(&file_path)? {
                debug!("Skipping already managed file: {:?}", file_path);
                continue;
            }

            // Check if ignored
            if self.is_ignored(&file_path) {
                let action = self.handle_ignored_large_file(&file_path)?;
                if action == "skip" {
                    info!("Skipping ignored large file: {:?}", file_path);
                    continue;
                }
            }

            // Upload to Drive
            info!("Uploading large file: {:?}", file_path);
            let metadata = drive.upload_file(&file_path, &self.repo_path).await?;

            // Add to tracker
            let folder_path = file_path.parent().unwrap_or(&self.repo_path);
            let filename = file_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            self.tracker.add_file_mapping(folder_path, &filename, metadata)?;

            // Add to gitignore
            if let Some(ref mut parser) = self.gitignore_parser {
                let rel_path = file_path
                    .strip_prefix(&self.repo_path)
                    .unwrap_or(&file_path)
                    .to_string_lossy()
                    .replace('\\', "/");

                parser.add_pattern(
                    &rel_path,
                    Some(&self.repo_config.settings.managed_files_marker),
                )?;
            }

            info!("Added {:?} to Drive and .gitignore", file_path);
        }

        Ok(())
    }

    /// Find all large files in the repository
    fn find_large_files(&self) -> Result<Vec<PathBuf>> {
        let mut large_files = Vec::new();

        for entry in WalkDir::new(&self.repo_path)
            .into_iter()
            .filter_entry(|e| {
                !e.path().starts_with(self.repo_path.join(".git"))
            })
        {
            let entry = entry?;
            if entry.file_type().is_file() {
                let path = entry.path();
                let filename = path.file_name().unwrap_or_default().to_string_lossy();

                // Skip config files
                if filename == ".woilah" || filename == ".woilah-config.json" {
                    continue;
                }

                if self.is_large_file(path) {
                    large_files.push(path.to_path_buf());
                }
            }
        }

        Ok(large_files)
    }

    /// Handle a large file that is already ignored
    fn handle_ignored_large_file(&self, file_path: &Path) -> Result<String> {
        let handle_ignored = &self.repo_config.settings.handle_ignored_large_files;

        if handle_ignored == "skip" {
            return Ok("skip".to_string());
        } else if handle_ignored == "manage" {
            return Ok("manage".to_string());
        }

        // Ask user
        let rel_path = file_path
            .strip_prefix(&self.repo_path)
            .unwrap_or(file_path);

        println!("\nLarge file {:?} is already in .gitignore.", rel_path);
        println!("What would you like to do?");
        println!("1. Manage it with Ditrive (upload to Drive)");
        println!("2. Skip it (keep it ignored)");

        print!("Enter your choice (1-2): ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim() {
            "1" => Ok("manage".to_string()),
            _ => Ok("skip".to_string()),
        }
    }

    /// Sync missing files from Drive
    pub async fn sync_missing_files(&self) -> Result<()> {
        let managed_files = self.tracker.get_all_managed_files()?;

        let missing: Vec<_> = managed_files
            .into_iter()
            .filter(|(path, _)| !path.exists())
            .collect();

        if missing.is_empty() {
            info!("No missing files to download");
            return Ok(());
        }

        info!("Found {} missing files to download", missing.len());

        let drive = self.create_drive_manager().await?;

        for (path, metadata) in missing {
            info!("Downloading missing file: {:?}", path);
            drive.download_file(&metadata.id, &path).await?;
        }

        Ok(())
    }

    /// Full sync: process new files and download missing ones
    pub async fn sync(&mut self) -> Result<()> {
        info!("Starting sync...");

        // Process new large files
        self.process_new_files().await?;

        // Download missing files
        self.sync_missing_files().await?;

        info!("Sync complete");
        Ok(())
    }

    /// List all managed files
    pub async fn list_managed(&self) -> Result<()> {
        println!("Ditrive Managed Files for: {}", self.repo_name);
        println!("Repository path: {:?}", self.repo_path);
        println!();

        if !self.repo_config.drive.folder_id.is_empty() {
            println!("Google Drive folder: {}", self.repo_config.drive.folder_id);
        } else {
            println!("Google Drive: Not configured");
        }

        if !self.repo_config.github.repository_url.is_empty() {
            println!("GitHub: {}", self.repo_config.github.repository_url);
        } else {
            println!("GitHub: Not connected");
        }

        println!(
            "Large file threshold: {} MB",
            self.repo_config.settings.large_file_threshold_mb
        );
        println!();

        let managed_files = self.tracker.get_all_managed_files()?;

        if managed_files.is_empty() {
            println!("No files are currently managed by Ditrive.");
            return Ok(());
        }

        println!("Managed files:");
        println!(
            "{:<50} {:>10} {:>6} {}",
            "File Path", "Size", "Local", "Drive ID"
        );
        println!("{}", "-".repeat(100));

        for (path, metadata) in managed_files {
            let rel_path = path
                .strip_prefix(&self.repo_path)
                .unwrap_or(&path)
                .display()
                .to_string();

            let exists_locally = path.exists();
            let local_status = if exists_locally { "Yes" } else { "No" };

            let size = if metadata.size > 0 {
                format!("{:.2} MB", metadata.size as f64 / 1024.0 / 1024.0)
            } else {
                "Unknown".to_string()
            };

            println!(
                "{:<50} {:>10} {:>6} {}",
                rel_path, size, local_status, metadata.id
            );
        }

        Ok(())
    }

    /// Login to Google Drive using OAuth
    pub async fn login(&self) -> Result<()> {
        use crate::oauth::{OAuthManager, OAuthCredentials};
        
        if self.global_config.drive.auth_type != crate::config::DriveAuthType::OAuth {
            return Err(DitriveError::Config(
                "OAuth is not configured. Run 'ditrive configure' and select OAuth as auth method.".to_string()
            ));
        }

        if self.global_config.drive.client_id.is_empty() || self.global_config.drive.client_secret.is_empty() {
            return Err(DitriveError::Config(
                "OAuth client ID and secret are not configured. Run 'ditrive configure'.".to_string()
            ));
        }

        let credentials = OAuthCredentials {
            client_id: self.global_config.drive.client_id.clone(),
            client_secret: self.global_config.drive.client_secret.clone(),
            redirect_uri: "http://localhost:8085".to_string(),
        };
        
        let oauth_manager = OAuthManager::new(credentials);

        // Check if already authenticated
        if oauth_manager.is_authenticated() {
            println!("Already logged in. Use 'ditrive logout' to sign out first.");
            return Ok(());
        }

        // Start OAuth flow - this will open browser and wait for callback
        println!("\nStarting Google OAuth login...");
        oauth_manager.authorize().await?;

        println!("\n✓ Successfully logged in to Google Drive!");
        println!("Your credentials are saved in ~/.ditrive/tokens.json");

        Ok(())
    }

    /// Logout from Google Drive (clear OAuth tokens)
    pub async fn logout(&self) -> Result<()> {
        use crate::oauth::{OAuthManager, OAuthCredentials};

        if self.global_config.drive.auth_type != crate::config::DriveAuthType::OAuth {
            println!("OAuth is not configured. Nothing to logout from.");
            return Ok(());
        }

        let credentials = OAuthCredentials {
            client_id: self.global_config.drive.client_id.clone(),
            client_secret: self.global_config.drive.client_secret.clone(),
            redirect_uri: "http://localhost:8085".to_string(),
        };
        
        let oauth_manager = OAuthManager::new(credentials);
        oauth_manager.logout().await?;

        println!("✓ Successfully logged out from Google Drive.");
        println!("Run 'ditrive login' to authenticate again.");

        Ok(())
    }

    /// Check login status
    pub async fn status(&self) -> Result<()> {
        println!("Ditrive Status");
        println!("==============\n");

        // Check configuration
        println!("Configuration:");
        println!("  GitHub username: {}", 
            if self.global_config.github.username.is_empty() { "<not set>" } 
            else { &self.global_config.github.username });
        println!("  GitHub token: {}", 
            if self.global_config.github.token.is_empty() { "<not set>" } 
            else { "********" });
        
        let auth_type = match self.global_config.drive.auth_type {
            crate::config::DriveAuthType::OAuth => "OAuth",
            crate::config::DriveAuthType::ServiceAccount => "Service Account",
        };
        println!("  Drive auth type: {}", auth_type);
        
        match self.global_config.drive.auth_type {
            crate::config::DriveAuthType::OAuth => {
                println!("  OAuth client ID: {}", 
                    if self.global_config.drive.client_id.is_empty() { "<not set>" }
                    else { &self.global_config.drive.client_id });
                
                // Check login status
                use crate::oauth::{OAuthManager, OAuthCredentials};
                let credentials = OAuthCredentials {
                    client_id: self.global_config.drive.client_id.clone(),
                    client_secret: self.global_config.drive.client_secret.clone(),
                    redirect_uri: "http://localhost:8085".to_string(),
                };
                let oauth_manager = OAuthManager::new(credentials);
                
                if oauth_manager.is_authenticated() {
                    println!("  Login status: ✓ Logged in");
                } else {
                    println!("  Login status: ✗ Not logged in (run 'ditrive login')");
                }
            }
            crate::config::DriveAuthType::ServiceAccount => {
                println!("  Service account: {}", 
                    if self.global_config.drive.service_account_file.is_empty() { "<not set>" }
                    else { &self.global_config.drive.service_account_file });
            }
        }
        
        println!("  Root folder ID: {}", 
            if self.global_config.drive.root_folder_id.is_empty() { "<not set>" } 
            else { &self.global_config.drive.root_folder_id });

        // Check repo status
        println!("\nRepository:");
        if self.git_manager.is_some() {
            println!("  Git initialized: ✓");
            println!("  Repository name: {}", self.repo_name);
        } else {
            println!("  Git initialized: ✗");
        }

        let managed_count = self.tracker.get_all_managed_files()?.len();
        println!("  Large files tracked: {}", managed_count);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_ditrive_new() {
        let dir = tempdir().unwrap();
        let ditrive = Ditrive::new(dir.path()).unwrap();
        assert!(ditrive.git_manager.is_none());
    }
}
