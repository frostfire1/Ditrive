//! Git operations and gitignore parsing

use git2::{Repository, Status, StatusOptions};
use glob::Pattern;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use crate::error::Result;

/// Parses and evaluates .gitignore patterns
pub struct GitIgnoreParser {
    repo_path: PathBuf,
    gitignore_path: PathBuf,
    patterns: Vec<(Pattern, bool)>, // (pattern, is_negated)
}

impl GitIgnoreParser {
    /// Create a new GitIgnoreParser for a repository
    pub fn new(repo_path: &Path) -> Self {
        let gitignore_path = repo_path.join(".gitignore");
        let patterns = Self::load_patterns(&gitignore_path).unwrap_or_default();

        Self {
            repo_path: repo_path.to_path_buf(),
            gitignore_path,
            patterns,
        }
    }

    /// Load patterns from .gitignore file
    fn load_patterns(gitignore_path: &Path) -> Result<Vec<(Pattern, bool)>> {
        if !gitignore_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(gitignore_path)?;
        let mut patterns = Vec::new();

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Check if it's a negated pattern
            let (pattern_str, is_negated) = if line.starts_with('!') {
                (&line[1..], true)
            } else {
                (line, false)
            };

            // Convert gitignore pattern to glob pattern
            let glob_pattern = Self::gitignore_to_glob(pattern_str);
            
            if let Ok(pattern) = Pattern::new(&glob_pattern) {
                patterns.push((pattern, is_negated));
            } else {
                warn!("Invalid gitignore pattern: {}", pattern_str);
            }
        }

        Ok(patterns)
    }

    /// Convert a gitignore pattern to a glob pattern
    fn gitignore_to_glob(pattern: &str) -> String {
        let mut glob = pattern.to_string();

        // Handle leading slash (absolute path from repo root)
        if glob.starts_with('/') {
            glob = glob[1..].to_string();
        } else {
            // Pattern can match at any directory level
            glob = format!("**/{}", glob);
        }

        // Handle trailing slash (directory only)
        if glob.ends_with('/') {
            glob = format!("{}**", glob);
        }

        glob
    }

    /// Check if a file is ignored by .gitignore patterns
    pub fn is_ignored(&self, file_path: &Path) -> bool {
        let rel_path = match file_path.strip_prefix(&self.repo_path) {
            Ok(p) => p,
            Err(_) => return false,
        };

        let rel_path_str = rel_path.to_string_lossy().replace('\\', "/");
        let mut is_ignored = false;

        for (pattern, is_negated) in &self.patterns {
            if pattern.matches(&rel_path_str) {
                is_ignored = !is_negated;
            }
        }

        is_ignored
    }

    /// Check if a file is explicitly ignored (exact match in .gitignore)
    pub fn is_explicitly_ignored(&self, file_path: &Path) -> bool {
        let rel_path = match file_path.strip_prefix(&self.repo_path) {
            Ok(p) => p,
            Err(_) => return false,
        };

        let rel_path_str = rel_path.to_string_lossy().replace('\\', "/");

        // Read the gitignore file and check for exact match
        if let Ok(content) = fs::read_to_string(&self.gitignore_path) {
            for line in content.lines() {
                let line = line.trim();
                if !line.starts_with('#') && !line.starts_with('!') && line == rel_path_str {
                    return true;
                }
            }
        }

        false
    }

    /// Reload patterns from .gitignore file
    pub fn reload(&mut self) -> Result<()> {
        self.patterns = Self::load_patterns(&self.gitignore_path)?;
        Ok(())
    }

    /// Add a pattern to .gitignore with an optional comment
    pub fn add_pattern(&mut self, pattern: &str, comment: Option<&str>) -> Result<()> {
        // Read existing content first
        let mut content = if self.gitignore_path.exists() {
            fs::read_to_string(&self.gitignore_path)?
        } else {
            String::new()
        };

        // Check if pattern already exists
        if content.lines().any(|line| line.trim() == pattern) {
            return Ok(());
        }

        // Ensure newline at end
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }

        // Add comment if provided
        if let Some(c) = comment {
            content.push_str(&format!("# {}\n", c));
        }

        // Add pattern
        content.push_str(&format!("{}\n", pattern));

        fs::write(&self.gitignore_path, content)?;
        debug!("Added pattern '{}' to .gitignore", pattern);

        self.reload()?;
        Ok(())
    }
}

