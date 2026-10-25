# TEST_INFRA.md: LightCraft SD Ingest Hub & Modular Launcher Test Infrastructure

## 1. Test Philosophy & Principles

### 1.1 Opaque-Box & Requirement-Driven Verification
The testing strategy for the LightCraft SD Ingest Hub and Modular Launcher is **opaque-box** and **strictly requirement-driven**. Tests exercise public interfaces, CLI arguments, filesystem layouts, cryptographic hashes, and configuration artifacts without asserting private implementation details. Every test case maps directly to user stories and functional requirements specified in `tasks/prd-sd-ingest-hub.md`, `ORIGINAL_REQUEST.md`, and `PROJECT.md`.

### 1.2 Strict 100% Read-Only SD Protocol Invariant
A foundational requirement of the system (R3, FR-3) is that removable SD cards must be treated as immutable read-only media. To prove compliance:
- Tests verify source directories and photo files remain bit-for-bit identical before and after all ingest operations.
- Cryptographic SHA-256 trees of the mock SD cards are computed before and compared after execution.
- Source files are opened strictly in read-only mode (`File::open` or equivalent `std::fs::OpenOptions::new().read(true).write(false)`).
- Tests simulate physical write-lock switches by setting filesystem read-only permissions on source files and directories, ensuring the pipeline succeeds without encountering permission errors.

### 1.3 Authoritative Derivation of Expected Outputs
In compliance with the expected output derivation policy:
1. **Cryptographic Integrity**: Expected SHA-256 digests are computed in-memory using standard FIPS 180-4 algorithms and compared against both the in-flight stream hash and the saved session manifest.
2. **EXIF Fingerprinting**: Authentic EXIF metadata tags (`Tag::Make`, `Tag::Model`, and `Tag(Context::Exif, 0xa431)` for `BodySerialNumber`) are generated conforming to the TIFF/EXIF 2.3 specification.
3. **Persistence State**: Registry schemas (`~/.lightcraft/camera_registry.toml`) are validated against authoritative TOML serialization models.
4. **Volume Calculations**: Volume free vs. total capacity boundaries (including the 99% margin check for formatted cards) are tested against explicit integer arithmetic.

---

## 2. Test Architecture & Harness

### 2.1 Directory Layout
```
tests/
├── common/                               # Test infrastructure & synthetic fixtures
│   ├── mod.rs                            # Re-exports of common test utilities
│   ├── contracts.rs                      # Interface contracts (ToolOption, SdCardVolume, CameraProfile, etc.)
│   ├── sd_fixture.rs                     # Synthetic SD card directory builder (tempfile, DCIM, capacities)
│   ├── exif_gen.rs                       # Authentic EXIF binary header & JPEG/TIFF generator
│   ├── registry_fixture.rs               # Isolated camera registry TOML manager
│   ├── pipeline_harness.rs               # Streaming ingest executor & mock Windows toast notifier
│   ├── cr3_integrity.rs                  # CR3 RAW cryptographic immutability verifier
│   └── mock_mcp.rs                       # Mock LightCraft MCP server for studio fallback
├── tier1_feature_tests.rs                # Tier 1: Feature unit & integration tests (>=5 tests per feature)
├── tier2_boundary_tests.rs               # Tier 2: Boundary, corner cases & fault injection tests
├── tier3_pairwise_tests.rs               # Tier 3: Pairwise cross-feature combinations
└── tier4_scenario_tests.rs               # Tier 4: Realistic end-to-end user workflows
```

### 2.2 Synthetic SD Card Fixture Engine
The test harness builds temporary synthetic SD card filesystems in `tempfile::tempdir()`:
- **Filesystem Simulation**: Configurable volume labels (`"EOS_DIGITAL"`, `"LUMIX"`, `"NIKON"`), file systems (`"exFAT"`, `"FAT32"`), drive letters, total capacities (e.g. 32 GB, 64 GB, 128 GB, 256 GB), and free space ratios.
- **DCIM Hierarchy**: Generates standard DCIM folder structures (e.g., `DCIM/100CANON/`, `DCIM/101EOSR6/`, `DCIM/100MSDCF/`).
- **Media Generator**: Generates authentic mock photos (`.CR3`, `.CR2`, `.ARW`, `.NEF`, `.JPG`) containing valid EXIF metadata:
  - `Tag::Make` (e.g. `"Canon"`, `"Sony"`, `"Nikon"`)
  - `Tag::Model` (e.g. `"Canon EOS R6 Mark II"`, `"ILCE-7M4"`)
  - `Tag::BodySerialNumber` (`0xa431`, e.g. `"042021000123"`)
  - `Tag::DateTimeOriginal` for folder hierarchy dispatch (`{target}/{YYYY}/{YYYY-MM-DD}/`)
- **Empty Card Emulation**: Configures volumes with 0 photos in `DCIM/` and `free_bytes == total_bytes` (or within the 99% formatted filesystem margin).

### 2.3 Isolated Registry Fixture
All tests isolate their registry files inside a dedicated temporary directory (`tempfile::tempdir()`), avoiding any modification or reliance upon the host user's `~/.lightcraft/camera_registry.toml`.

---

## 3. Test Tier Definitions & Pass/Fail Semantics

### 3.1 Pass/Fail Criteria
- **Exit Code**: Test process exits with code 0.
- **Immutability Invariant**: 0 bytes modified, created, or deleted on the synthetic SD card mount path.
- **Data Integrity**: 100% of copied files match the SHA-256 hash computed at source.
- **Manifest Audit**: `ingest_manifest_{timestamp}.json` is present in destination, contains valid JSON, and records accurate counts and hash mappings.
- **Zero Panic**: No unhandled panics, unwrap failures, or hanging asynchronous tasks.

