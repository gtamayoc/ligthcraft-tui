//! Cryptographic immutability verifier and non-destructive safety guard.
//!
//! Guarantees that source photographic assets (especially Canon .CR3 RAW files in `./DRAW/`)
//! remain 100% byte-for-byte untouched across all preview, develop, and export operations.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::error::IntegrityViolationError;

/// Baseline manifest entry for an immutable source RAW file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineEntry {
    pub filename: &'static str,
    pub expected_size: u64,
    pub expected_sha256: &'static str,
}

/// Authoritative cryptographic baseline for Canon CR3 files in `./DRAW/`.
pub const CR3_BASELINE: &[BaselineEntry] = &[
    BaselineEntry {
        filename: "IMG_0588.CR3",
        expected_size: 26_527_598,
        expected_sha256: "e6d7ac9abc64b6c7a4e16f829839c7969f6cc61cab8068ffc4e02c264fc437ae",
    },
    BaselineEntry {
        filename: "IMG_0589.CR3",
        expected_size: 28_369_647,
        expected_sha256: "530d4c07bb272f4cd42eafe4518c957f59cf8def72b2cfda103e10b31da0f50e",
    },
    BaselineEntry {
        filename: "IMG_0590.CR3",
        expected_size: 27_704_456,
        expected_sha256: "e814e953874e09fed4d6549ffcde9608061a0571bda466927d6af719b1a4d8fe",
    },
    BaselineEntry {
        filename: "IMG_0969.CR3",
        expected_size: 26_614_589,
        expected_sha256: "a640c4f5b34b45ed5a953d4e74497bcbc29416e7e2c9a772f0d52abad8b9f5b2",
    },
    BaselineEntry {
        filename: "IMG_0981.CR3",
        expected_size: 29_087_028,
        expected_sha256: "471a29115b1ae58b8fccff0d61308cbe440112403bc9f79e1bd5e70a57595514",
    },
];

/// Computes the SHA-256 hex digest and size of a file using buffered streaming I/O.
pub fn compute_file_sha256(path: &Path) -> io::Result<(String, u64)> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total_bytes = 0u64;

    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        total_bytes += n as u64;
    }

    let digest = hasher.finalize();
    let hex_string: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    Ok((hex_string, total_bytes))
}

/// Locate the `DRAW/` directory by checking current directory and parents.
pub fn find_draw_dir() -> PathBuf {
    let mut candidate = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for _ in 0..5 {
        let draw_path = candidate.join("DRAW");
        if draw_path.is_dir() {
            return draw_path;
        }
        if let Some(parent) = candidate.parent() {
            candidate = parent.to_path_buf();
        } else {
            break;
        }
    }
    PathBuf::from("DRAW")
}

/// Result of checking a single CR3 file against baseline.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub filename: String,
    pub actual_size: u64,
    pub expected_size: u64,
    pub actual_sha256: String,
    pub expected_sha256: String,
    pub is_valid: bool,
    pub error_message: Option<String>,
}

