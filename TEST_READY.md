# TEST_READY.md: LightCraft SD Ingest Hub & Modular Launcher Test Suite Baseline

**Status**: READY  
**Generated At**: 2026-10-24T03:15:00Z  
**Target Repository**: `lightcraft-tui` (`hydroid`)  
**Specification References**: `tasks/prd-sd-ingest-hub.md`, `ORIGINAL_REQUEST.md`, `PROJECT.md`, `TEST_INFRA.md`  

---

## 1. Test Suite Summary by Tier

| Test Tier | Source File | Purpose | Test Count | Pass / Fail | Execution Time |
|---|---|---|---|---|---|
| **Tier 1: Feature Integration** | `tests/tier1_feature_tests.rs` | Unit & integration coverage across 7 core feature groups (Launcher, Scanner, EXIF, Registry, Streaming, Empty Card, Toast) | 47 tests | **47 / 0 PASS** | ~2.3s |
| **Tier 2: Boundary & Corner Cases** | `tests/tier2_boundary_tests.rs` | Edge cases, exact 99% margin threshold, 0-byte photos, truncated/corrupt EXIF, read-only medium write-lock, duplicate filename collisions, path sandboxing | 19 tests | **19 / 0 PASS** | ~0.5s |
| **Tier 3: Pairwise Combinations** | `tests/tier3_pairwise_tests.rs` | Cross-feature interactions (first-time wizard -> registry save -> second insert -> duplicate skip; empty card -> capacity match -> 1-touch bind -> transfer; multi-camera isolation; capacity expansion) | 13 tests | **13 / 0 PASS** | ~0.5s |
| **Tier 4: Realistic User Scenarios** | `tests/tier4_scenario_tests.rs` | End-to-end workflows: Canon R6 50-photo ingest with SHA-256 manifest & toast; Day 2 incremental shooting with duplicate skipping; in-field card formatting; multi-camera sports/portraits isolation | 9 tests | **9 / 0 PASS** | ~0.6s |
| **Total Test Suite Baseline** | All 4 Test Tiers | Full regression and end-to-end coverage for SD Ingest Hub & Modular Launcher Hub | **88 tests** | **88 / 0 PASS (100%)** | **~3.8s** |

---

## 2. Test Infrastructure Artifacts Created

1. **`TEST_INFRA.md`**: Authoritative test architecture, opaque-box philosophy, strict read-only SD invariants, and complete feature coverage mapping across Features 1–20.
2. **`tests/common/contracts.rs`**: Strongly-typed interface contracts (`ToolOption`, `SdCardVolume`, `VolumeScanner`, `CameraFingerprint`, `CameraProfile`, `IngestSummary`, `IngestSessionManifest`, `ToastNotifier`, `MockToastNotifier`, `IngestError`).
3. **`tests/common/exif_gen.rs`**: Authentic binary EXIF generator and bounded header parser reading first 64KB–128KB in < 50ms (`Tag::Make`, `Tag::Model`, `Tag::BodySerialNumber (0xa431)`, `Tag::DateTimeOriginal`, `Tag::LensModel`).
4. **`tests/common/sd_fixture.rs`**: Synthetic SD card directory fixture with `tempfile::tempdir()`, DCIM crawler simulation, capacity modeling, read-only permissions emulation, and cryptographic SHA-256 tree immutability verifier (`assert_unmodified`).
5. **`tests/common/registry_fixture.rs`**: Isolated camera registry TOML manager with atomic temp-file write + rename and $O(1)$ in-memory lookups.
6. **`tests/common/pipeline_harness.rs`**: Async 1 MB streaming copy engine, in-flight SHA-256 checksumming, duplicate skipping, date-structured organization, manifest exporter, and toast notifier.

---

## 3. Test Runner Commands

Run individual tiers:
```bash
# Tier 1 Feature Integration Tests
cargo test --test tier1_feature_tests

# Tier 2 Boundary & Corner Case Tests
cargo test --test tier2_boundary_tests

# Tier 3 Pairwise Cross-Feature Tests
cargo test --test tier3_pairwise_tests

# Tier 4 Realistic User Workflow Scenarios
cargo test --test tier4_scenario_tests
```

Run entire SD Ingest Hub test suite together:
```bash
cargo test --test tier1_feature_tests --test tier2_boundary_tests --test tier3_pairwise_tests --test tier4_scenario_tests
```
