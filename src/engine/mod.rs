//! Photographic recipe engine, preset catalog, and LightCraft develop bridges.

pub mod develop;
pub mod presets;
pub mod recipe;

pub use develop::{DevelopBridge, NativePresetRegistry, PresetApplicationStrategy};
pub use presets::BuiltinPreset;
pub use recipe::{
    DevelopSlider, Recipe, RecipeCategory, RecipeMetadata, SliderBounds, SliderSection, TrackKind,
};
