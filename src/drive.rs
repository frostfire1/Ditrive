//! Google Drive manager for file uploads and downloads using REST API

use indicatif::{ProgressBar, ProgressStyle};
use reqwest::{header, multipart, Client};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use tracing::info;

use crate::error::{DitriveError, Result};
use crate::oauth::{OAuthCredentials, OAuthManager};

/// Authentication method for Google Drive
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// OAuth2 user authentication (for collaboration)
    OAuth(OAuthCredentials),
    /// Service account authentication (for automation)
    ServiceAccount(String), // Path to service account JSON file
}

/// File metadata stored in .woilah files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub id: String,
    pub hash: String,
    pub size: u64,
    pub uploaded_at: i64,
}

/// Calculate SHA-256 hash of a file
pub fn calculate_file_hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Service account key structure
#[derive(Debug, Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    token_uri: String,
}

/// Google API token response
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    token_type: String,
}

/// Drive file response
#[derive(Debug, Deserialize)]
struct DriveFileResponse {
    id: Option<String>,
    name: Option<String>,
    size: Option<String>,
}

/// Drive files list response
#[derive(Debug, Deserialize)]
struct DriveFilesListResponse {
    files: Option<Vec<DriveFileResponse>>,
}

/// Google Drive manager using REST API
pub struct DriveManager {
    client: Client,
    access_token: String,
    root_folder_id: String,
    repo_name: String,
    repo_folder_id: String,
    folder_cache: HashMap<String, String>,
    auth_method: AuthMethod,
}

impl DriveManager {
    const API_BASE: &'static str = "https://www.googleapis.com/drive/v3";
    const UPLOAD_BASE: &'static str = "https://www.googleapis.com/upload/drive/v3";

    /// Create a new DriveManager with OAuth authentication (for collaboration)
    pub async fn with_oauth(
        credentials: OAuthCredentials,
        root_folder_id: &str,
        repo_name: &str,
    ) -> Result<Self> {
        let client = Client::new();
        
        // Get access token via OAuth
        let oauth = OAuthManager::new(credentials.clone());
        let access_token = oauth.get_access_token().await?;

        let mut manager = Self {
            client,
            access_token,
            root_folder_id: root_folder_id.to_string(),
            repo_name: repo_name.to_string(),
            repo_folder_id: String::new(),
            folder_cache: HashMap::new(),
            auth_method: AuthMethod::OAuth(credentials),
        };

        // Get or create repository folder
        manager.repo_folder_id = manager
            .get_or_create_folder(repo_name, root_folder_id)
            .await?;

        info!(
            "DriveManager (OAuth) initialized for repo '{}' with folder ID: {}",
            repo_name, manager.repo_folder_id
        );

        Ok(manager)
    }

    /// Create a new DriveManager with service account (legacy/automation)
    pub async fn with_service_account(
        service_account_file: &str,
        root_folder_id: &str,
        repo_name: &str,
    ) -> Result<Self> {
        let client = Client::new();
        
        // Get access token via service account
        let access_token = Self::get_service_account_token(&client, service_account_file).await?;

        let mut manager = Self {
            client,
            access_token,
            root_folder_id: root_folder_id.to_string(),
            repo_name: repo_name.to_string(),
            repo_folder_id: String::new(),
            folder_cache: HashMap::new(),
            auth_method: AuthMethod::ServiceAccount(service_account_file.to_string()),
        };

        // Get or create repository folder
        manager.repo_folder_id = manager
            .get_or_create_folder(repo_name, root_folder_id)
            .await?;

        info!(
            "DriveManager (ServiceAccount) initialized for repo '{}' with folder ID: {}",
            repo_name, manager.repo_folder_id
        );

        Ok(manager)
    }

    /// Create a new DriveManager (auto-detect auth method based on config)
    pub async fn new(
        service_account_file: &str,
        root_folder_id: &str,
        repo_name: &str,
    ) -> Result<Self> {
        Self::with_service_account(service_account_file, root_folder_id, repo_name).await
    }

