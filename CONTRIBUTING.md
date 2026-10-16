# Contributing to LightCraft TUI

Thank you for your interest in contributing to **LightCraft TUI**! We welcome contributions to our native photographic workflow engine, high-performance TUI terminal interfaces, MCP integration, and queue architecture.

## Code of Conduct

Please be respectful and constructive in all discussions, issues, and pull requests.

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) (edition 2021, MSRV 1.74+)
- `cargo` and `clippy`
- Standard Git CLI

### Build & Check

Clone the repository and verify compilation:

```bash
git clone https://github.com/gtamayoc/ligthcraft-tui.git
cd ligthcraft-tui
cargo check --all-targets
```

### Running Tests

We maintain an extensive, multi-tier test suite:

```bash
# Run unit tests
cargo test --lib

# Run specific integration test tiers
cargo test --test tier1_feature_tests
cargo test --test tier1_tui_layout_tests
cargo test --test tier2_boundary_tests
cargo test --test tier3_pairwise_tests
```

*Note: Tier 4 tests require full local 24MP Canon CR3 RAW files in the `DRAW/` directory with verified SHA-256 baseline hashes.*

## Contribution Workflow

1. **Fork and Branch:** Create a feature or bugfix branch from `main`:
   ```bash
   git checkout -b feature/my-new-feature
   ```
2. **Follow Rust Idioms & Formatting:**
   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   ```
3. **Commit Cleanly:** Write clear, concise commit messages following the Conventional Commits style (e.g., `feat: ...`, `fix: ...`, `docs: ...`, `refactor: ...`).
4. **Open a Pull Request:** Describe your changes in detail, link any relevant issues, and ensure all CI checks pass.

## Architecture Guidelines

- **Zero Unsoundness:** Avoid `unsafe` blocks whenever possible.
- **Source Immutability:** Never alter or delete original RAW assets. All image operations and develop recipes must remain strictly non-destructive.
- **Decoupled Engine:** Keep core image processing and storage structures independent of terminal rendering logic.

## Licensing

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you shall be dual licensed under the Apache 2.0 License and the MIT License, without any additional terms or conditions.