/// Verify all CR3 files in `DRAW/` against the authoritative cryptographic baseline.
pub fn verify_cr3_baseline(draw_dir: &Path) -> Result<Vec<VerificationResult>, String> {
    let mut results = Vec::new();
    let mut failures = Vec::new();

    for entry in CR3_BASELINE {
        let file_path = draw_dir.join(entry.filename);
        if !file_path.exists() {
            let msg = format!("CR3 file missing: {}", file_path.display());
            failures.push(msg.clone());
            results.push(VerificationResult {
                filename: entry.filename.to_string(),
                actual_size: 0,
                expected_size: entry.expected_size,
                actual_sha256: String::new(),
                expected_sha256: entry.expected_sha256.to_string(),
                is_valid: false,
                error_message: Some(msg),
            });
            continue;
        }

        match compute_file_sha256(&file_path) {
            Ok((hash, size)) => {
                let size_ok = size == entry.expected_size;
                let hash_ok = hash.eq_ignore_ascii_case(entry.expected_sha256);
                let valid = size_ok && hash_ok;

                let err = if !valid {
                    let msg = format!(
                        "CR3 INTEGRITY VIOLATION for {}: expected size={}, actual={}; expected sha256={}, actual={}",
                        entry.filename, entry.expected_size, size, entry.expected_sha256, hash
                    );
                    failures.push(msg.clone());
                    Some(msg)
                } else {
                    None
                };

                results.push(VerificationResult {
                    filename: entry.filename.to_string(),
                    actual_size: size,
                    expected_size: entry.expected_size,
                    actual_sha256: hash,
                    expected_sha256: entry.expected_sha256.to_string(),
                    is_valid: valid,
                    error_message: err,
                });
            }
            Err(e) => {
                let msg = format!("Failed to read CR3 file {}: {}", file_path.display(), e);
                failures.push(msg.clone());
                results.push(VerificationResult {
                    filename: entry.filename.to_string(),
                    actual_size: 0,
                    expected_size: entry.expected_size,
                    actual_sha256: String::new(),
                    expected_sha256: entry.expected_sha256.to_string(),
                    is_valid: false,
                    error_message: Some(msg),
                });
            }
        }
    }

    if failures.is_empty() {
        Ok(results)
    } else {
        Err(failures.join("\n"))
    }
}

/// Cryptographic snapshot of a single photographic source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileRecord {
    pub canonical_path: PathBuf,
    pub filename: String,
    pub size_bytes: u64,
    pub sha256_hex: String,
    pub modified_time: SystemTime,
    pub is_read_only: bool,
}

/// Baseline source manifest capturing the state of all protected files.
#[derive(Debug, Clone)]
pub struct SourceManifest {
    pub session_id: String,
    pub created_at: SystemTime,
    pub records: HashMap<PathBuf, SourceFileRecord>,
}

/// Core safety engine providing pre/post-flight hashing, import filtering,
/// and output destination containment.
#[derive(Debug, Clone)]
pub struct IntegrityEngine {
    pub source_directories: Vec<PathBuf>,
    pub is_poisoned: Arc<AtomicBool>,
    pub strict_mode: bool,
}

impl IntegrityEngine {
    /// Tolerance for FAT/FAT32 filesystems with 2-second timestamp resolution.
    pub const FAT_TIMESTAMP_TOLERANCE: Duration = Duration::from_secs(2);

    /// Initialize safety engine for a set of source directories.
    pub fn new(source_dirs: Vec<PathBuf>, strict_mode: bool) -> Self {
        Self {
            source_directories: source_dirs,
            is_poisoned: Arc::new(AtomicBool::new(false)),
            strict_mode,
        }
    }

    /// Check if the safety engine has been poisoned by an earlier violation.
    pub fn is_poisoned(&self) -> bool {
        self.is_poisoned.load(Ordering::SeqCst)
    }

    /// Poison the engine, permanently disabling further mutating calls.
    pub fn poison(&self) {
        self.is_poisoned.store(true, Ordering::SeqCst);
    }

