//! GitHub API manager for repository operations

use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::error::{DitriveError, Result};

/// GitHub repository response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRepo {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub html_url: String,
    pub clone_url: String,
    pub ssh_url: String,
    pub private: bool,
    pub default_branch: Option<String>,
}

/// Create repository request
#[derive(Debug, Serialize)]
struct CreateRepoRequest {
    name: String,
    description: String,
    private: bool,
    auto_init: bool,
}

/// GitHub API manager
pub struct GitHubManager {
    client: Client,
    username: String,
    token: String,
}

impl GitHubManager {
    const API_BASE: &'static str = "https://api.github.com";

    /// Create a new GitHubManager
    pub fn new(username: &str, token: &str) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("token {}", token))
                .map_err(|e| DitriveError::Auth(format!("Invalid token: {}", e)))?,
        );
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/vnd.github.v3+json"),
        );
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("ditrive/0.1.0"),
        );

        let client = Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|e| DitriveError::Http(e))?;

        Ok(Self {
            client,
            username: username.to_string(),
            token: token.to_string(),
        })
    }

    /// Get the username
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Get the authenticated remote URL
    pub fn get_auth_url(&self, repo_name: &str) -> String {
        format!(
            "https://{}:{}@github.com/{}/{}.git",
            self.username, self.token, self.username, repo_name
        )
    }

    /// Create a new repository
    pub async fn create_repository(
        &self,
        name: &str,
        description: &str,
        private: bool,
    ) -> Result<GitHubRepo> {
        let request = CreateRepoRequest {
            name: name.to_string(),
            description: description.to_string(),
            private,
            auto_init: false,
        };

        let response = self
            .client
            .post(&format!("{}/user/repos", Self::API_BASE))
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            let repo: GitHubRepo = response.json().await?;
            info!("Created GitHub repository: {}", repo.html_url);
            Ok(repo)
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            Err(DitriveError::GitHub(format!(
                "Failed to create repository ({}): {}",
                status, error_text
            )))
        }
    }

    /// Get repository information
    pub async fn get_repository(&self, owner: &str, name: &str) -> Result<GitHubRepo> {
        let response = self
            .client
            .get(&format!("{}/repos/{}/{}", Self::API_BASE, owner, name))
            .send()
            .await?;

        if response.status().is_success() {
            let repo: GitHubRepo = response.json().await?;
            debug!("Retrieved repository info for: {}", repo.full_name);
            Ok(repo)
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            Err(DitriveError::GitHub(format!(
                "Failed to get repository ({}): {}",
                status, error_text
            )))
        }
    }

    /// Check if a repository exists
    pub async fn repository_exists(&self, owner: &str, name: &str) -> bool {
        self.get_repository(owner, name).await.is_ok()
    }

    /// Delete a repository (use with caution!)
    pub async fn delete_repository(&self, owner: &str, name: &str) -> Result<()> {
        let response = self
            .client
            .delete(&format!("{}/repos/{}/{}", Self::API_BASE, owner, name))
            .send()
            .await?;

        if response.status().is_success() || response.status() == reqwest::StatusCode::NO_CONTENT {
            info!("Deleted repository: {}/{}", owner, name);
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            Err(DitriveError::GitHub(format!(
                "Failed to delete repository ({}): {}",
                status, error_text
            )))
        }
    }

    /// List user's repositories
    pub async fn list_repositories(&self) -> Result<Vec<GitHubRepo>> {
        let response = self
            .client
            .get(&format!("{}/user/repos", Self::API_BASE))
            .query(&[("per_page", "100"), ("sort", "updated")])
            .send()
            .await?;

        if response.status().is_success() {
            let repos: Vec<GitHubRepo> = response.json().await?;
            debug!("Listed {} repositories", repos.len());
            Ok(repos)
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            Err(DitriveError::GitHub(format!(
                "Failed to list repositories ({}): {}",
                status, error_text
            )))
        }
    }

    /// Validate the token by making a simple API call
    pub async fn validate_token(&self) -> Result<bool> {
        let response = self
            .client
            .get(&format!("{}/user", Self::API_BASE))
            .send()
            .await?;

        Ok(response.status().is_success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_github_manager_auth_url() {
        let manager = GitHubManager::new("testuser", "testtoken").unwrap();
        let url = manager.get_auth_url("myrepo");
        assert_eq!(
            url,
            "https://testuser:testtoken@github.com/testuser/myrepo.git"
        );
    }
}
