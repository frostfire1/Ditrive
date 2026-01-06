//! OAuth2 authentication for Google Drive
//! 
//! Supports user OAuth flow with token persistence for collaboration

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use tracing::{debug, info};

use crate::error::{DitriveError, Result};

/// OAuth2 client credentials (from Google Cloud Console)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl Default for OAuthCredentials {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            redirect_uri: "http://localhost:8085".to_string(),
        }
    }
}

/// Stored OAuth tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    pub token_type: String,
}

/// Google OAuth token response
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    token_type: String,
}

/// OAuth2 manager for Google Drive
pub struct OAuthManager {
    credentials: OAuthCredentials,
    tokens_path: PathBuf,
    client: reqwest::Client,
}

impl OAuthManager {
    const AUTH_URL: &'static str = "https://accounts.google.com/o/oauth2/v2/auth";
    const TOKEN_URL: &'static str = "https://oauth2.googleapis.com/token";
    const SCOPES: &'static str = "https://www.googleapis.com/auth/drive";

    /// Create a new OAuthManager
    pub fn new(credentials: OAuthCredentials) -> Self {
        let tokens_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ditrive")
            .join("tokens.json");

        Self {
            credentials,
            tokens_path,
            client: reqwest::Client::new(),
        }
    }

    /// Get a valid access token (refreshing if needed)
    pub async fn get_access_token(&self) -> Result<String> {
        // Try to load existing tokens
        if let Ok(tokens) = self.load_tokens() {
            // Check if token is still valid (with 5 min buffer)
            let now = chrono::Utc::now().timestamp();
            if tokens.expires_at > now + 300 {
                debug!("Using cached access token");
                return Ok(tokens.access_token);
            }

            // Try to refresh the token
            if let Some(refresh_token) = &tokens.refresh_token {
                info!("Refreshing access token...");
                if let Ok(new_tokens) = self.refresh_token(refresh_token).await {
                    return Ok(new_tokens.access_token);
                }
            }
        }

        // Need to do full OAuth flow
        info!("Starting OAuth authorization flow...");
        let tokens = self.authorize().await?;
        Ok(tokens.access_token)
    }

    /// Load tokens from disk
    fn load_tokens(&self) -> Result<StoredTokens> {
        let content = fs::read_to_string(&self.tokens_path)?;
        let tokens: StoredTokens = serde_json::from_str(&content)?;
        Ok(tokens)
    }

    /// Save tokens to disk
    fn save_tokens(&self, tokens: &StoredTokens) -> Result<()> {
        if let Some(parent) = self.tokens_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(tokens)?;
        fs::write(&self.tokens_path, content)?;
        debug!("Saved tokens to {:?}", self.tokens_path);
        Ok(())
    }

    /// Start the OAuth authorization flow
    pub async fn authorize(&self) -> Result<StoredTokens> {
        // Build authorization URL
        let auth_url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
            Self::AUTH_URL,
            urlencoding::encode(&self.credentials.client_id),
            urlencoding::encode(&self.credentials.redirect_uri),
            urlencoding::encode(Self::SCOPES),
        );

        println!("\n🔐 Google Drive Authorization Required\n");
        println!("Please open this URL in your browser:\n");
        println!("  {}\n", auth_url);

        // Try to open browser automatically
        if let Err(_) = open::that(&auth_url) {
            println!("(Could not open browser automatically)");
        }

        // Start local server to receive callback
        let code = self.wait_for_callback()?;
        
        println!("\n✓ Authorization code received!");

        // Exchange code for tokens
        let tokens = self.exchange_code(&code).await?;
        
        println!("✓ Successfully authenticated with Google Drive!\n");