    /// Capture a cryptographic pre-flight manifest across all configured source directories.
    pub fn capture_preflight(&self) -> Result<SourceManifest, IntegrityViolationError> {
        if self.is_poisoned() {
            return Err(IntegrityViolationError::SessionPoisoned);
        }

        let mut records = HashMap::new();
        for dir in &self.source_directories {
            if dir.is_dir() {
                self.scan_directory_records(dir, &mut records)?;
            }
        }

        Ok(SourceManifest {
            session_id: format!("mcp_session_{}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis()),
            created_at: SystemTime::now(),
            records,
        })
    }

    fn scan_directory_records(
        &self,
        dir: &Path,
        records: &mut HashMap<PathBuf, SourceFileRecord>,
    ) -> Result<(), IntegrityViolationError> {
        let entries = std::fs::read_dir(dir).map_err(IntegrityViolationError::Io)?;
        for entry in entries {
            let entry = entry.map_err(IntegrityViolationError::Io)?;
            let path = entry.path();
            if path.is_file() {
                let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
                let meta = std::fs::metadata(&path).map_err(IntegrityViolationError::Io)?;
                let (sha256_hex, size_bytes) = compute_file_sha256(&path).map_err(IntegrityViolationError::Io)?;
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                records.insert(
                    canonical.clone(),
                    SourceFileRecord {
                        canonical_path: canonical,
                        filename,
                        size_bytes,
                        sha256_hex,
                        modified_time: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        is_read_only: meta.permissions().readonly(),
                    },
                );
            }
        }
        Ok(())
    }

    /// Register an individual source file into an active manifest.
    pub fn register_file(
        &self,
        path: &Path,
        manifest: &mut SourceManifest,
    ) -> Result<(), IntegrityViolationError> {
        let canonical = path
            .canonicalize()
            .map_err(|_| IntegrityViolationError::FileNotFound(path.to_path_buf()))?;
        let meta = std::fs::metadata(path)
            .map_err(|_| IntegrityViolationError::FileNotFound(path.to_path_buf()))?;
        let (sha256_hex, size_bytes) =
            compute_file_sha256(path).map_err(IntegrityViolationError::Io)?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        manifest.records.insert(
            canonical.clone(),
            SourceFileRecord {
                canonical_path: canonical,
                filename,
                size_bytes,
                sha256_hex,
                modified_time: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                is_read_only: meta.permissions().readonly(),
            },
        );
        Ok(())
    }

    /// Re-hash and verify that all source files in the manifest remain 100% byte-for-byte identical.
    pub fn verify_manifest(&self, manifest: &SourceManifest) -> Result<(), IntegrityViolationError> {
        let tolerance = if self.strict_mode {
            Duration::ZERO
        } else {
            Self::FAT_TIMESTAMP_TOLERANCE
        };
        self.verify_manifest_with_tolerance(manifest, tolerance)
    }

    /// Re-hash and verify manifest with an explicit timestamp tolerance.
    pub fn verify_manifest_with_tolerance(
        &self,
        manifest: &SourceManifest,
        tolerance: Duration,
    ) -> Result<(), IntegrityViolationError> {
        if self.is_poisoned() {
            return Err(IntegrityViolationError::SessionPoisoned);
        }

        for (path, record) in &manifest.records {
            if !path.exists() {
                self.poison();
                return Err(IntegrityViolationError::FileNotFound(path.clone()));
            }

            let meta = std::fs::metadata(path).map_err(IntegrityViolationError::Io)?;
            let current_size = meta.len();
            if current_size != record.size_bytes {
                self.poison();
                return Err(IntegrityViolationError::SizeMismatch {
                    path: path.clone(),
                    expected_bytes: record.size_bytes,
                    actual_bytes: current_size,
                });
            }

            // Timestamp modification check
            let current_mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let mtime_diff = if current_mtime >= record.modified_time {
                current_mtime.duration_since(record.modified_time).unwrap_or_default()
            } else {
                record.modified_time.duration_since(current_mtime).unwrap_or_default()
            };

            if mtime_diff > tolerance {
                self.poison();
                return Err(IntegrityViolationError::TimestampMismatch {
                    path: path.clone(),
                    expected: record.modified_time,
                    actual: current_mtime,
                });
            }

            let (current_hash, _) = compute_file_sha256(path).map_err(IntegrityViolationError::Io)?;
            if !current_hash.eq_ignore_ascii_case(&record.sha256_hex) {
                self.poison();
                return Err(IntegrityViolationError::HashMismatch {
                    path: path.clone(),
                    expected: record.sha256_hex.clone(),
                    actual: current_hash,
                });
            }
        }

        Ok(())
    }

    /// Validate that an import request strictly uses mode: "add" (or None, defaulting to "add").
    pub fn validate_import_mode(&self, mode: Option<&str>) -> Result<(), IntegrityViolationError> {
        match mode {
            None | Some("add") => Ok(()),
            Some(illegal) => {
                self.poison();
                Err(IntegrityViolationError::ProhibitedImportMode {
                    mode: illegal.to_string(),
                })
            }
        }
    }

    /// Validate that an export or rendered photo target path does not lie inside any source folder,
    /// contains no lexical path traversal sequences ('..'), and does not target prohibited .CR3 formats.
    pub fn validate_destination_path(&self, target_path: &Path) -> Result<PathBuf, IntegrityViolationError> {
        // 1. Lexical parent traversal check before any filesystem access
        for comp in target_path.components() {
            if matches!(comp, std::path::Component::ParentDir) {
                self.poison();
                return Err(IntegrityViolationError::PathTraversal {
                    target: target_path.to_path_buf(),
                });
            }
        }

        let path_str = target_path.to_string_lossy();
        if path_str.contains("..") || path_str.contains("../") || path_str.contains("..\\")
            || path_str.contains("%2e%2e") || path_str.contains("%2E%2E")
        {
            self.poison();
            return Err(IntegrityViolationError::PathTraversal {
                target: target_path.to_path_buf(),
            });
        }

        // 2. Unconditional veto against any target path ending with .cr3 or .CR3
        self.check_extension_veto(target_path)?;

        // 3. Normalization and source directory containment check
        let normalized_dest = Self::normalize_path(target_path);

        for source_dir in &self.source_directories {
            let normalized_src = Self::normalize_path(source_dir);

            if Self::is_subpath(&normalized_dest, &normalized_src) {
                self.poison();
                return Err(IntegrityViolationError::SourceDirectoryContamination {
                    path: normalized_dest,
                    source_dir: normalized_src,
                });
            }
        }

        Ok(normalized_dest)
    }

    /// Checks whether `dest` is equal to or resides inside `src`, respecting path component boundaries.
    pub fn is_subpath(dest: &Path, src: &Path) -> bool {
        let dest_comps: Vec<_> = dest.components().collect();
        let src_comps: Vec<_> = src.components().collect();

        if dest_comps.len() < src_comps.len() {
            return false;
        }

        for (d, s) in dest_comps.iter().zip(src_comps.iter()) {
            #[cfg(windows)]
            {
                let d_str = d.as_os_str().to_string_lossy();
                let s_str = s.as_os_str().to_string_lossy();
                if !d_str.eq_ignore_ascii_case(&s_str) {
                    return false;
                }
            }
            #[cfg(not(windows))]
            {
                if d != s {
                    return false;
                }
            }
        }
        true
    }

    /// Unconditionally rejects any target path targeting a .CR3 / .cr3 source extension.
    pub fn check_extension_veto(&self, target: &Path) -> Result<(), IntegrityViolationError> {
        if let Some(ext) = target.extension().and_then(|e| e.to_str()) {
            if ext.eq_ignore_ascii_case("cr3") {
                self.poison();
                return Err(IntegrityViolationError::ProhibitedTargetExtension {
                    path: target.to_path_buf(),
                    extension: ext.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Normalizes path for cross-platform comparison, resolving ancestors for non-existent files
    /// and stripping Windows verbatim prefixes (\\?\).
    pub fn normalize_path(p: &Path) -> PathBuf {
    let abs = if p.is_relative() {
        if let Ok(cwd) = std::env::current_dir() {
            cwd.join(p)
        } else {
            p.to_path_buf()
        }
    } else {
        p.to_path_buf()
    };

    let can = if let Ok(c) = abs.canonicalize() {
        c
    } else if let Some(parent) = abs.parent() {
        if let Ok(c_parent) = parent.canonicalize() {
            if let Some(file_name) = abs.file_name() {
                c_parent.join(file_name)
            } else {
                c_parent
            }
        } else {
            abs
        }
    } else {
        abs
    };

    let s = can.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        can
    }
}

    /// Audit a source directory to verify no unexpected sidecars (*.xmp) or temporary files were written.
    pub fn audit_source_directory(
        &self,
        dir: &Path,
        baseline_files: &[String],
    ) -> Result<(), IntegrityViolationError> {
        let entries = std::fs::read_dir(dir).map_err(IntegrityViolationError::Io)?;
        for entry in entries {
            let entry = entry.map_err(IntegrityViolationError::Io)?;
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if !baseline_files.iter().any(|b| b.eq_ignore_ascii_case(name)) {
                    self.poison();
                    return Err(IntegrityViolationError::DirectoryContaminated {
                        stray_path: path.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// RAII session guard that verifies source integrity on drop if not disarmed.
pub struct IntegritySessionGuard<'a> {
    pub engine: &'a IntegrityEngine,
    pub manifest: SourceManifest,
    pub disarmed: bool,
}

impl<'a> IntegritySessionGuard<'a> {
    pub fn new(engine: &'a IntegrityEngine) -> Result<Self, IntegrityViolationError> {
        let manifest = engine.capture_preflight()?;
        Ok(Self {
            engine,
            manifest,
            disarmed: false,
        })
    }

    pub fn verify_now(&mut self) -> Result<(), IntegrityViolationError> {
        self.engine.verify_manifest(&self.manifest)
    }

    pub fn disarm(mut self) {
        self.disarmed = true;
    }
}

impl<'a> Drop for IntegritySessionGuard<'a> {
    fn drop(&mut self) {
        if !self.disarmed && !std::thread::panicking() {
            if let Err(e) = self.engine.verify_manifest(&self.manifest) {
                panic!("FATAL CR3 IMMUTABILITY VIOLATION ON DROP: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_known_vector() {
        let mut hasher = Sha256::new();
        hasher.update(b"The quick brown fox jumps over the lazy dog");
        let digest = hasher.finalize();
        let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
    }

    #[test]
    fn test_draw_cr3_baseline_verification() {
        let draw_dir = find_draw_dir();
        assert!(draw_dir.exists(), "DRAW directory must exist");
        let results = verify_cr3_baseline(&draw_dir).expect("Baseline verification must pass");
        assert_eq!(results.len(), 5);
        for r in results {
            assert!(r.is_valid, "File {} must match baseline", r.filename);
        }
    }

    #[test]
    fn test_import_mode_enforcement() {
        let engine = IntegrityEngine::new(vec![find_draw_dir()], true);
        assert!(engine.validate_import_mode(None).is_ok());
        assert!(engine.validate_import_mode(Some("add")).is_ok());
        assert!(engine.validate_import_mode(Some("move")).is_err());
        assert!(engine.validate_import_mode(Some("copy")).is_err());
    }

    #[test]
    fn test_destination_path_sandboxing() {
        let draw_dir = find_draw_dir();
        let engine = IntegrityEngine::new(vec![draw_dir.clone()], true);

        // Export directly into DRAW should be blocked
        let forbidden = draw_dir.join("output.jpg");
        assert!(engine.validate_destination_path(&forbidden).is_err());

        // Export to a clean separate folder should pass
        let target_dir = PathBuf::from("target").join("test_out").join("output.jpg");
        assert!(engine.validate_destination_path(&target_dir).is_ok());

        // Lexical path traversal check
        let traversal_target = PathBuf::from("target").join("..").join("output.jpg");
        assert!(matches!(
            engine.validate_destination_path(&traversal_target),
            Err(IntegrityViolationError::PathTraversal { .. })
        ));

        // Prohibited .cr3 extension veto
        let cr3_target = PathBuf::from("target").join("test_out").join("output.CR3");
        assert!(matches!(
            engine.validate_destination_path(&cr3_target),
            Err(IntegrityViolationError::ProhibitedTargetExtension { .. })
        ));
    }
}

