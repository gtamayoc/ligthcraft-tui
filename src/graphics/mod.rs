//! Terminal graphics, protocol auto-detection, image downscaling, and preview caching facade.

pub mod cache;
pub mod protocol;
pub mod renderer;

pub use cache::{
    CacheProtocolKind, CachedProtocol, PreviewCache, PreviewCacheConfig, PreviewCacheKey,
};
pub use protocol::{GraphicsProtocol, ProtocolOverride};
pub use renderer::{FitBounds, ImageRenderer};
