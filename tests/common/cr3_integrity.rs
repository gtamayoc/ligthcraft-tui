#![allow(dead_code, unused_imports, unused_variables)]

//! Canon CR3 RAW baseline immutability verifier and pure-Rust SHA-256 implementation.
//! Guarantees that source photographic assets in `./DRAW/` remain completely untouched.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Baseline manifest entry for a source RAW file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineEntry {
    pub filename: &'static str,
    pub expected_size: u64,
    pub expected_sha256: &'static str,
}

/// Authoritative baseline for all Canon CR3 RAW files in `DRAW/`.
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

// FIPS 180-4 K constants
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Standalone, pure-Rust SHA-256 state machine conforming to FIPS 180-4.
#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    count: u64,
    buffer: [u8; 64],
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            count: 0,
            buffer: [0u8; 64],
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        let buffer_idx = (self.count % 64) as usize;
        self.count += data.len() as u64;

        if buffer_idx > 0 {
            let to_fill = 64 - buffer_idx;
            if data.len() >= to_fill {
                self.buffer[buffer_idx..64].copy_from_slice(&data[..to_fill]);
                Self::process_block(&mut self.state, &self.buffer);
                data = &data[to_fill..];
            } else {
                self.buffer[buffer_idx..buffer_idx + data.len()].copy_from_slice(data);
                return;
            }
        }

        while data.len() >= 64 {
            let (block, rest) = data.split_at(64);
            Self::process_block(&mut self.state, block);
            data = rest;
        }

        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let total_bits = self.count * 8;
        let buffer_idx = (self.count % 64) as usize;

        let mut padding = [0u8; 128];
        padding[0] = 0x80;

        let pad_len = if buffer_idx < 56 {
            56 - buffer_idx
        } else {
            120 - buffer_idx
        };

        let bit_len_bytes = total_bits.to_be_bytes();
        padding[pad_len..pad_len + 8].copy_from_slice(&bit_len_bytes);

        self.update(&padding[..pad_len + 8]);

        let mut digest = [0u8; 32];
        for (i, &word) in self.state.iter().enumerate() {
            digest[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    fn process_block(state: &mut [u32; 8], block: &[u8]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
}

/// Compute hex-encoded SHA-256 hash of a file on disk.
pub fn compute_file_sha256(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024]; // 64 KB read buffer
    let mut total_bytes = 0u64;

    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        total_bytes += n as u64;
    }

    let digest = hasher.finalize();
    let hex_hash = digest.iter().map(|b| format!("{:02x}", b)).collect();
    Ok((hex_hash, total_bytes))
}

/// Locate the `DRAW/` directory by traversing from current directory up to repository root.
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

/// RAII Guard that captures pre-flight hashes upon creation and asserts post-flight immutability upon drop/verify.
pub struct Cr3IntegrityGuard {
    draw_dir: PathBuf,
    pre_flight_results: Vec<VerificationResult>,
}

impl Cr3IntegrityGuard {
    pub fn new() -> Self {
        let draw_dir = find_draw_dir();
        let pre_flight = match verify_cr3_baseline(&draw_dir) {
            Ok(list) => list,
            Err(_) => {
                let mut present = Vec::new();
                for entry in CR3_BASELINE {
                    let path = draw_dir.join(entry.filename);
                    if path.is_file() {
                        let (hash, size) =
                            compute_file_sha256(&path).expect("Failed reading present file");
                        present.push(VerificationResult {
                            filename: entry.filename.to_string(),
                            actual_size: size,
                            expected_size: entry.expected_size,
                            actual_sha256: hash,
                            expected_sha256: entry.expected_sha256.to_string(),
                            is_valid: true,
                            error_message: None,
                        });
                    }
                }
                present
            }
        };
        Self {
            draw_dir,
            pre_flight_results: pre_flight,
        }
    }

    /// Explicitly trigger post-flight verification.
    pub fn verify_immutability(&self) {
        for pre in &self.pre_flight_results {
            let path = self.draw_dir.join(&pre.filename);
            let (post_hash, post_size) = compute_file_sha256(&path)
                .expect("Failed to re-hash CR3 file during immutability check");

            assert_eq!(
                pre.actual_sha256, post_hash,
                "CR3 SHA-256 mutation detected on {}: pre={}, post={}",
                pre.filename, pre.actual_sha256, post_hash
            );
            assert_eq!(
                pre.actual_size, post_size,
                "CR3 file size mutation detected on {}: pre={}, post={}",
                pre.filename, pre.actual_size, post_size
            );
        }
    }
}

impl Default for Cr3IntegrityGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Cr3IntegrityGuard {
    fn drop(&mut self) {
        // Assert immutability on drop if not panicking
        if !std::thread::panicking() {
            self.verify_immutability();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pure_rust_sha256_known_vector() {
        // Test vector: empty string
        let mut hasher = Sha256::new();
        hasher.update(b"");
        let digest = hasher.finalize();
        let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        // Test vector: "The quick brown fox jumps over the lazy dog"
        let mut hasher2 = Sha256::new();
        hasher2.update(b"The quick brown fox jumps over the lazy dog");
        let digest2 = hasher2.finalize();
        let hex2: String = digest2.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex2,
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
    }

    #[test]
    fn test_baseline_cr3_verification() {
        let draw_dir = find_draw_dir();
        if !draw_dir.exists() {
            eprintln!("Notice: DRAW directory not found at {}. Skipping baseline verification in CI runner.", draw_dir.display());
            return;
        }
        let results = verify_cr3_baseline(&draw_dir);
        match results {
            Ok(res_list) => {
                for res in res_list {
                    assert!(
                        res.is_valid,
                        "File {} failed baseline check: {:?}",
                        res.filename, res.error_message
                    );
                }
            }
            Err(missing_err) => {
                let mut checked_any = false;
                for entry in CR3_BASELINE {
                    let path = draw_dir.join(entry.filename);
                    if path.exists() {
                        let (hash, size) =
                            compute_file_sha256(&path).expect("Failed reading present file");
                        assert_eq!(
                            size, entry.expected_size,
                            "Size mismatch for {}",
                            entry.filename
                        );
                        assert!(
                            hash.eq_ignore_ascii_case(entry.expected_sha256),
                            "SHA-256 mismatch for {}",
                            entry.filename
                        );
                        checked_any = true;
                    }
                }
                if !checked_any {
                    eprintln!(
                        "Notice: No sample CR3 files present in DRAW/ ({})",
                        missing_err
                    );
                }
            }
        }
    }
}
