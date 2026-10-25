# Project: LightCraft SD Ingest Hub & Modular Launcher

## Architecture
LightCraft TUI (`hydroid`) is extended with a modular tool launcher hub and a secure, 100% read-only SD photo ingest engine:

1. **Modular Hub Layer (`src/hub/`)**:
   - `ToolOption` enum: `Studio` (existing RAW developer), `SdIngest` (SD Ingest & Storage Analyzer), `Exit`.
   - `src/hub/view.rs`: Interactive Ratatui interface with arrow navigation `[↑/↓]`, numeric shortcuts `[1]`, `[2]`, `[Q]`, and seamless terminal transition sharing `TerminalGuard`.
   - `src/main.rs`: CLI dispatcher supporting direct flags (`--tui`, `--sd-ingest`, `--verify-integrity`, `--sequential-export`) to bypass launcher, defaulting to interactive Launcher Hub on bare terminal.

2. **Removable Drive Scanner Layer (`src/ingest/scanner.rs`, `src/ingest/empty_handler.rs`)**:
   - `VolumeScanner` trait abstracting drive enumeration and filesystem inspection.
   - `WindowsVolumeScanner`: Win32 `GetLogicalDrives`, `GetDriveTypeW` (`DRIVE_REMOVABLE = 2`), `GetVolumeInformationW` (label, `exFAT`/`FAT32`), `GetDiskFreeSpaceExW` (total, free bytes).
   - DCIM crawler enumerating camera folders (`100CANON`, etc.) and counting RAW (`.CR3`, `.CR2`, `.ARW`, `.NEF`), JPEG, and MP4/MOV videos.
   - Empty card detector: recognizes formatted cards (`free_bytes == total_bytes` within 99% margin, 0 photos in `DCIM/`).

3. **EXIF Fingerprint & Security Layer (`src/ingest/exif_fingerprint.rs`)**:
   - 100% Read-Only SD Protocol: strict read-only file descriptors (`File::open`), zero write markers on SD. Compatible with physical write-lock switch.
   - Fast partial EXIF extraction: reads only the first 64 KB – 128 KB via bounded `BufReader` and `kamadak_exif::Reader::new().read_from_container()` in < 50ms.
   - Camera identity resolution: extracts `BodySerialNumber` (`Tag(Context::Exif, 0xa431)`) and `Model` (`Tag::Model`), falling back to composite `{Make}_{Model}`.

4. **Camera Registry Layer (`src/ingest/registry.rs`)**:
   - Persistent store in `~/.lightcraft/camera_registry.toml` using atomic temp-file write + rename.
   - In-memory `HashMap<String, CameraProfile>` for $O(1)$ lookups.
   - Stores `camera_id`, `display_name`, `target_directory`, `known_card_capacities`, `last_ingest_timestamp`, `total_photos_ingested`.

5. **Streaming Ingest Pipeline Layer (`src/ingest/pipeline.rs`, `src/ingest/toast.rs`)**:
   - Asynchronous 1 MB chunk streaming via `tokio::io::AsyncReadExt` and `tokio::io::AsyncWriteExt` (< 50 MB RAM usage).
   - In-flight cryptographic SHA-256 calculation (`sha2`) without disk re-reads.
   - Deduplication: skips transfer if destination file exists with identical size and SHA-256 hash.
   - Date-based folder organization: `{target_directory}/{YYYY}/{YYYY-MM-DD}/`.
   - Session audit manifest: writes `ingest_manifest_{timestamp}.json` in destination directory.
   - Native Windows Toast notification: dispatches non-blocking PowerShell WinRT XML DOM toast notification with `CREATE_NO_WINDOW`.