---

## 4. Complete Feature Coverage Mapping across Tiers 1–4

| Feature ID | Feature Name | Tier 1 (Feature) | Tier 2 (Boundary) | Tier 3 (Pairwise) | Tier 4 (Scenario) |
|---|---|---|---|---|---|
| F-01 | Interactive Tool Selector | `test_t1_feat01_*` (6 tests) | - | `test_t3_pairwise_cli_bypass_*` | `test_t4_scenario_*` |
| F-02 | CLI Flag Direct Bypass | `test_t1_feat01_cli_bypass_*` | `test_t2_boundary_invalid_cli_flag` | `test_t3_pairwise_cli_bypass_*` | `test_t4_scenario_*` |
| F-03 | Windows Removable Drive Discovery | `test_t1_feat02_mock_volume_*` | `test_t2_boundary_no_removable_drives` | `test_t3_pairwise_multiple_cameras_*` | `test_t4_scenario_photographer_*` |
| F-04 | DCIM Structure & Media Crawler | `test_t1_feat02_dcim_*` | `test_t2_boundary_nested_dcim_*` | `test_t3_pairwise_repeat_insertion_*` | `test_t4_scenario_photographer_*` |
| F-05 | Empty Card Detection Logic | `test_t1_feat06_empty_card_*` | `test_t2_boundary_exact_99_percent_*` | `test_t3_pairwise_formatted_card_*` | `test_t4_scenario_formatted_card_*` |
| F-06 | Bounded Header EXIF Reader | `test_t1_feat03_bounded_header_*` | `test_t2_boundary_truncated_exif_*` | `test_t3_pairwise_read_only_sd_*` | `test_t4_scenario_photographer_*` |
| F-07 | Body Serial & Model Fallback | `test_t1_feat03_body_serial_*` | `test_t2_boundary_corrupted_exif_*` | `test_t3_pairwise_first_detection_*` | `test_t4_scenario_photographer_*` |
| F-08 | 100% Read-Only SD Protocol | `test_t1_feat03_read_only_*` | `test_t2_boundary_source_read_only_*` | `test_t3_pairwise_read_only_sd_*` | `test_t4_scenario_photographer_*` |
| F-09 | Atomic Camera Registry Persistence | `test_t1_feat04_camera_registry_*` | `test_t2_boundary_corrupt_registry_toml` | `test_t3_pairwise_first_detection_*` | `test_t4_scenario_photographer_*` |
| F-10 | $O(1)$ In-Memory Registry Index | `test_t1_feat04_o1_in_memory_*` | `test_t2_boundary_registry_scale_1000` | `test_t3_pairwise_multiple_cameras_*` | `test_t4_scenario_multi_camera_*` |
| F-11 | Async 1 MB Streaming Transfer | `test_t1_feat05_streaming_transfer_*` | `test_t2_boundary_massive_file_chunking` | `test_t3_pairwise_streaming_ingest_*` | `test_t4_scenario_photographer_*` |
| F-12 | In-Flight SHA-256 Checksumming | `test_t1_feat05_inflight_sha256_*` | `test_t2_boundary_zero_byte_photo` | `test_t3_pairwise_streaming_ingest_*` | `test_t4_scenario_photographer_*` |
| F-13 | Deduplication Skipping | `test_t1_feat05_deduplication_*` | `test_t2_boundary_same_name_diff_hash` | `test_t3_pairwise_repeat_insertion_*` | `test_t4_scenario_second_day_*` |
| F-14 | Date-Structured Hierarchy | `test_t1_feat05_date_structured_*` | `test_t2_boundary_missing_date_fallback` | `test_t3_pairwise_streaming_ingest_*` | `test_t4_scenario_photographer_*` |
| F-15 | Session Manifest Export | `test_t1_feat05_session_manifest_*` | `test_t2_boundary_manifest_write_failure` | `test_t3_pairwise_streaming_ingest_*` | `test_t4_scenario_photographer_*` |
| F-16 | Windows Native Toast Notification | `test_t1_feat07_toast_*` | `test_t2_boundary_toast_special_chars` | `test_t3_pairwise_streaming_ingest_*` | `test_t4_scenario_photographer_*` |
| F-17 | First-Time Interactive Association | `test_t1_feat04_register_*` | `test_t2_boundary_path_with_spaces_*` | `test_t3_pairwise_first_detection_*` | `test_t4_scenario_photographer_*` |
| F-18 | Repeat Insertion Unattended Mode | `test_t1_feat04_repeat_*` | `test_t2_boundary_deleted_destination_path` | `test_t3_pairwise_repeat_insertion_*` | `test_t4_scenario_second_day_*` |
| F-19 | 1-Key Empty Card Association | `test_t1_feat06_known_capacity_*` | `test_t2_boundary_multiple_capacity_match` | `test_t3_pairwise_formatted_card_*` | `test_t4_scenario_formatted_card_*` |
| F-20 | E2E Test Suite Pass & Adversarial | Suite execution | Adversarial suite pass | Cross-feature pass | Real-world workflow pass |

---

## 5. Test Execution Commands

```bash
# Compile and run entire SD Ingest Hub test suite
cargo test --test tier1_feature_tests
cargo test --test tier2_boundary_tests
cargo test --test tier3_pairwise_tests
cargo test --test tier4_scenario_tests

# Run all test suites in parallel
cargo test --tests
```
