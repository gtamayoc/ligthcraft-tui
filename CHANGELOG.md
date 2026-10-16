# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Variant Matrix Navigation (2D):** Full two-dimensional navigation (`Up`, `Down`, `Left`, `Right`) across 2×2 and 3×2 variant grids with synchronized active focus indicators.
- **Immediate Sequential Dual Export (`[E]`):** One-key generation of dual archival masters (TIFF 16-bit uncompressed + Full-Resolution JPEG 100%) targeting `export/interactive/`.
- **Status Badges & Progress Feedback:** Per-slot visual status badges (`✔ Listo`, `⟳ Generando...`, `⏳ Pendiente`, `✖ Error`) in variant matrix slots.
- **Dynamic Viewport Scroll:** Library pane list scrolling to prevent active cursor drift during keyboard navigation.
- **Photographic Metadata Architecture:** Consolidated zero-copy native EXIF metadata extraction supporting shutter speed, aperture, ISO, focal length, aspect ratios, and megapixel calculations.

### Fixed
- Fixed indefinite preview hang in TUI by introducing background worker dispatch (`tokio::spawn_blocking`) with protective 10-second timeouts.
- Fixed focus ring desynchronization between library viewport and variant matrix items.
- Fixed Windows console double-trigger behavior by filtering out key-release events.

## [0.1.0] - 2026-10-15

### Added
- Initial release of LightCraft TUI.
- Mode A (Single Photo Inspector) with 4-channel real-time luminance and RGB histogram (<2ms).
- Mode B (Batch Queue Controller) with multi-worker concurrent pool, pause, resume, and cancellation.
- Model Context Protocol (MCP) JSON-RPC 2.0 client bridge with tool discovery and develop command execution.
- Canon CR3 RAW baseline immutability guard using pure-Rust SHA-256 verification.