/// Git repository manager
pub struct GitManager {
    repo: Repository,
    repo_path: PathBuf,
}

impl GitManager {
    /// Open an existing Git repository
    pub fn open(repo_path: &Path) -> Result<Self> {
        let repo = Repository::open(repo_path)?;
        Ok(Self {
            repo,
            repo_path: repo_path.to_path_buf(),
        })
    }

    /// Initialize a new Git repository
    pub fn init(repo_path: &Path) -> Result<Self> {
        let repo = Repository::init(repo_path)?;
        info!("Initialized Git repository at {:?}", repo_path);
        Ok(Self {
            repo,
            repo_path: repo_path.to_path_buf(),
        })
    }

    /// Open or initialize a Git repository
    pub fn open_or_init(repo_path: &Path) -> Result<Self> {
        if repo_path.join(".git").exists() {
            Self::open(repo_path)
        } else {
            Self::init(repo_path)
        }
    }

    /// Get the repository path
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    /// Get a reference to the underlying repository
    pub fn repository(&self) -> &Repository {
        &self.repo
    }

    /// Get all tracked files in the repository
    pub fn get_tracked_files(&self) -> Result<HashSet<PathBuf>> {
        let head = match self.repo.head() {
            Ok(h) => h,
            Err(_) => return Ok(HashSet::new()), // No commits yet
        };

        let tree = head.peel_to_tree()?;
        let mut files = HashSet::new();

        tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob) {
                let path = PathBuf::from(format!("{}{}", dir, entry.name().unwrap_or("")));
                files.insert(path);
            }
            git2::TreeWalkResult::Ok
        })?;

        Ok(files)
    }

    /// Get untracked files in the repository
    pub fn get_untracked_files(&self) -> Result<Vec<PathBuf>> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true);

        let statuses = self.repo.statuses(Some(&mut opts))?;
        let mut untracked = Vec::new();

        for entry in statuses.iter() {
            if entry.status().contains(Status::WT_NEW) {
                if let Some(path) = entry.path() {
                    untracked.push(PathBuf::from(path));
                }
            }
        }

        Ok(untracked)
    }

    /// Stage files for commit
    pub fn stage_files(&self, paths: &[&str]) -> Result<()> {
        let mut index = self.repo.index()?;
        
        for path in paths {
            index.add_path(Path::new(path))?;
        }

        index.write()?;
        debug!("Staged {} files", paths.len());
        Ok(())
    }

    /// Create a commit with staged changes
    pub fn commit(&self, message: &str) -> Result<git2::Oid> {
        let signature = self.repo.signature()?;
        let mut index = self.repo.index()?;
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;

        let parent = match self.repo.head() {
            Ok(head) => Some(head.peel_to_commit()?),
            Err(_) => None,
        };

        let parents: Vec<&git2::Commit> = parent.iter().collect();

        let oid = self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )?;

        info!("Created commit: {}", oid);
        Ok(oid)
    }

    /// Set the remote URL with authentication
    pub fn set_remote_url(&self, name: &str, url: &str) -> Result<()> {
        if self.repo.find_remote(name).is_ok() {
            self.repo.remote_set_url(name, url)?;
            debug!("Updated remote '{}' URL", name);
        } else {
            self.repo.remote(name, url)?;
            debug!("Created remote '{}' with URL: {}", name, url);
        }
        Ok(())
    }

    /// Configure user name and email
    pub fn configure_user(&self, name: &str, email: &str) -> Result<()> {
        let mut config = self.repo.config()?;
        config.set_str("user.name", name)?;
        config.set_str("user.email", email)?;
        debug!("Configured user: {} <{}>", name, email);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_gitignore_parser_basic() {
        let dir = tempdir().unwrap();
        let gitignore_path = dir.path().join(".gitignore");
        fs::write(&gitignore_path, "*.log\nnode_modules/\n").unwrap();

        let parser = GitIgnoreParser::new(dir.path());
        
        let log_file = dir.path().join("test.log");
        assert!(parser.is_ignored(&log_file));
    }

    #[test]
    fn test_git_manager_init() {
        let dir = tempdir().unwrap();
        let manager = GitManager::init(dir.path()).unwrap();
        assert!(dir.path().join(".git").exists());
    }
}
