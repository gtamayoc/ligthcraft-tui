//! Terminal user interface (TUI) subsystem facade.

pub mod event;
pub mod layout;
pub mod mode_a;
pub mod mode_b;
pub mod terminal;

pub use event::{AppEvent, EventManager};
pub use layout::{AppLayout, PaneId};
pub use mode_a::{
    compute_variant_grid_rects, HistogramData, LoupeState, ModeAState, ModeAView, PhotoMetadata,
    SplitViewKind, VariantSlot,
};
pub use mode_b::{BatchJobStatus, BatchJobStub, ModeBState};
pub use terminal::TerminalGuard;