        Ok(tokens)
    }

    /// Wait for OAuth callback on local server
    fn wait_for_callback(&self) -> Result<String> {
        // Parse port from redirect URI
        let port: u16 = self.credentials.redirect_uri
            .split(':')
            .last()
            .and_then(|s| s.trim_matches('/').parse().ok())
            .unwrap_or(8085);

        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .map_err(|e| DitriveError::Auth(format!("Failed to start callback server: {}", e)))?;

        println!("Waiting for authorization (listening on port {})...", port);

        for stream in listener.incoming() {
            if let Ok(mut stream) = stream {
                let mut reader = BufReader::new(&stream);
                let mut request_line = String::new();
                reader.read_line(&mut request_line)?;

                // Parse the authorization code from the request
                if let Some(code) = self.parse_code_from_request(&request_line) {
                    // Send success response
                    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
                        <html><body style='font-family: sans-serif; text-align: center; padding: 50px;'>\
                        <h1>✓ Authorization Successful!</h1>\
                        <p>You can close this window and return to the terminal.</p>\
                        </body></html>";
                    stream.write_all(response.as_bytes())?;
                    
                    return Ok(code);
                }

                // Send error response for other requests
                let response = "HTTP/1.1 400 Bad Request\r\n\r\nMissing authorization code";
                let _ = stream.write_all(response.as_bytes());
            }
        }

        Err(DitriveError::Auth("Failed to receive authorization callback".to_string()))
    }

    /// Parse authorization code from HTTP request
    fn parse_code_from_request(&self, request: &str) -> Option<String> {
        // Request format: GET /?code=xxx&scope=... HTTP/1.1
        let parts: Vec<&str> = request.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }

        let path = parts[1];
        if !path.contains("code=") {
            return None;
        }

        // Parse query parameters
        let query = path.split('?').nth(1)?;
        for param in query.split('&') {
            let mut kv = param.split('=');
            if let (Some("code"), Some(code)) = (kv.next(), kv.next()) {
                return Some(urlencoding::decode(code).ok()?.into_owned());
            }
        }

        None
    }

    /// Exchange authorization code for tokens
    async fn exchange_code(&self, code: &str) -> Result<StoredTokens> {
        let response = self.client
            .post(Self::TOKEN_URL)
            .form(&[
                ("client_id", self.credentials.client_id.as_str()),
                ("client_secret", self.credentials.client_secret.as_str()),
                ("code", code),
                ("grant_type", "authorization_code"),
                ("redirect_uri", self.credentials.redirect_uri.as_str()),
            ])
            .send()
            .await
            .map_err(|e| DitriveError::Auth(format!("Token exchange failed: {}", e)))?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(DitriveError::Auth(format!("Token exchange failed: {}", error)));
        }

        let token_response: TokenResponse = response.json().await
            .map_err(|e| DitriveError::Auth(format!("Failed to parse token response: {}", e)))?;

        let now = chrono::Utc::now().timestamp();
        let tokens = StoredTokens {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token,
            expires_at: now + token_response.expires_in as i64,
            token_type: token_response.token_type,
        };

        self.save_tokens(&tokens)?;
        Ok(tokens)
    }

    /// Refresh an expired access token
    async fn refresh_token(&self, refresh_token: &str) -> Result<StoredTokens> {
        let response = self.client
            .post(Self::TOKEN_URL)
            .form(&[
                ("client_id", self.credentials.client_id.as_str()),
                ("client_secret", self.credentials.client_secret.as_str()),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|e| DitriveError::Auth(format!("Token refresh failed: {}", e)))?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            return Err(DitriveError::Auth(format!("Token refresh failed: {}", error)));
        }

        let token_response: TokenResponse = response.json().await
            .map_err(|e| DitriveError::Auth(format!("Failed to parse token response: {}", e)))?;

        let now = chrono::Utc::now().timestamp();
        let tokens = StoredTokens {
            access_token: token_response.access_token,
            // Keep the old refresh token if new one not provided
            refresh_token: token_response.refresh_token.or_else(|| Some(refresh_token.to_string())),
            expires_at: now + token_response.expires_in as i64,
            token_type: token_response.token_type,
        };

        self.save_tokens(&tokens)?;
        Ok(tokens)
    }

    /// Revoke tokens and clear stored credentials
    pub async fn logout(&self) -> Result<()> {
        if let Ok(tokens) = self.load_tokens() {
            // Revoke the token
            let _ = self.client
                .post("https://oauth2.googleapis.com/revoke")
                .form(&[("token", tokens.access_token.as_str())])
                .send()
                .await;
        }

        // Remove stored tokens
        if self.tokens_path.exists() {
            fs::remove_file(&self.tokens_path)?;
        }

        info!("Logged out and cleared stored tokens");
        Ok(())
    }

    /// Check if user is currently authenticated
    pub fn is_authenticated(&self) -> bool {
        if let Ok(tokens) = self.load_tokens() {
            let now = chrono::Utc::now().timestamp();
            // Has valid token or has refresh token
            tokens.expires_at > now || tokens.refresh_token.is_some()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_code_from_request() {
        let oauth = OAuthManager::new(OAuthCredentials::default());
        
        let request = "GET /?code=4/0AfJohXn...abc123&scope=https://www.googleapis.com/auth/drive HTTP/1.1";
        let code = oauth.parse_code_from_request(request);
        assert!(code.is_some());
        assert!(code.unwrap().starts_with("4/0AfJohXn"));
    }
}