6. **Interactive Ingest Setup Layer (`src/tui/ingest_setup.rs`)**:
   - First-time camera detection modal (prompts for destination path, validates path, saves profile).
   - Repeat insertion automated banner (shows recognized camera name, destination path, auto-starts transfer after 2s countdown, cancellable via `[Esc]`).
   - Formatted empty card 1-touch binding prompt (`[Enter] = Yes / [Esc] = Ignore`).

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---------|-------------|-----------|--------|
| 1 | Interactive Tool Selector | Launcher hub menu with arrows and `[1]`, `[2]`, `[Q]` keys | M1 | US-001, FR-1 |
| 2 | CLI Flag Direct Bypass | Direct bypass for `--tui`, `--sd-ingest`, `--verify-integrity`, etc. | M1 | US-001, FR-1 |
| 3 | Windows Removable Drive Discovery | Win32 volume scanner for `DRIVE_REMOVABLE` and exFAT/FAT32 | M2 | US-002, FR-2 |
| 4 | DCIM Structure & Media Crawler | Traverses `DCIM/`, counts RAW, JPEG, videos, and total size | M2 | US-002, FR-2 |
| 5 | Empty Card Detection Logic | Checks formatted cards (`free_bytes == total_bytes`), matching capacity | M2 | US-007, FR-10 |
| 6 | Bounded Header EXIF Reader | Reads first 64KB-128KB in <50ms to extract camera tags | M3 | US-003, FR-4 |
| 7 | Body Serial & Model Fallback | Extracts EXIF `0xa431` serial, fallback to Make+Model | M3 | US-003, R3 |
| 8 | 100% Read-Only SD Protocol | Zero writes to SD card; safe on write-locked cards | M3 | US-004, FR-3 |
| 9 | Atomic Camera Registry Persistence | Atomic TOML persistence at `~/.lightcraft/camera_registry.toml` | M3 | US-004, FR-5 |
| 10 | $O(1)$ In-Memory Registry Index | In-memory `HashMap<String, CameraProfile>` for instant lookups | M3 | US-004 |
| 11 | Async 1 MB Streaming Transfer | Copies files in 1 MB chunks using `tokio::io` with <50MB RAM | M4 | US-006, FR-8 |
| 12 | In-Flight SHA-256 Checksumming | Computes SHA-256 on the fly during copy without re-reading | M4 | US-006, FR-8 |
| 13 | Deduplication Skipping | Skips transfer if destination file has matching size and hash | M4 | US-006, FR-9 |
| 14 | Date-Structured Hierarchy | Stores photos in `{target_directory}/{YYYY}/{YYYY-MM-DD}/` | M4 | US-006 |
| 15 | Session Manifest Export | Writes `ingest_manifest_{timestamp}.json` with full audit trail | M4 | US-006, FR-8 |
| 16 | Windows Native Toast Notification | Dispatches native toast notification upon ingest completion | M4 | FR-11, R5 |
| 17 | First-Time Interactive Association | Modal dialog prompting for destination directory on new camera | M5 | US-005, FR-6 |
| 18 | Repeat Insertion Unattended Mode | 2-second countdown banner and automated transfer start | M5 | US-005, FR-7 |
| 19 | 1-Key Empty Card Association Dialog | 1-touch `[Enter]` association for formatted empty cards | M5 | US-007, FR-10 |
| 20 | E2E Test Suite Pass & Adversarial Hardening | Pass 100% of E2E tests (Tiers 1-4) + Tier 5 coverage audit | M6 | Acceptance Criteria |

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| 1 | M1: Modular Tool Launcher Hub & CLI Bypass | `src/hub/mod.rs`, `src/hub/view.rs`, `src/main.rs` CLI update | none | DONE |
| 2 | M2: Windows Removable Drive & DCIM Scanner | `src/ingest/scanner.rs`, `src/ingest/empty_handler.rs`, `VolumeScanner` trait, Win32 FFI | none | DONE |
| 3 | M3: Camera EXIF Fingerprint & Local Registry Persistence | `src/ingest/exif_fingerprint.rs`, `src/ingest/registry.rs`, 100% read-only SD protocol, atomic TOML | none | DONE |
| 4 | M4: Streaming Ingest Pipeline, In-Flight SHA-256 & Toast | `src/ingest/pipeline.rs`, `src/ingest/toast.rs`, streaming copy, deduplication, manifest, Windows toast | M2, M3 | DONE |
| 5 | M5: Interactive TUI Ingest Setup & Full Hub Integration | `src/tui/ingest_setup.rs`, first-time modal, 2s banner, empty card dialog, hub option `[2]` integration | M1, M2, M3, M4 | DONE |
| 6 | M6: E2E Test Suite 100% Pass & Adversarial Hardening | Phase 1: Pass 100% E2E tests (Tiers 1-4); Phase 2: Tier 5 Adversarial Coverage Hardening | M5, TEST_READY.md | DONE |

## Interface Contracts

