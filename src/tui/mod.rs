//! Terminal user interface (TUI) subsystem facade.

pub mod event;
pub mod glyphs;
pub mod ingest_setup;
pub mod layout;
pub mod mode_a;
pub mod mode_b;
pub mod terminal;

pub use event::{AppEvent, EventManager};
pub use glyphs::{GlyphMode, GlyphSet};
pub use ingest_setup::{
    render_ingest_setup, run_ingest_tui, IngestPhase, IngestSetupState, IngestUiAction,
    SetupInputField,
};
pub use layout::{AppLayout, PaneId};
pub use mode_a::{
    compute_variant_grid_rects, HistogramData, LoupeState, ModeAState, ModeAView, PhotoMetadata,
    SplitViewKind, VariantSlot,
};
pub use mode_b::{BatchJobStatus, BatchJobStub, ModeBState};
pub use terminal::TerminalGuard;