    /// Get access token using service account
    async fn get_service_account_token(client: &Client, service_account_file: &str) -> Result<String> {
        let key_content = fs::read_to_string(service_account_file)?;
        let key: ServiceAccountKey = serde_json::from_str(&key_content)
            .map_err(|e| DitriveError::Auth(format!("Failed to parse service account key: {}", e)))?;

        // Create JWT
        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": key.client_email,
            "scope": "https://www.googleapis.com/auth/drive",
            "aud": key.token_uri,
            "iat": now,
            "exp": now + 3600,
        });

        // Sign JWT with RS256
        let header = base64_url_encode(r#"{"alg":"RS256","typ":"JWT"}"#.as_bytes());
        let payload = base64_url_encode(claims.to_string().as_bytes());
        let signing_input = format!("{}.{}", header, payload);
        
        let signature = sign_rs256(&signing_input, &key.private_key)
            .map_err(|e| DitriveError::Auth(format!("Failed to sign JWT: {}", e)))?;
        
        let jwt = format!("{}.{}", signing_input, signature);

        // Exchange JWT for access token
        let response = client
            .post(&key.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .await
            .map_err(|e| DitriveError::Auth(format!("Failed to get access token: {}", e)))?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(DitriveError::Auth(format!("Token request failed: {}", error)));
        }

        let token_response: TokenResponse = response.json().await
            .map_err(|e| DitriveError::Auth(format!("Failed to parse token response: {}", e)))?;

        Ok(token_response.access_token)
    }

    /// Get the repository folder ID
    pub fn repo_folder_id(&self) -> &str {
        &self.repo_folder_id
    }

    /// Get or create a folder in Drive
    async fn get_or_create_folder(&mut self, name: &str, parent_id: &str) -> Result<String> {
        let cache_key = format!("{}/{}", parent_id, name);

        // Check cache first
        if let Some(id) = self.folder_cache.get(&cache_key) {
            return Ok(id.clone());
        }

        // Search for existing folder
        let query = format!(
            "name='{}' and '{}' in parents and mimeType='application/vnd.google-apps.folder' and trashed=false",
            name, parent_id
        );

        let response = self
            .client
            .get(&format!("{}/files", Self::API_BASE))
            .bearer_auth(&self.access_token)
            .query(&[("q", &query), ("fields", &"files(id,name)".to_string())])
            .send()
            .await
            .map_err(|e| DitriveError::Drive(format!("Failed to list folders: {}", e)))?;

        let list_response: DriveFilesListResponse = response.json().await
            .map_err(|e| DitriveError::Drive(format!("Failed to parse response: {}", e)))?;

        let folder_id = if let Some(files) = list_response.files {
            if let Some(folder) = files.first() {
                folder.id.clone().unwrap_or_default()
            } else {
                self.create_folder(name, parent_id).await?
            }
        } else {
            self.create_folder(name, parent_id).await?
        };

        // Update cache
        self.folder_cache.insert(cache_key, folder_id.clone());

        Ok(folder_id)
    }

    /// Create a folder in Drive
    async fn create_folder(&self, name: &str, parent_id: &str) -> Result<String> {
        let metadata = serde_json::json!({
            "name": name,
            "parents": [parent_id],
            "mimeType": "application/vnd.google-apps.folder"
        });

        let response = self
            .client
            .post(&format!("{}/files", Self::API_BASE))
            .bearer_auth(&self.access_token)
            .header(header::CONTENT_TYPE, "application/json")
            .json(&metadata)
            .send()
            .await
            .map_err(|e| DitriveError::Drive(format!("Failed to create folder: {}", e)))?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(DitriveError::Drive(format!("Failed to create folder: {}", error)));
        }

        let folder: DriveFileResponse = response.json().await
            .map_err(|e| DitriveError::Drive(format!("Failed to parse response: {}", e)))?;

        let folder_id = folder.id.ok_or_else(|| DitriveError::Drive("No folder ID returned".to_string()))?;

        info!("Created folder '{}' with ID: {}", name, folder_id);
        Ok(folder_id)
    }

    /// Get the folder ID for a file path, creating folders as needed
    async fn get_folder_for_path(&mut self, file_path: &Path, repo_path: &Path) -> Result<String> {
        let rel_path = file_path
            .strip_prefix(repo_path)
            .map_err(|_| DitriveError::FileNotFound(file_path.display().to_string()))?;

        let mut current_folder_id = self.repo_folder_id.clone();

        // Create or get each folder in the path (excluding filename)
        if let Some(parent) = rel_path.parent() {
            for component in parent.components() {
                if let std::path::Component::Normal(name) = component {
                    let name_str = name.to_string_lossy();
                    current_folder_id = self
                        .get_or_create_folder(&name_str, &current_folder_id)
                        .await?;
                }
            }
        }

        Ok(current_folder_id)
    }

    /// Upload a file to Drive with progress indication
    pub async fn upload_file(
        &mut self,
        file_path: &Path,
        repo_path: &Path,
    ) -> Result<FileMetadata> {
        let file_name = file_path
            .file_name()
            .ok_or_else(|| DitriveError::FileNotFound(file_path.display().to_string()))?
            .to_string_lossy()
            .to_string();

        let file_size = fs::metadata(file_path)?.len();
        let file_hash = calculate_file_hash(file_path)?;
        let mime_type = mime_guess::from_path(file_path)
            .first_or_octet_stream()
            .to_string();

        // Get the folder for this file
        let folder_id = self.get_folder_for_path(file_path, repo_path).await?;

        // Create progress bar
        let pb = ProgressBar::new(file_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        pb.set_message(format!("Uploading {}", file_name));

        // Read file content
        let file_content = fs::read(file_path)?;
        pb.set_position(file_size / 3);

        // Create metadata part
        let metadata = serde_json::json!({
            "name": file_name,
            "parents": [folder_id]
        });

        // Use multipart upload
        let form = multipart::Form::new()
            .text("metadata", metadata.to_string())
            .part("file", multipart::Part::bytes(file_content).mime_str(&mime_type)?);

        pb.set_position(file_size * 2 / 3);

        let response = self
            .client
            .post(&format!("{}/files?uploadType=multipart", Self::UPLOAD_BASE))
            .bearer_auth(&self.access_token)
            .multipart(form)
            .send()
            .await
            .map_err(|e| DitriveError::Drive(format!("Failed to upload file: {}", e)))?;

        pb.set_position(file_size);

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(DitriveError::Drive(format!("Upload failed: {}", error)));
        }

        let file_response: DriveFileResponse = response.json().await
            .map_err(|e| DitriveError::Drive(format!("Failed to parse response: {}", e)))?;

        pb.finish_with_message(format!("Uploaded {}", file_name));

        let drive_id = file_response.id
            .ok_or_else(|| DitriveError::Drive("No file ID returned".to_string()))?;

        info!("Uploaded {} ({} bytes) to Drive", file_name, file_size);

        Ok(FileMetadata {
            id: drive_id,
            hash: file_hash,
            size: file_size,
            uploaded_at: chrono::Utc::now().timestamp(),
        })
    }

    /// Download a file from Drive with progress indication
    pub async fn download_file(&self, file_id: &str, destination: &Path) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        let file_name = destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();

        info!("Downloading {} from Drive...", file_name);

        // Get file metadata for size
        let meta_response = self
            .client
            .get(&format!("{}/files/{}", Self::API_BASE, file_id))
            .bearer_auth(&self.access_token)
            .query(&[("fields", "size,name")])
            .send()
            .await
            .map_err(|e| DitriveError::Drive(format!("Failed to get file metadata: {}", e)))?;

        let file_meta: DriveFileResponse = meta_response.json().await.unwrap_or(DriveFileResponse {
            id: None,
            name: None,
            size: None,
        });

        let file_size = file_meta.size.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);

        // Create progress bar
        let pb = ProgressBar::new(file_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        pb.set_message(format!("Downloading {}", file_name));

        // Download file content
        let response = self
            .client
            .get(&format!("{}/files/{}?alt=media", Self::API_BASE, file_id))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .map_err(|e| DitriveError::Drive(format!("Failed to download file: {}", e)))?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(DitriveError::Drive(format!("Download failed: {}", error)));
        }

        let bytes = response.bytes().await
            .map_err(|e| DitriveError::Drive(format!("Failed to read response: {}", e)))?;

        fs::write(destination, &bytes)?;
        pb.finish_with_message(format!("Downloaded {}", file_name));

        info!("Downloaded {} to {:?}", file_name, destination);
        Ok(())
    }

    /// Check if a file exists in Drive
    pub async fn file_exists(&self, file_id: &str) -> bool {
        self.client
            .get(&format!("{}/files/{}", Self::API_BASE, file_id))
            .bearer_auth(&self.access_token)
            .query(&[("fields", "id")])
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

/// Base64 URL-safe encoding without padding
fn base64_url_encode(data: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, data)
}

/// Sign data with RS256 (RSA-SHA256)
fn sign_rs256(data: &str, private_key_pem: &str) -> std::result::Result<String, Box<dyn std::error::Error>> {
    use sha2::Sha256;
    use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey};
    use rsa::signature::{SignatureEncoding, Signer};
    use rsa::pkcs1v15::SigningKey;

    let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)?;
    let signing_key = SigningKey::<Sha256>::new(private_key);
    let signature = signing_key.sign(data.as_bytes());
    
    Ok(base64_url_encode(&signature.to_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_calculate_file_hash() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "Hello, World!").unwrap();

        let hash = calculate_file_hash(&file_path).unwrap();
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA-256 produces 64 hex characters
    }
}