### M1 (Launcher) ↔ App & Ingest
- `pub enum ToolOption { Studio, SdIngest, Exit }`
- `pub async fn run_launcher(terminal: &mut TerminalGuard) -> Result<ToolOption, IngestError>`
- In `src/main.rs`:
  - When `--tui`: launches Studio directly.
  - When `--sd-ingest`: launches SD Ingest directly.
  - When bare interactive terminal: runs `run_launcher(&mut terminal)`.
    - If `ToolOption::Studio`: executes `app.run(terminal)`.
    - If `ToolOption::SdIngest`: executes `run_ingest_tui(&mut terminal)`.
    - If `ToolOption::Exit`: cleans up terminal and exits cleanly.

### M2 (Scanner) ↔ M4 (Pipeline) & M5 (TUI)
- `pub trait VolumeScanner: Send + Sync { fn scan_volumes(&self) -> Result<Vec<SdCardVolume>, IngestError>; }`
- `pub struct SdCardVolume { pub drive_letter: char, pub volume_label: String, pub file_system: String, pub total_bytes: u64, pub free_bytes: u64, pub has_dcim: bool, pub camera_folders: Vec<PathBuf> }`
- `pub fn check_empty_card(volume: &SdCardVolume) -> bool`

### M3 (Fingerprint & Registry) ↔ M4 (Pipeline) & M5 (TUI)
- `pub struct CameraFingerprint { pub make: String, pub model: String, pub body_serial: Option<String>, pub lens_model: Option<String> }`
- `pub fn extract_camera_fingerprint(photo_path: &Path) -> Result<CameraFingerprint, IngestError>`
- `pub struct CameraProfile { pub camera_id: String, pub display_name: String, pub target_directory: PathBuf, pub known_card_capacities: Vec<u64>, pub last_ingest_timestamp: Option<DateTime<Utc>>, pub total_photos_ingested: u64 }`
- `pub struct CameraRegistry { ... }` with methods:
  - `pub fn load() -> Result<Self, IngestError>`
  - `pub fn save(&self) -> Result<(), IngestError>`
  - `pub fn get_profile(&self, camera_id: &str) -> Option<&CameraProfile>`
  - `pub fn register_or_update(&mut self, profile: CameraProfile) -> Result<(), IngestError>`
  - `pub fn find_by_capacity(&self, total_bytes: u64) -> Option<&CameraProfile>`

### M4 (Pipeline & Toast) ↔ M5 (TUI)
- `pub struct IngestPipeline { ... }` with methods:
  - `pub async fn run_ingest(volume: &SdCardVolume, profile: &CameraProfile, progress_tx: Option<mpsc::Sender<IngestProgress>>) -> Result<IngestSummary, IngestError>`
- `pub struct IngestSummary { pub transferred_files: usize, pub skipped_duplicates: usize, pub total_bytes: u64, pub manifest_path: PathBuf }`
- `pub trait ToastNotifier: Send + Sync { fn send_toast(&self, title: &str, message: &str) -> Result<(), IngestError>; }`
- `pub struct WindowsToastNotifier;`

## Code Layout
```
src/
├── main.rs                   # CLI flag parsing & launcher bypass routing
├── lib.rs                    # Module exports
├── hub/
│   ├── mod.rs                # ToolOption enum and module exports
│   └── view.rs               # Launcher Hub TUI view & event handling
├── ingest/
│   ├── mod.rs                # Ingest module definitions & error types
│   ├── scanner.rs            # VolumeScanner trait & WindowsVolumeScanner Win32 implementation
│   ├── exif_fingerprint.rs   # Read-only bounded EXIF extractor (0xa431 serial & model fallback)
│   ├── registry.rs           # Atomic TOML camera registry & HashMap index
│   ├── empty_handler.rs      # Formatted empty card detection and capacity matching
│   ├── pipeline.rs           # Async 1MB chunk streaming, SHA-256 in flight, deduplication, manifest
│   └── toast.rs              # Windows native Toast notification via PowerShell WinRT
└── tui/
    └── ingest_setup.rs       # Interactive SD Ingest TUI view (first-time, repeat, empty card dialogs)
tests/
├── e2e_test_harness/         # Test infrastructure & synthetic SD card fixtures
├── tier1_feature_tests.rs    # Tier 1 Feature coverage tests
├── tier2_boundary_tests.rs   # Tier 2 Boundary & corner case tests
├── tier3_pairwise_tests.rs   # Tier 3 Cross-feature combination tests
└── tier4_scenario_tests.rs   # Tier 4 Real-world application scenarios
```
