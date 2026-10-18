//! Collision handling policies and thread-safe reservation registry (F-EXP-04).

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::export::ExportError;

/// Explicit file collision handling policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollisionPolicy {
    #[default]
    Rename,
    Skip,
    Overwrite,
}

impl CollisionPolicy {
    pub fn to_mcp_conflict(&self) -> crate::mcp::tools::ConflictPolicy {
        match self {
            Self::Rename => crate::mcp::tools::ConflictPolicy::Unique,
            Self::Skip => crate::mcp::tools::ConflictPolicy::Skip,
            Self::Overwrite => crate::mcp::tools::ConflictPolicy::Overwrite,
        }
    }
}

/// Resolved action indicating whether to write to a path or skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollisionResolution {
    /// Proceed to write output file at this path.
    Write(PathBuf),
    /// Skip write because destination exists and policy is Skip.
    Skip(PathBuf),
}

/// Thread-safe in-memory reservation registry and filesystem conflict resolver.
#[derive(Debug, Clone, Default)]
pub struct CollisionResolver {
    reserved_paths: Arc<Mutex<HashSet<PathBuf>>>,
}

impl CollisionResolver {
    pub fn new() -> Self {
        Self {
            reserved_paths: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Resolves target destination according to policy and atomically reserves it
    /// to avoid race conditions across concurrent worker threads (T2-EXP04-04).
    pub fn resolve_path(
        &self,
        output_dir: &Path,
        file_stem: &str,
        extension: &str,
        policy: CollisionPolicy,
    ) -> Result<CollisionResolution, ExportError> {
        let clean_ext = extension.trim_start_matches('.');
        let initial_name = format!("{}.{}", file_stem, clean_ext);
        let candidate_path = output_dir.join(&initial_name);

        let mut reserved = self
            .reserved_paths
            .lock()
            .map_err(|_| ExportError::Other("Mutex poisoned in CollisionResolver".into()))?;

        match policy {
            CollisionPolicy::Overwrite => {
                reserved.insert(candidate_path.clone());
                Ok(CollisionResolution::Write(candidate_path))
            }
            CollisionPolicy::Skip => {
                if candidate_path.exists() || reserved.contains(&candidate_path) {
                    Ok(CollisionResolution::Skip(candidate_path))
                } else {
                    reserved.insert(candidate_path.clone());
                    Ok(CollisionResolution::Write(candidate_path))
                }
            }
            CollisionPolicy::Rename => {
                if !candidate_path.exists() && !reserved.contains(&candidate_path) {
                    reserved.insert(candidate_path.clone());
                    return Ok(CollisionResolution::Write(candidate_path));
                }

                // Cascading unique suffix resolution (_1, _2, ..., _10000)
                for counter in 1..=10_000 {
                    let suffix_name = format!("{}_{}.{}", file_stem, counter, clean_ext);
                    let path = output_dir.join(&suffix_name);
                    if !path.exists() && !reserved.contains(&path) {
                        reserved.insert(path.clone());
                        return Ok(CollisionResolution::Write(path));
                    }
                }

                Err(ExportError::CollisionLimitExceeded(candidate_path))
            }
        }
    }

    /// Releases a reserved path when a job fails or is cancelled.
    pub fn release_path(&self, path: &Path) {
        if let Ok(mut reserved) = self.reserved_paths.lock() {
            reserved.remove(path);
        }
    }

    /// Clears all path reservations.
    pub fn clear(&self) {
        if let Ok(mut reserved) = self.reserved_paths.lock() {
            reserved.clear();
        }
    }
}
