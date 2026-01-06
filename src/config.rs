//! Configuration management for ditrive
//! 
//! Handles both global configuration (~/.ditrive/config.json) and
//! repository-specific configuration (.woilah-config.json)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::fs;
use crate::error::{DitriveError, Result};

/// Authentication type for Google Drive
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DriveAuthType {
    /// OAuth2 user authentication (for collaboration)
    OAuth,
    /// Service account authentication (for automation)
    ServiceAccount,
}

impl Default for DriveAuthType {
    fn default() -> Self {
        DriveAuthType::OAuth
    }
}

/// Global configuration shared across all repositories
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub github: GitHubGlobalConfig,
    pub drive: DriveGlobalConfig,
    pub settings: GlobalSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubGlobalConfig {
    pub username: String,
    pub token: String,
    pub default_visibility: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveGlobalConfig {
    /// Authentication type: "oauth" or "service_account"
    #[serde(default)]
    pub auth_type: DriveAuthType,
    /// OAuth client ID (for OAuth auth)
    #[serde(default)]
    pub client_id: String,
    /// OAuth client secret (for OAuth auth)
    #[serde(default)]
    pub client_secret: String,
    /// Service account file path (for service account auth)
    #[serde(default)]
    pub service_account_file: String,
    /// Root folder ID in Google Drive
    pub root_folder_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalSettings {
    pub large_file_threshold_mb: u64,
    pub handle_ignored_large_files: String,
    pub managed_files_marker: String,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            github: GitHubGlobalConfig {
                username: String::new(),
                token: String::new(),
                default_visibility: "private".to_string(),
            },
            drive: DriveGlobalConfig {
                auth_type: DriveAuthType::OAuth,
                client_id: String::new(),
                client_secret: String::new(),
                service_account_file: String::new(),
                root_folder_id: String::new(),
            },
            settings: GlobalSettings {
                large_file_threshold_mb: 10,
                handle_ignored_large_files: "ask".to_string(),
                managed_files_marker: "# Managed by Git Drive Sync".to_string(),
            },
        }
    }
}

impl GlobalConfig {
    /// Get the global config directory path
    pub fn config_dir() -> Result<PathBuf> {
        dirs::home_dir()
            .map(|h| h.join(".ditrive"))
            .ok_or_else(|| DitriveError::Config("Could not find home directory".to_string()))
    }

    /// Get the global config file path
    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.json"))
    }

    /// Load global configuration from file or create default
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;
        
        if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            let config: GlobalConfig = serde_json::from_str(&content)?;
            Ok(config)
        } else {
            let config = GlobalConfig::default();
            config.save()?;
            Ok(config)
        }
    }

    /// Save global configuration to file
    pub fn save(&self) -> Result<()> {
        let config_dir = Self::config_dir()?;
        let config_path = Self::config_path()?;

        // Create config directory if it doesn't exist
        if !config_dir.exists() {
            fs::create_dir_all(&config_dir)?;
        }

        let content = serde_json::to_string_pretty(self)?;
        fs::write(&config_path, content)?;
        Ok(())
    }

    /// Check if the configuration is complete
    pub fn is_configured(&self) -> bool {
        let github_ok = !self.github.token.is_empty();
        let drive_ok = match self.drive.auth_type {
            DriveAuthType::OAuth => {
                !self.drive.client_id.is_empty() 
                    && !self.drive.client_secret.is_empty()
                    && !self.drive.root_folder_id.is_empty()
            }
            DriveAuthType::ServiceAccount => {
                !self.drive.service_account_file.is_empty()
                    && !self.drive.root_folder_id.is_empty()
            }
        };
        github_ok && drive_ok
    }

    /// Check if Drive is configured (without GitHub)
    pub fn is_drive_configured(&self) -> bool {
        match self.drive.auth_type {
            DriveAuthType::OAuth => {
                !self.drive.client_id.is_empty() 
                    && !self.drive.client_secret.is_empty()
                    && !self.drive.root_folder_id.is_empty()
            }
            DriveAuthType::ServiceAccount => {
                !self.drive.service_account_file.is_empty()
                    && !self.drive.root_folder_id.is_empty()
            }
        }
    }

    /// Update a specific field and save
    pub fn update<F>(&mut self, updater: F) -> Result<()>
    where
        F: FnOnce(&mut Self),
    {
        updater(self);
        self.save()
    }
}

/// Repository-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub github: GitHubRepoConfig,
    pub drive: DriveRepoConfig,
    pub settings: RepoSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRepoConfig {
    pub repository_url: String,
    pub branch: String,
    pub username: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveRepoConfig {
    pub service_account_file: String,
    pub folder_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSettings {
    pub large_file_threshold_mb: u64,
    pub auto_sync: bool,
    pub additional_ignore_patterns: Vec<String>,
    pub handle_ignored_large_files: String,
    pub managed_files_marker: String,
}

impl RepoConfig {
    const CONFIG_FILENAME: &'static str = ".woilah-config.json";

    /// Create a new repo config with defaults from global config
    pub fn new_with_global(global: &GlobalConfig) -> Self {
        Self {
            github: GitHubRepoConfig {
                repository_url: String::new(),
                branch: "main".to_string(),
                username: global.github.username.clone(),
                token: global.github.token.clone(),
            },
            drive: DriveRepoConfig {
                service_account_file: global.drive.service_account_file.clone(),
                folder_id: String::new(),
            },
            settings: RepoSettings {
                large_file_threshold_mb: global.settings.large_file_threshold_mb,
                auto_sync: true,
                additional_ignore_patterns: vec!["*.tmp".to_string(), "*.log".to_string()],
                handle_ignored_large_files: global.settings.handle_ignored_large_files.clone(),
                managed_files_marker: global.settings.managed_files_marker.clone(),
            },
        }
    }

    /// Get the config file path for a repository
    pub fn config_path(repo_path: &Path) -> PathBuf {
        repo_path.join(Self::CONFIG_FILENAME)
    }

    /// Load repository configuration from file or create default
    pub fn load(repo_path: &Path) -> Result<Self> {
        let config_path = Self::config_path(repo_path);
        
        if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            let config: RepoConfig = serde_json::from_str(&content)?;
            Ok(config)
        } else {
            let global = GlobalConfig::load()?;
            let config = RepoConfig::new_with_global(&global);
            config.save(repo_path)?;
            Ok(config)
        }
    }

    /// Save repository configuration to file
    pub fn save(&self, repo_path: &Path) -> Result<()> {
        let config_path = Self::config_path(repo_path);
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&config_path, content)?;
        Ok(())
    }

    /// Update a specific field and save
    pub fn update<F>(&mut self, repo_path: &Path, updater: F) -> Result<()>
    where
        F: FnOnce(&mut Self),
    {
        updater(self);
        self.save(repo_path)
    }

    /// Get the large file threshold in bytes
    pub fn large_file_threshold_bytes(&self) -> u64 {
        self.settings.large_file_threshold_mb * 1024 * 1024
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_global_config_default() {
        let config = GlobalConfig::default();
        assert_eq!(config.settings.large_file_threshold_mb, 10);
        assert!(!config.is_configured());
    }

    #[test]
    fn test_repo_config_inherits_global() {
        let mut global = GlobalConfig::default();
        global.github.username = "testuser".to_string();
        global.github.token = "testtoken".to_string();
        
        let repo = RepoConfig::new_with_global(&global);
        assert_eq!(repo.github.username, "testuser");
        assert_eq!(repo.github.token, "testtoken");
    }
}
