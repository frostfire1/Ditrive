//! Woilah tracker for managing file mappings to Google Drive

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use crate::drive::{calculate_file_hash, FileMetadata};
use crate::error::Result;

const WOILAH_FILENAME: &str = ".woilah";

/// Manages .woilah files for tracking Drive file mappings
pub struct WoilahTracker {
    repo_path: PathBuf,
}

impl WoilahTracker {
    /// Create a new WoilahTracker
    pub fn new(repo_path: &Path) -> Self {
        Self {
            repo_path: repo_path.to_path_buf(),
        }
    }

    /// Get the path to the .woilah file in a specific folder
    fn woilah_path(&self, folder_path: &Path) -> PathBuf {
        folder_path.join(WOILAH_FILENAME)
    }

    /// Read the .woilah file in a folder
    pub fn read_woilah_file(&self, folder_path: &Path) -> Result<HashMap<String, FileMetadata>> {
        let woilah_path = self.woilah_path(folder_path);

        if !woilah_path.exists() {
            return Ok(HashMap::new());
        }

        let content = fs::read_to_string(&woilah_path)?;

        // Handle both old format (string values) and new format (metadata objects)
        match serde_json::from_str::<HashMap<String, serde_json::Value>>(&content) {
            Ok(raw_data) => {
                let mut result = HashMap::new();

                for (filename, value) in raw_data {
                    let metadata = if value.is_string() {
                        // Old format: just the file ID as a string
                        FileMetadata {
                            id: value.as_str().unwrap_or_default().to_string(),
                            hash: String::new(),
                            size: 0,
                            uploaded_at: 0,
                        }
                    } else {
                        // New format: full metadata object
                        serde_json::from_value(value).unwrap_or_else(|_| FileMetadata {
                            id: String::new(),
                            hash: String::new(),
                            size: 0,
                            uploaded_at: 0,
                        })
                    };

                    result.insert(filename, metadata);
                }

                Ok(result)
            }
            Err(e) => {
                warn!("Failed to parse .woilah file at {:?}: {}", woilah_path, e);
                Ok(HashMap::new())
            }
        }
    }

    /// Write the .woilah file in a folder
    pub fn write_woilah_file(
        &self,
        folder_path: &Path,
        mappings: &HashMap<String, FileMetadata>,
    ) -> Result<()> {
        let woilah_path = self.woilah_path(folder_path);
        let content = serde_json::to_string_pretty(mappings)?;
        fs::write(&woilah_path, content)?;
        debug!("Updated .woilah file at {:?}", woilah_path);
        Ok(())
    }

    /// Add a file mapping to the .woilah file
    pub fn add_file_mapping(
        &self,
        folder_path: &Path,
        filename: &str,
        metadata: FileMetadata,
    ) -> Result<()> {
        let mut mappings = self.read_woilah_file(folder_path)?;
        mappings.insert(filename.to_string(), metadata);
        self.write_woilah_file(folder_path, &mappings)
    }

    /// Remove a file mapping from the .woilah file
    pub fn remove_file_mapping(&self, folder_path: &Path, filename: &str) -> Result<()> {
        let mut mappings = self.read_woilah_file(folder_path)?;
        if mappings.remove(filename).is_some() {
            self.write_woilah_file(folder_path, &mappings)?;
            debug!("Removed mapping for '{}'", filename);
        }
        Ok(())
    }

    /// Get the file metadata for a local file
    pub fn get_file_info(&self, folder_path: &Path, filename: &str) -> Result<Option<FileMetadata>> {
        let mappings = self.read_woilah_file(folder_path)?;
        Ok(mappings.get(filename).cloned())
    }

    /// Get the Drive file ID for a local file
    pub fn get_file_id(&self, folder_path: &Path, filename: &str) -> Result<Option<String>> {
        Ok(self.get_file_info(folder_path, filename)?.map(|m| m.id))
    }

    /// Check if a file is managed by woilah
    pub fn is_managed(&self, file_path: &Path) -> Result<bool> {
        let folder_path = file_path.parent().unwrap_or(file_path);
        let filename = file_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        Ok(self.get_file_id(folder_path, &filename)?.is_some())
    }

    /// Check if a file needs to be re-uploaded (hash changed)
    pub fn file_needs_update(&self, file_path: &Path) -> Result<bool> {
        let folder_path = file_path.parent().unwrap_or(file_path);
        let filename = file_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let info = self.get_file_info(folder_path, &filename)?;

        match info {
            Some(metadata) if !metadata.hash.is_empty() => {
                let current_hash = calculate_file_hash(file_path)?;
                Ok(current_hash != metadata.hash)
            }
            _ => Ok(true), // File not tracked or no hash stored
        }
    }

    /// Get all managed files in the repository
    pub fn get_all_managed_files(&self) -> Result<Vec<(PathBuf, FileMetadata)>> {
        let mut result = Vec::new();

        for entry in walkdir::WalkDir::new(&self.repo_path)
            .into_iter()
            .filter_entry(|e| !e.path().starts_with(self.repo_path.join(".git")))
        {
            let entry = entry?;
            if entry.file_name() == WOILAH_FILENAME {
                let folder_path = entry.path().parent().unwrap_or(entry.path());
                let mappings = self.read_woilah_file(folder_path)?;

                for (filename, metadata) in mappings {
                    let file_path = folder_path.join(&filename);
                    result.push((file_path, metadata));
                }
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_woilah_tracker_add_and_get() {
        let dir = tempdir().unwrap();
        let tracker = WoilahTracker::new(dir.path());

        let metadata = FileMetadata {
            id: "test-id-123".to_string(),
            hash: "abc123".to_string(),
            size: 1024,
            uploaded_at: 1234567890,
        };

        tracker
            .add_file_mapping(dir.path(), "test.bin", metadata.clone())
            .unwrap();

        let retrieved = tracker.get_file_info(dir.path(), "test.bin").unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "test-id-123");
    }

    #[test]
    fn test_woilah_tracker_remove() {
        let dir = tempdir().unwrap();
        let tracker = WoilahTracker::new(dir.path());

        let metadata = FileMetadata {
            id: "test-id".to_string(),
            hash: String::new(),
            size: 0,
            uploaded_at: 0,
        };

        tracker
            .add_file_mapping(dir.path(), "test.bin", metadata)
            .unwrap();
        tracker.remove_file_mapping(dir.path(), "test.bin").unwrap();

        let retrieved = tracker.get_file_info(dir.path(), "test.bin").unwrap();
        assert!(retrieved.is_none());
    }
}
